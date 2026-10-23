<script lang="ts">
  import { onMount } from "svelte";
  import { api, audioUrl, clock, longDate, dueDate, appName, speakerColor, initials, type NoteDetail, type SummaryMode, type Task } from "$lib/api";
  import { markdown } from "$lib/md";
  import Icon from "$lib/Icon.svelte";

  let {
    path,
    at = null,
    compact = false,
    onchange,
    ondelete,
  }: { path: string; at?: number | null; compact?: boolean; onchange: () => void; ondelete: () => void } = $props();

  let note = $state<NoteDetail | null>(null);
  let error = $state("");
  let editing = $state(false);
  let title = $state("");
  let summarizing = $state(false);
  let mode = $state<SummaryMode>("standard");
  const MODES: SummaryMode[] = ["concise", "standard", "nuanced"];
  let confirmDelete = $state(false);
  let copied = $state(false);
  let me = $state("Me");

  // speakers
  let naming = $state<string | null>(null);
  let newName = $state("");
  let suggestions = $state<[string, string][] | null>(null);
  let suggesting = $state(false);

  let audio = $state<HTMLAudioElement>();
  let playing = $state(false);
  let now = $state(0);
  let total = $state(0);

  let isMeeting = $derived(note?.meta.kind === "meeting");
  let speakers = $derived.by(() => {
    const seen: string[] = [];
    for (const l of note?.lines ?? []) if (l.speaker && !seen.includes(l.speaker)) seen.push(l.speaker);
    return seen;
  });
  let active = $derived.by(() => {
    if (!note || (!playing && now === 0)) return -1;
    let i = -1;
    for (const [k, l] of note.lines.entries()) if (l.t <= now + 0.25) i = k;
    return i;
  });
  const color = (s: string) => speakerColor(s, speakers, me);
  const unconfirmed = (s: string) => note?.unconfirmed?.includes(s) ?? false;
  const generic = (s: string) => /^Speaker \d+$/.test(s) || s === "Others";

  async function load() {
    try {
      note = await api.getNote(path);
      title = note.meta.title;
      if (!note.summaries[mode]) mode = MODES.find((m) => note!.summaries[m]) ?? mode;
      if (at != null) requestAnimationFrame(() => jump(at!, false));
    } catch (e) {
      error = String(e);
    }
  }
  onMount(() => {
    load();
    api.getSettings().then((s) => (me = s.config.your_name || "Me"), () => {});
  });

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
      await api.resummarize(path, mode);
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

  function startNaming(s: string) {
    naming = naming === s ? null : s;
    newName = generic(s) ? "" : s;
  }

  async function rename(from: string, to: string) {
    to = to.trim();
    if (!to) return;
    error = "";
    try {
      await api.renameSpeaker(path, from, to);
      naming = null;
      if (suggestions) suggestions = suggestions.filter(([label]) => label !== from);
      await load();
      onchange();
    } catch (e) {
      error = String(e);
    }
  }

  async function suggest() {
    suggesting = true;
    error = "";
    try {
      suggestions = (await api.suggestSpeakerNames(path)).filter(([label]) => speakers.includes(label));
    } catch (e) {
      error = String(e);
    } finally {
      suggesting = false;
    }
  }

  async function tick(i: number, done: boolean) {
    if (!note) return;
    note.tasks[i].done = done;
    try {
      await api.setTask(path, i, done);
      onchange();
    } catch (e) {
      error = String(e);
      await load();
    }
  }

  async function toCalendar(i: number) {
    try {
      await api.taskToCalendar(path, i);
    } catch (e) {
      error = String(e);
    }
  }

  let article = $state<HTMLElement>();
  function jump(t: number, play = true) {
    if (note?.meta.audio && audio) {
      if (play) seek(t);
      else audio.currentTime = now = t;
    }
    const i = note?.lines.findIndex((l) => l.t >= t) ?? -1;
    article?.querySelector(`[data-line="${i}"]`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  }

  const taskMd = (t: Task) =>
    `- [${t.done ? "x" : " "}] ${t.owner ? `**${t.owner}** — ` : ""}${t.text}${t.due ? ` (due ${t.due})` : ""}`;

  function asMarkdown() {
    if (!note) return "";
    const parts = [`# ${note.meta.title}`, longDate(note.meta.date)];
    if (note.meta.gist) parts.push(`> ${note.meta.gist}`);
    if (note.summaries[mode]) parts.push(note.summaries[mode]!);
    if (note.tasks.length) parts.push(`### Action items\n${note.tasks.map(taskMd).join("\n")}`);
    if (note.chapters.length) parts.push(`### Chapters\n${note.chapters.map((c) => `- [${clock(c.t)}] ${c.title}`).join("\n")}`);
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
  <article class:compact bind:this={article}>
    <header>
      <div class="kicker">
        <span class="kind-dot" class:meeting={isMeeting}></span>
        <span>{isMeeting ? "Meeting" : "Voice note"}</span>
        <span class="sep">·</span>
        <span>{longDate(note.meta.date)}</span>
      </div>
      {#if editing}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="title-input"
          bind:value={title}
          autofocus
          onblur={saveTitle}
          onkeydown={(e) => {
            if (e.key === "Enter") saveTitle();
            if (e.key === "Escape") {
              title = note!.meta.title;
              editing = false;
            }
          }}
        />
      {:else}
        <h1>
          <button class="title-btn" onclick={() => (editing = true)} title="Rename">
            {note.meta.title}<span class="pen"><Icon name="edit" size={13} /></span>
          </button>
        </h1>
      {/if}
      <div class="meta">
        {#if note.meta.duration}<span>{note.meta.duration}</span>{/if}
        {#if note.meta.app}<span>{appName(note.meta.app)}</span>{/if}
        <span>{note.meta.words} words</span>
        <span class="tools">
          <button class="icon-btn" onclick={copy} title="Copy as Markdown"><Icon name={copied ? "check" : "copy"} size={15} /></button>
          <button class="icon-btn" onclick={() => api.reveal(path)} title="Show in folder"><Icon name="reveal" size={15} /></button>
          <button class="icon-btn del" class:confirm={confirmDelete} onclick={remove} title="Move to Recycle Bin">
            <Icon name="trash" size={15} />{#if confirmDelete}<span>Delete?</span>{/if}
          </button>
        </span>
      </div>
      {#if note.meta.gist}<p class="gist selectable">{note.meta.gist}</p>{/if}
    </header>

    {#if error}<div class="error">{error}</div>{/if}

    {#if note.meta.audio}
      <div class="player">
        <button class="play" onclick={toggle} aria-label={playing ? "Pause" : "Play"}>
          <Icon name={playing ? "pause" : "play"} size={13} />
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

    {#if isMeeting && speakers.length}
      <section class="people">
        <div class="people-head">
          <h3>People</h3>
          {#if speakers.some(generic)}
            <button class="link" onclick={suggest} disabled={suggesting}>
              <Icon name="sparkle" size={12} />{suggesting ? "Listening for names…" : "Suggest names"}
            </button>
          {/if}
        </div>
        <div class="chips">
          {#each speakers as s (s)}
            <button class="person" class:on={naming === s} class:guess={unconfirmed(s)} onclick={() => startNaming(s)}>
              <span class="avatar" style:background={color(s)}>{initials(s)}</span>
              <span>{s}</span>
              {#if unconfirmed(s)}<span class="q" title="Recognised by voice; click to confirm">?</span>{/if}
            </button>
          {/each}
        </div>
        {#if naming}
          {@const s = naming}
          <div class="namer">
            <input
              class="field"
              bind:value={newName}
              placeholder={`Who is ${s}?`}
              onkeydown={(e) => {
                if (e.key === "Enter") rename(s, newName);
                if (e.key === "Escape") naming = null;
              }}
            />
            {#if unconfirmed(s)}
              <button class="btn" onclick={() => rename(s, s)}><Icon name="check" size={13} />It’s {s}</button>
            {/if}
            <button class="btn primary" onclick={() => rename(s, newName)} disabled={!newName.trim() || newName.trim() === s}>Rename</button>
          </div>
          <p class="hint">Renames them everywhere in this note{s === me ? "" : " and remembers the voice for next time"}.</p>
        {/if}
        {#if suggestions}
          <div class="suggest">
            {#if suggestions.length}
              {#each suggestions as [label, name] (label)}
                <button class="sugg" onclick={() => rename(label, name)}>
                  <span class="faint">{label}</span><Icon name="chevron" size={11} /><strong>{name}</strong><Icon name="check" size={12} />
                </button>
              {/each}
            {:else}
              <span class="faint">No names came up in the conversation.</span>
            {/if}
          </div>
        {/if}
      </section>
    {/if}

    {#if isMeeting}
      <section class="card summary selectable">
        <div class="card-head">
          <h3><Icon name="sparkle" size={13} />Summary</h3>
          <span class="seg">
            {#each MODES as m}
              <button class:on={mode === m} class:missing={!note.summaries[m]} disabled={summarizing} onclick={() => (mode = m)}>
                {m[0].toUpperCase() + m.slice(1)}
              </button>
            {/each}
          </span>
          {#if note.summaries[mode] && !summarizing}
            <button class="icon-btn" onclick={summarize} title="Summarize again"><Icon name="history" size={14} /></button>
          {/if}
        </div>
        {#if summarizing}
          <div class="shimmer"><div></div><div></div><div></div><div></div></div>
          <p class="faint small">Running locally; {mode === "concise" ? "about 20 seconds" : mode === "standard" ? "a minute or so" : "about two minutes"}.</p>
        {:else if note.summaries[mode]}
          <div class="md">{@html markdown(note.summaries[mode]!)}</div>
        {:else if note.meta.words < 40}
          <p class="faint">Too short to summarize.</p>
        {:else}
          <div class="gen">
            <p class="faint">
              {mode === "concise"
                ? "A TL;DR in three bullets."
                : mode === "standard"
                  ? "Key points, decisions and open questions."
                  : "Standard, plus what was left unsaid and suggested next steps."}
            </p>
            <button class="btn primary" onclick={summarize}><Icon name="sparkle" size={13} />Generate</button>
          </div>
        {/if}
      </section>
    {/if}

    {#if note.tasks.length}
      <section class="card tasks">
        <div class="card-head">
          <h3><Icon name="tasks" size={13} />Action items</h3>
          <span class="faint small">{note.tasks.filter((t) => !t.done).length} open</span>
        </div>
        <ul>
          {#each note.tasks as task, i (i)}
            <li class:done={task.done}>
              <input type="checkbox" checked={task.done} onchange={(e) => tick(i, e.currentTarget.checked)} />
              <div class="task selectable">
                {#if task.owner}<strong style:color={color(task.owner)}>{task.owner}</strong>{/if}
                <span>{task.text}</span>
                <span class="task-meta">
                  {#if task.due}
                    <button class="chip" onclick={() => toCalendar(i)} title="Add to calendar"><Icon name="calendar" size={12} />{dueDate(task.due)}</button>
                  {/if}
                  {#if task.t != null && note.lines.length}
                    <button class="chip" onclick={() => jump(task.t!)} title="Where it was said">{clock(task.t)}</button>
                  {/if}
                </span>
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if note.chapters.length}
      <section class="chapters">
        <h3><Icon name="chapters" size={13} />Chapters</h3>
        <ol>
          {#each note.chapters as ch}
            <li><button onclick={() => jump(ch.t)}><span class="t">{clock(ch.t)}</span>{ch.title}</button></li>
          {/each}
        </ol>
      </section>
    {/if}

    {#if note.lines.length}
      <section class="transcript selectable">
        <h3>Transcript</h3>
        {#each note.lines as line, i (i)}
          {@const first = i === 0 || note.lines[i - 1].speaker !== line.speaker}
          <div class="line" class:active={i === active} class:cont={!first} data-line={i}>
            <button class="t" onclick={() => seek(line.t)} disabled={!note.meta.audio}>{clock(line.t)}</button>
            <div>
              {#if first}
                <button class="who" style:color={color(line.speaker)} onclick={() => isMeeting && startNaming(line.speaker)}>
                  {line.speaker}{#if unconfirmed(line.speaker)}<span class="q">?</span>{/if}
                </button>
              {/if}
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
    max-width: 800px;
    margin: 0 auto;
    animation: fade-in 0.2s var(--ease);
  }
  article.compact {
    padding: 6px 18px 40px;
  }
  header {
    margin-bottom: 16px;
  }
  .kicker {
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--muted);
    font-size: 12px;
    margin-bottom: 6px;
  }
  .sep {
    color: var(--faint);
  }
  h1 {
    font-size: 24px;
    line-height: 1.25;
    letter-spacing: -0.01em;
  }
  .compact h1 {
    font-size: 19px;
  }
  .title-btn {
    all: unset;
    cursor: text;
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
    font: 600 24px "Segoe UI Variable Display", "Segoe UI", sans-serif;
    border: 0;
    border-bottom: 2px solid var(--accent);
    background: transparent;
    outline: 0;
    padding: 0;
  }
  .compact .title-input {
    font-size: 19px;
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--muted);
    font-size: 12.5px;
    margin-top: 6px;
  }
  .tools {
    margin-left: auto;
    display: flex;
    gap: 2px;
  }
  .del.confirm {
    width: auto;
    padding: 0 8px;
    gap: 4px;
    display: flex;
    background: var(--danger);
    color: white;
    font-size: 12px;
  }
  .error {
    padding: 9px 12px;
    border-radius: 9px;
    margin-bottom: 14px;
    background: color-mix(in srgb, var(--danger) 10%, transparent);
    color: var(--danger);
  }
  .player {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px 6px 6px;
    border-radius: 22px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    margin-bottom: 18px;
    position: sticky;
    top: 0;
    z-index: 2;
  }
  .play {
    width: 30px;
    height: 30px;
    flex: none;
    border-radius: 50%;
    border: 0;
    background: var(--text);
    color: var(--bg);
    display: grid;
    place-items: center;
  }
  .clock {
    font-size: 11.5px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    min-width: 30px;
  }
  .bar {
    flex: 1;
    height: 4px;
    border-radius: 4px;
    background: var(--border-strong);
    cursor: pointer;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }

  /* people */
  .people {
    margin-bottom: 18px;
  }
  .people-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: 12px;
    padding: 2px 6px;
    border-radius: 6px;
  }
  .link:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .link:disabled {
    color: var(--muted);
    cursor: default;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .person {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 10px 0 3px;
    border-radius: 14px;
    border: 1px solid var(--border);
    background: var(--raised);
    font-size: 12.5px;
    transition: border-color 0.15s, box-shadow 0.15s;
  }
  .person:hover {
    border-color: var(--border-strong);
  }
  .person.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .person.guess {
    border-style: dashed;
  }
  .avatar {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    color: white;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .q {
    display: inline-grid;
    place-items: center;
    width: 15px;
    height: 15px;
    margin-left: 3px;
    border-radius: 50%;
    background: var(--hover);
    color: var(--muted);
    font-size: 10px;
    font-weight: 700;
  }
  .namer {
    display: flex;
    gap: 6px;
    margin-top: 10px;
    animation: fade-in 0.16s var(--ease);
  }
  .namer .field {
    flex: 1;
    min-width: 0;
  }
  .hint {
    margin: 6px 2px 0;
    font-size: 12px;
    color: var(--faint);
  }
  .suggest {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 10px;
  }
  .sugg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    border-radius: 14px;
    border: 1px dashed color-mix(in srgb, var(--accent) 45%, transparent);
    background: var(--accent-soft);
    color: var(--text);
    font-size: 12.5px;
  }
  .sugg :global(svg:last-child) {
    color: var(--accent-text);
  }
  .sugg:hover {
    border-style: solid;
  }

  /* summary */
  .card {
    padding: 14px 18px 16px;
    border-radius: 14px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    margin-bottom: 24px;
  }
  .compact .card {
    padding: 12px 14px 14px;
  }
  .card-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .card-head h3 {
    flex: 1;
  }
  .seg button.missing:not(.on) {
    color: var(--faint);
  }
  .md {
    line-height: 1.6;
  }
  .md :global(h4) {
    font-size: 13.5px;
    margin: 14px 0 4px;
  }
  .md :global(h4:first-child) {
    margin-top: 0;
  }
  .md :global(p) {
    margin: 0 0 6px;
  }
  .md :global(ul) {
    margin: 0 0 6px;
    padding-left: 18px;
  }
  .md :global(li) {
    margin: 3px 0;
  }
  .md :global(li::marker) {
    color: var(--faint);
  }
  .md :global(strong) {
    font-weight: 600;
  }
  .gen {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .faint {
    color: var(--faint);
    margin: 0;
  }
  .small {
    font-size: 12px;
  }
  .shimmer div {
    height: 10px;
    border-radius: 5px;
    margin: 10px 0;
    background: linear-gradient(90deg, var(--hover), var(--border-strong), var(--hover));
    background-size: 200% 100%;
    animation: shimmer 1.4s infinite linear;
  }
  .shimmer div:nth-child(2) {
    width: 88%;
  }
  .shimmer div:nth-child(3) {
    width: 72%;
  }
  .shimmer div:nth-child(4) {
    width: 50%;
  }
  @keyframes shimmer {
    to {
      background-position: -200% 0;
    }
  }

  .gist {
    margin: 8px 0 0;
    color: var(--muted);
    line-height: 1.5;
  }

  /* action items */
  .tasks ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .tasks li {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 5px 0;
  }
  .tasks li + li {
    border-top: 1px solid var(--border);
  }
  .tasks input {
    margin-top: 4px;
    accent-color: var(--accent);
  }
  .task {
    line-height: 1.5;
  }
  .task strong {
    font-weight: 600;
    margin-right: 6px;
  }
  .tasks li.done .task > span:not(.task-meta) {
    color: var(--faint);
    text-decoration: line-through;
  }
  .task-meta {
    display: inline-flex;
    gap: 4px;
    margin-left: 6px;
    vertical-align: 1px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: transparent;
    border-radius: 999px;
    padding: 0 7px;
    font-size: 11.5px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .chip:hover {
    color: var(--accent-text);
    border-color: var(--accent);
  }

  /* chapters */
  .chapters {
    margin-bottom: 24px;
  }
  .chapters h3 {
    margin-bottom: 6px;
  }
  .chapters ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .chapters button {
    all: unset;
    cursor: pointer;
    display: grid;
    grid-template-columns: 44px 1fr;
    gap: 6px;
    width: 100%;
    padding: 3px 0;
  }
  .chapters button:hover {
    color: var(--accent-text);
  }

  /* transcript */
  .transcript h3 {
    margin-bottom: 6px;
  }
  .line {
    display: grid;
    grid-template-columns: 44px 1fr;
    gap: 6px;
    padding: 6px 8px;
    margin: 4px -8px 0;
    border-radius: 9px;
    transition: background 0.2s;
  }
  .line.cont {
    margin-top: 0;
    padding-top: 2px;
  }
  .line.active {
    background: var(--accent-soft);
  }
  .t {
    border: 0;
    background: transparent;
    color: var(--faint);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    padding: 3px 0 0;
    text-align: left;
    height: fit-content;
  }
  .t:not(:disabled):hover {
    color: var(--accent-text);
  }
  .who {
    all: unset;
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 600;
  }
  .who:hover {
    text-decoration: underline;
  }
  .line p,
  .body p {
    margin: 1px 0 0;
    line-height: 1.6;
  }
  .body p {
    margin-bottom: 12px;
    font-size: 14px;
  }
</style>
