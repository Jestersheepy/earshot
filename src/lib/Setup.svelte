<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, type SetupProgress, type SetupStatus } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

  let { ondone }: { ondone: () => void } = $props();

  let status = $state<SetupStatus | null>(null);
  let p = $state<SetupProgress | null>(null);
  let summaries = $state(true);
  // download speed, smoothed, for the time left
  let rate = $state(0);
  let last: { t: number; done: number } | null = null;

  const size = (b: number) => (b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(b / 1e6))} MB`);
  let total = $derived(status ? status.needed + (summaries ? status.summaries : 0) : 0);
  let running = $derived(p?.state === "downloading" || p?.state === "verifying");
  let done = $derived(p?.state === "done" || (status !== null && !p && total === 0));
  let pct = $derived(p && p.total ? Math.min(100, (p.done / p.total) * 100) : 0);
  let left = $derived.by(() => {
    if (!p || !rate || p.state !== "downloading") return "";
    const s = (p.total - p.done) / rate;
    return s < 60 ? "less than a minute left" : `about ${Math.round(s / 60)} min left`;
  });

  function track(n: SetupProgress) {
    const t = performance.now();
    if (last && n.done > last.done && t > last.t) {
      const r = ((n.done - last.done) * 1000) / (t - last.t);
      rate = rate ? rate * 0.9 + r * 0.1 : r;
    }
    last = { t, done: n.done };
  }

  onMount(() => {
    api.setupStatus().then((s) => {
      status = s;
      if (s.progress && s.progress.state !== "idle") p = s.progress;
    });
    const un = listen<SetupProgress>("setup-progress", (e) => {
      track(e.payload);
      p = e.payload;
    });
    return () => un.then((f) => f());
  });

  function start() {
    if (!status) return;
    rate = 0;
    last = null;
    p = { state: "downloading", done: 0, total, part: "", speech_ready: status.speech_ready, error: null };
    api.setupStart(summaries);
  }
</script>

<div class="setup">
  <div class="mark" class:done><Icon name={done ? "check" : "voice"} size={22} /></div>

  {#if done}
    <h1>You’re all set</h1>
    <p class="lead">Hold <kbd>Ctrl</kbd> <kbd>Win</kbd> anywhere and speak; let go to paste. Right-click the dot for voice notes and meetings.</p>
    <button class="btn primary big" onclick={ondone}>Start using Earshot</button>
  {:else if running && p}
    <h1>Getting Earshot ready</h1>
    <p class="lead">Downloading the models, once. After this, nothing leaves your PC.</p>
    <div class="track" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
      <div class="fill" style:width="{pct}%"></div>
    </div>
    <div class="meta">
      <span>{p.state === "verifying" ? `Checking the ${p.part.toLowerCase()}…` : p.part || "Starting…"}</span>
      <span>{size(p.done)} of {size(p.total)}{left ? ` · ${left}` : ""}</span>
    </div>
    {#if p.speech_ready}
      <div class="ready"><Icon name="check" size={14} /><span>Dictation is ready: hold <kbd>Ctrl</kbd> <kbd>Win</kbd> and speak. The rest keeps downloading in the background.</span></div>
    {/if}
  {:else if status}
    <h1>Welcome to Earshot</h1>
    <p class="lead">Dictation and meeting notes that run on your PC. First, Earshot needs its speech and language models.</p>
    <div class="card">
      {#if status.needed > 0}
        <div class="row">
          <div class="label"><div>Speech and formatting</div><div class="hint">Dictation, voice notes, telling voices apart.</div></div>
          <span class="size">{size(status.needed)}</span>
        </div>
      {/if}
      {#if status.summaries > 0}
        <label class="row">
          <div class="label"><div>Meeting summaries on this PC</div><div class="hint">Or skip this and use a cloud model, in Settings.</div></div>
          <span class="size" class:off={!summaries}>{size(status.summaries)}</span>
          <span class="switch"><input type="checkbox" bind:checked={summaries} /><span></span></span>
        </label>
      {/if}
    </div>
    {#if p?.state === "failed"}<p class="error">{p.error}</p>{/if}
    <button class="btn primary big" onclick={start} disabled={total === 0}>
      {p?.state === "failed" ? "Try again" : `Download ${size(total)}`}
    </button>
    <p class="fine">From GitHub and Hugging Face. Interrupted downloads pick up where they left off.</p>
  {/if}
</div>

<style>
  .setup {
    height: 100%;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    padding: 36px 28px 24px;
    max-width: 460px;
    margin: 0 auto;
    animation: fade-in 0.3s var(--ease);
  }
  .mark {
    width: 52px;
    height: 52px;
    border-radius: 16px;
    display: grid;
    place-items: center;
    color: white;
    background: linear-gradient(160deg, color-mix(in srgb, var(--accent) 70%, white), var(--accent));
    box-shadow: 0 6px 18px color-mix(in srgb, var(--accent) 35%, transparent);
    margin-bottom: 18px;
    transition: transform 0.3s var(--ease);
  }
  .mark.done {
    animation: pop 0.45s var(--ease);
  }
  @keyframes pop {
    40% {
      transform: scale(1.12);
    }
  }
  h1 {
    margin: 0;
    font: 600 20px/1.25 "Segoe UI Variable Display", "Segoe UI", sans-serif;
    letter-spacing: -0.01em;
  }
  .lead {
    margin: 8px 0 22px;
    color: var(--muted);
    max-width: 340px;
  }
  kbd {
    font: 600 11px "Segoe UI", sans-serif;
    padding: 1px 6px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    background: var(--raised);
    color: var(--text);
  }
  .card {
    width: 100%;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--raised);
    margin-bottom: 18px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
  }
  .row + .row {
    border-top: 1px solid var(--rule);
  }
  .label {
    flex: 1;
  }
  .hint {
    font-size: 12px;
    color: var(--muted);
  }
  .size {
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    transition: opacity 0.15s;
  }
  .size.off {
    opacity: 0.4;
    text-decoration: line-through;
  }
  .big {
    height: 38px;
    padding: 0 22px;
    font-size: 13.5px;
    border-radius: 10px;
  }
  .fine {
    margin: 12px 0 0;
    font-size: 11.5px;
    color: var(--faint);
  }
  .error {
    margin: -6px 0 14px;
    color: var(--danger);
    font-size: 12px;
  }
  .track {
    width: 100%;
    height: 6px;
    border-radius: 3px;
    background: var(--surface);
    overflow: hidden;
  }
  .fill {
    height: 100%;
    border-radius: 3px;
    background: var(--accent);
    transition: width 0.25s linear;
    background-image: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.35), transparent);
    background-size: 120px 100%;
    background-repeat: no-repeat;
    animation: sheen 1.6s linear infinite;
  }
  @keyframes sheen {
    from {
      background-position: -120px 0;
    }
    to {
      background-position: calc(100% + 120px) 0;
    }
  }
  .meta {
    width: 100%;
    display: flex;
    justify-content: space-between;
    gap: 12px;
    margin-top: 8px;
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .ready {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin-top: 20px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--accent-soft);
    color: var(--accent-text);
    text-align: left;
    font-size: 12.5px;
    animation: fade-in 0.3s var(--ease);
  }
  .ready :global(svg) {
    flex: none;
    margin-top: 3px;
  }
</style>
