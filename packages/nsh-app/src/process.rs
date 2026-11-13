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
//! The exit cleanup finds no hook at a runner either — `AppOptions` carries no exit
//! callback — but NSH 0.5 added one *inside* the app: `register_exit_cleanup` takes
//! `on_app_quit` from the root component, which every runner reaches because every
//! runner mounts it. It runs on the Quit item, on the last window closing, and when the
//! system ends the app, in a packaged build exactly as in `cargo run`.
//!
//! Before 0.5 there was genuinely nothing to hook — the sentence this one replaces was
//! true when written — and a packaged build had only the helper's lifecycle lease, which
//! stops the core and removes routes when the parent dies but does not restore a system
//! proxy. Quitting with 系统代理 on therefore left the machine pointed at a
//! 127.0.0.1:7890 that nothing answered, which is what users hit. The boot cleanup is now
//! the fallback it was always meant to be — for a run that was killed rather than quit —
//! rather than the only cure, one that could only ever act a session too late.
//!
//! [`shut_down`] stays for `src/main.rs`, the one entry point NSH lets this crate write
//! and a different crate's caller. It is not made to defer to the hook: the cleanup is
//! idempotent, and a development run that somehow mounted no component still cleans up.
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
/// through the same entry a release does — and for the same reason it guards
/// [`register_exit_cleanup`], which would otherwise fire on every window the suite
/// closes. A scratch profile is also the signature of a web server run against a test
/// profile, which must not undo the desktop app's proxy.
#[cfg(all(
    not(target_arch = "wasm32"),
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
fn cleanup_is_ours(scratch_profile: Option<&std::ffi::OsStr>) -> bool {
    scratch_profile.is_none()
}

/// Take the exit cleanup through the app's own quit hook, so a packaged build gets it.
///
/// Call this from a component. `on_app_quit` belongs to whichever owner is current and is
/// removed with it — with no owner it never runs at all — which is why it is the root
/// component's and not [`boot`]'s: the `native_module` entry has no owner to hang one on.
/// Every runner mounts the root component, including the one `nsh package` generates, so
/// this is the only place a packaged build can be reached from.
///
/// What it runs is [`api::disable_managed_system_proxy_before_core_outage`] and nothing
/// more — deliberately not [`api::shutdown_native_runtime`], which is what [`shut_down`]
/// calls. That one takes `core_slot`, and `restart_native_core` holds that lock across the
/// bind retry window and the helper authorization wait; a listener that blocked on it would
/// block the engine thread, and `Engine::drop` joins that thread with no deadline once its
/// quit budget is spent — so quitting during a core restart would hold the window closed
/// only by waiting out a transaction this cleanup does not need. Clearing the proxy takes
/// the proxy operation lock and nothing else.
///
/// Not stopping the core costs nothing, though the two modes stop it differently. Normally
/// the core is embedded, so this process ending ends it. In TUN mode it runs in a root
/// re-launch of this same executable, and that helper watches the lifecycle lease socket
/// `singbox::macos` holds open — an exit closes the socket, and the helper takes the core
/// and its routes down with it. Either way the one thing that outlives the process is the
/// system proxy, which is exactly what the listener is here for.
///
/// The work is synchronous on purpose. The engine gives listeners
/// [`nsh_core::element::desktop::QUIT_TIMEOUT`] (200ms) and then quits regardless, so
/// handing the cleanup to a thread would let the process end while it was still writing —
/// the half-written proxy this exists to prevent. Measured here, the clear is about 15ms
/// per `networksetup` call over nine calls, which fits, and the sentinel is what catches
/// the case where it does not: a run cut off mid-write leaves it behind, and the next boot
/// clears a proxy with nothing listening on its port.
pub fn register_exit_cleanup() {
    #[cfg(all(
        not(target_arch = "wasm32"),
        any(target_os = "macos", target_os = "windows", target_os = "linux")
    ))]
    {
        // Decided here rather than inside the listener, and the difference is not
        // cosmetic: a registered listener is one the engine waits for at every shutdown,
        // so registering a listener that only returned early would still move every one of
        // this suite's ~70 engines onto a quit path they never used to take. Registering
        // nothing leaves them where they were measured.
        if !cleanup_is_ours(std::env::var_os("KITTY_PROFILE_DIR").as_deref()) {
            return;
        }
        nsh_core::element::desktop::on_app_quit(move || async move {
            if let Err(error) = api::disable_managed_system_proxy_before_core_outage() {
                // Same line as [`shut_down`]'s, same reason it is English. A packaged
                // app started by Finder writes stderr to /dev/null; the runner installs
                // the unified-log handler, so it is lost there rather than silent here.
                eprintln!("failed to clean up the native runtime before exit: {error}");
            }
        });
    }
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
