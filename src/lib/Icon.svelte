<script lang="ts">
  // 16px line icons, stroke = currentColor.
  let { name, size = 16 }: { name: string; size?: number } = $props();
  const paths: Record<string, string> = {
    all: "M3 4.5h10M3 8h10M3 11.5h6",
    meeting: "M5.5 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4ZM1.8 13c.4-2.2 1.9-3.5 3.7-3.5s3.3 1.3 3.7 3.5M11 6.6a1.7 1.7 0 1 0 0-3.4M12.2 9.6c1.2.4 1.9 1.6 2.1 3.4",
    voice: "M8 2.2a2 2 0 0 1 2 2v3.6a2 2 0 0 1-4 0V4.2a2 2 0 0 1 2-2ZM4.2 7.6a3.8 3.8 0 0 0 7.6 0M8 11.4v2.4",
    history: "M2.8 8a5.2 5.2 0 1 0 1.6-3.8M2.6 2.6v2.6h2.6M8 5.2V8l2 1.4",
    settings: "M8 10.2a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4ZM8 1.8v1.6M8 12.6v1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M1.8 8h1.6M12.6 8h1.6M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1",
    folder: "M2 4.6c0-.6.4-1 1-1h3.2l1.4 1.5H13c.6 0 1 .4 1 1v5.8c0 .6-.4 1-1 1H3c-.6 0-1-.4-1-1V4.6Z",
    search: "M7.2 12.2a5 5 0 1 0 0-10 5 5 0 0 0 0 10ZM10.8 10.8 14 14",
    copy: "M5.5 5.5V3.3c0-.5.4-.8.8-.8h6.4c.5 0 .8.3.8.8v6.4c0 .5-.3.8-.8.8h-2.2M3.3 5.5h6.4c.5 0 .8.3.8.8v6.4c0 .5-.3.8-.8.8H3.3a.8.8 0 0 1-.8-.8V6.3c0-.5.3-.8.8-.8Z",
    trash: "M2.8 4.3h10.4M6.4 2.5h3.2M4.2 4.3l.6 8.5c0 .5.4.8.8.8h4.8c.4 0 .8-.3.8-.8l.6-8.5",
    sparkle: "M8 2.2l1.3 3.4 3.5 1.3-3.5 1.3L8 11.6 6.7 8.2 3.2 6.9l3.5-1.3L8 2.2ZM12.6 10.8l.5 1.3 1.3.5-1.3.5-.5 1.3-.5-1.3-1.3-.5 1.3-.5.5-1.3Z",
    reveal: "M9.5 2.5h4v4M13.5 2.5 8 8M6.5 3.5H3.8c-.7 0-1.3.6-1.3 1.3v7.4c0 .7.6 1.3 1.3 1.3h7.4c.7 0 1.3-.6 1.3-1.3V9.5",
    play: "M5 3.2v9.6c0 .5.5.8.9.5l7-4.8a.6.6 0 0 0 0-1L5.9 2.7c-.4-.3-.9 0-.9.5Z",
    pause: "M5 3v10M11 3v10",
    edit: "M10.6 2.9a1.5 1.5 0 0 1 2.1 2.1L5.6 12.1 2.8 13l.9-2.8 6.9-7.3Z",
    plus: "M8 3v10M3 8h10",
    x: "M4 4l8 8M12 4l-8 8",
    check: "M3.2 8.4l3 3 6.6-6.8",
    wand: "M2.5 13.5 10 6M9.2 2.5v1.6M12.6 5.9h1.6M11.6 3.6l1.1-1.1M6.3 3.6l-.6-.6M12.6 9.2l.6.6",
  };
</script>

<svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "play"}
    <path d={paths.play} fill="currentColor" stroke="none" />
  {:else}
    <path d={paths[name] ?? ""} />
  {/if}
</svg>
