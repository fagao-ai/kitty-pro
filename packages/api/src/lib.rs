//! Shared fullstack APIs used by web, desktop, and mobile shells.

use dioxus::prelude::*;
use proxy_core::{
    AppProfile, ConnectionRequest, ParseReport, ProxyGroup, ProxyNode, RuleSetCachePaths,
    SyncSnapshot,
};

#[cfg(target_os = "macos")]
mod macos_route;
#[cfg(not(target_arch = "wasm32"))]
use proxy_core::{
    SingBoxOptions, TunnelMode, CHINA_GEOIP_RULE_SET_URL, CHINA_GEOSITE_RULE_SET_URL,
};
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
mod sync;
#[cfg(not(target_arch = "wasm32"))]
use sync::{load_native_sync_config, save_native_sync_config, save_native_sync_state};

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

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const MAX_CONCURRENT_LATENCY_PROBES: usize = 8;

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
    #[serde(default)]
    pub outbound_chain: Vec<String>,
    #[serde(default)]
    pub source_ip: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningProcess {
    pub pid: u32,
    pub name: String,
    pub exe_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncProviderKind {
    WebDav,
    S3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default)]
    pub enabled: bool,
    pub provider: SyncProviderKind,
    /// WebDAV base URL or S3-compatible endpoint.
    #[serde(default)]
    pub endpoint: String,
    /// Remote object path. For S3 this is the object key.
    #[serde(default = "default_sync_path")]
    pub path: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default = "default_sync_region")]
    pub region: String,
    #[serde(default)]
    pub access_key: String,
    #[serde(default)]
    pub secret_key: String,
    /// Timestamp of the last successful remote read/write, kept locally.
    #[serde(default)]
    pub last_sync_at: u64,
    /// Opaque ETag returned by the current remote target, kept locally.
    #[serde(default)]
    pub last_remote_revision: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: SyncProviderKind::WebDav,
            endpoint: String::new(),
            path: default_sync_path(),
            username: String::new(),
            password: String::new(),
            bucket: String::new(),
            region: default_sync_region(),
            access_key: String::new(),
            secret_key: String::new(),
            last_sync_at: 0,
            last_remote_revision: String::new(),
        }
    }
}

fn default_sync_path() -> String {
    "kitty-pro-sync.json".to_string()
}

fn default_sync_region() -> String {
    "us-east-1".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncResult {
    pub updated_at: u64,
    pub subscription_count: usize,
    pub rule_count: usize,
    #[serde(default)]
    pub remote_revision: String,
    #[serde(default)]
    pub checkpoint_saved: bool,
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPullResult {
    pub snapshot: SyncSnapshot,
    #[serde(default)]
    pub remote_revision: String,
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
    get("/api/sync/config")
)]
pub async fn load_sync_config() -> Result<SyncConfig, ServerFnError> {
    run_native_blocking("读取同步配置任务失败", load_native_sync_config).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/sync/config")
)]
pub async fn save_sync_config(config: SyncConfig) -> Result<SyncConfig, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        run_native_blocking("保存同步配置任务失败", move || {
            save_native_sync_config(&config)
        })
        .await
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = config;
        Err(ServerFnError::new("浏览器端不能直接写入同步配置"))
    }
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/sync/push")
)]
pub async fn sync_push(
    config: SyncConfig,
    snapshot: SyncSnapshot,
    force: bool,
) -> Result<SyncResult, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        let config = save_native_sync_config(&config)?;
        let mut result = sync::push(&config, snapshot, force).await?;
        let mut saved = config;
        saved.last_sync_at = result.updated_at;
        saved.last_remote_revision = result.remote_revision.clone();
        match save_native_sync_state(&saved) {
            Ok(()) => result.checkpoint_saved = true,
            Err(error) => {
                append_sync_warning(
                    &mut result,
                    format!("远端上传成功，但本地同步基线保存失败: {error}"),
                );
            }
        }
        Ok(result)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (config, snapshot, force);
        Err(ServerFnError::new("浏览器目标不能直接同步配置"))
    }
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/sync/pull")
)]
pub async fn sync_pull(config: SyncConfig) -> Result<SyncPullResult, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        let config = save_native_sync_config(&config)?;
        let downloaded = sync::pull(&config).await?;
        Ok(SyncPullResult {
            snapshot: downloaded.snapshot,
            remote_revision: downloaded.remote_revision,
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = config;
        Err(ServerFnError::new("浏览器目标不能直接同步配置"))
    }
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/sync/pull/commit")
)]
pub async fn commit_sync_pull(
    config: SyncConfig,
    mut current_profile: AppProfile,
    snapshot: SyncSnapshot,
    remote_revision: String,
) -> Result<SyncResult, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        sync::validate_snapshot(&snapshot)?;
        sync::validate_remote_revision(&remote_revision)?;
        let config = save_native_sync_config(&config)?;
        snapshot.apply_to_profile(&mut current_profile);
        let profile = current_profile;
        run_native_blocking("保存下载的同步配置失败", move || {
            save_native_profile(&profile)
        })
        .await?;

        let mut saved = config;
        saved.last_sync_at = snapshot.updated_at;
        saved.last_remote_revision = remote_revision.clone();
        let mut result = SyncResult {
            updated_at: snapshot.updated_at,
            subscription_count: snapshot.subscriptions.len(),
            rule_count: snapshot.custom_rules.len(),
            remote_revision,
            checkpoint_saved: false,
            warning: None,
        };
        match save_native_sync_state(&saved) {
            Ok(()) => result.checkpoint_saved = true,
            Err(error) => {
                result.warning = Some(format!("远端数据已保存到本机，但同步基线保存失败: {error}"));
            }
        }
        Ok(result)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (config, current_profile, snapshot, remote_revision);
        Err(ServerFnError::new("浏览器目标不能直接提交同步配置"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn append_sync_warning(result: &mut SyncResult, warning: String) {
    if let Some(existing) = &mut result.warning {
        existing.push('；');
        existing.push_str(&warning);
    } else {
        result.warning = Some(warning);
    }
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn load_native_sync_config() -> Result<SyncConfig, ServerFnError> {
    Ok(SyncConfig::default())
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn save_native_sync_config(_config: &SyncConfig) -> Result<SyncConfig, ServerFnError> {
    Err(ServerFnError::new("浏览器端不能直接写入同步配置"))
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
        restart_native_core(request, rule_set_cache)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/tun/prepare")
)]
pub async fn prepare_tun_mode(request: ConnectionRequest) -> Result<(), ServerFnError> {
    if !request.tun {
        return Err(ServerFnError::new("TUN 权限预检需要启用 TUN 的连接配置"));
    }
    if request.nodes.is_empty() {
        return Err(ServerFnError::new("请先导入并选择一个节点"));
    }
    proxy_core::validate_custom_rules(&request.custom_rules)
        .map_err(|error| ServerFnError::new(format!("自定义规则无效: {error}")))?;
    let rule_set_cache = prepare_rule_set_cache_for_request(Some(&request)).await?;
    run_native_blocking("TUN 权限预检任务失败", move || {
        prepare_native_tun_mode(request, rule_set_cache)
    })
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/tun/release")
)]
pub async fn release_tun_mode() -> Result<(), ServerFnError> {
    run_native_blocking("释放 TUN 权限准备任务失败", release_native_tun_mode).await
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
    proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
    measure_native_latency(nodes, proxy_server_nameservers).await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/core/latency/start")
)]
pub async fn start_node_latency(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<u64, ServerFnError> {
    start_native_latency_session(nodes, proxy_server_nameservers).await
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

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/environment/copy")
)]
pub async fn copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    run_native_blocking(
        "代理环境变量复制任务失败",
        native_copy_proxy_environment_variables,
    )
    .await
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    get("/api/processes")
)]
pub async fn list_running_processes() -> Result<Vec<RunningProcess>, ServerFnError> {
    run_native_blocking("进程列表读取任务失败", native_list_running_processes).await
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
async fn measure_native_latency(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
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
        proxy_server_nameservers,
        mode: TunnelMode::Global,
        tun: false,
        allow_lan: false,
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
async fn measure_native_latency(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
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
        proxy_server_nameservers,
        mode: TunnelMode::Global,
        tun: false,
        allow_lan: false,
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
async fn measure_native_latency(
    _nodes: Vec<ProxyNode>,
    _proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接运行节点延迟探测"))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
async fn start_native_latency_session(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<u64, ServerFnError> {
    validate_latency_nodes(&nodes)?;
    let node_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let request = ConnectionRequest {
        nodes,
        selected_tag: node_tags.first().cloned().unwrap_or_default(),
        proxy_server_nameservers,
        mode: TunnelMode::Global,
        tun: false,
        allow_lan: false,
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
        let concurrency =
            std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_LATENCY_PROBES));
        for (index, tag) in node_tags.into_iter().enumerate() {
            let core = std::sync::Arc::clone(&core);
            let concurrency = std::sync::Arc::clone(&concurrency);
            tasks.spawn(async move {
                if index > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        index as u64 * LATENCY_PROBE_STAGGER_MS,
                    ))
                    .await;
                }
                let _permit = concurrency
                    .acquire_owned()
                    .await
                    .expect("latency probe semaphore should remain open");
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
async fn start_native_latency_session(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<u64, ServerFnError> {
    validate_latency_nodes(&nodes)?;
    let total = nodes.len();
    let fallback_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let (session_id, session) = register_latency_session(total)?;
    tokio::spawn(async move {
        for (node_batch, fallback_batch) in nodes
            .chunks(MAX_CONCURRENT_LATENCY_PROBES)
            .zip(fallback_tags.chunks(MAX_CONCURRENT_LATENCY_PROBES))
        {
            let results =
                measure_native_latency(node_batch.to_vec(), proxy_server_nameservers.clone())
                    .await
                    .unwrap_or_else(|error| {
                        fallback_batch
                            .iter()
                            .map(|tag| NodeLatency {
                                tag: tag.clone(),
                                latency_ms: None,
                                error: Some(error.to_string()),
                            })
                            .collect()
                    });
            for result in results {
                session.push(result);
            }
        }
        session.finish();
    });
    Ok(session_id)
}

#[cfg(target_arch = "wasm32")]
async fn start_native_latency_session(
    _nodes: Vec<ProxyNode>,
    _proxy_server_nameservers: Vec<String>,
) -> Result<u64, ServerFnError> {
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
        .user_agent("clash.meta kitty-pro/0.1")
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
fn prepare_native_cache_file() -> Result<String, ServerFnError> {
    let profile = profile_path()?;
    let directory = profile
        .parent()
        .ok_or_else(|| ServerFnError::new("无法确定 sing-box 缓存目录"))?
        .join("cache");
    std::fs::create_dir_all(&directory)
        .map_err(|error| ServerFnError::new(format!("创建 sing-box 缓存目录失败: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| ServerFnError::new(format!("限制 sing-box 缓存目录权限失败: {error}")),
        )?;
    }

    let path = directory.join("sing-box.db");
    prepare_private_cache_file(&path)?;
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| ServerFnError::new("sing-box 缓存路径不是有效 UTF-8"))
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare_private_cache_file(path: &std::path::Path) -> Result<(), ServerFnError> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(ServerFnError::new("sing-box 缓存文件不能是符号链接"));
        }
        if !metadata.file_type().is_file() {
            return Err(ServerFnError::new("sing-box 缓存路径不是普通文件"));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| ServerFnError::new(format!("准备 sing-box 缓存文件失败: {error}")))?;
    if !file
        .metadata()
        .map_err(|error| ServerFnError::new(format!("检查 sing-box 缓存文件失败: {error}")))?
        .is_file()
    {
        return Err(ServerFnError::new("sing-box 缓存路径不是普通文件"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                ServerFnError::new(format!("限制 FakeIP 缓存文件权限失败: {error}"))
            })?;
    }
    drop(file);
    Ok(())
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
        core.engine.status()
    } else {
        match SingBox::discover().map(NativeCore::new) {
            Ok(core) => {
                let status = core.engine.status();
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
    let path = profile_path()?;
    save_native_profile_to_path(profile, &path)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_native_profile_to_path(
    profile: &AppProfile,
    path: &std::path::Path,
) -> Result<(), ServerFnError> {
    use std::fs;
    use std::io::Write;

    let _guard = profile_write_lock()
        .lock()
        .map_err(|_| ServerFnError::new("本地配置写入锁已损坏"))?;
    let parent = path
        .parent()
        .ok_or_else(|| ServerFnError::new("本地配置目录无效"))?;
    fs::create_dir_all(parent)
        .map_err(|error| ServerFnError::new(format!("创建本地配置目录失败: {error}")))?;

    let bytes = serde_json::to_vec_pretty(profile)
        .map_err(|error| ServerFnError::new(format!("序列化本地配置失败: {error}")))?;
    let (temporary, mut file) = create_unique_profile_temporary_file(path)
        .map_err(|error| ServerFnError::new(format!("写入本地配置失败: {error}")))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(ServerFnError::new(format!("写入本地配置失败: {error}")));
    }
    drop(file);

    #[cfg(windows)]
    if path.exists() {
        if let Err(error) = fs::remove_file(path) {
            let _ = fs::remove_file(&temporary);
            return Err(ServerFnError::new(format!("替换本地配置失败: {error}")));
        }
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(ServerFnError::new(format!("保存本地配置失败: {error}")));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn profile_write_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(not(target_arch = "wasm32"))]
fn create_unique_profile_temporary_file(
    path: &std::path::Path,
) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    loop {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), id));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;

            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
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
    core.engine
        .select_outbound(&group, &outbound)
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
    apply_remote_rule_set_download_detour(&mut config);
    apply_owned_cache_config(&mut config, request, options);
    proxy_core::apply_proxy_group_selections(&mut config, &request.group_selections);
    #[cfg(target_os = "macos")]
    macos_route::pin_non_default_outbound_sources(&mut config)
        .map_err(|error| ServerFnError::new(format!("TUN 路由预检失败: {error}")))?;
    Ok(config)
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_remote_rule_set_download_detour(config: &mut serde_json::Value) {
    const DOWNLOAD_OUTBOUND: &str = "__kitty-rule-set-download";

    let Some(rule_sets) = config["route"]["rule_set"].as_array_mut() else {
        return;
    };
    let needs_download_outbound = rule_sets
        .iter()
        .any(|rule_set| rule_set["type"] == "remote" && rule_set["download_detour"] == "direct");
    if !needs_download_outbound {
        return;
    }
    for rule_set in rule_sets {
        if rule_set["type"] == "remote" && rule_set["download_detour"] == "direct" {
            rule_set["download_detour"] = serde_json::Value::String(DOWNLOAD_OUTBOUND.to_string());
        }
    }
    let Some(outbounds) = config["outbounds"].as_array_mut() else {
        return;
    };
    if !outbounds
        .iter()
        .any(|outbound| outbound["tag"] == DOWNLOAD_OUTBOUND)
    {
        outbounds.push(serde_json::json!({
            "type": "direct",
            "tag": DOWNLOAD_OUTBOUND,
            "domain_resolver": "dns-local",
        }));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_owned_cache_config(
    config: &mut serde_json::Value,
    request: &ConnectionRequest,
    options: &SingBoxOptions,
) {
    let Some(path) = options.cache_file.as_deref() else {
        return;
    };
    let Some(config) = config.as_object_mut() else {
        return;
    };
    let experimental = config
        .entry("experimental")
        .or_insert_with(|| serde_json::json!({}));
    if !experimental.is_object() {
        *experimental = serde_json::json!({});
    }
    experimental["cache_file"] = serde_json::json!({
        "enabled": true,
        "path": path,
        "store_fakeip": request.tun && request.mode != TunnelMode::Direct,
    });
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
#[cfg(not(target_arch = "wasm32"))]
fn mixed_listen_address(allow_lan: bool) -> &'static str {
    if allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    }
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

    if !enabled {
        // Never leave the OS pointing at a loopback listener that is about to
        // disappear. If disabling the proxy fails, keep the core alive.
        disable_managed_system_proxy_before_core_outage().map_err(ServerFnError::new)?;
        let mut guard = core_slot()
            .lock()
            .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
        if let Some(core) = guard.as_mut() {
            if core
                .engine
                .is_running()
                .map_err(|error| ServerFnError::new(error.to_string()))?
            {
                if let Err(stop_error) = core.engine.stop() {
                    let state = match core.engine.is_running() {
                        Ok(false) => "复查确认内核已停止".to_string(),
                        Ok(true) => "复查发现内核仍在运行".to_string(),
                        Err(status_error) => {
                            format!("无法确认内核状态，复查失败: {status_error}")
                        }
                    };
                    core.engine.force_shutdown();
                    core.config = None;
                    return Err(ServerFnError::new(format!(
                        "停止 sing-box 内核返回错误，{state}；已强制清理内核: {stop_error}"
                    )));
                }
            }
            core.engine.discard_prepared_config();
            core.config = None;
        }
        drop(guard);
        return native_core_status();
    }

    let request = request.ok_or_else(|| ServerFnError::new("缺少连接配置"))?;
    if request.nodes.is_empty() {
        return Err(ServerFnError::new("请先导入并选择一个节点"));
    }
    let options = SingBoxOptions {
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
        ..SingBoxOptions::default()
    };
    let config = build_native_config(&request, &options)?;
    singbox::check_config(&config).map_err(|error| ServerFnError::new(error.to_string()))?;

    let mut guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    if guard.is_none() {
        *guard = Some(
            SingBox::discover()
                .map(NativeCore::new)
                .map_err(|error| ServerFnError::new(error.to_string()))?,
        );
    }
    let core = guard.as_mut().expect("sing-box core was just initialized");
    if core
        .engine
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        drop(guard);
        return native_core_status();
    }
    core.config = None;
    if let Err(error) = start_config_with_bind_retry(&mut core.engine, &config) {
        return Err(ServerFnError::new(error));
    }
    core.config = Some(config);
    drop(guard);
    native_core_status()
}

/// Start the core, retrying while the mixed-in port is still held by
/// TIME_WAIT sockets from the previous core.
///
/// On macOS, connections accepted by the Go runtime do not carry
/// SO_REUSEADDR, and their TIME_WAIT state keeps blocking a cross-process
/// rebind of the same port for 30 seconds (2 * MSL). With the system proxy
/// enabled this happens on every restart: apps hold active connections to
/// 127.0.0.1:7890, the stopped core closes them (server-side close => TIME_WAIT),
/// and the replacement core immediately fails to bind. Retrying over the
/// TIME_WAIT window makes toggling TUN / switching configs succeed instead of
/// erroring with "bind: address already in use".
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn start_config_with_bind_retry(
    engine: &mut singbox::SingBox,
    config: &serde_json::Value,
) -> Result<(), String> {
    // 45 attempts * 1s covers the macOS 30s TIME_WAIT window with margin.
    const MAX_ATTEMPTS: u32 = 45;
    for attempt in 0..MAX_ATTEMPTS {
        match engine.start_config(config) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let message = error.to_string();
                if !message.contains("address already in use") || attempt + 1 >= MAX_ATTEMPTS {
                    return Err(message);
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
    unreachable!("retry loop always returns")
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
trait RestartCoreEngine {
    fn restart_is_running(&self) -> Result<bool, String>;
    fn restart_stop(&mut self) -> Result<(), String>;
    fn restart_start(&mut self, config: &serde_json::Value) -> Result<(), String>;
    fn restart_force_shutdown(&mut self);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
impl RestartCoreEngine for singbox::SingBox {
    fn restart_is_running(&self) -> Result<bool, String> {
        self.is_running().map_err(|error| error.to_string())
    }

    fn restart_stop(&mut self) -> Result<(), String> {
        self.stop().map_err(|error| error.to_string())
    }

    fn restart_start(&mut self, config: &serde_json::Value) -> Result<(), String> {
        start_config_with_bind_retry(self, config)
    }

    fn restart_force_shutdown(&mut self) {
        self.force_shutdown();
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[derive(Debug, PartialEq, Eq)]
enum CoreRestartOutcome {
    CandidateRunning,
    PreviousPreserved { error: String },
    PreviousRestored { error: String },
    CoreOffline { error: String },
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
enum CoreStartObservation {
    Running,
    Stopped { error: String },
    Unknown { error: String },
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn start_core_and_observe<E: RestartCoreEngine>(
    engine: &mut E,
    config: &serde_json::Value,
) -> CoreStartObservation {
    match engine.restart_start(config) {
        Ok(()) => CoreStartObservation::Running,
        Err(start_error) => match engine.restart_is_running() {
            Ok(true) => CoreStartObservation::Running,
            Ok(false) => CoreStartObservation::Stopped { error: start_error },
            Err(status_error) => CoreStartObservation::Unknown {
                error: format!("{start_error}；复查运行状态失败: {status_error}"),
            },
        },
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn restore_previous_core<E: RestartCoreEngine>(
    engine: &mut E,
    previous: Option<&serde_json::Value>,
    cause: String,
) -> CoreRestartOutcome {
    let Some(previous) = previous else {
        return CoreRestartOutcome::CoreOffline {
            error: format!("{cause}；没有可回滚的旧配置"),
        };
    };
    match start_core_and_observe(engine, previous) {
        CoreStartObservation::Running => CoreRestartOutcome::PreviousRestored {
            error: format!("{cause}；已恢复旧配置"),
        },
        CoreStartObservation::Stopped {
            error: rollback_error,
        } => CoreRestartOutcome::CoreOffline {
            error: format!("{cause}；恢复旧配置失败: {rollback_error}"),
        },
        CoreStartObservation::Unknown {
            error: rollback_error,
        } => {
            engine.restart_force_shutdown();
            CoreRestartOutcome::CoreOffline {
                error: format!(
                    "{cause}；恢复旧配置后无法确认内核状态，已强制关闭内核: {rollback_error}"
                ),
            }
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn restart_core_transaction<E, C>(
    engine: &mut E,
    candidate: &serde_json::Value,
    previous: Option<&serde_json::Value>,
    check_candidate: C,
) -> CoreRestartOutcome
where
    E: RestartCoreEngine,
    C: FnOnce(&serde_json::Value) -> Result<(), String>,
{
    if let Err(error) = check_candidate(candidate) {
        return CoreRestartOutcome::PreviousPreserved {
            error: format!("新配置静态校验失败，内核未重启: {error}"),
        };
    }

    let was_running = match engine.restart_is_running() {
        Ok(running) => running,
        Err(error) => {
            engine.restart_force_shutdown();
            return CoreRestartOutcome::CoreOffline {
                error: format!("读取 sing-box 运行状态失败，已强制关闭无法确认状态的内核: {error}"),
            };
        }
    };
    if was_running {
        if let Err(stop_error) = engine.restart_stop() {
            // Some backends report a shutdown error after they have already
            // released their runtime handle. Re-check before claiming that
            // the previous core was preserved.
            match engine.restart_is_running() {
                Ok(true) => {
                    return CoreRestartOutcome::PreviousPreserved {
                        error: format!(
                            "停止旧 sing-box 内核失败，但旧内核仍在运行，配置未切换: {stop_error}"
                        ),
                    };
                }
                Ok(false) => {
                    // The stop reported an error but the core is confirmed
                    // stopped, so the shutdown actually completed. Apply the
                    // candidate instead of resurrecting the previous config:
                    // turning TUN off must not bounce back to the TUN config
                    // just because the old core reported a noisy stop.
                    return match start_core_and_observe(engine, candidate) {
                        CoreStartObservation::Running => CoreRestartOutcome::CandidateRunning,
                        CoreStartObservation::Stopped {
                            error: candidate_error,
                        } => restore_previous_core(
                            engine,
                            previous,
                            format!(
                                "停止旧 sing-box 内核返回错误且内核已停止: {stop_error}；新配置启动失败: {candidate_error}"
                            ),
                        ),
                        CoreStartObservation::Unknown {
                            error: candidate_error,
                        } => {
                            engine.restart_force_shutdown();
                            CoreRestartOutcome::CoreOffline {
                                error: format!(
                                    "停止旧 sing-box 内核返回错误且内核已停止: {stop_error}；新配置启动后无法确认内核状态，已强制关闭内核: {candidate_error}"
                                ),
                            }
                        }
                    };
                }
                Err(status_error) => {
                    engine.restart_force_shutdown();
                    return CoreRestartOutcome::CoreOffline {
                        error: format!(
                            "停止旧 sing-box 内核失败且无法确认状态，已强制关闭内核: {stop_error}；复查状态失败: {status_error}"
                        ),
                    };
                }
            }
        }
    }

    match start_core_and_observe(engine, candidate) {
        CoreStartObservation::Running => CoreRestartOutcome::CandidateRunning,
        CoreStartObservation::Stopped {
            error: candidate_error,
        } => restore_previous_core(
            engine,
            previous,
            format!("新配置启动失败: {candidate_error}"),
        ),
        CoreStartObservation::Unknown {
            error: candidate_error,
        } => {
            engine.restart_force_shutdown();
            CoreRestartOutcome::CoreOffline {
                error: format!("新配置启动后无法确认内核状态，已强制关闭内核: {candidate_error}"),
            }
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn core_config_after_restart(
    outcome: &CoreRestartOutcome,
    candidate: serde_json::Value,
    previous: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match outcome {
        CoreRestartOutcome::CandidateRunning => Some(candidate),
        CoreRestartOutcome::PreviousPreserved { .. }
        | CoreRestartOutcome::PreviousRestored { .. } => previous,
        CoreRestartOutcome::CoreOffline { .. } => None,
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn finish_core_restart<F>(
    outcome: CoreRestartOutcome,
    disable_managed_system_proxy: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    match outcome {
        CoreRestartOutcome::CandidateRunning => Ok(()),
        CoreRestartOutcome::PreviousPreserved { error }
        | CoreRestartOutcome::PreviousRestored { error } => Err(error),
        CoreRestartOutcome::CoreOffline { error } => match disable_managed_system_proxy() {
            Ok(()) => Err(error),
            Err(disable_error) => Err(format!(
                "{error}；关闭 Kitty Pro 管理的系统代理失败: {disable_error}"
            )),
        },
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn disable_managed_system_proxy_before_core_outage() -> Result<(), String> {
    let status = match native_system_proxy_status() {
        Ok(status) => status,
        // A transient read failure must not block shutting the core down.
        // The next core start re-applies the managed proxy anyway, so the
        // only cost of skipping cleanup here is a briefly dangling setting.
        Err(_) => return Ok(()),
    };
    if !status.enabled {
        return Ok(());
    }
    set_native_system_proxy(false)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
fn disable_managed_system_proxy_before_core_outage() -> Result<(), String> {
    Ok(())
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn restart_native_core(
    request: ConnectionRequest,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ServerFnError> {
    use proxy_core::SingBoxOptions;
    use singbox::SingBox;

    let options = SingBoxOptions {
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
        ..SingBoxOptions::default()
    };
    let candidate = build_native_config(&request, &options)?;

    let mut guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    if guard.is_none() {
        *guard = Some(
            SingBox::discover()
                .map(NativeCore::new)
                .map_err(|error| ServerFnError::new(error.to_string()))?,
        );
    }
    let core = guard.as_mut().expect("sing-box core was just initialized");
    let previous = core.config.clone();
    core.engine
        .prepare_config(&candidate)
        .map_err(|error| ServerFnError::new(format!("新配置预检失败，内核未重启: {error}")))?;
    let restart_result =
        restart_core_transaction(&mut core.engine, &candidate, previous.as_ref(), |_| Ok(()));
    core.engine.discard_prepared_config();
    core.config = core_config_after_restart(&restart_result, candidate, previous);
    drop(guard);

    finish_core_restart(
        restart_result,
        disable_managed_system_proxy_before_core_outage,
    )
    .map_err(ServerFnError::new)?;
    native_core_status()
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
fn restart_native_core(
    request: ConnectionRequest,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ServerFnError> {
    // Starting the active VpnService again makes it stop the previous core,
    // establish a fresh VPN interface, and apply the replacement config.
    toggle_native_core(true, Some(request), rule_set_cache)
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn prepare_native_tun_mode(
    request: ConnectionRequest,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<(), ServerFnError> {
    use proxy_core::SingBoxOptions;
    use singbox::SingBox;

    let options = SingBoxOptions {
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
        ..SingBoxOptions::default()
    };
    let candidate = build_native_config(&request, &options)?;
    let mut guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    if guard.is_none() {
        *guard = Some(
            SingBox::discover()
                .map(NativeCore::new)
                .map_err(|error| ServerFnError::new(error.to_string()))?,
        );
    }
    let core = guard.as_mut().expect("sing-box core was just initialized");
    if core
        .engine
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        return Err(ServerFnError::new(
            "sing-box 已在运行，请通过核心重启应用 TUN 设置",
        ));
    }
    core.engine
        .prepare_config(&candidate)
        .map_err(|error| ServerFnError::new(format!("TUN 配置或权限预检失败: {error}")))
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn release_native_tun_mode() -> Result<(), ServerFnError> {
    let mut guard = core_slot()
        .lock()
        .map_err(|_| ServerFnError::new("sing-box 状态锁已损坏"))?;
    if let Some(core) = guard.as_mut() {
        if core
            .engine
            .is_running()
            .map_err(|error| ServerFnError::new(error.to_string()))?
        {
            return Err(ServerFnError::new(
                "sing-box 正在运行，不能单独释放当前 TUN helper",
            ));
        }
        core.engine.discard_prepared_config();
    }
    Ok(())
}

#[allow(dead_code)]
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn prepare_native_tun_mode(
    _request: ConnectionRequest,
    _rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<(), ServerFnError> {
    Err(ServerFnError::new("当前平台不使用桌面 TUN 权限预检"))
}

#[allow(dead_code)]
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
fn release_native_tun_mode() -> Result<(), ServerFnError> {
    Ok(())
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
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
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
        .engine
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        return Ok(CoreTraffic::default());
    }
    let traffic = core
        .engine
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
        .engine
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        return Ok(CoreLogBatch {
            next_cursor: cursor,
            entries: Vec::new(),
        });
    }
    core.engine
        .logs(cursor)
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
        .engine
        .is_running()
        .map_err(|error| ServerFnError::new(error.to_string()))?
    {
        return Ok(());
    }
    core.engine
        .set_log_enabled(enabled)
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
                route: parse_route_log(&entry.message, entry.outbound_chain, entry.source_ip),
                message: entry.message,
            })
            .collect(),
    }
}

fn parse_route_log(
    message: &str,
    outbound_chain: Vec<String>,
    source_ip: Option<String>,
) -> Option<RouteLogDetail> {
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
        outbound_chain,
        source_ip,
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
fn proxy_environment_variables() -> String {
    format!(
        "$env:http_proxy=\"http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}\"; \
         $env:https_proxy=\"http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}\"; \
         $env:all_proxy=\"socks5://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}\""
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn proxy_environment_variables() -> String {
    format!(
        "export http_proxy=http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT} && \
         export https_proxy=http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT} && \
         export all_proxy=socks5://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}"
    )
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn pipe_text_to_command(program: &str, args: &[&str], text: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 {program} 失败: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("{program} 未提供标准输入"))?
        .write_all(text.as_bytes())
        .map_err(|error| format!("写入 {program} 失败: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("等待 {program} 失败: {error}"))?;
    if !status.success() {
        return Err(format!("{program} 返回失败状态 {status}"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    pipe_text_to_command("/usr/bin/pbcopy", &[], &proxy_environment_variables())
        .map_err(ServerFnError::new)
}

#[cfg(target_os = "windows")]
fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    pipe_text_to_command("clip.exe", &[], &proxy_environment_variables())
        .map_err(ServerFnError::new)
}

#[cfg(target_os = "linux")]
fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    let text = proxy_environment_variables();
    let commands: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    let mut errors = Vec::new();
    for (program, args) in commands {
        match pipe_text_to_command(program, args, &text) {
            Ok(()) => return Ok(()),
            Err(error) => errors.push(error),
        }
    }
    Err(ServerFnError::new(format!(
        "未找到可用的系统剪贴板工具: {}",
        errors.join("；")
    )))
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    Err(ServerFnError::new("当前平台尚未实现系统剪贴板适配"))
}

#[cfg(target_arch = "wasm32")]
fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接写入系统剪贴板"))
}

#[cfg(target_os = "windows")]
const SYSTEM_PROXY_BYPASS: &str = "localhost;127.*;192.168.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;<local>";
#[cfg(target_os = "linux")]
const SYSTEM_PROXY_BYPASS: &str = "localhost,127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,::1";
#[cfg(target_os = "macos")]
const SYSTEM_PROXY_BYPASS: &str =
    "127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,localhost,*.local,<local>";

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn system_proxy_operation_lock() -> Result<std::sync::MutexGuard<'static, ()>, ServerFnError> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| ServerFnError::new("系统代理操作锁已损坏"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn read_native_system_proxy() -> Result<sysproxy::Sysproxy, ServerFnError> {
    sysproxy::Sysproxy::get_system_proxy()
        .map_err(|error| ServerFnError::new(format!("读取系统代理失败: {error}")))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn apply_native_system_proxy(proxy: &sysproxy::Sysproxy) -> Result<(), ServerFnError> {
    proxy
        .set_system_proxy()
        .map_err(|error| ServerFnError::new(format!("设置系统代理失败: {error}")))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn apply_native_auto_proxy(proxy: &sysproxy::Autoproxy) -> Result<(), ServerFnError> {
    proxy
        .set_auto_proxy()
        .map_err(|error| ServerFnError::new(format!("设置自动代理失败: {error}")))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn is_kitty_system_proxy(proxy: &sysproxy::Sysproxy) -> bool {
    proxy.enable
        && proxy.host.eq_ignore_ascii_case(SYSTEM_PROXY_HOST)
        && proxy.port == SYSTEM_PROXY_PORT
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn native_system_proxy_status_unlocked() -> Result<SystemProxyStatus, ServerFnError> {
    let proxy = read_native_system_proxy()?;
    let enabled = is_kitty_system_proxy(&proxy);
    let detail = if enabled {
        format!(
            "已设置 {} 系统代理 {SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}",
            system_proxy_platform_name()
        )
    } else if proxy.enable {
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

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    let _operation = system_proxy_operation_lock()?;
    native_system_proxy_status_unlocked()
}

#[cfg(target_os = "macos")]
const fn system_proxy_platform_name() -> &'static str {
    "macOS"
}

#[cfg(target_os = "windows")]
const fn system_proxy_platform_name() -> &'static str {
    "Windows"
}

#[cfg(target_os = "linux")]
const fn system_proxy_platform_name() -> &'static str {
    "Linux"
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

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn set_native_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    let _operation = system_proxy_operation_lock()?;

    if enabled {
        ensure_core_is_running()?;
        let proxy = sysproxy::Sysproxy {
            host: SYSTEM_PROXY_HOST.to_string(),
            bypass: SYSTEM_PROXY_BYPASS.to_string(),
            port: SYSTEM_PROXY_PORT,
            enable: true,
        };
        apply_native_auto_proxy(&sysproxy::Autoproxy::default())?;
        apply_native_system_proxy(&proxy)?;
    } else {
        // sysproxy always rewrites the bypass list on macOS, and its default
        // (empty) bypass makes `networksetup -setproxybypassdomains` fail with
        // an argument count error. Reuse the managed bypass list so disabling
        // the proxy stays idempotent instead of erroring out.
        let proxy = sysproxy::Sysproxy {
            host: SYSTEM_PROXY_HOST.to_string(),
            bypass: SYSTEM_PROXY_BYPASS.to_string(),
            port: SYSTEM_PROXY_PORT,
            enable: false,
        };
        apply_native_system_proxy(&proxy)?;
        apply_native_auto_proxy(&sysproxy::Autoproxy::default())?;
    }

    native_system_proxy_status_unlocked()
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
fn native_list_running_processes() -> Result<Vec<RunningProcess>, ServerFnError> {
    use std::collections::BTreeMap;

    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .without_tasks(),
    );

    let current_pid = std::process::id();
    let mut by_identity = BTreeMap::new();
    for process in system.processes().values() {
        let pid = process.pid().as_u32();
        if pid == current_pid {
            continue;
        }
        let Some(exe_path) = process.exe() else {
            continue;
        };
        insert_running_process(&mut by_identity, pid, exe_path);
    }

    Ok(by_identity.into_values().collect())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn insert_running_process(
    by_identity: &mut std::collections::BTreeMap<(String, String), RunningProcess>,
    pid: u32,
    exe_path: &std::path::Path,
) {
    let Some(name) = exe_path.file_name() else {
        return;
    };
    let name = name.to_string_lossy().into_owned();
    let exe_path = exe_path.to_string_lossy().into_owned();
    if name.is_empty() || exe_path.is_empty() {
        return;
    }

    let process = RunningProcess {
        pid,
        name: name.clone(),
        exe_path: Some(exe_path.clone()),
    };
    by_identity
        .entry((name, exe_path))
        .and_modify(|existing| {
            if pid < existing.pid {
                existing.pid = pid;
            }
        })
        .or_insert(process);
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
fn native_list_running_processes() -> Result<Vec<RunningProcess>, ServerFnError> {
    Ok(Vec::new())
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn native_list_running_processes() -> Result<Vec<RunningProcess>, ServerFnError> {
    Ok(Vec::new())
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
    let status = core.engine.status();
    if status.state != CoreState::Running {
        return Err(ServerFnError::new("请先建立连接，再启用系统代理"));
    }
    Ok(())
}

/// Disable any active Kitty Pro system proxy, then stop the embedded core.
/// Desktop launchers call this during normal event-loop teardown so users are
/// not left with a dead loopback proxy after exit.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn shutdown_native_runtime() -> Result<(), String> {
    let mut errors = Vec::new();

    if let Err(error) = disable_managed_system_proxy_before_core_outage() {
        errors.push(format!("关闭系统代理失败: {error}"));
    }

    match core_slot().lock() {
        Ok(mut guard) => {
            if let Some(core) = guard.as_mut() {
                match core.engine.is_running() {
                    Ok(true) => match core.engine.stop() {
                        Ok(()) => {
                            core.engine.discard_prepared_config();
                            core.config = None;
                        }
                        Err(error) => {
                            errors.push(format!("停止 sing-box 内核失败: {error}"));
                        }
                    },
                    Ok(false) => {
                        core.engine.discard_prepared_config();
                        core.config = None;
                    }
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
fn core_slot() -> &'static std::sync::Mutex<Option<NativeCore>> {
    use std::sync::{Mutex, OnceLock};

    static CORE: OnceLock<Mutex<Option<NativeCore>>> = OnceLock::new();
    CORE.get_or_init(|| Mutex::new(None))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[derive(Debug)]
struct NativeCore {
    engine: singbox::SingBox,
    config: Option<serde_json::Value>,
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
impl NativeCore {
    fn new(engine: singbox::SingBox) -> Self {
        Self {
            engine,
            config: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn system_proxy_status_only_accepts_kitty_endpoint() {
        let mut proxy = sysproxy::Sysproxy {
            host: "127.0.0.1".to_string(),
            bypass: String::new(),
            port: SYSTEM_PROXY_PORT,
            enable: true,
        };
        assert!(is_kitty_system_proxy(&proxy));

        proxy.port += 1;
        assert!(!is_kitty_system_proxy(&proxy));
        proxy.port = SYSTEM_PROXY_PORT;
        proxy.enable = false;
        assert!(!is_kitty_system_proxy(&proxy));
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn copied_proxy_environment_targets_the_local_mixed_listener() {
        let variables = proxy_environment_variables();

        assert!(variables.contains("http://127.0.0.1:7890"));
        assert!(variables.contains("socks5://127.0.0.1:7890"));
        for name in ["http_proxy", "https_proxy", "all_proxy"] {
            assert!(variables.contains(name));
        }
        assert!(!variables.contains("no_proxy"));

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert_eq!(variables.matches(" && ").count(), 2);
        #[cfg(target_os = "windows")]
        assert_eq!(variables.matches("; ").count(), 2);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn list_running_processes_excludes_self_and_sorts_by_name() {
        let processes = native_list_running_processes().expect("process list should succeed");
        // 当前测试进程自身应被过滤
        let self_pid = std::process::id();
        assert!(
            !processes.iter().any(|p| p.pid == self_pid),
            "self process should be excluded from the list"
        );
        // 至少能列出一些系统进程（本机必然有多个进程在跑）
        assert!(!processes.is_empty(), "expected at least some processes");

        let identities = processes
            .iter()
            .map(|process| (&process.name, &process.exe_path))
            .collect::<Vec<_>>();
        let mut sorted = identities.clone();
        sorted.sort();
        assert_eq!(identities, sorted, "processes should be sorted by identity");

        for p in &processes {
            assert!(!p.name.is_empty(), "name must not be empty");
            assert!(
                p.exe_path.as_deref().is_some_and(|path| !path.is_empty()),
                "exe_path must not be empty"
            );
            assert_eq!(
                p.exe_path
                    .as_deref()
                    .and_then(|path| std::path::Path::new(path).file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .as_deref(),
                Some(p.name.as_str()),
                "name must match the executable basename"
            );
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn process_identity_keeps_distinct_paths_and_lowest_pid() {
        let mut processes = std::collections::BTreeMap::new();
        insert_running_process(&mut processes, 20, std::path::Path::new("/opt/one/tool"));
        insert_running_process(&mut processes, 10, std::path::Path::new("/opt/one/tool"));
        insert_running_process(&mut processes, 30, std::path::Path::new("/opt/two/tool"));

        let processes = processes.into_values().collect::<Vec<_>>();
        assert_eq!(processes.len(), 2);
        assert_eq!(processes[0].pid, 10);
        assert_eq!(processes[0].name, "tool");
        assert_eq!(processes[0].exe_path.as_deref(), Some("/opt/one/tool"));
        assert_eq!(processes[1].exe_path.as_deref(), Some("/opt/two/tool"));
    }

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

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    struct FakeRestartCore {
        running: bool,
        fail_status: bool,
        fail_status_on_call: Option<usize>,
        status_calls: std::cell::Cell<usize>,
        fail_stop: bool,
        stop_error_after_shutdown: bool,
        fail_start_tags: std::collections::HashSet<String>,
        start_error_after_running_tags: std::collections::HashSet<String>,
        events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    impl FakeRestartCore {
        fn running(events: std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Self {
            Self {
                running: true,
                fail_status: false,
                fail_status_on_call: None,
                status_calls: std::cell::Cell::new(0),
                fail_stop: false,
                stop_error_after_shutdown: false,
                fail_start_tags: std::collections::HashSet::new(),
                start_error_after_running_tags: std::collections::HashSet::new(),
                events,
            }
        }

        fn push_event(&self, event: impl Into<String>) {
            self.events.lock().unwrap().push(event.into());
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    impl RestartCoreEngine for FakeRestartCore {
        fn restart_is_running(&self) -> Result<bool, String> {
            self.push_event("status");
            let call = self.status_calls.get() + 1;
            self.status_calls.set(call);
            if self.fail_status || self.fail_status_on_call == Some(call) {
                Err("status failed".to_string())
            } else {
                Ok(self.running)
            }
        }

        fn restart_stop(&mut self) -> Result<(), String> {
            self.push_event("stop");
            if self.fail_stop {
                if self.stop_error_after_shutdown {
                    self.running = false;
                }
                return Err("stop failed".to_string());
            }
            self.running = false;
            Ok(())
        }

        fn restart_start(&mut self, config: &serde_json::Value) -> Result<(), String> {
            let tag = config["tag"].as_str().unwrap_or("unknown");
            self.push_event(format!("start:{tag}"));
            if self.fail_start_tags.contains(tag) {
                return Err(format!("{tag} failed"));
            }
            self.running = true;
            if self.start_error_after_running_tags.contains(tag) {
                return Err(format!("{tag} response failed"));
            }
            Ok(())
        }

        fn restart_force_shutdown(&mut self) {
            self.push_event("force-shutdown");
            self.running = false;
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    fn restart_fixture() -> (
        serde_json::Value,
        serde_json::Value,
        std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        FakeRestartCore,
    ) {
        let candidate = serde_json::json!({ "tag": "candidate" });
        let previous = serde_json::json!({ "tag": "previous" });
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let core = FakeRestartCore::running(events.clone());
        (candidate, previous, events, core)
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn restart_checks_candidate_before_stopping_previous_core() {
        let (candidate, previous, events, mut core) = restart_fixture();
        let check_events = events.clone();

        let outcome =
            restart_core_transaction(&mut core, &candidate, Some(&previous), move |config| {
                check_events
                    .lock()
                    .unwrap()
                    .push(format!("check:{}", config["tag"].as_str().unwrap()));
                Ok(())
            });

        assert_eq!(outcome, CoreRestartOutcome::CandidateRunning);
        assert!(core.running);
        assert_eq!(
            *events.lock().unwrap(),
            ["check:candidate", "status", "stop", "start:candidate"]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate.clone(), Some(previous)),
            Some(candidate)
        );
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn invalid_restart_candidate_leaves_previous_core_untouched() {
        let (candidate, previous, events, mut core) = restart_fixture();

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| {
            Err("invalid candidate".to_string())
        });

        assert!(matches!(
            outcome,
            CoreRestartOutcome::PreviousPreserved { .. }
        ));
        assert!(core.running);
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(
            core_config_after_restart(&outcome, candidate, Some(previous.clone())),
            Some(previous)
        );
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn failed_candidate_restores_previous_config_without_proxy_disable() {
        let (candidate, previous, events, mut core) = restart_fixture();
        core.fail_start_tags.insert("candidate".to_string());

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert!(matches!(
            outcome,
            CoreRestartOutcome::PreviousRestored { .. }
        ));
        assert!(core.running);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "status",
                "stop",
                "start:candidate",
                "status",
                "start:previous"
            ]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate, Some(previous.clone())),
            Some(previous)
        );

        let restore_calls = std::cell::Cell::new(0);
        assert!(finish_core_restart(outcome, || {
            restore_calls.set(restore_calls.get() + 1);
            Ok(())
        })
        .is_err());
        assert_eq!(restore_calls.get(), 0);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn failed_candidate_and_rollback_marks_core_offline_and_disables_proxy() {
        let (candidate, previous, events, mut core) = restart_fixture();
        core.fail_start_tags = ["candidate".to_string(), "previous".to_string()]
            .into_iter()
            .collect();

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert!(matches!(outcome, CoreRestartOutcome::CoreOffline { .. }));
        assert!(!core.running);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "status",
                "stop",
                "start:candidate",
                "status",
                "start:previous",
                "status"
            ]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate, Some(previous)),
            None
        );

        let restore_calls = std::cell::Cell::new(0);
        assert!(finish_core_restart(outcome, || {
            restore_calls.set(restore_calls.get() + 1);
            Ok(())
        })
        .is_err());
        assert_eq!(restore_calls.get(), 1);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn failed_stop_preserves_previous_config_without_proxy_disable() {
        let (candidate, previous, events, mut core) = restart_fixture();
        core.fail_stop = true;

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert!(matches!(
            outcome,
            CoreRestartOutcome::PreviousPreserved { .. }
        ));
        assert!(core.running);
        assert_eq!(*events.lock().unwrap(), ["status", "stop", "status"]);
        assert_eq!(
            core_config_after_restart(&outcome, candidate, Some(previous.clone())),
            Some(previous)
        );

        let restore_calls = std::cell::Cell::new(0);
        assert!(finish_core_restart(outcome, || {
            restore_calls.set(restore_calls.get() + 1);
            Ok(())
        })
        .is_err());
        assert_eq!(restore_calls.get(), 0);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn failed_stop_after_shutdown_continues_with_candidate() {
        // A noisy stop error after the core is confirmed stopped must not
        // resurrect the previous config (e.g. turning TUN off bouncing back
        // to the TUN config); the candidate config is what the user asked for.
        let (candidate, previous, events, mut core) = restart_fixture();
        core.fail_stop = true;
        core.stop_error_after_shutdown = true;

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert!(matches!(outcome, CoreRestartOutcome::CandidateRunning));
        assert!(core.running);
        assert_eq!(
            *events.lock().unwrap(),
            ["status", "stop", "status", "start:candidate"]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate.clone(), Some(previous.clone())),
            Some(candidate)
        );

        let restore_calls = std::cell::Cell::new(0);
        assert!(finish_core_restart(outcome, || {
            restore_calls.set(restore_calls.get() + 1);
            Ok(())
        })
        .is_ok());
        assert_eq!(restore_calls.get(), 0);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn failed_stop_with_unknown_state_forces_shutdown_and_disables_proxy() {
        let (candidate, previous, events, mut core) = restart_fixture();
        core.fail_stop = true;
        core.fail_status_on_call = Some(2);

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert!(matches!(outcome, CoreRestartOutcome::CoreOffline { .. }));
        assert!(!core.running);
        assert_eq!(
            *events.lock().unwrap(),
            ["status", "stop", "status", "force-shutdown"]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate, Some(previous)),
            None
        );

        let restore_calls = std::cell::Cell::new(0);
        assert!(finish_core_restart(outcome, || {
            restore_calls.set(restore_calls.get() + 1);
            Ok(())
        })
        .is_err());
        assert_eq!(restore_calls.get(), 1);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn candidate_start_error_after_running_commits_candidate() {
        let (candidate, previous, events, mut core) = restart_fixture();
        core.start_error_after_running_tags
            .insert("candidate".to_string());

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert_eq!(outcome, CoreRestartOutcome::CandidateRunning);
        assert!(core.running);
        assert_eq!(
            *events.lock().unwrap(),
            ["status", "stop", "start:candidate", "status"]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate.clone(), Some(previous)),
            Some(candidate)
        );
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn candidate_start_with_unknown_state_is_forced_offline() {
        let (candidate, previous, events, mut core) = restart_fixture();
        core.fail_start_tags.insert("candidate".to_string());
        core.fail_status_on_call = Some(2);

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert!(matches!(outcome, CoreRestartOutcome::CoreOffline { .. }));
        assert!(!core.running);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "status",
                "stop",
                "start:candidate",
                "status",
                "force-shutdown"
            ]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate, Some(previous)),
            None
        );

        let restore_calls = std::cell::Cell::new(0);
        assert!(finish_core_restart(outcome, || {
            restore_calls.set(restore_calls.get() + 1);
            Ok(())
        })
        .is_err());
        assert_eq!(restore_calls.get(), 1);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
    #[test]
    fn rollback_start_error_after_running_is_still_restored() {
        let (candidate, previous, events, mut core) = restart_fixture();
        core.fail_start_tags.insert("candidate".to_string());
        core.start_error_after_running_tags
            .insert("previous".to_string());

        let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

        assert!(matches!(
            outcome,
            CoreRestartOutcome::PreviousRestored { .. }
        ));
        assert!(core.running);
        assert_eq!(
            *events.lock().unwrap(),
            [
                "status",
                "stop",
                "start:candidate",
                "status",
                "start:previous",
                "status"
            ]
        );
        assert_eq!(
            core_config_after_restart(&outcome, candidate, Some(previous.clone())),
            Some(previous)
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn concurrent_profile_saves_are_atomic_and_last_write_wins() {
        let directory = TestDirectory::new("profile-concurrency");
        let path = directory.0.join("profile.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut writers = Vec::new();

        for index in 0..8 {
            let path = path.clone();
            let barrier = barrier.clone();
            writers.push(std::thread::spawn(move || {
                let profile = AppProfile {
                    selected_tag: format!("concurrent-{index}"),
                    config_script: "x".repeat(32 * 1024 + index),
                    ..AppProfile::default()
                };
                barrier.wait();
                save_native_profile_to_path(&profile, &path)
            }));
        }
        for writer in writers {
            writer
                .join()
                .expect("profile writer should not panic")
                .expect("concurrent profile save should succeed");
        }

        let concurrent: AppProfile = serde_json::from_slice(
            &std::fs::read(&path).expect("concurrent profile should be readable"),
        )
        .expect("concurrent profile should contain one complete JSON document");
        assert!(concurrent.selected_tag.starts_with("concurrent-"));

        let last = AppProfile {
            selected_tag: "last-write".to_string(),
            ..AppProfile::default()
        };
        save_native_profile_to_path(&last, &path).expect("last profile save should succeed");
        let saved: AppProfile =
            serde_json::from_slice(&std::fs::read(&path).expect("last profile should be readable"))
                .expect("last profile should be valid");
        assert_eq!(saved, last);

        let remaining = std::fs::read_dir(&directory.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(remaining, [std::ffi::OsString::from("profile.json")]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn singbox_cache_file_is_private_and_reused() {
        let directory = TestDirectory::new("singbox-cache");
        let path = directory.0.join("sing-box.db");

        prepare_private_cache_file(&path).expect("cache file should be created");
        std::fs::write(&path, b"persistent sing-box cache")
            .expect("cache fixture should be written");
        prepare_private_cache_file(&path).expect("cache file should be reusable");

        assert_eq!(
            std::fs::read(&path).expect("cache fixture should be readable"),
            b"persistent sing-box cache"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&path)
                    .expect("cache metadata should be readable")
                    .permissions()
                    .mode()
                    & 0o077,
                0
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn singbox_cache_file_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("singbox-cache-symlink");
        let target = directory.0.join("target.db");
        let path = directory.0.join("sing-box.db");
        std::fs::write(&target, b"unrelated data").expect("target should be written");
        symlink(&target, &path).expect("cache symlink should be created");

        let error =
            prepare_private_cache_file(&path).expect_err("cache symlink should be rejected");

        assert!(error.to_string().contains("符号链接"));
        assert_eq!(
            std::fs::read(&target).expect("symlink target should remain readable"),
            b"unrelated data"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_listener_follows_allow_lan_setting() {
        assert_eq!(mixed_listen_address(false), "127.0.0.1");
        assert_eq!(mixed_listen_address(true), "0.0.0.0");
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
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: false,
            allow_lan: false,
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
    fn native_config_resolves_script_rule_set_downloads_with_system_dns() {
        let nodes = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: false,
            allow_lan: true,
            custom_rules: Vec::new(),
            config_script: Some(
                "function main(config) {\
                    config.route.rule_set.push({\
                        type: 'remote',\
                        tag: 'script-rule-set',\
                        format: 'binary',\
                        url: 'https://rules.example.com/script.srs',\
                        download_detour: 'direct'\
                    });\
                    return config;\
                }"
                .to_string(),
            ),
            group_selections: Default::default(),
        };

        let config = build_native_config(&request, &SingBoxOptions::default())
            .expect("script rule set should be accepted");
        let script_rule_set = config["route"]["rule_set"]
            .as_array()
            .and_then(|rule_sets| {
                rule_sets
                    .iter()
                    .find(|rule_set| rule_set["tag"] == "script-rule-set")
            })
            .expect("script rule set should remain in the config");

        assert_eq!(
            script_rule_set["download_detour"],
            "__kitty-rule-set-download"
        );
        let download_outbound = config["outbounds"]
            .as_array()
            .and_then(|outbounds| {
                outbounds
                    .iter()
                    .find(|outbound| outbound["tag"] == "__kitty-rule-set-download")
            })
            .expect("rule-set download outbound should be generated");
        assert_eq!(download_outbound["type"], "direct");
        assert_eq!(download_outbound["domain_resolver"], "dns-local");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn app_owned_cache_path_overrides_script_output() {
        let nodes = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Global,
            tun: true,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: Default::default(),
        };
        let options = SingBoxOptions {
            cache_file: Some("/app-data/cache/sing-box.db".to_string()),
            ..SingBoxOptions::default()
        };
        let mut config = serde_json::json!({
            "experimental": {
                "cache_file": {
                    "enabled": true,
                    "path": "/tmp/untrusted.db",
                    "store_fakeip": false,
                }
            }
        });

        apply_owned_cache_config(&mut config, &request, &options);

        assert_eq!(
            config["experimental"]["cache_file"],
            serde_json::json!({
                "enabled": true,
                "path": "/app-data/cache/sing-box.db",
                "store_fakeip": true,
            })
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn app_owned_cache_is_enabled_without_tun() {
        let nodes = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: false,
            allow_lan: true,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: Default::default(),
        };
        let options = SingBoxOptions {
            cache_file: Some("/app-data/cache/sing-box.db".to_string()),
            ..SingBoxOptions::default()
        };
        let mut config = serde_json::json!({
            "route": {
                "rule_set": [{
                    "type": "remote",
                    "tag": "script-rule-set",
                    "format": "binary",
                    "url": "https://rules.example.com/script.srs"
                }]
            }
        });

        apply_owned_cache_config(&mut config, &request, &options);

        assert_eq!(config["experimental"]["cache_file"]["enabled"], true);
        assert_eq!(config["experimental"]["cache_file"]["store_fakeip"], false);
        assert_eq!(config["route"]["rule_set"][0]["tag"], "script-rule-set");
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
                "http://proxy.example.com:8080#HTTP".to_string(),
            ))
            .expect("HTTP proxy URI should parse locally");

        assert_eq!(report.nodes.len(), 1);
        assert_eq!(report.nodes[0].protocol, proxy_core::ProxyProtocol::Http);
        assert_eq!(report.nodes[0].server, "proxy.example.com");
        assert_eq!(report.nodes[0].port, 8080);
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "server")))]
    #[test]
    fn native_company_proxy_preview_creates_one_socks5_node() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("native test runtime should start");
        let report = runtime
            .block_on(preview_subscription(
                "socks5://100.64.0.2:11080#Company".to_string(),
            ))
            .expect("SOCKS5 proxy URI should parse locally");

        assert_eq!(report.nodes.len(), 1);
        let node = &report.nodes[0];
        assert_eq!(node.name, "Company");
        assert_eq!(node.protocol, proxy_core::ProxyProtocol::Socks5);
        assert_eq!(node.server, "100.64.0.2");
        assert_eq!(node.port, 11080);
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
            report
                .nodes
                .sort_by_key(|node| node.protocol != proxy_core::ProxyProtocol::AnyTls);
            report.nodes.truncate(8);
            if report.nodes.is_empty() {
                return Err(ServerFnError::new("订阅没有可探测的节点"));
            }
            measure_node_latency(report.nodes, report.proxy_server_nameservers).await
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
            Vec::new(),
            Some("127.0.0.1".to_string()),
        )
        .expect("direct route should be parsed");

        assert_eq!(route.decision, RouteDecision::Direct);
        assert_eq!(route.host, "www.baidu.com");
        assert_eq!(route.port, Some(443));
        assert_eq!(route.target_kind, RouteTargetKind::Domain);
        assert_eq!(route.outbound_tag, "direct");
        assert_eq!(route.source_ip.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn parses_proxy_ip_and_ipv6_route_logs() {
        let proxy = parse_route_log(
            "INFO[0002] outbound/vless[subscription-1-edge]: outbound packet connection to 8.8.8.8:53",
            vec![
                "AI节点".to_string(),
                "美国节点".to_string(),
                "subscription-1-edge".to_string(),
            ],
            Some("192.168.1.23".to_string()),
        )
        .expect("proxy route should be parsed");
        assert_eq!(proxy.decision, RouteDecision::Proxy);
        assert_eq!(proxy.host, "8.8.8.8");
        assert_eq!(proxy.port, Some(53));
        assert_eq!(proxy.target_kind, RouteTargetKind::Ip);
        assert_eq!(proxy.outbound_type, "vless");
        assert_eq!(proxy.outbound_chain[0], "AI节点");
        assert_eq!(proxy.source_ip.as_deref(), Some("192.168.1.23"));

        let ipv6 = parse_route_log(
            "INFO[0003] outbound/direct[direct]: outbound connection to [2001:db8::1]:443",
            Vec::new(),
            None,
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
            Vec::new(),
            None,
        )
        .expect("block route should be parsed");

        assert_eq!(route.decision, RouteDecision::Block);
        assert_eq!(route.outbound_tag, "block");
    }

    #[test]
    fn ignores_non_outbound_log_lines() {
        assert!(parse_route_log(
            "INFO[0001] inbound/mixed[mixed-in]: inbound connection to example.com:443",
            Vec::new(),
            None,
        )
        .is_none());
    }
}
