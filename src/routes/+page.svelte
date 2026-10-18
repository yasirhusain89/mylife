<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    blurredAssets,
    cleanupStats,
    clustersList,
    clusterEvents,
    eventsList,
    eventAssets,
    renameEvent,
    exportHighlight,
    exportBundle,
    type ExportSummary,
    type HighlightResult,
    faceThumb,
    facesInCluster,
    duplicatesList,
    engineThumb,
    enrichAssets,
    findDuplicates,
    keepCopy,
    onHashProgress,
    onQualityProgress,
    scoreQuality,
    trashAssets,
    importFolder,
    onEnrichProgress,
    libraryList,
    libraryStats,
    onImportError,
    onImportProgress,
    onClusterProgress,
    runFaceClustering,
    verifyLibrary,
    type AssetRow,
    type CleanupProgress,
    type CleanupStats,
    type FaceClusterRow,
    type EventRow,
    type EventAssetRow,
    type FaceRow,
    type DuplicateGroup,
    type EnrichProgress,
    type ImportProgress,
    type LibraryStats
  } from "$lib/library";
  import { engineCall, engineSysInfo, type SysInfo } from "$lib/engine";

  type Row = { key: string; val: string };
  type EngineStatus =
    | { state: "idle" }
    | { state: "checking" }
    | { state: "up"; rows: Row[]; ms: number }
    | { state: "down"; error: string };

  const PAGE = 60;

  let status = $state<EngineStatus>({ state: "idle" });
  let assets = $state<AssetRow[]>([]);
  let stats = $state<LibraryStats | null>(null);
  let filter = $state<"all" | "image" | "video">("all");
  let root = $state<string | null>(null);
  let importing = $state(false);
  let progress = $state<ImportProgress | null>(null);
  let error = $state<string | null>(null);
  let thumbs = $state<Record<string, string>>({});
  let people = $state<FaceClusterRow[]>([]);
  let typedPath = $state("");
  let enriching = $state(false);
  let enrichState = $state<EnrichProgress | null>(null);
  let cleanup = $state<CleanupStats | null>(null);
  let groups = $state<DuplicateGroup[]>([]);
  let blurred = $state<AssetRow[]>([]);
  let hashing = $state(false);
  let scoring = $state(false);
  let hashState = $state<CleanupProgress | null>(null);
  let qualityState = $state<CleanupProgress | null>(null);
  let deleteMode = $state<"trash" | "flag">("trash");
  let confirm = $state<{ ids: number[]; label: string } | null>(null);
  let tab = $state<"library" | "cleanup" | "people" | "events">("library");
  let flagged = $state<Set<number>>(new Set());
  let peopleLoading = $state(false);
  let events = $state<EventRow[]>([]);
  let eventsLoading = $state(false);
  let toast = $state<string | null>(null);

  async function checkEngine() {
    status = { state: "checking" };
    const t0 = performance.now();
    try {
      const pong = await engineCall<{ pong: boolean }>("ping");
      if (!pong.pong) throw new Error("engine did not answer pong");
      const info = await engineSysInfo();
      status = {
        state: "up",
        ms: Math.round(performance.now() - t0),
        rows: [
          { key: "OS", val: `${info.os}` },
          { key: "Architecture", val: `${info.arch} · ${info.cpuCount} cores` },
          { key: "Memory", val: `${info.memoryGB.toFixed(1)} GB unified` },
          { key: "Swift engine", val: info.swift }
        ]
      };
    } catch (e) {
      status = { state: "down", error: String(e).replace(/^Error: /, "") };
    }
  }

  function fmtBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
    return `${(n / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function fmtDate(epoch: number): string {
    return new Date(epoch * 1000).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric"
    });
  }

  async function refresh() {
    assets = await libraryList({ kind: filter === "all" ? undefined : filter, limit: PAGE });
    stats = await libraryStats();
    void loadThumbs(assets);
  }

  async function loadThumbs(rows: AssetRow[]) {
    for (const a of rows) {
      if (thumbs[a.path]) continue;
      try {
        const { image } = await engineThumb(a.path, 256);
        if (image) thumbs[a.path] = `data:image/jpeg;base64,${image}`;
      } catch {
        /* unreadable asset: leave the placeholder */
      }
    }
  }

  async function runImport(target: string) {
    error = null;
    root = target;
    importing = true;
    progress = null;
    try {
      const summary = await importFolder(target, true);
      progress = {
        scanId: summary.scanId,
        scanned: summary.scanned,
        imported: summary.imported,
        unchanged: summary.unchanged,
        bytes: summary.bytes
      };
      await refresh();
      void runEnrich();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      importing = false;
    }
  }

  async function refreshCleanup() {
    cleanup = await cleanupStats();
    groups = await duplicatesList();
    blurred = await blurredAssets(36);
    void loadThumbs(blurred);
    for (const g of groups) {
      for (const m of g.members) void loadThumbs([{ path: m.path } as AssetRow]);
    }
  }

  let highlightBusy = $state<number | null>(null);
  let highlightResult = $state<HighlightResult | null>(null);
  let highlightEventId = $state<number | null>(null);

  let exportBusy = $state<string | null>(null);
  let exportResult = $state<ExportSummary | null>(null);
  let exportEventId = $state<number | null>(null);

  async function runExport(event: EventRow, mode: "drive" | "whatsapp") {
    const dest = await open({ directory: true, multiple: false, title: "Choose export folder" });
    if (typeof dest !== "string" || dest === "") return;
    exportBusy = `${event.id}-${mode}`;
    exportResult = null;
    exportEventId = event.id;
    try {
      exportResult = await exportBundle(event.id, event.name, mode, dest);
      toast = `Exported ${exportResult.exported} photos for ${mode === "drive" ? "Drive" : "WhatsApp"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      exportBusy = null;
    }
  }

  async function makeHighlight(event: EventRow) {
    highlightBusy = event.id;
    highlightResult = null;
    highlightEventId = event.id;
    try {
      highlightResult = await exportHighlight(event.id, event.name.replace(/\s+/g, "-").toLowerCase());
      toast = "Highlight video exported";
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      highlightBusy = null;
    }
  }

  let peopleFaces = $state<Record<number, FaceRow[]>>({});
  let clusterState = $state<{ processed: number; total: number } | null>(null);

  async function runVerify() {
    try {
      const r = await verifyLibrary();
      await refresh();
      toast =
        r.missing === 0 && r.restored === 0
          ? "All indexed files are present"
          : `${r.missing} missing · ${r.restored} restored`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function loadPeople() {
    peopleLoading = true;
    people = [];
    try {
      people = await clustersList();
      void loadClusterFaces();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      peopleLoading = false;
    }
  }

  /// Runs the engine's face detector + clusterer over the whole library.
  async function findPeople() {
    peopleLoading = true;
    clusterState = { processed: 0, total: 0 };
    try {
      await runFaceClustering();
      people = await clustersList();
      void loadClusterFaces();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      peopleLoading = false;
      clusterState = null;
    }
  }

  async function loadClusterFaces() {
    const next: Record<number, FaceRow[]> = {};
    for (const c of people) {
      try {
        next[c.id] = (await facesInCluster(c.id)).slice(0, 8);
      } catch {
        /* cluster listing is best-effort */
      }
    }
    peopleFaces = next;
    for (const faces of Object.values(next)) {
      for (const f of faces) void loadFaceThumb(f);
    }
  }

  async function loadFaceThumb(f: FaceRow) {
    const key = `face:${f.id}`;
    if (thumbs[key]) return;
    try {
      const { image } = await faceThumb(f.path, f.x, f.y, f.width, f.height, 160);
      if (image) thumbs[key] = `data:image/jpeg;base64,${image}`;
    } catch {
      /* unreadable face crop */
    }
  }

  async function loadEvents() {
    eventsLoading = true;
    try {
      events = await eventsList();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventsLoading = false;
    }
  }

  async function runClusterEvents() {
    eventsLoading = true;
    try {
      await clusterEvents(4, 500);
      events = await eventsList();
      toast = `Grouped into ${events.length} event${events.length === 1 ? "" : "s"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventsLoading = false;
    }
  }

  async function doRename(event: EventRow) {
    const name = window.prompt("Event name", event.name);
    if (name === null || name.trim() === "" || name === event.name) return;
    try {
      await renameEvent(event.id, name.trim());
      events = await eventsList();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function runFindDuplicates() {
    hashing = true;
    hashState = null;
    try {
      cleanup = await findDuplicates();
      await refreshCleanup();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      hashing = false;
    }
  }

  async function runScoreQuality() {
    scoring = true;
    qualityState = null;
    try {
      cleanup = await scoreQuality();
      await refreshCleanup();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      scoring = false;
    }
  }

  async function chooseKeeper(groupId: number, keepId: number) {
    groups = await keepCopy(groupId, keepId);
  }

  async function applyDeletion() {
    if (!confirm) return;
    const { ids, label } = confirm;
    confirm = null;
    try {
      if (deleteMode === "trash") {
        const result = await trashAssets(ids);
        if (result.failed.length > 0) {
          error = `${result.trashed} moved to Trash, ${result.failed.length} failed`;
        }
      } else {
        // Flag only: nothing on disk changes, the choice stays reversible.
        for (const id of ids) flagged.add(id);
        flagged = flagged;
      }
      await refreshCleanup();
      toast = `${label} · ${deleteMode === "trash" ? "moved to Trash" : "flagged only"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function runEnrich() {
    if (enriching) return;
    enriching = true;
    enrichState = { processed: 0, enriched: 0, failed: 0, pending: 0 };
    try {
      const summary = await enrichAssets(3);
      enrichState = {
        processed: summary.processed,
        enriched: summary.enriched,
        failed: summary.failed,
        pending: summary.pending
      };
      await refresh();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      enriching = false;
    }
  }

  async function chooseFolder() {
    try {
      const picked = await open({ directory: true, multiple: false, title: "Add photo folder" });
      if (typeof picked !== "string" || picked === "") return;
      await runImport(picked);
    } catch (e) {
      error = `Folder picker unavailable: ${String(e).replace(/^Error: /, "")} — paste a path below instead.`;
    }
  }

  async function importTypedPath() {
    const target = typedPath.trim();
    if (target === "") return;
    await runImport(target);
  }

  // The dev server is also reachable from a plain browser tab, where the Tauri
  // IPC bridge does not exist. Skip registration there instead of throwing.
  const inApp = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  onMount(() => {
    if (!inApp) {
      error = "Open this inside the fotoflow app window — the dev-server tab has no engine bridge.";
      return;
    }
    const offs = [
      onImportProgress((p) => {
        if (importing) progress = p;
      }),
      onImportError((m) => {
        error = m;
      }),
      onEnrichProgress((p) => {
        if (enriching) enrichState = p;
      }),
      onHashProgress((p) => {
        if (hashing) hashState = p;
      }),
      onQualityProgress((p) => {
        if (scoring) qualityState = p;
      }),
      onClusterProgress((p) => {
        if (peopleLoading && clusterState) {
          clusterState = { processed: p.processed, total: p.total };
        }
      })
    ];
    void refresh();
    return () => {
      for (const off of offs) void off.then((f) => f());
    };
  });
</script>

<main class="container">
  <header>
    <h1>fotoflow</h1>
    <p class="tagline">on-device library management for photo and video</p>
  </header>

  <nav class="tabs">
    <button class="tab" class:on={tab === "library"} onclick={() => { tab = "library"; void refresh(); }}>Library</button>
    <button class="tab" class:on={tab === "cleanup"} onclick={() => { tab = "cleanup"; void refreshCleanup(); }}>
      Cleanup{#if cleanup && cleanup.duplicateGroups > 0}&nbsp;{cleanup.duplicateGroups}{/if}
    </button>
    <button class="tab" class:on={tab === "people"} onclick={() => { tab = "people"; void loadPeople(); }}>
      People{#if people.length > 0}&nbsp;{people.length}{/if}
    </button>
    <button class="tab" class:on={tab === "events"} onclick={() => { tab = "events"; void loadEvents(); }}>
      Events{#if events.length > 0}&nbsp;{events.length}{/if}
    </button>
  </nav>

  {#if error}
    <p class="error global-error">{error}</p>
  {/if}

  {#if tab === "cleanup"}
    <section class="card" aria-labelledby="cleanup-heading">
      <div class="card-head">
        <h2 id="cleanup-heading">Cleanup</h2>
      </div>

      <div class="actions">
        <button onclick={() => void runFindDuplicates()} disabled={hashing}>
          {hashing ? "Hashing…" : "Find exact duplicates"}
        </button>
        <button onclick={() => void runScoreQuality()} disabled={scoring}>
          {scoring ? "Scoring…" : "Score sharpness"}
        </button>
      </div>

      {#if hashState}
        <p class="hint">{hashState.hashed ?? 0} hashed across {hashState.buckets ?? 0} size buckets</p>
      {/if}
      {#if qualityState}
        <p class="hint">{qualityState.scored ?? 0} scored · {qualityState.blurred ?? 0} look blurred</p>
      {/if}

      {#if cleanup}
        <div class="stats">
          <span class="chip static">{cleanup.duplicateGroups} duplicate groups</span>
          <span class="chip static">{fmtBytes(cleanup.wasteBytes)} recoverable</span>
          <span class="chip static">{cleanup.blurred} blurred</span>
        </div>
      {/if}

      <label class="mode">
        Deleting moves files to the macOS Trash
        <select bind:value={deleteMode}>
          <option value="trash">Move to Trash (reversible)</option>
          <option value="flag">Flag only (nothing changes)</option>
        </select>
      </label>

      {#if groups.length}
        <h3 class="sub">Duplicate groups</h3>
        {#each groups as group (group.id)}
          <article class="group">
            <header>
              <span>{group.members.length} copies · {fmtBytes(group.sizeBytes)} each</span>
              <span class="waste">keep 1, free {fmtBytes(group.wasteBytes)}</span>
            </header>
            <div class="members">
              {#each group.members as member (member.id)}
                <figure class:keeper={member.id === group.keptId}>
                  <div class="thumb small">
                    {#if thumbs[member.path]}
                      <img src={thumbs[member.path]} alt={member.name} loading="lazy" />
                    {:else}
                      <span class="placeholder">IMG</span>
                    {/if}
                  </div>
                  <figcaption>
                    <span class="name" title={member.path}>{member.name}</span>
                    {#if member.id === group.keptId}
                      <span class="badge keep">keeper</span>
                    {:else}
                      <div class="row-actions">
                        <button class="tiny" onclick={() => void chooseKeeper(group.id, member.id)}>
                          Keep this
                        </button>
                        <button class="tiny danger" onclick={() => (confirm = { ids: [member.id], label: member.name })}>
                          Remove
                        </button>
                      </div>
                    {/if}
                  </figcaption>
                </figure>
              {/each}
            </div>
            {#if group.members.length > 1}
              <button
                class="tiny danger"
                onclick={() => (confirm = { ids: group.members.filter((m) => m.id !== group.keptId).map((m) => m.id), label: `${group.members.length - 1} duplicates` })}
              >
                Move {group.members.length - 1} extra {group.members.length - 1 === 1 ? "copy" : "copies"} to Trash
              </button>
            {/if}
          </article>
        {/each}
      {/if}

      {#if blurred.length}
        <h3 class="sub">Likely blurred</h3>
        <div class="grid">
          {#each blurred as asset (asset.id)}
            <figure>
              <div class="thumb">
                {#if thumbs[asset.path]}
                  <img src={thumbs[asset.path]} alt={asset.name} loading="lazy" />
                {:else}
                  <span class="placeholder">{asset.ext.toUpperCase()}</span>
                {/if}
              </div>
              <figcaption>
                <span class="name" title={asset.name}>{asset.name}</span>
                <button class="tiny danger" onclick={() => (confirm = { ids: [asset.id], label: asset.name })}>
                  {flagged.has(asset.id) ? "flagged" : "remove"}
                </button>
              </figcaption>
            </figure>
          {/each}
        </div>
      {/if}
    </section>
  {:else if tab === "library"}
  <section class="card" aria-labelledby="library-heading">
    <div class="card-head">
      <h2 id="library-heading">Library</h2>
      <button onclick={chooseFolder} disabled={importing}>
        {importing ? "Importing…" : "Add folder"}
      </button>
    </div>

    <form
      class="path-entry"
      onsubmit={(e) => {
        e.preventDefault();
        void importTypedPath();
      }}
    >
      <input
        type="text"
        placeholder="…or paste a folder path"
        bind:value={typedPath}
        disabled={importing}
      />
      <button type="submit" disabled={importing || typedPath.trim() === ""}>Scan</button>
    </form>

    {#if root}
      <p class="root" title={root}>{root}</p>
    {/if}

    {#if progress}
      <div class="progress">
        <div class="bar" class:live={importing}><span></span></div>
        <p class="hint">
          {progress.scanned} scanned · {progress.imported} new · {progress.unchanged} unchanged ·
          {fmtBytes(progress.bytes)}
        </p>
      </div>
    {/if}

    {#if stats}
      <div class="stats">
        <button class="chip" class:on={filter === "all"} onclick={() => { filter = "all"; void refresh(); }}>
          All
        </button>
        <button class="chip" class:on={filter === "image"} onclick={() => { filter = "image"; void refresh(); }}>
          {stats.photos} photos
        </button>
        <button class="chip" class:on={filter === "video"} onclick={() => { filter = "video"; void refresh(); }}>
          {stats.videos} videos
        </button>
        <span class="total">{fmtBytes(stats.bytes)} indexed</span>
      </div>

      {#if stats.missing > 0}
        <p class="error">
          {stats.missing} file{stats.missing === 1 ? "" : "s"} unreachable (ejected volume or deleted) —
          <button class="tiny" onclick={() => void runVerify()}>Check files</button>
        </p>
      {:else}
        <p class="hint">
          <button class="tiny" onclick={() => void runVerify()}>Check files</button>
          <span> verify that indexed files still exist on disk</span>
        </p>
      {/if}

      {#if (stats.photos + stats.videos) > 0}
        <div class="enrich">
          <button class="ghost" onclick={() => void runEnrich()} disabled={enriching}>
            {enriching ? "Reading metadata…" : "Refresh metadata"}
          </button>
          {#if enrichState && enrichState.processed > 0}
            <span class="hint">
              {enrichState.enriched} enriched · {enrichState.failed} unreadable
              {#if enrichState.pending > 0}· {enrichState.pending} pending{/if}
            </span>
          {/if}
        </div>
      {/if}
    {/if}

    {#if assets.length}
      <div class="grid">
        {#each assets as asset (asset.id)}
          <figure>
            <div class="thumb">
              {#if thumbs[asset.path]}
                <img src={thumbs[asset.path]} alt={asset.name} loading="lazy" />
              {:else}
                <span class="placeholder">{asset.ext.toUpperCase()}</span>
              {/if}
            </div>
            <figcaption>
              <span class="name" title={asset.name}>{asset.name}</span>
              <span class="meta">
                {asset.capturedAt ?? asset.modified ? fmtDate(asset.capturedAt ?? asset.modified) : "—"}
                {#if asset.width && asset.height} · {asset.width}×{asset.height}{/if}
                · {fmtBytes(asset.sizeBytes)}
              </span>
            </figcaption>
          </figure>
        {/each}
      </div>
    {:else if !importing}
      <p class="hint">No assets yet — add a folder to build the index.</p>
    {/if}
  </section>

  {/if}

  {#if tab === "people"}
    <section class="card" aria-labelledby="people-heading">
      <div class="card-head">
        <h2 id="people-heading">People</h2>
        <button onclick={findPeople} disabled={peopleLoading}>
          {peopleLoading ? "Scanning faces…" : "Find people in library"}
        </button>
      </div>
      {#if clusterState}
        <p class="hint">{clusterState.processed}/{clusterState.total} images scanned…</p>
      {/if}
      {#if stats && stats.missing > 0}
        <p class="error">
          {stats.missing} indexed file{stats.missing === 1 ? " is" : "s are"} unreadable —
          the volume may be ejected. Reconnect it, then press “Check files” in the Library tab.
        </p>
      {/if}
      {#if people.length}
        {#each people as cluster (cluster.id)}
          <article class="group">
            <header>
              <span>Cluster {cluster.id}</span>
              <span class="waste">{cluster.faceCount} face{cluster.faceCount === 1 ? "" : "s"}</span>
            </header>
            {#if peopleFaces[cluster.id]?.length}
              <div class="members">
                {#each peopleFaces[cluster.id] as face (face.id)}
                  <figure>
                    <div class="thumb small">
                      {#if thumbs[`face:${face.id}`]}
                        <img src={thumbs[`face:${face.id}`]} alt="detected face" loading="lazy" />
                      {:else}
                        <span class="placeholder">…</span>
                      {/if}
                    </div>
                    <figcaption>
                      <span class="name" title={face.path}>{face.path.split("/").pop()}</span>
                    </figcaption>
                  </figure>
                {/each}
              </div>
            {/if}
          </article>
        {/each}
      {:else if !peopleLoading}
        <p class="hint">No people clusters yet — scan the library to detect faces.</p>
      {/if}
    </section>
  {/if}

  {#if tab === "events"}
    <section class="card" aria-labelledby="events-heading">
      <div class="card-head">
        <h2 id="events-heading">Events</h2>
        <button onclick={runClusterEvents} disabled={eventsLoading}>
          {eventsLoading ? "Clustering…" : "Cluster events by time & place"}
        </button>
      </div>
      {#if events.length}
        {#each events as event (event.id)}
          <article class="group event-card">
            <header>
              <button class="ename" title="Click to rename" onclick={() => void doRename(event)}>{event.name}</button>
              <span class="waste">
                {new Date(event.startedAt * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                – {new Date(event.endedAt * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
              </span>
            </header>
            <p class="meta">
              {event.assetCount} asset{event.assetCount === 1 ? "" : "s"} ·
              {event.faceCount} face{event.faceCount === 1 ? "" : "s"} ·
              {event.latitude !== null && event.longitude !== null
                ? `${event.latitude.toFixed(4)}, ${event.longitude.toFixed(4)}`
                : "no location"}
            </p>
            <div class="row-actions">
              <button class="tiny" disabled={event.assetCount === 0 || exportBusy !== null} onclick={() => void runExport(event, "drive")}>
                {exportBusy === `${event.id}-drive` ? "Copying…" : "For Google Drive"}
              </button>
              <button class="tiny" disabled={event.assetCount === 0 || exportBusy !== null} onclick={() => void runExport(event, "whatsapp")}>
                {exportBusy === `${event.id}-whatsapp` ? "Compressing…" : "For WhatsApp"}
              </button>
              <button class="tiny" disabled={event.assetCount === 0 || exportBusy !== null} onclick={() => void makeHighlight(event)}>
                {highlightBusy === event.id ? "Exporting…" : "Highlight video"}
              </button>
            </div>
            {#if highlightResult && highlightEventId === event.id}
              <p class="hint">
                Highlight: {highlightResult.seconds.toFixed(1)}s · {fmtBytes(highlightResult.bytes)}
                {#if highlightResult.videosSkipped > 0}· {highlightResult.videosSkipped} video{highlightResult.videosSkipped === 1 ? "" : "s"} skipped{/if}
                <br />{highlightResult.output}
              </p>
            {/if}
            {#if exportResult && exportEventId === event.id}
              <p class="hint">
                Exported {exportResult.exported} to {exportResult.dir}
                {#if exportResult.skipped > 0}· {exportResult.skipped} skipped{/if}
              </p>
            {/if}
          </article>
        {/each}
      {:else if !eventsLoading}
        <p class="hint">No events yet — cluster the library by time and place.</p>
      {/if}
    </section>
  {/if}

  <section class="card" aria-labelledby="engine-heading">
    <div class="card-head">
      <h2 id="engine-heading">Engine</h2>
      <button onclick={checkEngine} disabled={status.state === "checking"}>
        {status.state === "checking" ? "Checking…" : "Check engine"}
      </button>
    </div>

    <div class="status">
      <span class="sink" class:up={status.state === "up"} class:down={status.state === "down"}></span>
      <span class="state">
        {#if status.state === "idle"}Not verified
        {:else if status.state === "checking"}Connecting to engine…
        {:else if status.state === "up"}Connected · round-trip {status.ms} ms
        {:else}Engine error{/if}
      </span>
    </div>

    {#if status.state === "up"}
      <dl>
        {#each status.rows as row (row.key)}
          <div class="row">
            <dt>{row.key}</dt>
            <dd>{row.val}</dd>
          </div>
        {/each}
      </dl>
    {:else if status.state === "down"}
      <p class="error">{status.error}</p>
    {/if}
  </section>

  {#if confirm}
    <div class="scrim">
      <div class="dialog" role="alertdialog" aria-labelledby="confirm-heading">
        <h3 id="confirm-heading">Remove {confirm.label}?</h3>
        <p class="hint">
          {#if deleteMode === "trash"}
            This moves the file to the macOS Trash. You can put it back from Finder until the
            Trash is emptied.
          {:else}
            Flag only — nothing on disk changes.
          {/if}
        </p>
        <div class="dialog-actions">
          <button class="ghost" onclick={() => (confirm = null)}>Cancel</button>
          <button class="danger-solid" onclick={() => void applyDeletion()}>
            {deleteMode === "trash" ? "Move to Trash" : "Flag"}
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if toast}
    <p class="toast" role="status">{toast}</p>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    min-height: 100vh;
    background: #0d0f14;
    color: #e7eaf0;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", sans-serif;
  }
  .container {
    max-width: 120rem;
    width: 100%;
    margin: 0 auto;
    padding: 1.5rem clamp(1rem, 4vw, 3rem) 3rem;
  }
  header h1 {
    font-size: 2rem;
    margin: 0;
    letter-spacing: -0.02em;
  }
  .tagline {
    margin: 0.25rem 0 1.5rem;
    color: #8a93a5;
  }
  .card {
    background: #151820;
    border: 1px solid #232733;
    border-radius: 12px;
    padding: 1rem 1.25rem;
    margin-bottom: 1rem;
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }
  h2 {
    font-size: 1rem;
    margin: 0;
  }
  button {
    background: #2f6bff;
    color: #fff;
    border: 0;
    border-radius: 8px;
    padding: 0.4rem 0.9rem;
    font: inherit;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .path-entry {
    display: flex;
    gap: 0.4rem;
    margin-bottom: 0.6rem;
  }
  .path-entry input {
    flex: 1;
    background: #0d0f14;
    border: 1px solid #232733;
    border-radius: 8px;
    color: #e7eaf0;
    font: inherit;
    font-size: 0.85rem;
    padding: 0.35rem 0.6rem;
  }
  .path-entry input:focus {
    outline: none;
    border-color: #2f6bff;
  }
  .root {
    margin: 0 0 0.6rem;
    color: #8a93a5;
    font-size: 0.8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .progress {
    margin-bottom: 0.6rem;
  }
  .bar {
    height: 4px;
    background: #232733;
    border-radius: 2px;
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: #2f6bff;
    transition: width 0.2s ease;
  }
  /* Scan size is unknown up front, so the live bar sweeps instead of filling. */
  .bar.live span {
    width: 35%;
    animation: sweep 1.1s ease-in-out infinite;
  }
  @keyframes sweep {
    0% { margin-left: 0; }
    50% { margin-left: 65%; }
    100% { margin-left: 0; }
  }
  .hint {
    margin: 0.4rem 0 0;
    color: #8a93a5;
    font-size: 0.8rem;
  }
  .stats {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.6rem 0;
    flex-wrap: wrap;
  }
  .chip {
    background: #1c212c;
    color: #b6bdc9;
    border: 1px solid #232733;
    border-radius: 999px;
    padding: 0.2rem 0.7rem;
    font-size: 0.8rem;
  }
  .chip.on {
    background: #2f6bff;
    border-color: #2f6bff;
    color: #fff;
  }
  .total {
    margin-left: auto;
    color: #8a93a5;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
  .enrich {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
    margin: 0.2rem 0 0.4rem;
  }
  .enrich .hint {
    margin: 0;
  }
  button.ghost {
    background: transparent;
    color: #b6bdc9;
    border: 1px solid #232733;
    font-size: 0.8rem;
    padding: 0.25rem 0.7rem;
  }
  button.ghost:hover:not(:disabled) {
    border-color: #3a445c;
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-bottom: 0.8rem;
  }
  .tab {
    background: transparent;
    color: #8a93a5;
    border: 1px solid transparent;
    font-size: 0.85rem;
    padding: 0.3rem 0.8rem;
  }
  .tab.on {
    background: #1c212c;
    border-color: #232733;
    color: #e7eaf0;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-bottom: 0.5rem;
  }
  .sub {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #8a93a5;
    margin: 1rem 0 0.5rem;
  }
  .chip.static {
    cursor: default;
  }
  .mode {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
    color: #8a93a5;
    margin: 0.5rem 0;
  }
  .mode select {
    background: #0d0f14;
    color: #e7eaf0;
    border: 1px solid #232733;
    border-radius: 6px;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.2rem 0.4rem;
  }
  .group {
    background: #10131a;
    border: 1px solid #232733;
    border-radius: 10px;
    padding: 0.6rem;
    margin-bottom: 0.6rem;
  }
  .group > header {
    display: flex;
    justify-content: space-between;
    font-size: 0.78rem;
    color: #b6bdc9;
    margin-bottom: 0.5rem;
    min-width: 0;
  }
  .waste {
    color: #7b8496;
  }
  .members {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(6rem, 100%), 1fr));
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }
  .members figure {
    min-width: 0;
  }
  .members figure {
    border: 1px solid transparent;
    border-radius: 8px;
    padding: 0.25rem;
  }
  .members figure.keeper {
    border-color: #2f6bff;
    background: rgba(47, 107, 255, 0.08);
  }
  .thumb.small {
    aspect-ratio: 1;
  }
  .event-card .meta {
    margin: 0 0 0.5rem;
  }
  .ename {
    background: transparent;
    border: 0;
    padding: 0;
    color: #e7eaf0;
    font-size: inherit;
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .ename:hover {
    text-decoration: underline;
  }
  .badge.keep {
    display: inline-block;
    font-size: 0.65rem;
    color: #2f6bff;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .row-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-top: 0.2rem;
  }
  button.tiny {
    background: #1c212c;
    color: #b6bdc9;
    border: 1px solid #232733;
    font-size: 0.68rem;
    padding: 0.15rem 0.4rem;
    border-radius: 6px;
  }
  button.tiny.danger {
    color: #e5484d;
  }
  button.danger-solid {
    background: #e5484d;
    color: #fff;
  }
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(5, 7, 10, 0.72);
    display: grid;
    place-items: center;
    padding: 1rem;
  }
  .dialog {
    background: #151820;
    border: 1px solid #232733;
    border-radius: 12px;
    padding: 1rem 1.2rem;
    width: min(24rem, 92vw);
  }
  .dialog h3 {
    margin: 0 0 0.5rem;
    font-size: 0.95rem;
  }
  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.9rem;
  }
  .toast {
    position: fixed;
    bottom: 1rem;
    left: 50%;
    transform: translateX(-50%);
    background: #1c212c;
    border: 1px solid #232733;
    border-radius: 999px;
    padding: 0.35rem 0.9rem;
    font-size: 0.8rem;
    color: #b6bdc9;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(8.5rem, 100%), 1fr));
    gap: 0.6rem;
    margin-top: 0.6rem;
  }
  .grid > figure {
    min-width: 0;
  }
  figure {
    margin: 0;
  }
  .thumb {
    aspect-ratio: 1;
    background: #0d0f14;
    border-radius: 8px;
    overflow: hidden;
    display: grid;
    place-items: center;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .placeholder {
    color: #4a5162;
    font-size: 0.75rem;
    letter-spacing: 0.05em;
  }
  figcaption {
    margin-top: 0.25rem;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: #7b8496;
    font-size: 0.68rem;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
    font-size: 0.9rem;
    color: #b6bdc9;
  }
  .sink {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #555a66;
  }
  .sink.up {
    background: #35c773;
  }
  .sink.down {
    background: #e5484d;
  }
  dl {
    margin: 0;
  }
  .row {
    display: flex;
    justify-content: space-between;
    padding: 0.35rem 0;
    border-top: 1px solid #232733;
    font-size: 0.9rem;
    min-width: 0;
  }
  dt {
    color: #8a93a5;
  }
  dd {
    margin: 0;
    min-width: 0;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error {
    margin: 0;
    color: #e5484d;
    font-size: 0.85rem;
  }
  .global-error {
    background: rgba(229, 72, 77, 0.08);
    border: 1px solid rgba(229, 72, 77, 0.35);
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    margin-bottom: 0.8rem;
  }
</style>
