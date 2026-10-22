<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, dayLabel, timeOf, appName, type HistoryRow } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  let rows = $state<HistoryRow[]>([]);
  let loaded = $state(false);
  let query = $state("");
  let showRaw = $state<Record<number, boolean>>({});
  let copied = $state<number | null>(null);

  async function load() {
    rows = await api.listHistory(query);
    loaded = true;
  }

  let timer: ReturnType<typeof setTimeout>;
  const onInput = () => {
    clearTimeout(timer);
    timer = setTimeout(load, 120);
  };

  let groups = $derived.by(() => {
    const out: { label: string; items: HistoryRow[] }[] = [];
    for (const r of rows) {
      const label = dayLabel(r.ts);
      if (out.at(-1)?.label !== label) out.push({ label, items: [] });
      out.at(-1)!.items.push(r);
    }
    return out;
  });

  async function copy(r: HistoryRow) {
    await navigator.clipboard.writeText(r.text);
    copied = r.id;
    setTimeout(() => (copied = null), 1200);
  }

  async function remove(r: HistoryRow) {
    await api.deleteHistory(r.id);
    rows = rows.filter((x) => x.id !== r.id);
  }

  onMount(() => {
    load();
    const un = listen("history-changed", load);
    return () => un.then((f) => f());
  });
</script>

<div class="wrap">
  <header>
    <div>
      <h2>Dictation history</h2>
      <p class="sub">Everything you’ve dictated, newest first. Alt+Shift+V pastes the latest.</p>
    </div>
    <label class="filter">
      <Icon name="search" size={14} />
      <input bind:value={query} oninput={onInput} placeholder="Filter by text or app" spellcheck="false" />
    </label>
  </header>

  <div class="scroll">
    {#if loaded && rows.length === 0}
      <div class="empty">
        {#if query}
          <strong>No matches</strong><span>Nothing you dictated matches “{query}”.</span>
        {:else}
          <strong>No dictations yet</strong><span>Hold Ctrl+Win and speak. Text lands wherever your cursor is.</span>
        {/if}
      </div>
    {/if}
    {#each groups as g (g.label)}
      <div class="day">{g.label}</div>
      <div class="group">
        {#each g.items as r (r.id)}
          <div class="row">
            <div class="when">
              <span>{timeOf(r.ts)}</span>
              {#if r.mode === "command"}<span class="tag cmd">Command</span>{:else if r.llm}<span class="tag">Formatted</span>{/if}
            </div>
            <div class="text selectable">
              <div class="final">{r.text}</div>
              {#if showRaw[r.id] && r.raw !== r.text}
                <div class="raw">Heard: {r.raw}</div>
              {/if}
              <div class="info">
                {#if r.app}<span>{appName(r.app)}</span>{/if}
                <span>{(r.ms / 1000).toFixed(1)} s</span>
                {#if r.raw !== r.text}
                  <button class="link" onclick={() => (showRaw[r.id] = !showRaw[r.id])}>{showRaw[r.id] ? "Hide original" : "Show original"}</button>
                {/if}
              </div>
            </div>
            <div class="tools">
              <button class="btn ghost" onclick={() => copy(r)} title="Copy"><Icon name={copied === r.id ? "check" : "copy"} size={14} /></button>
              <button class="btn ghost danger" onclick={() => remove(r)} title="Delete"><Icon name="trash" size={14} /></button>
            </div>
          </div>
        {/each}
      </div>
    {/each}
  </div>
</div>

<style>
  .wrap {
    height: 100%;
    display: flex;
    flex-direction: column;
    max-width: 920px;
    margin: 0 auto;
    padding: 0 32px;
  }
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 20px;
    padding: 28px 0 16px;
  }
  h2 {
    font-size: 22px;
    font-weight: 600;
  }
  .sub {
    margin: 4px 0 0;
    color: var(--muted);
    font-size: 13px;
  }
  /* compact panel */
  @media (max-width: 560px) {
    .wrap {
      padding: 0 12px;
    }
    header {
      flex-direction: column;
      align-items: stretch;
      gap: 10px;
      padding: 14px 0 10px;
    }
    h2 {
      font-size: 18px;
    }
  }
  .filter {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 260px;
    height: 32px;
    padding: 0 10px;
    border-radius: 8px;
    border: 1px solid var(--border-strong);
    background: var(--raised);
    color: var(--faint);
  }
  .filter:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .filter input {
    flex: 1;
    border: 0;
    outline: 0;
    background: transparent;
    min-width: 0;
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding-bottom: 40px;
  }
  .day {
    font-size: 12px;
    font-weight: 600;
    color: var(--faint);
    margin: 18px 0 8px;
  }
  .group {
    border-radius: 12px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .row {
    display: grid;
    grid-template-columns: 92px 1fr auto;
    gap: 12px;
    padding: 12px 14px;
  }
  .row + .row {
    border-top: 1px solid var(--border);
  }
  .row:hover {
    background: var(--hover);
  }
  .when {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: flex-start;
    color: var(--muted);
    font-size: 12.5px;
    font-variant-numeric: tabular-nums;
  }
  .tag {
    font-size: 11px;
    font-weight: 600;
    padding: 0 7px;
    border-radius: 10px;
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .tag.cmd {
    color: var(--command);
    background: rgba(167, 139, 250, 0.15);
  }
  .final {
    white-space: pre-wrap;
    line-height: 1.55;
  }
  .raw {
    margin-top: 6px;
    padding: 6px 10px;
    border-left: 2px solid var(--border-strong);
    color: var(--muted);
    font-size: 13px;
  }
  .info {
    display: flex;
    gap: 12px;
    margin-top: 4px;
    font-size: 12px;
    color: var(--faint);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font-size: 12px;
  }
  .tools {
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity 0.12s;
  }
  .row:hover .tools {
    opacity: 1;
  }
  .tools .btn {
    height: 28px;
    padding: 0 8px;
  }
</style>
