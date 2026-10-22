//! What the Notes window can ask of the app. Paths from the webview are checked against the notes folder.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::backend::{PROVIDERS, Summarizer};
use crate::config::{Cloud, Config};
use crate::controller::AppEvent;
use crate::history::{History, Row};
use crate::library::{self, NoteDetail, NoteMeta, SearchHit};
use crate::llm::Pool;
use crate::summary::Mode;

pub struct AppState {
    pub pool: Pool,
    pub history: Option<Arc<History>>,
    pub tx: Sender<AppEvent>,
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
        let summary = crate::summary::summarize(&by, &lines, &head, mode).map_err(err)?;
        library::set_summary(&p, mode, &summary).map_err(err)?;
        Ok(summary)
    })
    .await
    .map_err(err)?
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
