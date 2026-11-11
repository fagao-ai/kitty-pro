//! The app's backend: profile, core control, sync, updates and system integration.
//!
//! These are plain async functions called in-process by the NSH app. They used to
//! be Dioxus server functions, which is why the module names and error wording
//! still speak of endpoints; there is no HTTP layer left.

#[cfg(all(
    feature = "native-desktop",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
pub mod autostart;
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
pub mod error;
pub mod update;
#[allow(unused_imports)]
pub(crate) use error::*;
pub use error::{api_err, error_code, ApiError, ApiErrorCode};

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
const MAX_SUBSCRIPTION_BYTES: usize = 10 * 1024 * 1024;

// The inline-subscription size check in `preview_subscription` lives in a
// server-fn body that is compiled for every target, so the cap must exist on
// wasm32 too.
#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
const MAX_SUBSCRIPTION_BYTES: usize = 10 * 1024 * 1024;

/// B8: hard limits applied to every profile that reaches the server. These are
/// deliberately shared by `save_profile` and the core-startup entry so a
/// hijacked client cannot persist an oversized profile or feed unbounded node
/// lists into sing-box.
#[allow(dead_code)]
const MAX_PROFILE_SUBSCRIPTIONS: usize = 64;
#[allow(dead_code)]
const MAX_SUBSCRIPTION_SOURCE_BYTES: usize = 64 * 1024;
#[allow(dead_code)]
const MAX_NODES_PER_SUBSCRIPTION: usize = 4096;
#[allow(dead_code)]
const MAX_TOTAL_NODES: usize = 16384;
#[allow(dead_code)]
const MAX_NODE_TAG_BYTES: usize = 512;
#[allow(dead_code)]
const MAX_PROFILE_CONFIG_SCRIPT_BYTES: usize = 512 * 1024;
#[allow(dead_code)]
const MAX_GROUP_SELECTIONS: usize = 4096;

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
    /// WebDAV base URL. Kept apart from `s3_endpoint` so switching the provider
    /// in the UI never reinterprets an address saved for the other provider.
    #[serde(default)]
    pub webdav_endpoint: String,
    /// S3-compatible endpoint. Empty selects AWS S3.
    #[serde(default)]
    pub s3_endpoint: String,
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
            webdav_endpoint: String::new(),
            s3_endpoint: String::new(),
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
    /// How many DNS rules travelled. `None` means the remote document carried no
    /// DNS section at all - every object uploaded before the feature shipped -
    /// which the settings page has to word differently from "zero rules": the
    /// local list is kept in that case, not emptied.
    #[serde(default)]
    pub dns_rule_count: Option<usize>,
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

pub async fn load_profile() -> Result<AppProfile, ApiError> {
    run_native_blocking("读取本地配置任务失败", load_native_profile).await
}

pub async fn save_profile(profile: AppProfile) -> Result<(), ApiError> {
    validate_app_profile(&profile)?;
    run_native_blocking("保存本地配置任务失败", move || {
        save_native_profile(&profile)
    })
    .await
}

/// Result of the control-plane connectivity probe rendered by the UI.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConnectivityProbe {
    pub ok: bool,
    pub detail: String,
}

pub async fn probe_connectivity() -> Result<ConnectivityProbe, ApiError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::Duration;

        // The downstream target is a fixed constant, never user input. It is
        // still passed through the shared outbound validator as defence in
        // depth, so a future edit cannot silently turn it into an SSRF sink.
        const PROBE_URL: &str = "https://www.gstatic.com/generate_204";
        if validate_outbound_url(PROBE_URL).is_err() {
            return Ok(ConnectivityProbe {
                ok: false,
                detail: "出口探测地址不在允许范围".into(),
            });
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .proxy(
                reqwest::Proxy::all("http://127.0.0.1:7890")
                    .map_err(|error| ApiError::new(redact_secrets(&error.to_string())))?,
            )
            .build()
            .map_err(|error| ApiError::new(redact_secrets(&error.to_string())))?;
        match client.get(PROBE_URL).send().await {
            Ok(response) if response.status() == reqwest::StatusCode::NO_CONTENT => {
                Ok(ConnectivityProbe {
                    ok: true,
                    detail: "出口可用（HTTP 204）".into(),
                })
            }
            Ok(response) => Ok(ConnectivityProbe {
                ok: false,
                detail: redact_secrets(&format!("出口探测失败：返回 HTTP {}", response.status())),
            }),
            Err(error) => Ok(ConnectivityProbe {
                ok: false,
                detail: redact_secrets(&format!("出口探测失败：{error}")),
            }),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(ConnectivityProbe {
            ok: false,
            detail: "浏览器控制面无法直接探测，请在服务端使用".into(),
        })
    }
}

/// B8: validate a profile before it is persisted or used to start the core.
/// The numeric limits are shared with `validate_connection_request_shape`
/// through `validate_profile_parts`; error strings are Chinese and never
/// contain credentials.
#[allow(dead_code)]
pub(crate) fn validate_app_profile(profile: &AppProfile) -> Result<(), ApiError> {
    if profile.subscriptions.len() > MAX_PROFILE_SUBSCRIPTIONS {
        return Err(ApiError::new("订阅数量超过 64 个限制"));
    }
    for subscription in &profile.subscriptions {
        if subscription.source.len() > MAX_SUBSCRIPTION_SOURCE_BYTES {
            return Err(ApiError::new("订阅来源内容超过 64 KiB 限制"));
        }
        if subscription.nodes.len() > MAX_NODES_PER_SUBSCRIPTION {
            return Err(ApiError::new("单个订阅节点数超过 4096 个限制"));
        }
    }
    validate_profile_parts(
        profile.subscriptions.iter().flat_map(|s| s.nodes.iter()),
        profile.custom_rules.len(),
        profile.config_script.len(),
        profile.group_selections.len(),
    )
}

/// The same limits for a `ConnectionRequest`, whose nodes are a flat list.
/// Reuses `validate_profile_parts` so the core-restart entry and `save_profile`
/// cannot drift apart (at least `save_profile` + one core-start entry).
#[allow(dead_code)]
pub(crate) fn validate_connection_request_shape(
    request: &ConnectionRequest,
) -> Result<(), ApiError> {
    validate_profile_parts(
        request.nodes.iter(),
        request.custom_rules.len(),
        request.config_script.as_deref().map_or(0, str::len),
        request.group_selections.len(),
    )
}

#[allow(dead_code)]
fn validate_profile_parts<'a>(
    nodes: impl Iterator<Item = &'a ProxyNode>,
    custom_rule_count: usize,
    config_script_len: usize,
    group_selection_count: usize,
) -> Result<(), ApiError> {
    let mut total_nodes = 0usize;
    for node in nodes {
        total_nodes += 1;
        if total_nodes > MAX_TOTAL_NODES {
            return Err(ApiError::new("节点总数超过 16384 个限制"));
        }
        validate_node_tag(&node.tag)?;
    }
    if custom_rule_count > proxy_core::MAX_CUSTOM_RULES {
        return Err(ApiError::new(format!(
            "自定义规则数量超过 {} 条限制",
            proxy_core::MAX_CUSTOM_RULES
        )));
    }
    if config_script_len > MAX_PROFILE_CONFIG_SCRIPT_BYTES {
        return Err(ApiError::new("配置脚本超过 512 KiB 限制"));
    }
    if group_selection_count > MAX_GROUP_SELECTIONS {
        return Err(ApiError::new("代理组选择项超过 4096 个限制"));
    }
    Ok(())
}

#[allow(dead_code)]
fn validate_node_tag(tag: &str) -> Result<(), ApiError> {
    if tag.len() > MAX_NODE_TAG_BYTES {
        return Err(ApiError::new("节点 tag 长度超过 512 字节限制"));
    }
    // An empty tag is tolerated (legacy / unnamed nodes); other obviously
    // malformed values such as control characters are rejected.
    if tag.chars().any(|character| character.is_control()) {
        return Err(ApiError::new("节点 tag 含有非法控制字符"));
    }
    Ok(())
}

pub async fn load_sync_config() -> Result<SyncConfig, ApiError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        run_native_blocking("读取同步配置任务失败", || {
            load_native_sync_config().map(|config| sync::masked_for_client(&config))
        })
        .await
    }
    #[cfg(target_arch = "wasm32")]
    {
        // `sync` is a native module: credentials live in `sync.json`, which a browser
        // must never read. `load_native_sync_config` already answers with a default
        // here; the masking it used to pass through is what does not exist.
        load_native_sync_config()
    }
}

pub async fn save_sync_config(config: SyncConfig) -> Result<SyncConfig, ApiError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        run_native_blocking("保存同步配置任务失败", move || {
            save_native_sync_config(&config).map(|saved| sync::masked_for_client(&saved))
        })
        .await
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = config;
        Err(ApiError::new("浏览器端不能直接写入同步配置"))
    }
}

pub async fn sync_push(
    config: SyncConfig,
    snapshot: SyncSnapshot,
    force: bool,
) -> Result<SyncResult, ApiError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        // B11: keep synchronous file IO off the async executor thread.
        let config = run_native_blocking("保存同步配置任务失败", move || {
            save_native_sync_config(&config)
        })
        .await?;
        let mut result = sync::push(&config, snapshot, force).await?;
        let mut saved = config;
        saved.last_sync_at = result.updated_at;
        saved.last_remote_revision = result.remote_revision.clone();
        let state_result = run_native_blocking("保存同步基线任务失败", move || {
            save_native_sync_state(&saved)
        })
        .await;
        match state_result {
            Ok(()) => result.checkpoint_saved = true,
            Err(error) => {
                append_sync_warning(
                    &mut result,
                    redact_secrets(&format!("远端上传成功，但本地同步基线保存失败: {error}")),
                );
            }
        }
        Ok(result)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (config, snapshot, force);
        Err(ApiError::new("浏览器目标不能直接同步配置"))
    }
}

pub async fn sync_pull(config: SyncConfig) -> Result<SyncPullResult, ApiError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        // B11: keep synchronous file IO off the async executor thread.
        let config = run_native_blocking("保存同步配置任务失败", move || {
            save_native_sync_config(&config)
        })
        .await?;
        let downloaded = sync::pull(&config).await?;
        Ok(SyncPullResult {
            snapshot: downloaded.snapshot,
            remote_revision: downloaded.remote_revision,
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = config;
        Err(ApiError::new("浏览器目标不能直接同步配置"))
    }
}

pub async fn commit_sync_pull(
    config: SyncConfig,
    current_profile: AppProfile,
    snapshot: SyncSnapshot,
    remote_revision: String,
) -> Result<SyncResult, ApiError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = sync::operation_lock().lock().await;
        sync::validate_snapshot(&snapshot)?;
        sync::validate_remote_revision(&remote_revision)?;
        // B11: all three synchronous file operations (config save, profile
        // backup with create_dir_all/copy/read_dir/remove_file, and state
        // save) run off the async executor thread.
        let config = run_native_blocking("保存同步配置任务失败", move || {
            save_native_sync_config(&config)
        })
        .await?;
        // Keep a timestamped copy of the local profile before the remote
        // snapshot overwrites it, so a bad pull can still be rolled back.
        let backup_note =
            run_native_blocking("备份本地配置失败", backup_local_profile_before_sync_pull).await;
        let mut profile = current_profile;
        snapshot.apply_to_profile(&mut profile);
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
            dns_rule_count: snapshot.dns_overrides.as_ref().map(Vec::len),
            remote_revision,
            checkpoint_saved: false,
            warning: None,
        };
        let state_result = run_native_blocking("保存同步基线任务失败", move || {
            save_native_sync_state(&saved)
        })
        .await;
        match state_result {
            Ok(()) => result.checkpoint_saved = true,
            Err(error) => {
                result.warning = Some(redact_secrets(&format!(
                    "远端数据已保存到本机，但同步基线保存失败: {error}"
                )));
            }
        }
        if let Ok(Some(note)) = backup_note {
            append_sync_warning(&mut result, note);
        }
        Ok(result)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (config, current_profile, snapshot, remote_revision);
        Err(ApiError::new("浏览器目标不能直接提交同步配置"))
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
fn load_native_sync_config() -> Result<SyncConfig, ApiError> {
    Ok(SyncConfig::default())
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn save_native_sync_config(_config: &SyncConfig) -> Result<SyncConfig, ApiError> {
    Err(ApiError::new("浏览器端不能直接写入同步配置"))
}

pub async fn preview_subscription(source: String) -> Result<ParseReport, ApiError> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err(ApiError::new("请输入订阅地址或内容"));
    }

    let content = if (source.starts_with("http://") || source.starts_with("https://"))
        && !proxy_core::is_http_proxy_share_link(&source)
    {
        download_subscription(&source).await?
    } else {
        // B13: inline (non-download) subscription content is user-supplied and
        // is fed straight to the parser, so bound its size and line count here
        // (the parser itself cannot be changed from the api crate).
        if source.len() > MAX_SUBSCRIPTION_BYTES {
            return Err(ApiError::new("内联订阅内容超过 10 MiB 限制"));
        }
        const MAX_INLINE_LINES: usize = 200_000;
        if source.lines().count() > MAX_INLINE_LINES {
            return Err(ApiError::new("内联订阅内容行数过多"));
        }
        source
    };
    run_native_blocking("解析订阅任务失败", move || {
        Ok(proxy_core::parse_subscription(&content))
    })
    .await
}

pub async fn core_status() -> Result<ApiCoreStatus, ApiError> {
    run_native_blocking("读取 sing-box 状态任务失败", native_core_status).await
}

pub async fn set_core_enabled(
    enabled: bool,
    request: Option<ConnectionRequest>,
) -> Result<ApiCoreStatus, ApiError> {
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

pub async fn restart_core(request: ConnectionRequest) -> Result<ApiCoreStatus, ApiError> {
    validate_connection_request_shape(&request)?;
    if request.nodes.is_empty() {
        return Err(ApiError::new("请先导入并选择一个节点"));
    }
    proxy_core::validate_custom_rules(&request.custom_rules)
        .map_err(|error| ApiError::new(format!("自定义规则无效: {error}")))?;
    let rule_set_cache = prepare_rule_set_cache_for_request(Some(&request)).await?;
    run_native_blocking("sing-box 重启任务失败", move || {
        restart_native_core(request, rule_set_cache)
    })
    .await
}

pub async fn prepare_tun_mode(request: ConnectionRequest) -> Result<(), ApiError> {
    if !request.tun {
        return Err(ApiError::new("TUN 权限预检需要启用 TUN 的连接配置"));
    }
    if request.nodes.is_empty() {
        return Err(ApiError::new("请先导入并选择一个节点"));
    }
    proxy_core::validate_custom_rules(&request.custom_rules)
        .map_err(|error| ApiError::new(format!("自定义规则无效: {error}")))?;
    let rule_set_cache = prepare_rule_set_cache_for_request(Some(&request)).await?;
    run_native_blocking("TUN 权限预检任务失败", move || {
        prepare_native_tun_mode(request, rule_set_cache)
    })
    .await
}

pub async fn release_tun_mode() -> Result<(), ApiError> {
    run_native_blocking("释放 TUN 权限准备任务失败", release_native_tun_mode).await
}

pub async fn update_rule_sets(force: bool) -> Result<RuleSetUpdateResult, ApiError> {
    native_update_rule_sets(force).await
}

pub async fn validate_config_script(request: ConnectionRequest) -> Result<(), ApiError> {
    run_native_blocking("校验配置脚本任务失败", move || {
        validate_native_config_script(request)
    })
    .await
}

pub async fn preview_proxy_groups(request: ConnectionRequest) -> Result<Vec<ProxyGroup>, ApiError> {
    run_native_blocking("生成代理组预览任务失败", move || {
        preview_native_proxy_groups(request)
    })
    .await
}

pub async fn select_proxy_group(group: String, outbound: String) -> Result<(), ApiError> {
    run_native_blocking("切换代理组任务失败", move || {
        select_native_proxy_group(group, outbound)
    })
    .await
}

pub async fn core_traffic() -> Result<CoreTraffic, ApiError> {
    run_native_blocking("读取 sing-box 流量任务失败", native_core_traffic).await
}

pub async fn core_logs(cursor: u64) -> Result<CoreLogBatch, ApiError> {
    run_native_blocking("读取 sing-box 日志任务失败", move || {
        native_core_logs(cursor)
    })
    .await
}

pub async fn set_core_log_collection(enabled: bool) -> Result<(), ApiError> {
    run_native_blocking("切换 sing-box 日志采集任务失败", move || {
        native_set_core_log_collection(enabled)
    })
    .await
}

pub async fn measure_node_latency(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ApiError> {
    measure_native_latency(nodes, proxy_server_nameservers).await
}

pub async fn start_node_latency(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<u64, ApiError> {
    start_native_latency_session(nodes, proxy_server_nameservers).await
}

pub async fn poll_node_latency(session_id: u64) -> Result<LatencyProbeSnapshot, ApiError> {
    run_native_blocking("读取节点测速任务失败", move || {
        poll_native_latency_session(session_id)
    })
    .await
}

pub async fn system_proxy_status() -> Result<SystemProxyStatus, ApiError> {
    run_native_blocking("系统代理状态读取任务失败", native_system_proxy_status).await
}

pub async fn set_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ApiError> {
    run_native_blocking("系统代理设置任务失败", move || {
        set_native_system_proxy(enabled)
    })
    .await
}

pub async fn copy_proxy_environment_variables() -> Result<(), ApiError> {
    run_native_blocking(
        "代理环境变量复制任务失败",
        native_copy_proxy_environment_variables,
    )
    .await
}

pub async fn list_running_processes() -> Result<Vec<RunningProcess>, ApiError> {
    run_native_blocking("进程列表读取任务失败", native_list_running_processes).await
}

// 内聚子系统拆分；`pub(crate) use ...::*` 保持 `use super::*` 的共享作用域。
// 某些模块在 wasm32 下内容被 cfg 全部裁掉，glob 为空属于正常情况。
mod latency;
#[allow(unused_imports)]
pub(crate) use latency::*;
mod outbound;
#[allow(unused_imports)]
pub(crate) use outbound::*;
mod rule_set_cache;
#[allow(unused_imports)]
pub(crate) use rule_set_cache::*;
mod core;
#[allow(unused_imports)]
pub(crate) use core::*;
mod platform;
#[allow(unused_imports)]
pub(crate) use platform::*;
mod atomic_write;
#[allow(unused_imports)]
pub(crate) use atomic_write::*;
#[cfg(test)]
mod tests;

// 进程入口（桌面 runner）直接调用这三个，必须保持 crate 根公开；
// 它们只在桌面平台存在，因此 re-export 也要跟平台 cfg 保持一致。
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use platform::{
    cleanup_stale_system_proxy_sentinel, run_tun_helper_from_args, shutdown_native_runtime,
};
