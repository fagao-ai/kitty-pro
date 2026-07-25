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

#[cfg_attr(not(target_os = "android"), get("/api/profile"))]
pub async fn load_profile() -> Result<AppProfile, ServerFnError> {
    load_native_profile()
}

#[cfg_attr(not(target_os = "android"), post("/api/profile"))]
pub async fn save_profile(profile: AppProfile) -> Result<(), ServerFnError> {
    save_native_profile(&profile)
}

#[cfg_attr(not(target_os = "android"), post("/api/subscriptions/preview"))]
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

#[cfg_attr(not(target_os = "android"), post("/api/core/status"))]
pub async fn core_status() -> Result<ApiCoreStatus, ServerFnError> {
    native_core_status()
}

#[cfg_attr(not(target_os = "android"), post("/api/core/toggle"))]
pub async fn set_core_enabled(
    enabled: bool,
    request: Option<ConnectionRequest>,
) -> Result<ApiCoreStatus, ServerFnError> {
    toggle_native_core(enabled, request)
}

#[cfg_attr(not(target_os = "android"), get("/api/core/traffic"))]
pub async fn core_traffic() -> Result<CoreTraffic, ServerFnError> {
    native_core_traffic()
}

#[cfg_attr(not(target_os = "android"), post("/api/core/logs"))]
pub async fn core_logs(cursor: u64) -> Result<CoreLogBatch, ServerFnError> {
    native_core_logs(cursor)
}

#[cfg_attr(not(target_os = "android"), post("/api/core/latency"))]
pub async fn measure_node_latency(
    nodes: Vec<ProxyNode>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
    measure_native_latency(nodes).await
}

#[cfg_attr(not(target_os = "android"), get("/api/system-proxy/status"))]
pub async fn system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    native_system_proxy_status()
}

#[cfg_attr(not(target_os = "android"), post("/api/system-proxy"))]
pub async fn set_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    set_native_system_proxy(enabled)
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn measure_native_latency(nodes: Vec<ProxyNode>) -> Result<Vec<NodeLatency>, ServerFnError> {
    use futures_util::stream::{self, StreamExt};

    if nodes.is_empty() {
        return Err(ServerFnError::new("没有可探测的节点"));
    }
    if nodes.len() > MAX_LATENCY_NODES {
        return Err(ServerFnError::new(format!(
            "单次最多探测 {MAX_LATENCY_NODES} 个节点"
        )));
    }

    Ok(stream::iter(nodes)
        .map(measure_one_node)
        .buffered(4)
        .collect()
        .await)
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
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
    config["inbounds"] = serde_json::json!([]);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    let config = serde_json::to_string(&config)
        .map_err(|error| ServerFnError::new(format!("序列化 Android 探测配置失败: {error}")))?;
    tokio::task::spawn_blocking(move || {
        singbox::android::probe(&config, &node_tags, LATENCY_CHECK_URL)
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
    .map_err(|error| ServerFnError::new(format!("Android 探测任务失败: {error}")))?
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
async fn measure_native_latency(_nodes: Vec<ProxyNode>) -> Result<Vec<NodeLatency>, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接运行节点延迟探测"))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn measure_one_node(node: ProxyNode) -> NodeLatency {
    let tag = node.tag.clone();
    match probe_node_latency(node).await {
        Ok(latency_ms) => NodeLatency {
            tag,
            latency_ms: Some(latency_ms),
            error: None,
        },
        Err(error) => NodeLatency {
            tag,
            latency_ms: None,
            error: Some(error.to_string()),
        },
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn probe_node_latency(node: ProxyNode) -> Result<u64, ServerFnError> {
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| ServerFnError::new(format!("分配探测端口失败: {error}")))?;
    let port = listener
        .local_addr()
        .map_err(|error| ServerFnError::new(format!("读取探测端口失败: {error}")))?
        .port();
    drop(listener);

    let selected_tag = node.tag.clone();
    let request = ConnectionRequest {
        nodes: vec![node],
        selected_tag,
        mode: TunnelMode::Global,
        tun: false,
    };
    let options = SingBoxOptions {
        mixed_port: port,
        listen: "127.0.0.1".to_string(),
        log_level: "error".to_string(),
        traffic_api_port: None,
        traffic_api_secret: None,
    };
    let mut core =
        singbox::SingBox::new().map_err(|error| ServerFnError::new(error.to_string()))?;
    core.start(&request, &options)
        .map_err(|error| ServerFnError::new(error.to_string()))?;

    let probe = async {
        let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
            .map_err(|error| ServerFnError::new(format!("创建探测代理失败: {error}")))?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(proxy)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|error| ServerFnError::new(format!("创建探测客户端失败: {error}")))?;
        let started = Instant::now();
        client
            .get(LATENCY_CHECK_URL)
            .send()
            .await
            .map_err(|error| ServerFnError::new(format!("节点请求失败: {error}")))?
            .error_for_status()
            .map_err(|error| ServerFnError::new(format!("探测地址返回错误: {error}")))?;
        Ok::<u64, ServerFnError>(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
    }
    .await;

    let stop = core.stop();
    let latency_ms = probe?;
    stop.map_err(|error| ServerFnError::new(error.to_string()))?;
    Ok(latency_ms)
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

#[cfg(target_os = "macos")]
const SYSTEM_PROXY_HOST: &str = "127.0.0.1";
#[cfg(target_os = "macos")]
const SYSTEM_PROXY_PORT: u16 = 7890;

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

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "macos")))]
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

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "macos")))]
fn set_native_system_proxy(_enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    Err(ServerFnError::new("当前平台尚未实现系统代理适配"))
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn set_native_system_proxy(_enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接修改系统代理"))
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
fn system_proxy_backup_path() -> Result<std::path::PathBuf, ServerFnError> {
    let profile = profile_path()?;
    let parent = profile
        .parent()
        .ok_or_else(|| ServerFnError::new("本地配置目录无效"))?;
    Ok(parent.join("system-proxy-backup.json"))
}

#[cfg(target_os = "macos")]
fn write_system_proxy_backup(backup: &MacSystemProxyBackup) -> Result<(), ServerFnError> {
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

#[cfg(target_os = "macos")]
fn read_system_proxy_backup(path: &std::path::Path) -> Result<MacSystemProxyBackup, ServerFnError> {
    let bytes = std::fs::read(path)
        .map_err(|error| ServerFnError::new(format!("没有可恢复的系统代理备份: {error}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ServerFnError::new(format!("系统代理备份损坏: {error}")))
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
