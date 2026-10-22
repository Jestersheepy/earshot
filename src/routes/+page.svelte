<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, dayLabel, timeOf, appName, type NoteMeta, type SearchHit } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import NoteView from "$lib/NoteView.svelte";
  import History from "$lib/History.svelte";
  import Settings from "$lib/Settings.svelte";

  type Section = "notes" | "meetings" | "voice" | "history" | "settings";
  let section = $state<Section>("notes");
  let notes = $state<NoteMeta[]>([]);
  let loaded = $state(false);
  let query = $state("");
  let hits = $state<SearchHit[] | null>(null);
  let selected = $state<string | null>(null);
  let searchBox: HTMLInputElement;

  const nav: { id: Section; label: string; icon: string }[] = [
    { id: "notes", label: "All notes", icon: "all" },
    { id: "meetings", label: "Meetings", icon: "meeting" },
    { id: "voice", label: "Voice notes", icon: "voice" },
    { id: "history", label: "Dictation history", icon: "history" },
  ];

  const inSection = (m: NoteMeta) =>
    section === "meetings" ? m.kind === "meeting" : section === "voice" ? m.kind !== "meeting" : true;

  let shown = $derived.by(() => {
    const items = hits ? hits.map((h) => ({ meta: h.meta, snippet: h.snippet })) : notes.map((meta) => ({ meta, snippet: "" }));
    return items.filter((i) => inSection(i.meta));
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
    notes: notes.length,
    meetings: notes.filter((n) => n.kind === "meeting").length,
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
      if (section === "history" || section === "settings") section = "notes";
    }, 120);
  }

  function go(s: string) {
    if (["notes", "meetings", "voice", "history", "settings"].includes(s)) section = s as Section;
  }

  function openNote(path: string) {
    query = "";
    hits = null;
    section = "notes";
    selected = path;
  }

  onMount(() => {
    const start = (window as any).__EARSHOT_START__;
    if (start?.event === "open-note") openNote(start.arg);
    else if (start?.arg) go(start.arg);
    refresh();
    const subs = [
      listen("notes-changed", refresh),
      listen<string>("navigate", (e) => go(e.payload)),
      listen<string>("open-note", (e) => refresh().then(() => openNote(e.payload))),
    ];
    const keys = (e: KeyboardEvent) => {
      if ((e.ctrlKey && e.key === "f") || (e.key === "/" && document.activeElement === document.body)) {
        e.preventDefault();
        searchBox?.focus();
      }
    };
    window.addEventListener("keydown", keys);
    return () => {
      subs.forEach((p) => p.then((un) => un()));
      window.removeEventListener("keydown", keys);
    };
  });
</script>

<div class="app">
  <aside class="side">
    <div class="brand">
      <span class="logo"></span>
      <span>Earshot</span>
    </div>
    <label class="search">
      <Icon name="search" size={14} />
      <input bind:this={searchBox} bind:value={query} oninput={onSearch} placeholder="Search notes" spellcheck="false" />
      {#if query}
        <button class="clear" onclick={() => { query = ""; hits = null; }} aria-label="Clear search"><Icon name="x" size={12} /></button>
      {/if}
    </label>
    <nav>
      {#each nav as n (n.id)}
        <button class="nav" class:active={section === n.id} onclick={() => (section = n.id)}>
          <Icon name={n.icon} />
          <span>{n.label}</span>
          {#if n.id !== "history"}<span class="count">{counts[n.id as "notes" | "meetings" | "voice"]}</span>{/if}
        </button>
      {/each}
    </nav>
    <div class="spacer"></div>
    <nav>
      <button class="nav" onclick={() => api.openNotesFolder()}>
        <Icon name="folder" />
        <span>Open notes folder</span>
      </button>
      <button class="nav" class:active={section === "settings"} onclick={() => (section = "settings")}>
        <Icon name="settings" />
        <span>Settings</span>
      </button>
    </nav>
  </aside>

  {#if section === "history"}
    <main class="wide"><History /></main>
  {:else if section === "settings"}
    <main class="wide"><Settings /></main>
  {:else}
    <section class="list">
      <header>
        <h2>{hits ? "Search" : nav.find((n) => n.id === section)?.label}</h2>
        <span class="muted">{shown.length} {shown.length === 1 ? "note" : "notes"}</span>
      </header>
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
              <button class="item" class:selected={selected === meta.path} onclick={() => (selected = meta.path)}>
                <div class="row1">
                  <span class="dot" class:meeting={meta.kind === "meeting"}></span>
                  <span class="title">{meta.title}</span>
                  <span class="time">{timeOf(meta.date)}</span>
                </div>
                <div class="preview">{snippet || meta.preview}</div>
                <div class="tags">
                  {#if meta.duration}<span>{meta.duration}</span>{/if}
                  {#if meta.app}<span>{appName(meta.app)}</span>{/if}
                  {#if meta.words}<span>{meta.words} words</span>{/if}
                </div>
              </button>
            {/each}
          {/each}
        {/if}
      </div>
    </section>
    <main>
      {#if selected}
        {#key selected}
          <NoteView path={selected} onchange={refresh} ondelete={() => { selected = null; refresh(); }} />
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

<style>
  .app {
    display: grid;
    grid-template-columns: 232px 340px 1fr;
    height: 100vh;
  }
  .app:has(main.wide) {
    grid-template-columns: 232px 1fr;
  }
  .side {
    background: var(--side);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 14px 10px;
    gap: 4px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font: 600 15px "Segoe UI Variable Display", "Segoe UI", sans-serif;
    padding: 4px 8px 12px;
  }
  .logo {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: radial-gradient(circle at 35% 30%, #9db8ff, var(--accent) 60%, #3b62d6);
    box-shadow: 0 0 0 3px rgba(91, 140, 255, 0.18);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 8px 0 10px;
    margin-bottom: 10px;
    border-radius: 8px;
    background: var(--raised);
    border: 1px solid var(--border);
    color: var(--faint);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px rgba(91, 140, 255, 0.18);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
  }
  .clear {
    border: 0;
    background: transparent;
    color: var(--faint);
    display: grid;
    padding: 2px;
    border-radius: 4px;
  }
  nav {
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
    border-radius: 7px;
    background: transparent;
    color: var(--muted);
    text-align: left;
  }
  .nav span:first-of-type {
    flex: 1;
  }
  .nav:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nav.active {
    background: var(--selected);
    color: var(--text);
  }
  .nav.active :global(svg) {
    color: var(--accent);
  }
  .count {
    font-size: 12px;
    color: var(--faint);
  }
  .spacer {
    flex: 1;
  }
  .list {
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    background: var(--panel);
    min-width: 0;
  }
  .list header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    padding: 18px 18px 10px;
  }
  .list h2 {
    font-size: 18px;
    font-weight: 600;
  }
  .muted {
    color: var(--faint);
    font-size: 12px;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 8px 16px;
  }
  .day {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--panel);
    padding: 10px 10px 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--faint);
  }
  .item {
    display: block;
    width: 100%;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 10px 10px 10px 12px;
    border-radius: 9px;
    margin-bottom: 2px;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.selected {
    background: var(--selected);
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--note);
    flex: none;
  }
  .dot.meeting {
    background: var(--meeting);
  }
  .title {
    flex: 1;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .time {
    font-size: 12px;
    color: var(--faint);
  }
  .preview {
    margin: 3px 0 0 15px;
    color: var(--muted);
    font-size: 13px;
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .tags {
    display: flex;
    gap: 10px;
    margin: 5px 0 0 15px;
    font-size: 11.5px;
    color: var(--faint);
  }
  main {
    min-width: 0;
    overflow: hidden;
    background: var(--bg);
  }
</style>
