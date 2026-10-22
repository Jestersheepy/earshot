//! Global hotkeys via a low-level keyboard hook (WH_KEYBOARD_LL).
//!
//! - `dictate` is a hold combo (default Ctrl+Win): HoldStart on press, HoldEnd on release.
//! - `command` extends it (default Ctrl+Win+Alt): pressing the extra key mid-hold emits HoldUpgrade.
//! - toggles fire once on the combo's final key, which is swallowed so it doesn't type into the app.
//! - After Win was part of a combo, a dummy key is injected before Win's release so Start doesn't open.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::mpsc::Sender;
use std::time::Instant;

use anyhow::{Result, bail};
use parking_lot::Mutex;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// A key that may have left/right variants.
#[derive(Clone, Debug, PartialEq)]
struct Key(Vec<u16>);

#[derive(Clone, Debug, PartialEq)]
pub struct Combo(Vec<Key>);

impl Combo {
    pub fn parse(s: &str) -> Result<Self> {
        let keys = s
            .split('+')
            .map(|k| {
                let k = k.trim().to_lowercase();
                let vks: Vec<u16> = match k.as_str() {
                    "ctrl" | "control" => vec![VK_LCONTROL.0, VK_RCONTROL.0],
                    "win" | "windows" | "super" => vec![VK_LWIN.0, VK_RWIN.0],
                    "alt" => vec![VK_LMENU.0, VK_RMENU.0],
                    "shift" => vec![VK_LSHIFT.0, VK_RSHIFT.0],
                    "space" => vec![VK_SPACE.0],
                    "enter" => vec![VK_RETURN.0],
                    "esc" | "escape" => vec![VK_ESCAPE.0],
                    k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => {
                        vec![k.to_ascii_uppercase().as_bytes()[0] as u16]
                    }
                    k if k.starts_with('f') && k[1..].parse::<u16>().is_ok_and(|n| (1..=24).contains(&n)) => {
                        vec![VK_F1.0 + k[1..].parse::<u16>().unwrap() - 1]
                    }
                    _ => bail!("unknown key '{k}' in hotkey '{s}'"),
                };
                Ok(Key(vks))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self(keys))
    }

    fn held(&self, down: &HashSet<u16>) -> bool {
        self.0.iter().all(|k| k.0.iter().any(|vk| down.contains(vk)))
    }

    fn uses_win(&self) -> bool {
        self.0.iter().any(|k| k.0.contains(&VK_LWIN.0))
    }

    fn has(&self, vk: u16) -> bool {
        self.0.iter().any(|k| k.0.contains(&vk))
    }

    /// Physically held right now (GetAsyncKeyState), for "wait until released before pasting".
    pub fn any_down(&self) -> bool {
        self.0.iter().flat_map(|k| &k.0).any(|&vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    HoldStart,
    /// The command key joined an active hold.
    HoldUpgrade,
    HoldEnd { held_ms: u64 },
    Toggle(String),
}

pub struct Bindings {
    pub dictate: Combo,
    pub command: Combo,
    pub toggles: Vec<(String, Combo)>,
}

struct State {
    bindings: Bindings,
    tx: Sender<Event>,
    down: HashSet<u16>,
    holding: Option<Instant>,
    upgraded: bool,
    win_used: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
const VK_DUMMY: u16 = 0xE8; // unassigned; breaks Win down/up so Start doesn't open

pub fn rebind(bindings: Bindings) {
    if let Some(s) = STATE.get() {
        s.lock().bindings = bindings;
    }
}

/// Installs the hook on its own thread (it needs a message loop). Call once.
pub fn start(bindings: Bindings, tx: Sender<Event>) {
    if STATE
        .set(Mutex::new(State { bindings, tx, down: HashSet::new(), holding: None, upgraded: false, win_used: false }))
        .is_err()
    {
        return;
    }
    std::thread::Builder::new()
        .name("hotkeys".into())
        .spawn(|| unsafe {
            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(HINSTANCE::default()), 0);
            if hook.is_err() {
                eprintln!("keyboard hook failed: {hook:?}");
                return;
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
        })
        .expect("hotkey thread");
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let injected = kb.flags.0 & LLKHF_INJECTED.0 != 0;
        if !injected {
            let is_down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if let Some(state) = STATE.get() {
                if handle(&mut state.lock(), kb.vkCode as u16, is_down) {
                    return LRESULT(1); // swallow
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Returns true to swallow the key.
fn handle(s: &mut State, vk: u16, is_down: bool) -> bool {
    if is_down {
        let repeat = !s.down.insert(vk);
        if repeat {
            return s.bindings.toggles.iter().any(|(_, c)| c.held(&s.down) && !is_modifier(vk) && c.has(vk));
        }
        if s.holding.is_none() && s.bindings.dictate.held(&s.down) {
            s.holding = Some(Instant::now());
            s.upgraded = false;
            s.win_used |= s.bindings.dictate.uses_win();
            let _ = s.tx.send(Event::HoldStart);
        } else if s.holding.is_some() && !s.upgraded && s.bindings.command.held(&s.down) {
            s.upgraded = true;
            s.win_used |= s.bindings.command.uses_win();
            let _ = s.tx.send(Event::HoldUpgrade);
        }
        if !is_modifier(vk) {
            for (name, c) in &s.bindings.toggles {
                if c.has(vk) && c.held(&s.down) {
                    s.win_used |= c.uses_win();
                    let _ = s.tx.send(Event::Toggle(name.clone()));
                    return true;
                }
            }
        }
        false
    } else {
        s.down.remove(&vk);
        if let Some(t) = s.holding {
            if !s.bindings.dictate.held(&s.down) {
                s.holding = None;
                let _ = s.tx.send(Event::HoldEnd { held_ms: t.elapsed().as_millis() as u64 });
            }
        }
        if (vk == VK_LWIN.0 || vk == VK_RWIN.0) && s.win_used {
            s.win_used = false;
            tap(VK_DUMMY);
        }
        false
    }
}

fn is_modifier(vk: u16) -> bool {
    [VK_LCONTROL, VK_RCONTROL, VK_LWIN, VK_RWIN, VK_LMENU, VK_RMENU, VK_LSHIFT, VK_RSHIFT, VK_CONTROL, VK_MENU, VK_SHIFT]
        .iter()
        .any(|k| k.0 == vk)
}

fn key_input(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                ..Default::default()
            },
        },
    }
}

fn tap(vk: u16) {
    send(&[key_input(vk, false), key_input(vk, true)]);
}

pub(crate) fn send(inputs: &[INPUT]) {
    unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
}

/// Ctrl+<key> (e.g. b'V', b'C'), releasing Alt/Win/Shift first if they're physically held so the
/// target app sees a clean chord.
pub fn send_ctrl(key: u8) {
    let mut seq = Vec::new();
    for vk in [VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT] {
        if unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000 != 0 {
            seq.push(key_input(vk.0, true));
        }
    }
    seq.extend([
        key_input(VK_CONTROL.0, false),
        key_input(key as u16, false),
        key_input(key as u16, true),
        key_input(VK_CONTROL.0, true),
    ]);
    send(&seq);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combos() {
        let c = Combo::parse("Ctrl+Win+Alt").unwrap();
        assert_eq!(c.0.len(), 3);
        assert!(Combo::parse("ctrl+alt+n").unwrap().has(b'N' as u16));
        assert!(Combo::parse("alt+f13").is_ok());
        assert!(Combo::parse("ctrl+banana").is_err());
    }

    #[test]
    fn hold_upgrade_and_toggle() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut s = State {
            bindings: Bindings {
                dictate: Combo::parse("ctrl+win").unwrap(),
                command: Combo::parse("ctrl+win+alt").unwrap(),
                toggles: vec![("note".into(), Combo::parse("ctrl+alt+n").unwrap())],
            },
            tx,
            down: HashSet::new(),
            holding: None,
            upgraded: false,
            win_used: false,
        };
        handle(&mut s, VK_LCONTROL.0, true);
        handle(&mut s, VK_LWIN.0, true);
        handle(&mut s, VK_LMENU.0, true);
        handle(&mut s, VK_LMENU.0, false);
        handle(&mut s, VK_LWIN.0, false);
        handle(&mut s, VK_LCONTROL.0, false);
        let ev: Vec<Event> = rx.try_iter().collect();
        assert_eq!(ev[0], Event::HoldStart);
        assert_eq!(ev[1], Event::HoldUpgrade);
        assert!(matches!(ev[2], Event::HoldEnd { .. }));
        handle(&mut s, VK_LCONTROL.0, true);
        handle(&mut s, VK_LMENU.0, true);
        assert!(handle(&mut s, b'N' as u16, true), "toggle key is swallowed");
        assert_eq!(rx.try_recv().unwrap(), Event::Toggle("note".into()));
    }
}
