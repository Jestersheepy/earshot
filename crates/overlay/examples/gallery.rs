//! Renders every pill state, idle style and size to one PNG for review:
//! `cargo run -p earshot-overlay --example gallery -- out.png`
use earshot_overlay::pill::{self, Accent, PillState, View};
use earshot_overlay::theme::{self, IdleStyle, PillSize, Style};
use tiny_skia::Pixmap;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "gallery.png".into());
    let scale = 2.0;
    let (cw, ch) = (340.0, 64.0);
    let levels: Vec<f32> = (0..40).map(|i| (((i as f32) * 0.7).sin().abs() * 0.08 + 0.004) * ((i % 5) as f32 * 0.3 + 0.4)).collect();
    let base = Style { peek_title: "Click · Voice note".into(), peek_hint: "Hold Ctrl+Win to dictate".into(), ..Style::default() };
    // (style, state, hover, peek)
    let mut cases: Vec<(Style, PillState, bool, bool)> = Vec::new();
    for idle in [IdleStyle::Dot, IdleStyle::Mic, IdleStyle::Breathing] {
        for size in [PillSize::Small, PillSize::Medium, PillSize::Large] {
            cases.push((Style { idle, size, ..base.clone() }, PillState::Idle, false, false));
        }
    }
    for (st, hover, peek) in [
        (PillState::Idle, true, false),
        (PillState::Idle, true, true),
        (PillState::Listening(Accent::Dictate), false, false),
        (PillState::Listening(Accent::Command), false, false),
        (PillState::HandsFree(Accent::Dictate), false, false),
        (PillState::Thinking, false, false),
        (PillState::Recording { accent: Accent::Note, secs: 42 }, false, false),
        (PillState::Recording { accent: Accent::Meeting, secs: 754 }, true, false),
        (PillState::Done, false, false),
    ] {
        cases.push((base.clone(), st, hover, peek));
    }
    let cols = 3;
    let rows = cases.len().div_ceil(cols);
    let (tw, th) = ((cw * scale) as u32, (ch * scale) as u32);
    let mut sheet = Pixmap::new(tw * cols as u32, th * rows as u32 * 2).unwrap();
    for (bg_i, bg) in [[240u8, 237, 231], [28, 27, 25]].iter().enumerate() {
        for (i, (style, st, hover, peek)) in cases.iter().enumerate() {
            theme::set(style.clone());
            let mut pm = Pixmap::new(tw, th).unwrap();
            pm.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], 255));
            let (w, h) = st.size(*hover, *peek);
            let view = View { state: st, peek: *peek, w, h, content: 1.0, hover: *hover, hover_stop: false, levels: &levels, t: 0.9, scale };
            pill::draw(&mut pm, &view, cw, ch);
            let (c, r) = ((i % cols) as i32, (i / cols + bg_i * rows) as i32);
            sheet.draw_pixmap(c * tw as i32, r * th as i32, pm.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
    }
    sheet.save_png(&out).unwrap();
    println!("{out}");
}
