<script lang="ts">
  import { onMount } from "svelte";
  import { api, audioUrl, clock, longDate, appName, type NoteDetail } from "$lib/api";
  import { markdown } from "$lib/md";
  import Icon from "$lib/Icon.svelte";

  let { path, onchange, ondelete }: { path: string; onchange: () => void; ondelete: () => void } = $props();

  let note = $state<NoteDetail | null>(null);
  let error = $state("");
  let editing = $state(false);
  let title = $state("");
  let summarizing = $state(false);
  let confirmDelete = $state(false);
  let copied = $state(false);

  let audio = $state<HTMLAudioElement>();
  let playing = $state(false);
  let now = $state(0);
  let total = $state(0);

  let isMeeting = $derived(note?.meta.kind === "meeting");
  let active = $derived.by(() => {
    if (!note || !playing && now === 0) return -1;
    let i = -1;
    for (const [k, l] of note.lines.entries()) if (l.t <= now + 0.25) i = k;
    return i;
  });

  async function load() {
    try {
      note = await api.getNote(path);
      title = note.meta.title;
    } catch (e) {
      error = String(e);
    }
  }
  onMount(load);

  async function saveTitle() {
    editing = false;
    if (!note || !title.trim() || title === note.meta.title) return;
    await api.renameNote(path, title.trim());
    await load();
    onchange();
  }

  async function summarize() {
    summarizing = true;
    error = "";
    try {
      await api.resummarize(path);
      await load();
      onchange();
    } catch (e) {
      error = String(e);
    } finally {
      summarizing = false;
    }
  }

  async function remove() {
    if (!confirmDelete) {
      confirmDelete = true;
      setTimeout(() => (confirmDelete = false), 3000);
      return;
    }
    await api.deleteNote(path);
    ondelete();
  }

  function asMarkdown() {
    if (!note) return "";
    const parts = [`# ${note.meta.title}`, longDate(note.meta.date)];
    if (note.summary) parts.push(note.summary);
    if (note.lines.length) parts.push(note.lines.map((l) => `**[${clock(l.t)}] ${l.speaker}:** ${l.text}`).join("\n\n"));
    if (note.body) parts.push(note.body);
    return parts.join("\n\n");
  }

  async function copy() {
    await navigator.clipboard.writeText(asMarkdown());
    copied = true;
    setTimeout(() => (copied = false), 1400);
  }

  function seek(t: number) {
    if (!audio) return;
    audio.currentTime = t;
    audio.play();
  }
  function toggle() {
    if (!audio) return;
    if (audio.paused) audio.play();
    else audio.pause();
  }
  function scrub(e: MouseEvent) {
    const bar = e.currentTarget as HTMLElement;
    const r = bar.getBoundingClientRect();
    if (audio && total) audio.currentTime = ((e.clientX - r.left) / r.width) * total;
  }
</script>

{#if error && !note}
  <div class="empty"><strong>Couldn’t open this note</strong><span>{error}</span></div>
{:else if note}
  <article>
    <header>
      <span class="chip" class:meeting={isMeeting}>{isMeeting ? "Meeting" : "Voice note"}</span>
      {#if editing}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="title-input"
          bind:value={title}
          autofocus
          onblur={saveTitle}
          onkeydown={(e) => {
            if (e.key === "Enter") saveTitle();
            if (e.key === "Escape") { title = note!.meta.title; editing = false; }
          }}
        />
      {:else}
        <h1>
          <button class="title-btn" onclick={() => (editing = true)} title="Rename">
            {note.meta.title}
            <span class="pen"><Icon name="edit" size={14} /></span>
          </button>
        </h1>
      {/if}
      <div class="meta">
        <span>{longDate(note.meta.date)}</span>
        {#if note.meta.duration}<span>{note.meta.duration}</span>{/if}
        {#if note.meta.app}<span>{appName(note.meta.app)}</span>{/if}
        <span>{note.meta.words} words</span>
      </div>
      <div class="actions">
        <button class="btn" onclick={copy}><Icon name={copied ? "check" : "copy"} size={14} />{copied ? "Copied" : "Copy"}</button>
        {#if isMeeting}
          <button class="btn" onclick={summarize} disabled={summarizing || note.meta.words < 40}>
            <Icon name="sparkle" size={14} />{summarizing ? "Summarizing…" : note.summary ? "Re-summarize" : "Summarize"}
          </button>
        {/if}
        <button class="btn" onclick={() => api.reveal(path)}><Icon name="reveal" size={14} />Show in folder</button>
        <button class="btn danger" class:confirm={confirmDelete} onclick={remove}>
          <Icon name="trash" size={14} />{confirmDelete ? "Move to Recycle Bin?" : "Delete"}
        </button>
      </div>
    </header>

    {#if error}<div class="error">{error}</div>{/if}

    {#if note.meta.audio}
      <div class="player">
        <button class="play" onclick={toggle} aria-label={playing ? "Pause" : "Play"}>
          <Icon name={playing ? "pause" : "play"} size={14} />
        </button>
        <span class="clock">{clock(now)}</span>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="bar" onclick={scrub}>
          <div class="fill" style:width="{total ? (now / total) * 100 : 0}%"></div>
        </div>
        <span class="clock">{clock(total)}</span>
        <audio
          bind:this={audio}
          src={audioUrl(note.meta.audio)}
          preload="metadata"
          bind:currentTime={now}
          bind:duration={total}
          onplay={() => (playing = true)}
          onpause={() => (playing = false)}
          onended={() => (playing = false)}
        ></audio>
      </div>
    {/if}

    {#if isMeeting}
      <section class="card summary selectable">
        <h3><Icon name="sparkle" size={14} /> Summary</h3>
        {#if summarizing}
          <div class="shimmer"><div></div><div></div><div></div></div>
        {:else if note.summary}
          {@html markdown(note.summary)}
        {:else}
          <p class="faint">
            {note.meta.words < 40 ? "Too short to summarize." : "No summary yet. Summaries run locally and take about a minute."}
          </p>
        {/if}
      </section>
    {/if}

    {#if note.lines.length}
      <section class="transcript selectable">
        <h3>Transcript</h3>
        {#each note.lines as line, i (i)}
          <div class="line" class:active={i === active}>
            <button class="t" onclick={() => seek(line.t)} disabled={!note.meta.audio}>{clock(line.t)}</button>
            <div>
              <span class="who" class:me={line.speaker === "Me"} class:others={line.speaker === "Others"}>{line.speaker}</span>
              <p>{line.text}</p>
            </div>
          </div>
        {/each}
      </section>
    {/if}

    {#if note.body}
      <section class="body selectable">
        {#each note.body.split(/\n\s*\n/) as para}
          <p>{para}</p>
        {/each}
      </section>
    {/if}
  </article>
{/if}

<style>
  article {
    height: 100%;
    overflow-y: auto;
    padding: 28px 40px 60px;
    max-width: 880px;
    margin: 0 auto;
  }
  header {
    margin-bottom: 18px;
  }
  .chip {
    display: inline-block;
    font-size: 11.5px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 20px;
    color: #b7700a;
    background: rgba(245, 165, 36, 0.14);
    margin-bottom: 8px;
  }
  .chip.meeting {
    color: var(--meeting);
    background: rgba(240, 74, 74, 0.12);
  }
  h1 {
    font-size: 26px;
    font-weight: 600;
    line-height: 1.25;
  }
  .title-btn {
    all: unset;
    cursor: text;
    border-radius: 6px;
  }
  .pen {
    opacity: 0;
    color: var(--faint);
    margin-left: 6px;
    transition: opacity 0.15s;
  }
  .title-btn:hover .pen {
    opacity: 1;
  }
  .title-input {
    width: 100%;
    font: 600 26px "Segoe UI Variable Display", "Segoe UI", sans-serif;
    border: 0;
    border-bottom: 2px solid var(--accent);
    background: transparent;
    outline: 0;
    padding: 0;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
    color: var(--muted);
    font-size: 13px;
    margin-top: 6px;
  }
  .meta span + span::before {
    content: "·";
    margin-right: 14px;
    color: var(--faint);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 16px;
  }
  .btn.confirm {
    color: white;
    background: var(--danger);
    border-color: transparent;
  }
  .error {
    padding: 10px 14px;
    border-radius: 8px;
    margin-bottom: 14px;
    background: rgba(229, 72, 77, 0.1);
    color: var(--danger);
  }
  .player {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-radius: 12px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    margin-bottom: 18px;
    position: sticky;
    top: 0;
    z-index: 2;
  }
  .play {
    width: 32px;
    height: 32px;
    flex: none;
    border-radius: 50%;
    border: 0;
    background: var(--accent);
    color: white;
    display: grid;
    place-items: center;
  }
  .clock {
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    min-width: 34px;
  }
  .bar {
    flex: 1;
    height: 6px;
    border-radius: 6px;
    background: var(--border-strong);
    cursor: pointer;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 6px;
  }
  .card {
    padding: 18px 22px;
    border-radius: 14px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    margin-bottom: 26px;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 10px;
  }
  .summary :global(h4) {
    font-size: 14.5px;
    font-weight: 600;
    margin: 14px 0 4px;
  }
  .summary :global(h4:first-of-type) {
    margin-top: 0;
  }
  .summary :global(p) {
    margin: 0 0 6px;
  }
  .summary :global(ul) {
    margin: 0 0 6px;
    padding-left: 20px;
  }
  .summary :global(li) {
    margin: 2px 0;
  }
  .summary :global(li::marker) {
    color: var(--accent);
  }
  .faint {
    color: var(--faint);
    margin: 0;
  }
  .shimmer div {
    height: 12px;
    border-radius: 6px;
    margin: 10px 0;
    background: linear-gradient(90deg, var(--hover), var(--border-strong), var(--hover));
    background-size: 200% 100%;
    animation: shimmer 1.4s infinite linear;
  }
  .shimmer div:nth-child(2) {
    width: 85%;
  }
  .shimmer div:nth-child(3) {
    width: 60%;
  }
  @keyframes shimmer {
    to {
      background-position: -200% 0;
    }
  }
  .line {
    display: grid;
    grid-template-columns: 56px 1fr;
    gap: 6px;
    padding: 8px 10px;
    margin: 0 -10px;
    border-radius: 9px;
    transition: background 0.2s;
  }
  .line.active {
    background: var(--selected);
  }
  .t {
    border: 0;
    background: transparent;
    color: var(--faint);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    padding: 2px 0 0;
    text-align: left;
    height: fit-content;
  }
  .t:not(:disabled):hover {
    color: var(--accent-text);
    text-decoration: underline;
  }
  .who {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--muted);
  }
  .who.me {
    color: var(--accent-text);
  }
  .who.others {
    color: var(--others);
  }
  .line p,
  .body p {
    margin: 2px 0 0;
    line-height: 1.6;
  }
  .body p {
    margin-bottom: 12px;
    font-size: 15px;
  }
</style>
