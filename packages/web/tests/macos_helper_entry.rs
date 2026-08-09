#![cfg(target_os = "macos")]

use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn helper_arguments_exit_before_dioxus_launch() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_web"))
        .arg("--kitty-pro-tun-helper")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("web test binary should launch");
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        match child
            .try_wait()
            .expect("web test binary should be observable")
        {
            Some(status) => {
                let output = child
                    .wait_with_output()
                    .expect("web test binary output should be readable");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(!status.success(), "incomplete helper arguments must fail");
                assert!(
                    stderr.contains("invalid TUN helper arguments"),
                    "helper arguments fell through to another entry point: {stderr}"
                );
                assert!(!stderr.contains("Failed to bind"), "{stderr}");
                return;
            }
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            None => {
                child
                    .kill()
                    .expect("hung web test binary should be stoppable");
                let output = child
                    .wait_with_output()
                    .expect("hung web test binary output should be readable");
                panic!(
                    "helper arguments reached Dioxus instead of exiting; stdout: {}; stderr: {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}
