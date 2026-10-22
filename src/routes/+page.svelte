<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, applyLook, dayLabel, type NoteMeta, type PanelMode, type SearchHit } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import Home from "$lib/Home.svelte";
  import NoteRow from "$lib/NoteRow.svelte";
  import NoteView from "$lib/NoteView.svelte";
  import History from "$lib/History.svelte";
  import Settings from "$lib/Settings.svelte";

  type Section = "home" | "notes" | "history" | "settings";
  type Filter = "all" | "meeting" | "voice";

  let width = $state(400);
  let wide = $derived(width >= 720);
  let section = $state<Section>("home");
  let filter = $state<Filter>("all");
  let notes = $state<NoteMeta[]>([]);
  let loaded = $state(false);
  let query = $state("");
  let hits = $state<SearchHit[] | null>(null);
  let selected = $state<string | null>(null);
  let pinned = $state(false);
  let origin = $state<PanelMode["origin"]>("none");
  let opened = $state(0);
  let searchBox = $state<HTMLInputElement>();

  const filters: { id: Filter; label: string }[] = [
    { id: "all", label: "All" },
    { id: "meeting", label: "Meetings" },
    { id: "voice", label: "Voice notes" },
  ];
  const tabs: { id: Section; label: string; icon: string }[] = [
    { id: "home", label: "Home", icon: "home" },
    { id: "notes", label: "Notes", icon: "all" },
    { id: "history", label: "History", icon: "history" },
    { id: "settings", label: "Settings", icon: "settings" },
  ];

  const matches = (m: NoteMeta) => (filter === "meeting" ? m.kind === "meeting" : filter === "voice" ? m.kind !== "meeting" : true);
  let shown = $derived.by(() => {
    const items = hits ? hits.map((h) => ({ meta: h.meta, snippet: h.snippet })) : notes.map((meta) => ({ meta, snippet: "" }));
    return items.filter((i) => matches(i.meta));
  });
  let groups = $derived.by(() => {
    const out: { label: string; items: typeof shown }[] = [];
    for (const item of shown) {
      const label = dayLabel(item.meta.date);
      if (out.at(-1)?.label !== label) out.push({ label, items: [] });
      out.at(-1)!.items.push(item);
    }
    return out;
  });
  let counts = $derived({
    all: notes.length,
    meeting: notes.filter((n) => n.kind === "meeting").length,
    voice: notes.filter((n) => n.kind !== "meeting").length,
  });

  async function refresh() {
    notes = await api.listNotes();
    loaded = true;
    if (query.trim()) hits = await api.searchNotes(query);
    if (selected && !notes.some((n) => n.path === selected)) selected = null;
  }

  let searchTimer: ReturnType<typeof setTimeout>;
  function onSearch() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(async () => {
      hits = query.trim() ? await api.searchNotes(query) : null;
      if (hits) {
        section = "notes";
        if (!wide) selected = null;
      }
    }, 120);
  }
  function clearSearch() {
    query = "";
    hits = null;
  }

  /** Sections as the app names them: the dot's menu, the tray, toasts. */
  function go(s: string) {
    selected = wide ? selected : null;
    if (s === "meetings" || s === "voice" || s === "notes") {
      section = "notes";
      filter = s === "meetings" ? "meeting" : s === "voice" ? "voice" : "all";
    } else if (s === "home" || s === "history" || s === "settings") section = s;
  }

  function openNote(path: string) {
    clearSearch();
    section = "notes";
    filter = "all";
    selected = path;
  }

  async function loadLook() {
    try {
      applyLook(await api.getSettings());
    } catch {}
  }

  onMount(() => {
    const start = (window as any).__EARSHOT_START__;
    if (start?.event === "open-note") openNote(start.arg);
    else if (start?.arg) go(start.arg);
    loadLook();
    refresh();
    const subs = [
      listen("notes-changed", refresh),
      listen<string>("navigate", (e) => go(e.payload)),
      listen<string>("open-note", (e) => refresh().then(() => openNote(e.payload))),
      listen<PanelMode>("panel-mode", (e) => {
        pinned = e.payload.pinned;
        origin = e.payload.expanded ? "none" : e.payload.origin;
        opened++;
      }),
      listen("panel-hidden", () => {
        if (!wide) selected = null;
      }),
    ];
    const keys = (e: KeyboardEvent) => {
      if ((e.ctrlKey && e.key === "f") || (e.key === "/" && document.activeElement === document.body)) {
        e.preventDefault();
        searchBox?.focus();
      } else if (e.key === "Escape" && document.activeElement?.tagName !== "INPUT") {
        if (!wide && selected) selected = null;
        else if (!wide) api.panel.close();
      }
    };
    window.addEventListener("keydown", keys);
    return () => {
      subs.forEach((p) => p.then((un) => un()));
      window.removeEventListener("keydown", keys);
    };
  });

  function togglePin() {
    pinned = !pinned;
    api.panel.pin(pinned);
  }
</script>

<svelte:window bind:innerWidth={width} />

{#snippet list()}
  {#if wide}
    <div class="list-head">{hits ? "Search" : filter === "all" ? "All notes" : filters.find((f) => f.id === filter)?.label}</div>
  {:else}
    <div class="filters">
      {#each filters as f (f.id)}
        <button class:on={filter === f.id} onclick={() => (filter = f.id)}>
          {f.label}<span>{counts[f.id]}</span>
        </button>
      {/each}
    </div>
  {/if}
  <div class="scroll">
    {#if !loaded}
      <div></div>
    {:else if shown.length === 0}
      <div class="empty">
        {#if hits}
          <strong>No matches</strong>
          <span>Nothing in your notes mentions “{query}”.</span>
        {:else}
          <strong>Nothing here yet</strong>
          <span>Record a voice note with Ctrl+Alt+N,<br />or a meeting with Ctrl+Alt+M.</span>
        {/if}
      </div>
    {:else}
      {#each groups as g (g.label)}
        <div class="day">{g.label}</div>
        {#each g.items as { meta, snippet } (meta.path)}
          <NoteRow {meta} {snippet} selected={wide && selected === meta.path} onclick={() => (selected = meta.path)} />
        {/each}
      {/each}
    {/if}
  </div>
{/snippet}

{#snippet search()}
  <label class="search">
    <Icon name="search" size={14} />
    <input bind:this={searchBox} bind:value={query} oninput={onSearch} placeholder="Search notes" spellcheck="false" />
    {#if query}
      <button class="clear" onclick={clearSearch} aria-label="Clear search"><Icon name="x" size={12} /></button>
    {:else}
      <kbd>Ctrl F</kbd>
    {/if}
  </label>
{/snippet}

{#key opened}
  <div class="app origin-{origin}" class:wide>
    <header class="bar" data-tauri-drag-region>
      {#if !wide && selected}
        <button class="icon-btn" onclick={() => (selected = null)} aria-label="Back"><Icon name="back" /></button>
        <span class="crumb" data-tauri-drag-region>{hits ? "Search" : filter === "all" ? "Notes" : filters.find((f) => f.id === filter)?.label}</span>
      {:else}
        <span class="logo" data-tauri-drag-region></span>
        <span class="brand" data-tauri-drag-region>Earshot</span>
      {/if}
      <div class="spacer" data-tauri-drag-region></div>
      {#if !wide}
        <button class="icon-btn" class:on={pinned} onclick={togglePin} title={pinned ? "Unpin" : "Keep open"}><Icon name="pin" size={15} /></button>
      {/if}
      <button class="icon-btn" onclick={() => api.panel.expand(!wide)} title={wide ? "Back to the dot" : "Expand"}>
        <Icon name={wide ? "collapse" : "expand"} size={15} />
      </button>
      {#if wide}
        <button class="icon-btn" onclick={() => api.panel.minimize()} title="Minimise"><Icon name="minimize" size={15} /></button>
      {/if}
      <button class="icon-btn close" onclick={() => api.panel.close()} title="Close"><Icon name="x" size={15} /></button>
    </header>

    {#if wide}
      <div class="cols" class:two={section !== "notes"}>
        <aside class="side">
          {@render search()}
          <nav>
            <button class="nav" class:active={section === "home"} onclick={() => (section = "home")}><Icon name="home" /><span>Home</span></button>
            {#each filters as f (f.id)}
              <button class="nav" class:active={section === "notes" && filter === f.id} onclick={() => ((section = "notes"), (filter = f.id))}>
                <Icon name={f.id === "meeting" ? "meeting" : f.id === "voice" ? "voice" : "all"} />
                <span>{f.id === "all" ? "All notes" : f.label}</span>
                <span class="count">{counts[f.id]}</span>
              </button>
            {/each}
            <button class="nav" class:active={section === "history"} onclick={() => (section = "history")}><Icon name="history" /><span>Dictation history</span></button>
          </nav>
          <div class="spacer"></div>
          <nav>
            <button class="nav" onclick={() => api.openNotesFolder()}><Icon name="folder" /><span>Notes folder</span></button>
            <button class="nav" class:active={section === "settings"} onclick={() => (section = "settings")}><Icon name="settings" /><span>Settings</span></button>
          </nav>
        </aside>
        {#if section === "home"}
          <main class="page"><div class="narrow"><Home {notes} onopen={openNote} onall={() => (section = "notes")} /></div></main>
        {:else if section === "history"}
          <main class="page"><History /></main>
        {:else if section === "settings"}
          <main class="page"><Settings /></main>
        {:else}
          <section class="list">{@render list()}</section>
          <main class="reader">
            {#if selected}
              {#key selected}
                <NoteView path={selected} onchange={refresh} ondelete={() => ((selected = null), refresh())} />
              {/key}
            {:else}
              <div class="empty">
                <strong>Pick a note</strong>
                <span>Summaries, transcripts and recordings show up here.</span>
              </div>
            {/if}
          </main>
        {/if}
      </div>
    {:else if selected}
      <main class="pushed">
        {#key selected}
          <NoteView path={selected} compact onchange={refresh} ondelete={() => ((selected = null), refresh())} />
        {/key}
      </main>
    {:else}
      <div class="compact-top">
        {@render search()}
        <nav class="tabs">
          {#each tabs as t (t.id)}
            <button class:on={section === t.id} onclick={() => (section = t.id)}>
              <Icon name={t.icon} size={15} /><span>{t.label}</span>
            </button>
          {/each}
        </nav>
      </div>
      <main class="body">
        {#if section === "home"}
          <Home {notes} onopen={openNote} onall={() => (section = "notes")} />
        {:else if section === "history"}
          <History />
        {:else if section === "settings"}
          <Settings />
        {:else}
          <div class="listwrap">{@render list()}</div>
        {/if}
      </main>
    {/if}
  </div>
{/key}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--bg);
    animation: open 0.2s var(--ease);
  }
  .app.origin-bottom {
    transform-origin: 50% 100%;
  }
  .app.origin-top {
    transform-origin: 50% 0;
  }
  .app.origin-left {
    transform-origin: 0 50%;
  }
  .app.origin-right {
    transform-origin: 100% 50%;
  }
  .app.origin-none {
    animation: none;
  }
  @keyframes open {
    from {
      opacity: 0;
      transform: scale(0.96);
    }
  }

  /* ---- title bar ---- */
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    height: 44px;
    padding: 0 8px 0 14px;
  }
  .app.wide .bar {
    height: 40px;
    border-bottom: 1px solid var(--rule);
    background: var(--surface);
  }
  .logo {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: radial-gradient(circle at 35% 30%, color-mix(in srgb, var(--accent) 45%, white), var(--accent) 65%);
    box-shadow: 0 0 0 3px var(--accent-soft);
    margin-right: 9px;
  }
  .brand {
    font: 600 14px "Segoe UI Variable Display", "Segoe UI", sans-serif;
    letter-spacing: -0.005em;
  }
  .crumb {
    font-weight: 600;
    margin-left: 4px;
  }
  .bar .icon-btn:first-child {
    margin-left: -8px;
  }
  .spacer {
    flex: 1;
    align-self: stretch;
  }
  .close:hover {
    background: var(--danger);
    color: white;
  }

  /* ---- search ---- */
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 8px 0 11px;
    border-radius: 10px;
    background: var(--surface);
    border: 1px solid transparent;
    color: var(--faint);
    transition: border-color 0.15s, box-shadow 0.15s, background 0.15s;
  }
  .search:focus-within {
    background: var(--raised);
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--text);
  }
  .search kbd {
    font: 11px "Segoe UI", sans-serif;
    color: var(--faint);
  }
  .clear {
    border: 0;
    background: transparent;
    color: var(--faint);
    display: grid;
    padding: 3px;
    border-radius: 5px;
  }

  /* ---- compact ---- */
  .compact-top {
    flex: none;
    padding: 0 12px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin-top: 8px;
    border-bottom: 1px solid var(--rule);
  }
  .tabs button {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 36px;
    border: 0;
    background: transparent;
    color: var(--muted);
    font-size: 12.5px;
    position: relative;
    transition: color 0.15s;
  }
  .tabs button:hover {
    color: var(--text);
  }
  .tabs button.on {
    color: var(--text);
    font-weight: 600;
  }
  .tabs button.on::after {
    content: "";
    position: absolute;
    left: 22%;
    right: 22%;
    bottom: -1px;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .listwrap {
    height: 100%;
    display: flex;
    flex-direction: column;
    animation: fade-in 0.2s var(--ease);
  }
  .pushed {
    flex: 1;
    min-height: 0;
    animation: push-in 0.22s var(--ease);
  }

  /* ---- list ---- */
  .filters {
    flex: none;
    display: flex;
    gap: 6px;
    padding: 12px 14px 4px;
  }
  .filters button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 10px;
    border-radius: 13px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--muted);
    font-size: 12px;
    transition: background 0.15s, color 0.15s, border-color 0.15s;
  }
  .filters button span {
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
  .filters button:hover {
    color: var(--text);
  }
  .filters button.on {
    background: var(--text);
    border-color: var(--text);
    color: var(--bg);
  }
  .filters button.on span {
    color: color-mix(in srgb, var(--bg) 65%, transparent);
  }
  .list-head {
    flex: none;
    padding: 16px 18px 2px;
    font: 600 15px "Segoe UI Variable Display", "Segoe UI", sans-serif;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 6px 16px;
  }
  .day {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--bg);
    padding: 12px 12px 4px;
    font-size: 12px;
    font-weight: 600;
    color: var(--faint);
  }

  /* ---- expanded ---- */
  .cols {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 220px 320px 1fr;
  }
  .cols.two {
    grid-template-columns: 220px 1fr;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 10px;
    background: var(--surface);
    border-right: 1px solid var(--rule);
  }
  .side .search {
    background: var(--raised);
    border-color: var(--border);
  }
  .side nav {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .nav {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--muted);
    text-align: left;
    transition: background 0.15s, color 0.15s;
  }
  .nav span:first-of-type {
    flex: 1;
  }
  .nav:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nav.active {
    background: var(--raised);
    color: var(--text);
    box-shadow: var(--shadow-sm);
  }
  .nav.active :global(svg) {
    color: var(--accent);
  }
  .count {
    font-size: 12px;
    color: var(--faint);
  }
  .side .spacer {
    flex: 1;
  }
  .list {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border-right: 1px solid var(--rule);
  }
  .reader,
  .page {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .narrow {
    height: 100%;
    max-width: 680px;
    margin: 0 auto;
    padding-top: 12px;
  }
</style>
