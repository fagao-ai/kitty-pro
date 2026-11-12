//! Kitty Pro on Native Shell: a sing-box client whose engine is the app itself.
//!
//! The backend is the workspace's own `api` and `proxy-core` crates, called directly. An NSH
//! binary is a native process, so there is no HTTP hop and no server to run: every backend call
//! is a plain async function call.

pub(crate) mod autostart;
/// The backend: called directly, or over HTTP when this build runs in a browser.
pub mod backend;
mod domain;
pub mod feedback;
/// Chinese and English, keyed by the Chinese the app was written in.
pub mod i18n;
mod pages;
/// The calls that belong to whoever starts the process.
///
/// Public because `src/main.rs` is the binary target — a separate crate from this
/// library — and it is the entry point NSH lets us write for `cargo run`.
pub mod process;
pub(crate) mod shell;
mod store;
pub mod ui;

/// The band the window's traffic lights occupy once the title bar is hidden. Re-exported because
/// the test that keeps the rail's brand from sitting under those buttons has to measure against
/// the same number the shell reserves.
pub use shell::TITLE_BAR_HEIGHT;

use nsh_base::tokens_of;
use nsh_core::{
    element::{Registry, Root, Tokens},
    platform::Appearance,
    prelude::*,
};

/// The app's design tokens, from `theme.json`. A file that does not parse would otherwise
/// leave the app silently on NSH's defaults, so the error surfaces at the caller.
pub const THEME_JSON: &str = include_str!("../theme.json");

/// The three faces the app draws with, as variable fonts. Latin only: CJK falls through to
/// the system UI font.
pub const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Variable.woff2"),
    include_bytes!("../assets/fonts/SpaceGrotesk-Variable.woff2"),
    include_bytes!("../assets/fonts/JetBrainsMono-Variable.woff2"),
];

/// The component set NSH's hosts and headless tests mount.
///
/// NSH's own generated runners call this before their `main` body, on every target,
/// which makes it the only place a packaged build can be given its process boot. That
/// is why [`process::boot`] runs here and not only in `main.rs`; see that module.
#[native_module("kitty/native")]
fn native(registry: &mut Registry) {
    process::boot();
    registry.register::<App>();
}

/// The app's tokens, or the error from parsing `theme.json`.
pub fn tokens() -> Result<Tokens, String> {
    tokens_of(Some(THEME_JSON))
}

/// The root component with the app's theme and fonts attached: what every host mounts.
///
/// # Panics
///
/// If `theme.json` does not parse. The file ships in the crate and its shape is checked by
/// `theme_json_parses`, so a panic here means the file was edited into an invalid state.
pub fn app_root() -> Root {
    let tokens = tokens().expect("theme.json must parse");
    FONTS
        .iter()
        .fold(Root::of::<App>().tokens(tokens), |root, font| {
            root.font(*font)
        })
        .app_id("com.kitty.pro")
        // The stored profile decides the appearance once it loads, so the window starts with
        // the system's rather than guessing light.
        .appearance(Appearance::System)
}

/// The root component: it owns the store and the chrome, and nothing else.
#[component]
pub fn App() -> impl IntoElement {
    provide_context(store::AppStore::new());
    shell::root()
}
