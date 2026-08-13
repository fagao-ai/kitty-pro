//! Rust lifecycle API for the statically linked sing-box core.
//!
//! The actual networking engine is written in Go by the sing-box project. The
//! `embedded-core` feature compiles a small C ABI bridge and links it into the
//! host application, so no `sing-box` executable is spawned or discovered at
//! runtime.

#[cfg(feature = "embedded-core")]
use proxy_core::{build_singbox_config, validate_custom_rules};
use proxy_core::{ConnectionRequest, SingBoxOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[cfg(target_os = "android")]
pub mod android;
#[cfg(all(
    feature = "embedded-core",
    any(test, target_os = "windows", target_os = "linux")
))]
pub mod desktop_helper;
#[cfg(all(feature = "embedded-core", target_os = "macos"))]
pub mod macos;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    Unavailable,
    Stopped,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreStatus {
    pub state: CoreState,
    pub version: Option<String>,
    pub platform_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficStats {
    pub upload_total: u64,
    pub download_total: u64,
    pub active_connections: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub sequence: u64,
    pub timestamp: String,
    pub level: String,
    pub message: String,
    #[serde(default)]
    pub outbound_chain: Vec<String>,
    #[serde(default)]
    pub source_ip: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogBatch {
    pub next_cursor: u64,
    pub entries: Vec<LogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub tag: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

pub type LatencyProbeResult = ProbeResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    pub embedded_core: bool,
    pub requires_mobile_vpn_bridge: bool,
    pub browser_control_only: bool,
}

impl PlatformCapabilities {
    pub fn current() -> Self {
        Self {
            embedded_core: cfg!(feature = "embedded-core"),
            requires_mobile_vpn_bridge: cfg!(any(target_os = "android", target_os = "ios")),
            browser_control_only: cfg!(target_arch = "wasm32"),
        }
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("当前构建未包含嵌入式 sing-box 内核")]
    EmbeddedCoreUnavailable,
    #[error("嵌入式 sing-box 配置无效: {0}")]
    InvalidConfig(String),
    #[error("嵌入式 sing-box 已经在运行")]
    AlreadyRunning,
    #[error("嵌入式 sing-box 未运行")]
    NotRunning,
    #[error("读取嵌入式 sing-box 流量失败: {0}")]
    TrafficUnavailable(String),
    #[error("读取嵌入式 sing-box 日志失败: {0}")]
    LogsUnavailable(String),
    #[error("sing-box 规则集无效: {0}")]
    RuleSetInvalid(String),
    #[error("切换 sing-box 代理组失败: {0}")]
    SelectionUnavailable(String),
    #[error("sing-box 节点探测失败: {0}")]
    ProbeUnavailable(String),
    #[error("配置序列化失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Android VPN 服务错误: {0}")]
    AndroidVpn(String),
    #[error("macOS TUN helper 错误: {0}")]
    MacosTunHelper(String),
    #[error("桌面 TUN helper 错误: {0}")]
    DesktopTunHelper(String),
    #[error("TUN 权限尚未准备，请先关闭并重新开启 TUN 模式")]
    TunPermissionNotPrepared,
}

#[cfg(feature = "embedded-core")]
#[derive(Debug)]
pub(crate) struct EmbeddedCore {
    handle: Option<u64>,
    version: String,
}

#[cfg(feature = "embedded-core")]
impl EmbeddedCore {
    pub(crate) fn new() -> Self {
        Self {
            handle: None,
            version: ffi::version(),
        }
    }

    pub(crate) fn start_config(&mut self, config: &Value) -> Result<(), CoreError> {
        if self.handle.is_some() {
            return Err(CoreError::AlreadyRunning);
        }
        let content = serde_json::to_string(config)?;
        self.handle = Some(ffi::start(&content)?);
        Ok(())
    }

    pub(crate) fn stop(&mut self) -> Result<(), CoreError> {
        let handle = self.handle.ok_or(CoreError::NotRunning)?;
        let result = ffi::stop(handle);
        self.handle = None;
        result
    }

    pub(crate) fn is_running(&self) -> bool {
        self.handle.is_some()
    }

    pub(crate) fn status(&self) -> CoreStatus {
        CoreStatus {
            state: if self.is_running() {
                CoreState::Running
            } else {
                CoreState::Stopped
            },
            version: Some(self.version.clone()),
            platform_note: None,
        }
    }

    pub(crate) fn traffic(&self) -> Result<TrafficStats, CoreError> {
        let handle = self.handle.ok_or(CoreError::NotRunning)?;
        let payload = ffi::traffic(handle)?;
        serde_json::from_str(&payload)
            .map_err(|error| CoreError::TrafficUnavailable(error.to_string()))
    }

    pub(crate) fn logs(&self, cursor: u64) -> Result<LogBatch, CoreError> {
        let handle = self.handle.ok_or(CoreError::NotRunning)?;
        let payload = ffi::logs(handle, cursor)?;
        serde_json::from_str(&payload)
            .map_err(|error| CoreError::LogsUnavailable(error.to_string()))
    }

    pub(crate) fn set_log_enabled(&self, enabled: bool) -> Result<(), CoreError> {
        let handle = self.handle.ok_or(CoreError::NotRunning)?;
        ffi::set_log_enabled(handle, enabled)
    }

    pub(crate) fn select_outbound(&self, group: &str, outbound: &str) -> Result<(), CoreError> {
        let handle = self.handle.ok_or(CoreError::NotRunning)?;
        ffi::select_outbound(handle, group, outbound)
    }

    pub(crate) fn probe_outbound(
        &self,
        tag: &str,
        probe_url: &str,
    ) -> Result<LatencyProbeResult, CoreError> {
        let handle = self.handle.ok_or(CoreError::NotRunning)?;
        ffi::probe_outbound(handle, tag, probe_url)
    }
}

#[cfg(feature = "embedded-core")]
impl Drop for EmbeddedCore {
    fn drop(&mut self) {
        if self.handle.is_some() {
            let _ = self.stop();
        }
    }
}

#[cfg(feature = "embedded-core")]
#[derive(Debug)]
enum CoreBackend {
    Stopped,
    Embedded(EmbeddedCore),
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    Privileged(TunHelper),
}

#[cfg(all(feature = "embedded-core", target_os = "macos"))]
type TunHelper = macos::PrivilegedCore;

#[cfg(all(
    feature = "embedded-core",
    any(target_os = "windows", target_os = "linux")
))]
type TunHelper = desktop_helper::PrivilegedCore;

#[cfg(all(
    feature = "embedded-core",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
#[derive(Debug)]
struct PreparedConfig {
    config: Value,
    tun_helper: Option<TunHelper>,
}

#[derive(Debug)]
pub struct SingBox {
    #[cfg(feature = "embedded-core")]
    backend: CoreBackend,
    #[cfg(feature = "embedded-core")]
    version: String,
    #[cfg(all(
        feature = "embedded-core",
        any(target_os = "macos", target_os = "windows", target_os = "linux")
    ))]
    prepared_config: Option<PreparedConfig>,
}

impl SingBox {
    pub fn new() -> Result<Self, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            Ok(Self {
                backend: CoreBackend::Stopped,
                version: ffi::version(),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                prepared_config: None,
            })
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    /// Kept as the construction entry point for callers that previously used
    /// process discovery. It no longer checks `PATH` or `SING_BOX_PATH`.
    pub fn discover() -> Result<Self, CoreError> {
        Self::new()
    }

    pub fn version(&self) -> Result<String, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            Ok(self.version.clone())
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    /// Validate a candidate configuration without starting an inbound or
    /// changing routes. On macOS, a TUN candidate also obtains the separate
    /// helper's administrator authorization while the currently running core
    /// remains untouched. Call this before stopping a live core for restart.
    pub fn prepare_config(&mut self, config: &Value) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            check_config(config)?;

            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                let needs_tun_helper = config_uses_tun(config);
                if let Some(prepared) = self.prepared_config.as_mut() {
                    let helper_ready = prepared
                        .tun_helper
                        .as_ref()
                        .is_some_and(|helper| helper.is_ready().is_ok());
                    if !needs_tun_helper || helper_ready {
                        prepared.config = config.clone();
                        if !needs_tun_helper {
                            prepared.tun_helper = None;
                        }
                        return Ok(());
                    }
                }

                let current_helper_ready = needs_tun_helper
                    && matches!(
                        &self.backend,
                        CoreBackend::Privileged(helper) if helper.is_ready().is_ok()
                    );
                if current_helper_ready {
                    self.prepared_config = Some(PreparedConfig {
                        config: config.clone(),
                        tun_helper: None,
                    });
                    return Ok(());
                }

                let tun_helper = if needs_tun_helper {
                    Some(TunHelper::launch()?)
                } else {
                    None
                };
                self.prepared_config = Some(PreparedConfig {
                    config: config.clone(),
                    tun_helper,
                });
                Ok(())
            }

            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            Ok(())
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = config;
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    /// Discard a preflight helper when a restart exits before stopping the
    /// currently running core.
    pub fn discard_prepared_config(&mut self) {
        #[cfg(all(
            feature = "embedded-core",
            any(target_os = "macos", target_os = "windows", target_os = "linux")
        ))]
        {
            self.prepared_config = None;
            let stopped_helper = matches!(
                &self.backend,
                CoreBackend::Privileged(helper) if matches!(helper.is_running(), Ok(false))
            );
            if stopped_helper {
                if let CoreBackend::Privileged(mut helper) =
                    std::mem::replace(&mut self.backend, CoreBackend::Stopped)
                {
                    helper.shutdown();
                }
            }
        }
    }

    pub fn start(
        &mut self,
        request: &ConnectionRequest,
        options: &SingBoxOptions,
    ) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            if self.is_running()? {
                return Err(CoreError::AlreadyRunning);
            }
            validate_custom_rules(&request.custom_rules)
                .map_err(|error| CoreError::InvalidConfig(error.to_string()))?;
            let config = build_singbox_config(request, options);
            self.start_config(&config)
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = (request, options);
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    pub fn start_config(&mut self, config: &Value) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            if self.is_running()? {
                return Err(CoreError::AlreadyRunning);
            }
            // Static validation must finish before macOS asks for administrator
            // privileges. This never opens an inbound or changes host routes.
            check_config(config)?;

            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            let mut prepared = self.prepared_config.take();

            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            if config_uses_tun(config) {
                let previous = std::mem::replace(&mut self.backend, CoreBackend::Stopped);
                let mut previous_helper = match previous {
                    CoreBackend::Stopped => None,
                    CoreBackend::Privileged(helper) => Some(helper),
                    CoreBackend::Embedded(_) => None,
                };
                if previous_helper
                    .as_ref()
                    .is_some_and(|helper| helper.is_ready().is_err())
                {
                    if let Some(mut stale) = previous_helper.take() {
                        stale.shutdown();
                    }
                }
                let prepared_helper = prepared
                    .as_mut()
                    .and_then(|prepared| prepared.tun_helper.take());
                let reusing_previous_helper =
                    prepared_helper.is_none() && previous_helper.is_some();
                let mut helper = match prepared_helper {
                    Some(helper) => helper,
                    None => match previous_helper.take() {
                        Some(helper) => helper,
                        // Permission prompts belong to `prepare_config`, which
                        // is called by the TUN switch. Starting the core must
                        // never unexpectedly request administrator access.
                        None => return Err(CoreError::TunPermissionNotPrepared),
                    },
                };
                return match helper.start(config) {
                    Ok(()) => {
                        if let Some(mut old_helper) = previous_helper {
                            old_helper.shutdown();
                        }
                        self.backend = CoreBackend::Privileged(helper);
                        Ok(())
                    }
                    Err(error) => {
                        if reusing_previous_helper {
                            self.backend = CoreBackend::Privileged(helper);
                        } else {
                            helper.shutdown();
                            if let Some(old_helper) = previous_helper {
                                self.backend = CoreBackend::Privileged(old_helper);
                            }
                        }
                        Err(error)
                    }
                };
            }

            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                drop(prepared);
                let previous = std::mem::replace(&mut self.backend, CoreBackend::Stopped);
                let mut core = EmbeddedCore::new();
                match core.start_config(config) {
                    Ok(()) => {
                        if let CoreBackend::Privileged(mut helper) = previous {
                            helper.shutdown();
                        }
                        self.backend = CoreBackend::Embedded(core);
                        Ok(())
                    }
                    Err(error) => {
                        self.backend = previous;
                        Err(error)
                    }
                }
            }

            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            {
                let mut core = EmbeddedCore::new();
                core.start_config(config)?;
                self.backend = CoreBackend::Embedded(core);
                Ok(())
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = config;
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    pub fn stop(&mut self) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            let backend = std::mem::replace(&mut self.backend, CoreBackend::Stopped);
            match backend {
                CoreBackend::Stopped => Err(CoreError::NotRunning),
                CoreBackend::Embedded(mut core) => match core.stop() {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        #[cfg(any(
                            target_os = "macos",
                            target_os = "windows",
                            target_os = "linux"
                        ))]
                        {
                            self.prepared_config = None;
                        }
                        self.backend = CoreBackend::Embedded(core);
                        Err(error)
                    }
                },
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(mut helper) => {
                    let result = helper.stop();
                    if result.is_err() {
                        self.prepared_config = None;
                    }
                    if result.is_err() || self.prepared_config.is_some() {
                        self.backend = CoreBackend::Privileged(helper);
                    } else {
                        helper.shutdown();
                    }
                    result
                }
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    /// Tear down a backend whose RPC state can no longer be trusted. The
    /// macOS helper closes its reverse lease, so its watchdog guarantees that
    /// the privileged process exits even if a normal stop request is stuck.
    pub fn force_shutdown(&mut self) {
        #[cfg(feature = "embedded-core")]
        {
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            {
                self.prepared_config = None;
            }
            let backend = std::mem::replace(&mut self.backend, CoreBackend::Stopped);
            match backend {
                CoreBackend::Stopped => {}
                CoreBackend::Embedded(mut core) => {
                    if core.is_running() {
                        let _ = core.stop();
                    }
                }
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(mut helper) => helper.shutdown(),
            }
        }
    }

    pub fn is_running(&self) -> Result<bool, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            match &self.backend {
                CoreBackend::Stopped => Ok(false),
                CoreBackend::Embedded(core) => Ok(core.is_running()),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(helper) => helper.is_running(),
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    pub fn status(&self) -> CoreStatus {
        #[cfg(feature = "embedded-core")]
        {
            match &self.backend {
                CoreBackend::Stopped => CoreStatus {
                    state: CoreState::Stopped,
                    version: Some(self.version.clone()),
                    platform_note: None,
                },
                CoreBackend::Embedded(core) => core.status(),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(helper) => helper.status(),
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        unavailable_status()
    }

    pub fn traffic(&self) -> Result<TrafficStats, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            match &self.backend {
                CoreBackend::Stopped => Err(CoreError::NotRunning),
                CoreBackend::Embedded(core) => core.traffic(),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(helper) => helper.traffic(),
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    pub fn logs(&self, cursor: u64) -> Result<LogBatch, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            match &self.backend {
                CoreBackend::Stopped => Err(CoreError::NotRunning),
                CoreBackend::Embedded(core) => core.logs(cursor),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(helper) => helper.logs(cursor),
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = cursor;
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    pub fn set_log_enabled(&self, enabled: bool) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            match &self.backend {
                CoreBackend::Stopped => Err(CoreError::NotRunning),
                CoreBackend::Embedded(core) => core.set_log_enabled(enabled),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(helper) => helper.set_log_enabled(enabled),
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = enabled;
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    pub fn select_outbound(&self, group: &str, outbound: &str) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            match &self.backend {
                CoreBackend::Stopped => Err(CoreError::NotRunning),
                CoreBackend::Embedded(core) => core.select_outbound(group, outbound),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(helper) => helper.select_outbound(group, outbound),
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = (group, outbound);
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    pub fn probe_config(
        config: &Value,
        node_tags: &[String],
        probe_url: &str,
    ) -> Result<Vec<LatencyProbeResult>, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            if config_uses_tun(config) {
                return Err(CoreError::ProbeUnavailable(
                    "节点探测配置不能包含 TUN 入站".to_string(),
                ));
            }

            let config = serde_json::to_string(config)?;
            let node_tags = serde_json::to_string(node_tags)?;
            let payload = ffi::probe(&config, &node_tags, probe_url)?;
            serde_json::from_str(&payload)
                .map_err(|error| CoreError::ProbeUnavailable(error.to_string()))
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = (config, node_tags, probe_url);
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    pub fn probe_outbound(
        &self,
        tag: &str,
        probe_url: &str,
    ) -> Result<LatencyProbeResult, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            match &self.backend {
                CoreBackend::Stopped => Err(CoreError::NotRunning),
                CoreBackend::Embedded(core) => core.probe_outbound(tag, probe_url),
                #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
                CoreBackend::Privileged(helper) => helper.probe_outbound(tag, probe_url),
            }
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = (tag, probe_url);
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }
}

impl Drop for SingBox {
    fn drop(&mut self) {
        if self.is_running().unwrap_or(false) {
            let _ = self.stop();
        }
    }
}

#[cfg(all(
    feature = "embedded-core",
    any(test, target_os = "macos", target_os = "windows", target_os = "linux")
))]
fn config_uses_tun(config: &Value) -> bool {
    config
        .get("inbounds")
        .and_then(Value::as_array)
        .is_some_and(|inbounds| {
            inbounds
                .iter()
                .any(|inbound| inbound.get("type").and_then(Value::as_str) == Some("tun"))
        })
}

pub fn probe_outbounds(
    config: &str,
    node_tags: &[String],
    probe_url: &str,
) -> Result<Vec<ProbeResult>, CoreError> {
    #[cfg(all(feature = "embedded-core", target_os = "android"))]
    {
        return android::probe(config, node_tags, probe_url);
    }

    #[cfg(all(feature = "embedded-core", not(target_os = "android")))]
    {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            // The Go parser accepts extended JSON. Parsing failures must not
            // fall through to a temporary start, otherwise a commented TUN
            // config could bypass the privileged-backend guard.
            let config_value = serde_json::from_str::<Value>(config).map_err(|error| {
                CoreError::ProbeUnavailable(format!("桌面节点探测配置必须是标准 JSON: {error}"))
            })?;
            SingBox::probe_config(&config_value, node_tags, probe_url)
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let node_tags = serde_json::to_string(node_tags)?;
            let payload = ffi::probe(config, &node_tags, probe_url)?;
            serde_json::from_str(&payload)
                .map_err(|error| CoreError::ProbeUnavailable(error.to_string()))
        }
    }

    #[cfg(not(feature = "embedded-core"))]
    {
        let _ = (config, node_tags, probe_url);
        Err(CoreError::EmbeddedCoreUnavailable)
    }
}

pub fn validate_rule_set_file(path: &std::path::Path) -> Result<(), CoreError> {
    #[cfg(feature = "embedded-core")]
    {
        ffi::validate_rule_set_file(path)
    }

    #[cfg(not(feature = "embedded-core"))]
    {
        let _ = path;
        Err(CoreError::EmbeddedCoreUnavailable)
    }
}

/// Parse and construct a sing-box configuration without starting any inbound,
/// opening a TUN interface, or changing host routes.
pub fn check_config(config: &Value) -> Result<(), CoreError> {
    #[cfg(feature = "embedded-core")]
    {
        let content = serde_json::to_string(config)?;
        ffi::check_config(&content)
    }

    #[cfg(not(feature = "embedded-core"))]
    {
        let _ = config;
        Err(CoreError::EmbeddedCoreUnavailable)
    }
}

pub fn unavailable_status() -> CoreStatus {
    let capabilities = PlatformCapabilities::current();
    let platform_note = if capabilities.browser_control_only {
        "浏览器不能加载原生网络内核，可连接远程 Kitty Pro 核心服务".to_string()
    } else if capabilities.requires_mobile_vpn_bridge {
        "移动端内核需要通过 iOS NetworkExtension 或 Android VpnService 注入 TUN".to_string()
    } else {
        "当前构建没有链接嵌入式 sing-box 内核".to_string()
    };
    CoreStatus {
        state: CoreState::Unavailable,
        version: None,
        platform_note: Some(platform_note),
    }
}

#[cfg(feature = "embedded-core")]
mod ffi {
    use super::CoreError;
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    unsafe extern "C" {
        fn kitty_singbox_probe(
            config_content: *const c_char,
            node_tags_json: *const c_char,
            probe_url: *const c_char,
        ) -> *mut c_char;
        fn kitty_singbox_probe_outbound(
            handle: u64,
            tag: *const c_char,
            probe_url: *const c_char,
        ) -> *mut c_char;
        fn kitty_singbox_start(config_content: *const c_char) -> u64;
        fn kitty_singbox_stop(handle: u64) -> i32;
        fn kitty_singbox_version() -> *mut c_char;
        fn kitty_singbox_last_error() -> *mut c_char;
        fn kitty_singbox_traffic(handle: u64) -> *mut c_char;
        fn kitty_singbox_logs(handle: u64, cursor: u64) -> *mut c_char;
        fn kitty_singbox_set_log_enabled(handle: u64, enabled: i32) -> i32;
        fn kitty_singbox_validate_rule_set_file(path: *const c_char) -> *mut c_char;
        fn kitty_singbox_check_config(config_content: *const c_char) -> *mut c_char;
        fn kitty_singbox_select_outbound(
            handle: u64,
            group: *const c_char,
            outbound: *const c_char,
        ) -> i32;
        fn kitty_singbox_free_string(value: *mut c_char);
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_start(
            config_content: *const c_char,
            tun_fd: std::os::raw::c_int,
            data_path: *const c_char,
        ) -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_stop() -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_traffic() -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_logs(cursor: u64) -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_set_log_enabled(enabled: i32);
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_select_outbound(
            group: *const c_char,
            outbound: *const c_char,
        ) -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_probe(
            config_content: *const c_char,
            node_tags_json: *const c_char,
            probe_url: *const c_char,
            data_path: *const c_char,
            result: *mut *mut c_char,
        ) -> *mut c_char;
    }

    pub fn start(config: &str) -> Result<u64, CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::InvalidConfig("配置中包含 NUL 字符".to_string()))?;
        let handle = unsafe { kitty_singbox_start(config.as_ptr()) };
        if handle == 0 {
            return Err(CoreError::InvalidConfig(last_error()));
        }
        Ok(handle)
    }

    pub fn probe(config: &str, node_tags_json: &str, probe_url: &str) -> Result<String, CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::ProbeUnavailable("配置中包含 NUL 字符".to_string()))?;
        let node_tags_json = CString::new(node_tags_json)
            .map_err(|_| CoreError::ProbeUnavailable("节点标识中包含 NUL 字符".to_string()))?;
        let probe_url = CString::new(probe_url)
            .map_err(|_| CoreError::ProbeUnavailable("探测地址中包含 NUL 字符".to_string()))?;
        let payload = take_string(unsafe {
            kitty_singbox_probe(config.as_ptr(), node_tags_json.as_ptr(), probe_url.as_ptr())
        });
        if payload.is_empty() {
            return Err(CoreError::ProbeUnavailable(last_error()));
        }
        Ok(payload)
    }

    pub fn probe_outbound(
        handle: u64,
        tag: &str,
        probe_url: &str,
    ) -> Result<super::LatencyProbeResult, CoreError> {
        let tag = CString::new(tag)
            .map_err(|_| CoreError::ProbeUnavailable("节点标识中包含 NUL 字符".to_string()))?;
        let probe_url = CString::new(probe_url)
            .map_err(|_| CoreError::ProbeUnavailable("探测地址中包含 NUL 字符".to_string()))?;
        let payload = take_string(unsafe {
            kitty_singbox_probe_outbound(handle, tag.as_ptr(), probe_url.as_ptr())
        });
        if payload.is_empty() {
            return Err(CoreError::ProbeUnavailable(last_error()));
        }
        serde_json::from_str(&payload)
            .map_err(|error| CoreError::ProbeUnavailable(error.to_string()))
    }

    pub fn stop(handle: u64) -> Result<(), CoreError> {
        if unsafe { kitty_singbox_stop(handle) } == 0 {
            return Err(CoreError::NotRunning);
        }
        Ok(())
    }

    pub fn version() -> String {
        take_string(unsafe { kitty_singbox_version() })
    }

    pub fn traffic(handle: u64) -> Result<String, CoreError> {
        let payload = take_string(unsafe { kitty_singbox_traffic(handle) });
        if payload.is_empty() {
            return Err(CoreError::TrafficUnavailable(last_error()));
        }
        Ok(payload)
    }

    pub fn logs(handle: u64, cursor: u64) -> Result<String, CoreError> {
        let payload = take_string(unsafe { kitty_singbox_logs(handle, cursor) });
        if payload.is_empty() {
            return Err(CoreError::LogsUnavailable(last_error()));
        }
        Ok(payload)
    }

    pub fn set_log_enabled(handle: u64, enabled: bool) -> Result<(), CoreError> {
        if unsafe { kitty_singbox_set_log_enabled(handle, i32::from(enabled)) } == 0 {
            return Err(CoreError::LogsUnavailable(last_error()));
        }
        Ok(())
    }

    pub fn select_outbound(handle: u64, group: &str, outbound: &str) -> Result<(), CoreError> {
        let group = CString::new(group)
            .map_err(|_| CoreError::SelectionUnavailable("分组名称包含 NUL 字符".to_string()))?;
        let outbound = CString::new(outbound)
            .map_err(|_| CoreError::SelectionUnavailable("节点名称包含 NUL 字符".to_string()))?;
        if unsafe { kitty_singbox_select_outbound(handle, group.as_ptr(), outbound.as_ptr()) } == 0
        {
            return Err(CoreError::SelectionUnavailable(last_error()));
        }
        Ok(())
    }

    pub fn validate_rule_set_file(path: &std::path::Path) -> Result<(), CoreError> {
        let path = path
            .to_str()
            .ok_or_else(|| CoreError::RuleSetInvalid("规则文件路径不是有效 UTF-8".to_string()))?;
        let path = CString::new(path)
            .map_err(|_| CoreError::RuleSetInvalid("规则文件路径包含 NUL 字符".to_string()))?;
        let error = unsafe { kitty_singbox_validate_rule_set_file(path.as_ptr()) };
        if error.is_null() {
            Ok(())
        } else {
            Err(CoreError::RuleSetInvalid(take_string(error)))
        }
    }

    pub fn check_config(config: &str) -> Result<(), CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::InvalidConfig("配置中包含 NUL 字符".to_string()))?;
        let error = unsafe { kitty_singbox_check_config(config.as_ptr()) };
        if error.is_null() {
            Ok(())
        } else {
            Err(CoreError::InvalidConfig(take_string(error)))
        }
    }
    #[cfg(target_os = "android")]
    pub fn android_start(config: &str, tun_fd: i32, data_path: &str) -> Result<(), CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::InvalidConfig("配置中包含 NUL 字符".to_string()))?;
        let data_path = CString::new(data_path)
            .map_err(|_| CoreError::AndroidVpn("应用数据目录包含 NUL 字符".to_string()))?;
        let error =
            unsafe { kitty_singbox_android_start(config.as_ptr(), tun_fd, data_path.as_ptr()) };
        take_optional_error(error)
    }

    #[cfg(target_os = "android")]
    pub fn android_stop() -> Result<(), CoreError> {
        take_optional_error(unsafe { kitty_singbox_android_stop() })
    }

    #[cfg(target_os = "android")]
    pub fn android_traffic() -> Result<String, CoreError> {
        let payload = take_string(unsafe { kitty_singbox_android_traffic() });
        if payload.is_empty() {
            return Err(CoreError::TrafficUnavailable(last_error()));
        }
        Ok(payload)
    }

    #[cfg(target_os = "android")]
    pub fn android_logs(cursor: u64) -> Result<String, CoreError> {
        let payload = take_string(unsafe { kitty_singbox_android_logs(cursor) });
        if payload.is_empty() {
            return Err(CoreError::LogsUnavailable(last_error()));
        }
        Ok(payload)
    }

    #[cfg(target_os = "android")]
    pub fn android_set_log_enabled(enabled: bool) {
        unsafe { kitty_singbox_android_set_log_enabled(i32::from(enabled)) }
    }

    #[cfg(target_os = "android")]
    pub fn android_select_outbound(group: &str, outbound: &str) -> Result<(), CoreError> {
        let group = CString::new(group)
            .map_err(|_| CoreError::SelectionUnavailable("分组名称包含 NUL 字符".to_string()))?;
        let outbound = CString::new(outbound)
            .map_err(|_| CoreError::SelectionUnavailable("节点名称包含 NUL 字符".to_string()))?;
        take_optional_error(unsafe {
            kitty_singbox_android_select_outbound(group.as_ptr(), outbound.as_ptr())
        })
    }

    #[cfg(target_os = "android")]
    pub fn android_probe(
        config: &str,
        node_tags_json: &str,
        probe_url: &str,
        data_path: &str,
    ) -> Result<String, CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::InvalidConfig("配置中包含 NUL 字符".to_string()))?;
        let node_tags_json = CString::new(node_tags_json)
            .map_err(|_| CoreError::AndroidVpn("节点标识包含 NUL 字符".to_string()))?;
        let probe_url = CString::new(probe_url)
            .map_err(|_| CoreError::AndroidVpn("探测地址包含 NUL 字符".to_string()))?;
        let data_path = CString::new(data_path)
            .map_err(|_| CoreError::AndroidVpn("应用数据目录包含 NUL 字符".to_string()))?;
        let mut result = std::ptr::null_mut();
        let error = unsafe {
            kitty_singbox_android_probe(
                config.as_ptr(),
                node_tags_json.as_ptr(),
                probe_url.as_ptr(),
                data_path.as_ptr(),
                &mut result,
            )
        };
        take_optional_error(error)?;
        Ok(take_string(result))
    }

    fn last_error() -> String {
        let message = take_string(unsafe { kitty_singbox_last_error() });
        if message.is_empty() {
            "未知错误".to_string()
        } else {
            message
        }
    }

    fn take_string(value: *mut c_char) -> String {
        if value.is_null() {
            return String::new();
        }
        let result = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        unsafe { kitty_singbox_free_string(value) };
        result
    }

    #[cfg(target_os = "android")]
    fn take_optional_error(value: *mut c_char) -> Result<(), CoreError> {
        if value.is_null() {
            return Ok(());
        }
        Err(CoreError::AndroidVpn(take_string(value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_core_is_constructed_without_a_binary() {
        let core = SingBox::new().expect("embedded core should be linked");
        assert!(!core
            .version()
            .expect("version should be available")
            .is_empty());
        assert_eq!(core.status().state, CoreState::Stopped);
    }

    #[test]
    fn embedded_core_checks_tun_config_without_starting_it() {
        let config = serde_json::json!({
            "log": { "level": "error" },
            "inbounds": [{
                "type": "tun",
                "tag": "tun-in",
                "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
                "auto_route": true,
                "strict_route": true,
                "stack": "system"
            }],
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "route": { "auto_detect_interface": true, "final": "direct" },
        });

        check_config(&config).expect("valid TUN config should pass static validation");
    }

    #[test]
    fn starting_tun_never_requests_privileges_without_switch_preparation() {
        let config = serde_json::json!({
            "log": { "level": "error" },
            "inbounds": [{
                "type": "tun",
                "tag": "tun-in",
                "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
                "auto_route": true,
                "strict_route": true,
                "stack": "system"
            }],
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "route": { "auto_detect_interface": true, "final": "direct" },
        });
        let mut core = SingBox::new().expect("embedded core should be linked");

        let error = core
            .start_config(&config)
            .expect_err("TUN start must require explicit switch preparation");

        assert!(matches!(error, CoreError::TunPermissionNotPrepared));
        assert!(!core.is_running().expect("core state should be readable"));
    }

    #[test]
    fn embedded_core_rejects_invalid_config_before_start() {
        let config = serde_json::json!({
            "inbounds": [{ "type": "not-a-real-inbound" }],
        });

        let error = check_config(&config).expect_err("invalid config should be rejected");
        assert!(matches!(error, CoreError::InvalidConfig(_)));
    }

    #[test]
    fn only_a_tun_inbound_requires_the_privileged_backend() {
        let tun = serde_json::json!({
            "inbounds": [
                { "type": "mixed" },
                { "type": "tun", "tag": "tun-in" }
            ]
        });
        let non_tun = serde_json::json!({
            "inbounds": [{ "type": "mixed", "tag": "mixed-in" }],
            "outbounds": [{ "type": "direct", "tag": "tun" }]
        });

        assert!(config_uses_tun(&tun));
        assert!(!config_uses_tun(&non_tun));
        assert!(!config_uses_tun(&serde_json::json!({})));
    }

    #[test]
    fn preparing_a_non_tun_config_does_not_start_the_core() {
        let config = serde_json::json!({
            "log": { "level": "error" },
            "inbounds": [],
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "route": { "auto_detect_interface": false, "final": "direct" },
        });
        let mut core = SingBox::new().expect("embedded core should be linked");

        core.prepare_config(&config)
            .expect("preparing a non-TUN config should only validate it");

        assert!(!core.is_running().expect("core state should be readable"));
        assert_eq!(core.status().state, CoreState::Stopped);
    }

    #[test]
    fn tun_config_can_be_checked_without_starting_an_interface() {
        let request = ConnectionRequest {
            selected_tag: "direct-node".to_string(),
            nodes: proxy_core::parse_subscription("socks5://127.0.0.1:1080#DirectNode").nodes,
            proxy_server_nameservers: Vec::new(),
            mode: proxy_core::TunnelMode::Global,
            tun: true,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: Default::default(),
        };
        let config = build_singbox_config(&request, &SingBoxOptions::default());

        check_config(&config).expect("TUN config should parse without opening an interface");
    }

    #[test]
    fn rule_mode_tun_dns_ipv4_only_strategy_passes_embedded_parser() {
        // Regression guard for the WeChat image-send fix: the embedded
        // sing-box parser must accept `strategy: "ipv4_only"` on DNS route
        // rules that serve directly-routed (geosite-cn) domains.
        let request = ConnectionRequest {
            selected_tag: "direct-node".to_string(),
            nodes: proxy_core::parse_subscription("socks5://127.0.0.1:1080#DirectNode").nodes,
            proxy_server_nameservers: Vec::new(),
            mode: proxy_core::TunnelMode::Rule,
            tun: true,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: Default::default(),
        };
        let config = build_singbox_config(&request, &SingBoxOptions::default());
        let direct_rule = config["dns"]["rules"]
            .as_array()
            .expect("DNS rules should be an array")
            .iter()
            .find(|rule| rule["rule_set"] == "geosite-cn")
            .expect("geosite-cn DNS rule should exist");
        assert_eq!(direct_rule["strategy"], "ipv4_only");

        check_config(&config).expect("Rule-mode TUN config should pass static validation");
    }

    #[test]
    fn embedded_probe_without_clash_api_returns_node_errors() {
        let config = serde_json::json!({
            "log": { "level": "error" },
            "inbounds": [],
            "outbounds": [{ "type": "direct", "tag": "direct" }],
            "route": { "auto_detect_interface": false, "final": "direct" },
        });
        let results = probe_outbounds(
            &serde_json::to_string(&config).expect("probe config should serialize"),
            &["missing".to_string()],
            "https://www.gstatic.com/generate_204",
        )
        .expect("probe without Clash API should return a result instead of aborting");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tag, "missing");
        assert!(results[0].latency_ms.is_none());
        assert!(results[0].error.is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn commented_tun_probe_config_is_rejected_before_start() {
        let config = r#"{
            // Extended JSON comments must not bypass the macOS TUN guard.
            "inbounds": [{ "type": "tun", "tag": "tun-in" }],
            "outbounds": [{ "type": "direct", "tag": "direct" }]
        }"#;

        let error = probe_outbounds(
            config,
            &["direct".to_string()],
            "https://example.com/generate_204",
        )
        .expect_err("commented TUN probe config must be rejected");

        assert!(matches!(error, CoreError::ProbeUnavailable(_)));
    }

    #[test]
    fn embedded_core_starts_without_an_external_executable() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::thread;
        use std::time::Duration;

        let nodes = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge\n\
             anytls://secret@127.0.0.1:443?insecure=1&sni=edge.example.com#AnyTLS",
        )
        .nodes;
        assert!(nodes
            .iter()
            .any(|node| node.protocol == proxy_core::ProxyProtocol::AnyTls));
        let selected_tag = nodes[0].tag.clone();
        let request = ConnectionRequest {
            selected_tag: selected_tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: proxy_core::TunnelMode::Direct,
            tun: false,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: Default::default(),
        };
        let target_listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("local HTTP listener should bind");
        let target_port = target_listener
            .local_addr()
            .expect("local HTTP listener should expose its port")
            .port();
        let target = thread::spawn(move || {
            let (mut stream, _) = target_listener
                .accept()
                .expect("sing-box should connect to the local HTTP listener");
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("local HTTP listener should respond");
        });
        let (mixed_port, traffic_port) = available_loopback_ports();
        let options = SingBoxOptions {
            mixed_port,
            listen: "127.0.0.1".to_string(),
            log_level: "error".to_string(),
            traffic_api_port: Some(traffic_port),
            traffic_api_secret: Some("test-traffic-secret".to_string()),
            rule_set_cache: None,
            cache_file: None,
        };
        let mut core = SingBox::new().expect("embedded core should be linked");

        core.start(&request, &options)
            .expect("embedded core should start");
        core.set_log_enabled(true)
            .expect("embedded core log collection should start");
        assert!(core.is_running().expect("core state should be readable"));
        core.select_outbound("proxy", &selected_tag)
            .expect("selector should switch through the embedded Clash API");
        let mut client = TcpStream::connect(("127.0.0.1", mixed_port))
            .expect("mixed inbound should accept HTTP proxy requests");
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("client read timeout should be configured");
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{target_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .expect("request should be written to mixed inbound");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .expect("mixed inbound should return the local HTTP response");
        target.join().expect("local HTTP thread should finish");
        assert!(response.starts_with(b"HTTP/1.1 204"));

        thread::sleep(Duration::from_millis(100));
        let traffic = core.traffic().expect("traffic counters should be readable");
        assert!(traffic.upload_total > 0);
        assert!(traffic.download_total > 0);
        let logs = core.logs(0).expect("core logs should be readable");
        assert!(logs.next_cursor > 0);
        assert!(logs.entries.iter().any(|entry| {
            entry.message.contains("outbound/direct[direct]")
                && entry.message.contains(&format!("127.0.0.1:{target_port}"))
        }));
        core.stop().expect("embedded core should stop");

        let probe_options = SingBoxOptions {
            mixed_port: available_loopback_ports().0,
            log_level: "error".to_string(),
            ..SingBoxOptions::default()
        };
        core.start(&request, &probe_options)
            .expect("embedded core should restart without the Clash API");
        core.stop().expect("restarted embedded core should stop");
    }

    fn available_loopback_ports() -> (u16, u16) {
        let mixed = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("temporary mixed listener should bind");
        let traffic = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("temporary traffic listener should bind");
        let ports = (
            mixed
                .local_addr()
                .expect("temporary mixed listener should expose its port")
                .port(),
            traffic
                .local_addr()
                .expect("temporary traffic listener should expose its port")
                .port(),
        );
        drop((mixed, traffic));
        ports
    }
}
