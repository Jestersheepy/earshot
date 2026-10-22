//! What the Notes window can ask of the app. Paths from the webview are checked against the notes folder.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::config::Config;
use crate::controller::AppEvent;
use crate::history::{History, Row};
use crate::library::{self, NoteDetail, NoteMeta, SearchHit};
use crate::llm::Pool;

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
pub async fn resummarize(path: String, state: State<'_, AppState>) -> R<String> {
    let p = note(&path)?;
    let pool = state.pool.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = Config::load();
        let model = cfg.llm_model(&cfg.llm.summary_model);
        if !pool.available(&model) {
            return Err(format!("Summary model not found: {}", model.display()));
        }
        let lines = library::transcript(&p).map_err(err)?;
        if !crate::meeting::worth_summarizing(&lines) {
            return Err("This transcript is too short to summarize.".into());
        }
        let d = library::detail(&p).map_err(err)?;
        let head = format!("Title: {}\nDate: {}\nDuration: {}", d.meta.title, d.meta.date, d.meta.duration);
        let summary = crate::meeting::summarize(&pool, &model, cfg.llm.ctx, &lines, &head).map_err(err)?;
        library::set_summary(&p, &summary).map_err(err)?;
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
    Settings { config: cfg, models, autostart: app.autolaunch().is_enabled().unwrap_or(false) }
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
