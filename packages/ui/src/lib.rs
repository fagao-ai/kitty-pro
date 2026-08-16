//! Shared Kitty Pro interface for every Dioxus platform shell.

mod app;

pub use app::{
    DesktopTrayBridge, DesktopTrayCommand, DesktopTrayNode, DesktopTrayState,
    DesktopTraySubscription, ProxyApp,
};

/// Complete application styling for native shells that need CSS in their
/// initial WebView document before Dioxus mounts the first frame.
pub const APP_CSS: &str = include_str!("../assets/styling/app.css");
