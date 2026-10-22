import { invoke, convertFileSrc } from "@tauri-apps/api/core";

export type NoteMeta = {
  path: string;
  kind: "meeting" | "voice-note" | string;
  title: string;
  date: string;
  duration: string;
  app: string | null;
  audio: string | null;
  preview: string;
  words: number;
};
export type Line = { t: number; speaker: string; text: string };
export type SummaryMode = "concise" | "standard" | "nuanced";
export type NoteDetail = {
  meta: NoteMeta;
  summaries: Partial<Record<SummaryMode, string>>;
  lines: Line[];
  body: string;
  /** Speaker names guessed from a known voice, not yet confirmed. */
  unconfirmed: string[];
};
export type SearchHit = { meta: NoteMeta; snippet: string };
export type HistoryRow = {
  id: number;
  ts: string;
  mode: string;
  app: string | null;
  raw: string;
  text: string;
  llm: boolean;
  ms: number;
};
export type Snippet = { trigger: string; text: string };
export type Config = {
  notes_dir: string;
  models_dir: string | null;
  hotkeys: Record<"dictate" | "command" | "voice_note" | "meeting" | "paste_last" | "toggle_pill", string>;
  stt_threads: number;
  idle_unload_min: number;
  preload: boolean;
  keep_wav: boolean;
  smart_format: boolean;
  literal_apps: string[];
  llm: { server: string | null; summary_model: string; format_model: string; ctx: number; threads: number; keep_warm_min: number };
  pill: {
    x: number | null;
    y: number | null;
    hidden: boolean;
    size: "small" | "medium" | "large";
    idle: "dot" | "mic" | "breathing";
    accent: string;
    peek: boolean;
    click: "dictate" | "note" | "meeting" | "panel";
  };
  dictionary: string[];
  snippets: Snippet[];
  meeting_apps: Record<string, "ask" | "always" | "never">;
  detect_meetings: boolean;
  summary_mode: SummaryMode;
  cloud: Cloud;
  detect_speakers: boolean;
  your_name: string;
  theme: "system" | "light" | "dark";
  sounds: boolean;
};
export type Provider = "anthropic" | "openai" | "custom";
export type Cloud = { enabled: boolean; provider: Provider; model: string; url: string };
export type Settings = {
  config: Config;
  models: { dir: string; speech: boolean; server: boolean; summary: boolean; format: boolean };
  autostart: boolean;
  /** The dot's accent, resolved to #rrggbb. */
  accent: string;
  /** Cloud providers with an API key saved. */
  cloud_keys: Provider[];
};
export type SetupProgress = {
  state: "idle" | "downloading" | "verifying" | "done" | "failed";
  done: number;
  total: number;
  part: string;
  speech_ready: boolean;
  error: string | null;
};
/** Bytes still to download: `needed` without the summary model, `summaries` for it. */
export type SetupStatus = { needed: number; summaries: number; speech_ready: boolean; progress: SetupProgress | null };
export type Stats ={ today_words: number; week_words: number; total_words: number; today_count: number };
/** `anchor`: where the dot is, as fractions of the panel's size (null when it isn't beside the dot). */
export type PanelMode = { expanded: boolean; pinned: boolean; anchor: [number, number] | null };

export const api = {
  listNotes: () => invoke<NoteMeta[]>("list_notes"),
  searchNotes: (query: string) => invoke<SearchHit[]>("search_notes", { query }),
  getNote: (path: string) => invoke<NoteDetail>("get_note", { path }),
  renameNote: (path: string, title: string) => invoke<void>("rename_note", { path, title }),
  deleteNote: (path: string) => invoke<void>("delete_note", { path }),
  resummarize: (path: string, mode: SummaryMode) => invoke<string>("resummarize", { path, mode }),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  openNotesFolder: () => invoke<void>("open_notes_folder"),
  listHistory: (query: string, limit = 300) => invoke<HistoryRow[]>("list_history", { query, limit }),
  deleteHistory: (id: number) => invoke<void>("delete_history", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (config: Config) => invoke<void>("save_settings", { config }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setupStatus: () => invoke<SetupStatus>("setup_status"),
  setupStart: (summaries: boolean) => invoke<void>("setup_start", { summaries }),
  setCloudKey:(provider: Provider, key: string) => invoke<void>("set_cloud_key", { provider, key }),
  testCloud: (cloud: Cloud, key: string | null) => invoke<string>("test_cloud", { cloud, key }),
  renameSpeaker: (path: string, from: string, to: string) => invoke<void>("rename_speaker", { path, from, to }),
  suggestSpeakerNames: (path: string) => invoke<[string, string][]>("suggest_speaker_names", { path }),
  listVoices: () => invoke<[string, number][]>("list_voices"),
  forgetVoice: (name: string) => invoke<void>("forget_voice", { name }),
  historyStats: () => invoke<Stats>("history_stats"),
  runAction: (action: "note" | "meeting") => invoke<void>("run_action", { action }),
  panel: {
    expand: (expanded: boolean) => invoke<void>("panel_expand", { expanded }),
    pin: (pinned: boolean) => invoke<void>("panel_pin", { pinned }),
    close: () => invoke<void>("panel_close"),
    minimize: () => invoke<void>("panel_minimize"),
  },
};

/** Theme and accent from Settings, applied to the page. */
export function applyLook(s: Settings) {
  const root = document.documentElement;
  if (s.config.theme === "system") delete root.dataset.theme;
  else root.dataset.theme = s.config.theme;
  root.style.setProperty("--accent", s.accent);
}

/** Minutes saved by speaking instead of typing (about 40 wpm typed vs 150 spoken). */
export const minutesSaved = (words: number) => Math.round(words / 40 - words / 150);

/** A stable colour per speaker label. */
const HUES = ["#4f6ef7", "#14a38b", "#e29a1d", "#d6457a", "#8b6cf0", "#3a9bd9", "#7a8f2e"];
export function speakerColor(name: string, order: string[], me: string) {
  if (name === me || name === "Me") return "var(--accent)";
  const i = order.filter((n) => n !== me && n !== "Me").indexOf(name);
  return HUES[(i < 0 ? 0 : i + 1) % HUES.length];
}
export const initials = (name: string) =>
  name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => (/^\d+$/.test(w) ? w : w[0].toUpperCase()))
    .join("");

export const audioUrl = (path: string) => convertFileSrc(path);

const pad = (n: number) => String(n).padStart(2, "0");
export const clock = (s: number) => {
  s = Math.max(0, Math.floor(s));
  const h = Math.floor(s / 3600);
  return h ? `${h}:${pad(Math.floor(s / 60) % 60)}:${pad(s % 60)}` : `${Math.floor(s / 60)}:${pad(s % 60)}`;
};

const parse = (iso: string) => new Date(iso.length === 19 ? iso : iso.replace(" ", "T"));
export const dayLabel = (iso: string) => {
  const d = parse(iso);
  const today = new Date();
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "Today";
  if (d.toDateString() === y.toDateString()) return "Yesterday";
  return d.toLocaleDateString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: d.getFullYear() === today.getFullYear() ? undefined : "numeric",
  });
};
export const timeOf = (iso: string) => parse(iso).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
export const longDate = (iso: string) =>
  parse(iso).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
export const appName = (app: string | null) => (app ?? "").replace(/\.exe$/i, "");
