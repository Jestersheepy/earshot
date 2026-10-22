//! Context menu and toast visuals (pure drawing; windowing lives in ui.rs).

use tiny_skia::Pixmap;

use crate::paint::*;
use crate::theme::{palette, style};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Mic,
    People,
    Notes,
    Clipboard,
    Lock,
    Gear,
    Hide,
    Power,
    Record,
    Check,
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub hint: String,
    pub icon: Icon,
    pub separator_before: bool,
    /// Draw the icon in the accent colour (e.g. "Stop meeting" while recording).
    pub danger: bool,
    /// A segmented row instead of an action: (choice id, label). Picking one emits "{id}:{choice id}" and
    /// keeps the menu open.
    pub choices: Vec<(String, String)>,
    pub selected: usize,
}

impl MenuItem {
    pub fn action(id: &str, label: &str, hint: &str, icon: Icon) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: hint.into(),
            icon,
            separator_before: false,
            danger: false,
            choices: Vec::new(),
            selected: 0,
        }
    }

    pub fn choice(id: &str, label: &str, choices: &[(&str, &str)], selected: &str) -> Self {
        Self {
            choices: choices.iter().map(|(i, l)| (i.to_string(), l.to_string())).collect(),
            selected: choices.iter().position(|(i, _)| *i == selected).unwrap_or(0),
            ..Self::action(id, label, "", Icon::Mic)
        }
    }

    fn height(&self) -> f32 {
        if self.choices.is_empty() { ITEM_H } else { CHOICE_H }
    }
}

pub const MENU_W: f32 = 268.0;
const ITEM_H: f32 = 32.0;
const CHOICE_H: f32 = 46.0;
const SEG_H: f32 = 28.0;
const PAD: f32 = 6.0;
const SEP_H: f32 = 9.0;
pub const MARGIN: f32 = 16.0; // room for the shadow

pub fn menu_height(items: &[MenuItem]) -> f32 {
    2.0 * PAD + items.iter().map(|i| i.height() + if i.separator_before { SEP_H } else { 0.0 }).sum::<f32>()
}

/// Top of each item (logical, window coords).
fn tops(items: &[MenuItem]) -> Vec<f32> {
    let mut top = MARGIN + PAD;
    items
        .iter()
        .map(|it| {
            if it.separator_before {
                top += SEP_H;
            }
            let t = top;
            top += it.height();
            t
        })
        .collect()
}

/// Segment rectangles of a choice row whose top is `top` (logical).
fn segments(it: &MenuItem, top: f32) -> Vec<(f32, f32, f32, f32)> {
    let f = &fonts().semibold;
    let widths: Vec<f32> = it.choices.iter().map(|(_, l)| text_width(f, 12.5, l) + 22.0).collect();
    let mut x = MARGIN + MENU_W - PAD - 6.0 - widths.iter().sum::<f32>() - 4.0;
    let y = top + (CHOICE_H - SEG_H) / 2.0 + 2.0;
    widths
        .iter()
        .map(|w| {
            let r = (x + 2.0, y + 2.0, *w, SEG_H - 4.0);
            x += w;
            r
        })
        .collect()
}

/// The item at logical (x, y), and for a choice row the segment under the cursor.
pub fn hit(items: &[MenuItem], x: f32, y: f32) -> Option<(usize, Option<usize>)> {
    if x < MARGIN || x > MARGIN + MENU_W {
        return None;
    }
    let tops = tops(items);
    let i = items.iter().zip(&tops).position(|(it, &t)| y >= t && y < t + it.height())?;
    let seg = segments(&items[i], tops[i]).iter().position(|&(sx, sy, sw, sh)| x >= sx && x < sx + sw && y >= sy - 4.0 && y < sy + sh + 4.0);
    Some((i, seg))
}

pub fn draw_menu(pm: &mut Pixmap, items: &[MenuItem], hover: Option<(usize, Option<usize>)>, s: f32) {
    let pal = palette();
    let (x, y, w, h) = (MARGIN * s, MARGIN * s, MENU_W * s, menu_height(items) * s);
    let panel = capsule(x, y, w, h, 14.0 * s);
    shadow(pm, &panel, 22.0 * s, 8.0 * s, pal.shadow);
    fill(pm, &panel, pal.bg);
    stroke(pm, &capsule(x + 0.5 * s, y + 0.5 * s, w - s, h - s, 13.5 * s), pal.border, s);
    let f = fonts();
    let accent = style().accent;
    for (i, (it, &t)) in items.iter().zip(&tops(items)).enumerate() {
        let top = t * s;
        if it.separator_before {
            let ly = top - SEP_H * s / 2.0;
            line(pm, &[(x + 14.0 * s, ly), (x + w - 14.0 * s, ly)], pal.rule, s);
        }
        let cy = top + it.height() * s / 2.0;
        if !it.choices.is_empty() {
            text(pm, &f.regular, 12.5 * s, x + 16.0 * s, cy + 6.4 * s, &it.label, pal.muted);
            let segs = segments(it, t);
            let (fx, fy) = (segs[0].0 - 2.0, segs[0].1 - 2.0);
            let fw = segs.iter().map(|g| g.2).sum::<f32>() + 4.0;
            fill(pm, &capsule(fx * s, fy * s, fw * s, SEG_H * s, 8.0 * s), pal.hover);
            for (k, ((_, label), &(sx, sy, sw, sh))) in it.choices.iter().zip(&segs).enumerate() {
                let on = k == it.selected;
                let hov = hover == Some((i, Some(k)));
                if on {
                    let r = capsule(sx * s, sy * s, sw * s, sh * s, 6.5 * s);
                    shadow(pm, &r, 3.0 * s, 0.5 * s, Rgba(0, 0, 0, 30));
                    fill(pm, &r, pal.raised);
                } else if hov {
                    fill(pm, &capsule(sx * s, sy * s, sw * s, sh * s, 6.5 * s), pal.hover);
                }
                let font = if on { &f.semibold } else { &f.regular };
                let tw = text_width(font, 12.5 * s, label);
                let tc = if on { pal.text } else { pal.muted };
                text(pm, font, 12.5 * s, sx * s + (sw * s - tw) / 2.0, (sy + sh / 2.0 + 4.4) * s, label, tc);
            }
            continue;
        }
        if hover.map(|h| h.0) == Some(i) {
            fill(pm, &capsule(x + PAD * s, top, w - 2.0 * PAD * s, ITEM_H * s, 8.0 * s), pal.hover);
        }
        let ic = if it.danger { Rgba(229, 72, 77, 255) } else if hover.map(|h| h.0) == Some(i) { accent } else { pal.muted };
        draw_icon(pm, it.icon, x + 24.0 * s, cy, s, ic);
        text(pm, &f.regular, 13.5 * s, x + 44.0 * s, cy + 4.8 * s, &it.label, pal.text);
        if !it.hint.is_empty() {
            let hw = text_width(&f.regular, 11.5 * s, &it.hint);
            text(pm, &f.regular, 11.5 * s, x + w - 16.0 * s - hw, cy + 4.2 * s, &it.hint, pal.muted.alpha(0.85));
        }
    }
}

/// 16px line icons centred on (cx, cy).
pub fn draw_icon(pm: &mut Pixmap, icon: Icon, cx: f32, cy: f32, s: f32, c: Rgba) {
    let lw = 1.5 * s;
    let p = |dx: f32, dy: f32| (cx + dx * s, cy + dy * s);
    match icon {
        Icon::Mic => {
            stroke(pm, &capsule(cx - 3.0 * s, cy - 7.0 * s, 6.0 * s, 10.0 * s, 3.0 * s), c, lw);
            let mut pts = Vec::new();
            for i in 0..=12 {
                let a = std::f32::consts::PI * i as f32 / 12.0;
                pts.push(p(-5.5 * a.cos(), 0.5 + 5.0 * a.sin()));
            }
            line(pm, &pts, c, lw);
            line(pm, &[p(0.0, 5.5), p(0.0, 7.5)], c, lw);
        }
        Icon::People => {
            stroke(pm, &circle(cx - 2.5 * s, cy - 3.0 * s, 2.6 * s), c, lw);
            line(pm, &[p(-7.0, 6.5), p(-6.3, 3.0), p(-2.5, 1.5), p(1.3, 3.0), p(2.0, 6.5)], c, lw);
            stroke(pm, &circle(cx + 4.0 * s, cy - 3.5 * s, 2.2 * s), c, lw);
            line(pm, &[p(3.5, 1.2), p(6.6, 2.4), p(7.4, 5.5)], c, lw);
        }
        Icon::Notes => {
            stroke(pm, &capsule(cx - 6.0 * s, cy - 7.0 * s, 12.0 * s, 14.0 * s, 2.5 * s), c, lw);
            for dy in [-3.0, 0.0, 3.0] {
                line(pm, &[p(-3.0, dy), p(3.0, dy)], c, lw);
            }
        }
        Icon::Clipboard => {
            stroke(pm, &capsule(cx - 5.5 * s, cy - 5.5 * s, 11.0 * s, 13.0 * s, 2.0 * s), c, lw);
            fill(pm, &capsule(cx - 3.0 * s, cy - 7.5 * s, 6.0 * s, 3.5 * s, 1.2 * s), c);
        }
        Icon::Lock => {
            stroke(pm, &capsule(cx - 5.5 * s, cy - 1.0 * s, 11.0 * s, 8.0 * s, 2.0 * s), c, lw);
            let mut pts = Vec::new();
            for i in 0..=12 {
                let a = std::f32::consts::PI * i as f32 / 12.0;
                pts.push(p(-3.2 * a.cos(), -1.0 - 4.5 * a.sin()));
            }
            line(pm, &pts, c, lw);
        }
        Icon::Gear => {
            stroke(pm, &circle(cx, cy, 2.4 * s), c, lw);
            for i in 0..8 {
                let a = std::f32::consts::TAU * i as f32 / 8.0;
                line(pm, &[p(4.6 * a.cos(), 4.6 * a.sin()), p(6.8 * a.cos(), 6.8 * a.sin())], c, lw * 1.3);
            }
            stroke(pm, &circle(cx, cy, 5.0 * s), c, lw);
        }
        Icon::Hide => {
            let mut top = Vec::new();
            let mut bot = Vec::new();
            for i in 0..=16 {
                let t = i as f32 / 16.0 * 2.0 - 1.0;
                top.push(p(t * 7.0, -(1.0 - t * t) * 4.5));
                bot.push(p(t * 7.0, (1.0 - t * t) * 4.5));
            }
            line(pm, &top, c, lw);
            line(pm, &bot, c, lw);
            stroke(pm, &circle(cx, cy, 2.0 * s), c, lw);
            line(pm, &[p(-6.0, 6.0), p(6.0, -6.0)], c, lw);
        }
        Icon::Power => {
            let mut pts = Vec::new();
            for i in 0..=20 {
                let a = (-60.0 + 300.0 * i as f32 / 20.0).to_radians(); // gap at the top
                pts.push(p(6.0 * a.cos(), 6.0 * a.sin() + 1.0));
            }
            line(pm, &pts, c, lw);
            line(pm, &[p(0.0, -7.0), p(0.0, -1.0)], c, lw);
        }
        Icon::Record => {
            stroke(pm, &circle(cx, cy, 6.5 * s), c, lw);
            fill(pm, &circle(cx, cy, 3.5 * s), c);
        }
        Icon::Check => line(pm, &[p(-5.0, 0.5), p(-1.5, 4.0), p(5.5, -4.0)], c, lw * 1.3),
    }
}

// ---- toast ---------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ToastSpec {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub icon: Icon,
    pub accent: Rgba,
    /// (action id, label); the first is the primary button.
    pub buttons: Vec<(String, String)>,
    pub timeout_s: f32,
}

pub const TOAST_W: f32 = 340.0;
const BTN_H: f32 = 30.0;

pub fn toast_height(t: &ToastSpec) -> f32 {
    if t.buttons.len() > 1 { 108.0 } else { 66.0 }
}

/// Toasts with a button row get a close (×) in the corner; inline ones just time out.
pub fn has_close(t: &ToastSpec) -> bool {
    !inline_button(t)
}

/// A single button sits inline on the right instead of on its own row.
fn inline_button(t: &ToastSpec) -> bool {
    t.buttons.len() == 1
}

pub struct ToastHit {
    pub buttons: Vec<(f32, f32, f32, f32)>,
    pub close: (f32, f32, f32),
}

/// Button rectangles and close-button circle in logical coords (relative to the window).
pub fn toast_layout(t: &ToastSpec) -> ToastHit {
    let (x, y, w, h) = (MARGIN, MARGIN, TOAST_W, toast_height(t));
    let f = &fonts().semibold;
    // primary (index 0) sits rightmost, the rest follow to its left
    let mut right = x + w - 14.0;
    let buttons = t
        .buttons
        .iter()
        .map(|(_, label)| {
            let bw = text_width(f, 13.0, label) + 28.0;
            let by = if inline_button(t) { y + (h - BTN_H) / 2.0 } else { y + h - 14.0 - BTN_H };
            let b = (right - bw, by, bw, BTN_H);
            right -= bw + 8.0;
            b
        })
        .collect();
    ToastHit { buttons, close: (x + w - 14.0, y + 14.0, 9.0) }
}

pub fn draw_toast(pm: &mut Pixmap, t: &ToastSpec, s: f32, hover_btn: Option<usize>, hover: bool, progress: f32) {
    let theme = palette();
    let (x, y, w, h) = (MARGIN * s, MARGIN * s, TOAST_W * s, toast_height(t) * s);
    let panel = capsule(x, y, w, h, 16.0 * s);
    shadow(pm, &panel, 22.0 * s, 8.0 * s, theme.shadow);
    fill(pm, &panel, theme.bg);
    stroke(pm, &capsule(x + 0.5 * s, y + 0.5 * s, w - s, h - s, 15.5 * s), theme.border, s);
    // icon badge
    let (ix, iy) = (x + 32.0 * s, y + 33.0 * s);
    fill(pm, &circle(ix, iy, 17.0 * s), t.accent.alpha(0.16));
    draw_icon(pm, t.icon, ix, iy, s, t.accent);
    let f = fonts();

    let reserve = if inline_button(t) { toast_layout(t).buttons[0].2 + 16.0 } else { 30.0 };
    let body = ellipsize(&f.regular, 13.0, &t.body, TOAST_W - 60.0 - reserve);
    let title = ellipsize(&f.semibold, 14.0, &t.title, TOAST_W - 60.0 - reserve);
    text(pm, &f.semibold, 14.0 * s, x + 60.0 * s, y + 29.0 * s, &title, theme.text);
    text(pm, &f.regular, 13.0 * s, x + 60.0 * s, y + 48.0 * s, &body, theme.muted);
    if hover && has_close(t) {
        let (cx, cy, r) = toast_layout(t).close;
        fill(pm, &circle(cx * s, cy * s, r * s), theme.hover);
        let k = 3.2 * s;
        line(pm, &[(cx * s - k, cy * s - k), (cx * s + k, cy * s + k)], theme.muted, 1.4 * s);
        line(pm, &[(cx * s + k, cy * s - k), (cx * s - k, cy * s + k)], theme.muted, 1.4 * s);
    }
    for (i, ((_, label), b)) in t.buttons.iter().zip(toast_layout(t).buttons).enumerate() {
        let r = capsule(b.0 * s, b.1 * s, b.2 * s, b.3 * s, b.3 * s / 2.0);
        let primary = i == 0;
        let hov = hover_btn == Some(i);
        let tc = if primary {
            fill(pm, &r, if hov { t.accent.alpha(0.88) } else { t.accent });
            Rgba(255, 255, 255, 255)
        } else {
            fill(pm, &r, if hov { theme.hover.alpha(2.0) } else { theme.hover });
            theme.text
        };
        let tw = text_width(&f.semibold, 13.0 * s, label);
        text(pm, &f.semibold, 13.0 * s, b.0 * s + (b.2 * s - tw) / 2.0, (b.1 + b.3 / 2.0 + 4.6) * s, label, tc);
    }
    if t.timeout_s > 0.0 && progress < 1.0 {
        let bw = (w - 28.0 * s) * (1.0 - progress);
        fill(pm, &capsule(x + 14.0 * s, y + h - 4.0 * s, bw, 2.0 * s, 1.0 * s), t.accent.alpha(0.5));
    }
}

fn ellipsize(font: &ab_glyph::FontVec, size: f32, s: &str, max_w: f32) -> String {
    if text_width(font, size, s) <= max_w {
        return s.to_string();
    }
    let mut out: String = s.to_string();
    while !out.is_empty() && text_width(font, size, &format!("{out}…")) > max_w {
        out.pop();
    }
    format!("{}…", out.trim_end())
}
