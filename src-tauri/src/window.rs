//! The Earshot panel: a compact, frameless window that opens beside the dot and can expand into a full
//! window. Clicking elsewhere hides it (unless pinned); a hidden panel is destroyed after a few minutes so
//! WebView2 only costs memory while you use it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

const LABEL: &str = "main";
const COMPACT: (f64, f64) = (400.0, 580.0);
const EXPANDED: (f64, f64) = (1100.0, 760.0);
const GAP: f64 = 12.0;
/// Half the dot's width, so the panel clears it.
const DOT: f64 = 22.0;
const DESTROY_AFTER: Duration = Duration::from_secs(180);

/// The pill's centre and its monitor's work area (left, top, right, bottom), physical px.
pub type Place = ((i32, i32), (i32, i32, i32, i32));

static EXPANDED_NOW: AtomicBool = AtomicBool::new(false);
static PINNED: AtomicBool = AtomicBool::new(false);
static HIDE_GEN: AtomicU64 = AtomicU64::new(0);
static LAST_PLACE: Mutex<Option<Place>> = Mutex::new(None);

/// Transparent room around the compact panel for its shadow, which the page draws (logical px).
const MARGIN: f64 = 18.0;
/// How long the page gets to animate out before the window hides.
const LEAVE: Duration = Duration::from_millis(200);

/// What the page needs to lay itself out and animate in from the dot.
#[derive(Serialize, Clone)]
pub struct Mode {
    pub expanded: bool,
    pub pinned: bool,
    /// Where the dot is, as fractions of the panel's width and height, so it can grow out of it.
    /// None when the panel isn't beside the dot.
    pub anchor: Option<(f64, f64)>,
}

/// `section`: "home" | "notes" | "meetings" | "voice" | "history" | "settings". `place`: where the dot is,
/// or None when it's hidden (the panel then opens expanded and centred).
pub fn show(app: &AppHandle, section: &str, place: Option<Place>) {
    open(app, "navigate", section, place);
}

/// Open the panel on one note.
pub fn show_note(app: &AppHandle, path: &std::path::Path, place: Option<Place>) {
    open(app, "open-note", &path.to_string_lossy(), place);
}

/// Create the panel hidden, so opening it from the menu is instant.
pub fn prewarm(app: &AppHandle) {
    if app.get_webview_window(LABEL).is_none() {
        let _ = build(app, None);
        HIDE_GEN.fetch_add(1, Ordering::SeqCst);
        schedule_destroy(app.clone());
    }
}

fn open(app: &AppHandle, event: &str, arg: &str, place: Option<Place>) {
    HIDE_GEN.fetch_add(1, Ordering::SeqCst);
    let start = serde_json::json!({ "event": event, "arg": arg });
    let w = match app.get_webview_window(LABEL) {
        Some(w) => {
            let _ = w.emit(event, arg);
            w
        }
        None => match build(app, Some(&start)) {
            Some(w) => w,
            None => return,
        },
    };
    if place.is_some() {
        *LAST_PLACE.lock() = place;
    }
    let expanded = place.is_none() || (EXPANDED_NOW.load(Ordering::SeqCst) && w.is_visible().unwrap_or(false));
    arrange(&w, expanded, place);
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

fn build(app: &AppHandle, start: Option<&serde_json::Value>) -> Option<WebviewWindow> {
    let start = start.cloned().unwrap_or(serde_json::Value::Null);
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Earshot")
        .inner_size(COMPACT.0, COMPACT.1)
        .decorations(false)
        // Compact, the page draws the panel's corners and shadow so it can grow out of the dot; expanded,
        // Windows 11 draws them (see arrange).
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .initialization_script(format!("window.__EARSHOT_START__ = {start};"))
        .build();
    let w = match built {
        Ok(w) => w,
        Err(e) => {
            eprintln!("opening the Earshot panel: {e}");
            return None;
        }
    };
    let handle = w.clone();
    w.on_window_event(move |e| match e {
        WindowEvent::Focused(false) if !EXPANDED_NOW.load(Ordering::SeqCst) && !PINNED.load(Ordering::SeqCst) => leave(&handle),
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            leave(&handle);
        }
        _ => {}
    });
    Some(w)
}

/// Size and place the panel: compact beside the dot, or expanded and centred on the dot's monitor.
fn arrange(w: &WebviewWindow, expanded: bool, place: Option<Place>) {
    EXPANDED_NOW.store(expanded, Ordering::SeqCst);
    let place = place.or(*LAST_PLACE.lock());
    let monitor = match place {
        Some(((x, y), _)) => w.app_handle().monitor_from_point(x as f64, y as f64).ok().flatten(),
        None => w.current_monitor().ok().flatten(),
    };
    let s = monitor.as_ref().map_or(1.0, |m| m.scale_factor());
    let (lw, lh) = if expanded { EXPANDED } else { COMPACT };
    let (pw, ph) = ((lw * s) as i32, (lh * s) as i32);
    let work = match (place, &monitor) {
        (Some((_, wa)), _) => wa,
        (None, Some(m)) => {
            let (p, z) = (m.position(), m.size());
            (p.x, p.y, p.x + z.width as i32, p.y + z.height as i32)
        }
        (None, None) => (0, 0, 1920, 1080),
    };
    let (pos, anchor) = match (expanded, place) {
        (false, Some((dot, _))) => {
            let (pos, _) = beside(dot, work, (pw, ph), s);
            let frac = |d: i32, p: i32, len: i32| ((d - p) as f64 / len as f64).clamp(0.0, 1.0);
            (pos, Some((frac(dot.0, pos.0, pw), frac(dot.1, pos.1, ph))))
        }
        _ => {
            let (l, t, r, b) = work;
            (((l + r - pw) / 2, (t + b - ph) / 2), None)
        }
    };
    // compact, the window is the panel plus a transparent margin for the shadow the page draws
    let m = if expanded { 0 } else { (MARGIN * s) as i32 };
    let _ = w.set_shadow(expanded);
    let _ = w.set_skip_taskbar(!expanded);
    let _ = w.set_resizable(expanded);
    let min = if expanded { (780.0, 500.0) } else { (COMPACT.0 + 2.0 * MARGIN, COMPACT.1 + 2.0 * MARGIN) };
    let _ = w.set_min_size(Some(tauri::Size::Logical(min.into())));
    let _ = w.set_size(PhysicalSize::new((pw + 2 * m) as u32, (ph + 2 * m) as u32));
    let _ = w.set_position(PhysicalPosition::new(pos.0 - m, pos.1 - m));
    let _ = w.emit("panel-mode", Mode { expanded, pinned: PINNED.load(Ordering::SeqCst), anchor });
}

/// Top-left corner for a panel of `size` next to the dot: above or below when it fits there, otherwise
/// on the side with more room; always inside the work area.
fn beside((x, y): (i32, i32), (l, t, r, b): (i32, i32, i32, i32), (w, h): (i32, i32), s: f64) -> ((i32, i32), &'static str) {
    let off = ((DOT + GAP) * s) as i32;
    let margin = (8.0 * s) as i32;
    let (above, below) = (y - off - t, b - (y + off));
    let (left, right) = (x - off - l, r - (x + off));
    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi.max(lo));
    let (px, py, origin) = if above.max(below) >= h + margin {
        let py = if above >= below { y - off - h } else { y + off };
        (x - w / 2, py, if above >= below { "bottom" } else { "top" })
    } else if left >= right {
        (x - off - w, y - h / 2, "right")
    } else {
        (x + off, y - h / 2, "left")
    };
    ((clamp(px, l + margin, r - w - margin), clamp(py, t + margin, b - h - margin)), origin)
}

/// Let the page animate out, then hide, unless the panel is opened again meanwhile.
fn leave(w: &WebviewWindow) {
    let _ = w.emit("panel-leave", ());
    let id = HIDE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let w = w.clone();
    std::thread::spawn(move || {
        std::thread::sleep(LEAVE);
        if HIDE_GEN.load(Ordering::SeqCst) == id {
            hide(&w);
        }
    });
}

fn hide(w: &WebviewWindow) {
    let _ = w.hide();
    let _ = w.emit("panel-hidden", ());
    HIDE_GEN.fetch_add(1, Ordering::SeqCst);
    schedule_destroy(w.app_handle().clone());
}

fn schedule_destroy(app: AppHandle) {
    let id = HIDE_GEN.load(Ordering::SeqCst);
    std::thread::spawn(move || {
        std::thread::sleep(DESTROY_AFTER);
        if HIDE_GEN.load(Ordering::SeqCst) != id {
            return;
        }
        if let Some(w) = app.get_webview_window(LABEL) {
            if !w.is_visible().unwrap_or(true) {
                EXPANDED_NOW.store(false, Ordering::SeqCst);
                let _ = w.destroy();
            }
        }
    });
}

// ---- called from the page --------------------------------------------------------------------

pub fn set_expanded(app: &AppHandle, expanded: bool) {
    if let Some(w) = app.get_webview_window(LABEL) {
        arrange(&w, expanded, None);
        let _ = w.set_focus();
    }
}

pub fn set_pinned(app: &AppHandle, pinned: bool) {
    PINNED.store(pinned, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.set_always_on_top(pinned);
    }
}

pub fn close(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        leave(&w);
    }
}

pub fn minimize(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.minimize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_stays_on_screen_beside_the_dot() {
        let wa = (0, 0, 1920, 1040);
        // dot at the bottom centre: above it, centred
        let ((x, y), o) = beside((960, 1000), wa, (400, 580), 1.0);
        assert_eq!(o, "bottom");
        assert_eq!((x, y), (760, 1000 - 34 - 580));
        // dot in the top-right corner: below it, pushed left onto the screen
        let ((x, y), o) = beside((1900, 20), wa, (400, 580), 1.0);
        assert_eq!(o, "top");
        assert!(x + 400 <= 1920 - 8 && y == 54);
        // dot at the middle of the right edge on a short screen: to its left
        let ((x, _), o) = beside((1900, 400), (0, 0, 1920, 800), (400, 580), 1.0);
        assert_eq!(o, "right");
        assert_eq!(x, 1900 - 34 - 400);
    }
}
