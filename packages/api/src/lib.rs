//! Shared fullstack APIs used by web, desktop, and mobile shells.

use dioxus::prelude::*;
use proxy_core::{
    AppProfile, ConnectionRequest, ParseReport, ProxyGroup, ProxyNode, RuleSetCachePaths,
};
#[cfg(not(target_arch = "wasm32"))]
use proxy_core::{
    SingBoxOptions, TunnelMode, CHINA_GEOIP_RULE_SET_URL, CHINA_GEOSITE_RULE_SET_URL,
};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const MAX_SUBSCRIPTION_BYTES: usize = 10 * 1024 * 1024;

/// Kitty Pro's per-session safety limit; sing-box itself does not impose this limit.
pub const MAX_LATENCY_NODES: usize = 100;

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const LATENCY_CHECK_URL: &str = "https://www.gstatic.com/generate_204";

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const MAX_RULE_SET_BYTES: usize = 8 * 1024 * 1024;

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const RULE_SET_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const LATENCY_PROBE_STAGGER_MS: u64 = 10;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyProbeSnapshot {
    pub results: Vec<NodeLatency>,
    pub completed: usize,
    pub total: usize,
    pub done: bool,
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
    Block,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSetUpdateResult {
    pub updated: bool,
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
    run_native_blocking("读取本地配置任务失败", load_native_profile).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/profile")
)]
pub async fn save_profile(profile: AppProfile) -> Result<(), ServerFnError> {
    run_native_blocking("保存本地配置任务失败", move || {
        save_native_profile(&profile)
    })
    .await
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

    let content = if (source.starts_with("http://") || source.starts_with("https://"))
        && !proxy_core::is_http_proxy_share_link(&source)
    {
        download_subscription(&source).await?
    } else {
        source
    };
    run_native_blocking("解析订阅任务失败", move || {
        Ok(proxy_core::parse_subscription(&content))
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/status")
)]
pub async fn core_status() -> Result<ApiCoreStatus, ServerFnError> {
    run_native_blocking("读取 sing-box 状态任务失败", native_core_status).await
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
    let rule_set_cache = if enabled {
        prepare_rule_set_cache_for_request(request.as_ref()).await?
    } else {
        None
    };
    run_native_blocking("sing-box 状态切换任务失败", move || {
        toggle_native_core(enabled, request, rule_set_cache)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/restart")
)]
pub async fn restart_core(request: ConnectionRequest) -> Result<ApiCoreStatus, ServerFnError> {
    if request.nodes.is_empty() {
        return Err(ServerFnError::new("请先导入并选择一个节点"));
    }
    proxy_core::validate_custom_rules(&request.custom_rules)
        .map_err(|error| ServerFnError::new(format!("自定义规则无效: {error}")))?;
    let rule_set_cache = prepare_rule_set_cache_for_request(Some(&request)).await?;
    run_native_blocking("sing-box 重启任务失败", move || {
        toggle_native_core(false, None, None)?;
        toggle_native_core(true, Some(request), rule_set_cache)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/rules/update")
)]
pub async fn update_rule_sets(force: bool) -> Result<RuleSetUpdateResult, ServerFnError> {
    native_update_rule_sets(force).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/config-script/validate")
)]
pub async fn validate_config_script(request: ConnectionRequest) -> Result<(), ServerFnError> {
    run_native_blocking("校验配置脚本任务失败", move || {
        validate_native_config_script(request)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/proxy-groups/preview")
)]
pub async fn preview_proxy_groups(
    request: ConnectionRequest,
) -> Result<Vec<ProxyGroup>, ServerFnError> {
    run_native_blocking("生成代理组预览任务失败", move || {
        preview_native_proxy_groups(request)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/proxy-groups/select")
)]
pub async fn select_proxy_group(group: String, outbound: String) -> Result<(), ServerFnError> {
    run_native_blocking("切换代理组任务失败", move || {
        select_native_proxy_group(group, outbound)
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
    run_native_blocking("读取 sing-box 流量任务失败", native_core_traffic).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/logs")
)]
pub async fn core_logs(cursor: u64) -> Result<CoreLogBatch, ServerFnError> {
    run_native_blocking("读取 sing-box 日志任务失败", move || {
        native_core_logs(cursor)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/logs/collection")
)]
pub async fn set_core_log_collection(enabled: bool) -> Result<(), ServerFnError> {
    run_native_blocking("切换 sing-box 日志采集任务失败", move || {
        native_set_core_log_collection(enabled)
    })
    .await
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
    post("/api/core/latency/start")
)]
pub async fn start_node_latency(nodes: Vec<ProxyNode>) -> Result<u64, ServerFnError> {
    start_native_latency_session(nodes).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/latency/poll")
)]
pub async fn poll_node_latency(session_id: u64) -> Result<LatencyProbeSnapshot, ServerFnError> {
    run_native_blocking("读取节点测速任务失败", move || {
        poll_native_latency_session(session_id)
    })
    .await
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
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
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
        custom_rules: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let probe_options = SingBoxOptions {
        log_level: "error".to_string(),
        ..SingBoxOptions::default()
    };
    let mut config = proxy_core::build_singbox_config(&request, &probe_options);
    config["inbounds"] = serde_json::json!([]);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    tokio::task::spawn_blocking(move || {
        singbox::SingBox::probe_config(&config, &node_tags, LATENCY_CHECK_URL)
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
        custom_rules: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let probe_options = SingBoxOptions {
        log_level: "error".to_string(),
        ..SingBoxOptions::default()
    };
    let mut config = proxy_core::build_singbox_config(&request, &probe_options);
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

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn start_native_latency_session(nodes: Vec<ProxyNode>) -> Result<u64, ServerFnError> {
    validate_latency_nodes(&nodes)?;
    let node_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let request = ConnectionRequest {
        nodes,
        selected_tag: node_tags.first().cloned().unwrap_or_default(),
        mode: TunnelMode::Global,
        tun: false,
        custom_rules: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let probe_options = SingBoxOptions {
        log_level: "error".to_string(),
        ..SingBoxOptions::default()
    };
    let mut config = proxy_core::build_singbox_config(&request, &probe_options);
    config["inbounds"] = serde_json::json!([]);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    let core = tokio::task::spawn_blocking(move || {
        let mut core = singbox::SingBox::new().map_err(|error| error.to_string())?;
        core.start_config(&config)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(core)
    })
    .await
    .map_err(|error| ServerFnError::new(format!("启动节点探测任务失败: {error}")))?
    .map_err(ServerFnError::new)?;

    let (session_id, session) = register_latency_session(node_tags.len())?;
    let core = std::sync::Arc::new(core);
    tokio::spawn(async move {
        let mut tasks = tokio::task::JoinSet::new();
        for (index, tag) in node_tags.into_iter().enumerate() {
            let core = std::sync::Arc::clone(&core);
            tasks.spawn(async move {
                if index > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        index as u64 * LATENCY_PROBE_STAGGER_MS,
                    ))
                    .await;
                }
                let fallback_tag = tag.clone();
                match tokio::task::spawn_blocking(move || {
                    core.probe_outbound(&tag, LATENCY_CHECK_URL)
                })
                .await
                {
                    Ok(Ok(result)) => NodeLatency {
                        tag: result.tag,
                        latency_ms: result.latency_ms,
                        error: result.error,
                    },
                    Ok(Err(error)) => NodeLatency {
                        tag: fallback_tag,
                        latency_ms: None,
                        error: Some(error.to_string()),
                    },
                    Err(error) => NodeLatency {
                        tag: fallback_tag,
                        latency_ms: None,
                        error: Some(format!("节点探测任务失败: {error}")),
                    },
                }
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok(result) = result {
                session.push(result);
            }
        }
        session.finish();
    });
    Ok(session_id)
}

#[cfg(target_os = "android")]
async fn start_native_latency_session(nodes: Vec<ProxyNode>) -> Result<u64, ServerFnError> {
    validate_latency_nodes(&nodes)?;
    let total = nodes.len();
    let fallback_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let (session_id, session) = register_latency_session(total)?;
    tokio::spawn(async move {
        let results = measure_native_latency(nodes).await.unwrap_or_else(|error| {
            fallback_tags
                .into_iter()
                .map(|tag| NodeLatency {
                    tag,
                    latency_ms: None,
                    error: Some(error.to_string()),
                })
                .collect()
        });
        for result in results {
            session.push(result);
        }
        session.finish();
    });
    Ok(session_id)
}

#[cfg(target_arch = "wasm32")]
async fn start_native_latency_session(_nodes: Vec<ProxyNode>) -> Result<u64, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接启动节点探测"))
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_latency_nodes(nodes: &[ProxyNode]) -> Result<(), ServerFnError> {
    if nodes.is_empty() {
        return Err(ServerFnError::new("没有可探测的节点"));
    }
    if nodes.len() > MAX_LATENCY_NODES {
        return Err(ServerFnError::new(format!(
            "单次最多探测 {MAX_LATENCY_NODES} 个节点"
        )));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
struct LatencySession {
    results: std::sync::Mutex<Vec<NodeLatency>>,
    completed: std::sync::atomic::AtomicUsize,
    total: usize,
    done: std::sync::atomic::AtomicBool,
}

#[cfg(not(target_arch = "wasm32"))]
impl LatencySession {
    fn push(&self, result: NodeLatency) {
        if let Ok(mut results) = self.results.lock() {
            results.push(result);
        }
        self.completed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn finish(&self) {
        self.done.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn register_latency_session(
    total: usize,
) -> Result<(u64, std::sync::Arc<LatencySession>), ServerFnError> {
    static NEXT_SESSION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let session_id = NEXT_SESSION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let session = std::sync::Arc::new(LatencySession {
        results: std::sync::Mutex::new(Vec::new()),
        completed: std::sync::atomic::AtomicUsize::new(0),
        total,
        done: std::sync::atomic::AtomicBool::new(false),
    });
    latency_sessions()
        .lock()
        .map_err(|_| ServerFnError::new("节点探测会话锁已损坏"))?
        .insert(session_id, std::sync::Arc::clone(&session));
    Ok((session_id, session))
}

#[cfg(not(target_arch = "wasm32"))]
fn poll_native_latency_session(session_id: u64) -> Result<LatencyProbeSnapshot, ServerFnError> {
    let session = latency_sessions()
        .lock()
        .map_err(|_| ServerFnError::new("节点探测会话锁已损坏"))?
        .get(&session_id)
        .cloned()
        .ok_or_else(|| ServerFnError::new("节点探测会话不存在或已结束"))?;
    let results = {
        let mut results = session
            .results
            .lock()
            .map_err(|_| ServerFnError::new("节点探测结果锁已损坏"))?;
        std::mem::take(&mut *results)
    };
    let done = session.done.load(std::sync::atomic::Ordering::Acquire);
    let snapshot = LatencyProbeSnapshot {
        results,
        completed: session.completed.load(std::sync::atomic::Ordering::Relaxed),
        total: session.total,
        done,
    };
    if done {
        latency_sessions()
            .lock()
            .map_err(|_| ServerFnError::new("节点探测会话锁已损坏"))?
            .remove(&session_id);
    }
    Ok(snapshot)
}

#[cfg(target_arch = "wasm32")]
fn poll_native_latency_session(_session_id: u64) -> Result<LatencyProbeSnapshot, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接读取节点探测"))
}

#[cfg(not(target_arch = "wasm32"))]
fn latency_sessions(
) -> &'static std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<LatencySession>>> {
    static SESSIONS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<LatencySession>>>,
    > = std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
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

#[cfg(not(target_arch = "wasm32"))]
fn rule_set_update_lock() -> &'static tokio::sync::Mutex<()> {
    use std::sync::OnceLock;

    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[cfg(not(target_arch = "wasm32"))]
async fn prepare_rule_set_cache_for_request(
    request: Option<&ConnectionRequest>,
) -> Result<Option<RuleSetCachePaths>, ServerFnError> {
    if request.map(|request| request.mode) != Some(TunnelMode::Rule) {
        return Ok(None);
    }
    let paths = native_rule_set_cache_paths()?;
    let check_paths = paths.clone();
    if run_native_blocking("检查规则缓存失败", move || {
        native_rule_set_cache_ready(&check_paths)
    })
    .await?
    {
        return Ok(Some(paths));
    }

    let _guard = rule_set_update_lock().lock().await;
    let check_paths = paths.clone();
    if !run_native_blocking("检查规则缓存失败", move || {
        native_rule_set_cache_ready(&check_paths)
    })
    .await?
    {
        download_and_install_rule_sets(paths.clone()).await?;
    }
    Ok(Some(paths))
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
async fn prepare_rule_set_cache_for_request(
    _request: Option<&ConnectionRequest>,
) -> Result<Option<RuleSetCachePaths>, ServerFnError> {
    Ok(None)
}

#[cfg(not(target_arch = "wasm32"))]
async fn native_update_rule_sets(force: bool) -> Result<RuleSetUpdateResult, ServerFnError> {
    let _guard = rule_set_update_lock().lock().await;
    let paths = native_rule_set_cache_paths()?;
    if !force {
        let check_paths = paths.clone();
        let current = run_native_blocking("检查规则缓存失败", move || {
            native_rule_set_cache_current(&check_paths)
        })
        .await?;
        if current {
            return Ok(RuleSetUpdateResult { updated: false });
        }
    }
    download_and_install_rule_sets(paths).await?;
    Ok(RuleSetUpdateResult { updated: true })
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
async fn native_update_rule_sets(_force: bool) -> Result<RuleSetUpdateResult, ServerFnError> {
    Err(ServerFnError::new("浏览器端不能直接更新分流规则"))
}

#[cfg(not(target_arch = "wasm32"))]
async fn download_and_install_rule_sets(paths: RuleSetCachePaths) -> Result<(), ServerFnError> {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("kitty-pro/0.1")
        .build()
        .map_err(|error| ServerFnError::new(format!("创建规则下载客户端失败: {error}")))?;
    let geosite_download =
        download_rule_set(client.clone(), CHINA_GEOSITE_RULE_SET_URL, "域名分流规则");
    let geoip_download = download_rule_set(client, CHINA_GEOIP_RULE_SET_URL, "IP 分流规则");
    let (geosite, geoip) = tokio::try_join!(geosite_download, geoip_download)?;
    run_native_blocking("保存规则缓存失败", move || {
        install_rule_set_cache(&paths, &geosite, &geoip)
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
async fn download_rule_set(
    client: reqwest::Client,
    url: &'static str,
    label: &'static str,
) -> Result<Vec<u8>, ServerFnError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| ServerFnError::new(format!("下载{label}失败: {error}")))?
        .error_for_status()
        .map_err(|error| ServerFnError::new(format!("{label}服务返回错误: {error}")))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RULE_SET_BYTES as u64)
    {
        return Err(ServerFnError::new(format!("{label}超过 8 MiB 限制")));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ServerFnError::new(format!("读取{label}失败: {error}")))?;
    if bytes.is_empty() || bytes.len() > MAX_RULE_SET_BYTES {
        return Err(ServerFnError::new(format!("{label}大小无效")));
    }
    Ok(bytes.to_vec())
}

#[cfg(not(target_arch = "wasm32"))]
fn native_rule_set_cache_paths() -> Result<RuleSetCachePaths, ServerFnError> {
    let profile = profile_path()?;
    let directory = profile
        .parent()
        .ok_or_else(|| ServerFnError::new("无法确定规则缓存目录"))?
        .join("rules");
    let geosite = directory.join("geosite-cn.srs");
    let geoip = directory.join("geoip-cn.srs");
    Ok(RuleSetCachePaths {
        geosite: geosite
            .to_str()
            .ok_or_else(|| ServerFnError::new("域名规则缓存路径不是有效 UTF-8"))?
            .to_string(),
        geoip: geoip
            .to_str()
            .ok_or_else(|| ServerFnError::new("IP 规则缓存路径不是有效 UTF-8"))?
            .to_string(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn native_rule_set_cache_ready(paths: &RuleSetCachePaths) -> Result<bool, ServerFnError> {
    let geosite = std::path::Path::new(&paths.geosite);
    let geoip = std::path::Path::new(&paths.geoip);
    recover_rule_set_transaction(geosite, geoip)?;
    if !geosite.is_file() || !geoip.is_file() {
        return Ok(false);
    }
    Ok(singbox::validate_rule_set_file(geosite).is_ok()
        && singbox::validate_rule_set_file(geoip).is_ok())
}

#[cfg(not(target_arch = "wasm32"))]
fn native_rule_set_cache_current(paths: &RuleSetCachePaths) -> Result<bool, ServerFnError> {
    if !native_rule_set_cache_ready(paths)? {
        return Ok(false);
    }
    for path in [&paths.geosite, &paths.geoip] {
        let age = std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
            .map_err(|error| ServerFnError::new(format!("读取规则缓存时间失败: {error}")))?;
        if age > RULE_SET_CACHE_TTL {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(not(target_arch = "wasm32"))]
fn install_rule_set_cache(
    paths: &RuleSetCachePaths,
    geosite_content: &[u8],
    geoip_content: &[u8],
) -> Result<(), ServerFnError> {
    use std::io::Write;

    let geosite = std::path::Path::new(&paths.geosite);
    let geoip = std::path::Path::new(&paths.geoip);
    let directory = geosite
        .parent()
        .ok_or_else(|| ServerFnError::new("域名规则缓存目录无效"))?;
    if geoip.parent() != Some(directory) {
        return Err(ServerFnError::new("规则缓存文件不在同一目录"));
    }
    std::fs::create_dir_all(directory)
        .map_err(|error| ServerFnError::new(format!("创建规则缓存目录失败: {error}")))?;
    recover_rule_set_transaction(geosite, geoip)?;

    let geosite_temp = directory.join(".geosite-cn.pending.tmp");
    let geoip_temp = directory.join(".geoip-cn.pending.tmp");
    let write_result = (|| -> std::io::Result<()> {
        for (path, content) in [
            (geosite_temp.as_path(), geosite_content),
            (geoip_temp.as_path(), geoip_content),
        ] {
            let mut file = create_private_file(path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        return Err(ServerFnError::new(format!("保存规则临时文件失败: {error}")));
    }
    if let Err(error) = singbox::validate_rule_set_file(&geosite_temp)
        .and_then(|_| singbox::validate_rule_set_file(&geoip_temp))
    {
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        return Err(ServerFnError::new(error.to_string()));
    }

    let geosite_backup = rule_set_backup_path(geosite);
    let geoip_backup = rule_set_backup_path(geoip);
    let geosite_existed = geosite.exists();
    let geoip_existed = geoip.exists();
    let transaction = rule_set_transaction_path(directory);
    let committed = rule_set_commit_path(directory);
    let state = RuleSetInstallTransaction {
        geosite_existed,
        geoip_existed,
    };
    if let Err(error) = write_rule_set_transaction(&transaction, &state) {
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        return Err(ServerFnError::new(format!("创建规则更新事务失败: {error}")));
    }

    if let Err(error) = stage_rule_set_backup(geosite, &geosite_backup)
        .and_then(|_| stage_rule_set_backup(geoip, &geoip_backup))
        .and_then(|_| std::fs::rename(&geosite_temp, geosite))
        .and_then(|_| std::fs::rename(&geoip_temp, geoip))
        .and_then(|_| write_rule_set_commit(&committed))
    {
        let rollback_result = rollback_rule_set_pair(geosite, geoip, &state);
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        if rollback_result.is_ok() {
            let _ = std::fs::remove_file(&transaction);
            let _ = std::fs::remove_file(&committed);
        }
        if let Err(rollback_error) = rollback_result {
            return Err(ServerFnError::new(format!(
                "替换规则缓存失败: {error}；回滚失败: {rollback_error}"
            )));
        }
        return Err(ServerFnError::new(format!("替换规则缓存失败: {error}")));
    }
    finish_rule_set_transaction(geosite, geoip)
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Serialize, Deserialize)]
struct RuleSetInstallTransaction {
    geosite_existed: bool,
    geoip_existed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn rule_set_backup_path(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension("srs.backup")
}

#[cfg(not(target_arch = "wasm32"))]
fn rule_set_transaction_path(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join(".rule-set-install.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn rule_set_commit_path(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join(".rule-set-install.committed")
}

#[cfg(not(target_arch = "wasm32"))]
fn write_rule_set_transaction(
    path: &std::path::Path,
    state: &RuleSetInstallTransaction,
) -> std::io::Result<()> {
    use std::io::Write;

    let content = serde_json::to_vec(state).map_err(std::io::Error::other)?;
    let mut file = create_private_file(path)?;
    file.write_all(&content)?;
    file.sync_all()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_rule_set_commit(path: &std::path::Path) -> std::io::Result<()> {
    use std::io::Write;

    let mut file = create_private_file(path)?;
    file.write_all(b"committed")?;
    file.sync_all()
}

#[cfg(not(target_arch = "wasm32"))]
fn stage_rule_set_backup(path: &std::path::Path, backup: &std::path::Path) -> std::io::Result<()> {
    if path.exists() {
        std::fs::rename(path, backup)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn restore_rule_set_backup(
    path: &std::path::Path,
    backup: &std::path::Path,
) -> std::io::Result<()> {
    if backup.exists() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(backup, path)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn rollback_rule_set_install(
    path: &std::path::Path,
    backup: &std::path::Path,
    existed: bool,
) -> std::io::Result<()> {
    if backup.exists() {
        return restore_rule_set_backup(path, backup);
    }
    if !existed && path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn rollback_rule_set_pair(
    geosite: &std::path::Path,
    geoip: &std::path::Path,
    state: &RuleSetInstallTransaction,
) -> std::io::Result<()> {
    let geosite_result = rollback_rule_set_install(
        geosite,
        &rule_set_backup_path(geosite),
        state.geosite_existed,
    );
    let geoip_result =
        rollback_rule_set_install(geoip, &rule_set_backup_path(geoip), state.geoip_existed);
    geosite_result.and(geoip_result)
}

#[cfg(not(target_arch = "wasm32"))]
fn finish_rule_set_transaction(
    geosite: &std::path::Path,
    geoip: &std::path::Path,
) -> Result<(), ServerFnError> {
    let directory = geosite
        .parent()
        .ok_or_else(|| ServerFnError::new("域名规则缓存目录无效"))?;
    for path in [rule_set_backup_path(geosite), rule_set_backup_path(geoip)] {
        if path.exists() {
            std::fs::remove_file(path)
                .map_err(|error| ServerFnError::new(format!("清理旧规则缓存备份失败: {error}")))?;
        }
    }
    let transaction = rule_set_transaction_path(directory);
    if transaction.exists() {
        std::fs::remove_file(&transaction)
            .map_err(|error| ServerFnError::new(format!("清理规则更新事务失败: {error}")))?;
    }
    let committed = rule_set_commit_path(directory);
    if committed.exists() {
        std::fs::remove_file(committed)
            .map_err(|error| ServerFnError::new(format!("完成规则更新事务失败: {error}")))?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn recover_rule_set_transaction(
    geosite: &std::path::Path,
    geoip: &std::path::Path,
) -> Result<(), ServerFnError> {
    let directory = geosite
        .parent()
        .ok_or_else(|| ServerFnError::new("域名规则缓存目录无效"))?;
    if geoip.parent() != Some(directory) {
        return Err(ServerFnError::new("规则缓存文件不在同一目录"));
    }
    let transaction = rule_set_transaction_path(directory);
    let committed = rule_set_commit_path(directory);
    if transaction.exists() {
        if committed.exists() {
            return finish_rule_set_transaction(geosite, geoip);
        }
        let content = std::fs::read(&transaction)
            .map_err(|error| ServerFnError::new(format!("读取规则更新事务失败: {error}")))?;
        let state: RuleSetInstallTransaction = match serde_json::from_slice(&content) {
            Ok(state) => state,
            Err(error)
                if !rule_set_backup_path(geosite).exists()
                    && !rule_set_backup_path(geoip).exists() =>
            {
                std::fs::remove_file(&transaction).map_err(|remove_error| {
                    ServerFnError::new(format!(
                        "规则更新事务损坏: {error}；清理失败: {remove_error}"
                    ))
                })?;
                return Ok(());
            }
            Err(error) => {
                return Err(ServerFnError::new(format!("规则更新事务损坏: {error}")));
            }
        };
        rollback_rule_set_pair(geosite, geoip, &state)
            .map_err(|error| ServerFnError::new(format!("恢复规则缓存失败: {error}")))?;
        let _ = std::fs::remove_file(directory.join(".geosite-cn.pending.tmp"));
        let _ = std::fs::remove_file(directory.join(".geoip-cn.pending.tmp"));
        std::fs::remove_file(&transaction)
            .map_err(|error| ServerFnError::new(format!("清理规则更新事务失败: {error}")))?;
    } else {
        // Compatibility with caches written before pair transactions were introduced.
        recover_rule_set_backup(geosite)?;
        recover_rule_set_backup(geoip)?;
    }
    if committed.exists() {
        std::fs::remove_file(committed)
            .map_err(|error| ServerFnError::new(format!("清理规则提交标记失败: {error}")))?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn recover_rule_set_backup(path: &std::path::Path) -> Result<(), ServerFnError> {
    let backup = rule_set_backup_path(path);
    if !backup.exists() {
        return Ok(());
    }
    if path.exists() && singbox::validate_rule_set_file(path).is_ok() {
        std::fs::remove_file(backup)
            .map_err(|error| ServerFnError::new(format!("清理旧规则备份失败: {error}")))?;
        return Ok(());
    }
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| ServerFnError::new(format!("移除损坏规则缓存失败: {error}")))?;
    }
    std::fs::rename(backup, path)
        .map_err(|error| ServerFnError::new(format!("恢复规则缓存失败: {error}")))
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
#[cfg(not(target_arch = "wasm32"))]
fn validate_native_config_script(request: ConnectionRequest) -> Result<(), ServerFnError> {
    if request.config_script.is_none() {
        return Err(ServerFnError::new("脚本内容为空"));
    }
    build_native_config(&request, &SingBoxOptions::default()).map(|_| ())
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn validate_native_config_script(_request: ConnectionRequest) -> Result<(), ServerFnError> {
    Err(ServerFnError::new("浏览器端不能直接执行配置脚本"))
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
fn preview_native_proxy_groups(
    request: ConnectionRequest,
) -> Result<Vec<ProxyGroup>, ServerFnError> {
    let config = build_native_config(&request, &SingBoxOptions::default())?;
    Ok(proxy_core::extract_proxy_groups(&config))
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn select_native_proxy_group(group: String, outbound: String) -> Result<(), ServerFnError> {
    if group.trim().is_empty() || outbound.trim().is_empty() {
        return Err(ServerFnError::new("代理组和节点不能为空"));
    }
    let guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    let core = guard
        .as_ref()
        .ok_or_else(|| ServerFnError::new("sing-box 尚未启动"))?;
    core.select_outbound(&group, &outbound)
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
fn select_native_proxy_group(group: String, outbound: String) -> Result<(), ServerFnError> {
    if group.trim().is_empty() || outbound.trim().is_empty() {
        return Err(ServerFnError::new("代理组和节点不能为空"));
    }
    singbox::android::select_outbound(&group, &outbound)
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn select_native_proxy_group(_group: String, _outbound: String) -> Result<(), ServerFnError> {
    Err(ServerFnError::new("浏览器端不能直接切换代理组"))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn preview_native_proxy_groups(
    _request: ConnectionRequest,
) -> Result<Vec<ProxyGroup>, ServerFnError> {
    Err(ServerFnError::new("浏览器端不能直接生成代理组"))
}

#[cfg(not(target_arch = "wasm32"))]
fn build_native_config(
    request: &ConnectionRequest,
    options: &SingBoxOptions,
) -> Result<serde_json::Value, ServerFnError> {
    let mut config = proxy_core::build_singbox_config(request, options);
    if let Some(script) = request.config_script.as_deref() {
        let node_names = request
            .nodes
            .iter()
            .map(|node| {
                (
                    node.tag.clone(),
                    serde_json::Value::String(node.name.clone()),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        config["__kitty_context"] = serde_json::json!({ "node_names": node_names });
        config = proxy_core::apply_config_script(script, config)
            .map_err(|error| ServerFnError::new(error.to_string()))?;
        if let Some(config) = config.as_object_mut() {
            config.remove("__kitty_context");
        }
    }
    proxy_core::apply_proxy_group_selections(&mut config, &request.group_selections);
    Ok(config)
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
    rule_set_cache: Option<RuleSetCachePaths>,
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
        rule_set_cache,
        ..SingBoxOptions::default()
    };
    let config = build_native_config(&request, &options)?;
    core.start_config(&config)
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
    rule_set_cache: Option<RuleSetCachePaths>,
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
    proxy_core::validate_custom_rules(&request.custom_rules)
        .map_err(|error| ServerFnError::new(format!("自定义规则无效: {error}")))?;
    let options = SingBoxOptions {
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        ..SingBoxOptions::default()
    };
    let mut config = build_native_config(&request, &options)?;
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

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn native_set_core_log_collection(enabled: bool) -> Result<(), ServerFnError> {
    let guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    let Some(core) = guard.as_ref() else {
        return Ok(());
    };
    if !core
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        return Ok(());
    }
    core.set_log_enabled(enabled)
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
fn native_set_core_log_collection(enabled: bool) -> Result<(), ServerFnError> {
    singbox::android::set_log_enabled(enabled)
        .map_err(|error| ServerFnError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
fn native_set_core_log_collection(_enabled: bool) -> Result<(), ServerFnError> {
    Ok(())
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
        } else if outbound_type == "block" || outbound_tag == "block" {
            RouteDecision::Block
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
    _rule_set_cache: Option<RuleSetCachePaths>,
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

    #[cfg(not(target_arch = "wasm32"))]
    struct TestDirectory(std::path::PathBuf);

    #[cfg(not(target_arch = "wasm32"))]
    impl TestDirectory {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};

            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "kitty-pro-{label}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("test directory should be created");
            Self(path)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_config_applies_javascript_transform() {
        let nodes = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@example.com:443#香港节点",
        )
        .nodes;
        let selected_tag = nodes[0].tag.clone();
        let request = ConnectionRequest {
            nodes,
            selected_tag,
            mode: TunnelMode::Rule,
            tun: false,
            custom_rules: Vec::new(),
            config_script: Some(
                "function main(config) {\
                    config.log.level = Object.values(config.__kitty_context.node_names)[0];\
                    return config;\
                }"
                .to_string(),
            ),
            group_selections: Default::default(),
        };

        let config = build_native_config(&request, &SingBoxOptions::default())
            .expect("script should be applied before startup");

        assert_eq!(config["log"]["level"], "香港节点");
        assert!(config.get("__kitty_context").is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn unfinished_rule_set_transaction_restores_both_old_files() {
        let directory = TestDirectory::new("rule-set-rollback");
        let geosite = directory.0.join("geosite-cn.srs");
        let geoip = directory.0.join("geoip-cn.srs");
        std::fs::write(&geosite, b"old geosite").expect("old geosite should be written");
        std::fs::write(&geoip, b"old geoip").expect("old geoip should be written");
        std::fs::rename(&geosite, rule_set_backup_path(&geosite))
            .expect("geosite backup should be staged");
        std::fs::rename(&geoip, rule_set_backup_path(&geoip))
            .expect("geoip backup should be staged");
        std::fs::write(&geosite, b"new geosite").expect("new geosite should be installed");
        std::fs::write(&geoip, b"new geoip").expect("new geoip should be installed");
        write_rule_set_transaction(
            &rule_set_transaction_path(&directory.0),
            &RuleSetInstallTransaction {
                geosite_existed: true,
                geoip_existed: true,
            },
        )
        .expect("transaction should be written");

        recover_rule_set_transaction(&geosite, &geoip)
            .expect("unfinished transaction should roll back");

        assert_eq!(std::fs::read(&geosite).unwrap(), b"old geosite");
        assert_eq!(std::fs::read(&geoip).unwrap(), b"old geoip");
        assert!(!rule_set_backup_path(&geosite).exists());
        assert!(!rule_set_backup_path(&geoip).exists());
        assert!(!rule_set_transaction_path(&directory.0).exists());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn committed_rule_set_transaction_keeps_both_new_files() {
        let directory = TestDirectory::new("rule-set-commit");
        let geosite = directory.0.join("geosite-cn.srs");
        let geoip = directory.0.join("geoip-cn.srs");
        std::fs::write(rule_set_backup_path(&geosite), b"old geosite")
            .expect("geosite backup should be written");
        std::fs::write(rule_set_backup_path(&geoip), b"old geoip")
            .expect("geoip backup should be written");
        std::fs::write(&geosite, b"new geosite").expect("new geosite should be installed");
        std::fs::write(&geoip, b"new geoip").expect("new geoip should be installed");
        write_rule_set_transaction(
            &rule_set_transaction_path(&directory.0),
            &RuleSetInstallTransaction {
                geosite_existed: true,
                geoip_existed: true,
            },
        )
        .expect("transaction should be written");
        write_rule_set_commit(&rule_set_commit_path(&directory.0))
            .expect("commit marker should be written");

        recover_rule_set_transaction(&geosite, &geoip)
            .expect("committed transaction should finish cleanup");

        assert_eq!(std::fs::read(&geosite).unwrap(), b"new geosite");
        assert_eq!(std::fs::read(&geoip).unwrap(), b"new geoip");
        assert!(!rule_set_backup_path(&geosite).exists());
        assert!(!rule_set_backup_path(&geoip).exists());
        assert!(!rule_set_transaction_path(&directory.0).exists());
        assert!(!rule_set_commit_path(&directory.0).exists());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn latency_session_returns_only_incremental_results() {
        let (session_id, session) = register_latency_session(2).expect("session should register");
        session.push(NodeLatency {
            tag: "fast".to_string(),
            latency_ms: Some(42),
            error: None,
        });

        let first = poll_native_latency_session(session_id).expect("first poll should succeed");
        assert_eq!(first.completed, 1);
        assert_eq!(first.results.len(), 1);
        assert!(!first.done);

        let second = poll_native_latency_session(session_id).expect("second poll should succeed");
        assert!(second.results.is_empty());
        assert_eq!(second.completed, 1);

        session.push(NodeLatency {
            tag: "slow".to_string(),
            latency_ms: None,
            error: Some("timeout".to_string()),
        });
        session.finish();
        let final_snapshot =
            poll_native_latency_session(session_id).expect("final poll should succeed");
        assert_eq!(final_snapshot.completed, 2);
        assert_eq!(final_snapshot.results.len(), 1);
        assert!(final_snapshot.done);
        assert!(poll_native_latency_session(session_id).is_err());
    }

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

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "server")))]
    #[test]
    fn native_http_proxy_preview_is_not_downloaded_as_a_subscription() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("native test runtime should start");
        let report = runtime
            .block_on(preview_subscription(
                "http://100.64.0.2:11080#Company".to_string(),
            ))
            .expect("HTTP proxy URI should parse locally");

        assert_eq!(report.nodes.len(), 1);
        assert_eq!(report.nodes[0].protocol, proxy_core::ProxyProtocol::Http);
        assert_eq!(report.nodes[0].server, "100.64.0.2");
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
        if std::env::var("KITTY_TEST_REQUIRE_SUCCESS").as_deref() == Ok("1") {
            assert!(
                results.iter().any(|result| result.latency_ms.is_some()),
                "live probe returned no successful nodes: {results:?}"
            );
        }
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
    fn parses_block_route_log() {
        let route = parse_route_log(
            "INFO[0004] outbound/block[block]: outbound connection to ads.example.com:443",
        )
        .expect("block route should be parsed");

        assert_eq!(route.decision, RouteDecision::Block);
        assert_eq!(route.outbound_tag, "block");
    }

    #[test]
    fn ignores_non_outbound_log_lines() {
        assert!(parse_route_log(
            "INFO[0001] inbound/mixed[mixed-in]: inbound connection to example.com:443"
        )
        .is_none());
    }
}
