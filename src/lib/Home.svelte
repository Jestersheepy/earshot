<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, minutesSaved, type NoteMeta, type Stats } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import NoteRow from "$lib/NoteRow.svelte";

  let { notes, onopen, onall }: { notes: NoteMeta[]; onopen: (path: string) => void; onall: () => void } = $props();

  let stats = $state<Stats | null>(null);
  const load = async () => (stats = await api.historyStats());
  onMount(() => {
    load();
    const un = listen("history-changed", load);
    return () => un.then((f) => f());
  });

  const hour = new Date().getHours();
  const greeting = hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
  const today = new Date().toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
  const fmt = (n: number) => n.toLocaleString();
  const saved = (words: number) => {
    const m = minutesSaved(words);
    return m >= 60 ? `${Math.floor(m / 60)} h ${m % 60} m` : `${m} min`;
  };
  let recent = $derived(notes.slice(0, 6));
</script>

<div class="home">
  <header>
    <h1>{greeting}</h1>
    <p>{today}</p>
  </header>

  <div class="stats">
    <div class="stat">
      <span class="v">{stats ? fmt(stats.today_words) : "–"}</span>
      <span class="k">words today</span>
    </div>
    <div class="stat">
      <span class="v">{stats ? fmt(stats.week_words) : "–"}</span>
      <span class="k">this week</span>
    </div>
    <div class="stat">
      <span class="v">{stats ? saved(stats.week_words) : "–"}</span>
      <span class="k">saved this week</span>
    </div>
  </div>

  <div class="quick">
    <button class="action" onclick={() => api.runAction("note")}>
      <span class="ic note"><Icon name="voice" size={15} /></span>
      <span><strong>Voice note</strong><small>Think out loud, get a tidy note</small></span>
    </button>
    <button class="action" onclick={() => api.runAction("meeting")}>
      <span class="ic meeting"><Icon name="meeting" size={15} /></span>
      <span><strong>Meeting</strong><small>Transcript, speakers and summary</small></span>
    </button>
  </div>

  <section>
    <div class="head">
      <h2>Recent</h2>
      {#if notes.length > recent.length}<button class="link" onclick={onall}>All notes <Icon name="chevron" size={12} /></button>{/if}
    </div>
    {#if recent.length}
      <div class="list">
        {#each recent as meta (meta.path)}
          <NoteRow {meta} onclick={() => onopen(meta.path)} />
        {/each}
      </div>
    {:else}
      <p class="none">Nothing yet. Hold Ctrl+Win anywhere to dictate, or start a voice note above.</p>
    {/if}
  </section>
</div>

<style>
  .home {
    height: 100%;
    overflow-y: auto;
    padding: 18px 16px 28px;
    animation: fade-in 0.22s var(--ease);
  }
  header {
    padding: 0 4px;
  }
  h1 {
    font-size: 20px;
    letter-spacing: -0.01em;
  }
  header p {
    margin: 2px 0 0;
    color: var(--muted);
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    margin: 16px 0 14px;
    border-radius: var(--radius);
    background: var(--surface);
  }
  .stat {
    display: flex;
    flex-direction: column;
    padding: 12px 14px;
  }
  .stat + .stat {
    border-left: 1px solid var(--rule);
  }
  .v {
    font: 600 20px "Segoe UI Variable Display", "Segoe UI", sans-serif;
    letter-spacing: -0.01em;
    font-variant-numeric: tabular-nums;
  }
  .k {
    color: var(--muted);
    font-size: 12px;
  }
  .quick {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .action {
    display: flex;
    align-items: center;
    gap: 10px;
    text-align: left;
    padding: 10px 12px;
    border-radius: var(--radius);
    border: 1px solid var(--border);
    background: var(--raised);
    box-shadow: var(--shadow-sm);
    transition: border-color 0.15s, transform 0.15s var(--ease);
  }
  .action:hover {
    border-color: var(--border-strong);
  }
  .action:active {
    transform: scale(0.985);
  }
  .action strong {
    display: block;
    font-weight: 600;
  }
  .action small {
    display: block;
    color: var(--muted);
    font-size: 11.5px;
    line-height: 1.35;
  }
  .ic {
    flex: none;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 9px;
  }
  .ic.note {
    color: var(--note);
    background: color-mix(in srgb, var(--note) 14%, transparent);
  }
  .ic.meeting {
    color: var(--meeting);
    background: color-mix(in srgb, var(--meeting) 12%, transparent);
  }
  section {
    margin-top: 20px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 4px 4px;
  }
  h2 {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: 12px;
    padding: 2px 4px;
    border-radius: 5px;
  }
  .link:hover {
    background: var(--accent-soft);
  }
  .list {
    margin: 0 -8px;
  }
  .none {
    color: var(--muted);
    padding: 8px 4px;
    margin: 0;
  }
</style>
