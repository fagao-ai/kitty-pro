use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::{self, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const DAEMON_CHILD: &str = "KITTY_PRO_WEB_DAEMON_CHILD";
const READY_PATH: &str = "/__kitty_pro_daemon_ready";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
pub const HELP: &str = "用法: web [--port <1-65535>] [--daemon [--log-file <路径>]]\n\n  --port       覆盖 PORT 环境变量（默认 7897）\n  --daemon     后台启动，等待服务就绪后退出\n  --log-file   后台日志追加路径（默认 kitty-pro-web.log）\n  -h, --help   显示帮助\n\nIP 环境变量指定绑定 IP（默认 127.0.0.1，仅允许 loopback）。\n保留 dx serve 设置的 PORT/IP；不要在 dx serve 下使用 --daemon。";

#[derive(Debug, Default, PartialEq)]
struct Options {
    port: Option<u16>,
    daemon: bool,
    log_file: Option<PathBuf>,
    help: bool,
}

fn port(value: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("无效端口 {value:?}，必须是 1-65535 的整数"))
}

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let text = arg.to_str().ok_or("参数名必须是 UTF-8 文本")?;
        match text {
            "-h" | "--help" => options.help = true,
            "--daemon" if !options.daemon => options.daemon = true,
            "--port" | "--log-file" => {
                let value = args.next().ok_or_else(|| format!("{text} 缺少参数值"))?;
                set_value(&mut options, text, value)?;
            }
            _ if text.starts_with("--port=") || text.starts_with("--log-file=") => {
                let (name, value) = text.split_once('=').unwrap();
                set_value(&mut options, name, OsString::from(value))?;
            }
            _ => return Err(format!("未知或重复参数 {text:?}；使用 --help 查看用法")),
        }
    }
    if !options.help && options.log_file.is_some() && !options.daemon {
        return Err("--log-file 只能与 --daemon 一起使用".into());
    }
    Ok(options)
}

fn set_value(options: &mut Options, name: &str, value: OsString) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{name} 缺少参数值"));
    }
    match name {
        "--port" if options.port.is_none() => {
            options.port = Some(port(value.to_str().ok_or("端口必须是 UTF-8 文本")?)?);
        }
        "--log-file" if options.log_file.is_none() => {
            options.log_file = Some(PathBuf::from(value));
        }
        _ => return Err(format!("重复参数 {name}")),
    }
    Ok(())
}

fn address(
    options: &Options,
    env_port: Option<&str>,
    env_ip: Option<&str>,
) -> Result<SocketAddr, String> {
    let port = match options.port {
        Some(port) => port,
        None => port(env_port.unwrap_or(super::DEFAULT_WEB_PORT))?,
    };
    let ip: IpAddr = match env_ip {
        Some(ip) => ip.parse().map_err(|_| format!("无效 IP 环境变量 {ip:?}"))?,
        None => Ipv4Addr::LOCALHOST.into(),
    };
    if !ip.is_loopback() {
        return Err("IP 必须是 loopback 地址，当前控制接口不支持对外监听".into());
    }
    Ok(SocketAddr::new(ip, port))
}

fn env_text(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => Err(format!("{name} 环境变量必须是 UTF-8 文本")),
    }
}

/// 返回 false 表示帮助或后台父进程已处理完毕，无需启动 Dioxus。
pub fn prepare() -> Result<bool, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let options = parse(args.clone())?;
    if options.help {
        println!("{HELP}");
        return Ok(false);
    }
    let env_port = if options.port.is_none() {
        env_text("PORT")?
    } else {
        None
    };
    let addr = address(&options, env_port.as_deref(), env_text("IP")?.as_deref())?;
    // Dioxus 0.7 的 serve 从运行时 PORT/IP 读取绑定地址，devserver 变量不是监听端口。
    std::env::set_var("PORT", addr.port().to_string());
    std::env::set_var("IP", addr.ip().to_string());
    if options.daemon && std::env::var_os(DAEMON_CHILD).is_none() {
        if std::env::var_os("DIOXUS_CLI_ENABLED").is_some() {
            return Err("dx serve 下不支持 --daemon，请直接运行发布后的 web 程序".into());
        }
        spawn_daemon(&options, &args, addr)?;
        return Ok(false);
    }
    Ok(true)
}

pub fn readiness_route() -> Option<(String, String)> {
    let token = std::env::var(DAEMON_CHILD).ok()?;
    Some((format!("{READY_PATH}/{token}"), token))
}

fn child_args(args: &[OsString]) -> impl Iterator<Item = &OsString> {
    // --log-file 只影响父进程重定向，不传给已经删除 --daemon 的子进程。
    let mut skip_value = false;
    args.iter().filter(move |arg| {
        if skip_value {
            skip_value = false;
            return false;
        }
        if *arg == "--log-file" {
            skip_value = true;
            return false;
        }
        *arg != "--daemon" && !arg.to_string_lossy().starts_with("--log-file=")
    })
}

fn spawn_daemon(options: &Options, args: &[OsString], addr: SocketAddr) -> Result<(), String> {
    // 提前检查占用；检查与子进程 bind 之间的竞争由独有的 HTTP 就绪标识消除。
    drop(TcpListener::bind(addr).map_err(|error| format!("无法监听 {addr}: {error}"))?);
    let log_path = options
        .log_file
        .clone()
        .unwrap_or_else(|| "kitty-pro-web.log".into());
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("无法打开日志 {}: {error}", log_path.display()))?;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|error| format!("无法生成启动标识: {error}"))?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut command = Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    command
        .args(child_args(args))
        .env(DAEMON_CHILD, &token)
        .stdin(Stdio::null())
        .stderr(Stdio::from(
            log.try_clone().map_err(|error| error.to_string())?,
        ))
        .stdout(Stdio::from(log));
    detach(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("后台启动失败: {error}"))?;
    if let Err(error) = wait_ready(&mut child, addr, &token, STARTUP_TIMEOUT) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{error}；请检查日志 {}", log_path.display()));
    }
    println!(
        "后台服务已就绪，PID={}，监听 {addr}，日志 {}",
        child.id(),
        log_path.display()
    );
    Ok(())
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // pre_exec 中只调用异步信号安全的 setsid，不分配内存或获取锁。
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach(_: &mut Command) {}

fn wait_ready(
    child: &mut Child,
    addr: SocketAddr,
    token: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Err(format!("后台进程在就绪前退出: {status}"));
        }
        if Instant::now() >= deadline {
            return Err(format!("后台启动超时，{addr} 未返回本次启动的就绪标识"));
        }
        if probe_ready(addr, token).unwrap_or(false) {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("后台进程退出: {status}"));
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn probe_ready(addr: SocketAddr, token: &str) -> io::Result<bool> {
    let timeout = Duration::from_millis(200);
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write!(
        stream,
        "GET {READY_PATH}/{token} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let deadline = Instant::now() + timeout;
    let mut response = Vec::new();
    let mut buffer = [0; 1024];
    while response.len() < 8192 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        stream.set_read_timeout(Some(remaining))?;
        let length = stream.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..length]);
    }
    let response = String::from_utf8_lossy(&response);
    Ok(response
        .split_once("\r\n\r\n")
        .is_some_and(|(headers, body)| headers.starts_with("HTTP/1.1 200 ") && body == token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Result<Options, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn default_and_environment_priority() {
        let opts = options(&[]).unwrap();
        assert_eq!(
            address(&opts, None, None).unwrap(),
            "127.0.0.1:7897".parse().unwrap()
        );
        assert_eq!(
            address(&opts, Some("8123"), Some("::1")).unwrap(),
            "[::1]:8123".parse().unwrap()
        );
        let opts = options(&["--port", "9000"]).unwrap();
        assert_eq!(address(&opts, Some("invalid"), None).unwrap().port(), 9000);
        assert!(address(&Options::default(), Some("invalid"), None).is_err());
        assert!(address(&Options::default(), None, Some("0.0.0.0")).is_err());
        assert!(address(&Options::default(), None, Some("invalid")).is_err());
    }

    #[test]
    fn parse_flags_and_values() {
        assert_eq!(
            options(&["--port=9000", "--daemon", "--log-file=a.log"]).unwrap(),
            Options {
                port: Some(9000),
                daemon: true,
                log_file: Some("a.log".into()),
                help: false,
            }
        );
        assert!(options(&["--help"]).unwrap().help);
        assert!(options(&["-h"]).unwrap().help);
        for args in [
            vec!["--unknown"],
            vec!["--port"],
            vec!["--port=0"],
            vec!["--port=-1"],
            vec!["--port=65536"],
            vec!["--port=x"],
            vec!["--port=1", "--port=2"],
            vec!["--daemon", "--daemon"],
            vec!["--log-file=a"],
            vec!["--daemon", "--log-file="],
            vec!["--daemon", "--log-file=a", "--log-file=b"],
        ] {
            assert!(options(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn daemon_arguments_removed() {
        let args: Vec<_> = ["--daemon", "--port", "8123", "--log-file", "--daemon"]
            .map(OsString::from)
            .into();
        assert_eq!(
            child_args(&args).cloned().collect::<Vec<_>>(),
            ["--port", "8123"].map(OsString::from)
        );
        let args = ["--log-file=a.log", "--daemon", "--port=8123"].map(OsString::from);
        assert_eq!(
            child_args(&args).cloned().collect::<Vec<_>>(),
            ["--port=8123"].map(OsString::from)
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_exit_and_timeout_are_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        child.wait().unwrap();
        assert!(
            wait_ready(&mut child, addr, "token", Duration::from_secs(1))
                .unwrap_err()
                .contains("退出")
        );
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read value"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let result = wait_ready(&mut child, addr, "token", Duration::from_millis(100));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(result.unwrap_err().contains("超时"));
    }

    fn read_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() <= 8192);
        }
        String::from_utf8(request).unwrap()
    }

    #[test]
    fn matching_listener_is_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let responder = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.contains("/__kitty_pro_daemon_ready/expected"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nexpected",
                )
                .unwrap();
        });
        assert!(probe_ready(addr, "expected").unwrap());
        responder.join().unwrap();
    }

    #[test]
    fn existing_listener_is_not_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let responder = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nother",
                )
                .unwrap();
        });
        assert!(!probe_ready(addr, "expected").unwrap_or(false));
        responder.join().unwrap();
    }
}
