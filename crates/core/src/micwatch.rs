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
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut out = std::collections::HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.insert(String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase());
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
                    let ev = MicEvent::Started { app: app.clone(), name: display_name(app), known: meeting_app(app).is_some() };
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
        let _ = in_use(); // must not panic whatever the registry holds
    }
}
