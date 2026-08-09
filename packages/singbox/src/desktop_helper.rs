#![cfg_attr(test, allow(dead_code))]

use crate::{
    CoreError, CoreState, CoreStatus, EmbeddedCore, LatencyProbeResult, LogBatch, TrafficStats,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

const HELPER_FLAG: &str = "--kitty-pro-tun-helper";
const MAX_MESSAGE_BYTES: u64 = 16 * 1024 * 1024;
const HELPER_START_TIMEOUT: Duration = Duration::from_secs(120);
const HELPER_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const HELPER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub(crate) struct PrivilegedCore {
    rpc_port: u16,
    token: String,
    lease: Option<TcpStream>,
    child: Option<Child>,
}

impl PrivilegedCore {
    pub(crate) fn launch() -> Result<Self, CoreError> {
        let executable = std::env::current_exe()
            .map_err(|error| helper_error(format!("无法定位可执行文件: {error}")))?;
        let lease_listener = loopback_listener()
            .map_err(|error| helper_error(format!("创建 helper 生命周期租约失败: {error}")))?;
        lease_listener
            .set_nonblocking(true)
            .map_err(|error| helper_error(format!("配置 helper 生命周期租约失败: {error}")))?;
        let lease_port = listener_port(&lease_listener)?;
        let rpc_port = reserve_loopback_port()?;
        let token = random_token()?;
        let token_path = write_session_token(&token)?;
        let child = match launch_elevated(
            &executable,
            rpc_port,
            lease_port,
            &token_path,
            session_owner_id(),
        ) {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_file(&token_path);
                return Err(error);
            }
        };
        let mut helper = Self {
            rpc_port,
            token,
            lease: None,
            child: Some(child),
        };
        let ready = helper.wait_until_ready(&lease_listener);
        let _ = fs::remove_file(&token_path);
        if let Err(error) = ready {
            helper.shutdown();
            return Err(error);
        }
        Ok(helper)
    }

    pub(crate) fn start(&mut self, config: &Value) -> Result<(), CoreError> {
        self.request(HelperAction::Start {
            config: config.clone(),
        })?
        .ensure_success(helper_error)
    }

    pub(crate) fn stop(&mut self) -> Result<(), CoreError> {
        self.request(HelperAction::Stop)?
            .ensure_success(helper_error)
    }

    pub(crate) fn is_running(&self) -> Result<bool, CoreError> {
        let response = self.request(HelperAction::IsRunning)?;
        response.ensure_success(helper_error)?;
        response
            .running
            .ok_or_else(|| helper_error("helper 未返回运行状态".to_string()))
    }

    pub(crate) fn is_ready(&self) -> Result<(), CoreError> {
        self.request(HelperAction::Ping)?
            .ensure_success(helper_error)
    }

    pub(crate) fn status(&self) -> CoreStatus {
        match self.request(HelperAction::Status) {
            Ok(response) if response.error.is_none() => response
                .status
                .unwrap_or_else(|| unavailable_status("TUN helper 未返回状态")),
            _ => unavailable_status("TUN helper 已失去连接"),
        }
    }

    pub(crate) fn traffic(&self) -> Result<TrafficStats, CoreError> {
        let response = self.request(HelperAction::Traffic)?;
        response.ensure_success(CoreError::TrafficUnavailable)?;
        response
            .traffic
            .ok_or_else(|| CoreError::TrafficUnavailable("TUN helper 未返回流量数据".to_string()))
    }

    pub(crate) fn logs(&self, cursor: u64) -> Result<LogBatch, CoreError> {
        let response = self.request(HelperAction::Logs { cursor })?;
        response.ensure_success(CoreError::LogsUnavailable)?;
        response
            .logs
            .ok_or_else(|| CoreError::LogsUnavailable("TUN helper 未返回日志".to_string()))
    }

    pub(crate) fn set_log_enabled(&self, enabled: bool) -> Result<(), CoreError> {
        self.request(HelperAction::SetLogEnabled { enabled })?
            .ensure_success(CoreError::LogsUnavailable)
    }

    pub(crate) fn select_outbound(&self, group: &str, outbound: &str) -> Result<(), CoreError> {
        self.request(HelperAction::SelectOutbound {
            group: group.to_string(),
            outbound: outbound.to_string(),
        })?
        .ensure_success(CoreError::SelectionUnavailable)
    }

    pub(crate) fn probe_outbound(
        &self,
        tag: &str,
        probe_url: &str,
    ) -> Result<LatencyProbeResult, CoreError> {
        let response = self.request(HelperAction::ProbeOutbound {
            tag: tag.to_string(),
            probe_url: probe_url.to_string(),
        })?;
        response.ensure_success(CoreError::ProbeUnavailable)?;
        response
            .probe
            .ok_or_else(|| CoreError::ProbeUnavailable("TUN helper 未返回探测结果".to_string()))
    }

    fn wait_until_ready(&mut self, listener: &TcpListener) -> Result<(), CoreError> {
        let deadline = Instant::now() + HELPER_START_TIMEOUT;
        loop {
            if self.lease.is_none() {
                match listener.accept() {
                    Ok((mut lease, peer)) if peer.ip().is_loopback() => {
                        lease
                            .set_read_timeout(Some(HELPER_REQUEST_TIMEOUT))
                            .map_err(|error| {
                                helper_error(format!("配置 helper 租约失败: {error}"))
                            })?;
                        let mut presented = String::new();
                        BufReader::new(&mut lease)
                            .read_line(&mut presented)
                            .map_err(|error| {
                                helper_error(format!("读取 helper 租约失败: {error}"))
                            })?;
                        if presented.trim_end() == self.token {
                            self.lease = Some(lease);
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => {
                        return Err(helper_error(format!(
                            "接受 helper 生命周期租约失败: {error}"
                        )));
                    }
                }
            }
            if self.lease.is_some() && self.is_ready().is_ok() {
                return Ok(());
            }
            if let Some(child) = self.child.as_mut() {
                if let Some(status) = child
                    .try_wait()
                    .map_err(|error| helper_error(format!("读取 helper 状态失败: {error}")))?
                {
                    let mut detail = String::new();
                    if let Some(stderr) = child.stderr.as_mut() {
                        let _ = stderr.read_to_string(&mut detail);
                    }
                    return Err(helper_error(if detail.trim().is_empty() {
                        format!("授权被取消或 helper 启动失败 ({status})")
                    } else {
                        format!("helper 启动失败: {}", detail.trim())
                    }));
                }
            }
            if Instant::now() >= deadline {
                return Err(helper_error("等待管理员授权超时".to_string()));
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn request(&self, action: HelperAction) -> Result<HelperResponse, CoreError> {
        self.request_with_timeout(action, HELPER_REQUEST_TIMEOUT)
    }

    fn request_with_timeout(
        &self,
        action: HelperAction,
        timeout: Duration,
    ) -> Result<HelperResponse, CoreError> {
        if self.lease.is_none() {
            return Err(helper_error("TUN helper 尚未建立生命周期租约".to_string()));
        }
        let mut stream = TcpStream::connect(loopback_address(self.rpc_port))
            .map_err(|error| helper_error(format!("连接 TUN helper 失败: {error}")))?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|_| stream.set_write_timeout(Some(timeout)))
            .map_err(|error| helper_error(format!("配置 TUN helper 连接失败: {error}")))?;
        serde_json::to_writer(
            &mut stream,
            &HelperRequest {
                token: self.token.clone(),
                action,
            },
        )
        .map_err(|error| helper_error(format!("发送 TUN helper 请求失败: {error}")))?;
        stream
            .shutdown(Shutdown::Write)
            .map_err(|error| helper_error(format!("结束 TUN helper 请求失败: {error}")))?;
        let mut payload = Vec::new();
        (&mut stream)
            .take(MAX_MESSAGE_BYTES + 1)
            .read_to_end(&mut payload)
            .map_err(|error| helper_error(format!("读取 TUN helper 响应失败: {error}")))?;
        if payload.len() as u64 > MAX_MESSAGE_BYTES {
            return Err(helper_error("TUN helper 响应超过安全限制".to_string()));
        }
        serde_json::from_slice(&payload)
            .map_err(|error| helper_error(format!("解析 TUN helper 响应失败: {error}")))
    }

    pub(crate) fn shutdown(&mut self) {
        if self.lease.is_some() {
            let _ = self.request_with_timeout(HelperAction::Exit, HELPER_SHUTDOWN_TIMEOUT);
        }
        self.lease = None;
        let Some(mut child) = self.child.take() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for PrivilegedCore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct HelperRequest {
    token: String,
    #[serde(flatten)]
    action: HelperAction,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum HelperAction {
    Ping,
    Start { config: Value },
    Stop,
    Exit,
    IsRunning,
    Status,
    Traffic,
    Logs { cursor: u64 },
    SetLogEnabled { enabled: bool },
    SelectOutbound { group: String, outbound: String },
    ProbeOutbound { tag: String, probe_url: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct HelperResponse {
    error: Option<String>,
    running: Option<bool>,
    status: Option<CoreStatus>,
    traffic: Option<TrafficStats>,
    logs: Option<LogBatch>,
    probe: Option<LatencyProbeResult>,
}

impl HelperResponse {
    fn success() -> Self {
        Self {
            error: None,
            running: None,
            status: None,
            traffic: None,
            logs: None,
            probe: None,
        }
    }

    fn error(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::success()
        }
    }

    fn ensure_success(
        &self,
        create_error: impl FnOnce(String) -> CoreError,
    ) -> Result<(), CoreError> {
        match &self.error {
            Some(error) => Err(create_error(error.clone())),
            None => Ok(()),
        }
    }
}

pub fn run_helper_from_args() -> Option<i32> {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments.get(1).map(String::as_str) != Some(HELPER_FLAG) {
        return None;
    }
    let result = parse_helper_arguments(&arguments).and_then(run_helper);
    if let Err(error) = &result {
        eprintln!("{error}");
    }
    Some(if result.is_ok() { 0 } else { 1 })
}

#[derive(Debug)]
struct HelperArguments {
    rpc_port: u16,
    lease_port: u16,
    token: String,
}

fn parse_helper_arguments(arguments: &[String]) -> Result<HelperArguments, String> {
    if arguments.len() != 6 {
        return Err("invalid TUN helper arguments".to_string());
    }
    ensure_elevated()?;
    let rpc_port = arguments[2]
        .parse::<u16>()
        .map_err(|_| "invalid TUN helper RPC port".to_string())?;
    let lease_port = arguments[3]
        .parse::<u16>()
        .map_err(|_| "invalid TUN helper lease port".to_string())?;
    let owner_id = arguments[5]
        .parse::<u32>()
        .map_err(|_| "invalid TUN helper session owner".to_string())?;
    let token_path = std::path::Path::new(&arguments[4]);
    let token = read_session_token(token_path, owner_id)?;
    if rpc_port == 0 || lease_port == 0 || rpc_port == lease_port || token.len() != 64 {
        return Err("unsafe TUN helper session parameters".to_string());
    }
    Ok(HelperArguments {
        rpc_port,
        lease_port,
        token,
    })
}

fn run_helper(arguments: HelperArguments) -> Result<(), String> {
    let mut lease = TcpStream::connect(loopback_address(arguments.lease_port))
        .map_err(|error| format!("connect TUN helper lifecycle lease: {error}"))?;
    lease
        .write_all(format!("{}\n", arguments.token).as_bytes())
        .map_err(|error| format!("authenticate TUN helper lifecycle lease: {error}"))?;
    let watchdog = lease
        .try_clone()
        .map_err(|error| format!("clone TUN helper lifecycle lease: {error}"))?;
    let lease_alive = Arc::new(AtomicBool::new(true));
    let watchdog_lease_alive = Arc::clone(&lease_alive);
    thread::spawn(move || {
        let mut watchdog = watchdog;
        let mut byte = [0_u8; 1];
        loop {
            match watchdog.read(&mut byte) {
                Ok(0) | Err(_) => {
                    watchdog_lease_alive.store(false, Ordering::Release);
                    break;
                }
                Ok(_) => {}
            }
        }
    });

    let listener = TcpListener::bind(loopback_address(arguments.rpc_port))
        .map_err(|error| format!("bind TUN helper RPC listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure TUN helper RPC listener: {error}"))?;
    let mut core = EmbeddedCore::new();
    let serve_result = (|| -> Result<(), String> {
        while lease_alive.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if handle_helper_connection(&mut stream, &arguments.token, &mut core)? {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(format!("accept TUN helper request: {error}")),
            }
        }
        Ok(())
    })();
    let cleanup_result = if core.is_running() {
        core.stop().map_err(|error| error.to_string())
    } else {
        Ok(())
    };
    serve_result.and(cleanup_result)
}

fn handle_helper_connection(
    stream: &mut TcpStream,
    expected_token: &str,
    core: &mut EmbeddedCore,
) -> Result<bool, String> {
    stream
        .set_read_timeout(Some(HELPER_REQUEST_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(HELPER_REQUEST_TIMEOUT)))
        .map_err(|error| format!("configure TUN helper request: {error}"))?;
    let mut payload = Vec::new();
    (&mut *stream)
        .take(MAX_MESSAGE_BYTES + 1)
        .read_to_end(&mut payload)
        .map_err(|error| format!("read TUN helper request: {error}"))?;
    let (response, exit) = if payload.len() as u64 > MAX_MESSAGE_BYTES {
        (
            HelperResponse::error("TUN helper request is too large"),
            false,
        )
    } else {
        match serde_json::from_slice::<HelperRequest>(&payload) {
            Ok(request)
                if constant_time_eq(request.token.as_bytes(), expected_token.as_bytes()) =>
            {
                dispatch_helper_action(request.action, core)
            }
            Ok(_) => (HelperResponse::error("invalid TUN helper session"), false),
            Err(error) => (
                HelperResponse::error(format!("invalid TUN helper request: {error}")),
                false,
            ),
        }
    };
    serde_json::to_writer(stream, &response)
        .map_err(|error| format!("write TUN helper response: {error}"))?;
    Ok(exit)
}

fn dispatch_helper_action(action: HelperAction, core: &mut EmbeddedCore) -> (HelperResponse, bool) {
    let result = match action {
        HelperAction::Ping => Ok(HelperResponse::success()),
        HelperAction::Start { config } => core
            .start_config(&config)
            .map(|_| HelperResponse::success()),
        HelperAction::Stop => core.stop().map(|_| HelperResponse::success()),
        HelperAction::Exit => return (HelperResponse::success(), true),
        HelperAction::IsRunning => {
            let mut response = HelperResponse::success();
            response.running = Some(core.is_running());
            Ok(response)
        }
        HelperAction::Status => {
            let mut response = HelperResponse::success();
            response.status = Some(core.status());
            Ok(response)
        }
        HelperAction::Traffic => core.traffic().map(|traffic| {
            let mut response = HelperResponse::success();
            response.traffic = Some(traffic);
            response
        }),
        HelperAction::Logs { cursor } => core.logs(cursor).map(|logs| {
            let mut response = HelperResponse::success();
            response.logs = Some(logs);
            response
        }),
        HelperAction::SetLogEnabled { enabled } => core
            .set_log_enabled(enabled)
            .map(|_| HelperResponse::success()),
        HelperAction::SelectOutbound { group, outbound } => core
            .select_outbound(&group, &outbound)
            .map(|_| HelperResponse::success()),
        HelperAction::ProbeOutbound { tag, probe_url } => {
            core.probe_outbound(&tag, &probe_url).map(|probe| {
                let mut response = HelperResponse::success();
                response.probe = Some(probe);
                response
            })
        }
    };
    match result {
        Ok(response) => (response, false),
        Err(error) => (HelperResponse::error(error.to_string()), false),
    }
}

fn loopback_address(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

fn loopback_listener() -> std::io::Result<TcpListener> {
    TcpListener::bind(loopback_address(0))
}

fn listener_port(listener: &TcpListener) -> Result<u16, CoreError> {
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| helper_error(format!("读取 helper 本地端口失败: {error}")))
}

fn reserve_loopback_port() -> Result<u16, CoreError> {
    let listener = loopback_listener()
        .map_err(|error| helper_error(format!("分配 helper 本地端口失败: {error}")))?;
    listener_port(&listener)
}

fn random_token() -> Result<String, CoreError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| helper_error(format!("生成 helper 会话凭据失败: {error}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn write_session_token(token: &str) -> Result<std::path::PathBuf, CoreError> {
    use std::fs::OpenOptions;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let prefix = &token[..24];
    let path = std::env::temp_dir().join(format!(
        "kitty-pro-tun-session-{}-{prefix}.key",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(|error| helper_error(format!("创建 helper 会话文件失败: {error}")))?;
    if let Err(error) = file
        .write_all(token.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(helper_error(format!("写入 helper 会话文件失败: {error}")));
    }
    Ok(path)
}

#[cfg(unix)]
fn session_owner_id() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn session_owner_id() -> u32 {
    0
}

#[cfg(unix)]
fn read_session_token(path: &std::path::Path, owner_id: u32) -> Result<String, String> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let expected_parent = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| format!("resolve TUN helper session directory: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "TUN helper session path has no parent".to_string())?
        .canonicalize()
        .map_err(|error| format!("resolve TUN helper session parent: {error}"))?;
    if parent != expected_parent {
        return Err(
            "TUN helper session file is outside the private temporary directory".to_string(),
        );
    }

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| format!("open TUN helper session token safely: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect TUN helper session token: {error}"))?;
    if !metadata.file_type().is_file() || metadata.uid() != owner_id || metadata.mode() & 0o077 != 0
    {
        return Err("unsafe TUN helper session token file".to_string());
    }
    let mut token = String::new();
    file.read_to_string(&mut token)
        .map_err(|error| format!("read TUN helper session token: {error}"))?;
    Ok(token)
}

#[cfg(windows)]
fn read_session_token(path: &std::path::Path, _owner_id: u32) -> Result<String, String> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let expected_parent = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| format!("resolve TUN helper session directory: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "TUN helper session path has no parent".to_string())?
        .canonicalize()
        .map_err(|error| format!("resolve TUN helper session parent: {error}"))?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| format!("open TUN helper session token safely: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect TUN helper session token: {error}"))?;
    if parent != expected_parent
        || !metadata.file_type().is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err("unsafe TUN helper session token file".to_string());
    }
    let mut token = String::new();
    file.read_to_string(&mut token)
        .map_err(|error| format!("read TUN helper session token: {error}"))?;
    Ok(token)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(target_os = "linux")]
fn ensure_elevated() -> Result<(), String> {
    if unsafe { libc::geteuid() } == 0 {
        Ok(())
    } else {
        Err("TUN helper must run as root".to_string())
    }
}

#[cfg(target_os = "windows")]
fn ensure_elevated() -> Result<(), String> {
    if unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } != 0 {
        Ok(())
    } else {
        Err("TUN helper must run as administrator".to_string())
    }
}

#[cfg(all(test, not(any(target_os = "windows", target_os = "linux"))))]
fn ensure_elevated() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_elevated(
    executable: &std::path::Path,
    rpc_port: u16,
    lease_port: u16,
    token_path: &std::path::Path,
    owner_id: u32,
) -> Result<Child, CoreError> {
    let mut command = if unsafe { libc::geteuid() } == 0 {
        Command::new(executable)
    } else {
        let mut command = Command::new("pkexec");
        command.arg(executable);
        command
    };
    command
        .args([
            HELPER_FLAG,
            &rpc_port.to_string(),
            &lease_port.to_string(),
            &token_path.to_string_lossy(),
            &owner_id.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| helper_error(format!("无法请求 root 授权（需要 pkexec/polkit）: {error}")))
}

#[cfg(target_os = "windows")]
fn launch_elevated(
    executable: &std::path::Path,
    rpc_port: u16,
    lease_port: u16,
    token_path: &std::path::Path,
    owner_id: u32,
) -> Result<Child, CoreError> {
    let executable = executable.to_string_lossy().replace('\'', "''");
    let token_path = format!("\"{}\"", token_path.to_string_lossy()).replace('\'', "''");
    let script = format!(
        "$p=Start-Process -FilePath '{executable}' -ArgumentList @('{HELPER_FLAG}','{rpc_port}','{lease_port}','{token_path}','{owner_id}') -Verb RunAs -PassThru -Wait; exit $p.ExitCode"
    );
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| helper_error(format!("无法请求管理员授权: {error}")))
}

#[cfg(all(test, not(any(target_os = "windows", target_os = "linux"))))]
fn launch_elevated(
    executable: &std::path::Path,
    rpc_port: u16,
    lease_port: u16,
    token_path: &std::path::Path,
    owner_id: u32,
) -> Result<Child, CoreError> {
    Command::new(executable)
        .args([
            HELPER_FLAG,
            &rpc_port.to_string(),
            &lease_port.to_string(),
            &token_path.to_string_lossy(),
            &owner_id.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| helper_error(format!("无法启动测试 TUN helper: {error}")))
}

fn helper_error(message: String) -> CoreError {
    CoreError::DesktopTunHelper(message)
}

fn unavailable_status(message: &str) -> CoreStatus {
    CoreStatus {
        state: CoreState::Unavailable,
        version: None,
        platform_note: Some(message.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_token_check_requires_exact_match() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn helper_arguments_reject_missing_fields_before_privilege_check() {
        let arguments = vec!["kitty-pro".to_string(), HELPER_FLAG.to_string()];
        assert_eq!(
            parse_helper_arguments(&arguments).expect_err("arguments must be rejected"),
            "invalid TUN helper arguments"
        );
    }

    #[test]
    fn helper_reads_the_session_token_from_a_private_file() {
        let token = "ab".repeat(32);
        let path = write_session_token(&token).expect("session token should be written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mode = fs::metadata(&path)
                .expect("session token metadata should be readable")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        let arguments = vec![
            "kitty-pro".to_string(),
            HELPER_FLAG.to_string(),
            "41001".to_string(),
            "41002".to_string(),
            path.to_string_lossy().into_owned(),
            session_owner_id().to_string(),
        ];
        let parsed = parse_helper_arguments(&arguments).expect("session should be parsed");
        assert_eq!(parsed.token, token);
        fs::remove_file(path).expect("session token should be removed");
    }

    #[cfg(unix)]
    #[test]
    fn helper_rejects_a_symlink_session_token() {
        use std::os::unix::fs::symlink;

        let token = "cd".repeat(32);
        let target = write_session_token(&token).expect("session token should be written");
        let link = target.with_extension("link");
        symlink(&target, &link).expect("session token symlink should be created");

        let error = read_session_token(&link, session_owner_id())
            .expect_err("session token symlinks must be rejected");
        assert!(
            error.contains("safely") || error.contains("unsafe"),
            "{error}"
        );

        fs::remove_file(link).expect("session token symlink should be removed");
        fs::remove_file(target).expect("session token should be removed");
    }
}
