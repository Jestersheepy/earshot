mod commands;
pub mod config;
mod controller;
mod history;
pub mod library;
pub mod llm;
mod meeting;
mod notes;
mod speakers;
pub mod summary;
pub mod text;
mod window;

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use earshot_core::hotkeys::{self, Bindings, Combo};
use earshot_core::stt::Engine;
use earshot_overlay::Overlay;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, RunEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use config::Config;
use controller::{AppEvent, Controller};

/// Passed by the sign-in launch so it can stay quiet.
const AUTOSTART_ARG: &str = "--autostart";

pub(crate) fn bindings(cfg: &Config) -> anyhow::Result<Bindings> {
    let hk = &cfg.hotkeys;
    let mut toggles = Vec::new();
    for (name, combo) in [
        ("voice_note", &hk.voice_note),
        ("meeting", &hk.meeting),
        ("paste_last", &hk.paste_last),
        ("toggle_pill", &hk.toggle_pill),
    ] {
        toggles.push((name.to_string(), Combo::parse(combo)?));
    }
    Ok(Bindings { dictate: Combo::parse(&hk.dictate)?, command: Combo::parse(&hk.command)?, toggles })
}

/// Pipe events from another component's channel into the controller's.
fn forward<T: Send + 'static>(tx: &Sender<AppEvent>, wrap: fn(T) -> AppEvent) -> Sender<T> {
    let (t, r) = channel::<T>();
    let tx = tx.clone();
    std::thread::spawn(move || {
        for ev in r {
            if tx.send(wrap(ev)).is_err() {
                break;
            }
        }
    });
    t
}

fn tray(app: &tauri::App, tx: Sender<AppEvent>) -> tauri::Result<()> {
    let item = |id: &str, label: &str, combo: Option<&str>| {
        MenuItem::with_id(app, id, label, true, combo.map(controller::pretty_combo).filter(|s| !s.is_empty()))
    };
    let menu = Menu::with_items(
        app,
        &[
            &item("note", "Voice note (start / stop)", None)?,
            &item("meeting", "Meeting (start / stop)", None)?,
            &PredefinedMenuItem::separator(app)?,
            &item("open", "Open Earshot", None)?,
            &item("history", "Dictation history", None)?,
            &item("toggle", "Show / hide pill", None)?,
            &item("settings", "Settings", None)?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Quit Earshot", None)?,
        ],
    )?;
    let tx2 = tx.clone();
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("bundle icon"))
        .tooltip("Earshot")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |_, e| {
            let _ = tx.send(AppEvent::Action(e.id().as_ref().to_string()));
        })
        .on_tray_icon_event(move |_, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                let _ = tx2.send(AppEvent::Action("toggle".into()));
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (tx, rx) = channel::<AppEvent>();
    let tx_dup = tx.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(move |_, _, _| {
            let _ = tx_dup.send(AppEvent::Action("open".into()));
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![AUTOSTART_ARG])))
        .invoke_handler(tauri::generate_handler![
            commands::panel_expand,
            commands::history_stats,
            commands::run_action,
            commands::panel_pin,
            commands::panel_close,
            commands::panel_minimize,
            commands::list_notes,
            commands::search_notes,
            commands::get_note,
            commands::rename_note,
            commands::delete_note,
            commands::resummarize,
            commands::reveal,
            commands::open_notes_folder,
            commands::list_history,
            commands::delete_history,
            commands::get_settings,
            commands::save_settings,
            commands::set_autostart,
            commands::rename_speaker,
            commands::suggest_speaker_names,
            commands::list_voices,
            commands::forget_voice,
        ])
        .setup(move |app| {
            // First run: start with Windows by default (Settings can turn it off).
            if !config::path().exists() {
                let _ = app.autolaunch().enable();
                Config::load().save();
            }
            let cfg = Config::load();
            let engine = Arc::new(Engine::new(
                cfg.models_dir().join("bark-onnx"),
                cfg.stt_threads,
                Duration::from_secs(cfg.idle_unload_min * 60),
            ));
            if cfg.preload {
                let e = engine.clone();
                std::thread::spawn(move || {
                    if let Err(err) = e.preload() {
                        eprintln!("preloading speech model: {err:#}");
                    }
                });
            }
            let pool = llm::Pool::new(cfg.llama_server(), cfg.llm.threads, Duration::from_secs(cfg.llm.keep_warm_min * 60));
            let pos = cfg.pill.x.zip(cfg.pill.y);
            let overlay = Overlay::start(forward(&tx, AppEvent::Ui), pos, !cfg.pill.hidden);
            match bindings(&cfg) {
                Ok(b) => hotkeys::start(b, forward(&tx, AppEvent::Hotkey)),
                Err(e) => eprintln!("hotkeys: {e:#}"),
            }
            // One registry read every 3 s; the controller applies the detection settings.
            let mic = forward(&tx, AppEvent::Mic);
            earshot_core::micwatch::watch(mic, Duration::from_secs(3), Duration::from_secs(5));
            tray(app, tx.clone())?;
            let history = history::History::open().map(Arc::new).map_err(|e| eprintln!("history: {e:#}")).ok();
            app.manage(commands::AppState { pool: pool.clone(), history: history.clone(), tx: tx.clone() });
            let _ = app.asset_protocol_scope().allow_directory(&cfg.notes_dir, true);
            let ctl = Controller::new(cfg, engine, pool, history, tx.clone(), overlay, app.handle().clone());
            std::thread::spawn(move || ctl.run(rx));
            // Started by hand rather than at sign-in: say where Earshot lives, since it has no window.
            if !std::env::args().any(|a| a == AUTOSTART_ARG) {
                let _ = tx.send(AppEvent::Action("welcome".into()));
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Earshot")
        .run(|_, ev| {
            // No windows is the normal state: only quit when asked to.
            if let RunEvent::ExitRequested { api, code: None, .. } = ev {
                api.prevent_exit();
            }
        });
}
