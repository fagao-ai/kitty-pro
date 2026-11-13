//! The quit hook a packaged build's exit cleanup rests on, checked on NSH's own engine.
//!
//! `src/process.rs` clears the system proxy through
//! `nsh_core::element::desktop::on_app_quit`, because `nsh package` compiles a runner of
//! its own whose `main` returns straight to exit — the app gets no code after the window
//! closes, and a run that quit with 系统代理 on left the machine pointing at a core that
//! was already gone. That fix is worth one thing only if NSH runs the listener as the
//! engine stops. It is a promise made by a dependency: it holds today, and nothing else in
//! this repo would notice the day it does not.
//!
//! So this file tests the promise, not the proxy. The cleanup itself is deliberately out
//! of reach of the suite — it writes the machine's real system proxy, which is why
//! `process::cleanup_is_ours` refuses to run it under a scratch profile, and the same
//! reason no test here may call it.

use std::sync::atomic::{AtomicUsize, Ordering};

use nsh_core::element::desktop::on_app_quit;
use nsh_core::element::Root;
use nsh_core::prelude::*;
use nsh_testing::Headless;

/// Quit listeners that have run, counted rather than flagged: the value is read before the
/// engine starts and after it stops, so the test asks whether *this* engine's quit ran and
/// not merely whether some listener ran at some point.
static RAN: AtomicUsize = AtomicUsize::new(0);

/// The component the engine runs. One only, and this file holds one test, because `RAN` is
/// shared by the process and two engines quitting at once would both answer to it.
#[component]
fn QuitProbe() -> impl IntoElement {
    // Registered from a component on purpose: a listener belongs to the owner that adds
    // it and never runs without one, which is the shape `register_exit_cleanup` relies on.
    on_app_quit(|| async move {
        RAN.fetch_add(1, Ordering::SeqCst);
    });
    view().child(text("mounted"))
}

#[test]
fn a_quit_listener_runs_as_the_engine_stops() {
    let before = RAN.load(Ordering::SeqCst);

    // `Headless` owns the `Engine`, and `Engine::drop` is the shutdown a closing window
    // turns into, so leaving this scope is the app quitting.
    {
        let mut app = Headless::start(nsh_engine::EngineConfig::new(Root::of::<QuitProbe>()))
            .expect("the probe starts");
        app.wait_for("mounted").expect("the probe mounts");
        assert_eq!(
            RAN.load(Ordering::SeqCst),
            before,
            "the quit listener ran while the app was still up"
        );
    }

    assert_eq!(
        RAN.load(Ordering::SeqCst),
        before + 1,
        "the quit listener did not run as the engine stopped: `process::register_exit_\
         cleanup` has no hook to hang on, and a packaged build leaves the system proxy \
         pointing at a dead core"
    );
}
