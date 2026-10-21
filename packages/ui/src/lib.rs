//! Shared Kitty Pro interface for every Dioxus platform shell.

mod app;

pub use app::{
    DesktopTrayBridge, DesktopTrayCommand, DesktopTrayNode, DesktopTrayState,
    DesktopTraySubscription, ProxyApp,
};

/// Complete application styling for native shells that need CSS in their
/// initial WebView document before Dioxus mounts the first frame.
/// The display/data faces are inlined as base64 woff2 ahead of the rules: the
/// first screen is the "not connected" state, so a webfont fetched over the
/// proxy it just started (or with no proxy at all) would delay or lose it.
pub const APP_CSS: &str = concat!(
    include_str!("../assets/styling/fonts.css"),
    include_str!("../assets/styling/app.css"),
);
