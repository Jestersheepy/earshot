//! Which apps are using the microphone, from Windows' privacy bookkeeping:
//! HKCU\...\CapabilityAccessManager\ConsentStore\microphone has a key per app (packaged apps by family
//! name, desktop apps under NonPackaged by exe path with '#' for '\'). LastUsedTimeStop is 0 while in use.
//! No permissions, no audio hooks, nothing injected: just a registry read every few seconds.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

#[derive(Clone, Debug, PartialEq)]
pub enum MicEvent {
    /// `app` has held the mic for a while. `name` is friendly ("Zoom"); `known` = a meeting app or browser.
    Started { app: String, name: String, known: bool },
    Stopped { app: String },
}

/// Lower-case exe name ("zoom.exe") or package family name of every app holding the mic right now.
pub fn in_use() -> Vec<String> {
    let Ok(root) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(STORE) else { return Vec::new() };
    let mut apps = Vec::new();
    for sub in root.enum_keys().flatten() {
        let Ok(k) = root.open_subkey(&sub) else { continue };
        let start: u64 = k.get_value("LastUsedTimeStart").unwrap_or(0);
        let stop: u64 = k.get_value("LastUsedTimeStop").unwrap_or(1);
        if start != 0 && stop == 0 {
            apps.push(sub.to_lowercase());
        }
    }
    apps.retain(|a| a != "nonpackaged");
    if let Ok(np) = root.open_subkey("NonPackaged") {
        let mut desktop = Vec::new();
        let mut scan_np = |k: &RegKey| {
            for sub in k.enum_keys().flatten() {
                let Ok(e) = k.open_subkey(&sub) else { continue };
                let start: u64 = e.get_value("LastUsedTimeStart").unwrap_or(0);
                let stop: u64 = e.get_value("LastUsedTimeStop").unwrap_or(1);
                if start != 0 && stop == 0 {
                    desktop.push(sub.rsplit('#').next().unwrap_or(&sub).to_lowercase());
                }
            }
        };
        scan_np(&np);
        // An app that crashed never writes its stop time: only trust entries whose process is running.
        if !desktop.is_empty() {
            let running = running_exes();
            desktop.retain(|a| running.contains(a));
        }
        apps.extend(desktop);
    }
    apps
}

/// Lower-case exe names of all running processes.
fn running_exes() -> std::collections::HashSet<String> {
    processes().into_values().collect()
}

/// Process id to lower-case exe name.
fn processes() -> HashMap<u32, String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut out = HashMap::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.insert(e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase());
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Friendly names for apps people hold meetings in. Browsers count: Meet, Zoom web, Teams web.
pub fn meeting_app(app: &str) -> Option<&'static str> {
    const KNOWN: &[(&str, &str)] = &[
        ("zoom", "Zoom"),
        ("ms-teams", "Teams"),
        ("msteams", "Teams"),
        ("teams.exe", "Teams"),
        ("slack", "Slack"),
        ("discord", "Discord"),
        ("webex", "Webex"),
        ("ciscocollabhost", "Webex"),
        ("atmgr", "Webex"),
        ("skype", "Skype"),
        ("whatsapp", "WhatsApp"),
        ("telegram", "Telegram"),
        ("signal", "Signal"),
        ("gotomeeting", "GoTo"),
        ("g2mcomm", "GoTo"),
        ("chrome.exe", "Chrome"),
        ("msedge.exe", "Edge"),
        ("firefox.exe", "Firefox"),
        ("brave.exe", "Brave"),
        ("arc.exe", "Arc"),
        ("opera.exe", "Opera"),
        ("vivaldi.exe", "Vivaldi"),
    ];
    KNOWN.iter().find(|(k, _)| app.contains(k)).map(|(_, n)| *n)
}

const BROWSERS: &[&str] = &["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "arc.exe", "opera.exe", "vivaldi.exe"];

/// The call site open in a browser's windows ("Google Meet"), from window titles. Browsers title a window
/// after its active tab, so a call in a background tab isn't seen; the caller falls back to the browser.
pub fn browser_meeting(exe: &str) -> Option<&'static str> {
    if !BROWSERS.contains(&exe) {
        return None;
    }
    let procs = processes();
    window_titles().into_iter().filter(|(pid, _)| procs.get(pid).is_some_and(|e| e == exe)).find_map(|(_, t)| meeting_site(&t))
}

/// Meeting sites by tab title.
fn meeting_site(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    let t = t.trim_end_matches(" - google chrome").trim_end_matches(" - microsoft\u{200b} edge").trim_end_matches(" - microsoft edge");
    if t.starts_with("meet - ") || t.starts_with("meet \u{2013} ") || t.contains("google meet") || t.contains("meet.google.com") {
        Some("Google Meet")
    } else if t.contains("| microsoft teams") || t.contains("microsoft teams meeting") {
        Some("Teams")
    } else if t.contains("zoom meeting") || t.contains("zoom workplace") || t.ends_with("- zoom") {
        Some("Zoom")
    } else if t.contains("webex") {
        Some("Webex")
    } else if t.contains("whereby") {
        Some("Whereby")
    } else if t.contains("jitsi meet") {
        Some("Jitsi")
    } else if t.contains("huddle") && t.contains("slack") {
        Some("Slack")
    } else {
        None
    }
}

/// (process id, title) of every visible top-level window with a title.
fn window_titles() -> Vec<(u32, String)> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};
    use windows::core::BOOL;
    unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> BOOL {
        unsafe {
            let out = &mut *(out.0 as *mut Vec<(u32, String)>);
            if IsWindowVisible(hwnd).as_bool() {
                let mut buf = [0u16; 512];
                let n = GetWindowTextW(hwnd, &mut buf);
                if n > 0 {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(hwnd, Some(&mut pid));
                    out.push((pid, String::from_utf16_lossy(&buf[..n as usize])));
                }
            }
        }
        BOOL(1)
    }
    let mut out: Vec<(u32, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

fn display_name(app: &str) -> String {
    meeting_app(app).map(String::from).unwrap_or_else(|| {
        let stem = app.trim_end_matches(".exe");
        let stem = stem.split('_').next().unwrap_or(stem); // package family -> name
        stem.rsplit('.').next().unwrap_or(stem).to_string()
    })
}

/// Polls on its own thread forever. Reports an app once it has held the mic for `min_on`, and again
/// when it lets go. This process is ignored (our own dictation and recordings use the mic too).
pub fn watch(tx: Sender<MicEvent>, poll: Duration, min_on: Duration) {
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    std::thread::spawn(move || {
        let mut seen: HashMap<String, (Instant, bool)> = HashMap::new(); // first seen, reported
        loop {
            let now: Vec<String> = in_use().into_iter().filter(|a| *a != me).collect();
            for app in &now {
                let (since, reported) = seen.entry(app.clone()).or_insert((Instant::now(), false));
                if !*reported && since.elapsed() >= min_on {
                    *reported = true;
                    let name = browser_meeting(app).map(String::from).unwrap_or_else(|| display_name(app));
                    let ev = MicEvent::Started { app: app.clone(), name, known: meeting_app(app).is_some() };
                    if tx.send(ev).is_err() {
                        return;
                    }
                }
            }
            let gone: Vec<String> = seen.keys().filter(|a| !now.contains(a)).cloned().collect();
            for app in gone {
                if seen.remove(&app).is_some_and(|(_, reported)| reported) && tx.send(MicEvent::Stopped { app }).is_err() {
                    return;
                }
            }
            std::thread::sleep(poll);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(meeting_app("zoom.exe"), Some("Zoom"));
        assert_eq!(meeting_app("msteams_8wekyb3d8bbwe"), Some("Teams"));
        assert_eq!(meeting_app("wavelink.exe"), None);
        assert_eq!(display_name("com.tinyspeck.slackdesktop_8yrtsj140pw4g"), "Slack");
        assert_eq!(display_name("wavelink.exe"), "wavelink");
        assert_eq!(meeting_site("Meet - abc-defg-hij - Google Chrome"), Some("Google Meet"));
        assert_eq!(meeting_site("Meet \u{2013} Weekly sync"), Some("Google Meet"));
        assert_eq!(meeting_site("Meeting with Sam | Microsoft Teams"), Some("Teams"));
        assert_eq!(meeting_site("Inbox (3) - Gmail - Google Chrome"), None);
        let _ = browser_meeting("chrome.exe");
        let _ = in_use(); // must not panic whatever the registry holds
    }
}
