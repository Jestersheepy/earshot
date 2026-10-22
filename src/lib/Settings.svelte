<script lang="ts">
  import { onMount } from "svelte";
  import { api, type Config, type Settings } from "$lib/api";
  import Icon from "$lib/Icon.svelte";

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
  }
  onMount(load);

  async function save() {
    if (!cfg) return;
    error = "";
    try {
      await api.saveSettings($state.snapshot(cfg) as Config);
      saved = JSON.stringify(cfg);
      flash = true;
      setTimeout(() => (flash = false), 1600);
      settings = await api.getSettings();
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
    <header>
      <h2>Settings</h2>
      <p class="sub">Everything runs on this PC. Nothing is sent anywhere.</p>
    </header>

    <div class="scroll">
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
  header {
    padding: 28px 32px 8px;
    max-width: 820px;
    width: 100%;
    margin: 0 auto;
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
    max-width: 756px;
    margin: 22px auto 0;
  }
  h3 {
    font-size: 12px;
    font-weight: 600;
    color: var(--faint);
    text-transform: uppercase;
    letter-spacing: 0.05em;
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
    border-top: 1px solid var(--border);
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
    box-shadow: 0 0 0 3px rgba(91, 140, 255, 0.18);
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
  .switch {
    position: relative;
    flex: none;
    width: 38px;
    height: 22px;
  }
  .switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }
  .switch span {
    position: absolute;
    inset: 0;
    border-radius: 22px;
    background: var(--border-strong);
    transition: background 0.18s;
    cursor: pointer;
  }
  .switch span::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: white;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: transform 0.18s;
  }
  .switch input:checked + span {
    background: var(--accent);
  }
  .switch input:checked + span::after {
    transform: translateX(16px);
  }
  .switch input:focus-visible + span {
    box-shadow: 0 0 0 3px rgba(91, 140, 255, 0.3);
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
    background: var(--selected);
    font-size: 13px;
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
  .seg {
    display: flex;
    padding: 2px;
    border-radius: 8px;
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .seg button {
    border: 0;
    background: transparent;
    padding: 3px 10px;
    border-radius: 6px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .seg button.on {
    background: var(--raised);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
  .wide {
    width: 100%;
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
