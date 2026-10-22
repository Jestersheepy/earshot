<script lang="ts">
  // A stand-in for the floating dot (drawn natively by crates/overlay/src/pill.rs), so Settings can show
  // size, idle style, accent and the hover peek before saving.
  import type { Config } from "$lib/api";

  let { pill, accent, dictate }: { pill: Config["pill"]; accent: string; dictate: string } = $props();

  const SIZE = { small: 16, medium: 20, large: 26 };
  const SCALE = 1.5;
  let d = $derived((SIZE[pill.size] + (pill.idle === "mic" ? 6 : 0)) * SCALE);
  let title = $derived(
    pill.click === "note"
      ? "Click for a voice note"
      : pill.click === "meeting"
        ? "Click to record a meeting"
        : pill.click === "panel"
          ? "Click to open notes"
          : "Click to dictate",
  );
  let hint = $derived(pill.click === "dictate" ? "Right-click for more" : `Hold ${dictate} to dictate`);

  let peek = $state(false);
  let timer: ReturnType<typeof setTimeout>;
  function enter() {
    clearTimeout(timer);
    if (pill.peek) timer = setTimeout(() => (peek = true), 450);
  }
  function leave() {
    clearTimeout(timer);
    peek = false;
  }
</script>

<div class="preview">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="stage" onmouseenter={enter} onmouseleave={leave}>
    <div class="pill" class:peek style:--d="{d}px" style:--accent={accent}>
      {#if peek}
        <span class="pdot"></span>
        <span class="ptitle">{title}</span>
        <span class="phint">{hint}</span>
      {:else if pill.idle === "mic"}
        <svg width={d * 0.5} height={d * 0.5} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
          <path d="M8 2.2a2 2 0 0 1 2 2v3.6a2 2 0 0 1-4 0V4.2a2 2 0 0 1 2-2ZM4.2 7.6a3.8 3.8 0 0 0 7.6 0M8 11.4v2.4" />
        </svg>
      {:else}
        <span class="dot" class:breathing={pill.idle === "breathing"}></span>
      {/if}
    </div>
  </div>
  <p>{pill.peek ? "Hover to preview the peek" : "Your dot"}</p>
</div>

<style>
  .preview {
    padding: 14px 16px 10px;
    border-bottom: 1px solid var(--rule);
  }
  .stage {
    display: grid;
    place-items: center;
    height: 96px;
    border-radius: 10px;
    background:
      radial-gradient(120% 140% at 15% 0%, color-mix(in srgb, var(--accent) 22%, transparent), transparent 55%),
      linear-gradient(160deg, #d9d4ca, #b9b2a6);
    overflow: hidden;
  }
  .pill {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    width: var(--d);
    height: var(--d);
    border-radius: 999px;
    color: #ddd8cf;
    background: rgba(28, 27, 25, 0.94);
    box-shadow:
      inset 0 0 0 1px rgba(255, 255, 255, 0.1),
      0 2px 8px rgba(0, 0, 0, 0.3);
    transition:
      width 0.22s var(--ease),
      height 0.22s var(--ease),
      padding 0.22s var(--ease);
    white-space: nowrap;
    overflow: hidden;
  }
  .pill.peek {
    width: 300px;
    height: 40px;
    padding: 0 16px;
    justify-content: flex-start;
  }
  .dot {
    width: calc(var(--d) * 0.3);
    height: calc(var(--d) * 0.3);
    border-radius: 50%;
    background: #bdb8af;
  }
  .dot.breathing {
    background: var(--accent);
    animation: breathe 4s ease-in-out infinite;
  }
  @keyframes breathe {
    0%,
    100% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 45%, transparent);
    }
    50% {
      box-shadow: 0 0 0 calc(var(--d) * 0.22) color-mix(in srgb, var(--accent) 0%, transparent);
    }
  }
  .pdot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
  }
  .ptitle {
    color: #f4f1ec;
    font-weight: 600;
    font-size: 12.5px;
  }
  .phint {
    color: #a39d94;
    font-size: 12px;
  }
  p {
    margin: 6px 0 0;
    text-align: center;
    font-size: 11.5px;
    color: var(--faint);
  }
</style>
