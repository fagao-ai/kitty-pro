//! The desktop runner.
//!
//! ```sh
//! cargo run -p kitty-pro --features desktop
//! ```
//!
//! The windowing backends sit behind `desktop` because the Linux one needs GTK 4.14
//! development packages that the CI runners do not carry; the library and its headless
//! tests build without them. NSH's own CLI (`nsh run`) is the other way in, and the only
//! one that reaches iOS, Android and the web.
//!
//! What happens around the host call belongs to `kitty_pro::process`, not to this
//! file: the same three calls have to run in a packaged build too, and `nsh package`
//! compiles its own generated runner that never reaches `main`. Reading them here as
//! local functions is exactly how a packaged Kitty Pro ended up with no TUN.

#[cfg(all(feature = "desktop", target_os = "macos"))]
fn main() {
    kitty_pro::process::boot();
    // The host returns when the window closes, and every task is dropped with the app.
    let _ = nsh_platform_macos::run_app(
        nsh_platform_macos::AppSource::Rust(kitty_pro::app_root()),
        nsh_platform_macos::AppOptions {
            app_id: "com.kitty.pro".to_owned(),
            title: "Kitty Pro".to_owned(),
            size: (1280, 840),
            ..Default::default()
        },
    );
    kitty_pro::process::shut_down();
}

#[cfg(all(feature = "desktop", target_os = "linux"))]
fn main() {
    kitty_pro::process::boot();
    let _ = nsh_platform_linux::run_app(
        nsh_platform_linux::AppSource::Rust(kitty_pro::app_root()),
        nsh_platform_linux::AppOptions {
            app_id: "com.kitty.pro".to_owned(),
            title: "Kitty Pro".to_owned(),
            size: (1280, 840),
            ..Default::default()
        },
    );
    kitty_pro::process::shut_down();
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
         `cargo run -p kitty-pro --features desktop`, or `nsh run` for every target."
    );
}
