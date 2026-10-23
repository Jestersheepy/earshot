<script lang="ts">
  import { onMount } from "svelte";
  import { api, clock, dayLabel, dueDate, type TaskRow } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  let { onopen }: { onopen: (path: string) => void } = $props();

  let rows = $state<TaskRow[]>([]);
  let loaded = $state(false);
  let showDone = $state(false);
  let me = $state("Me");

  async function load() {
    rows = await api.listTasks();
    loaded = true;
  }
  onMount(() => {
    load();
    api.getSettings().then((s) => (me = s.config.your_name || "Me"), () => {});
  });

  const today = new Date().toISOString().slice(0, 10);
  const mine = (o: string | null) => o === me || o === "Me";

  /** Yours first, then each person, then the ones nobody took; soonest due first within each. */
  let groups = $derived.by(() => {
    const by = new Map<string, TaskRow[]>();
    for (const r of rows) {
      if (r.task.done && !showDone) continue;
      const key = mine(r.task.owner) ? me : (r.task.owner ?? "");
      by.set(key, [...(by.get(key) ?? []), r]);
    }
    const order = (k: string) => (k === me ? 0 : k === "" ? 2 : 1);
    return [...by.entries()]
      .sort(([a], [b]) => order(a) - order(b) || a.localeCompare(b))
      .map(([owner, items]) => ({
        owner,
        items: items.sort((x, y) => Number(x.task.done) - Number(y.task.done) || (x.task.due ?? "9").localeCompare(y.task.due ?? "9")),
      }));
  });
  let open = $derived(rows.filter((r) => !r.task.done).length);

  async function tick(r: TaskRow, done: boolean) {
    r.task.done = done;
    await api.setTask(r.note.path, r.index, done);
  }
</script>

<div class="wrap">
  <header>
    <div>
      <h2>Tasks</h2>
      <p class="sub">{open} open, from your meetings and voice notes. Ticking one ticks it in the note.</p>
    </div>
    <label class="toggle"><input type="checkbox" bind:checked={showDone} />Show done</label>
  </header>

  <div class="scroll">
    {#if loaded && groups.length === 0}
      <div class="empty">
        <strong>{rows.length ? "All done" : "No tasks yet"}</strong>
        <span>Action items from meetings, and voice notes that sound like a to-do, land here.</span>
      </div>
    {/if}
    {#each groups as g (g.owner)}
      <div class="day">{g.owner || "Unassigned"}</div>
      <div class="group">
        {#each g.items as r (r.note.path + r.index)}
          <div class="row" class:done={r.task.done}>
            <input type="checkbox" checked={r.task.done} onchange={(e) => tick(r, e.currentTarget.checked)} />
            <div class="what">
              <span class="text selectable">{r.task.text}</span>
              <button class="from" onclick={() => onopen(r.note.path)}>
                {r.note.title} · {dayLabel(r.note.date)}{#if r.task.t != null}&nbsp;· {clock(r.task.t)}{/if}
              </button>
            </div>
            {#if r.task.due}
              <button class="due" class:late={!r.task.done && r.task.due < today} onclick={() => api.taskToCalendar(r.note.path, r.index)} title="Add to calendar">
                <Icon name="calendar" size={12} />{dueDate(r.task.due)}
              </button>
            {/if}
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
  @media (max-width: 560px) {
    .wrap {
      padding: 0 12px;
    }
    header {
      padding: 14px 0 10px;
    }
    h2 {
      font-size: 18px;
    }
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12.5px;
    white-space: nowrap;
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
    grid-template-columns: auto 1fr auto;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 14px;
  }
  .row + .row {
    border-top: 1px solid var(--border);
  }
  input[type="checkbox"] {
    margin-top: 4px;
    accent-color: var(--accent);
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .text {
    line-height: 1.5;
  }
  .done .text {
    color: var(--faint);
    text-decoration: line-through;
  }
  .from {
    all: unset;
    cursor: pointer;
    font-size: 12px;
    color: var(--faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .from:hover {
    color: var(--accent-text);
  }
  .due {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: transparent;
    border-radius: 999px;
    padding: 1px 8px;
    font-size: 11.5px;
    color: var(--muted);
    white-space: nowrap;
  }
  .due.late {
    color: var(--danger, #c2410c);
    border-color: currentColor;
  }
  .due:hover {
    color: var(--accent-text);
    border-color: var(--accent);
  }
</style>
