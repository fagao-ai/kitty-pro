//! The app's semantic tokens.
//!
//! Colors live in `theme.json`; presentation code reads them here so no screen writes a
//! literal. A token that is missing falls back to a role whose absence is obvious on screen,
//! rather than panicking.

use nsh_base::theme;
use nsh_core::{
    element::{AdaptiveColor, SharedString},
    geometry::{px, Length, Points},
    BoxShadow,
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

/// The switch's thumb. Opaque in both appearances, which is the whole reason it has a role of
/// its own: the card color is a 68% white and the track it sits on is an 82% white, so a thumb
/// painted with the card is the same color as the track and the switch shows no state at all.
/// The deleted Dioxus `packages/ui` reached for `--switch-thumb` for the same reason.
pub fn switch_thumb() -> AdaptiveColor {
    theme()
        .color("switch-thumb")
        // The doctrine here is that a missing token falls back to something obviously wrong.
        // `foreground` is the inverted text color, so a theme that loses this role shows a
        // dark thumb in the light appearance rather than a quietly stateless switch.
        .unwrap_or(theme().foreground)
}

/// The switch thumb's drop shadow, the one layer `app.css` gives it.
///
/// This is not decoration. In the light appearance the thumb is `#ffffff` and the off track is
/// an 82% white, so the two differ by a few percent of luminance — the shadow is the only thing
/// that separates a thumb from the track it sits in. Dropping it makes an off switch read as a
/// blank pill.
pub fn switch_thumb_shadow() -> BoxShadow {
    // `app.css` gives the thumb one shadow for both appearances, so the role carries the same
    // value in each half and either resolves to it.
    let color = theme()
        .color("switch-thumb-shadow")
        .unwrap_or_else(|| theme().foreground.opacity(0.2));
    BoxShadow {
        offset_x: px(0.0),
        offset_y: px(2.0),
        blur: px(5.0),
        spread: px(0.0),
        color: color.light.into(),
        inset: false,
    }
}

/// The disc's fill for its skin, read from the role `domain::DiscSkin` names.
pub fn disc_fill(skin: crate::domain::DiscSkin) -> AdaptiveColor {
    theme().color(skin.fill_role()).unwrap_or(theme().muted)
}

/// The glyph's colour, paired with the fill so both clear AA in either skin.
pub fn disc_glyph(skin: crate::domain::DiscSkin) -> AdaptiveColor {
    match skin {
        crate::domain::DiscSkin::Live => theme().primary_foreground,
        crate::domain::DiscSkin::Idle => text_soft(),
    }
}

/// The ring: the accent while live, the input line otherwise. Always 1pt wide, so switching
/// skins moves no pixels.
pub fn disc_border(skin: crate::domain::DiscSkin) -> AdaptiveColor {
    match skin {
        crate::domain::DiscSkin::Live => theme().primary,
        crate::domain::DiscSkin::Idle => theme().input,
    }
}

/// The floating action's drop shadow, in the accent's own colour.
///
/// `app.css` gives the power button `box-shadow: 0 18px 46px var(--accent-glow)` while it is
/// lit, and nothing while it is not: a floating control has no card edge to borrow, so the
/// shadow is what puts it above the page.
pub fn action_glow() -> BoxShadow {
    BoxShadow {
        offset_x: px(0.0),
        offset_y: px(10.0),
        blur: px(28.0),
        spread: px(-4.0),
        color: glow_color(),
        inset: false,
    }
}

/// The ring around a lit action: the accent's wash, at 6pt, with no blur.
///
/// The second half of the original's `box-shadow` list. It is what makes "running" readable at
/// a glance from across the window, and it costs no layout because a shadow draws outside the
/// view.
pub fn action_halo() -> BoxShadow {
    BoxShadow {
        offset_x: px(0.0),
        offset_y: px(0.0),
        blur: px(0.0),
        spread: px(6.0),
        color: theme()
            .color("accent")
            .unwrap_or_else(|| theme().primary.opacity(0.11))
            .light
            .into(),
        inset: false,
    }
}

/// The floating action's shadows, which is where its state is written.
///
/// Lit: the accent's own drop shadow and a 6pt halo, the two layers `app.css` gives a running
/// power button. Unlit: a single short, faint shadow — enough to sit the disc on the page
/// without claiming anything.
pub fn action_shadows(skin: crate::domain::DiscSkin) -> Vec<BoxShadow> {
    if skin == crate::domain::DiscSkin::Live {
        vec![action_glow(), action_halo()]
    } else {
        vec![BoxShadow {
            offset_x: px(0.0),
            offset_y: px(4.0),
            blur: px(12.0),
            spread: px(-4.0),
            color: theme().foreground.opacity(0.18).light.into(),
            inset: false,
        }]
    }
}

fn glow_color() -> nsh_core::Color {
    theme()
        .color("accent-glow")
        .unwrap_or_else(|| theme().primary.opacity(0.35))
        .light
        .into()
}

/// Points for one step of the app's 4pt spacing scale.
pub fn space(steps: f32) -> Points {
    px(steps * 4.0)
}
