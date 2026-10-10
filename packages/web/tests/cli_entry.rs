use std::{net::TcpListener, process::Command};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_web"));
    for name in [
        "PORT",
        "IP",
        "DIOXUS_CLI_ENABLED",
        "DIOXUS_DEVSERVER_IP",
        "DIOXUS_DEVSERVER_PORT",
        "DIOXUS_DEVSERVER_WS_ENDPOINT",
        "DIOXUS_PUBLIC_PATH",
        "DIOXUS_BASE_PATH",
        "KITTY_PRO_WEB_DAEMON_CHILD",
    ] {
        command.env_remove(name);
    }
    command
}

#[test]
fn help_exits_without_starting_server() {
    let output = command()
        .arg("--help")
        .env("PORT", "invalid")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("7897"));
    assert!(stdout.contains("--daemon"));
    assert!(stdout.contains("--log-file"));
}

#[test]
fn invalid_arguments_fail_before_starting_server() {
    for args in [
        vec!["--unknown"],
        vec!["--port"],
        vec!["--port", "0"],
        vec!["--log-file", "unused.log"],
    ] {
        let output = command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("错误:"));
        assert!(!stderr.contains("Failed to bind"));
    }
    let output = command().env("IP", "0.0.0.0").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("loopback"));
}

#[test]
fn occupied_port_does_not_report_daemon_success() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let output = command()
        .args([
            "--daemon",
            "--port",
            &listener.local_addr().unwrap().port().to_string(),
        ])
        .env("PORT", "invalid")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("无法监听"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("已就绪"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod daemon_success {
    use super::*;
    use std::{
        fs::{self, File},
        io::{Read, Write},
        net::{SocketAddr, TcpStream},
        path::PathBuf,
        process::{Child, Stdio},
        time::{Duration, Instant},
    };

    struct DaemonTest {
        directory: PathBuf,
        parent: Option<Child>,
        pid: Option<libc::pid_t>,
    }

    impl DaemonTest {
        fn new() -> Self {
            let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
            fs::create_dir_all(&target).unwrap();
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let directory = target.join(format!("web-daemon-test-{suffix}"));
            fs::create_dir(&directory).unwrap();
            Self {
                directory,
                parent: None,
                pid: None,
            }
        }

        fn finish_parent(&mut self) -> std::process::ExitStatus {
            let deadline = Instant::now() + Duration::from_secs(25);
            loop {
                if let Some(status) = self.parent.as_mut().unwrap().try_wait().unwrap() {
                    self.parent = None;
                    self.read_pid();
                    return status;
                }
                assert!(Instant::now() < deadline, "daemon 父进程未及时退出");
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        fn read_pid(&mut self) {
            let stdout =
                fs::read_to_string(self.directory.join("parent.stdout")).unwrap_or_default();
            self.pid = stdout
                .split_once("PID=")
                .and_then(|(_, rest)| rest.split('，').next())
                .and_then(|pid| pid.parse::<libc::pid_t>().ok())
                .filter(|pid| *pid > 1);
        }

        fn running(pid: libc::pid_t) -> bool {
            #[cfg(target_os = "linux")]
            if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
                // 非本测试直接子进程由 init 回收；僵尸状态也代表已经退出。
                if stat
                    .rsplit_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with('Z'))
                {
                    return false;
                }
            }
            unsafe { libc::kill(pid, 0) == 0 }
        }

        fn stop(&mut self) -> bool {
            let Some(pid) = self.pid else { return true };
            if Self::running(pid) {
                // PID 仅来自本测试启动的二进制输出，不按端口查找或终止任何服务。
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while Self::running(pid) {
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            self.pid = None;
            true
        }
    }

    impl Drop for DaemonTest {
        fn drop(&mut self) {
            if let Some(mut parent) = self.parent.take() {
                // 即使断言失败，也等待 CLI 完成自身的就绪/失败清理再读取其 PID。
                let _ = parent.wait();
                self.read_pid();
            }
            if self.stop() {
                let _ = fs::remove_dir_all(&self.directory);
            } else {
                eprintln!(
                    "测试子进程未确认退出，保留诊断目录 {}",
                    self.directory.display()
                );
            }
        }
    }

    #[test]
    fn real_binary_daemon_is_ready_redirects_logs_and_exits() {
        let mut daemon = DaemonTest::new();
        let public = daemon.directory.join("public");
        fs::create_dir(&public).unwrap();
        fs::write(public.join("health.html"), "daemon-test-page").unwrap();
        let log = daemon.directory.join("daemon.log");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let stdout = File::create(daemon.directory.join("parent.stdout")).unwrap();
        let stderr = File::create(daemon.directory.join("parent.stderr")).unwrap();
        let mut cmd = command();
        cmd.current_dir(&daemon.directory)
            .args(["--daemon", "--port", &addr.port().to_string(), "--log-file"])
            .arg(&log)
            .env("DIOXUS_PUBLIC_PATH", &public)
            .env("RUST_LOG", "warn")
            .env("HOME", &daemon.directory)
            .env("XDG_CONFIG_HOME", &daemon.directory)
            .env("XDG_DATA_HOME", &daemon.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        drop(listener);
        daemon.parent = Some(cmd.spawn().unwrap());
        let status = daemon.finish_parent();
        let stdout = fs::read_to_string(daemon.directory.join("parent.stdout")).unwrap();
        let stderr = fs::read_to_string(daemon.directory.join("parent.stderr")).unwrap();
        assert!(
            status.success(),
            "{status}: {stderr}; 日志: {:?}",
            fs::read_to_string(&log)
        );
        assert!(stdout.contains("后台服务已就绪"), "{stdout}");
        let pid = daemon.pid.expect("成功输出必须包含子进程 PID");
        assert!(DaemonTest::running(pid));

        let timeout = Duration::from_secs(2);
        let mut stream = TcpStream::connect_timeout(&addr, timeout).unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        stream.set_write_timeout(Some(timeout)).unwrap();
        write!(
            stream,
            "GET /health.html HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.take(65536).read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 "), "{response}");
        assert!(response.contains("daemon-test-page"), "{response}");

        // 本地 Dioxus ServeConfig::new 在缺失 index.html 时必定写出该警告。
        let warning = "No index.html found in public directory";
        let logs = fs::read_to_string(&log).unwrap();
        assert!(logs.contains(warning), "子进程日志未实际重定向: {logs}");
        assert!(!stdout.contains(warning));
        assert!(!stderr.contains(warning));
        assert!(daemon.stop(), "子进程未确认退出");
        assert!(!DaemonTest::running(pid));
    }
}

#[test]
fn dx_serve_rejects_detaching() {
    let output = command()
        .arg("--daemon")
        .env("DIOXUS_CLI_ENABLED", "true")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("dx serve"));
}
