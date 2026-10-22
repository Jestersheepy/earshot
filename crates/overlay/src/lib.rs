//! Earshot's always-on-top UI, drawn natively with per-pixel alpha (no webview, no colour-key edges).

pub mod menu;
pub mod paint;
pub mod pill;
pub mod ui;

pub use ui::{Overlay, UiEvent, system_dark};
