//! Live overlay demo: cycles pill states, shows toasts, opens the menu on right-click, prints events.
//! `cargo run -p earshot-overlay --example demo -- [seconds-per-state]`
use std::time::{Duration, Instant};

use earshot_overlay::menu::{Icon, MenuItem, ToastSpec};
use earshot_overlay::paint::Rgba;
use earshot_overlay::pill::{Accent, PillState};
use earshot_overlay::{Overlay, UiEvent};

fn main() {
    let per: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2.5);
    let (tx, rx) = std::sync::mpsc::channel();
    let ov = Overlay::start(tx, None, true);
    let states = [
        PillState::Idle,
        PillState::Listening(Accent::Dictate),
        PillState::Thinking,
        PillState::Done,
        PillState::Listening(Accent::Command),
        PillState::HandsFree(Accent::Dictate),
        PillState::Recording { accent: Accent::Note, secs: 0 },
        PillState::Recording { accent: Accent::Meeting, secs: 0 },
        PillState::Idle,
    ];
    ov.toast(ToastSpec { id: 1, title: "Meeting detected".into(), body: "Zoom is using your microphone. Record notes?".into(),
        icon: Icon::People, accent: Rgba(240, 74, 74, 255),
        buttons: vec![("record".into(), "Record notes".into()), ("dismiss".into(), "Not now".into())], timeout_s: 20.0 });
    let t0 = Instant::now();
    let mut idx = usize::MAX;
    while t0.elapsed().as_secs_f32() < per * states.len() as f32 {
        let i = (t0.elapsed().as_secs_f32() / per) as usize;
        if i != idx && i < states.len() {
            idx = i;
            println!("state {:?}", states[i]);
            ov.set_state(states[i].clone());
        }
        let t = t0.elapsed().as_secs_f32();
        ov.set_level(((t * 7.0).sin().abs() * 0.06 + 0.01) * ((t * 1.7).sin() * 0.5 + 0.7));
        while let Ok(e) = rx.try_recv() {
            println!("event {e:?}");
            if let UiEvent::ContextMenu { x, y } = e {
                ov.menu(vec![MenuItem { id: "note".into(), label: "New voice note".into(), hint: "Ctrl+Alt+N".into(),
                    icon: Icon::Mic, separator_before: false, danger: false }], x, y);
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    ov.quit();
}
