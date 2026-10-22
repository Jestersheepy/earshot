//! The shared look: a warm, light-first palette (mirrored as CSS variables in src/app.css), the accent
//! colour, light/dark choice and the pill's personalisation. Set once by the app, read while drawing.

use parking_lot::RwLock;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

use crate::paint::Rgba;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PillSize {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum IdleStyle {
    /// A small dot inside the glass bead.
    Dot,
    /// A mic glyph, so it's obvious what it is.
    Mic,
    /// The dot in your accent with a slow halo that says "ready".
    Breathing,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub theme: ThemeMode,
    pub size: PillSize,
    pub idle: IdleStyle,
    pub accent: Rgba,
    /// Expand into a hint bar when hovered.
    pub peek: bool,
    /// The hint bar: what a click does, and a second line of help.
    pub peek_title: String,
    pub peek_hint: String,
}

pub const DEFAULT_ACCENT: Rgba = Rgba(20, 163, 139, 255);

impl Default for Style {
    fn default() -> Self {
        Self {
            theme: ThemeMode::System,
            size: PillSize::Medium,
            idle: IdleStyle::Mic,
            accent: DEFAULT_ACCENT,
            peek: true,
            peek_title: "Click to dictate".into(),
            peek_hint: "Right-click for more".into(),
        }
    }
}

static STYLE: RwLock<Option<Style>> = RwLock::new(None);

pub fn style() -> Style {
    STYLE.read().clone().unwrap_or_default()
}

pub fn set(s: Style) {
    *STYLE.write() = Some(s);
}

pub fn is_dark() -> bool {
    match style().theme {
        ThemeMode::System => system_dark(),
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
    }
}

/// Surfaces and text for the menu, toasts and (as CSS) the Earshot window.
pub struct Palette {
    pub bg: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub muted: Rgba,
    pub hover: Rgba,
    pub rule: Rgba,
    /// Raised control on a hover-coloured track (selected segment).
    pub raised: Rgba,
    pub shadow: Rgba,
}

pub fn palette() -> Palette {
    if is_dark() {
        Palette {
            bg: Rgba(32, 30, 27, 250),
            border: Rgba(255, 255, 255, 22),
            text: Rgba(242, 239, 234, 255),
            muted: Rgba(163, 157, 148, 255),
            hover: Rgba(255, 255, 255, 16),
            rule: Rgba(255, 255, 255, 14),
            raised: Rgba(62, 59, 54, 255),
            shadow: Rgba(0, 0, 0, 150),
        }
    } else {
        Palette {
            bg: Rgba(252, 251, 248, 250),
            border: Rgba(40, 30, 10, 20),
            text: Rgba(30, 28, 25, 255),
            muted: Rgba(117, 112, 106, 255),
            hover: Rgba(40, 30, 10, 11),
            rule: Rgba(40, 30, 10, 13),
            raised: Rgba(255, 255, 255, 255),
            shadow: Rgba(60, 40, 10, 60),
        }
    }
}

pub fn system_dark() -> bool {
    dword("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", "AppsUseLightTheme") == Some(0)
}

/// The Windows accent colour (Settings > Personalisation > Colours).
pub fn windows_accent() -> Option<Rgba> {
    // stored as 0xAABBGGRR
    dword("Software\\Microsoft\\Windows\\DWM", "AccentColor").map(|v| Rgba(v as u8, (v >> 8) as u8, (v >> 16) as u8, 255))
}

fn dword(key: &str, value: &str) -> Option<u32> {
    let key: Vec<u16> = key.encode_utf16().chain([0]).collect();
    let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let mut v: u32 = 0;
    let mut n = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(key.as_ptr()),
            windows::core::PCWSTR(value.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut n),
        )
    };
    r.is_ok().then_some(v)
}

/// "#RRGGBB" to a colour.
pub fn parse_hex(s: &str) -> Option<Rgba> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex() {
        assert_eq!(parse_hex("#4F6EF7"), Some(Rgba(79, 110, 247, 255)));
        assert_eq!(parse_hex("4F6EF7"), None);
        assert_eq!(parse_hex("#12345"), None);
    }
}
