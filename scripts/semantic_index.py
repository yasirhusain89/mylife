"""Local semantic code index: ollama embeddings + sqlite-vec.

Dev tooling. Same pattern as monartha's `python/semantic_index.py`, adapted
for fotoflow (Svelte frontend + Rust backend + SQL migrations).

Usage (wrappers in package.json):
    pnpm index:semantic   # rebuild .semindex/index.db
    pnpm semantic:q "where is photo import handled" [-k 8]   # no `--`: pnpm forwards it and breaks option parsing

Or directly:
    uv run --with sqlite-vec python scripts/semantic_index.py status

Environment:
    OLLAMA_URL             defaults to http://localhost:11434; non-localhost is refused unless
                           SEMANTIC_ALLOW_REMOTE=1 (source code would leave this machine)
    SEMANTIC_ALLOW_REMOTE  set to 1 to knowingly allow a remote OLLAMA_URL
    SEMANTIC_MODEL         defaults to nomic-embed-text (768 dims)
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import urlparse

import sqlite_vec

REPO_ROOT = Path(__file__).resolve().parent.parent
DB_PATH = REPO_ROOT / ".semindex" / "index.db"

SCAN_DIRS = ["src", "src-tauri/src", "src-tauri/migrations", "sidecar", "scripts", "e2e"]
SCAN_EXTS = {".ts", ".tsx", ".svelte", ".js", ".mjs", ".rs", ".py", ".sql", ".sh", ".css", ".swift"}
EXCLUDE_DIRS = {
    "node_modules",
    "build",
    ".gitnexus",
    ".zvec-grep",
    ".semindex",
    ".svelte-kit",
    ".worktrees",
    ".venv",
    "coverage",
    "test-results",
    "playwright-report",
    "storybook-static",
    "dist",
    "target",
    "data",
    "generated",
}
SKIP_FILES = {"package-lock.json", "uv.lock", "pnpm-lock.yaml"}
MAX_FILE_BYTES = 300_000

CHUNK_LINES = 60
CHUNK_OVERLAP = 10
EMBED_BATCH = 32
DIMENSIONS = 768


def log(msg: str) -> None:
    print(msg, file=sys.stderr)


def ollama_url() -> str:
    import os

    return os.environ.get("OLLAMA_URL", "http://localhost:11434").rstrip("/")


def model_name() -> str:
    import os

    return os.environ.get("SEMANTIC_MODEL", "nomic-embed-text")


LOCAL_HOSTS = {"localhost", "127.0.0.1", "::1", "[::1]"}


def guard_remote_url() -> None:
    import os

    host = urlparse(ollama_url()).hostname or ""
    if host in LOCAL_HOSTS:
        return
    if os.environ.get("SEMANTIC_ALLOW_REMOTE") != "1":
        raise SystemExit(
            f"error: OLLAMA_URL={ollama_url()} is not localhost — source code would leave this "
            "machine. Set SEMANTIC_ALLOW_REMOTE=1 to allow intentionally."
        )
    log(f"warning: sending source code to remote ollama at {ollama_url()}")


def embed(texts: list[str]) -> list[list[float]]:
    req = urllib.request.Request(
        f"{ollama_url()}/api/embed",
        data=json.dumps({"model": model_name(), "input": texts}).encode(),
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            body = json.loads(resp.read())
    except urllib.error.URLError as exc:
        raise SystemExit(
            f"error: cannot reach ollama at {ollama_url()} ({exc}). Start it with: ollama serve"
        ) from exc
    embeddings = body.get("embeddings")
    if not embeddings or len(embeddings) != len(texts):
        raise SystemExit(
            f"error: ollama returned {len(embeddings or [])} embeddings for {len(texts)} inputs"
        )
    return embeddings


def connect_db() -> sqlite3.Connection:
    db = sqlite3.connect(DB_PATH)
    db.enable_load_extension(True)
    sqlite_vec.load(db)
    db.enable_load_extension(False)
    return db


def create_schema(db: sqlite3.Connection) -> None:
    db.execute(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, path TEXT, start_line INTEGER, end_line INTEGER)"
    )
    db.execute("CREATE INDEX idx_chunks_path ON chunks(path)")
    db.execute(
        f"CREATE VIRTUAL TABLE vec_chunks USING vec0(chunk_id INTEGER PRIMARY KEY, embedding float[{DIMENSIONS}])"
    )


def iter_files():
    for scan_dir in SCAN_DIRS:
        root = REPO_ROOT / scan_dir
        if not root.is_dir():
            continue
        for path in sorted(root.rglob("*")):
            if not path.is_file() or path.suffix not in SCAN_EXTS:
                continue
            if EXCLUDE_DIRS & set(path.relative_to(REPO_ROOT).parts):
                continue
            if path.name in SKIP_FILES or path.stat().st_size > MAX_FILE_BYTES:
                continue
            yield path


def chunk_file(path: Path) -> list[tuple[str, int, int]]:
    lines = path.read_text(errors="replace").splitlines()
    chunks = []
    step = CHUNK_LINES - CHUNK_OVERLAP
    for start in range(0, max(len(lines), 1), step):
        window = lines[start : start + CHUNK_LINES]
        if not any(line.strip() for line in window):
            continue
        rel = path.relative_to(REPO_ROOT).as_posix()
        text = f"{rel}:{start + 1}-{start + len(window)}\n" + "\n".join(window)
        chunks.append((text, start + 1, start + len(window)))
    return chunks


def cmd_index(_args: argparse.Namespace) -> None:
    guard_remote_url()
    started = time.time()
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    for suffix in ("", "-wal", "-shm"):
        p = Path(str(DB_PATH) + suffix)
        if p.exists():
            p.unlink()

    files = list(iter_files())
    all_chunks: list[tuple[Path, str, int, int]] = []
    for path in files:
        for text, start_line, end_line in chunk_file(path):
            all_chunks.append((path, text, start_line, end_line))

    log(f"indexing {len(all_chunks)} chunks from {len(files)} files with {model_name()}...")
    db = connect_db()
    create_schema(db)
    for batch_start in range(0, len(all_chunks), EMBED_BATCH):
        batch = all_chunks[batch_start : batch_start + EMBED_BATCH]
        embeddings = embed([text for _, text, _, _ in batch])
        cur = db.cursor()
        for (path, text, start_line, end_line), vector in zip(batch, embeddings):
            cur.execute(
                "INSERT INTO chunks(path, start_line, end_line) VALUES (?, ?, ?)",
                (path.relative_to(REPO_ROOT).as_posix(), start_line, end_line),
            )
            chunk_id = cur.lastrowid
            cur.execute(
                "INSERT INTO vec_chunks(chunk_id, embedding) VALUES (?, ?)",
                (chunk_id, sqlite_vec.serialize_float32(vector)),
            )
        db.commit()
        done = min(batch_start + EMBED_BATCH, len(all_chunks))
        log(f"  {done}/{len(all_chunks)} chunks ({time.time() - started:.1f}s)")
    db.close()
    log(
        f"done: {len(all_chunks)} chunks in {time.time() - started:.1f}s -> {DB_PATH.relative_to(REPO_ROOT)}"
    )


def cmd_query(args: argparse.Namespace) -> None:
    guard_remote_url()
    if not DB_PATH.exists():
        raise SystemExit(
            "error: no index found. Run first: pnpm index:semantic"
        )
    db = connect_db()
    try:
        db.execute("SELECT count(*) FROM chunks").fetchone()
    except sqlite3.OperationalError:
        raise SystemExit(
            "error: index is empty or corrupt. Rebuild with: pnpm index:semantic"
        )
    vector = embed([args.text])[0]
    rows = db.execute(
        """
        SELECT c.path, c.start_line, c.end_line, v.distance
        FROM vec_chunks v
        JOIN chunks c ON c.id = v.chunk_id
        WHERE v.embedding MATCH ? AND k = ?
        ORDER BY v.distance
        """,
        (sqlite_vec.serialize_float32(vector), args.k),
    ).fetchall()
    db.close()
    for rank, (path, start_line, end_line, distance) in enumerate(rows, 1):
        print(f"{rank}. {path}:{start_line}-{end_line}  (distance {distance:.3f})")
        snippet = read_snippet(path, start_line, min(end_line, start_line + 3))
        for line in snippet:
            print(f"   {line}")
    if not rows:
        print("no results")


def read_snippet(path: str, start_line: int, end_line: int) -> list[str]:
    file_path = REPO_ROOT / path
    try:
        lines = file_path.read_text(errors="replace").splitlines()
    except OSError:
        return ["   <unreadable>"]
    return [
        f"   {lines[i - 1].strip()[:120]}" for i in range(start_line, min(end_line, len(lines)) + 1)
    ]


def cmd_status(_args: argparse.Namespace) -> None:
    if not DB_PATH.exists():
        print(f"no index at {DB_PATH.relative_to(REPO_ROOT)}")
        return
    db = connect_db()
    chunks = db.execute("SELECT count(*) FROM chunks").fetchone()[0]
    files = db.execute("SELECT count(DISTINCT path) FROM chunks").fetchone()[0]
    db.close()
    print(
        f"{chunks} chunks across {files} files | {DB_PATH.relative_to(REPO_ROOT)} | model {model_name()}"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Local semantic code index (ollama + sqlite-vec)")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("index", help="rebuild the index").set_defaults(func=cmd_index)
    q = sub.add_parser("query", help="semantic search")
    q.add_argument("text")
    q.add_argument("-k", type=int, default=8)
    q.set_defaults(func=cmd_query)
    sub.add_parser("status", help="index stats").set_defaults(func=cmd_status)
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
