//! Clipboard text, paste-into-focused-field, copy-selection, and foreground app name.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

use crate::hotkeys::send_ctrl;

const CF_UNICODETEXT: u32 = 13;

fn open() -> bool {
    for _ in 0..25 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10)); // another app may hold it briefly
    }
    false
}

pub fn get_text() -> Option<String> {
    if !open() {
        return None;
    }
    let text = unsafe {
        GetClipboardData(CF_UNICODETEXT).ok().and_then(|h| {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(g);
            Some(s)
        })
    };
    let _ = unsafe { CloseClipboard() };
    text
}

pub fn set_text(text: &str) -> bool {
    if !open() {
        return false;
    }
    let ok = unsafe {
        let _ = EmptyClipboard();
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        match GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
            Ok(g) => {
                let p = GlobalLock(g) as *mut u16;
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(g);
                SetClipboardData(CF_UNICODETEXT, Some(HANDLE(g.0))).is_ok()
            }
            Err(_) => false,
        }
    };
    let _ = unsafe { CloseClipboard() };
    ok
}

fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Put text into the focused field via the clipboard, then give the user's clipboard back.
pub fn paste(text: &str) -> bool {
    let previous = get_text();
    if !set_text(text) {
        return false;
    }
    send_ctrl(b'V');
    if let Some(prev) = previous {
        std::thread::sleep(Duration::from_millis(350)); // let the target app read it first
        set_text(&prev);
    }
    true
}

/// The current selection in the focused app (via Ctrl+C), with the clipboard restored afterwards.
/// None when nothing is selected.
pub fn copy_selection() -> Option<String> {
    let previous = get_text();
    let before = sequence();
    send_ctrl(b'C');
    let deadline = Instant::now() + Duration::from_millis(400);
    while sequence() == before && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let selection = (sequence() != before).then(get_text).flatten();
    if let Some(prev) = previous {
        set_text(&prev);
    }
    selection.filter(|s| !s.trim().is_empty())
}

/// Executable name of the foreground window's process, e.g. "slack.exe".
pub fn foreground_app() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(proc);
        ok.then(|| {
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit('\\').next().unwrap_or(&path).to_lowercase()
        })
    }
}
