//! Renders the context menu and toasts (light + dark) to one PNG for review.
use earshot_overlay::menu::*;
use earshot_overlay::paint::Rgba;
use tiny_skia::{Color, Pixmap, PixmapPaint, Transform};

fn item(id: &str, label: &str, hint: &str, icon: Icon, sep: bool) -> MenuItem {
    MenuItem { id: id.into(), label: label.into(), hint: hint.into(), icon, separator_before: sep, danger: false }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "menus.png".into());
    let s = 2.0;
    let items = vec![
        item("note", "New voice note", "Ctrl+Alt+N", Icon::Mic, false),
        item("meeting", "Start meeting notes", "Ctrl+Alt+M", Icon::People, false),
        item("paste", "Paste last dictation", "Alt+Shift+V", Icon::Clipboard, false),
        item("handsfree", "Hands-free dictation", "", Icon::Lock, false),
        item("open", "Open Earshot", "", Icon::Notes, true),
        item("settings", "Settings", "", Icon::Gear, false),
        item("hide", "Hide", "Ctrl+Alt+Space", Icon::Hide, true),
        item("quit", "Quit Earshot", "", Icon::Power, false),
    ];
    let toast = ToastSpec { id: 1, title: "Meeting detected".into(), body: "Zoom is using your microphone. Record notes?".into(),
        icon: Icon::People, accent: Rgba(240, 74, 74, 255),
        buttons: vec![("record".into(), "Record notes".into()), ("dismiss".into(), "Not now".into())], timeout_s: 20.0 };
    let saved = ToastSpec { id: 2, title: "Voice note saved".into(), body: "2026-10-22 0211 · 1:42 · 214 words".into(),
        icon: Icon::Check, accent: Rgba(52, 199, 89, 255), buttons: vec![("open".into(), "Open".into())], timeout_s: 6.0 };
    let mw = ((MENU_W + 2.0 * MARGIN) * s) as u32;
    let mh = ((menu_height(&items) + 2.0 * MARGIN) * s) as u32;
    let tw = ((TOAST_W + 2.0 * MARGIN) * s) as u32;
    let mut sheet = Pixmap::new(2 * (mw + tw), mh).unwrap();
    for (k, (dark, bg)) in [(false, [214u8, 220, 230]), (true, [24, 26, 32])].into_iter().enumerate() {
        let theme = Theme { dark };
        let mut pm = Pixmap::new(mw + tw, mh).unwrap();
        pm.fill(Color::from_rgba8(bg[0], bg[1], bg[2], 255));
        let mut m = Pixmap::new(mw, mh).unwrap();
        draw_menu(&mut m, &items, Some(1), theme, s);
        pm.draw_pixmap(0, 0, m.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let mut y = 0;
        for (t, hover) in [(&toast, true), (&saved, false)] {
            let th = ((toast_height(t) + 2.0 * MARGIN) * s) as u32;
            let mut tp = Pixmap::new(tw, th).unwrap();
            draw_toast(&mut tp, t, theme, s, if hover { Some(0) } else { None }, hover, 0.35);
            pm.draw_pixmap(mw as i32, y, tp.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            y += th as i32;
        }
        sheet.draw_pixmap((k as u32 * (mw + tw)) as i32, 0, pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    sheet.save_png(&out).unwrap();
    println!("{out}");
}
