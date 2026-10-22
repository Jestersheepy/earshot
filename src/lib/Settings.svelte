<script lang="ts">
  import { onMount } from "svelte";
  import { api, applyLook, type Config, type Settings } from "$lib/api";
  import Icon from "$lib/Icon.svelte";
  import DotPreview from "$lib/DotPreview.svelte";

  let settings = $state<Settings | null>(null);
  let cfg = $state<Config | null>(null);
  let saved = $state("");
  let error = $state("");
  let flash = $state(false);
  let dirty = $derived(cfg !== null && JSON.stringify(cfg) !== saved);

  let newTerm = $state("");
  let newLiteral = $state("");
  let newApp = $state("");
  let recording = $state<string | null>(null);
  let held: string[] = [];
  let voices = $state<[string, number][]>([]);

  const SWATCHES = ["#4f6ef7", "#14a38b", "#d6457a", "#8b6cf0", "#2f9e66", "#e0662f", "#3d3a35"];
  const clicks: { id: Config["pill"]["click"]; label: string }[] = [
    { id: "dictate", label: "Dictate" },
    { id: "note", label: "Voice note" },
    { id: "meeting", label: "Meeting" },
    { id: "panel", label: "Panel" },
  ];
  let accentShown = $derived(cfg && cfg.pill.accent !== "auto" ? cfg.pill.accent : (settings?.accent ?? "#4f6ef7"));

  // Preview theme and accent while editing; Save makes them stick, Discard puts them back.
  $effect(() => {
    if (!cfg || !settings) return;
    applyLook({ ...settings, config: cfg, accent: accentShown });
  });

  const hotkeys: { key: keyof Config["hotkeys"]; label: string; hint: string }[] = [
    { key: "dictate", label: "Dictate", hint: "Hold and speak; release to paste. Double-tap to go hands-free." },
    { key: "command", label: "Command mode", hint: "Hold over selected text and say how to change it." },
    { key: "voice_note", label: "Voice note", hint: "Press to start, press again to save." },
    { key: "meeting", label: "Meeting", hint: "Records your mic and the call’s audio." },
    { key: "paste_last", label: "Paste last", hint: "Pastes your most recent dictation again." },
    { key: "toggle_pill", label: "Show / hide pill", hint: "The floating indicator." },
  ];

  async function load() {
    settings = await api.getSettings();
    cfg = settings.config;
    saved = JSON.stringify(cfg);
    voices = await api.listVoices().catch(() => []);
  }
  onMount(() => {
    load();
    // leaving with unsaved changes drops the preview
    return () => settings && applyLook(settings);
  });

  async function forget(name: string) {
    await api.forgetVoice(name);
    voices = voices.filter(([n]) => n !== name);
  }

  async function save() {
    if (!cfg) return;
    error = "";
    try {
      await api.saveSettings($state.snapshot(cfg) as Config);
      saved = JSON.stringify(cfg);
      flash = true;
      setTimeout(() => (flash = false), 1600);
      settings = await api.getSettings();
      applyLook(settings);
    } catch (e) {
      error = String(e);
    }
  }

  const pretty = (combo: string) =>
    combo
      .split("+")
      .filter(Boolean)
      .map((k) => (k.length === 1 ? k.toUpperCase() : k[0].toUpperCase() + k.slice(1)));

  function keyName(code: string): string | null {
    if (code.startsWith("Key")) return code.slice(3).toLowerCase();
    if (code.startsWith("Digit")) return code.slice(5);
    if (/^F\d{1,2}$/.test(code)) return code.toLowerCase();
    if (code === "Space") return "space";
    if (code === "Enter") return "enter";
    return null;
  }
  const mods = (e: KeyboardEvent) =>
    [e.ctrlKey && "ctrl", e.metaKey && "win", e.altKey && "alt", e.shiftKey && "shift"].filter(Boolean) as string[];

  function onKeyDown(e: KeyboardEvent, key: keyof Config["hotkeys"]) {
    if (recording !== key || !cfg) return;
    e.preventDefault();
    if (e.code === "Escape") {
      recording = null;
      return;
    }
    const m = mods(e);
    if (m.length > held.length) held = m;
    const k = keyName(e.code);
    if (k) {
      cfg.hotkeys[key] = [...m, k].join("+");
      recording = null;
    }
  }
  function onKeyUp(e: KeyboardEvent, key: keyof Config["hotkeys"]) {
    if (recording !== key || !cfg) return;
    e.preventDefault();
    // Modifier-only chords (like Ctrl+Win) are recorded on release.
    if (mods(e).length === 0 && held.length >= 2) {
      cfg.hotkeys[key] = held.join("+");
      recording = null;
    }
  }
  function startRecording(key: string) {
    recording = key;
    held = [];
  }

  function addTo(list: string[], value: string) {
    const v = value.trim();
    if (v && !list.some((x) => x.toLowerCase() === v.toLowerCase())) list.push(v);
  }
</script>

{#if cfg && settings}
  <div class="wrap">
    <div class="scroll">
      <header>
        <h2>Settings</h2>
        <p class="sub">Everything runs on this PC. Nothing is sent anywhere.</p>
      </header>

      <section>
        <h3>General</h3>
        <div class="card">
          <div class="row stack">
            <div class="label">
              <div>Clicking the dot</div>
              <div class="hint">Also on the dot’s right-click menu. Holding {pretty(cfg.hotkeys.dictate).join("+")} always dictates.</div>
            </div>
            <div class="seg full">
              {#each clicks as c (c.id)}
                <button class:on={cfg.pill.click === c.id} onclick={() => (cfg!.pill.click = c.id)}>{c.label}</button>
              {/each}
            </div>
          </div>
          <div class="row">
            <div class="label"><div>Theme</div></div>
            <div class="seg">
              {#each [["system", "System"], ["light", "Light"], ["dark", "Dark"]] as [id, label]}
                <button class:on={cfg.theme === id} onclick={() => (cfg!.theme = id as Config["theme"])}>{label}</button>
              {/each}
            </div>
          </div>
          <div class="row">
            <div class="label">
              <div>Start with Windows</div>
              <div class="hint">Earshot waits quietly in the tray, ready for your shortcuts.</div>
            </div>
            <label class="switch">
              <input
                type="checkbox"
                checked={settings!.autostart}
                onchange={async (e) => {
                  const box = e.currentTarget;
                  const on = box.checked;
                  try {
                    await api.setAutostart(on);
                    settings!.autostart = on;
                  } catch (err) {
                    error = String(err);
                    box.checked = !on;
                  }
                }}
              /><span></span>
            </label>
          </div>
        </div>
      </section>

      <section>
        <h3>The dot</h3>
        <div class="card">
          <DotPreview pill={cfg.pill} accent={accentShown} dictate={pretty(cfg.hotkeys.dictate).join("+")} />
          <div class="row">
            <div class="label"><div>Size</div></div>
            <div class="seg">
              {#each [["small", "Small"], ["medium", "Medium"], ["large", "Large"]] as [id, label]}
                <button class:on={cfg.pill.size === id} onclick={() => (cfg!.pill.size = id as Config["pill"]["size"])}>{label}</button>
              {/each}
            </div>
          </div>
          <div class="row">
            <div class="label">
              <div>When idle</div>
              <div class="hint">{cfg.pill.idle === "breathing" ? "A slow glow in your accent colour." : cfg.pill.idle === "mic" ? "A mic, so it’s obvious what it is." : "A quiet dot."}</div>
            </div>
            <div class="seg">
              {#each [["dot", "Dot"], ["mic", "Mic"], ["breathing", "Breathing"]] as [id, label]}
                <button class:on={cfg.pill.idle === id} onclick={() => (cfg!.pill.idle = id as Config["pill"]["idle"])}>{label}</button>
              {/each}
            </div>
          </div>
          <div class="row">
            <div class="label"><div>Accent</div></div>
            <div class="swatches">
              <button class="swatch auto" class:on={cfg.pill.accent === "auto"} onclick={() => (cfg!.pill.accent = "auto")} title="Windows accent colour">
                <span style:background-color={settings.accent}></span>
              </button>
              {#each SWATCHES as c (c)}
                <button class="swatch" class:on={cfg.pill.accent.toLowerCase() === c} onclick={() => (cfg!.pill.accent = c)} aria-label={c}>
                  <span style:background-color={c}></span>
                </button>
              {/each}
              <label class="swatch custom" class:on={cfg.pill.accent !== "auto" && !SWATCHES.includes(cfg.pill.accent.toLowerCase())} title="Custom colour">
                <input type="color" value={accentShown} oninput={(e) => (cfg!.pill.accent = e.currentTarget.value)} />
                <span><Icon name="plus" size={11} /></span>
              </label>
            </div>
          </div>
          <div class="row">
            <div class="label">
              <div>Hover peek</div>
              <div class="hint">Hovering the dot shows what a click does.</div>
            </div>
            <label class="switch"><input type="checkbox" bind:checked={cfg.pill.peek} /><span></span></label>
          </div>
        </div>
      </section>
      <section>
        <h3>Shortcuts</h3>
        <div class="card">
          {#each hotkeys as h (h.key)}
            <div class="row">
              <div class="label">
                <div>{h.label}</div>
                <div class="hint">{h.hint}</div>
              </div>
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="keys"
                class:recording={recording === h.key}
                tabindex="0"
                role="button"
                onclick={() => startRecording(h.key)}
                onkeydown={(e) => (recording === h.key ? onKeyDown(e, h.key) : e.key === "Enter" && startRecording(h.key))}
                onkeyup={(e) => onKeyUp(e, h.key)}
                onblur={() => recording === h.key && (recording = null)}
              >
                {#if recording === h.key}
                  <span class="press">Press the keys… (Esc to cancel)</span>
                {:else}
                  {#each pretty(cfg.hotkeys[h.key]) as k}<kbd>{k}</kbd>{/each}
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </section>

      <section>
        <h3>Dictation</h3>
        <div class="card">
          <div class="row">
            <div class="label">
              <div>Smart formatting</div>
              <div class="hint">
                Removes fillers, applies “no wait…” corrections and turns spoken lists into bullets, using a small local
                model. {settings.models.format ? "" : "The formatting model isn’t installed, so only the quick rules run."}
              </div>
            </div>
            <label class="switch"><input type="checkbox" bind:checked={cfg.smart_format} /><span></span></label>
          </div>
          <div class="row stack">
            <div class="label">
              <div>Paste exactly as spoken in</div>
              <div class="hint">Apps where text should never be reworded, like code editors and terminals.</div>
            </div>
            <div class="tags">
              {#each cfg.literal_apps as app, i (app)}
                <span class="tag">{app}<button onclick={() => cfg!.literal_apps.splice(i, 1)} aria-label="Remove"><Icon name="x" size={11} /></button></span>
              {/each}
              <input
                class="tag-input"
                bind:value={newLiteral}
                placeholder="app.exe"
                onkeydown={(e) => {
                  if (e.key === "Enter") { addTo(cfg!.literal_apps, newLiteral.toLowerCase()); newLiteral = ""; }
                }}
              />
            </div>
          </div>
        </div>
      </section>

      <section>
        <h3>Dictionary</h3>
        <div class="card">
          <div class="row stack">
            <div class="label">
              <div>Words to spell right</div>
              <div class="hint">Names, products and jargon. Near misses (“jester sheepy”) are corrected to your spelling.</div>
            </div>
            <div class="tags">
              {#each cfg.dictionary as term, i (term)}
                <span class="tag">{term}<button onclick={() => cfg!.dictionary.splice(i, 1)} aria-label="Remove"><Icon name="x" size={11} /></button></span>
              {/each}
              <input
                class="tag-input"
                bind:value={newTerm}
                placeholder="Add a word and press Enter"
                onkeydown={(e) => {
                  if (e.key === "Enter") { addTo(cfg!.dictionary, newTerm); newTerm = ""; }
                }}
              />
            </div>
          </div>
          <div class="row stack">
            <div class="label">
              <div>Snippets</div>
              <div class="hint">Say the trigger phrase and the full text is typed instead.</div>
            </div>
            <div class="snippets">
              {#each cfg.snippets as s, i (i)}
                <div class="snippet">
                  <input class="field" bind:value={s.trigger} placeholder="When I say…" />
                  <textarea class="field" bind:value={s.text} rows="1" placeholder="Type this"></textarea>
                  <button class="btn ghost" onclick={() => cfg!.snippets.splice(i, 1)} aria-label="Remove snippet"><Icon name="trash" size={14} /></button>
                </div>
              {/each}
              <button class="btn add" onclick={() => cfg!.snippets.push({ trigger: "", text: "" })}><Icon name="plus" size={14} />Add snippet</button>
            </div>
          </div>
        </div>
      </section>

      <section>
        <h3>Meetings</h3>
        <div class="card">
          <div class="row">
            <div class="label">
              <div>Summary</div>
              <div class="hint">Concise: TL;DR and action items. Standard: every detail and action item. Nuanced: also reads between the lines and suggests actions (slower).</div>
            </div>
            <div class="seg">
              {#each ["concise", "standard", "nuanced"] as const as m}
                <button class:on={cfg.summary_mode === m} onclick={() => (cfg!.summary_mode = m)}>{m[0].toUpperCase() + m.slice(1)}</button>
              {/each}
            </div>
          </div>
          <div class="row">
            <div class="label">
              <div>Your name</div>
              <div class="hint">How you appear in meeting transcripts.</div>
            </div>
            <input class="field short" bind:value={cfg.your_name} placeholder="Me" />
          </div>
          <div class="row">
            <div class="label">
              <div>Tell speakers apart</div>
              <div class="hint">Labels voices Speaker 1, 2, … and recognises people you’ve named before.</div>
            </div>
            <label class="switch"><input type="checkbox" bind:checked={cfg.detect_speakers} /><span></span></label>
          </div>
          {#if cfg.detect_speakers && voices.length}
            <div class="row stack">
              <div class="label">
                <div>Voices Earshot knows</div>
                <div class="hint">Name someone in a meeting and their voice is remembered here, on this PC only.</div>
              </div>
              <div class="tags plain">
                {#each voices as [name] (name)}
                  <span class="tag">{name}<button onclick={() => forget(name)} aria-label="Forget {name}" title="Forget this voice"><Icon name="x" size={11} /></button></span>
                {/each}
              </div>
            </div>
          {/if}
          <div class="row">
            <div class="label">
              <div>Offer to record calls</div>
              <div class="hint">When Zoom, Teams, Slack, Discord, Webex or a browser starts using the mic, ask whether to take notes.</div>
            </div>
            <label class="switch"><input type="checkbox" bind:checked={cfg.detect_meetings} /><span></span></label>
          </div>
          {#if cfg.detect_meetings}
            <div class="row stack">
              <div class="label">
                <div>Per app</div>
                <div class="hint">Override the default for an app (exe name, e.g. zoom.exe). Other apps are ignored unless listed.</div>
              </div>
              <div class="apps">
                {#each Object.keys(cfg.meeting_apps) as app (app)}
                  <div class="app-row">
                    <span>{app}</span>
                    <div class="seg">
                      {#each ["ask", "always", "never"] as p}
                        <button class:on={cfg.meeting_apps[app] === p} onclick={() => (cfg!.meeting_apps[app] = p as "ask")}>
                          {p === "ask" ? "Ask" : p === "always" ? "Always record" : "Never"}
                        </button>
                      {/each}
                    </div>
                    <button class="btn ghost" onclick={() => delete cfg!.meeting_apps[app]} aria-label="Remove"><Icon name="x" size={13} /></button>
                  </div>
                {/each}
                <input
                  class="field"
                  bind:value={newApp}
                  placeholder="Add an app, e.g. zoom.exe"
                  onkeydown={(e) => {
                    const v = newApp.trim().toLowerCase();
                    if (e.key === "Enter" && v) { cfg!.meeting_apps[v] = "ask"; newApp = ""; }
                  }}
                />
              </div>
            </div>
          {/if}
        </div>
      </section>

      <section>
        <h3>Storage</h3>
        <div class="card">
          <div class="row stack">
            <div class="label">
              <div>Notes folder</div>
              <div class="hint">Plain Markdown files, so Obsidian and other editors can open them.</div>
            </div>
            <input class="field wide" bind:value={cfg.notes_dir} spellcheck="false" />
          </div>
          <div class="row">
            <div class="label">
              <div>Keep recordings</div>
              <div class="hint">Saves the audio next to each note so you can play it back.</div>
            </div>
            <label class="switch"><input type="checkbox" bind:checked={cfg.keep_wav} /><span></span></label>
          </div>
        </div>
      </section>

      <section>
        <h3>Performance</h3>
        <div class="card">
          <div class="row">
            <div class="label">
              <div>Load speech model at startup</div>
              <div class="hint">Faster first dictation, about 900 MB of memory until it unloads.</div>
            </div>
            <label class="switch"><input type="checkbox" bind:checked={cfg.preload} /><span></span></label>
          </div>
          <div class="row">
            <div class="label">
              <div>Unload speech model after</div>
              <div class="hint">Minutes without use. Applies after restarting Earshot.</div>
            </div>
            <div class="num"><input class="field" type="number" min="0" bind:value={cfg.idle_unload_min} /><span>min</span></div>
          </div>
          <div class="row">
            <div class="label">
              <div>Speech threads</div>
              <div class="hint">CPU threads for transcription. Applies after restarting Earshot.</div>
            </div>
            <div class="num"><input class="field" type="number" min="1" max="32" bind:value={cfg.stt_threads} /></div>
          </div>
          <div class="row">
            <div class="label">
              <div>Keep language models loaded for</div>
              <div class="hint">Minutes after last use. Applies after restarting Earshot.</div>
            </div>
            <div class="num"><input class="field" type="number" min="0" bind:value={cfg.llm.keep_warm_min} /><span>min</span></div>
          </div>
        </div>
      </section>

      <section>
        <h3>Models</h3>
        <div class="card">
          {#each [["Speech to text", settings.models.speech, "Bark (Parakeet) int8"], ["Model runtime", settings.models.server, "llama.cpp server"], ["Meeting summaries", settings.models.summary, cfg.llm.summary_model], ["Formatting & commands", settings.models.format, cfg.llm.format_model]] as [label, ok, detail]}
            <div class="row">
              <div class="label">
                <div>{label}</div>
                <div class="hint">{detail}</div>
              </div>
              <span class="status" class:ok>{ok ? "Installed" : "Missing"}</span>
            </div>
          {/each}
          <div class="row"><div class="hint mono">{settings.models.dir}</div></div>
        </div>
      </section>
    </div>

    {#if dirty || error || flash}
      <div class="savebar" class:error={!!error}>
        <span>{error ? error : flash ? "Saved" : "You have unsaved changes"}</span>
        {#if dirty}
          <button class="btn" onclick={() => { cfg = JSON.parse(saved); error = ""; }}>Discard</button>
          <button class="btn primary" onclick={save}>Save</button>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .wrap {
    height: 100%;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .wrap {
    container-type: inline-size;
  }
  header {
    max-width: 680px;
    margin: 0 auto;
    padding-top: 24px;
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
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 32px 100px;
  }
  section {
    max-width: 680px;
    margin: 22px auto 0;
  }
  @container (max-width: 560px) {
    header {
      padding-top: 14px;
    }
    h2 {
      font-size: 18px;
    }
    .scroll {
      padding: 0 12px 90px;
    }
    section {
      margin-top: 18px;
    }
    .row {
      padding: 12px 14px;
      gap: 16px;
    }
    .snippet {
      grid-template-columns: 1fr auto;
    }
    .snippet textarea {
      grid-column: 1;
    }
    .keys {
      min-width: 0;
    }
  }
  h3 {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    margin: 0 0 8px 4px;
  }
  .card {
    border-radius: 12px;
    background: var(--raised);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 14px 16px;
  }
  .row + .row {
    border-top: 1px solid var(--rule);
  }
  .row.stack {
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
  }
  .label {
    min-width: 0;
  }
  .hint {
    color: var(--muted);
    font-size: 12.5px;
    margin-top: 2px;
  }
  .mono {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 12px;
    user-select: text;
  }
  .keys {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 170px;
    justify-content: flex-end;
    padding: 4px 6px;
    border-radius: 8px;
    cursor: pointer;
    outline: none;
    border: 1px solid transparent;
  }
  .keys:hover,
  .keys:focus-visible {
    background: var(--hover);
  }
  .keys.recording {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .press {
    font-size: 12.5px;
    color: var(--accent-text);
  }
  kbd {
    font: 600 12px "Segoe UI", sans-serif;
    padding: 3px 8px;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    padding: 6px;
    border-radius: 9px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px 2px 10px;
    border-radius: 14px;
    background: var(--accent-soft);
    font-size: 12.5px;
  }
  .tag button {
    border: 0;
    background: transparent;
    color: var(--muted);
    display: grid;
    padding: 3px;
    border-radius: 50%;
  }
  .tag button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .tag-input {
    flex: 1;
    min-width: 160px;
    border: 0;
    outline: 0;
    background: transparent;
    height: 26px;
    padding: 0 6px;
  }
  .snippets {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .snippet {
    display: grid;
    grid-template-columns: 200px 1fr auto;
    gap: 8px;
    align-items: start;
  }
  .snippet textarea {
    height: auto;
    min-height: 32px;
    padding: 6px 10px;
    resize: vertical;
    field-sizing: content;
  }
  .add {
    align-self: flex-start;
  }
  .apps {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .app-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .app-row > span {
    flex: 1;
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 12.5px;
  }
  .wide {
    width: 100%;
  }
  .short {
    width: 160px;
  }
  .seg.full {
    display: flex;
  }
  .seg.full button {
    flex: 1;
  }
  .tags.plain {
    border: 0;
    padding: 0;
    background: transparent;
  }
  .swatches {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 4px;
  }
  .swatch {
    position: relative;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid transparent;
    background: transparent;
    cursor: pointer;
    transition: border-color 0.15s;
  }
  .swatch span {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    color: var(--muted);
  }
  .swatch.on {
    border-color: var(--text);
  }
  .swatch.auto span {
    background-image: linear-gradient(135deg, transparent 50%, rgba(255, 255, 255, 0.5) 50%);
  }
  .swatch.custom span {
    background: var(--hover);
  }
  .swatch.custom input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .num {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 13px;
  }
  .num input {
    width: 76px;
  }
  .status {
    font-size: 12px;
    font-weight: 600;
    padding: 2px 10px;
    border-radius: 12px;
    color: var(--danger);
    background: rgba(229, 72, 77, 0.1);
  }
  .status.ok {
    color: var(--ok);
    background: rgba(34, 160, 107, 0.12);
  }
  .savebar {
    position: absolute;
    left: 50%;
    bottom: 22px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 18px;
    border-radius: 12px;
    background: var(--raised);
    border: 1px solid var(--border-strong);
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.18);
    animation: rise 0.18s ease-out;
    white-space: nowrap;
  }
  .savebar span {
    margin-right: 10px;
    color: var(--muted);
  }
  .savebar.error span {
    color: var(--danger);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translate(-50%, 8px);
    }
  }
</style>
