//! Earshot's always-on-top UI, drawn natively with per-pixel alpha (no webview, no colour-key edges).

pub mod menu;
pub mod paint;
pub mod pill;
pub mod theme;
pub mod ui;

pub use theme::system_dark;
pub use ui::{Overlay, UiEvent};
