//! Renders every pill state to one PNG for review: `cargo run -p earshot-overlay --example gallery -- out.png`
use earshot_overlay::pill::{self, Accent, PillState, View};
use tiny_skia::Pixmap;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "gallery.png".into());
    let scale = 2.0;
    let (cw, ch) = (190.0, 60.0);
    let levels: Vec<f32> = (0..40).map(|i| (((i as f32) * 0.7).sin().abs() * 0.08 + 0.004) * ((i % 5) as f32 * 0.3 + 0.4)).collect();
    let states = [
        (PillState::Idle, false), (PillState::Idle, true),
        (PillState::Listening(Accent::Dictate), false), (PillState::Listening(Accent::Command), false),
        (PillState::HandsFree(Accent::Dictate), false), (PillState::Thinking, false),
        (PillState::Recording { accent: Accent::Note, secs: 42 }, false),
        (PillState::Recording { accent: Accent::Meeting, secs: 754 }, false),
        (PillState::Recording { accent: Accent::Meeting, secs: 754 }, true),
        (PillState::Done, false), (PillState::Error, false),
    ];
    let cols = 3;
    let rows = states.len().div_ceil(cols);
    let (tw, th) = ((cw * scale) as u32, (ch * scale) as u32);
    let mut sheet = Pixmap::new(tw * cols as u32, th * rows as u32 * 2).unwrap();
    for (bg_i, bg) in [[236u8, 238, 242], [32, 34, 40]].iter().enumerate() {
        for (i, (st, hover)) in states.iter().enumerate() {
            let mut pm = Pixmap::new(tw, th).unwrap();
            pm.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], 255));
            let (w, h) = st.size(*hover);
            pill::draw(&mut pm, &View { state: st, w, h, content: 1.0, hover: *hover, hover_stop: false, levels: &levels, t: 0.3, scale }, cw, ch);
            let (c, r) = ((i % cols) as i32, (i / cols + bg_i * rows) as i32);
            sheet.draw_pixmap(c * tw as i32, r * th as i32, pm.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
        }
    }
    sheet.save_png(&out).unwrap();
    println!("{out}");
}
