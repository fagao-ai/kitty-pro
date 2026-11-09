//! The app's semantic tokens.
//!
//! Colors live in `theme.json`; presentation code reads them here so no screen writes a
//! literal. A token that is missing falls back to a role whose absence is obvious on screen,
//! rather than panicking.

use nsh_base::theme;
use nsh_core::{
    element::{AdaptiveColor, SharedString},
    geometry::{px, Length, Points},
};

/// The status families the app colors: proxy outcomes and protocol groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Success,
    Info,
    Warning,
    Danger,
    Dns,
    /// No status: a row's plain mark, drawn from the quiet roles rather than a hue, so a
    /// list of them does not read as a legend.
    Neutral,
}

impl Tint {
    /// The saturated foreground for this status.
    pub fn foreground(self) -> AdaptiveColor {
        let name = match self {
            Tint::Success => "success",
            Tint::Info => "info",
            Tint::Warning => "warning",
            Tint::Danger => "danger",
            Tint::Dns => "dns",
            Tint::Neutral => return theme().muted_foreground,
        };
        theme().color(name).unwrap_or_else(|| theme().foreground)
    }

    /// The wash behind this status: a badge background, a row tint.
    pub fn wash(self) -> AdaptiveColor {
        let name = match self {
            Tint::Success => "success-soft",
            Tint::Info => "info-soft",
            Tint::Warning => "warning-soft",
            Tint::Danger => "danger-soft",
            Tint::Dns => "dns-soft",
            Tint::Neutral => return theme().muted,
        };
        theme()
            .color(name)
            .unwrap_or_else(|| theme().muted.opacity(0.5))
    }
}

nsh_core::static_reactive!(Tint: Tint);

/// Secondary copy on a card: a node's host, a rule's note.
pub fn text_soft() -> AdaptiveColor {
    theme()
        .color("text-soft")
        .unwrap_or(theme().muted_foreground)
}

/// Metadata that must stay legible: timestamps, units, empty states.
pub fn text_faint() -> AdaptiveColor {
    theme()
        .color("text-faint")
        .unwrap_or(theme().muted_foreground)
}

/// The card and sheet corner (`--radius-card`).
pub fn radius_card() -> Length {
    Points::from(theme().number("radius-card").unwrap_or(14.0)).into()
}

/// The corner for rows, chips and compact controls (`--radius-tile`).
pub fn radius_tile() -> Length {
    Points::from(theme().number("radius-tile").unwrap_or(10.0)).into()
}

/// The display face: brand lockup and page titles.
pub fn font_display() -> SharedString {
    theme()
        .font("font-display")
        .map(SharedString::from)
        .unwrap_or_else(|| SharedString::from("Inter"))
}

/// The monospaced face: logs, addresses, byte counts.
pub fn font_mono() -> SharedString {
    theme()
        .font("font-mono")
        .map(SharedString::from)
        .or(theme().font_mono.clone())
        .unwrap_or_else(|| SharedString::from("Menlo"))
}

/// Points for one step of the app's 4pt spacing scale.
pub fn space(steps: f32) -> Points {
    px(steps * 4.0)
}
