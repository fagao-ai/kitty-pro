//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

pub(crate) const RULE_SET_UPDATE_CHECK_SECS: u64 = 6 * 60 * 60;

/// Copy handle for "the running core lags behind local config edits".
/// Provided as context by `ProxyApp` so every page can flag edits without
/// prop-drilling, and so `CoreStatusSignals` can clear it on apply.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ConfigPending(pub(crate) Signal<bool>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppView {
    Overview,
    Nodes,
    Subscriptions,
    Rules,
    Logs,
    Settings,
}

/// Which slice of the log buffer the page shows. Each variant is one chip in
/// the count strip, so there is deliberately no "everything" filter: routed
/// decisions and kernel chatter are separated by the last chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogFilter {
    Routes,
    Direct,
    Proxy,
    Block,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshTarget {
    One(u64),
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncAction {
    Save,
    Pull,
    Push,
    ForcePush,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TrafficDisplay {
    pub(crate) upload_bytes_per_second: u64,
    pub(crate) download_bytes_per_second: u64,
    pub(crate) upload_total: u64,
    pub(crate) download_total: u64,
    pub(crate) active_connections: u32,
    /// Per-second samples, oldest first, newest last. Download and upload are
    /// kept apart so the live chart can stack them instead of summing them.
    pub(crate) download_history: Vec<u64>,
    pub(crate) upload_history: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SystemProxyLoadState {
    Loading,
    Ready(SystemProxyStatus),
    Failed(String),
}

#[derive(Clone, PartialEq)]
pub(crate) struct RuleSelectOption {
    pub(crate) value: String,
    pub(crate) label: String,
}

impl RuleSelectOption {
    pub(crate) fn new(value: &str, label: &str) -> Self {
        Self {
            value: value.to_string(),
            label: label.to_string(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn wait_for_traffic_tick() {
    gloo_timers::future::TimeoutFuture::new(1_000).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn wait_for_traffic_tick() {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn wait_for_rule_set_update_check() {
    gloo_timers::future::TimeoutFuture::new((RULE_SET_UPDATE_CHECK_SECS * 1_000) as u32).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn wait_for_rule_set_update_check() {
    tokio::time::sleep(std::time::Duration::from_secs(RULE_SET_UPDATE_CHECK_SECS)).await;
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn wait_for_latency_poll() {
    gloo_timers::future::TimeoutFuture::new(100).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn wait_for_latency_poll() {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn wait_for_toast_sweep() {
    gloo_timers::future::TimeoutFuture::new(TOAST_SWEEP_MILLIS as u32).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn wait_for_toast_sweep() {
    tokio::time::sleep(std::time::Duration::from_millis(TOAST_SWEEP_MILLIS)).await;
}

impl AppView {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Nodes => "节点",
            Self::Subscriptions => "订阅",
            Self::Rules => "规则",
            Self::Logs => "日志",
            Self::Settings => "设置",
        }
    }

    /// Latin kicker above the page title, matching the design's
    /// `SECTION / 中文副标题` eyebrow.
    pub(crate) fn eyebrow(self) -> &'static str {
        match self {
            Self::Overview => "OVERVIEW / 实时状态",
            Self::Nodes => "NODES / 节点管理",
            Self::Subscriptions => "SUBSCRIPTIONS / 订阅来源",
            Self::Rules => "RULES / 分流与 DNS",
            Self::Logs => "LOGS / 连接日志",
            Self::Settings => "SETTINGS / 内核与路由",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DesktopAutostartCapability(pub(crate) bool);
