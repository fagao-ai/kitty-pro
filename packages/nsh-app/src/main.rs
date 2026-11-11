//! The desktop runner.
//!
//! ```sh
//! cargo run -p nsh-app-kitty --features desktop
//! ```
//!
//! The windowing backends sit behind `desktop` because the Linux one needs GTK 4.14
//! development packages that the CI runners do not carry; the library and its headless
//! tests build without them. NSH's own CLI (`nsh run`) is the other way in, and the only
//! one that reaches iOS, Android and the web.
//!
//! Two things happen around the host call that the Dioxus desktop shell used to own, and
//! that belong to whichever process entry point starts the app: clearing a system proxy
//! left dangling by a run that died without cleanup, and cleaning up on the way out.
//!
//! They apply to *this* runner only. `nsh package` compiles its own generated runner, which
//! never reaches this file, so a packaged build gets neither — see the "What is still
//! missing" table in the crate README.

/// Serve the privileged TUN helper when these are its arguments.
///
/// TUN runs the core in a second copy of this executable, re-launched with
/// administrator rights and one identifying flag. That copy has to answer its
/// parent over a socket instead of opening a window, so every entry point must
/// make this call before it starts a UI. `None` means these are ordinary GUI
/// arguments.
#[cfg(all(feature = "desktop", any(target_os = "macos", target_os = "linux")))]
fn serve_tun_helper() {
    if let Some(exit_code) = api::run_tun_helper_from_args() {
        std::process::exit(exit_code);
    }
}

/// Clear a system proxy that a previous run left pointing at a core that is gone.
#[cfg(all(feature = "desktop", any(target_os = "macos", target_os = "linux")))]
fn clear_stale_system_proxy() {
    api::cleanup_stale_system_proxy_sentinel();
}

/// Take the system proxy back down and stop the core, so the next launch does not find a
/// dead loopback proxy in the OS settings.
#[cfg(all(feature = "desktop", any(target_os = "macos", target_os = "linux")))]
fn shut_down_runtime() {
    if let Err(error) = api::shutdown_native_runtime() {
        eprintln!("failed to clean up the native runtime before exit: {error}");
    }
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
fn main() {
    serve_tun_helper();
    clear_stale_system_proxy();
    // The host returns when the window closes, and every task is dropped with the app.
    let _ = nsh_platform_macos::run_app(
        nsh_platform_macos::AppSource::Rust(nsh_app_kitty::app_root()),
        nsh_platform_macos::AppOptions {
            app_id: "com.kitty.pro".to_owned(),
            title: "Kitty Pro".to_owned(),
            size: (1280, 840),
            ..Default::default()
        },
    );
    shut_down_runtime();
}

#[cfg(all(feature = "desktop", target_os = "linux"))]
fn main() {
    serve_tun_helper();
    clear_stale_system_proxy();
    let _ = nsh_platform_linux::run_app(
        nsh_platform_linux::AppSource::Rust(nsh_app_kitty::app_root()),
        nsh_platform_linux::AppOptions {
            app_id: "com.kitty.pro".to_owned(),
            title: "Kitty Pro".to_owned(),
            size: (1280, 840),
            ..Default::default()
        },
    );
    shut_down_runtime();
}

#[cfg(all(
    feature = "desktop",
    not(any(target_os = "macos", target_os = "linux"))
))]
fn main() {
    compile_error!("the desktop host covers macOS and Linux; other targets use `nsh run`");
}

#[cfg(not(feature = "desktop"))]
fn main() {
    eprintln!(
        "Kitty Pro's NSH screens are in the library; run them with \
         `cargo run -p nsh-app-kitty --features desktop`, or `nsh run` for every target."
    );
}
