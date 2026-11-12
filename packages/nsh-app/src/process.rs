//! The calls that belong to whoever starts the process.
//!
//! Three things have to happen around the host rather than inside it: dispatching the
//! privileged TUN helper when *this* process is that helper, clearing a system proxy a
//! previous run left pointing at a core that is gone, and taking the proxy and the
//! core down on the way out.
//!
//! They live here rather than only in `src/main.rs` because `nsh package` does not
//! compile `main.rs`. It generates and builds its own runner
//! (`.nsh/rust/<target>-app/main.rs`), whose `main` calls `set_ui_sets`,
//! `set_components` and the platform's `main`, and nothing of ours. Measured on a
//! packaged build: launched with the helper flag it opened a window and drew the
//! interface instead of serving the helper, and no string unique to `main.rs` appears
//! in the shipped binary. A packaged Kitty Pro therefore had no TUN at all — which is
//! the desktop release the workflow publishes.
//!
//! [`boot`] reaches a packaged runner through [`crate::native`]. Every runner NSH
//! generates — release, development host and test — evaluates that entry before its
//! `main` body runs, because it is how an app's components get into the UI set. It is
//! the hook NSH 0.4.0 gives a crate it builds for. `#[native_components]` was the
//! other candidate and is the wrong one: its own documentation says the runners call
//! it "on every view they make", which is per-window, not per-process.
//!
//! [`shut_down`] has no such hook: `AppOptions` carries no exit callback, so a
//! packaged build still depends on the helper's lifecycle lease, which stops the core
//! and removes routes when the parent dies but does not restore a system proxy. It is
//! public because `src/main.rs` is the other caller and is a different crate — the one
//! entry point NSH lets us write, which is why it does not duplicate these two calls.
//
// `std::ffi::OsStr` is spelled inline rather than imported: the only thing that takes
// it is gated to the desktop targets, and an import at this level would be unused on
// wasm32, where the web build compiles this module to nothing.

/// Serve the privileged TUN helper when these are its arguments, then clear a stale
/// system proxy. Never returns for a helper: it exits with the helper's code.
pub fn boot() {
    #[cfg(all(
        not(target_arch = "wasm32"),
        any(target_os = "macos", target_os = "windows", target_os = "linux")
    ))]
    {
        if let Some(exit_code) = api::run_tun_helper_from_args() {
            // The helper has served its one command. Returning instead would let the
            // generated runner open a window as root.
            std::process::exit(exit_code);
        }
        // The only other thing boot does, and only where no test can be harmed by it.
        if cleanup_is_ours(std::env::var_os("KITTY_PROFILE_DIR").as_deref()) {
            api::cleanup_stale_system_proxy_sentinel();
        }
    }
}

/// Called by `src/main.rs`, the one entry point NSH lets this crate write. A packaged
/// build does not get it: see the module documentation.
pub fn shut_down() {
    #[cfg(all(
        not(target_arch = "wasm32"),
        any(target_os = "macos", target_os = "windows", target_os = "linux")
    ))]
    if let Err(error) = api::shutdown_native_runtime() {
        // English, and out of the translation table, on purpose: this is a stderr line
        // at exit rather than something drawn on screen, so the i18n sweep would rightly
        // fail on a Chinese literal here. The words are the ones `src/main.rs` used.
        eprintln!("failed to clean up the native runtime before exit: {error}");
    }
}

/// Whether the boot cleanup may touch the operating system's proxy settings.
///
/// The sentinel it reads is scoped by `KITTY_PROFILE_DIR`, but what it *writes* is the
/// machine's real system proxy. Every headless test points that variable at a scratch
/// directory, which is how they are kept off a developer's profile; the same guard
/// keeps them off the developer's network settings, because `nsh test` runs [`boot`]
/// through the same entry a release does. A scratch profile is also the signature of a
/// web server run against a test profile, which must not undo the desktop app's proxy.
#[cfg(all(
    not(target_arch = "wasm32"),
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
fn cleanup_is_ours(scratch_profile: Option<&std::ffi::OsStr>) -> bool {
    scratch_profile.is_none()
}

/// The cleanup guard is the only logic in here, and it exists on desktop targets only.
#[cfg(all(
    test,
    not(target_arch = "wasm32"),
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
mod tests {
    use super::cleanup_is_ours;
    use std::ffi::OsStr;

    /// The hazard [`cleanup_is_ours`] exists for, stated as a test because nothing
    /// else would notice it being inverted: the boot cleanup writes the machine's real
    /// system proxy, so a headless run must not perform it.
    #[test]
    fn a_scratch_profile_keeps_the_boot_cleanup_off_the_machine() {
        // A real launch: no override, so the machine's own proxy is in scope — which is
        // the point, since that is the run that can leave one dangling.
        assert!(cleanup_is_ours(None));
        // A test, or a server on a test profile. Any value counts, including an empty
        // one: what matters is that someone redirected the profile.
        assert!(!cleanup_is_ours(Some(OsStr::new(
            "/tmp/kitty-test-profile"
        ))));
        assert!(!cleanup_is_ours(Some(OsStr::new(""))));
    }
}
