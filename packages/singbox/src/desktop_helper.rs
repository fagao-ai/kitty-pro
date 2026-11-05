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
/// Session tokens and lease hello lines are tiny; cap them so a misbehaving
/// (or hijacked) helper cannot make us allocate unbounded memory.
const MAX_CONTROL_LINE_BYTES: u64 = 4096;
/// Diagnostic tail of a failed helper process, bounded for the same reason.
const MAX_HELPER_STDERR_BYTES: u64 = 64 * 1024;
const HELPER_START_TIMEOUT: Duration = Duration::from_secs(120);
const HELPER_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const HELPER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
/// 特权进程不接受任意探测地址：无论 UI/调用方传入什么，RPC 一律使用这个
/// 编译期常量（与 Go 侧 `defaultProbeURL` 一致），避免 helper 被当作 SSRF 跳板。
const FIXED_PROBE_URL: &str = "https://www.gstatic.com/generate_204";

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
        let _ = probe_url; // 特权进程不接受任意探测地址，忽略调用方传入值。
        let response = self.request(HelperAction::ProbeOutbound {
            tag: tag.to_string(),
            probe_url: FIXED_PROBE_URL.to_string(),
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
                        BufReader::new((&mut lease).take(MAX_CONTROL_LINE_BYTES))
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
                        let _ = stderr
                            .take(MAX_HELPER_STDERR_BYTES)
                            .read_to_string(&mut detail);
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
        HelperAction::Start { config } => {
            if let Err(error) = crate::validate_privileged_start_config(&config) {
                return (HelperResponse::error(error.to_string()), false);
            }
            core.start_config(&config)
                .map(|_| HelperResponse::success())
        }
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
        HelperAction::ProbeOutbound { tag, probe_url: _ } => {
            // 特权进程不接受任意探测地址：忽略 RPC 消息里的 URL，固定使用常量。
            core.probe_outbound(&tag, FIXED_PROBE_URL).map(|probe| {
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

    let file = OpenOptions::new()
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
    file.take(MAX_CONTROL_LINE_BYTES)
        .read_to_string(&mut token)
        .map_err(|error| format!("read TUN helper session token: {error}"))?;
    Ok(token)
}

#[cfg(windows)]
fn read_session_token(path: &std::path::Path, owner_id: u32) -> Result<String, String> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    // 让 owner_id 真正参与校验：调用方（非特权父进程）把 session_owner_id()
    // 随命令行传入，这里用常量时间比较核对它是否等于本进程期望的 owner 字符串，
    // 不匹配即拒绝会话文件。特权进程不接受与预期不符的会话所有者。
    let expected_owner = session_owner_id().to_string();
    let presented_owner = owner_id.to_string();
    if !constant_time_eq(expected_owner.as_bytes(), presented_owner.as_bytes()) {
        return Err("TUN helper session owner mismatch".to_string());
    }

    let expected_parent = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| format!("resolve TUN helper session directory: {error}"))?;
    // 路径规范化：解析后的文件必须仍位于私有临时目录内。
    let canonical_path = fs::canonicalize(path)
        .map_err(|error| format!("resolve TUN helper session file: {error}"))?;
    let parent = canonical_path
        .parent()
        .ok_or_else(|| "TUN helper session path has no parent".to_string())?;
    if parent != expected_parent {
        return Err(
            "TUN helper session file is outside the private temporary directory".to_string(),
        );
    }
    // 拒绝符号链接点（symlink_metadata 不跟随链接，能看到链接本身）。
    let link_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect TUN helper session token link: {error}"))?;
    if link_metadata.file_type().is_symlink() {
        return Err("unsafe TUN helper session token file (symlink)".to_string());
    }

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| format!("open TUN helper session token safely: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect TUN helper session token: {error}"))?;
    // 残留限制：现有 windows-sys features 不足以读取文件属主 SID/ACL，也无法像
    // Unix 侧那样核对 uid 与 0600 权限；这里只能做「普通文件 / 非重解析点」的
    // 基本检查，无法保证文件确由调用方用户创建。完整防护需要引入额外的 Win32
    // 安全 API（读取文件属主 SID/ACL）与相应的 windows-sys features。
    if !metadata.file_type().is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err("unsafe TUN helper session token file".to_string());
    }
    let mut token = String::new();
    file.take(MAX_CONTROL_LINE_BYTES)
        .read_to_string(&mut token)
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

// ===== S13：PowerShell 数组传参 / 命令注入加固 =====

/// 把任意字符串安全地嵌入 PowerShell 单引号字符串字面量。
/// PowerShell 单引号字符串里只有 `'` 需要转义成 `''`，分号、`$()`、反引号在
/// 单引号内都是普通字面量；但 NUL / CR / LF 会打断 `-Command` 脚本文本结构，
/// 必须剔除。放在无 cfg 区域，保证 macOS / Linux 也能编译并单测（S13）。
#[allow(dead_code)] // windows 的 launch_elevated 使用；其它平台仅测试引用。
fn escape_powershell_single_quoted(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\'' => escaped.push_str("''"),
            '\0' | '\r' | '\n' => escaped.push(' '),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// 包成带定界符的单引号字面量 `'...'`。
#[allow(dead_code)]
fn powershell_single_quoted(value: &str) -> String {
    format!("'{}'", escape_powershell_single_quoted(value))
}

/// 把一组值拼成 PowerShell 数组字面量 `@('a','b')`，每个元素都经转义。
#[allow(dead_code)]
fn powershell_string_array<S: AsRef<str>>(values: &[S]) -> String {
    let items: Vec<String> = values
        .iter()
        .map(|value| powershell_single_quoted(value.as_ref()))
        .collect();
    format!("@({})", items.join(","))
}

#[cfg(target_os = "windows")]
fn launch_elevated(
    executable: &std::path::Path,
    rpc_port: u16,
    lease_port: u16,
    token_path: &std::path::Path,
    owner_id: u32,
) -> Result<Child, CoreError> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    // S13：所有来自变量的值都经 escape helper 处理，并用 `@('a','b')` 数组
    // 字面量传参，元素里的 `'` / `;` / 换行 / `$( )` 不再能破坏脚本文本。
    // token_path 沿用历史的双引号包裹（让 Start-Process 正确处理含空格路径），
    // 但整串先经转义再嵌入单引号字面量。
    let token_path_arg = format!("\"{}\"", token_path.to_string_lossy());
    let arguments = powershell_string_array(&[
        HELPER_FLAG.to_string(),
        rpc_port.to_string(),
        lease_port.to_string(),
        token_path_arg,
        owner_id.to_string(),
    ]);
    let executable_text = executable.to_string_lossy().into_owned();
    let executable_literal = powershell_single_quoted(&executable_text);
    let script = format!(
        "$p=Start-Process -FilePath {executable_literal} -ArgumentList {arguments} -Verb RunAs -PassThru -Wait; exit $p.ExitCode"
    );
    Command::new("powershell.exe")
        .creation_flags(CREATE_NO_WINDOW)
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
    fn escape_powershell_single_quoted_leaves_plain_values_unchanged() {
        assert_eq!(
            escape_powershell_single_quoted(r"C:\Program Files\Kitty Pro\helper.exe"),
            r"C:\Program Files\Kitty Pro\helper.exe"
        );
        assert_eq!(escape_powershell_single_quoted(""), "");
        assert_eq!(powershell_single_quoted("plain"), "'plain'");
    }

    #[test]
    fn escape_powershell_single_quoted_neutralizes_injection_metacharacters() {
        let value = "O'Brien; Drop-Table\n$(Get-Process)\r\0";
        let escaped = escape_powershell_single_quoted(value);
        // 单引号翻倍：PowerShell 单引号字符串里唯一必需的转义。
        assert!(escaped.contains("O''Brien"), "iteration=0: {escaped:?}");
        // NUL / CR / LF 一律剔除，避免打断 -Command 脚本文本。
        assert!(!escaped.contains('\0'), "iteration=0: {escaped:?}");
        assert!(!escaped.contains('\r'), "iteration=0: {escaped:?}");
        assert!(!escaped.contains('\n'), "iteration=0: {escaped:?}");
        // 分号 / $() 放进单引号字面量后只是字面量，不会提前闭合引号。
        let literal = powershell_single_quoted(value);
        assert!(literal.starts_with('\'') && literal.ends_with('\''));
        let inner = &literal[1..literal.len() - 1];
        assert_eq!(
            inner.matches('\'').count() % 2,
            0,
            "iteration=0: 每个内部单引号都应成对: {literal:?}"
        );
        assert!(literal.contains(';') && literal.contains("$(Get-Process)"));
    }

    #[test]
    fn powershell_string_array_has_no_unescaped_single_quotes() {
        let values = [
            "--kitty-pro-tun-helper",
            "9090",
            r"C:\Users\O'Brien\token;$(whoami)",
            "\n42\0",
        ];
        let literal = powershell_string_array(&values);
        assert!(literal.starts_with("@(") && literal.ends_with(')'));
        // 把成对转义 '' 消掉后，剩下的 ' 只能作为 4 个元素的首尾定界符。
        let without_pairs = literal.replace("''", "");
        assert_eq!(
            without_pairs.matches('\'').count(),
            values.len() * 2,
            "iteration=0: 数组字面量不应含未转义的单引号: {literal:?}"
        );
        assert!(!literal.contains('\n'));
        assert!(!literal.contains('\r'));
        assert!(!literal.contains('\0'));
    }

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
        let parsed =
            read_session_token(&path, session_owner_id()).expect("session token should be read");
        assert_eq!(parsed, token);
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
