use crate::{
    CoreError, CoreState, CoreStatus, EmbeddedCore, LatencyProbeResult, LogBatch, TrafficStats,
};
use core_foundation::{
    array::CFArray, base::TCFType, dictionary::CFDictionary, number::CFNumber, string::CFString,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::CString;
use std::fs;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use system_configuration::{
    dynamic_store::{SCDynamicStore, SCDynamicStoreBuilder},
    sys::schema_definitions::{
        kSCPropNetDNSServerAddresses, kSCPropNetDNSSupplementalMatchDomains,
        kSCPropNetDNSSupplementalMatchOrders,
    },
};

const HELPER_FLAG: &str = "--kitty-pro-tun-helper";
const SOCKET_PREFIX: &str = "kitty-pro-tun-";
const MAX_MESSAGE_BYTES: u64 = 16 * 1024 * 1024;
const HELPER_START_TIMEOUT: Duration = Duration::from_secs(120);
const HELPER_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const HELPER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const DNS_STORE_NAME: &str = "com.kitty.pro.tun-dns";
const DNS_KEY_PREFIX: &str = "State:/Network/Service/";
const DNS_KEY_SUFFIX: &str = "/DNS";
const DNS_MATCH_ORDER: i32 = 1;
const DNS_NO_SEARCH_KEY: &str = "SupplementalMatchDomainsNoSearch";

#[derive(Debug)]
pub(crate) struct PrivilegedCore {
    socket_path: PathBuf,
    lease_path: PathBuf,
    lease: Option<UnixStream>,
    helper_pid: Option<u32>,
    child: Option<Child>,
}

impl PrivilegedCore {
    pub(crate) fn launch() -> Result<Self, CoreError> {
        let executable = std::env::current_exe()
            .map_err(|error| CoreError::MacosTunHelper(format!("无法定位可执行文件: {error}")))?;
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let parent_pid = std::process::id();
        let nonce = random_nonce()?;
        let socket_path = PathBuf::from(format!(
            "/private/var/tmp/{SOCKET_PREFIX}{uid}-{}.sock",
            &nonce[..24]
        ));
        let lease_path = PathBuf::from(format!(
            "/private/var/tmp/{SOCKET_PREFIX}{uid}-{}.lease.sock",
            &nonce[..24]
        ));
        let lease_listener = UnixListener::bind(&lease_path).map_err(|error| {
            CoreError::MacosTunHelper(format!("创建 TUN helper 生命周期租约失败: {error}"))
        })?;
        let lease_cleanup = SocketCleanup(lease_path.clone());
        fs::set_permissions(&lease_path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            CoreError::MacosTunHelper(format!("保护 TUN helper 生命周期租约失败: {error}"))
        })?;
        lease_listener.set_nonblocking(true).map_err(|error| {
            CoreError::MacosTunHelper(format!("配置 TUN helper 生命周期租约失败: {error}"))
        })?;
        let command = format!(
            "exec {} {} {} {} {} {} {}",
            shell_quote(&executable.to_string_lossy()),
            HELPER_FLAG,
            shell_quote(&socket_path.to_string_lossy()),
            shell_quote(&lease_path.to_string_lossy()),
            uid,
            gid,
            parent_pid,
        );
        let script = format!(
            "do shell script \"{}\" with administrator privileges",
            apple_script_escape(&command)
        );
        let child = Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| CoreError::MacosTunHelper(format!("无法请求管理员授权: {error}")))?;
        let mut helper = Self {
            socket_path,
            lease_path,
            lease: None,
            helper_pid: None,
            child: Some(child),
        };
        if let Err(error) = helper.wait_until_ready(&lease_listener) {
            helper.shutdown();
            return Err(error);
        }
        drop(lease_cleanup);
        Ok(helper)
    }

    pub(crate) fn start(&mut self, config: &Value) -> Result<(), CoreError> {
        let response = self.request(HelperAction::Start {
            config: config.clone(),
        })?;
        response.ensure_success(CoreError::MacosTunHelper)?;
        Ok(())
    }

    pub(crate) fn stop(&mut self) -> Result<(), CoreError> {
        self.request(HelperAction::Stop)?
            .ensure_success(CoreError::MacosTunHelper)?;
        Ok(())
    }

    pub(crate) fn is_running(&self) -> Result<bool, CoreError> {
        let response = self.request(HelperAction::IsRunning)?;
        response.ensure_success(CoreError::MacosTunHelper)?;
        let running = response
            .running
            .ok_or_else(|| CoreError::MacosTunHelper("helper 未返回运行状态".to_string()))?;
        Ok(running)
    }

    pub(crate) fn is_ready(&self) -> Result<(), CoreError> {
        self.request(HelperAction::Ping)?
            .ensure_success(CoreError::MacosTunHelper)
    }

    pub(crate) fn status(&self) -> CoreStatus {
        match self.request(HelperAction::Status) {
            Ok(response) if response.error.is_none() => {
                response.status.unwrap_or_else(|| CoreStatus {
                    state: CoreState::Unavailable,
                    version: None,
                    platform_note: Some("macOS TUN helper 未返回状态".to_string()),
                })
            }
            _ => CoreStatus {
                state: CoreState::Unavailable,
                version: None,
                platform_note: Some("macOS TUN helper 已失去连接".to_string()),
            },
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

    fn wait_until_ready(&mut self, lease_listener: &UnixListener) -> Result<(), CoreError> {
        let deadline = Instant::now() + HELPER_START_TIMEOUT;
        loop {
            if self.lease.is_none() {
                match lease_listener.accept() {
                    Ok((lease, _)) => match peer_identity(&lease) {
                        Ok(identity) if identity.uid == 0 && identity.pid > 1 => {
                            self.helper_pid = Some(identity.pid);
                            self.lease = Some(lease);
                        }
                        Ok(_) | Err(_) => continue,
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => {
                        return Err(CoreError::MacosTunHelper(format!(
                            "接受 TUN helper 生命周期租约失败: {error}"
                        )));
                    }
                }
            }
            if self.lease.is_some() && self.is_ready().is_ok() {
                return Ok(());
            }
            if let Some(child) = self.child.as_mut() {
                if let Some(status) = child.try_wait().map_err(|error| {
                    CoreError::MacosTunHelper(format!("读取 helper 状态失败: {error}"))
                })? {
                    let mut detail = String::new();
                    if let Some(stderr) = child.stderr.as_mut() {
                        let _ = stderr.read_to_string(&mut detail);
                    }
                    let detail = detail.trim();
                    return Err(CoreError::MacosTunHelper(if detail.is_empty() {
                        format!("授权被取消或 helper 启动失败 ({status})")
                    } else {
                        format!("helper 启动失败: {detail}")
                    }));
                }
            }
            if Instant::now() >= deadline {
                return Err(CoreError::MacosTunHelper("等待管理员授权超时".to_string()));
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
        let helper_pid = self.helper_pid.ok_or_else(|| {
            CoreError::MacosTunHelper("TUN helper 尚未建立可信生命周期租约".to_string())
        })?;
        let mut stream = UnixStream::connect(&self.socket_path).map_err(helper_connection_error)?;
        validate_peer_identity(&stream, 0, helper_pid).map_err(CoreError::MacosTunHelper)?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|_| stream.set_write_timeout(Some(timeout)))
            .map_err(helper_connection_error)?;
        let request = HelperRequest { action };
        serde_json::to_writer(&mut stream, &request)
            .map_err(|error| CoreError::MacosTunHelper(format!("发送请求失败: {error}")))?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .map_err(helper_connection_error)?;
        let mut payload = Vec::new();
        stream
            .take(MAX_MESSAGE_BYTES + 1)
            .read_to_end(&mut payload)
            .map_err(helper_connection_error)?;
        if payload.len() as u64 > MAX_MESSAGE_BYTES {
            return Err(CoreError::MacosTunHelper("响应超过安全限制".to_string()));
        }
        serde_json::from_slice(&payload)
            .map_err(|error| CoreError::MacosTunHelper(format!("解析响应失败: {error}")))
    }

    pub(crate) fn shutdown(&mut self) {
        let Some(mut child) = self.child.take() else {
            self.lease = None;
            remove_socket_if_present(&self.socket_path);
            remove_socket_if_present(&self.lease_path);
            return;
        };
        if self.helper_pid.is_some() {
            let _ = self.request_with_timeout(HelperAction::Exit, HELPER_SHUTDOWN_TIMEOUT);
        }
        // Closing the reverse lease makes the root watchdog terminate the
        // helper even if its main thread is stuck inside sing-box shutdown.
        self.lease = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                remove_socket_if_present(&self.socket_path);
                remove_socket_if_present(&self.lease_path);
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        remove_socket_if_present(&self.socket_path);
        remove_socket_if_present(&self.lease_path);
    }
}

impl Drop for PrivilegedCore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct HelperRequest {
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct HelperResponse {
    error: Option<String>,
    running: Option<bool>,
    status: Option<CoreStatus>,
    traffic: Option<TrafficStats>,
    logs: Option<LogBatch>,
    probe: Option<LatencyProbeResult>,
}

impl HelperResponse {
    fn ok() -> Self {
        Self::default()
    }

    fn error(error: impl ToString) -> Self {
        Self {
            error: Some(error.to_string()),
            ..Self::default()
        }
    }

    fn ensure_success(&self, map_error: impl FnOnce(String) -> CoreError) -> Result<(), CoreError> {
        match self.error.as_deref() {
            Some(error) => Err(map_error(error.to_string())),
            None => Ok(()),
        }
    }
}

#[derive(Debug)]
struct HelperState {
    core: EmbeddedCore,
    dns_resolver: Option<SessionOwnedDnsResolver>,
    owner_uid: u32,
    owner_gid: u32,
}

impl HelperState {
    fn new(owner_uid: u32, owner_gid: u32) -> Self {
        Self {
            core: EmbeddedCore::new(),
            dns_resolver: None,
            owner_uid,
            owner_gid,
        }
    }

    fn start(&mut self, config: &Value) -> Result<(), CoreError> {
        if self.core.is_running() {
            return Err(CoreError::AlreadyRunning);
        }
        crate::check_config(config)?;
        if !crate::config_uses_tun(config) {
            return Err(CoreError::InvalidConfig(
                "macOS TUN helper 只接受包含 TUN 入站的配置".to_string(),
            ));
        }
        validate_privileged_cache_file(config, self.owner_uid, self.owner_gid)?;
        // Publish the TUN-local resolver target only after its route exists.
        self.core.start_config(config)?;
        match SessionOwnedDnsResolver::install(config) {
            Ok(resolver) => {
                self.dns_resolver = Some(resolver);
                Ok(())
            }
            Err(install_error) => {
                let rollback = self.core.stop();
                let message = match rollback {
                    Ok(()) | Err(CoreError::NotRunning) => install_error,
                    Err(stop_error) => {
                        format!("{install_error}; 回滚停止 TUN 失败: {stop_error}")
                    }
                };
                Err(CoreError::MacosTunHelper(message))
            }
        }
    }

    fn stop(&mut self) -> Result<(), CoreError> {
        // Reverse the startup order: stop publishing resolver targets before
        // removing the routes that make those targets safe to query.
        self.dns_resolver = None;
        self.core.stop()
    }
}

fn validate_privileged_cache_file(
    config: &Value,
    expected_uid: u32,
    expected_gid: u32,
) -> Result<(), CoreError> {
    let Some(cache) = config
        .get("experimental")
        .and_then(|experimental| experimental.get("cache_file"))
    else {
        return Ok(());
    };
    if cache.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Ok(());
    }
    let path = cache
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            CoreError::MacosTunHelper("FakeIP 缓存必须使用预创建的绝对路径".to_string())
        })?;
    if !path.is_absolute() {
        return Err(CoreError::MacosTunHelper(
            "FakeIP 缓存路径必须是绝对路径".to_string(),
        ));
    }
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| CoreError::MacosTunHelper(format!("读取 FakeIP 缓存文件失败: {error}")))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(CoreError::MacosTunHelper(
            "FakeIP 缓存路径必须是预创建的普通文件".to_string(),
        ));
    }
    if metadata.uid() != expected_uid || metadata.gid() != expected_gid {
        return Err(CoreError::MacosTunHelper(
            "FakeIP 缓存文件所有者与授权用户不一致".to_string(),
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        return Err(CoreError::MacosTunHelper(
            "FakeIP 缓存文件权限必须为 0600".to_string(),
        ));
    }
    let canonical = fs::canonicalize(&path)
        .map_err(|error| CoreError::MacosTunHelper(format!("解析 FakeIP 缓存路径失败: {error}")))?;
    if canonical != path {
        return Err(CoreError::MacosTunHelper(
            "FakeIP 缓存路径不能包含符号链接".to_string(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::MacosTunHelper("FakeIP 缓存目录无效".to_string()))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| CoreError::MacosTunHelper(format!("读取 FakeIP 缓存目录失败: {error}")))?;
    if !parent_metadata.file_type().is_dir()
        || parent_metadata.uid() != expected_uid
        || parent_metadata.gid() != expected_gid
        || parent_metadata.mode() & 0o077 != 0
    {
        return Err(CoreError::MacosTunHelper(
            "FakeIP 缓存目录必须由授权用户持有且权限为 0700".to_string(),
        ));
    }
    Ok(())
}

struct SessionOwnedDnsResolver {
    _store: SCDynamicStore,
    key: CFString,
    server_address: Ipv4Addr,
}

impl std::fmt::Debug for SessionOwnedDnsResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionOwnedDnsResolver")
            .field("key", &self.key.to_string())
            .field("server_address", &self.server_address)
            .finish_non_exhaustive()
    }
}

impl SessionOwnedDnsResolver {
    fn install(config: &Value) -> Result<Self, String> {
        let server_address = tun_dns_server(config)?;
        let store = SCDynamicStoreBuilder::new(DNS_STORE_NAME)
            .session_keys(true)
            .build()
            .ok_or_else(|| "创建 macOS DNS session 失败".to_string())?;
        let service_id = random_nonce().map_err(|error| error.to_string())?;
        let key = dns_key(&service_id);
        if !store.set(key.clone(), default_dns_dictionary(server_address)) {
            return Err(format!("写入 macOS session DNS 失败: {key}"));
        }
        Ok(Self {
            _store: store,
            key,
            server_address,
        })
    }
}

fn dns_key(service_id: &str) -> CFString {
    CFString::new(&format!(
        "{DNS_KEY_PREFIX}{DNS_STORE_NAME}-{service_id}{DNS_KEY_SUFFIX}"
    ))
}

fn default_dns_dictionary(server_address: Ipv4Addr) -> CFDictionary {
    let server_key = unsafe { CFString::wrap_under_get_rule(kSCPropNetDNSServerAddresses) };
    let match_domains_key =
        unsafe { CFString::wrap_under_get_rule(kSCPropNetDNSSupplementalMatchDomains) };
    let match_orders_key =
        unsafe { CFString::wrap_under_get_rule(kSCPropNetDNSSupplementalMatchOrders) };
    let no_search_key = CFString::from_static_string(DNS_NO_SEARCH_KEY);
    let addresses = CFArray::from_CFTypes(&[CFString::new(&server_address.to_string())]);
    let match_domains = CFArray::from_CFTypes(&[CFString::from_static_string("")]);
    let match_orders = CFArray::from_CFTypes(&[CFNumber::from(DNS_MATCH_ORDER)]);
    let no_search = CFNumber::from(1);
    CFDictionary::from_CFType_pairs(&[
        (server_key, addresses.as_CFType()),
        (match_domains_key, match_domains.as_CFType()),
        (match_orders_key, match_orders.as_CFType()),
        (no_search_key, no_search.as_CFType()),
    ])
    .into_untyped()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TunNetwork {
    address: IpAddr,
    prefix_len: u8,
}

fn configured_tun_networks(config: &Value) -> Result<Vec<TunNetwork>, String> {
    let inbound = config
        .get("inbounds")
        .and_then(Value::as_array)
        .and_then(|inbounds| {
            inbounds
                .iter()
                .find(|inbound| inbound.get("type").and_then(Value::as_str) == Some("tun"))
        })
        .ok_or_else(|| "macOS TUN helper 配置缺少 TUN 入站".to_string())?;
    let addresses = inbound
        .get("address")
        .and_then(Value::as_array)
        .ok_or_else(|| "macOS TUN 入站缺少 address 数组".to_string())?;
    let mut networks = Vec::with_capacity(addresses.len());
    for value in addresses {
        let value = value
            .as_str()
            .ok_or_else(|| "macOS TUN address 必须是 CIDR 字符串".to_string())?;
        let (address, prefix_len) = value
            .split_once('/')
            .ok_or_else(|| format!("macOS TUN address 不是 CIDR: {value}"))?;
        let address = address
            .parse::<IpAddr>()
            .map_err(|error| format!("macOS TUN address 无效 ({value}): {error}"))?;
        let prefix_len = prefix_len
            .parse::<u8>()
            .map_err(|_| format!("macOS TUN CIDR 前缀无效: {value}"))?;
        let max_prefix = if address.is_ipv4() { 32 } else { 128 };
        if prefix_len > max_prefix {
            return Err(format!("macOS TUN CIDR 前缀超出范围: {value}"));
        }
        networks.push(TunNetwork {
            address,
            prefix_len,
        });
    }
    if networks.is_empty() {
        return Err("macOS TUN address 数组不能为空".to_string());
    }
    Ok(networks)
}

fn tun_dns_server(config: &Value) -> Result<Ipv4Addr, String> {
    let network = configured_tun_networks(config)?
        .into_iter()
        .find(|network| network.address.is_ipv4())
        .ok_or_else(|| "macOS TUN 至少需要一个 IPv4 CIDR 才能发布系统 DNS".to_string())?;
    ipv4_tun_peer(network)
        .ok_or_else(|| "macOS TUN 的首个 IPv4 CIDR 没有可用的下一地址".to_string())
}

fn ipv4_tun_peer(network: TunNetwork) -> Option<Ipv4Addr> {
    let IpAddr::V4(address) = network.address else {
        return None;
    };
    if network.prefix_len >= 32 {
        return None;
    }

    let address = u32::from(address);
    let next = address.checked_add(1)?;
    let mask = if network.prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - network.prefix_len)
    };
    ((address & mask) == (next & mask)).then(|| Ipv4Addr::from(next))
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

struct HelperArguments {
    socket_path: PathBuf,
    lease_path: PathBuf,
    uid: u32,
    gid: u32,
    parent_pid: u32,
}

fn parse_helper_arguments(arguments: &[String]) -> Result<HelperArguments, String> {
    if arguments.len() != 7 {
        return Err("invalid TUN helper arguments".to_string());
    }
    if unsafe { libc::geteuid() } != 0 {
        return Err("TUN helper must run with administrator privileges".to_string());
    }
    let socket_path = PathBuf::from(&arguments[2]);
    validate_socket_path(&socket_path)?;
    let lease_path = PathBuf::from(&arguments[3]);
    validate_socket_path(&lease_path)?;
    if socket_path == lease_path {
        return Err("TUN helper socket and lifecycle lease must differ".to_string());
    }
    let uid = arguments[4]
        .parse::<u32>()
        .map_err(|_| "invalid TUN helper user id".to_string())?;
    let gid = arguments[5]
        .parse::<u32>()
        .map_err(|_| "invalid TUN helper group id".to_string())?;
    let parent_pid = arguments[6]
        .parse::<u32>()
        .map_err(|_| "invalid TUN helper parent pid".to_string())?;
    if uid == 0 || parent_pid <= 1 {
        return Err("unsafe TUN helper owner or parent".to_string());
    }
    Ok(HelperArguments {
        socket_path,
        lease_path,
        uid,
        gid,
        parent_pid,
    })
}

fn run_helper(arguments: HelperArguments) -> Result<(), String> {
    let lease = UnixStream::connect(&arguments.lease_path)
        .map_err(|error| format!("connect TUN helper lifecycle lease: {error}"))?;
    validate_peer_identity(&lease, arguments.uid, arguments.parent_pid)?;

    if arguments.socket_path.exists() {
        let metadata = fs::symlink_metadata(&arguments.socket_path)
            .map_err(|error| format!("inspect stale TUN helper socket: {error}"))?;
        if !metadata.file_type().is_socket() || metadata.uid() != arguments.uid {
            return Err("refusing to replace unsafe TUN helper socket path".to_string());
        }
        fs::remove_file(&arguments.socket_path)
            .map_err(|error| format!("remove stale TUN helper socket: {error}"))?;
    }
    let listener = UnixListener::bind(&arguments.socket_path)
        .map_err(|error| format!("bind TUN helper socket: {error}"))?;
    let _cleanup = SocketCleanup(arguments.socket_path.clone());
    fs::set_permissions(&arguments.socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("protect TUN helper socket: {error}"))?;
    let socket = CString::new(arguments.socket_path.as_os_str().as_encoded_bytes())
        .map_err(|_| "TUN helper socket path contains NUL".to_string())?;
    if unsafe { libc::chown(socket.as_ptr(), arguments.uid, arguments.gid) } != 0 {
        return Err(format!(
            "assign TUN helper socket owner: {}",
            std::io::Error::last_os_error()
        ));
    }
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure TUN helper socket: {error}"))?;
    spawn_lease_watchdog(
        lease,
        arguments.socket_path.clone(),
        arguments.lease_path.clone(),
    )?;

    // The helper is already the privileged process. Calling the public
    // `SingBox` facade here would detect the TUN inbound and recurse into a
    // second authorization request, so it owns the embedded handle directly.
    let mut state = HelperState::new(arguments.uid, arguments.gid);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Ok(true) = handle_helper_connection(
                    stream,
                    arguments.uid,
                    arguments.parent_pid,
                    &mut state,
                ) {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(format!("accept TUN helper request: {error}")),
        }
    }
    if state.core.is_running() {
        state.stop().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn handle_helper_connection(
    mut stream: UnixStream,
    expected_uid: u32,
    expected_pid: u32,
    state: &mut HelperState,
) -> Result<bool, String> {
    validate_peer_identity(&stream, expected_uid, expected_pid)?;
    stream
        .set_read_timeout(Some(HELPER_REQUEST_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(HELPER_REQUEST_TIMEOUT)))
        .map_err(|error| format!("configure TUN helper request: {error}"))?;
    let mut payload = Vec::new();
    (&mut stream)
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
            Ok(request) => dispatch_helper_action(request.action, state),
            Err(error) => (
                HelperResponse::error(format!("invalid TUN helper request: {error}")),
                false,
            ),
        }
    };
    serde_json::to_writer(&mut stream, &response)
        .map_err(|error| format!("write TUN helper response: {error}"))?;
    Ok(exit)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeerIdentity {
    uid: u32,
    pid: u32,
}

fn peer_identity(stream: &UnixStream) -> Result<PeerIdentity, String> {
    let mut uid = 0;
    let mut gid = 0;
    let mut pid: libc::pid_t = 0;
    let mut pid_size = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut pid_size,
        )
    } != 0
    {
        return Err(format!(
            "inspect TUN helper peer process: {}",
            std::io::Error::last_os_error()
        ));
    }
    if pid_size as usize != std::mem::size_of::<libc::pid_t>() || pid <= 1 {
        return Err("TUN helper peer returned an invalid process id".to_string());
    }
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(format!(
            "inspect TUN helper peer: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(PeerIdentity {
        uid,
        pid: pid as u32,
    })
}

fn validate_peer_identity(
    stream: &UnixStream,
    expected_uid: u32,
    expected_pid: u32,
) -> Result<(), String> {
    let identity = peer_identity(stream)?;
    if identity.uid != expected_uid || identity.pid != expected_pid {
        return Err("TUN helper peer does not match the authorized process".to_string());
    }
    Ok(())
}

fn dispatch_helper_action(action: HelperAction, state: &mut HelperState) -> (HelperResponse, bool) {
    let response = match action {
        HelperAction::Ping => HelperResponse::ok(),
        HelperAction::Start { config } => match state.start(&config) {
            Ok(()) => HelperResponse::ok(),
            Err(error) => HelperResponse::error(error),
        },
        HelperAction::Stop => match state.stop() {
            Ok(()) => HelperResponse::ok(),
            Err(CoreError::NotRunning) => HelperResponse::ok(),
            Err(error) => HelperResponse::error(error),
        },
        HelperAction::Exit => {
            let response = if state.core.is_running() {
                state
                    .stop()
                    .map(|_| HelperResponse::ok())
                    .unwrap_or_else(HelperResponse::error)
            } else {
                state.dns_resolver = None;
                HelperResponse::ok()
            };
            return (response, true);
        }
        HelperAction::IsRunning => HelperResponse {
            running: Some(state.core.is_running()),
            ..HelperResponse::ok()
        },
        HelperAction::Status => {
            let mut status = state.core.status();
            status.platform_note = Some("macOS TUN 由已授权 helper 运行".to_string());
            HelperResponse {
                status: Some(status),
                ..HelperResponse::ok()
            }
        }
        HelperAction::Traffic => match state.core.traffic() {
            Ok(traffic) => HelperResponse {
                traffic: Some(traffic),
                ..HelperResponse::ok()
            },
            Err(error) => HelperResponse::error(error),
        },
        HelperAction::Logs { cursor } => match state.core.logs(cursor) {
            Ok(logs) => HelperResponse {
                logs: Some(logs),
                ..HelperResponse::ok()
            },
            Err(error) => HelperResponse::error(error),
        },
        HelperAction::SetLogEnabled { enabled } => state
            .core
            .set_log_enabled(enabled)
            .map(|_| HelperResponse::ok())
            .unwrap_or_else(HelperResponse::error),
        HelperAction::SelectOutbound { group, outbound } => state
            .core
            .select_outbound(&group, &outbound)
            .map(|_| HelperResponse::ok())
            .unwrap_or_else(HelperResponse::error),
        HelperAction::ProbeOutbound { tag, probe_url } => {
            match state.core.probe_outbound(&tag, &probe_url) {
                Ok(probe) => HelperResponse {
                    probe: Some(probe),
                    ..HelperResponse::ok()
                },
                Err(error) => HelperResponse::error(error),
            }
        }
    };
    (response, false)
}

struct SocketCleanup(PathBuf);

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        remove_socket_if_present(&self.0);
    }
}

fn validate_socket_path(path: &Path) -> Result<(), String> {
    if path.parent() != Some(Path::new("/private/var/tmp"))
        || !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(SOCKET_PREFIX) && name.ends_with(".sock"))
    {
        return Err("invalid TUN helper socket path".to_string());
    }
    Ok(())
}

fn spawn_lease_watchdog(
    lease: UnixStream,
    socket_path: PathBuf,
    lease_path: PathBuf,
) -> Result<(), String> {
    thread::Builder::new()
        .name("kitty-tun-lease".to_string())
        .spawn(move || {
            wait_for_lease_disconnect(lease);
            remove_socket_if_present(&socket_path);
            remove_socket_if_present(&lease_path);
            // The main helper thread may be blocked in a privileged sing-box
            // operation. Process exit closes the utun descriptor and routes.
            unsafe { libc::_exit(0) }
        })
        .map(|_| ())
        .map_err(|error| format!("start TUN helper lifecycle watchdog: {error}"))
}

fn wait_for_lease_disconnect(mut lease: impl Read) {
    let mut buffer = [0u8; 1];
    loop {
        match lease.read(&mut buffer) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

fn remove_socket_if_present(path: &Path) {
    if fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
    {
        let _ = fs::remove_file(path);
    }
}

fn helper_connection_error(error: std::io::Error) -> CoreError {
    CoreError::MacosTunHelper(format!("连接失败: {error}"))
}

fn random_nonce() -> Result<String, CoreError> {
    use std::fmt::Write as _;

    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| CoreError::MacosTunHelper(format!("生成随机标识失败: {error}")))?;
    let mut nonce = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(nonce, "{byte:02x}");
    }
    Ok(nonce)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn apple_script_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_foundation::base::{CFType, ToVoid};

    #[test]
    fn helper_socket_path_is_restricted_to_private_var_tmp() {
        assert!(
            validate_socket_path(Path::new("/private/var/tmp/kitty-pro-tun-501-abcdef.sock"))
                .is_ok()
        );
        assert!(validate_socket_path(Path::new("/tmp/kitty-pro-tun-501-abcdef.sock")).is_err());
        assert!(validate_socket_path(Path::new("/private/var/tmp/unrelated.sock")).is_err());
    }

    #[test]
    fn shell_and_apple_script_escaping_preserve_argument_boundaries() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
        assert_eq!(apple_script_escape("a\\b\"c"), "a\\\\b\\\"c");
    }

    #[test]
    fn unix_peer_identity_includes_the_exact_process() {
        let (left, _right) = UnixStream::pair().expect("unix stream pair should be created");

        let identity = peer_identity(&left).expect("peer identity should be available");

        assert_eq!(identity.uid, unsafe { libc::geteuid() });
        assert_eq!(identity.pid, std::process::id());
        validate_peer_identity(&left, identity.uid, identity.pid)
            .expect("matching peer identity should pass");
        assert!(validate_peer_identity(&left, identity.uid, identity.pid + 1).is_err());
    }

    #[test]
    fn lifecycle_lease_detects_peer_disconnect() {
        let (reader, writer) = UnixStream::pair().expect("unix stream pair should be created");
        let waiter = thread::spawn(move || wait_for_lease_disconnect(reader));

        drop(writer);

        waiter
            .join()
            .expect("lease waiter should finish after peer disconnects");
    }

    #[test]
    fn helper_protocol_preserves_probe_parameters() {
        let request = HelperRequest {
            action: HelperAction::ProbeOutbound {
                tag: "proxy-a".to_string(),
                probe_url: "https://example.com/generate_204".to_string(),
            },
        };
        let payload = serde_json::to_vec(&request).expect("helper request should serialize");
        let decoded =
            serde_json::from_slice::<HelperRequest>(&payload).expect("request should deserialize");

        assert!(matches!(
            decoded.action,
            HelperAction::ProbeOutbound { tag, probe_url }
                if tag == "proxy-a" && probe_url == "https://example.com/generate_204"
        ));
    }

    #[test]
    fn helper_errors_keep_the_callers_error_category() {
        let error = HelperResponse::error("probe failed")
            .ensure_success(CoreError::ProbeUnavailable)
            .expect_err("helper error should be surfaced");

        assert!(matches!(error, CoreError::ProbeUnavailable(message) if message == "probe failed"));
    }

    #[test]
    fn helper_readiness_does_not_start_the_embedded_core() {
        let mut state = HelperState::new(unsafe { libc::getuid() }, unsafe { libc::getgid() });

        let (response, exit) = dispatch_helper_action(HelperAction::Ping, &mut state);

        assert!(response.error.is_none());
        assert!(!exit);
        assert!(!state.core.is_running());
        assert!(state.dns_resolver.is_none());
    }

    #[test]
    fn privileged_helper_accepts_only_private_user_owned_cache_files() {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        let directory = fs::canonicalize(std::env::temp_dir())
            .expect("temporary directory should resolve")
            .join(format!(
                "kitty-pro-cache-test-{}-{}",
                std::process::id(),
                &random_nonce().expect("nonce should be generated")[..12]
            ));
        fs::create_dir(&directory).expect("cache directory should be created");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("cache directory should be private");
        let path = directory.join("sing-box.db");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .expect("cache file should be created");
        let config = serde_json::json!({
            "experimental": {
                "cache_file": {
                    "enabled": true,
                    "path": path,
                    "store_fakeip": true,
                }
            }
        });
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };

        validate_privileged_cache_file(&config, uid, gid)
            .expect("private user-owned cache should pass");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("cache permissions should change");
        let error = validate_privileged_cache_file(&config, uid, gid)
            .expect_err("world-readable cache should be rejected");

        assert!(error.to_string().contains("0600"));
        fs::remove_dir_all(directory).expect("cache fixture should be removed");
    }

    #[test]
    fn session_dns_uses_an_ephemeral_service_dns_key() {
        assert_eq!(
            dns_key("0123456789abcdef").to_string(),
            "State:/Network/Service/com.kitty.pro.tun-dns-0123456789abcdef/DNS"
        );
    }

    #[test]
    fn session_dns_dictionary_targets_the_tun_peer_without_binding_an_interface() {
        let dictionary = default_dns_dictionary(Ipv4Addr::new(172, 19, 0, 2));
        let server_key = unsafe { CFString::wrap_under_get_rule(kSCPropNetDNSServerAddresses) };
        let match_domains_key =
            unsafe { CFString::wrap_under_get_rule(kSCPropNetDNSSupplementalMatchDomains) };
        let match_orders_key =
            unsafe { CFString::wrap_under_get_rule(kSCPropNetDNSSupplementalMatchOrders) };
        let no_search_key = CFString::from_static_string(DNS_NO_SEARCH_KEY);
        let servers = dictionary
            .find(server_key.to_void())
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
            .and_then(CFType::downcast_into::<CFArray>)
            .expect("server addresses must be an array");
        let server_names = servers
            .iter()
            .map(|value| {
                unsafe { CFType::wrap_under_get_rule(*value) }
                    .downcast_into::<CFString>()
                    .expect("server address must be a string")
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(server_names, ["172.19.0.2"]);

        let match_domains = dictionary
            .find(match_domains_key.to_void())
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
            .and_then(CFType::downcast_into::<CFArray>)
            .expect("match domains must be an array");
        let domains = match_domains
            .iter()
            .map(|value| {
                unsafe { CFType::wrap_under_get_rule(*value) }
                    .downcast_into::<CFString>()
                    .expect("match domain must be a string")
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(domains, [""]);

        let match_orders = dictionary
            .find(match_orders_key.to_void())
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
            .and_then(CFType::downcast_into::<CFArray>)
            .expect("match orders must be an array");
        let orders = match_orders
            .iter()
            .map(|value| {
                unsafe { CFType::wrap_under_get_rule(*value) }
                    .downcast_into::<CFNumber>()
                    .expect("match order must be a number")
                    .to_i32()
                    .expect("match order must fit i32")
            })
            .collect::<Vec<_>>();
        assert_eq!(orders, [DNS_MATCH_ORDER]);

        let no_search = dictionary
            .find(no_search_key.to_void())
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
            .and_then(CFType::downcast_into::<CFNumber>)
            .expect("no-search flag must be a number");
        assert_eq!(no_search.to_i32(), Some(1));
        assert_eq!(dictionary.len(), 4);
    }

    #[test]
    fn session_dns_uses_the_next_address_from_the_first_tun_ipv4_cidr() {
        let config = serde_json::json!({
            "inbounds": [
                { "type": "mixed", "tag": "mixed-in" },
                {
                    "type": "tun",
                    "tag": "tun-in",
                    "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"]
                }
            ]
        });
        assert_eq!(
            tun_dns_server(&config).expect("TUN DNS peer should be derived"),
            Ipv4Addr::new(172, 19, 0, 2)
        );
        assert_eq!(
            configured_tun_networks(&config).expect("TUN CIDRs should parse"),
            [
                TunNetwork {
                    address: "172.19.0.1".parse().unwrap(),
                    prefix_len: 30,
                },
                TunNetwork {
                    address: "fdfe:dcba:9876::1".parse().unwrap(),
                    prefix_len: 126,
                },
            ]
        );
        assert_eq!(
            ipv4_tun_peer(TunNetwork {
                address: "10.0.0.2".parse().unwrap(),
                prefix_len: 30,
            }),
            Some(Ipv4Addr::new(10, 0, 0, 3))
        );
        assert_eq!(
            ipv4_tun_peer(TunNetwork {
                address: "10.0.0.0".parse().unwrap(),
                prefix_len: 31,
            }),
            Some(Ipv4Addr::new(10, 0, 0, 1))
        );
        assert_eq!(
            ipv4_tun_peer(TunNetwork {
                address: "10.0.0.1".parse().unwrap(),
                prefix_len: 32,
            }),
            None
        );
    }

    #[test]
    fn session_dns_rejects_tun_configs_without_a_usable_ipv4_next_address() {
        let ipv6_only = serde_json::json!({
            "inbounds": [{
                "type": "tun",
                "address": ["fdfe:dcba:9876::1/126"]
            }]
        });
        let host_route = serde_json::json!({
            "inbounds": [{
                "type": "tun",
                "address": ["172.19.0.1/32", "172.19.0.5/30"]
            }]
        });

        assert!(tun_dns_server(&ipv6_only)
            .expect_err("IPv6-only TUN must be rejected")
            .contains("IPv4 CIDR"));
        assert!(tun_dns_server(&host_route)
            .expect_err("the first IPv4 CIDR must match sing-tun semantics")
            .contains("下一地址"));
    }

    #[test]
    fn invalid_tun_cidr_is_rejected_before_dns_publication() {
        let config = serde_json::json!({
            "inbounds": [{
                "type": "tun",
                "address": ["172.19.0.1/33"]
            }]
        });

        let error = configured_tun_networks(&config).expect_err("invalid CIDR must fail");

        assert!(error.contains("超出范围"));
    }
}
