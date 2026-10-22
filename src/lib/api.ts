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
export type NoteDetail = { meta: NoteMeta; summary: string | null; lines: Line[]; body: string };
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
  pill: { x: number | null; y: number | null; hidden: boolean };
  dictionary: string[];
  snippets: Snippet[];
  meeting_apps: Record<string, "ask" | "always" | "never">;
  detect_meetings: boolean;
};
export type Settings = {
  config: Config;
  models: { dir: string; speech: boolean; server: boolean; summary: boolean; format: boolean };
};

export const api = {
  listNotes: () => invoke<NoteMeta[]>("list_notes"),
  searchNotes: (query: string) => invoke<SearchHit[]>("search_notes", { query }),
  getNote: (path: string) => invoke<NoteDetail>("get_note", { path }),
  renameNote: (path: string, title: string) => invoke<void>("rename_note", { path, title }),
  deleteNote: (path: string) => invoke<void>("delete_note", { path }),
  resummarize: (path: string) => invoke<string>("resummarize", { path }),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  openNotesFolder: () => invoke<void>("open_notes_folder"),
  listHistory: (query: string, limit = 300) => invoke<HistoryRow[]>("list_history", { query, limit }),
  deleteHistory: (id: number) => invoke<void>("delete_history", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (config: Config) => invoke<void>("save_settings", { config }),
};

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
