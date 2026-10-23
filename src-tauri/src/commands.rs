//! What the Notes window can ask of the app. Paths from the webview are checked against the notes folder.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use earshot_core::audio::{Capture, Source};
use earshot_core::stt::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::backend::{PROVIDERS, Summarizer};
use crate::config::{Cloud, Config};
use crate::controller::AppEvent;
use crate::history::{History, Row};
use crate::library::{self, NoteDetail, NoteMeta, SearchHit};
use crate::llm::Pool;
use crate::search::{self, Filters, Searcher};
use crate::summary::Mode;

pub struct AppState {
    pub pool: Pool,
    pub history: Option<Arc<History>>,
    pub tx: Sender<AppEvent>,
    pub search: Option<Searcher>,
    pub engine: Arc<Engine>,
    /// The mic while a search is being spoken.
    pub listening: parking_lot::Mutex<Option<Capture>>,
}

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn note(path: &str) -> R<std::path::PathBuf> {
    library::checked(&Config::load().notes_dir, path).map_err(err)
}

#[tauri::command]
pub fn list_notes() -> Vec<NoteMeta> {
    library::list(&Config::load().notes_dir)
}

#[tauri::command]
pub fn search_notes(query: String) -> Vec<SearchHit> {
    library::search(&Config::load().notes_dir, &query)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    meta: NoteMeta,
    snippet: String,
    /// Seconds into the recording, for a transcript passage.
    t: Option<u32>,
    speakers: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Searched {
    hits: Vec<Found>,
    /// What the search was narrowed to, shown as chips.
    filters: Filters,
    /// False when the search model isn't downloaded and this fell back to matching words.
    by_meaning: bool,
}

/// Search by meaning, narrowed to any person, time or kind of note the query names.
#[tauri::command]
pub async fn smart_search(query: String, state: State<'_, AppState>) -> R<Searched> {
    let (history, searcher) = (state.history.clone(), state.search.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = Config::load();
        let Some(searcher) = searcher.filter(|_| crate::setup::search_ready(&cfg)) else {
            let hits = library::search(&cfg.notes_dir, &query);
            return Ok(Searched {
                hits: hits.into_iter().map(|h| Found { meta: h.meta, snippet: h.snippet, t: None, speakers: Vec::new() }).collect(),
                filters: Filters::default(),
                by_meaning: false,
            });
        };
        let notes = library::list(&cfg.notes_dir);
        // everyone named in a note, plus voices you've named
        let mut names: Vec<String> = notes.iter().flat_map(|m| m.participants.iter().cloned()).collect();
        names.extend(history.iter().flat_map(|h| h.voices().unwrap_or_default()).map(|(n, _)| n));
        names.sort();
        names.dedup();
        let (text, filters) = search::read_query(&query, &names, chrono::Local::now().date_naive());
        let found = searcher.search(&cfg.notes_dir, &text, &filters, 30).map_err(err)?;
        let metas: std::collections::HashMap<String, NoteMeta> = notes.into_iter().map(|m| (m.path.clone(), m)).collect();
        let hits = found
            .into_iter()
            .filter_map(|h| Some(Found { meta: metas.get(&h.path)?.clone(), snippet: h.text, t: h.t, speakers: h.speakers }))
            .collect();
        Ok(Searched { hits, filters, by_meaning: true })
    })
    .await
    .map_err(err)?
}

/// Start listening for a spoken search.
#[tauri::command]
pub fn listen_start(state: State<'_, AppState>) -> R<()> {
    if !crate::setup::speech_ready(&Config::load()) {
        return Err("The speech model isn't set up yet.".into());
    }
    let mut mic = state.listening.lock();
    if mic.is_none() {
        *mic = Some(Capture::start(Source::Mic, std::time::Instant::now()).map_err(err)?);
    }
    Ok(())
}

/// Stop listening and return what was said.
#[tauri::command]
pub async fn listen_stop(state: State<'_, AppState>) -> R<String> {
    let Some(cap) = state.listening.lock().take() else { return Ok(String::new()) };
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let wave = cap.stop().read(0, None);
        let cfg = Config::load();
        let raw = engine.transcribe(&wave).map_err(err)?;
        let said = crate::notes::clean(&crate::text::apply_dictionary(&raw, &cfg.dictionary), true);
        Ok(said.trim_end_matches(['.', '?', '!']).to_string())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn get_note(path: String) -> R<NoteDetail> {
    library::detail(&note(&path)?).map_err(err)
}

#[tauri::command]
pub fn rename_note(path: String, title: String) -> R<()> {
    library::set_front(&note(&path)?, "title", title.trim()).map_err(err)
}

/// Rename a meeting speaker everywhere in the note, and remember their voice for next time.
#[tauri::command]
pub fn rename_speaker(path: String, from: String, to: String, state: State<'_, AppState>) -> R<()> {
    let (p, to) = (note(&path)?, to.trim().to_string());
    if to.is_empty() || to.contains([':', '*', '\n', ',']) {
        return Err("Names can't be empty or contain : * or commas.".into());
    }
    let md = std::fs::read_to_string(&p).map_err(err)?.replace("\r\n", "\n");
    std::fs::write(&p, crate::speakers::rename_in_note(&md, &from, &to)).map_err(err)?;
    if let Some(h) = &state.history {
        let cfg = Config::load();
        let learn = to != cfg.your_name && to != "Me";
        h.name_speaker(&p.to_string_lossy(), &from, &to, learn).map_err(err)?;
    }
    Ok(())
}

/// Names the transcript gives away for "Speaker N" labels ("Thanks, Sarah"), from the local model. Only
/// names that appear in the transcript are returned.
#[tauri::command]
pub async fn suggest_speaker_names(path: String, state: State<'_, AppState>) -> R<Vec<(String, String)>> {
    let p = note(&path)?;
    let pool = state.pool.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = Config::load();
        let lines = library::transcript(&p).map_err(err)?;
        let labels: Vec<String> = crate::speakers::participants(&lines).into_iter().filter(|s| s.starts_with("Speaker ")).collect();
        if labels.is_empty() {
            return Ok(Vec::new());
        }
        let by = Summarizer::new(&cfg, &pool).map_err(err)?;
        let text = crate::summary::transcript_text(&lines).join("\n");
        let ask = format!(
            "Some speakers in this meeting transcript are labelled {}. For each of those labels, give the person's name only \
             if the transcript makes it clear: they introduce themselves, are addressed by name just before they speak, or \
             are thanked or answered by name just after. Answer one line per label as 'Speaker N: Name', or 'Speaker N: unknown'.\n\n{text}",
            labels.join(", ")
        );
        let answer = by.chat("You read meeting transcripts carefully and never guess.", &ask, 200, 0.0).map_err(err)?;
        let lower = text.to_lowercase();
        Ok(answer
            .lines()
            .filter_map(|l| l.trim().trim_start_matches(['-', '*', ' ']).split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches(['*', '.', ' ']).to_string()))
            .filter(|(k, v)| labels.contains(k) && !v.is_empty() && !v.eq_ignore_ascii_case("unknown") && v.len() < 40)
            .filter(|(_, v)| lower.contains(&v.to_lowercase()))
            .collect())
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn list_voices(state: State<'_, AppState>) -> R<Vec<(String, i64)>> {
    state.history.as_ref().map_or(Ok(Vec::new()), |h| h.voice_names().map_err(err))
}

#[tauri::command]
pub fn forget_voice(name: String, state: State<'_, AppState>) -> R<()> {
    state.history.as_ref().map_or(Ok(()), |h| h.forget_voice(&name).map_err(err))
}

/// To the Recycle Bin, with its recording.
#[tauri::command]
pub fn delete_note(path: String) -> R<()> {
    let p = note(&path)?;
    let audio = library::detail(&p).ok().and_then(|d| d.meta.audio).map(std::path::PathBuf::from);
    trash::delete(&p).map_err(err)?;
    if let Some(a) = audio {
        let _ = trash::delete(a);
    }
    Ok(())
}

#[tauri::command]
pub async fn resummarize(path: String, mode: Mode, state: State<'_, AppState>) -> R<String> {
    let p = note(&path)?;
    let pool = state.pool.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = Config::load();
        let by = Summarizer::new(&cfg, &pool).map_err(err)?;
        let lines = library::transcript(&p).map_err(err)?;
        if !crate::meeting::worth_summarizing(&lines) {
            return Err("This transcript is too short to summarize.".into());
        }
        let d = library::detail(&p).map_err(err)?;
        let head = format!("Title: {}\nDate: {}\nDuration: {}", d.meta.title, d.meta.date, d.meta.duration);
        // older notes also get the title, chapters and action items newer ones are saved with
        let want = library::missing_extras(&p).map_err(err)?;
        let w = crate::summary::summarize(&by, &lines, &head, mode, want).map_err(err)?;
        library::set_summary(&p, mode, &w.summary).map_err(err)?;
        library::save_extras(&p, &w).map_err(err)?;
        Ok(w.summary)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn list_tasks() -> Vec<library::TaskRow> {
    library::all_tasks(&Config::load().notes_dir)
}

#[tauri::command]
pub fn set_task(path: String, index: usize, done: bool) -> R<()> {
    library::set_task_done(&note(&path)?, index, done).map_err(err)
}

/// Open a dated action item as an all-day event in the default calendar app.
#[tauri::command]
pub fn task_to_calendar(path: String, index: usize) -> R<()> {
    let d = library::detail(&note(&path)?).map_err(err)?;
    let task = d.tasks.get(index).ok_or("no such task")?;
    let ics = crate::tasks::ics(task, &d.meta.title).ok_or("this action item has no due date")?;
    let file = std::env::temp_dir().join(format!("earshot-task-{}.ics", chrono::Local::now().format("%Y%m%d%H%M%S")));
    std::fs::write(&file, ics).map_err(err)?;
    std::process::Command::new("explorer").arg(&file).spawn().map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn reveal(path: String) -> R<()> {
    let p = note(&path)?;
    std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn().map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn open_notes_folder() -> R<()> {
    let dir = Config::load().notes_dir;
    std::fs::create_dir_all(&dir).map_err(err)?;
    std::process::Command::new("explorer").arg(dir).spawn().map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn list_history(query: String, limit: u32, state: State<'_, AppState>) -> R<Vec<Row>> {
    match &state.history {
        Some(h) => h.list(&query, limit.min(1000)).map_err(err),
        None => Ok(Vec::new()),
    }
}

#[tauri::command]
pub fn history_stats(state: State<'_, AppState>) -> R<crate::history::Stats> {
    state.history.as_ref().map_or(Ok(Default::default()), |h| h.stats().map_err(err))
}

#[tauri::command]
pub fn delete_history(id: i64, state: State<'_, AppState>) -> R<()> {
    state.history.as_ref().map_or(Ok(()), |h| h.delete(id).map_err(err))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Models {
    dir: String,
    speech: bool,
    server: bool,
    summary: bool,
    format: bool,
}

#[derive(Serialize)]
pub struct Settings {
    config: Config,
    models: Models,
    autostart: bool,
    /// The dot's accent as "#rrggbb" ("auto" resolved to the Windows accent).
    accent: String,
    /// Cloud providers with an API key saved.
    cloud_keys: Vec<&'static str>,
}

#[tauri::command]
pub fn setup_status() -> crate::setup::Status {
    crate::setup::status(&Config::load())
}

/// Download the missing models (`summaries`: include the local summary model); progress comes as
/// "setup-progress" events.
#[tauri::command]
pub fn setup_start(summaries: bool, app: AppHandle, state: State<'_, AppState>) {
    crate::setup::start(app, &Config::load(), summaries, state.tx.clone());
}

/// Save (or, when empty, remove) a cloud provider's API key in Windows Credential Manager.
#[tauri::command]
pub fn set_cloud_key(provider: String, key: String) -> R<()> {
    let p = PROVIDERS.iter().find(|p| **p == provider).ok_or("unknown provider")?;
    earshot_core::secret::set(p, key.trim()).map_err(err)
}

/// One small request with these (possibly unsaved) settings; returns the model's reply.
#[tauri::command]
pub async fn test_cloud(cloud: Cloud, key: Option<String>) -> R<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let key = key.filter(|k| !k.trim().is_empty()).map(|k| k.trim().to_string()).or_else(|| earshot_core::secret::get(&cloud.provider));
        let by = Summarizer::cloud(&cloud, key, Config::load().llm.ctx).map_err(err)?;
        by.chat("Answer in one word.", "Say OK.", 16, 0.0).map_err(err)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    let cfg = Config::load();
    let dir = cfg.models_dir();
    let models = Models {
        dir: dir.to_string_lossy().into_owned(),
        speech: dir.join("bark-onnx").join("encoder.int8.onnx").exists(),
        server: cfg.llama_server().exists(),
        summary: cfg.llm_model(&cfg.llm.summary_model).exists(),
        format: cfg.llm_model(&cfg.llm.format_model).exists(),
    };
    let accent = match cfg.pill.accent.as_str() {
        "auto" => earshot_overlay::theme::windows_accent(),
        hex => earshot_overlay::theme::parse_hex(hex),
    }
    .unwrap_or(earshot_overlay::theme::DEFAULT_ACCENT);
    let accent = format!("#{:02x}{:02x}{:02x}", accent.0, accent.1, accent.2);
    let cloud_keys = PROVIDERS.into_iter().filter(|p| earshot_core::secret::get(p).is_some()).collect();
    Settings { config: cfg, models, autostart: app.autolaunch().is_enabled().unwrap_or(false), accent, cloud_keys }
}

/// Start or stop a voice note or meeting from the panel ("note" | "meeting").
#[tauri::command]
pub fn run_action(action: String, state: State<'_, AppState>) -> R<()> {
    if !matches!(action.as_str(), "note" | "meeting") {
        return Err(format!("unknown action {action}"));
    }
    state.tx.send(AppEvent::Action(action)).map_err(err)
}

#[tauri::command]
pub fn panel_expand(expanded: bool, app: AppHandle) {
    crate::window::set_expanded(&app, expanded);
}

#[tauri::command]
pub fn panel_pin(pinned: bool, app: AppHandle) {
    crate::window::set_pinned(&app, pinned);
}

#[tauri::command]
pub fn panel_close(app: AppHandle) {
    crate::window::close(&app);
}

#[tauri::command]
pub fn panel_minimize(app: AppHandle) {
    crate::window::minimize(&app);
}

#[tauri::command]
pub fn set_autostart(enabled: bool, app: AppHandle) -> R<()> {
    let a = app.autolaunch();
    if enabled { a.enable() } else { a.disable() }.map_err(err)
}

/// Validate, save, and hand to the running app (hotkeys rebind immediately).
#[tauri::command]
pub fn save_settings(config: Config, app: AppHandle, state: State<'_, AppState>) -> R<()> {
    crate::bindings(&config).map_err(|e| format!("Hotkey: {e}"))?;
    config.save();
    let _ = app.asset_protocol_scope().allow_directory(&config.notes_dir, true);
    state.tx.send(AppEvent::Reload(Box::new(config))).map_err(err)
}
