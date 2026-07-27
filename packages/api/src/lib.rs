//! Shared fullstack APIs used by web, desktop, and mobile shells.

use dioxus::prelude::*;
use proxy_core::{AppProfile, ConnectionRequest, ParseReport, ProxyNode};
#[cfg(not(target_arch = "wasm32"))]
use proxy_core::{SingBoxOptions, TunnelMode};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const MAX_SUBSCRIPTION_BYTES: usize = 10 * 1024 * 1024;

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const MAX_LATENCY_NODES: usize = 32;

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const LATENCY_CHECK_URL: &str = "https://www.gstatic.com/generate_204";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiCoreStatus {
    pub state: String,
    pub version: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeLatency {
    pub tag: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreTraffic {
    pub upload_total: u64,
    pub download_total: u64,
    pub active_connections: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecision {
    Direct,
    Proxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteTargetKind {
    Domain,
    Ip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteLogDetail {
    pub decision: RouteDecision,
    pub target: String,
    pub host: String,
    pub port: Option<u16>,
    pub target_kind: RouteTargetKind,
    pub outbound_type: String,
    pub outbound_tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreLogEntry {
    pub sequence: u64,
    pub timestamp: String,
    pub level: String,
    pub message: String,
    pub route: Option<RouteLogDetail>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreLogBatch {
    pub next_cursor: u64,
    pub entries: Vec<CoreLogEntry>,
}

/// Status of the operating system proxy managed by Kitty Pro.
///
/// The proxy endpoint itself remains fixed to the local sing-box mixed
/// listener. This prevents a browser client from directing system traffic to
/// an arbitrary address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemProxyStatus {
    pub supported: bool,
    pub enabled: bool,
    pub detail: String,
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    get("/api/profile")
)]
pub async fn load_profile() -> Result<AppProfile, ServerFnError> {
    load_native_profile()
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/profile")
)]
pub async fn save_profile(profile: AppProfile) -> Result<(), ServerFnError> {
    save_native_profile(&profile)
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/subscriptions/preview")
)]
pub async fn preview_subscription(source: String) -> Result<ParseReport, ServerFnError> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err(ServerFnError::new("请输入订阅地址或内容"));
    }

    let content = if source.starts_with("http://") || source.starts_with("https://") {
        download_subscription(&source).await?
    } else {
        source
    };
    Ok(proxy_core::parse_subscription(&content))
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/status")
)]
pub async fn core_status() -> Result<ApiCoreStatus, ServerFnError> {
    native_core_status()
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/toggle")
)]
pub async fn set_core_enabled(
    enabled: bool,
    request: Option<ConnectionRequest>,
) -> Result<ApiCoreStatus, ServerFnError> {
    run_native_blocking("sing-box 状态切换任务失败", move || {
        toggle_native_core(enabled, request)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    get("/api/core/traffic")
)]
pub async fn core_traffic() -> Result<CoreTraffic, ServerFnError> {
    native_core_traffic()
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/logs")
)]
pub async fn core_logs(cursor: u64) -> Result<CoreLogBatch, ServerFnError> {
    native_core_logs(cursor)
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/latency")
)]
pub async fn measure_node_latency(
    nodes: Vec<ProxyNode>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
    measure_native_latency(nodes).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    get("/api/system-proxy/status")
)]
pub async fn system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    run_native_blocking("系统代理状态读取任务失败", native_system_proxy_status).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/system-proxy")
)]
pub async fn set_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    run_native_blocking("系统代理设置任务失败", move || {
        set_native_system_proxy(enabled)
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
async fn run_native_blocking<T, F>(
    failure_message: &'static str,
    operation: F,
) -> Result<T, ServerFnError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ServerFnError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| ServerFnError::new(format!("{failure_message}: {error}")))?
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
async fn run_native_blocking<T, F>(
    _failure_message: &'static str,
    operation: F,
) -> Result<T, ServerFnError>
where
    F: FnOnce() -> Result<T, ServerFnError>,
{
    operation()
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
async fn measure_native_latency(nodes: Vec<ProxyNode>) -> Result<Vec<NodeLatency>, ServerFnError> {
    if nodes.is_empty() {
        return Err(ServerFnError::new("没有可探测的节点"));
    }
    if nodes.len() > MAX_LATENCY_NODES {
        return Err(ServerFnError::new(format!(
            "单次最多探测 {MAX_LATENCY_NODES} 个节点"
        )));
    }

    let node_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let request = ConnectionRequest {
        nodes,
        selected_tag: node_tags.first().cloned().unwrap_or_default(),
        mode: TunnelMode::Global,
        tun: false,
    };
    let mut config = proxy_core::build_singbox_config(&request, &SingBoxOptions::default());
    config["log"]["level"] = serde_json::Value::String("error".to_string());
    config["inbounds"] = serde_json::json!([]);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    let config = serde_json::to_string(&config)
        .map_err(|error| ServerFnError::new(format!("序列化探测配置失败: {error}")))?;
    tokio::task::spawn_blocking(move || {
        singbox::probe_outbounds(&config, &node_tags, LATENCY_CHECK_URL)
            .map(|results| {
                results
                    .into_iter()
                    .map(|result| NodeLatency {
                        tag: result.tag,
                        latency_ms: result.latency_ms,
                        error: result.error,
                    })
                    .collect()
            })
            .map_err(|error| ServerFnError::new(error.to_string()))
    })
    .await
    .map_err(|error| ServerFnError::new(format!("节点探测任务失败: {error}")))?
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
async fn measure_native_latency(_nodes: Vec<ProxyNode>) -> Result<Vec<NodeLatency>, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接运行节点延迟探测"))
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
async fn download_subscription(url: &str) -> Result<String, ServerFnError> {
    use std::time::Duration;

    let parsed = reqwest::Url::parse(url)
        .map_err(|error| ServerFnError::new(format!("订阅地址无效: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ServerFnError::new("订阅地址只支持 HTTP/HTTPS"));
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("OKZTWO-Mac-Client-1.5.6 kitty-pro/0.1")
        .build()
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| ServerFnError::new(format!("订阅下载失败: {error}")))?
        .error_for_status()
        .map_err(|error| ServerFnError::new(format!("订阅服务返回错误: {error}")))?;

    if response
        .content_length()
        .is_some_and(|size| size > MAX_SUBSCRIPTION_BYTES as u64)
    {
        return Err(ServerFnError::new("订阅内容超过 10 MiB 限制"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ServerFnError::new(format!("读取订阅失败: {error}")))?;
    if bytes.len() > MAX_SUBSCRIPTION_BYTES {
        return Err(ServerFnError::new("订阅内容超过 10 MiB 限制"));
    }
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ServerFnError::new("订阅内容不是有效的 UTF-8 文本"))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
async fn download_subscription(_url: &str) -> Result<String, ServerFnError> {
    Err(ServerFnError::new("订阅下载只能由原生服务端执行"))
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn native_core_status() -> Result<ApiCoreStatus, ServerFnError> {
    use singbox::{unavailable_status, SingBox};

    let mut guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    let status = if let Some(core) = guard.as_mut() {
        core.status()
    } else {
        match SingBox::discover() {
            Ok(core) => {
                let status = core.status();
                *guard = Some(core);
                status
            }
            Err(_) => unavailable_status(),
        }
    };
    Ok(api_core_status(status))
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
fn native_core_status() -> Result<ApiCoreStatus, ServerFnError> {
    singbox::android::status()
        .map(api_core_status)
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
fn api_core_status(status: singbox::CoreStatus) -> ApiCoreStatus {
    use singbox::CoreState;

    let state = match status.state {
        CoreState::Unavailable => "unavailable",
        CoreState::Stopped => "stopped",
        CoreState::Running => "running",
    };
    ApiCoreStatus {
        state: state.to_string(),
        version: status.version,
        note: status.platform_note,
    }
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
fn load_native_profile() -> Result<AppProfile, ServerFnError> {
    use std::fs;

    let path = profile_path()?;
    match fs::read(&path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| ServerFnError::new(format!("本地配置损坏: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppProfile::default()),
        Err(error) => Err(ServerFnError::new(format!("读取本地配置失败: {error}"))),
    }
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn load_native_profile() -> Result<AppProfile, ServerFnError> {
    Ok(AppProfile::default())
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
fn save_native_profile(profile: &AppProfile) -> Result<(), ServerFnError> {
    use std::fs;
    use std::io::Write;

    let path = profile_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| ServerFnError::new("本地配置目录无效"))?;
    fs::create_dir_all(parent)
        .map_err(|error| ServerFnError::new(format!("创建本地配置目录失败: {error}")))?;

    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(profile)
        .map_err(|error| ServerFnError::new(format!("序列化本地配置失败: {error}")))?;
    let mut file = create_private_file(&temporary)
        .map_err(|error| ServerFnError::new(format!("写入本地配置失败: {error}")))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| ServerFnError::new(format!("写入本地配置失败: {error}")))?;
    drop(file);

    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| ServerFnError::new(format!("替换本地配置失败: {error}")))?;
    }
    fs::rename(&temporary, &path)
        .map_err(|error| ServerFnError::new(format!("保存本地配置失败: {error}")))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn save_native_profile(_profile: &AppProfile) -> Result<(), ServerFnError> {
    Err(ServerFnError::new("浏览器端不能直接写入本地配置"))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn profile_path() -> Result<std::path::PathBuf, ServerFnError> {
    let directories = directories::ProjectDirs::from("com", "kitty", "kitty-pro")
        .ok_or_else(|| ServerFnError::new("无法确定本地配置目录"))?;
    Ok(directories.data_local_dir().join("profile.json"))
}

#[cfg(target_os = "android")]
fn profile_path() -> Result<std::path::PathBuf, ServerFnError> {
    singbox::android::files_dir()
        .map(|directory| directory.join("profile.json"))
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
fn create_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    options.open(path)
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn native_core_status() -> Result<ApiCoreStatus, ServerFnError> {
    Ok(ApiCoreStatus {
        state: "unavailable".to_string(),
        version: None,
        note: Some("浏览器目标不能直接运行嵌入式 sing-box 内核".to_string()),
    })
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn toggle_native_core(
    enabled: bool,
    request: Option<ConnectionRequest>,
) -> Result<ApiCoreStatus, ServerFnError> {
    use proxy_core::SingBoxOptions;
    use singbox::SingBox;

    let mut guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    if !enabled {
        if let Some(core) = guard.as_mut() {
            if core
                .is_running()
                .map_err(|error| ServerFnError::new(error.to_string()))?
            {
                core.stop()
                    .map_err(|error| ServerFnError::new(error.to_string()))?;
            }
        }
        drop(guard);
        return native_core_status();
    }

    let request = request.ok_or_else(|| ServerFnError::new("缺少连接配置"))?;
    if request.nodes.is_empty() {
        return Err(ServerFnError::new("请先导入并选择一个节点"));
    }
    let mut core = match guard.take() {
        Some(core) => core,
        None => SingBox::discover().map_err(|error| ServerFnError::new(error.to_string()))?,
    };
    if core
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        *guard = Some(core);
        drop(guard);
        return native_core_status();
    }

    let options = SingBoxOptions {
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        ..SingBoxOptions::default()
    };
    core.start(&request, &options)
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    *guard = Some(core);
    drop(guard);
    native_core_status()
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
fn toggle_native_core(
    enabled: bool,
    request: Option<ConnectionRequest>,
) -> Result<ApiCoreStatus, ServerFnError> {
    if !enabled {
        singbox::android::stop().map_err(|error| ServerFnError::new(error.to_string()))?;
        return native_core_status();
    }

    let mut request = request.ok_or_else(|| ServerFnError::new("缺少连接配置"))?;
    if request.nodes.is_empty() {
        return Err(ServerFnError::new("请先导入并选择一个节点"));
    }
    // Android always owns the TUN through VpnService. A loopback mixed proxy
    // alone would not route device traffic.
    request.tun = true;
    let options = SingBoxOptions {
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        ..SingBoxOptions::default()
    };
    let mut config = proxy_core::build_singbox_config(&request, &options);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    let config = serde_json::to_string(&config)
        .map_err(|error| ServerFnError::new(format!("序列化 Android VPN 配置失败: {error}")))?;
    singbox::android::start(&config).map_err(|error| ServerFnError::new(error.to_string()))?;
    native_core_status()
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn native_core_traffic() -> Result<CoreTraffic, ServerFnError> {
    let guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    let Some(core) = guard.as_ref() else {
        return Ok(CoreTraffic::default());
    };
    if !core
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        return Ok(CoreTraffic::default());
    }
    let traffic = core
        .traffic()
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    Ok(CoreTraffic {
        upload_total: traffic.upload_total,
        download_total: traffic.download_total,
        active_connections: traffic.active_connections,
    })
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
fn native_core_traffic() -> Result<CoreTraffic, ServerFnError> {
    let traffic =
        singbox::android::traffic().map_err(|error| ServerFnError::new(error.to_string()))?;
    Ok(CoreTraffic {
        upload_total: traffic.upload_total,
        download_total: traffic.download_total,
        active_connections: traffic.active_connections,
    })
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn native_core_traffic() -> Result<CoreTraffic, ServerFnError> {
    Ok(CoreTraffic::default())
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn native_core_logs(cursor: u64) -> Result<CoreLogBatch, ServerFnError> {
    let guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    let Some(core) = guard.as_ref() else {
        return Ok(CoreLogBatch {
            next_cursor: cursor,
            entries: Vec::new(),
        });
    };
    if !core
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        return Ok(CoreLogBatch {
            next_cursor: cursor,
            entries: Vec::new(),
        });
    }
    core.logs(cursor)
        .map(normalize_log_batch)
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
fn native_core_logs(cursor: u64) -> Result<CoreLogBatch, ServerFnError> {
    singbox::android::logs(cursor)
        .map(normalize_log_batch)
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn native_core_logs(cursor: u64) -> Result<CoreLogBatch, ServerFnError> {
    Ok(CoreLogBatch {
        next_cursor: cursor,
        entries: Vec::new(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn normalize_log_batch(batch: singbox::LogBatch) -> CoreLogBatch {
    CoreLogBatch {
        next_cursor: batch.next_cursor,
        entries: batch
            .entries
            .into_iter()
            .map(|entry| CoreLogEntry {
                sequence: entry.sequence,
                timestamp: entry.timestamp,
                level: entry.level,
                route: parse_route_log(&entry.message),
                message: entry.message,
            })
            .collect(),
    }
}

fn parse_route_log(message: &str) -> Option<RouteLogDetail> {
    let component = message.split_once("outbound/")?.1;
    let type_end = component.find('[')?;
    let outbound_type = component[..type_end].trim();
    let tag_start = type_end + 1;
    let tag_end = component[tag_start..].find(']')? + tag_start;
    let outbound_tag = component[tag_start..tag_end].trim();
    let detail = component[tag_end + 1..].trim_start_matches(':').trim();
    let target_start = detail.find("connection to ")? + "connection to ".len();
    let target = detail[target_start..].trim();
    if target.is_empty() {
        return None;
    }

    let (host, port) = split_route_target(target);
    let target_kind = if host.parse::<std::net::IpAddr>().is_ok() {
        RouteTargetKind::Ip
    } else {
        RouteTargetKind::Domain
    };
    Some(RouteLogDetail {
        decision: if outbound_type == "direct" || outbound_tag == "direct" {
            RouteDecision::Direct
        } else {
            RouteDecision::Proxy
        },
        target: target.to_string(),
        host,
        port,
        target_kind,
        outbound_type: outbound_type.to_string(),
        outbound_tag: outbound_tag.to_string(),
    })
}

fn split_route_target(target: &str) -> (String, Option<u16>) {
    if let Some(ipv6) = target.strip_prefix('[') {
        if let Some((host, port)) = ipv6.split_once("]:") {
            return (host.to_string(), port.parse().ok());
        }
    }
    if let Some((host, port)) = target.rsplit_once(':') {
        if !host.is_empty() {
            if let Ok(port) = port.parse() {
                return (host.to_string(), Some(port));
            }
        }
    }
    (target.to_string(), None)
}

#[cfg(not(target_arch = "wasm32"))]
fn allocate_loopback_port() -> Result<u16, ServerFnError> {
    use std::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| ServerFnError::new(format!("分配流量统计端口失败: {error}")))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| ServerFnError::new(format!("读取流量统计端口失败: {error}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn generate_traffic_api_secret() -> Result<String, ServerFnError> {
    use std::fmt::Write;

    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes)
        .map_err(|error| ServerFnError::new(format!("生成流量统计令牌失败: {error}")))?;
    let mut secret = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(secret, "{byte:02x}");
    }
    Ok(secret)
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn toggle_native_core(
    _enabled: bool,
    _request: Option<ConnectionRequest>,
) -> Result<ApiCoreStatus, ServerFnError> {
    Err(ServerFnError::new(
        "浏览器目标不能直接运行嵌入式 sing-box 内核",
    ))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const SYSTEM_PROXY_HOST: &str = "127.0.0.1";
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const SYSTEM_PROXY_PORT: u16 = 7890;

#[cfg(target_os = "windows")]
const WINDOWS_INTERNET_SETTINGS: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

#[cfg(target_os = "linux")]
const LINUX_PROXY_KEYS: &[(&str, &str)] = &[
    ("org.gnome.system.proxy", "mode"),
    ("org.gnome.system.proxy", "use-same-proxy"),
    ("org.gnome.system.proxy.http", "enabled"),
    ("org.gnome.system.proxy.http", "host"),
    ("org.gnome.system.proxy.http", "port"),
    ("org.gnome.system.proxy.https", "host"),
    ("org.gnome.system.proxy.https", "port"),
    ("org.gnome.system.proxy.socks", "host"),
    ("org.gnome.system.proxy.socks", "port"),
];

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MacProxySettings {
    enabled: bool,
    server: String,
    port: u16,
    authenticated: bool,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MacServiceProxyBackup {
    service: String,
    web: MacProxySettings,
    secure_web: MacProxySettings,
    socks: MacProxySettings,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MacSystemProxyBackup {
    services: Vec<MacServiceProxyBackup>,
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowsSystemProxyBackup {
    proxy_enabled: Option<u32>,
    proxy_server: Option<String>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LinuxProxySettingBackup {
    schema: String,
    key: String,
    value: String,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LinuxSystemProxyBackup {
    settings: Vec<LinuxProxySettingBackup>,
}

#[cfg(target_os = "macos")]
fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    let services = mac_network_services()?;
    let mut configured = 0usize;

    for service in &services {
        let web = mac_proxy_settings(service, "-getwebproxy")?;
        let secure_web = mac_proxy_settings(service, "-getsecurewebproxy")?;
        let socks = mac_proxy_settings(service, "-getsocksfirewallproxy")?;
        if [web, secure_web, socks].into_iter().all(is_kitty_proxy) {
            configured += 1;
        }
    }

    let enabled = !services.is_empty() && configured == services.len();
    let detail = if enabled {
        format!("已为 {configured} 个网络服务设置 127.0.0.1:7890")
    } else if configured > 0 {
        format!(
            "已为 {configured}/{} 个网络服务设置本地代理",
            services.len()
        )
    } else {
        "未启用系统代理".to_string()
    };
    Ok(SystemProxyStatus {
        supported: true,
        enabled,
        detail,
    })
}

#[cfg(target_os = "windows")]
fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    let backup = windows_proxy_settings()?;
    let proxy_enabled = backup.proxy_enabled.unwrap_or(0) != 0;
    let proxy_server = backup.proxy_server.unwrap_or_default();
    let enabled = proxy_enabled && is_kitty_windows_proxy(&proxy_server);
    let detail = if enabled {
        "已设置 Windows 系统代理 127.0.0.1:7890".to_string()
    } else if proxy_enabled {
        "系统当前使用其他代理，Kitty Pro 未接管".to_string()
    } else {
        "未启用系统代理".to_string()
    };
    Ok(SystemProxyStatus {
        supported: true,
        enabled,
        detail,
    })
}

#[cfg(target_os = "linux")]
fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    if let Some(detail) = linux_proxy_unavailable_reason() {
        return Ok(SystemProxyStatus {
            supported: false,
            enabled: false,
            detail,
        });
    }

    let mode = linux_gsettings_get("org.gnome.system.proxy", "mode")?;
    let http_enabled = linux_gsettings_get("org.gnome.system.proxy.http", "enabled")?;
    let http_host = linux_gsettings_get("org.gnome.system.proxy.http", "host")?;
    let http_port = linux_gsettings_get("org.gnome.system.proxy.http", "port")?;
    let https_host = linux_gsettings_get("org.gnome.system.proxy.https", "host")?;
    let https_port = linux_gsettings_get("org.gnome.system.proxy.https", "port")?;
    let socks_host = linux_gsettings_get("org.gnome.system.proxy.socks", "host")?;
    let socks_port = linux_gsettings_get("org.gnome.system.proxy.socks", "port")?;
    let enabled = gvariant_string(&mode) == "manual"
        && http_enabled == "true"
        && [http_host, https_host, socks_host]
            .iter()
            .all(|host| gvariant_string(host) == SYSTEM_PROXY_HOST)
        && [http_port, https_port, socks_port]
            .iter()
            .all(|port| port == &SYSTEM_PROXY_PORT.to_string());
    let detail = if enabled {
        "已设置 GNOME 系统代理 127.0.0.1:7890".to_string()
    } else if gvariant_string(&mode) == "manual" {
        "GNOME 当前使用其他手动代理，Kitty Pro 未接管".to_string()
    } else {
        "未启用系统代理".to_string()
    };
    Ok(SystemProxyStatus {
        supported: true,
        enabled,
        detail,
    })
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    Ok(SystemProxyStatus {
        supported: false,
        enabled: false,
        detail: "当前平台尚未实现系统代理适配".to_string(),
    })
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    Ok(SystemProxyStatus {
        supported: false,
        enabled: false,
        detail: "浏览器目标不能直接修改系统代理".to_string(),
    })
}

#[cfg(target_os = "macos")]
fn set_native_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    if enabled {
        ensure_core_is_running()?;
        let backup_path = system_proxy_backup_path()?;
        if backup_path.exists() {
            let current = native_system_proxy_status()?;
            if current.enabled {
                return Ok(current);
            }
            return Err(ServerFnError::new(
                "检测到未恢复的系统代理备份，请先关闭系统代理",
            ));
        }

        let services = mac_network_services()?;
        if services.is_empty() {
            return Err(ServerFnError::new("未找到已启用的 macOS 网络服务"));
        }
        let backup = MacSystemProxyBackup {
            services: services
                .iter()
                .map(|service| {
                    Ok(MacServiceProxyBackup {
                        service: service.clone(),
                        web: mac_proxy_settings(service, "-getwebproxy")?,
                        secure_web: mac_proxy_settings(service, "-getsecurewebproxy")?,
                        socks: mac_proxy_settings(service, "-getsocksfirewallproxy")?,
                    })
                })
                .collect::<Result<Vec<_>, ServerFnError>>()?,
        };
        if backup.services.iter().any(|service| {
            [&service.web, &service.secure_web, &service.socks]
                .into_iter()
                .any(|settings| settings.authenticated)
        }) {
            return Err(ServerFnError::new(
                "检测到已有需要认证的代理配置，已拒绝覆盖以保护原设置",
            ));
        }

        write_system_proxy_backup(&backup)?;
        if let Err(error) = apply_kitty_proxy(&backup.services) {
            let _ = restore_mac_proxy_backup(&backup);
            let _ = std::fs::remove_file(&backup_path);
            return Err(error);
        }
        native_system_proxy_status()
    } else {
        let backup_path = system_proxy_backup_path()?;
        let backup = read_system_proxy_backup(&backup_path)?;
        restore_mac_proxy_backup(&backup)?;
        std::fs::remove_file(&backup_path)
            .map_err(|error| ServerFnError::new(format!("清理代理备份失败: {error}")))?;
        native_system_proxy_status()
    }
}

#[cfg(target_os = "windows")]
fn set_native_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    if enabled {
        ensure_core_is_running()?;
        let backup_path = system_proxy_backup_path()?;
        if backup_path.exists() {
            let current = native_system_proxy_status()?;
            if current.enabled {
                return Ok(current);
            }
            return Err(ServerFnError::new(
                "检测到未恢复的系统代理备份，请先关闭系统代理",
            ));
        }

        let backup = windows_proxy_settings()?;
        write_system_proxy_backup(&backup)?;
        if let Err(error) = apply_kitty_windows_proxy() {
            let _ = restore_windows_proxy_backup(&backup);
            let _ = std::fs::remove_file(&backup_path);
            return Err(error);
        }
        native_system_proxy_status()
    } else {
        let backup_path = system_proxy_backup_path()?;
        let backup: WindowsSystemProxyBackup = read_system_proxy_backup(&backup_path)?;
        restore_windows_proxy_backup(&backup)?;
        std::fs::remove_file(&backup_path)
            .map_err(|error| ServerFnError::new(format!("清理代理备份失败: {error}")))?;
        native_system_proxy_status()
    }
}

#[cfg(target_os = "linux")]
fn set_native_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    if let Some(detail) = linux_proxy_unavailable_reason() {
        return Err(ServerFnError::new(detail));
    }

    if enabled {
        ensure_core_is_running()?;
        let backup_path = system_proxy_backup_path()?;
        if backup_path.exists() {
            let current = native_system_proxy_status()?;
            if current.enabled {
                return Ok(current);
            }
            return Err(ServerFnError::new(
                "检测到未恢复的系统代理备份，请先关闭系统代理",
            ));
        }

        let backup = LinuxSystemProxyBackup {
            settings: LINUX_PROXY_KEYS
                .iter()
                .map(|(schema, key)| {
                    Ok(LinuxProxySettingBackup {
                        schema: (*schema).to_string(),
                        key: (*key).to_string(),
                        value: linux_gsettings_get(schema, key)?,
                    })
                })
                .collect::<Result<Vec<_>, ServerFnError>>()?,
        };
        write_system_proxy_backup(&backup)?;
        if let Err(error) = apply_kitty_linux_proxy() {
            let _ = restore_linux_proxy_backup(&backup);
            let _ = std::fs::remove_file(&backup_path);
            return Err(error);
        }
        native_system_proxy_status()
    } else {
        let backup_path = system_proxy_backup_path()?;
        let backup: LinuxSystemProxyBackup = read_system_proxy_backup(&backup_path)?;
        restore_linux_proxy_backup(&backup)?;
        std::fs::remove_file(&backup_path)
            .map_err(|error| ServerFnError::new(format!("清理代理备份失败: {error}")))?;
        native_system_proxy_status()
    }
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
fn set_native_system_proxy(_enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    Err(ServerFnError::new("当前平台尚未实现系统代理适配"))
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn set_native_system_proxy(_enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接修改系统代理"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn ensure_core_is_running() -> Result<(), ServerFnError> {
    use singbox::CoreState;

    let guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    let Some(core) = guard.as_ref() else {
        return Err(ServerFnError::new("请先建立连接，再启用系统代理"));
    };
    let status = core.status();
    if status.state != CoreState::Running {
        return Err(ServerFnError::new("请先建立连接，再启用系统代理"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_internet_settings(access: u32) -> Result<winreg::RegKey, ServerFnError> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(WINDOWS_INTERNET_SETTINGS, access)
        .map_err(|error| ServerFnError::new(format!("无法读取 Windows 系统代理: {error}")))
}

#[cfg(target_os = "windows")]
fn windows_proxy_settings() -> Result<WindowsSystemProxyBackup, ServerFnError> {
    use std::io::ErrorKind;
    use winreg::enums::KEY_READ;

    let settings = windows_internet_settings(KEY_READ)?;
    let proxy_enabled = match settings.get_value("ProxyEnable") {
        Ok(value) => Some(value),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(ServerFnError::new(format!(
                "读取 Windows ProxyEnable 失败: {error}"
            )))
        }
    };
    let proxy_server = match settings.get_value("ProxyServer") {
        Ok(value) => Some(value),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(ServerFnError::new(format!(
                "读取 Windows ProxyServer 失败: {error}"
            )))
        }
    };
    Ok(WindowsSystemProxyBackup {
        proxy_enabled,
        proxy_server,
    })
}

#[cfg(target_os = "windows")]
fn is_kitty_windows_proxy(server: &str) -> bool {
    let endpoint = format!("{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}");
    if !server.contains('=') {
        return server.eq_ignore_ascii_case(&endpoint);
    }

    let mut http = false;
    let mut https = false;
    let mut socks = false;
    for entry in server.split(';') {
        let Some((scheme, value)) = entry.trim().split_once('=') else {
            continue;
        };
        if !value.trim().eq_ignore_ascii_case(&endpoint) {
            continue;
        }
        match scheme.trim().to_ascii_lowercase().as_str() {
            "http" => http = true,
            "https" => https = true,
            "socks" => socks = true,
            _ => {}
        }
    }
    http && https && socks
}

#[cfg(target_os = "windows")]
fn apply_kitty_windows_proxy() -> Result<(), ServerFnError> {
    use winreg::enums::{KEY_READ, KEY_WRITE};

    let settings = windows_internet_settings(KEY_READ | KEY_WRITE)?;
    let endpoint = format!("{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}");
    let server = format!("http={endpoint};https={endpoint};socks={endpoint}");
    settings
        .set_value("ProxyServer", &server)
        .and_then(|_| settings.set_value("ProxyEnable", &1u32))
        .map_err(|error| ServerFnError::new(format!("设置 Windows 系统代理失败: {error}")))?;
    notify_windows_proxy_changed()
}

#[cfg(target_os = "windows")]
fn restore_windows_proxy_backup(backup: &WindowsSystemProxyBackup) -> Result<(), ServerFnError> {
    use std::io::ErrorKind;
    use winreg::enums::{KEY_READ, KEY_WRITE};

    let settings = windows_internet_settings(KEY_READ | KEY_WRITE)?;
    match backup.proxy_server.as_ref() {
        Some(value) => settings.set_value("ProxyServer", value),
        None => settings.delete_value("ProxyServer"),
    }
    .or_else(|error| {
        if error.kind() == ErrorKind::NotFound {
            Ok(())
        } else {
            Err(error)
        }
    })
    .map_err(|error| ServerFnError::new(format!("恢复 Windows ProxyServer 失败: {error}")))?;
    match backup.proxy_enabled {
        Some(value) => settings.set_value("ProxyEnable", &value),
        None => settings.delete_value("ProxyEnable"),
    }
    .or_else(|error| {
        if error.kind() == ErrorKind::NotFound {
            Ok(())
        } else {
            Err(error)
        }
    })
    .map_err(|error| ServerFnError::new(format!("恢复 Windows ProxyEnable 失败: {error}")))?;
    notify_windows_proxy_changed()
}

#[cfg(target_os = "windows")]
fn notify_windows_proxy_changed() -> Result<(), ServerFnError> {
    use std::ffi::c_void;
    use std::ptr::null_mut;

    const INTERNET_OPTION_REFRESH: u32 = 37;
    const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;

    #[link(name = "wininet")]
    extern "system" {
        fn InternetSetOptionW(
            internet: *mut c_void,
            option: u32,
            buffer: *mut c_void,
            buffer_length: u32,
        ) -> i32;
    }

    let settings_changed =
        unsafe { InternetSetOptionW(null_mut(), INTERNET_OPTION_SETTINGS_CHANGED, null_mut(), 0) };
    let refreshed =
        unsafe { InternetSetOptionW(null_mut(), INTERNET_OPTION_REFRESH, null_mut(), 0) };
    if settings_changed == 0 || refreshed == 0 {
        return Err(ServerFnError::new(format!(
            "通知 Windows 刷新系统代理失败: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_proxy_unavailable_reason() -> Option<String> {
    use std::process::Command;

    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !desktop.contains("gnome") && !desktop.contains("unity") {
        return Some("当前 Linux 桌面不是 GNOME/Unity，无法安全修改系统代理".to_string());
    }
    match Command::new("gsettings").arg("--version").output() {
        Ok(output) if output.status.success() => None,
        _ => Some("当前系统缺少可用的 gsettings，无法修改 GNOME 系统代理".to_string()),
    }
}

#[cfg(target_os = "linux")]
fn linux_gsettings_get(schema: &str, key: &str) -> Result<String, ServerFnError> {
    use std::process::Command;

    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .map_err(|error| ServerFnError::new(format!("无法读取 GNOME 系统代理: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ServerFnError::new(if message.is_empty() {
            format!("读取 GNOME 系统代理 {schema} {key} 失败")
        } else {
            format!("读取 GNOME 系统代理失败: {message}")
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
fn linux_gsettings_set(schema: &str, key: &str, value: &str) -> Result<(), ServerFnError> {
    use std::process::Command;

    let output = Command::new("gsettings")
        .args(["set", schema, key, value])
        .output()
        .map_err(|error| ServerFnError::new(format!("无法设置 GNOME 系统代理: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ServerFnError::new(if message.is_empty() {
            format!("设置 GNOME 系统代理 {schema} {key} 失败")
        } else {
            format!("设置 GNOME 系统代理失败: {message}")
        }));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn gvariant_string(value: &str) -> &str {
    value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .or_else(|| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        })
        .unwrap_or(value)
}

#[cfg(target_os = "linux")]
fn apply_kitty_linux_proxy() -> Result<(), ServerFnError> {
    let host = format!("'{SYSTEM_PROXY_HOST}'");
    let port = SYSTEM_PROXY_PORT.to_string();
    for (schema, key, value) in [
        ("org.gnome.system.proxy", "use-same-proxy", "false"),
        ("org.gnome.system.proxy.http", "enabled", "true"),
        ("org.gnome.system.proxy.http", "host", host.as_str()),
        ("org.gnome.system.proxy.http", "port", port.as_str()),
        ("org.gnome.system.proxy.https", "host", host.as_str()),
        ("org.gnome.system.proxy.https", "port", port.as_str()),
        ("org.gnome.system.proxy.socks", "host", host.as_str()),
        ("org.gnome.system.proxy.socks", "port", port.as_str()),
    ] {
        linux_gsettings_set(schema, key, value)?;
    }
    linux_gsettings_set("org.gnome.system.proxy", "mode", "'manual'")
}

#[cfg(target_os = "linux")]
fn restore_linux_proxy_backup(backup: &LinuxSystemProxyBackup) -> Result<(), ServerFnError> {
    for setting in backup
        .settings
        .iter()
        .filter(|setting| !(setting.schema == "org.gnome.system.proxy" && setting.key == "mode"))
    {
        linux_gsettings_set(&setting.schema, &setting.key, &setting.value)?;
    }
    if let Some(mode) = backup
        .settings
        .iter()
        .find(|setting| setting.schema == "org.gnome.system.proxy" && setting.key == "mode")
    {
        linux_gsettings_set(&mode.schema, &mode.key, &mode.value)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn mac_network_services() -> Result<Vec<String>, ServerFnError> {
    let output = run_networksetup(&["-listallnetworkservices"])?;
    Ok(output
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|service| !service.is_empty() && !service.starts_with('*'))
        .map(ToOwned::to_owned)
        .collect())
}

#[cfg(target_os = "macos")]
fn mac_proxy_settings(service: &str, flag: &str) -> Result<MacProxySettings, ServerFnError> {
    let output = run_networksetup(&[flag, service])?;
    let mut enabled = false;
    let mut server = String::new();
    let mut port = 0;
    let mut authenticated = false;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Enabled" => enabled = value.trim().eq_ignore_ascii_case("yes"),
            "Server" => server = value.trim().to_string(),
            "Port" => port = value.trim().parse().unwrap_or(0),
            "Authenticated Proxy Enabled" => authenticated = value.trim() == "1",
            _ => {}
        }
    }
    Ok(MacProxySettings {
        enabled,
        server,
        port,
        authenticated,
    })
}

#[cfg(target_os = "macos")]
fn is_kitty_proxy(settings: MacProxySettings) -> bool {
    settings.enabled && settings.server == SYSTEM_PROXY_HOST && settings.port == SYSTEM_PROXY_PORT
}

#[cfg(target_os = "macos")]
fn apply_kitty_proxy(services: &[MacServiceProxyBackup]) -> Result<(), ServerFnError> {
    for service in services {
        set_mac_proxy(&service.service, "-setwebproxy", "-setwebproxystate", None)?;
        set_mac_proxy(
            &service.service,
            "-setsecurewebproxy",
            "-setsecurewebproxystate",
            None,
        )?;
        set_mac_proxy(
            &service.service,
            "-setsocksfirewallproxy",
            "-setsocksfirewallproxystate",
            None,
        )?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn restore_mac_proxy_backup(backup: &MacSystemProxyBackup) -> Result<(), ServerFnError> {
    for service in &backup.services {
        set_mac_proxy(
            &service.service,
            "-setwebproxy",
            "-setwebproxystate",
            Some(&service.web),
        )?;
        set_mac_proxy(
            &service.service,
            "-setsecurewebproxy",
            "-setsecurewebproxystate",
            Some(&service.secure_web),
        )?;
        set_mac_proxy(
            &service.service,
            "-setsocksfirewallproxy",
            "-setsocksfirewallproxystate",
            Some(&service.socks),
        )?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_mac_proxy(
    service: &str,
    set_flag: &str,
    state_flag: &str,
    restore: Option<&MacProxySettings>,
) -> Result<(), ServerFnError> {
    let (server, port, state) = match restore {
        Some(settings) => (settings.server.as_str(), settings.port, settings.enabled),
        None => (SYSTEM_PROXY_HOST, SYSTEM_PROXY_PORT, true),
    };
    if !server.is_empty() && port > 0 {
        let port = port.to_string();
        run_networksetup(&[set_flag, service, server, &port])?;
    }
    run_networksetup(&[state_flag, service, if state { "on" } else { "off" }])?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_networksetup(arguments: &[&str]) -> Result<String, ServerFnError> {
    use std::process::Command;

    let output = Command::new("/usr/sbin/networksetup")
        .args(arguments)
        .output()
        .map_err(|error| ServerFnError::new(format!("无法执行 macOS 网络设置: {error}")))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ServerFnError::new(if message.is_empty() {
            "macOS 网络设置命令执行失败".to_string()
        } else {
            format!("macOS 网络设置失败: {message}")
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn system_proxy_backup_path() -> Result<std::path::PathBuf, ServerFnError> {
    let profile = profile_path()?;
    let parent = profile
        .parent()
        .ok_or_else(|| ServerFnError::new("本地配置目录无效"))?;
    Ok(parent.join("system-proxy-backup.json"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn write_system_proxy_backup<T: Serialize>(backup: &T) -> Result<(), ServerFnError> {
    use std::io::Write;

    let path = system_proxy_backup_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| ServerFnError::new("本地配置目录无效"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| ServerFnError::new(format!("创建代理备份目录失败: {error}")))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(backup)
        .map_err(|error| ServerFnError::new(format!("序列化代理备份失败: {error}")))?;
    let mut file = create_private_file(&temporary)
        .map_err(|error| ServerFnError::new(format!("写入代理备份失败: {error}")))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| ServerFnError::new(format!("写入代理备份失败: {error}")))?;
    drop(file);
    std::fs::rename(&temporary, path)
        .map_err(|error| ServerFnError::new(format!("保存代理备份失败: {error}")))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn read_system_proxy_backup<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
) -> Result<T, ServerFnError> {
    let bytes = std::fs::read(path)
        .map_err(|error| ServerFnError::new(format!("没有可恢复的系统代理备份: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ServerFnError::new(format!("系统代理备份损坏: {error}")))
}

/// Restore any operating-system proxy managed by Kitty Pro, then stop the
/// embedded core. Desktop launchers call this during normal event-loop
/// teardown so users are not left with a dead loopback proxy after exit.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn shutdown_native_runtime() -> Result<(), String> {
    let mut errors = Vec::new();

    match system_proxy_backup_path() {
        Ok(path) if path.exists() => {
            if let Err(error) = set_native_system_proxy(false) {
                errors.push(format!("恢复系统代理失败: {error}"));
            }
        }
        Ok(_) => {}
        Err(error) => errors.push(format!("读取系统代理备份路径失败: {error}")),
    }

    match core_slot().lock() {
        Ok(mut guard) => {
            if let Some(core) = guard.as_mut() {
                match core.is_running() {
                    Ok(true) => {
                        if let Err(error) = core.stop() {
                            errors.push(format!("停止 sing-box 内核失败: {error}"));
                        }
                    }
                    Ok(false) => {}
                    Err(error) => errors.push(format!("读取 sing-box 状态失败: {error}")),
                }
            }
        }
        Err(_) => errors.push("sing-box 状态锁已损坏".to_string()),
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn core_slot() -> &'static std::sync::Mutex<Option<singbox::SingBox>> {
    use std::sync::{Mutex, OnceLock};

    static CORE: OnceLock<Mutex<Option<singbox::SingBox>>> = OnceLock::new();
    CORE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "server")))]
    #[test]
    fn native_subscription_preview_runs_in_process() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("native test runtime should start");
        let report = runtime
            .block_on(preview_subscription(
                "trojan://password@example.com:443#Direct".to_string(),
            ))
            .expect("native subscription preview should not require an HTTP backend");

        assert_eq!(report.nodes.len(), 1);
        assert_eq!(report.nodes[0].protocol, proxy_core::ProxyProtocol::Trojan);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_blocking_operations_run_off_the_calling_thread() {
        let calling_thread = std::thread::current().id();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("native test runtime should start");
        let operation_thread = runtime
            .block_on(run_native_blocking("test task failed", || {
                Ok::<_, ServerFnError>(std::thread::current().id())
            }))
            .expect("blocking operation should complete");

        assert_ne!(calling_thread, operation_thread);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "server")))]
    #[test]
    #[ignore = "requires KITTY_TEST_SUBSCRIPTION_URL and live network access"]
    fn live_native_latency_probe_returns_without_aborting() {
        let source = std::env::var("KITTY_TEST_SUBSCRIPTION_URL")
            .expect("set KITTY_TEST_SUBSCRIPTION_URL to a live subscription");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("native test runtime should start");
        let results = runtime.block_on(async move {
            let mut report = preview_subscription(source).await?;
            report.nodes.truncate(8);
            if report.nodes.is_empty() {
                return Err(ServerFnError::new("订阅没有可探测的节点"));
            }
            measure_node_latency(report.nodes).await
        });
        let results = results.expect("live node probe should return per-node results");

        assert!(!results.is_empty());
        assert!(results
            .iter()
            .all(|result| result.latency_ms.is_some() || result.error.is_some()));
    }

    #[test]
    fn parses_direct_domain_route_log() {
        let route = parse_route_log(
            "INFO[0001] [12 0ms] outbound/direct[direct]: outbound connection to www.baidu.com:443",
        )
        .expect("direct route should be parsed");

        assert_eq!(route.decision, RouteDecision::Direct);
        assert_eq!(route.host, "www.baidu.com");
        assert_eq!(route.port, Some(443));
        assert_eq!(route.target_kind, RouteTargetKind::Domain);
        assert_eq!(route.outbound_tag, "direct");
    }

    #[test]
    fn parses_proxy_ip_and_ipv6_route_logs() {
        let proxy = parse_route_log(
            "INFO[0002] outbound/vless[subscription-1-edge]: outbound packet connection to 8.8.8.8:53",
        )
        .expect("proxy route should be parsed");
        assert_eq!(proxy.decision, RouteDecision::Proxy);
        assert_eq!(proxy.host, "8.8.8.8");
        assert_eq!(proxy.port, Some(53));
        assert_eq!(proxy.target_kind, RouteTargetKind::Ip);
        assert_eq!(proxy.outbound_type, "vless");

        let ipv6 = parse_route_log(
            "INFO[0003] outbound/direct[direct]: outbound connection to [2001:db8::1]:443",
        )
        .expect("IPv6 route should be parsed");
        assert_eq!(ipv6.host, "2001:db8::1");
        assert_eq!(ipv6.port, Some(443));
        assert_eq!(ipv6.target_kind, RouteTargetKind::Ip);
    }

    #[test]
    fn ignores_non_outbound_log_lines() {
        assert!(parse_route_log(
            "INFO[0001] inbound/mixed[mixed-in]: inbound connection to example.com:443"
        )
        .is_none());
    }
}
