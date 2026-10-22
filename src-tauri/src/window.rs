//! The Notes window: created when asked for and destroyed on close, so WebView2 only costs memory
//! while it's open.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// `section`: "notes" | "meetings" | "voice" | "history" | "settings".
pub fn show(app: &AppHandle, section: &str) {
    open(app, "navigate", section);
}

/// Open the window on one note.
pub fn show_note(app: &AppHandle, path: &std::path::Path) {
    open(app, "open-note", &path.to_string_lossy());
}

/// `event` + `arg` reach a running window as an event, or a new one through an init script.
fn open(app: &AppHandle, event: &str, arg: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.emit(event, arg);
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Earshot")
        .inner_size(1140.0, 760.0)
        .min_inner_size(780.0, 500.0)
        .center()
        .initialization_script(format!("window.__EARSHOT_START__ = {};", serde_json::json!({ "event": event, "arg": arg })))
        .build();
    if let Err(e) = built {
        eprintln!("opening the Notes window: {e}");
    }
}
