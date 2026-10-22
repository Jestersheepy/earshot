//! The pill: what it looks like in each state. Pure drawing, no windowing (see examples/gallery.rs).

use tiny_skia::Pixmap;

use crate::paint::*;
use crate::theme::{IdleStyle, PillSize, style};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Accent {
    Dictate,
    Command,
    Note,
    Meeting,
}

impl Accent {
    pub fn color(self) -> Rgba {
        match self {
            Accent::Dictate => style().accent,
            Accent::Command => Rgba(167, 139, 250, 255),
            Accent::Note => Rgba(245, 165, 36, 255),
            Accent::Meeting => Rgba(240, 74, 74, 255),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PillState {
    Idle,
    /// Hold-to-talk dictation or command mode.
    Listening(Accent),
    /// Locked on (double-tap); shows a stop button.
    HandsFree(Accent),
    Thinking,
    /// Voice note or meeting: timer since `secs`.
    Recording { accent: Accent, secs: u32 },
    Done,
    Error,
}

impl PillState {
    /// Target size in logical px. `peek`: idle and hovered long enough to show the hint bar.
    pub fn size(&self, hover: bool, peek: bool) -> (f32, f32) {
        match self {
            PillState::Idle if peek => (peek_width(), 36.0),
            PillState::Idle => {
                let st = style();
                let d = match st.size {
                    PillSize::Small => 16.0,
                    PillSize::Medium => 20.0,
                    PillSize::Large => 26.0,
                } + if st.idle == IdleStyle::Mic { 6.0 } else { 0.0 };
                let d = if hover { d + 6.0 } else { d };
                (d, d)
            }
            PillState::Listening(_) => (124.0, 34.0),
            PillState::HandsFree(_) => (152.0, 34.0),
            PillState::Thinking => (66.0, 30.0),
            PillState::Recording { .. } => (116.0, 34.0),
            PillState::Done | PillState::Error => (32.0, 32.0),
        }
    }
}

pub struct View<'a> {
    pub state: &'a PillState,
    /// Idle hint bar showing (see `PillState::size`).
    pub peek: bool,
    /// Current (animated) size, logical px.
    pub w: f32,
    pub h: f32,
    /// Opacity of the state's content (cross-fade while morphing).
    pub content: f32,
    pub hover: bool,
    /// Hovering the stop button (hands-free / recording).
    pub hover_stop: bool,
    /// Mic levels, oldest first, ~0..1.
    pub levels: &'a [f32],
    /// Seconds since start, for idle-free animation (pulses, dots).
    pub t: f32,
    pub scale: f32,
}

const GLASS_TOP: Rgba = Rgba(46, 43, 39, 250);
const GLASS_BOTTOM: Rgba = Rgba(24, 22, 20, 250);
const RIM: Rgba = Rgba(255, 255, 255, 32);
const WHITE: Rgba = Rgba(247, 244, 239, 255);
const MUTED: Rgba = Rgba(168, 161, 151, 255);
/// The hint bar's text sizes.
const PEEK_TITLE: f32 = 13.0;
const PEEK_HINT: f32 = 12.0;

fn peek_width() -> f32 {
    let st = style();
    let f = fonts();
    34.0 + text_width(&f.semibold, PEEK_TITLE, &st.peek_title) + 12.0 + text_width(&f.regular, PEEK_HINT, &st.peek_hint) + 16.0
}
pub const STOP_R: f32 = 11.0;

/// Pill centre within a canvas of `cw`×`ch` logical px.
pub fn draw(pm: &mut Pixmap, v: &View, cw: f32, ch: f32) {
    let s = v.scale;
    let (w, h) = (v.w * s, v.h * s);
    let (x, y) = ((cw * s - w) / 2.0, (ch * s - h) / 2.0);
    let body = capsule(x, y, w, h, h / 2.0);
    let st = style();
    if matches!(v.state, PillState::Idle) && st.idle == IdleStyle::Breathing && !v.peek {
        // a soft halo that swells and fades every 4 s
        let ph = 0.5 - 0.5 * (v.t * std::f32::consts::TAU / 4.0).cos();
        let grow = (2.0 + 5.0 * ph) * s;
        fill(pm, &capsule(x - grow, y - grow, w + 2.0 * grow, h + 2.0 * grow, h / 2.0 + grow), st.accent.alpha(0.28 * (1.0 - ph)));
    }
    shadow(pm, &body, 10.0 * s, 2.5 * s, Rgba(0, 0, 0, 120));
    fill_vgrad(pm, &body, y, y + h, GLASS_TOP, GLASS_BOTTOM);
    stroke(pm, &capsule(x + 0.5 * s, y + 0.5 * s, w - s, h - s, (h - s) / 2.0), RIM, s);
    if v.content <= 0.01 {
        return;
    }
    let a = v.content;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    match v.state {
        PillState::Idle if v.peek => {
            let f = fonts();
            fill(pm, &circle(x + 18.0 * s, cy, 4.2 * s), st.accent.alpha(a));
            let tx = x + 34.0 * s;
            text(pm, &f.semibold, PEEK_TITLE * s, tx, cy + 4.6 * s, &st.peek_title, WHITE.alpha(a));
            let hx = tx + (text_width(&f.semibold, PEEK_TITLE, &st.peek_title) + 12.0) * s;
            text(pm, &f.regular, PEEK_HINT * s, hx, cy + 4.3 * s, &st.peek_hint, MUTED.alpha(a));
        }
        PillState::Idle => match st.idle {
            IdleStyle::Dot => {
                let r = (h / s * 0.15).max(2.6) * s;
                fill(pm, &circle(cx, cy, r), (if v.hover { st.accent } else { MUTED }).alpha(a));
            }
            IdleStyle::Mic => {
                let k = h / s / 26.0;
                crate::menu::draw_icon(pm, crate::menu::Icon::Mic, cx, cy + 0.5 * s, s * k.clamp(0.6, 0.95), (if v.hover { WHITE } else { MUTED }).alpha(a));
            }
            IdleStyle::Breathing => {
                let r = (h / s * 0.17).max(2.8) * s;
                fill(pm, &circle(cx, cy, r), st.accent.alpha(a));
            }
        },
        PillState::Listening(acc) => {
            dot(pm, x + 17.0 * s, cy, *acc, v.t, s, a, false);
            bars(pm, x + 31.0 * s, x + w - 15.0 * s, cy, h * 0.62, v.levels, s, a);
        }
        PillState::HandsFree(acc) => {
            dot(pm, x + 17.0 * s, cy, *acc, v.t, s, a, false);
            bars(pm, x + 31.0 * s, x + w - 40.0 * s, cy, h * 0.62, v.levels, s, a);
            stop_button(pm, x + w - 19.0 * s, cy, s, a, v.hover_stop);
        }
        PillState::Thinking => {
            for i in 0..3 {
                let ph = v.t * 5.0 - i as f32 * 0.7;
                let lift = ph.sin().max(0.0);
                let px = cx + (i as f32 - 1.0) * 10.0 * s;
                fill(pm, &circle(px, cy - lift * 3.0 * s, 3.0 * s), WHITE.alpha(a * (0.45 + 0.55 * lift)));
            }
        }
        PillState::Recording { accent, secs } => {
            let lx = x + 17.0 * s;
            if v.hover {
                stop_button(pm, lx, cy, s, a, v.hover_stop);
            } else {
                dot(pm, lx, cy, *accent, v.t, s, a, true);
            }
            let label = format!("{}:{:02}", secs / 60, secs % 60);
            let f = &fonts().semibold;
            text(pm, f, 13.0 * s, x + 32.0 * s, cy + 4.6 * s, &label, WHITE.alpha(a));
            let tail = &v.levels[v.levels.len().saturating_sub(7)..];
            bars(pm, x + w - 45.0 * s, x + w - 14.0 * s, cy, h * 0.5, tail, s, a);
        }
        PillState::Done => {
            let k = s * 1.0;
            line(pm, &[(cx - 5.5 * k, cy + 0.5 * k), (cx - 1.5 * k, cy + 4.5 * k), (cx + 6.0 * k, cy - 4.0 * k)],
                 Rgba(52, 199, 89, 255).alpha(a), 2.4 * s);
        }
        PillState::Error => {
            let c = Rgba(245, 165, 36, 255).alpha(a);
            line(pm, &[(cx, cy - 6.0 * s), (cx, cy + 1.5 * s)], c, 2.4 * s);
            fill(pm, &circle(cx, cy + 5.5 * s, 1.4 * s), c);
        }
    }
}

/// Accent dot with a soft glow; `pulse` breathes it (recording).
fn dot(pm: &mut Pixmap, cx: f32, cy: f32, acc: Accent, t: f32, s: f32, a: f32, pulse: bool) {
    let c = acc.color();
    let breathe = if pulse { 0.5 + 0.5 * (t * 3.2).sin() } else { 1.0 };
    fill(pm, &circle(cx, cy, (7.0 + 2.0 * breathe) * s), c.alpha(a * 0.22 * breathe));
    fill(pm, &circle(cx, cy, 4.2 * s), c.alpha(a));
}

fn stop_button(pm: &mut Pixmap, cx: f32, cy: f32, s: f32, a: f32, hover: bool) {
    let bg = if hover { Rgba(255, 255, 255, 46) } else { Rgba(255, 255, 255, 26) };
    fill(pm, &circle(cx, cy, STOP_R * s), bg.alpha(a));
    let q = 4.2 * s;
    fill(pm, &capsule(cx - q, cy - q, 2.0 * q, 2.0 * q, 1.8 * s), Rgba(240, 74, 74, 255).alpha(a));
}

/// Live waveform: rounded bars from the most recent levels, filling [x0, x1].
fn bars(pm: &mut Pixmap, x0: f32, x1: f32, cy: f32, max_h: f32, levels: &[f32], s: f32, a: f32) {
    let (bw, gap) = (3.0 * s, 2.6 * s);
    let n = (((x1 - x0) + gap) / (bw + gap)).floor().max(1.0) as usize;
    let span = n as f32 * (bw + gap) - gap;
    let start = x0 + ((x1 - x0) - span) / 2.0;
    for i in 0..n {
        let lv = levels.len().checked_sub(n - i).map(|j| levels[j]).unwrap_or(0.0);
        let shaped = (lv * 9.0).sqrt().min(1.0); // perceptual: quiet speech still moves
        let hgt = (bw + shaped * (max_h - bw)).max(bw);
        let bx = start + i as f32 * (bw + gap);
        fill(pm, &capsule(bx, cy - hgt / 2.0, bw, hgt, bw / 2.0), WHITE.alpha(a * (0.55 + 0.45 * shaped)));
    }
}

/// Is (px, py) (logical, canvas coords) on the stop button?
pub fn hits_stop(state: &PillState, w: f32, cw: f32, ch: f32, px: f32, py: f32, hover: bool) -> bool {
    let x = (cw - w) / 2.0;
    let cx = match state {
        PillState::HandsFree(_) => x + w - 19.0,
        PillState::Recording { .. } if hover => x + 17.0,
        _ => return false,
    };
    ((px - cx).powi(2) + (py - ch / 2.0).powi(2)).sqrt() <= STOP_R + 2.0
}
