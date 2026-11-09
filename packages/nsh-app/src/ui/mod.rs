//! The app's styled layer: every control the screens compose, built once on `nsh_base`.
//!
//! `nsh_base` supplies behavior and no look, so a screen never styles a base control itself;
//! it calls the wrapper here. Colors come only from `theme.json`.

pub mod controls;
pub mod icons;
pub mod theme;

pub use controls::*;
pub use icons::Icon;
