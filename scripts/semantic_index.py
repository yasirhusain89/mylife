"""Thin project delegate to the universal semantic index.

All logic lives in ~/.config/opencode/scripts/semantic_index.py (ollama
nomic-embed-text + sqlite-vec, localhost-only guard). This shim preserves the
project's CLI entrypoint (`pnpm index:semantic` / `pnpm semantic:q`) and pins
the repo root to fotoflow.

Project-level customization lives in <repo>/.semindex/config.json
(keys: scan_exts, exclude_dirs, skip_files, max_file_bytes, max_json_bytes)
or SEMANTIC_* env vars — see the universal script docstring.
"""

from __future__ import annotations

import os
import runpy
import sys
from pathlib import Path

UNIVERSAL = Path.home() / ".config" / "opencode" / "scripts" / "semantic_index.py"
REPO_ROOT = Path(__file__).resolve().parent.parent


def main() -> None:
    if not UNIVERSAL.exists():
        raise SystemExit(f"error: universal script missing at {UNIVERSAL}")
    os.environ.setdefault("SEMANTIC_ROOT", str(REPO_ROOT))
    sys.argv[0] = str(UNIVERSAL)
    runpy.run_path(str(UNIVERSAL), run_name="__main__")


if __name__ == "__main__":
    main()
