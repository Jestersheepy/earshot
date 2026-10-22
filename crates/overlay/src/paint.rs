//! Drawing primitives on a premultiplied RGBA pixmap: soft shadows, glass fills, text, small icons.

use std::sync::OnceLock;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point, Rect, Shader,
    SpreadMode, Stroke, Transform,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub fn alpha(self, a: f32) -> Self {
        Rgba(self.0, self.1, self.2, (self.3 as f32 * a.clamp(0.0, 1.0)) as u8)
    }
    fn color(self) -> Color {
        Color::from_rgba8(self.0, self.1, self.2, self.3)
    }
}

pub fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c.color());
    p.anti_alias = true;
    p
}

pub fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Proper capsule/rounded rect with circular corners (quads above are slightly squarish).
pub fn capsule(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

pub fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(cx, cy, r)
}

pub fn fill(pm: &mut Pixmap, path: &Option<Path>, c: Rgba) {
    if let Some(p) = path {
        pm.fill_path(p, &paint(c), FillRule::Winding, Transform::identity(), None);
    }
}

pub fn fill_vgrad(pm: &mut Pixmap, path: &Option<Path>, y0: f32, y1: f32, top: Rgba, bottom: Rgba) {
    let Some(p) = path else { return };
    let shader = LinearGradient::new(
        Point::from_xy(0.0, y0),
        Point::from_xy(0.0, y1),
        vec![GradientStop::new(0.0, top.color()), GradientStop::new(1.0, bottom.color())],
        SpreadMode::Pad,
        Transform::identity(),
    );
    let mut pt = Paint::default();
    pt.anti_alias = true;
    pt.shader = shader.unwrap_or(Shader::SolidColor(top.color()));
    pm.fill_path(p, &pt, FillRule::Winding, Transform::identity(), None);
}

pub fn fill_hgrad(pm: &mut Pixmap, path: &Option<Path>, x0: f32, x1: f32, stops: &[(f32, Rgba)]) {
    let Some(p) = path else { return };
    let stops = stops.iter().map(|(t, c)| GradientStop::new(*t, c.color())).collect();
    let mut pt = Paint::default();
    pt.anti_alias = true;
    if let Some(s) =
        LinearGradient::new(Point::from_xy(x0, 0.0), Point::from_xy(x1, 0.0), stops, SpreadMode::Pad, Transform::identity())
    {
        pt.shader = s;
        pm.fill_path(p, &pt, FillRule::Winding, Transform::identity(), None);
    }
}

pub fn stroke(pm: &mut Pixmap, path: &Option<Path>, c: Rgba, width: f32) {
    if let Some(p) = path {
        let s = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(p, &paint(c), &s, Transform::identity(), None);
    }
}

pub fn line(pm: &mut Pixmap, pts: &[(f32, f32)], c: Rgba, width: f32) {
    let mut pb = PathBuilder::new();
    for (i, &(x, y)) in pts.iter().enumerate() {
        if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
    }
    stroke(pm, &pb.finish(), c, width);
}

/// Soft drop shadow: the shape's alpha, box-blurred three times (≈ gaussian), tinted and offset.
pub fn shadow(pm: &mut Pixmap, path: &Option<Path>, blur: f32, dy: f32, c: Rgba) {
    let Some(p) = path else { return };
    let (w, h) = (pm.width(), pm.height());
    let Some(mut sh) = Pixmap::new(w, h) else { return };
    sh.fill_path(p, &paint(Rgba(0, 0, 0, 255)), FillRule::Winding, Transform::from_translate(0.0, dy), None);
    let mut a: Vec<f32> = sh.data().chunks_exact(4).map(|px| px[3] as f32 / 255.0).collect();
    let r = (blur / 2.0).round().max(1.0) as usize;
    for _ in 0..3 {
        box_blur(&mut a, w as usize, h as usize, r);
    }
    let dst = pm.data_mut();
    for (i, alpha) in a.iter().enumerate() {
        let sa = alpha * c.3 as f32 / 255.0;
        if sa <= 0.0 {
            continue;
        }
        let px = &mut dst[i * 4..i * 4 + 4];
        let src = [c.0 as f32 * sa, c.1 as f32 * sa, c.2 as f32 * sa, 255.0 * sa];
        for k in 0..4 {
            px[k] = (src[k] + px[k] as f32 * (1.0 - sa)).round().min(255.0) as u8;
        }
    }
}

fn box_blur(a: &mut [f32], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0f32; a.len()];
    let norm = 1.0 / (2 * r + 1) as f32;
    for y in 0..h {
        let row = &a[y * w..(y + 1) * w];
        let mut acc: f32 = (0..=r).map(|i| row[i.min(w - 1)]).sum::<f32>() + r as f32 * row[0];
        for x in 0..w {
            tmp[y * w + x] = acc * norm;
            acc += row[(x + r + 1).min(w - 1)] - row[x.saturating_sub(r)];
        }
    }
    for x in 0..w {
        let col = |y: usize| tmp[y.min(h - 1) * w + x];
        let mut acc: f32 = (0..=r).map(col).sum::<f32>() + r as f32 * col(0);
        for y in 0..h {
            a[y * w + x] = acc * norm;
            acc += col(y + r + 1) - col(y.saturating_sub(r));
        }
    }
}

// ---- text ----------------------------------------------------------------------------------

pub struct Fonts {
    pub regular: FontVec,
    pub semibold: FontVec,
}

pub fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let load = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| std::fs::read(format!("C:\\Windows\\Fonts\\{n}")).ok())
                .and_then(|b| FontVec::try_from_vec(b).ok())
                .expect("no Segoe UI font found")
        };
        Fonts { regular: load(&["segoeui.ttf", "arial.ttf"]), semibold: load(&["seguisb.ttf", "segoeuib.ttf", "arial.ttf"]) }
    })
}

pub fn text_width(font: &FontVec, size: f32, s: &str) -> f32 {
    let sf = font.as_scaled(PxScale::from(size));
    let mut w = 0.0;
    let mut prev = None;
    for ch in s.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Draws text with its baseline at y (tabular-ish digits come for free with Segoe UI).
pub fn text(pm: &mut Pixmap, font: &FontVec, size: f32, x: f32, y: f32, s: &str, c: Rgba) {
    let sf = font.as_scaled(PxScale::from(size));
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let mut pen = x;
    let mut prev = None;
    let data = pm.data_mut();
    for ch in s.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            pen += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen, y));
        if let Some(og) = font.outline_glyph(glyph) {
            let b = og.px_bounds();
            og.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let sa = cov * c.3 as f32 / 255.0;
                let i = (py * w + px) as usize * 4;
                let src = [c.0 as f32 * sa, c.1 as f32 * sa, c.2 as f32 * sa, 255.0 * sa];
                for k in 0..4 {
                    data[i + k] = (src[k] + data[i + k] as f32 * (1.0 - sa)).round().min(255.0) as u8;
                }
            });
        }
        pen += sf.h_advance(id);
        prev = Some(id);
    }
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Option<Path> {
    Rect::from_xywh(x, y, w, h).map(PathBuilder::from_rect)
}
