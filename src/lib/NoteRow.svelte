<script lang="ts">
  import { timeOf, appName, type NoteMeta } from "$lib/api";

  let {
    meta,
    snippet = "",
    when = "",
    selected = false,
    onclick,
  }: { meta: NoteMeta; snippet?: string; when?: string; selected?: boolean; onclick: () => void } = $props();
</script>

<button class="row" class:selected {onclick}>
  <div class="top">
    <span class="kind-dot" class:meeting={meta.kind === "meeting"}></span>
    <span class="title">{meta.title}</span>
    <span class="time">{when || timeOf(meta.date)}</span>
  </div>
  {#if snippet || meta.preview}<div class="preview">{snippet || meta.preview}</div>{/if}
  <div class="tags">
    {#if meta.duration}<span>{meta.duration}</span>{/if}
    {#if meta.app}<span>{appName(meta.app)}</span>{/if}
    {#if meta.words}<span>{meta.words} words</span>{/if}
    {#each meta.tags as tag}<span>{tag}</span>{/each}
    {#if meta.openTasks}<span>{meta.openTasks} open {meta.openTasks === 1 ? "task" : "tasks"}</span>{/if}
  </div>
</button>

<style>
  .row {
    display: block;
    width: 100%;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 10px 12px;
    border-radius: 10px;
    transition: background 0.15s;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.selected {
    background: var(--accent-soft);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
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
    font-variant-numeric: tabular-nums;
  }
  .preview {
    margin: 2px 0 0 15px;
    color: var(--muted);
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .preview :global(mark) {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-radius: 3px;
  }
  .tags {
    display: flex;
    gap: 10px;
    margin: 4px 0 0 15px;
    font-size: 11.5px;
    color: var(--faint);
  }
  .tags:empty {
    display: none;
  }
</style>
