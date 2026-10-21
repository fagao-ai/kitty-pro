use api::{
    CoreLogEntry, CoreTraffic, NodeLatency, RouteDecision, RouteLogDetail, RouteTargetKind,
    SyncConfig, SyncProviderKind, SystemProxyStatus,
};
use dioxus::prelude::*;
use dioxus_free_icons::icons::ld_icons::{
    LdArrowDown, LdArrowUp, LdBan, LdCheck, LdChevronDown, LdChevronRight, LdCircleAlert,
    LdCircleCheck, LdClipboardCopy, LdCloud, LdCloudDownload, LdCloudUpload, LdDownload, LdGauge,
    LdGitFork, LdGlobe, LdInfo, LdLanguages, LdLayers, LdLayoutDashboard, LdListFilter, LdMoon,
    LdNetwork, LdPause, LdPencil, LdPlay, LdPlus, LdPower, LdRadioTower, LdRefreshCw, LdRoute,
    LdRss, LdSave, LdScrollText, LdSearch, LdShieldCheck, LdSlidersHorizontal, LdSplit, LdSun,
    LdTerminal, LdTimer, LdTrash2, LdWifi, LdX,
};
use dioxus_free_icons::Icon;
use proxy_core::{
    validate_custom_rules, AppProfile, ConnectionRequest, CustomRule, CustomRuleAction,
    CustomRuleMatch, DnsOverride, ParseReport, ProxyGroup, ProxyGroupKind, ProxyNode,
    ProxyProtocol, Subscription, SyncSnapshot, SyncSubscription, TunnelMode, MAX_CUSTOM_RULES,
    SYNC_SNAPSHOT_VERSION,
};
use std::collections::HashMap;

use crate::APP_CSS;

const BRAND_ICON_SVG: &str = include_str!("../assets/kitty-pro.svg");
const ANDROID_VPN_WAITING_NOTICE: &str = "正在等待 Android VPN 授权或启动服务";
const RULE_SET_UPDATE_CHECK_SECS: u64 = 6 * 60 * 60;
const TOAST_DISMISS_MILLIS: u64 = 4_500;
const TOAST_SWEEP_MILLIS: u64 = 250;
const TOAST_DISMISS_TICKS: u32 = (TOAST_DISMISS_MILLIS / TOAST_SWEEP_MILLIS) as u32;
const LATENCY_CACHE_KEY: &str = "kitty-pro.node-latency.v1";
/// Bars in the live traffic chart: one per second over the last minute.
const TRAFFIC_HISTORY_LEN: usize = 60;

type LatencyCacheEntry = (String, String, u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopTrayCommand {
    SelectSubscription(u64),
    SelectNode(String),
    ToggleSystemProxy,
    CopyEnvironmentVariables,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopTraySubscription {
    pub id: u64,
    pub name: String,
    pub nodes: Vec<DesktopTrayNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopTrayNode {
    pub tag: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesktopTrayState {
    pub ready: bool,
    pub busy: bool,
    pub connected: bool,
    pub system_proxy_ready: bool,
    pub system_proxy_supported: bool,
    pub system_proxy_enabled: bool,
    pub active_subscription_id: Option<u64>,
    pub selected_tag: String,
    pub subscriptions: Vec<DesktopTraySubscription>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct DesktopTrayBridge {
    pub state: Signal<DesktopTrayState>,
    pub command: Signal<Option<DesktopTrayCommand>>,
}

/// Signals describing the current tunnel/routing configuration. Grouped so
/// connection-request builders avoid a long positional argument list.
#[derive(Clone, Copy, PartialEq)]
struct RuntimeSettings {
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
}

fn build_connection_request(
    nodes: Vec<ProxyNode>,
    selected_tag: String,
    proxy_server_nameservers: Vec<String>,
    settings: RuntimeSettings,
    group_selections: HashMap<String, String>,
) -> ConnectionRequest {
    let RuntimeSettings {
        tunnel_mode,
        tun_enabled,
        allow_lan,
        custom_rules,
        dns_overrides,
        config_script_enabled,
        config_script,
    } = settings;
    let script_enabled = config_script_enabled();
    let script = config_script();
    let custom_rules = connection_custom_rules(script_enabled, &script, custom_rules());
    let config_script = if script_enabled && !script.trim().is_empty() {
        Some(script)
    } else {
        None
    };

    ConnectionRequest {
        nodes,
        selected_tag,
        proxy_server_nameservers,
        mode: tunnel_mode(),
        tun: tun_enabled(),
        allow_lan: allow_lan(),
        custom_rules,
        dns_overrides: dns_overrides(),
        config_script,
        group_selections,
    }
}

/// Signals mirroring the sing-box core status back into the UI after a
/// (re)start, grouped to keep `restart_core_from_ui`'s signature small.
#[derive(Clone, Copy, PartialEq)]
struct CoreStatusSignals {
    connected: Signal<bool>,
    core_restarting: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
}

async fn restart_core_from_ui(
    request: ConnectionRequest,
    status: CoreStatusSignals,
    toast: ToastManager,
    success_message: impl Into<String>,
) -> bool {
    let CoreStatusSignals {
        mut connected,
        mut core_restarting,
        mut core_state,
        mut core_version,
        mut core_note,
    } = status;
    let success_message = success_message.into();
    core_restarting.set(true);
    let success = match api::restart_core(request).await {
        Ok(status) => {
            let is_running = status.state == "running";
            connected.set(is_running);
            core_state.set(status.state);
            core_version.set(status.version);
            core_note.set(status.note);
            if is_running {
                toast.success(success_message);
            } else {
                toast.info(ANDROID_VPN_WAITING_NOTICE);
            }
            true
        }
        Err(error) => {
            toast.error(format!("内核重启失败: {error}"));
            match api::core_status().await {
                Ok(status) => {
                    connected.set(status.state == "running");
                    core_state.set(status.state);
                    core_version.set(status.version);
                    core_note.set(status.note);
                }
                Err(status_error) => {
                    connected.set(false);
                    core_state.set("unavailable".to_string());
                    core_note.set(Some(status_error.to_string()));
                }
            }
            false
        }
    };
    core_restarting.set(false);
    success
}

/// Busy/status signals shared by every runtime knob that restarts the core
/// (TUN, LAN listening, system proxy). Grouping them keeps the helpers below
/// small enough for both the settings page and the phone quick toggles.
#[derive(Clone, Copy, PartialEq)]
struct RuntimeSignals {
    status: CoreStatusSignals,
    core_busy: Signal<bool>,
    system_proxy_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
}

impl RuntimeSignals {
    fn busy(self) -> bool {
        (self.core_busy)()
            || (self.status.core_restarting)()
            || (self.system_proxy_busy)()
            || (self.runtime_action_busy)()
    }
}

/// Toggle TUN mode. With the core stopped the privileged helper is prepared or
/// released; while running, the core restarts with the new config.
/// Mirror a core status response back into the shared status signals.
fn apply_core_status(status: api::ApiCoreStatus, signals: CoreStatusSignals) -> bool {
    let CoreStatusSignals {
        mut connected,
        core_restarting: _,
        mut core_state,
        mut core_version,
        mut core_note,
    } = signals;
    let is_running = status.state == "running";
    connected.set(is_running);
    core_state.set(status.state);
    core_version.set(status.version);
    core_note.set(status.note);
    is_running
}

fn spawn_tun_toggle(
    enabled: bool,
    request: ConnectionRequest,
    mut tun_enabled: Signal<bool>,
    mut tun_busy: Signal<bool>,
    runtime: RuntimeSignals,
    toast: ToastManager,
) {
    if runtime.busy() {
        return;
    }
    let CoreStatusSignals {
        connected,
        mut core_restarting,
        core_state: _,
        core_version: _,
        mut core_note,
    } = runtime.status;
    let mut runtime_action_busy = runtime.runtime_action_busy;
    let previous_enabled = tun_enabled();
    tun_busy.set(true);
    runtime_action_busy.set(true);
    core_restarting.set(true);
    spawn(async move {
        if !connected() {
            let result = if enabled {
                api::prepare_tun_mode(request.clone()).await
            } else {
                api::release_tun_mode().await
            };
            match result {
                Ok(()) => {
                    tun_enabled.set(enabled);
                    toast.success(if enabled {
                        "TUN 权限已准备，启动内核时直接生效"
                    } else {
                        "TUN 模式已关闭"
                    });
                }
                Err(error) => {
                    tun_enabled.set(previous_enabled);
                    if !enabled {
                        // The local `connected` state can be stale (page reload,
                        // failed status poll), so the core may actually still be
                        // running and refuse a bare helper release. Refresh the
                        // real state and fall back to restarting without TUN.
                        match api::core_status().await {
                            Ok(status) if status.state == "running" => {
                                match api::restart_core(request).await {
                                    Ok(status) => {
                                        apply_core_status(status, runtime.status);
                                        tun_enabled.set(false);
                                        toast.success("TUN 模式已关闭");
                                    }
                                    Err(restart_error) => {
                                        toast.error(format!("关闭 TUN 失败: {restart_error}"));
                                    }
                                }
                            }
                            Ok(_) => {
                                toast.error(format!("关闭 TUN 失败: {error}"));
                            }
                            Err(status_error) => {
                                toast.error(format!(
                                    "关闭 TUN 失败: {error}；刷新内核状态失败: {status_error}"
                                ));
                            }
                        }
                    } else {
                        toast.error(format!("TUN 权限准备失败: {error}"));
                    }
                }
            }
            tun_busy.set(false);
            core_restarting.set(false);
            runtime_action_busy.set(false);
            return;
        }

        match api::restart_core(request).await {
            Ok(status) => {
                apply_core_status(status, runtime.status);
                tun_enabled.set(enabled);
                toast.success(if enabled {
                    "TUN 模式已启用"
                } else {
                    "TUN 模式已关闭"
                });
            }
            Err(error) => {
                tun_enabled.set(previous_enabled);
                match api::core_status().await {
                    Ok(status) => {
                        apply_core_status(status, runtime.status);
                    }
                    Err(status_error) => {
                        core_note.set(Some(status_error.to_string()));
                    }
                }
                toast.error(format!("应用 TUN 设置失败: {error}"));
            }
        }
        tun_busy.set(false);
        core_restarting.set(false);
        runtime_action_busy.set(false);
    });
}

/// Toggle LAN listening. While the core runs it restarts on the new bind
/// address; while stopped the preference applies to the next start.
fn spawn_allow_lan_toggle(
    enabled: bool,
    request: ConnectionRequest,
    mut allow_lan: Signal<bool>,
    runtime: RuntimeSignals,
    toast: ToastManager,
) {
    if runtime.busy() {
        return;
    }
    let CoreStatusSignals {
        connected,
        mut core_restarting,
        core_state: _,
        core_version: _,
        mut core_note,
    } = runtime.status;
    let mut runtime_action_busy = runtime.runtime_action_busy;
    runtime_action_busy.set(true);
    spawn(async move {
        if !connected() {
            allow_lan.set(enabled);
            toast.info(if enabled {
                "局域网连接将在下次启动内核时生效"
            } else {
                "仅本机访问将在下次启动内核时生效"
            });
            runtime_action_busy.set(false);
            return;
        }
        core_restarting.set(true);
        match api::restart_core(request).await {
            Ok(status) => {
                apply_core_status(status, runtime.status);
                allow_lan.set(enabled);
                toast.success(if enabled {
                    "已允许局域网连接，sing-box 内核已重启"
                } else {
                    "已恢复仅本机访问，sing-box 内核已重启"
                });
            }
            Err(error) => {
                match api::core_status().await {
                    Ok(status) => {
                        apply_core_status(status, runtime.status);
                    }
                    Err(status_error) => {
                        core_note.set(Some(status_error.to_string()));
                    }
                }
                toast.error(format!("应用局域网监听设置失败: {error}"));
            }
        }
        core_restarting.set(false);
        runtime_action_busy.set(false);
    });
}

/// Start or stop the core from the UI. The overview power button and the log
/// page's "启动代理开始采集" affordance share this so both keep the TUN
/// pre-flight, the status refresh on failure, and the system-proxy re-read.
#[derive(Clone, Copy, PartialEq)]
struct CoreToggleSignals {
    status: CoreStatusSignals,
    core_busy: Signal<bool>,
    system_proxy: Signal<SystemProxyLoadState>,
}

fn spawn_core_toggle(
    request: Option<ConnectionRequest>,
    signals: CoreToggleSignals,
    toast: ToastManager,
) {
    let CoreToggleSignals {
        status,
        mut core_busy,
        mut system_proxy,
    } = signals;
    let CoreStatusSignals {
        mut connected,
        core_restarting,
        mut core_state,
        mut core_note,
        ..
    } = status;
    if core_busy() || core_restarting() {
        return;
    }
    core_busy.set(true);
    let target = !connected();
    spawn(async move {
        // With TUN toggled on but the core stopped, the privileged helper is
        // gone and starting would fail with "TUN 权限尚未准备". Re-acquire the
        // permission (prompting for administrator access only when no
        // authorized helper exists) before starting, so the TUN switch stays a
        // persistent preference instead of a one-shot setup.
        // Only when TUN is actually enabled: the server rejects a pre-flight for
        // a non-TUN config, which would otherwise block a plain proxy start.
        if let Some(tun_request) = request.clone().filter(|req| target && req.tun) {
            if let Err(error) = api::prepare_tun_mode(tun_request).await {
                // The core may already be running behind a stale local state;
                // refresh before giving up so toggling again stays idempotent.
                match api::core_status().await {
                    Ok(status) if status.state == "running" => {}
                    Ok(_) => {
                        core_busy.set(false);
                        toast.error(format!("准备 TUN 权限失败，内核未启动: {error}"));
                        return;
                    }
                    Err(status_error) => {
                        core_busy.set(false);
                        toast.error(format!(
                            "准备 TUN 权限失败: {error}；刷新内核状态失败: {status_error}"
                        ));
                        return;
                    }
                }
            }
        }
        match api::set_core_enabled(target, request).await {
            Ok(core_status) => {
                let is_running = apply_core_status(core_status, status);
                if !target {
                    toast.success("连接已断开");
                } else if is_running {
                    toast.success("sing-box 已启动");
                } else {
                    toast.info(ANDROID_VPN_WAITING_NOTICE);
                }
            }
            Err(error) => {
                toast.error(error.to_string());
                match api::core_status().await {
                    Ok(core_status) => {
                        apply_core_status(core_status, status);
                    }
                    Err(status_error) => {
                        connected.set(false);
                        core_state.set("unavailable".to_string());
                        core_note.set(Some(status_error.to_string()));
                    }
                }
            }
        }
        if !target {
            match api::system_proxy_status().await {
                Ok(proxy_status) => {
                    system_proxy.set(SystemProxyLoadState::Ready(proxy_status));
                }
                Err(error) => {
                    system_proxy.set(SystemProxyLoadState::Failed(error.to_string()));
                }
            }
        }
        core_busy.set(false);
    });
}

/// Apply a system proxy change and always re-read the real status on failure.
fn spawn_system_proxy_toggle(
    enabled: bool,
    mut system_proxy: Signal<SystemProxyLoadState>,
    mut system_proxy_busy: Signal<bool>,
    core_action_busy: bool,
    toast: ToastManager,
) {
    if core_action_busy || system_proxy_busy() {
        return;
    }
    system_proxy_busy.set(true);
    spawn(async move {
        match api::set_system_proxy(enabled).await {
            Ok(status) => {
                system_proxy.set(SystemProxyLoadState::Ready(status));
                if enabled {
                    toast.success("系统代理已启用");
                } else {
                    toast.success("系统代理已关闭");
                }
            }
            Err(error) => {
                toast.error(format!("系统代理设置失败: {error}"));
                match api::system_proxy_status().await {
                    Ok(status) => {
                        system_proxy.set(SystemProxyLoadState::Ready(status));
                    }
                    Err(refresh_error) => {
                        system_proxy.set(SystemProxyLoadState::Failed(refresh_error.to_string()));
                    }
                }
            }
        }
        system_proxy_busy.set(false);
    });
}

fn load_latency_cache_script() -> String {
    r#"
const key = __CACHE_KEY__;
try {
    const raw = localStorage.getItem(key);
    const cached = raw ? JSON.parse(raw) : null;
    if (cached && Array.isArray(cached.results)) {
        dioxus.send(cached.results);
    } else {
        localStorage.removeItem(key);
        dioxus.send([]);
    }
} catch (_) {
    localStorage.removeItem(key);
    dioxus.send([]);
}
"#
    .replace("__CACHE_KEY__", &format!("{LATENCY_CACHE_KEY:?}"))
}

fn save_latency_cache_script() -> String {
    r#"
const key = __CACHE_KEY__;
const results = await dioxus.recv();
try {
    if (Array.isArray(results) && results.length > 0) {
        localStorage.setItem(key, JSON.stringify({ savedAt: Date.now(), results }));
    } else {
        localStorage.removeItem(key);
    }
} catch (_) {}
"#
    .replace("__CACHE_KEY__", &format!("{LATENCY_CACHE_KEY:?}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToastMessage {
    id: u64,
    kind: ToastKind,
    message: String,
    born_tick: u64,
}

#[derive(Clone, Copy)]
struct ToastManager {
    messages: Signal<Vec<ToastMessage>>,
    next_id: Signal<u64>,
    tick: Signal<u64>,
}

impl ToastManager {
    fn info(self, message: impl Into<String>) {
        self.push(ToastKind::Info, message.into());
    }

    fn success(self, message: impl Into<String>) {
        self.push(ToastKind::Success, message.into());
    }

    fn error(self, message: impl Into<String>) {
        self.push(ToastKind::Error, message.into());
    }

    fn push(self, kind: ToastKind, message: String) {
        let id = {
            let mut next_id = self.next_id;
            let mut value = next_id.write();
            *value = value.wrapping_add(1);
            *value
        };
        let mut messages = self.messages;
        {
            let mut stored = messages.write();
            if stored.len() >= 4 {
                stored.remove(0);
            }
            stored.push(ToastMessage {
                id,
                kind,
                message,
                born_tick: *self.tick.peek(),
            });
        }
    }

    /// 由根组件（ProxyApp）调用一次：在根 scope 中周期性清理过期 toast。
    /// 不能在 push 的调用方 scope 里 spawn 定时任务——组件卸载（切页、弹窗
    /// 关闭、列表行删除）会取消任务，导致 toast 永远残留。
    fn start_sweeper(self) {
        let mut messages = self.messages;
        let mut tick = self.tick;
        spawn(async move {
            loop {
                wait_for_toast_sweep().await;
                if messages.peek().is_empty() {
                    continue;
                }
                let now = {
                    let mut value = tick.write();
                    *value = value.wrapping_add(1);
                    *value
                };
                // 所有 toast 展示时长相同，按 push 顺序过期；只在真的有
                // toast 到期时才写 messages，避免每 tick 无谓重渲染视口。
                let has_expired = messages
                    .peek()
                    .iter()
                    .any(|toast| now.wrapping_sub(toast.born_tick) >= TOAST_DISMISS_TICKS as u64);
                if has_expired {
                    messages.write().retain(|toast| {
                        now.wrapping_sub(toast.born_tick) < TOAST_DISMISS_TICKS as u64
                    });
                }
            }
        });
    }

    fn dismiss(self, id: u64) {
        let mut messages = self.messages;
        messages.write().retain(|toast| toast.id != id);
    }

    #[cfg(target_os = "android")]
    fn contains(self, message: &str) -> bool {
        self.messages
            .peek()
            .iter()
            .any(|toast| toast.message == message)
    }

    #[cfg(target_os = "android")]
    fn dismiss_message(self, message: &str) {
        let mut messages = self.messages;
        messages.write().retain(|toast| toast.message != message);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
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
enum LogFilter {
    Routes,
    Direct,
    Proxy,
    Block,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshTarget {
    One(u64),
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncAction {
    Save,
    Pull,
    Push,
    ForcePush,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrafficDisplay {
    upload_bytes_per_second: u64,
    download_bytes_per_second: u64,
    upload_total: u64,
    download_total: u64,
    active_connections: u32,
    /// Per-second samples, oldest first, newest last. Download and upload are
    /// kept apart so the live chart can stack them instead of summing them.
    download_history: Vec<u64>,
    upload_history: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SystemProxyLoadState {
    Loading,
    Ready(SystemProxyStatus),
    Failed(String),
}

#[derive(Clone, PartialEq)]
struct RuleSelectOption {
    value: String,
    label: String,
}

impl RuleSelectOption {
    fn new(value: &str, label: &str) -> Self {
        Self {
            value: value.to_string(),
            label: label.to_string(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn wait_for_traffic_tick() {
    gloo_timers::future::TimeoutFuture::new(1_000).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn wait_for_traffic_tick() {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

#[cfg(target_arch = "wasm32")]
async fn wait_for_rule_set_update_check() {
    gloo_timers::future::TimeoutFuture::new((RULE_SET_UPDATE_CHECK_SECS * 1_000) as u32).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn wait_for_rule_set_update_check() {
    tokio::time::sleep(std::time::Duration::from_secs(RULE_SET_UPDATE_CHECK_SECS)).await;
}

#[cfg(target_arch = "wasm32")]
async fn wait_for_latency_poll() {
    gloo_timers::future::TimeoutFuture::new(100).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn wait_for_latency_poll() {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[cfg(target_arch = "wasm32")]
async fn wait_for_toast_sweep() {
    gloo_timers::future::TimeoutFuture::new(TOAST_SWEEP_MILLIS as u32).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn wait_for_toast_sweep() {
    tokio::time::sleep(std::time::Duration::from_millis(TOAST_SWEEP_MILLIS)).await;
}

impl AppView {
    fn title(self) -> &'static str {
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
    fn eyebrow(self) -> &'static str {
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
struct DesktopAutostartCapability(bool);

#[component]
pub fn ProxyApp(platform: String, desktop_tray: Option<DesktopTrayBridge>) -> Element {
    use_context_provider(|| DesktopAutostartCapability(desktop_tray.is_some()));
    let active_view = use_signal(|| AppView::Overview);
    let mut dark_mode = use_signal(|| false);
    let mut connected = use_signal(|| false);
    let core_busy = use_signal(|| false);
    let mut runtime_action_busy = use_signal(|| false);
    let core_restarting = use_signal(|| false);
    let mut core_state = use_signal(|| "checking".to_string());
    let mut core_version = use_signal(|| None::<String>);
    let mut core_note = use_signal(|| None::<String>);
    let toast = ToastManager {
        messages: use_signal(Vec::<ToastMessage>::new),
        next_id: use_signal(|| 0),
        tick: use_signal(|| 0u64),
    };
    use_context_provider(|| toast);
    use_effect(move || {
        toast.start_sweeper();
    });
    let mut nodes = use_signal(Vec::<ProxyNode>::new);
    let mut selected_tag = use_signal(String::new);
    let mut subscriptions = use_signal(Vec::<Subscription>::new);
    let mut custom_rules = use_signal(Vec::<CustomRule>::new);
    let mut dns_overrides = use_signal(Vec::<DnsOverride>::new);
    let rule_sets_busy = use_signal(|| false);
    let mut active_subscription_id = use_signal(|| None::<u64>);
    let mut tunnel_mode = use_signal(|| TunnelMode::Rule);
    let mut tun_enabled = use_signal(|| false);
    let mut allow_lan = use_signal(|| false);
    let mut config_script_enabled = use_signal(|| false);
    let mut config_script = use_signal(String::new);
    let mut group_selections = use_signal(HashMap::<String, String>::new);
    let mut import_open = use_signal(|| false);
    let mut import_name = use_signal(String::new);
    let mut import_source = use_signal(String::new);
    let mut import_busy = use_signal(|| false);
    let mut import_error = use_signal(|| None::<String>);
    let search = use_signal(String::new);
    let refresh_busy = use_signal(|| None::<RefreshTarget>);
    let mut latency_results = use_signal(HashMap::<String, NodeLatency>::new);
    let latency_busy = use_signal(|| false);
    let mut traffic = use_signal(TrafficDisplay::default);
    let mut traffic_poll_generation = use_signal(|| 0_u64);
    let mut core_logs = use_signal(Vec::<CoreLogEntry>::new);
    // Seconds since the core reached `running`; None while it is stopped. A plain
    // tick counter keeps this off platform clock APIs (wasm has none).
    let mut uptime_seconds = use_signal(|| None::<u64>);
    let mut uptime_generation = use_signal(|| 0_u32);
    let mut core_log_cursor = use_signal(|| 0_u64);
    let mut log_poll_generation = use_signal(|| 0_u64);
    let log_collection_paused = use_signal(|| false);
    let mut profile_loaded = use_signal(|| false);
    let mut system_proxy = use_signal(|| SystemProxyLoadState::Loading);
    let mut system_proxy_busy = use_signal(|| false);
    // Log page state lives here so the export action can hand the exact rows on
    // screen to the CSV writer, and the rules tab drives the topbar eyebrow.
    let log_filter = use_signal(|| LogFilter::Routes);
    let log_search = use_signal(String::new);
    let mut log_export_busy = use_signal(|| false);
    let rules_tab = use_signal(|| RulesTab::Routing);

    use_effect(move || {
        let Some(mut bridge) = desktop_tray else {
            return;
        };
        let (system_proxy_ready, system_proxy_supported, system_proxy_enabled) =
            match system_proxy() {
                SystemProxyLoadState::Ready(status) => (true, status.supported, status.enabled),
                SystemProxyLoadState::Loading | SystemProxyLoadState::Failed(_) => {
                    (false, false, false)
                }
            };
        bridge.state.set(DesktopTrayState {
            ready: profile_loaded(),
            busy: core_busy() || core_restarting() || system_proxy_busy() || runtime_action_busy(),
            connected: connected(),
            system_proxy_ready,
            system_proxy_supported,
            system_proxy_enabled,
            active_subscription_id: active_subscription_id(),
            selected_tag: selected_tag(),
            subscriptions: subscriptions()
                .into_iter()
                .map(|subscription| DesktopTraySubscription {
                    id: subscription.id,
                    name: subscription.name,
                    nodes: subscription
                        .nodes
                        .into_iter()
                        .map(|node| DesktopTrayNode {
                            tag: node.tag,
                            name: node.name,
                        })
                        .collect(),
                })
                .collect(),
        });
    });

    use_effect(move || {
        let Some(mut bridge) = desktop_tray else {
            return;
        };
        let Some(command) = (bridge.command)() else {
            return;
        };
        bridge.command.set(None);

        if !profile_loaded()
            || core_busy()
            || core_restarting()
            || system_proxy_busy()
            || runtime_action_busy()
        {
            return;
        }

        runtime_action_busy.set(true);
        spawn(async move {
            match command {
                DesktopTrayCommand::SelectSubscription(subscription_id) => {
                    if active_subscription_id() == Some(subscription_id) {
                        runtime_action_busy.set(false);
                        return;
                    }
                    if !subscriptions()
                        .iter()
                        .any(|subscription| subscription.id == subscription_id)
                    {
                        toast.error("托盘选择的订阅已不存在");
                        runtime_action_busy.set(false);
                        return;
                    }

                    let previous_subscription_id = active_subscription_id();
                    let previous_nodes = nodes();
                    let previous_tag = selected_tag();
                    let previous_selections = group_selections();
                    let active_nodes =
                        collect_subscription_nodes(&subscriptions(), Some(subscription_id));
                    let next_tag = select_available_tag(&active_nodes, "");
                    let mut next_selections = previous_selections.clone();
                    if next_tag.is_empty() {
                        next_selections.remove("proxy");
                    } else {
                        next_selections.insert("proxy".to_string(), next_tag.clone());
                    }

                    active_subscription_id.set(Some(subscription_id));
                    nodes.set(active_nodes.clone());
                    selected_tag.set(next_tag.clone());
                    group_selections.set(next_selections.clone());

                    if connected() {
                        let request = build_connection_request(
                            active_nodes,
                            next_tag,
                            active_subscription_proxy_server_nameservers(
                                &subscriptions(),
                                Some(subscription_id),
                            ),
                            RuntimeSettings {
                                tunnel_mode,
                                tun_enabled,
                                allow_lan,
                                custom_rules,
                                dns_overrides,
                                config_script_enabled,
                                config_script,
                            },
                            next_selections,
                        );
                        if !restart_core_from_ui(
                            request,
                            CoreStatusSignals {
                                connected,
                                core_restarting,
                                core_state,
                                core_version,
                                core_note,
                            },
                            toast,
                            "已从托盘切换订阅，内核已重启",
                        )
                        .await
                        {
                            active_subscription_id.set(previous_subscription_id);
                            nodes.set(previous_nodes);
                            selected_tag.set(previous_tag);
                            group_selections.set(previous_selections);
                        }
                    } else {
                        toast.success("已从托盘切换订阅");
                    }
                }
                DesktopTrayCommand::SelectNode(outbound) => {
                    if selected_tag() == outbound {
                        runtime_action_busy.set(false);
                        return;
                    }
                    let active_nodes = nodes();
                    if !active_nodes.iter().any(|node| node.tag == outbound) {
                        toast.error("托盘选择的节点已不存在");
                        runtime_action_busy.set(false);
                        return;
                    }

                    let previous_tag = selected_tag();
                    let previous_selections = group_selections();
                    let mut next_selections = previous_selections.clone();
                    next_selections.insert("proxy".to_string(), outbound.clone());
                    selected_tag.set(outbound.clone());

                    if connected() {
                        // TUN mode forwards this selector call to its existing
                        // privileged helper, so a normal node switch does not
                        // prompt for elevation or interrupt the tunnel.
                        if api::select_proxy_group("proxy".to_string(), outbound.clone())
                            .await
                            .is_ok()
                        {
                            group_selections.set(next_selections);
                            toast.success(format!("已从托盘切换到 {outbound}"));
                        } else {
                            // restart_core preflights the candidate and reuses
                            // a live TUN helper before stopping the old core.
                            group_selections.set(next_selections.clone());
                            let request = build_connection_request(
                                active_nodes,
                                outbound.clone(),
                                active_subscription_proxy_server_nameservers(
                                    &subscriptions(),
                                    active_subscription_id(),
                                ),
                                RuntimeSettings {
                                    tunnel_mode,
                                    tun_enabled,
                                    allow_lan,
                                    custom_rules,
                                    dns_overrides,
                                    config_script_enabled,
                                    config_script,
                                },
                                next_selections,
                            );
                            if !restart_core_from_ui(
                                request,
                                CoreStatusSignals {
                                    connected,
                                    core_restarting,
                                    core_state,
                                    core_version,
                                    core_note,
                                },
                                toast,
                                format!("已从托盘切换到 {outbound}，内核已重启"),
                            )
                            .await
                            {
                                selected_tag.set(previous_tag);
                                group_selections.set(previous_selections);
                            }
                        }
                    } else {
                        group_selections.set(next_selections);
                        toast.success(format!("已从托盘选择 {outbound}"));
                    }
                }
                DesktopTrayCommand::ToggleSystemProxy => match system_proxy() {
                    SystemProxyLoadState::Ready(status) if status.supported => {
                        let enabled = !status.enabled;
                        system_proxy_busy.set(true);
                        match api::set_system_proxy(enabled).await {
                            Ok(status) => {
                                system_proxy.set(SystemProxyLoadState::Ready(status));
                                if enabled {
                                    toast.success("系统代理已启用");
                                } else {
                                    toast.success("系统代理已关闭");
                                }
                            }
                            Err(error) => {
                                toast.error(format!("系统代理设置失败: {error}"));
                                match api::system_proxy_status().await {
                                    Ok(status) => {
                                        system_proxy.set(SystemProxyLoadState::Ready(status));
                                    }
                                    Err(refresh_error) => {
                                        system_proxy.set(SystemProxyLoadState::Failed(
                                            refresh_error.to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                        system_proxy_busy.set(false);
                    }
                    SystemProxyLoadState::Ready(_) => {
                        toast.error("当前平台不支持系统代理");
                    }
                    SystemProxyLoadState::Loading => {
                        toast.info("系统代理状态仍在读取");
                    }
                    SystemProxyLoadState::Failed(error) => {
                        toast.error(format!("系统代理状态不可用: {error}"));
                    }
                },
                DesktopTrayCommand::CopyEnvironmentVariables => {
                    match api::copy_proxy_environment_variables().await {
                        Ok(()) => toast.success("代理环境变量已复制"),
                        Err(error) => toast.error(format!("复制环境变量失败: {error}")),
                    }
                }
            }
            runtime_action_busy.set(false);
        });
    });

    #[cfg(not(target_os = "android"))]
    use_effect(move || {
        spawn(async move {
            match api::core_status().await {
                Ok(status) => {
                    connected.set(status.state == "running");
                    core_state.set(status.state);
                    core_version.set(status.version);
                    core_note.set(status.note);
                }
                Err(error) => {
                    core_state.set("unavailable".to_string());
                    core_note.set(Some(error.to_string()));
                }
            }
        });
    });

    #[cfg(target_os = "android")]
    use_effect(move || {
        spawn(async move {
            loop {
                match api::core_status().await {
                    Ok(status) => {
                        let is_running = status.state == "running";
                        connected.set(is_running);
                        if is_running && toast.contains(ANDROID_VPN_WAITING_NOTICE) {
                            toast.dismiss_message(ANDROID_VPN_WAITING_NOTICE);
                            toast.success("Android VPN 已连接");
                        }
                        core_state.set(status.state);
                        core_version.set(status.version);
                        core_note.set(status.note);
                    }
                    Err(error) => {
                        core_state.set("unavailable".to_string());
                        core_note.set(Some(error.to_string()));
                    }
                }
                wait_for_traffic_tick().await;
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            match api::system_proxy_status().await {
                Ok(status) => system_proxy.set(SystemProxyLoadState::Ready(status)),
                Err(error) => system_proxy.set(SystemProxyLoadState::Failed(error.to_string())),
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            loop {
                let _ = api::update_rule_sets(false).await;
                wait_for_rule_set_update_check().await;
            }
        });
    });

    use_effect(move || {
        spawn(async move {
            match api::load_profile().await {
                Ok(profile) => {
                    let restored_active_id = resolve_active_subscription_id(
                        &profile.subscriptions,
                        profile.active_subscription_id,
                        &profile.selected_tag,
                    );
                    let restored_nodes =
                        collect_subscription_nodes(&profile.subscriptions, restored_active_id);
                    let restored_tag = select_available_tag(&restored_nodes, &profile.selected_tag);
                    subscriptions.set(profile.subscriptions);
                    custom_rules.set(profile.custom_rules);
                    dns_overrides.set(profile.dns_overrides);
                    active_subscription_id.set(restored_active_id);
                    nodes.set(restored_nodes);
                    selected_tag.set(restored_tag);
                    tunnel_mode.set(profile.tunnel_mode);
                    tun_enabled.set(profile.tun_enabled);
                    allow_lan.set(profile.allow_lan);
                    dark_mode.set(profile.dark_mode);
                    config_script_enabled.set(profile.config_script_enabled);
                    config_script.set(profile.config_script);
                    group_selections.set(profile.group_selections);
                    profile_loaded.set(true);
                }
                Err(error) => toast.error(format!("无法恢复本地配置: {error}")),
            }
        });
    });

    use_effect(move || {
        if !profile_loaded() {
            return;
        }
        let current_nodes = nodes();
        if current_nodes.is_empty() {
            latency_results.set(HashMap::new());
            return;
        }
        spawn(async move {
            let mut eval = document::eval(&load_latency_cache_script());
            let Ok(entries) = eval.recv::<Vec<LatencyCacheEntry>>().await else {
                return;
            };
            latency_results.set(restore_cached_latencies(&current_nodes, entries));
        });
    });

    use_effect(move || {
        if !profile_loaded() {
            return;
        }
        let profile = AppProfile {
            subscriptions: subscriptions(),
            active_subscription_id: active_subscription_id(),
            selected_tag: selected_tag(),
            tunnel_mode: tunnel_mode(),
            tun_enabled: tun_enabled(),
            allow_lan: allow_lan(),
            dark_mode: dark_mode(),
            custom_rules: custom_rules(),
            dns_overrides: dns_overrides(),
            config_script_enabled: config_script_enabled(),
            config_script: config_script(),
            group_selections: group_selections(),
            ..AppProfile::default()
        };
        spawn(async move {
            if let Err(error) = api::save_profile(profile).await {
                toast.error(format!("本地配置保存失败: {error}"));
            }
        });
    });

    use_effect(move || {
        let running = core_state() == "running";
        let overview_visible = active_view() == AppView::Overview;
        let generation = {
            let mut current = traffic_poll_generation.write();
            *current = current.wrapping_add(1);
            *current
        };
        if !running {
            traffic.set(TrafficDisplay::default());
            return;
        }
        if !overview_visible {
            return;
        }
        spawn(async move {
            let mut previous = None::<CoreTraffic>;
            let mut download_history = traffic.peek().download_history.clone();
            let mut upload_history = traffic.peek().upload_history.clone();
            while *traffic_poll_generation.peek() == generation
                && core_state.peek().as_str() == "running"
                && *active_view.peek() == AppView::Overview
            {
                if let Ok(current) = api::core_traffic().await {
                    if *traffic_poll_generation.peek() != generation
                        || *active_view.peek() != AppView::Overview
                    {
                        break;
                    }
                    let (upload_bytes_per_second, download_bytes_per_second) = previous
                        .map(|last| {
                            (
                                current.upload_total.saturating_sub(last.upload_total),
                                current.download_total.saturating_sub(last.download_total),
                            )
                        })
                        .unwrap_or((0, 0));
                    download_history.push(download_bytes_per_second);
                    upload_history.push(upload_bytes_per_second);
                    while download_history.len() > TRAFFIC_HISTORY_LEN {
                        download_history.remove(0);
                        upload_history.remove(0);
                    }
                    traffic.set(TrafficDisplay {
                        upload_bytes_per_second,
                        download_bytes_per_second,
                        upload_total: current.upload_total,
                        download_total: current.download_total,
                        active_connections: current.active_connections,
                        download_history: download_history.clone(),
                        upload_history: upload_history.clone(),
                    });
                    previous = Some(current);
                }
                wait_for_traffic_tick().await;
            }
        });
    });

    use_effect(move || {
        let is_connected = connected();
        let generation = {
            let mut current = uptime_generation.write();
            *current = current.wrapping_add(1);
            *current
        };
        if !is_connected {
            uptime_seconds.set(None);
            return;
        }
        uptime_seconds.set(Some(0));
        let mut uptime_seconds = uptime_seconds;
        spawn(async move {
            let mut seconds = 0_u64;
            while *uptime_generation.peek() == generation && *connected.peek() {
                wait_for_traffic_tick().await;
                seconds = seconds.saturating_add(1);
                uptime_seconds.set(Some(seconds));
            }
        });
    });

    use_effect(move || {
        let enabled =
            core_state() == "running" && active_view() == AppView::Logs && !log_collection_paused();
        spawn(async move {
            let _ = api::set_core_log_collection(enabled).await;
        });
    });

    use_effect(move || {
        let enabled =
            core_state() == "running" && active_view() == AppView::Logs && !log_collection_paused();
        let generation = {
            let mut current = log_poll_generation.write();
            *current = current.wrapping_add(1);
            *current
        };
        if !enabled {
            return;
        }
        spawn(async move {
            let mut cursor = *core_log_cursor.peek();
            while *log_poll_generation.peek() == generation
                && core_state.peek().as_str() == "running"
                && *active_view.peek() == AppView::Logs
                && !*log_collection_paused.peek()
            {
                if let Ok(batch) = api::core_logs(cursor).await {
                    cursor = batch.next_cursor;
                    core_log_cursor.set(cursor);
                    if !batch.entries.is_empty() {
                        let mut stored = core_logs.write();
                        stored.extend(batch.entries);
                        if stored.len() > 500 {
                            let excess = stored.len() - 500;
                            stored.drain(..excess);
                        }
                    }
                }
                wait_for_traffic_tick().await;
            }
        });
    });

    let proxy_groups_resource = use_resource(move || {
        let request = ConnectionRequest {
            nodes: nodes(),
            selected_tag: selected_tag(),
            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                &subscriptions(),
                active_subscription_id(),
            ),
            mode: tunnel_mode(),
            tun: tun_enabled(),
            allow_lan: allow_lan(),
            custom_rules: connection_custom_rules(
                config_script_enabled(),
                &config_script(),
                custom_rules(),
            ),
            dns_overrides: dns_overrides(),
            config_script: if config_script_enabled() && !config_script().trim().is_empty() {
                Some(config_script())
            } else {
                None
            },
            group_selections: group_selections(),
        };
        async move {
            if request.nodes.is_empty() {
                Ok(Vec::new())
            } else {
                api::preview_proxy_groups(request)
                    .await
                    .map_err(|error| error.to_string())
            }
        }
    });

    let current_view = active_view();
    let eyebrow = match current_view {
        AppView::Rules if rules_tab() == RulesTab::Dns => "RULES / 专用 DNS",
        other => other.eyebrow(),
    };
    let (proxy_groups, proxy_groups_error, proxy_groups_loading) = match proxy_groups_resource() {
        Some(Ok(groups)) => (groups, None, false),
        Some(Err(error)) => (Vec::new(), Some(error), false),
        None => (Vec::new(), None, true),
    };
    let root_class = if dark_mode() {
        "proxy-app theme-dark"
    } else {
        "proxy-app"
    };
    let connection_allowed = !nodes().is_empty() && core_state() != "unavailable";

    rsx! {
        document::Style { "{APP_CSS}" }
        div { class: root_class,
            div { class: "ambient-grid" }
            aside { class: "sidebar glass-surface",
                Brand { kicker: brand_kicker(core_version()) }
                div { class: "nav-group-label", "MAIN" }
                nav { class: "primary-nav", aria_label: "主导航",
                    NavItem { view: AppView::Overview, active_view }
                    NavItem {
                        view: AppView::Nodes,
                        active_view,
                        count: nodes().len().to_string(),
                    }
                    NavItem {
                        view: AppView::Subscriptions,
                        active_view,
                        count: subscriptions().len().to_string(),
                    }
                    NavItem {
                        view: AppView::Rules,
                        active_view,
                        count: (custom_rules().len() + dns_overrides().len()).to_string(),
                    }
                    NavItem {
                        view: AppView::Logs,
                        active_view,
                        count: core_logs().len().to_string(),
                    }
                    NavItem { view: AppView::Settings, active_view }
                }
                div { class: "sidebar-footer",
                    div { class: "core-chip",
                        span { class: status_dot_class(&core_state()) }
                        div {
                            strong { {core_state_label(&core_state())} }
                            small { {core_version().unwrap_or_else(|| platform.clone())} }
                        }
                    }
                }
            }

            main { class: "main-content",
                header { class: "topbar",
                    div { class: "topbar-titles",
                        p { class: "eyebrow", "{eyebrow}" }
                        h1 { "{current_view.title()}" }
                    }
                    div { class: "topbar-actions",
                        div { class: connection_chip_class(connected(), &core_state()),
                            span { class: "status-badge-dot" }
                            {connection_chip_label(connected(), &core_state())}
                        }
                        RestartCoreButton {
                            status: CoreStatusSignals {
                                connected,
                                core_restarting,
                                core_state,
                                core_version,
                                core_note,
                            },
                            settings: RuntimeSettings {
                                tunnel_mode,
                                tun_enabled,
                                allow_lan,
                                custom_rules,
                                dns_overrides,
                                config_script_enabled,
                                config_script,
                            },
                            nodes,
                            selected_tag,
                            subscriptions,
                            active_subscription_id,
                            group_selections,
                            core_busy,
                            runtime_action_busy,
                            system_proxy_busy,
                        }
                        button {
                            class: "icon-button glass-control",
                            title: if dark_mode() { "切换浅色主题" } else { "切换深色主题" },
                            onclick: move |_| dark_mode.toggle(),
                            if dark_mode() {
                                Icon { icon: LdSun, width: 19, height: 19 }
                            } else {
                                Icon { icon: LdMoon, width: 19, height: 19 }
                            }
                        }
                        if matches!(current_view, AppView::Overview | AppView::Nodes | AppView::Subscriptions) {
                            button {
                                class: "primary-button compact",
                                onclick: move |_| {
                                    import_error.set(None);
                                    import_open.set(true);
                                },
                                Icon { icon: LdPlus, width: 18, height: 18 }
                                span { "添加订阅" }
                            }
                        }
                        // The design keeps the destructive/export pair in the page
                        // header so they are reachable without scrolling the log list.
                        if current_view == AppView::Logs {
                            button {
                                class: "secondary-button log-clear-button",
                                title: "清空当前缓冲区中的日志",
                                disabled: core_logs().is_empty() || log_export_busy(),
                                onclick: move |_| core_logs.write().clear(),
                                Icon { icon: LdTrash2, width: 15, height: 15 }
                                span { "清空日志" }
                            }
                            button {
                                class: "primary-button compact log-export-button",
                                title: "导出当前筛选结果为 CSV",
                                aria_busy: log_export_busy(),
                                disabled: core_logs().is_empty() || log_export_busy(),
                                onclick: move |_| {
                                    let rows = log_export_rows(&visible_log_entries(
                                        &core_logs(),
                                        log_filter(),
                                        log_search().trim().to_lowercase().as_str(),
                                    ));
                                    log_export_busy.set(true);
                                    spawn(async move {
                                        match download_csv("kitty-pro-logs.csv", rows).await {
                                            Ok(()) => toast.success("日志已导出为 CSV"),
                                            Err(error) => {
                                                toast.error(format!("日志导出失败: {error}"));
                                            }
                                        }
                                        log_export_busy.set(false);
                                    });
                                },
                                if log_export_busy() {
                                    span { class: "spinner" }
                                } else {
                                    Icon { icon: LdDownload, width: 15, height: 15 }
                                }
                                span { "导出 CSV" }
                            }
                        }
                    }
                }

                match current_view {
                    AppView::Overview => rsx! {
                        OverviewView {
                            nodes,
                            selected_tag,
                            active_view,
                            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                                &subscriptions(),
                                active_subscription_id(),
                            ),
                            connected,
                            core_busy,
                            core_restarting,
                            core_state,
                            core_note,
                            core_version,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            custom_rules,
                            dns_overrides,
                            connection_allowed,
                            latency_results,
                            traffic,
                            uptime_seconds,
                            subscription_count: subscriptions().len(),
                            config_script_enabled,
                            config_script,
                            group_selections,
                            system_proxy,
                            system_proxy_busy,
                            runtime_action_busy,
                            dark_mode,
                        }
                    },
                    AppView::Nodes => rsx! {
                        NodesView {
                            groups: proxy_groups,
                            groups_loading: proxy_groups_loading,
                            groups_error: proxy_groups_error,
                            nodes: nodes(),
                            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                                &subscriptions(),
                                active_subscription_id(),
                            ),
                            all_count: nodes().len(),
                            subscriptions: subscriptions(),
                            group_selections,
                            connected,
                            core_busy,
                            core_restarting,
                            system_proxy_busy,
                            runtime_action_busy,
                            core_state,
                            core_version,
                            core_note,
                            selected_tag,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            custom_rules,
        dns_overrides,
                            config_script_enabled,
                            config_script,
                            search,
                            latency_results,
                            latency_busy,
                            import_open,
                        }
                    },
                    AppView::Subscriptions => rsx! {
                        SubscriptionsView {
                            subscriptions,
                            active_subscription_id,
                            import_open,
                            nodes,
                            selected_tag,
                            refresh_busy,
                            connected,
                            core_busy,
                            core_restarting,
                            system_proxy_busy,
                            core_state,
                            core_version,
                            core_note,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            custom_rules,
        dns_overrides,
                            config_script_enabled,
                            config_script,
                            group_selections,
                            runtime_action_busy,
                        }
                    },
                    AppView::Rules => rsx! {
                        RulesView {
                            rules: custom_rules,
                            dns_overrides,
                            active_tab: rules_tab,
                            connected,
                            tunnel_mode,
                            rule_sets_busy,
                            config_script_enabled,
                        }
                    },
                    AppView::Logs => rsx! {
                        LogsView {
                            logs: core_logs,
                            connected,
                            collection_paused: log_collection_paused,
                            core_version,
                            filter: log_filter,
                            search: log_search,
                            on_start: move |_| {
                                let target = !connected();
                                if !target || nodes().is_empty() {
                                    return;
                                }
                                spawn_core_toggle(
                                    build_connection_request(
                                        nodes(),
                                        selected_tag(),
                                        active_subscription_proxy_server_nameservers(
                                            &subscriptions(),
                                            active_subscription_id(),
                                        ),
                                        RuntimeSettings {
                                            tunnel_mode,
                                            tun_enabled,
                                            allow_lan,
                                            custom_rules,
                                            dns_overrides,
                                            config_script_enabled,
                                            config_script,
                                        },
                                        group_selections(),
                                    )
                                    .into(),
                                    CoreToggleSignals {
                                        status: CoreStatusSignals {
                                            connected,
                                            core_restarting,
                                            core_state,
                                            core_version,
                                            core_note,
                                        },
                                        core_busy,
                                        system_proxy,
                                    },
                                    toast,
                                );
                            },
                        }
                    },
                    AppView::Settings => rsx! {
                        SettingsView {
                            platform: platform.clone(),
                            core_state,
                            core_version,
                            core_note,
                            connected,
                            core_busy,
                            core_restarting,
                            system_proxy_busy,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            dark_mode,
                            nodes,
                            selected_tag,
                            subscriptions,
                            active_subscription_id,
                            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                                &subscriptions(),
                                active_subscription_id(),
                            ),
                            custom_rules,
        dns_overrides,
                            config_script_enabled,
                            config_script,
                            group_selections,
                            runtime_action_busy,
                        }
                    },
                }
            }

            nav { class: "mobile-nav glass-surface", aria_label: "移动端导航",
                // The phone tab bar stays count-free: six slots leave no room for chips.
                NavItem { view: AppView::Overview, active_view, count: None::<String> }
                NavItem { view: AppView::Nodes, active_view, count: None::<String> }
                NavItem { view: AppView::Subscriptions, active_view, count: None::<String> }
                NavItem { view: AppView::Rules, active_view, count: None::<String> }
                NavItem { view: AppView::Logs, active_view, count: None::<String> }
                NavItem { view: AppView::Settings, active_view, count: None::<String> }
            }

            if import_open() {
                div {
                    class: "modal-backdrop",
                    role: "presentation",
                    onclick: move |_| {
                        if !import_busy() {
                            import_open.set(false);
                        }
                    },
                    div {
                        class: "modal glass-modal",
                        role: "dialog",
                        aria_modal: "true",
                        aria_label: "添加订阅",
                        onclick: move |event| event.stop_propagation(),
                        div { class: "modal-header",
                            div {
                                p { class: "eyebrow", "SUBSCRIPTION" }
                                h2 { "添加订阅" }
                            }
                            button {
                                class: "icon-button",
                                title: "关闭",
                                disabled: import_busy(),
                                onclick: move |_| import_open.set(false),
                                Icon { icon: LdX, width: 19, height: 19 }
                            }
                        }
                        div { class: "form-stack",
                            label {
                                span { "名称" }
                                input {
                                    value: import_name,
                                    placeholder: "例如：日常线路",
                                    oninput: move |event| import_name.set(event.value()),
                                }
                            }
                            label {
                                span { "订阅地址或内容" }
                                textarea {
                                    value: import_source,
                                    placeholder: "订阅地址，或代理 URL（HTTP/HTTPS/SOCKS5）",
                                    rows: 5,
                                    oninput: move |event| import_source.set(event.value()),
                                }
                            }
                            if let Some(error) = import_error() {
                                div { class: "form-error",
                                    Icon { icon: LdCircleAlert, width: 16, height: 16 }
                                    span { "{error}" }
                                }
                            }
                        }
                        div { class: "modal-actions",
                            button {
                                class: "secondary-button",
                                disabled: import_busy(),
                                onclick: move |_| import_open.set(false),
                                "取消"
                            }
                            button {
                                class: "primary-button",
                                disabled: import_busy()
                                    || core_busy()
                                    || core_restarting()
                                    || import_source().trim().is_empty(),
                                onclick: move |_| async move {
                                    import_busy.set(true);
                                    import_error.set(None);
                                    let source = import_source();
                                    match api::preview_subscription(source.clone()).await {
                                        Ok(report) if !report.nodes.is_empty() => {
                                            let ParseReport {
                                                nodes: report_nodes,
                                                rejected,
                                                proxy_server_nameservers,
                                            } = report;
                                            let count = report_nodes.len();
                                            let rejected = rejected.len();
                                            let id = next_subscription_id(&subscriptions());
                                            let parsed_nodes = namespace_nodes(id, report_nodes);
                                            let name = if import_name().trim().is_empty() {
                                                source_label(&source)
                                            } else {
                                                import_name().trim().to_string()
                                            };
                                            subscriptions.write().push(Subscription {
                                                id,
                                                name: name.clone(),
                                                source,
                                                nodes: parsed_nodes,
                                                proxy_server_nameservers,
                                                rejected_count: rejected,
                                            });
                                            if connected() {
                                                toast.success(format!("已导入 {name}，共 {count} 个节点"));
                                            } else {
                                                active_subscription_id.set(Some(id));
                                                let active_nodes = collect_subscription_nodes(
                                                    &subscriptions(),
                                                    Some(id),
                                                );
                                                let next_tag = select_available_tag(&active_nodes, "");
                                                let mut next_selections = group_selections();
                                                if next_tag.is_empty() {
                                                    next_selections.remove("proxy");
                                                } else {
                                                    next_selections
                                                        .insert("proxy".to_string(), next_tag.clone());
                                                }
                                                nodes.set(active_nodes);
                                                selected_tag.set(next_tag);
                                                group_selections.set(next_selections);
                                                toast.success(format!("已导入并切换到 {name}，共 {count} 个节点"));
                                            }
                                            import_source.set(String::new());
                                            import_name.set(String::new());
                                            import_open.set(false);
                                        }
                                        Ok(report) => {
                                            let reason = report
                                                .rejected
                                                .first()
                                                .map(|issue| issue.reason.clone())
                                                .unwrap_or_else(|| "没有找到可用节点".to_string());
                                            import_error.set(Some(reason));
                                        }
                                        Err(error) => import_error.set(Some(error.to_string())),
                                    }
                                    import_busy.set(false);
                                },
                                if import_busy() {
                                    span { class: "spinner" }
                                    "解析中"
                                } else {
                                    Icon { icon: LdPlus, width: 17, height: 17 }
                                    "导入"
                                }
                            }
                        }
                    }
                }
            }

            ToastViewport {}
        }
    }
}

/// Restart the embedded core. The design promotes this action from a bare
/// sidebar icon to a labelled topbar button, so phone and desktop share it.
#[component]
fn RestartCoreButton(
    status: CoreStatusSignals,
    settings: RuntimeSettings,
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    subscriptions: Signal<Vec<Subscription>>,
    active_subscription_id: Signal<Option<u64>>,
    group_selections: Signal<HashMap<String, String>>,
    core_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
    system_proxy_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let restarting = status.core_restarting;
    let is_restarting = restarting();
    let is_connected = (status.connected)();
    let busy = core_busy() || runtime_action_busy() || system_proxy_busy();
    let disabled = !is_connected || busy || is_restarting;
    rsx! {
        button {
            class: "secondary-button restart-core-button",
            title: "重启内核",
            aria_label: "重启内核",
            aria_busy: is_restarting,
            disabled,
            onclick: move |_| {
                let status = status;
                let settings = settings;
                let toast = toast;
                let request = build_connection_request(
                    nodes(),
                    selected_tag(),
                    active_subscription_proxy_server_nameservers(
                        &subscriptions(),
                        active_subscription_id(),
                    ),
                    settings,
                    group_selections(),
                );
                async move {
                    if core_busy() || runtime_action_busy() || system_proxy_busy() {
                        return;
                    }
                    restart_core_from_ui(request, status, toast, "sing-box 内核已重启").await;
                }
            },
            if is_restarting {
                span { class: "spinner" }
            } else {
                Icon { icon: LdRefreshCw, width: 15, height: 15 }
            }
            span { "重启内核" }
        }
    }
}
#[component]
fn ToastViewport() -> Element {
    let toast = use_context::<ToastManager>();

    rsx! {
        div {
            class: "toast-viewport",
            aria_live: "polite",
            aria_relevant: "additions",
            for item in (toast.messages)() {
                div {
                    class: match item.kind {
                        ToastKind::Info => "toast toast-info",
                        ToastKind::Success => "toast toast-success",
                        ToastKind::Error => "toast toast-error",
                    },
                    role: if item.kind == ToastKind::Error { "alert" } else { "status" },
                    span { class: "toast-icon",
                        match item.kind {
                            ToastKind::Info => rsx! { Icon { icon: LdInfo, width: 18, height: 18 } },
                            ToastKind::Success => rsx! { Icon { icon: LdCircleCheck, width: 18, height: 18 } },
                            ToastKind::Error => rsx! { Icon { icon: LdCircleAlert, width: 18, height: 18 } },
                        }
                    }
                    span { class: "toast-message", "{item.message}" }
                    button {
                        class: "toast-close",
                        title: "关闭提示",
                        aria_label: "关闭提示",
                        onclick: move |_| toast.dismiss(item.id),
                        Icon { icon: LdX, width: 16, height: 16 }
                    }
                }
            }
        }
    }
}

#[component]
/// Sidebar identity. The kicker is the real build (`EMBEDDED · 1.13.14`), not a
/// fixed tag, so the shell the user is driving is readable at a glance.
#[component]
fn Brand(kicker: String) -> Element {
    rsx! {
        div { class: "brand",
            div {
                class: "brand-mark",
                aria_hidden: "true",
                dangerous_inner_html: BRAND_ICON_SVG,
            }
            div {
                strong { "Kitty Pro" }
                small { "{kicker}" }
            }
        }
    }
}

/// Render the reported core build as the sidebar kicker: a core that reports
/// `kitty-pro-embedded/1.13.14` reads as `EMBEDDED · 1.13.14`.
fn brand_kicker(core_version: Option<String>) -> String {
    match core_version {
        Some(version) => version
            .strip_prefix("kitty-pro-")
            .unwrap_or(&version)
            .replace('/', " · ")
            .to_uppercase(),
        None => "SING-BOX CLIENT".to_string(),
    }
}

#[component]
fn NavItem(
    view: AppView,
    mut active_view: Signal<AppView>,
    /// Trailing count chip (nodes, subscriptions, rules, log entries).
    count: Option<String>,
) -> Element {
    let active = active_view() == view;
    rsx! {
        button {
            class: if active { "nav-item active" } else { "nav-item" },
            aria_current: if active { "page" },
            onclick: move |_| active_view.set(view),
            match view {
                AppView::Overview => rsx! { Icon { icon: LdLayoutDashboard, width: 20, height: 20 } },
                AppView::Nodes => rsx! { Icon { icon: LdGitFork, width: 20, height: 20 } },
                AppView::Subscriptions => rsx! { Icon { icon: LdRss, width: 20, height: 20 } },
                AppView::Rules => rsx! { Icon { icon: LdSplit, width: 20, height: 20 } },
                AppView::Logs => rsx! { Icon { icon: LdScrollText, width: 20, height: 20 } },
                AppView::Settings => rsx! { Icon { icon: LdSlidersHorizontal, width: 20, height: 20 } },
            }
            span { "{view.title()}" }
            if let Some(count) = count {
                span { class: "nav-count", "{count}" }
            }
        }
    }
}

#[component]
fn OverviewView(
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    mut active_view: Signal<AppView>,
    proxy_server_nameservers: Vec<String>,
    mut connected: Signal<bool>,
    mut core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    mut core_state: Signal<String>,
    mut core_note: Signal<Option<String>>,
    core_version: Signal<Option<String>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    connection_allowed: bool,
    latency_results: Signal<HashMap<String, NodeLatency>>,
    traffic: Signal<TrafficDisplay>,
    uptime_seconds: Signal<Option<u64>>,
    subscription_count: usize,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    group_selections: Signal<HashMap<String, String>>,
    mut system_proxy: Signal<SystemProxyLoadState>,
    system_proxy_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
    mut dark_mode: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let selected_node = nodes().into_iter().find(|node| node.tag == selected_tag());
    let is_connected = connected();
    let is_core_busy = core_busy();
    let is_core_restarting = core_restarting();
    let is_system_proxy_busy = system_proxy_busy();
    let is_runtime_action_busy = runtime_action_busy();
    let is_core_action_busy =
        is_core_busy || is_core_restarting || is_system_proxy_busy || is_runtime_action_busy;
    let status_title = if is_core_restarting {
        "正在重启"
    } else if is_system_proxy_busy {
        "系统代理设置中"
    } else if is_runtime_action_busy {
        "正在应用设置"
    } else if is_core_busy {
        if is_connected {
            "正在断开"
        } else {
            "正在启动"
        }
    } else if is_connected {
        "已连接"
    } else {
        "未连接"
    };
    let node_name = selected_node
        .as_ref()
        .map(|node| node.name.as_str())
        .unwrap_or("未选择节点");
    let mode_name = mode_label(tunnel_mode());
    let note = if is_core_restarting {
        "正在重启 sing-box 内核，请稍候".to_string()
    } else if is_system_proxy_busy {
        "正在设置系统代理，请稍候".to_string()
    } else if is_runtime_action_busy {
        "正在应用运行时设置，请稍候".to_string()
    } else if is_core_busy {
        if is_connected {
            "正在停止 sing-box 内核".to_string()
        } else {
            "正在启动 sing-box 内核，请稍候".to_string()
        }
    } else {
        core_note().unwrap_or_else(|| {
            if core_state() == "checking" {
                "正在检查 sing-box".to_string()
            } else {
                "sing-box 已就绪".to_string()
            }
        })
    };
    let selected_latency = selected_node.as_ref().and_then(|node| {
        latency_results()
            .get(&node.tag)
            .and_then(|result| result.latency_ms)
    });
    let traffic = traffic();
    let chart_peak = traffic
        .download_history
        .iter()
        .chain(traffic.upload_history.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);
    let chart_bars: Vec<(String, String)> = (0..TRAFFIC_HISTORY_LEN)
        .map(|index| {
            let offset = index + TRAFFIC_HISTORY_LEN - traffic.download_history.len();
            let download = traffic.download_history.get(offset).copied().unwrap_or(0);
            let upload = traffic.upload_history.get(offset).copied().unwrap_or(0);
            (
                traffic_bar_height(download, chart_peak),
                traffic_bar_height(upload, chart_peak),
            )
        })
        .collect();
    let chart_status = if traffic.active_connections > 0 {
        format!("{} 个活动连接", traffic.active_connections)
    } else if is_connected {
        "等待代理流量".to_string()
    } else {
        "内核未启动".to_string()
    };
    let uptime = uptime_seconds();
    let uptime_label = uptime
        .map(format_uptime)
        .unwrap_or_else(|| "--".to_string());
    let uptime_unit = if uptime.is_some() {
        "运行中"
    } else {
        "未启动"
    };
    let latency_unit = if selected_latency.is_some() {
        "ms"
    } else {
        "ms · 未测试"
    };

    rsx! {
        div { class: "overview-grid",
            section { class: if is_core_action_busy { "connection-panel glass-surface busy" } else if is_connected { "connection-panel glass-surface connected" } else { "connection-panel glass-surface" },
                div { class: "connection-copy",
                    div { class: "status-line", role: "status", aria_live: "polite",
                        span { class: if is_core_action_busy { "busy-dot" } else if is_connected { "live-dot" } else { "idle-dot" } }
                        span { "{status_title} · {mode_name}" }
                    }
                    h2 { "{node_name}" }
                    p { "{note}" }
                    if let Some(node) = selected_node.as_ref() {
                        div { class: "connection-meta",
                            span { "{node.endpoint()}" }
                            span { class: "meta-sep", aria_hidden: "true" }
                            span { "{node.protocol.label()}" }
                        }
                    }
                    // Design affordance: jump straight to the picker instead of
                    // making the user hunt for the nodes page.
                    if !nodes().is_empty() {
                        button {
                            class: "secondary-button compact switch-node-button",
                            onclick: move |_| active_view.set(AppView::Nodes),
                            Icon { icon: LdSplit, width: 15, height: 15 }
                            span { "切换节点" }
                        }
                    }
                }
                div { class: "connection-controls",
                    button {
                        class: if is_core_action_busy && is_connected { "power-button active loading" } else if is_core_action_busy { "power-button loading" } else if is_connected { "power-button active" } else { "power-button" },
                        title: if is_core_restarting { "正在重启内核" } else if is_system_proxy_busy { "正在设置系统代理" } else if is_runtime_action_busy { "正在应用运行时设置" } else if is_core_busy { "正在切换连接状态" } else if is_connected { "断开连接" } else { "建立连接" },
                        aria_label: if is_connected { "停止内核" } else { "启动内核" },
                        aria_busy: is_core_action_busy,
                        disabled: is_core_action_busy || (!is_connected && !connection_allowed),
                        onclick: move |_| {
                            let target = !connected();
                            let request = target.then(|| ConnectionRequest {
                                nodes: nodes(),
                                selected_tag: selected_tag(),
                                proxy_server_nameservers: proxy_server_nameservers.clone(),
                                mode: tunnel_mode(),
                                tun: tun_enabled(),
                                allow_lan: allow_lan(),
                                dns_overrides: dns_overrides(),
                                custom_rules: connection_custom_rules(
                                    config_script_enabled(),
                                    &config_script(),
                                    custom_rules(),
                                ),
                                config_script: if config_script_enabled()
                                    && !config_script().trim().is_empty()
                                {
                                    Some(config_script())
                                } else {
                                    None
                                },
                                group_selections: group_selections(),
                            });
                            spawn_core_toggle(
                                request,
                                CoreToggleSignals {
                                    status: CoreStatusSignals {
                                        connected,
                                        core_restarting,
                                        core_state,
                                        core_version,
                                        core_note,
                                    },
                                    core_busy,
                                    system_proxy,
                                },
                                toast,
                            );
                        },
                        if is_core_action_busy {
                            span { class: "spinner large" }
                        } else {
                            Icon { icon: LdPower, width: 32, height: 32 }
                        }
                        span { class: "power-label", if is_connected { "断开" } else { "启动代理" } }
                    }
                    SystemProxyToggle {
                        core_running: is_connected,
                        core_action_busy: is_core_busy
                            || is_core_restarting
                            || is_runtime_action_busy,
                        system_proxy,
                        system_proxy_busy,
                    }
                }
            }

            section { class: "traffic-panel glass-surface",
                div { class: "section-heading",
                    div { class: "chart-heading",
                        h2 { "实时流量" }
                        span { class: "chart-window", "{TRAFFIC_HISTORY_LEN}s" }
                    }
                    div { class: "chart-legend",
                        span {
                            class: "down",
                            title: "累计下载 {format_data_amount(traffic.download_total)}",
                            i {}
                            "下载 {format_data_rate(traffic.download_bytes_per_second)}"
                        }
                        span {
                            class: "up",
                            title: "累计上传 {format_data_amount(traffic.upload_total)}",
                            i {}
                            "上传 {format_data_rate(traffic.upload_bytes_per_second)}"
                        }
                    }
                }
                div { class: "traffic-chart",
                    // One bar per second, download stacked under upload. An idle
                    // second keeps a 2px stub so the baseline stays readable.
                    for (download, upload) in chart_bars {
                        div { class: "traffic-bar",
                            span { class: "download", style: "height: {download}" }
                            span { class: "upload", style: "height: {upload}" }
                        }
                    }
                }
                div { class: "chart-foot",
                    small { "累计 {format_data_amount(traffic.download_total + traffic.upload_total)}" }
                    small { class: "traffic-status", "{chart_status} · {proxy_port_label(allow_lan())}" }
                }
            }

            // Phone-only: TUN / LAN / theme / system proxy in one row under the
            // live traffic card, exactly where the design puts the quick toggles.
                OverviewQuickToggles {
                    nodes,
                    selected_tag,
                    proxy_server_nameservers: proxy_server_nameservers.clone(),
                    connected,
                    core_busy,
                    core_restarting,
                    core_state,
                    core_version,
                    core_note,
                    tunnel_mode,
                    tun_enabled,
                    allow_lan,
                    dark_mode,
                    custom_rules,
                    dns_overrides,
                    config_script_enabled,
                    config_script,
                    group_selections,
                    system_proxy,
                    system_proxy_busy,
                    runtime_action_busy,
                }
        }

        div { class: "stats-grid",
            MetricCard {
                label: "节点".to_string(),
                icon: "nodes".to_string(),
                value: nodes().len().to_string(),
                unit: format!("/ 共 {subscription_count} 订阅"),
            }
            MetricCard {
                label: "运行时间".to_string(),
                icon: "timer".to_string(),
                value: uptime_label,
                unit: uptime_unit.to_string(),
            }
            MetricCard {
                label: "当前延迟".to_string(),
                icon: "zap".to_string(),
                value: selected_latency
                    .map(|latency| latency.to_string())
                    .unwrap_or_else(|| "--".to_string()),
                unit: latency_unit.to_string(),
            }
        }

        section { class: "list-section glass-surface",
            div { class: "section-heading",
                div { class: "list-heading",
                    p { class: "eyebrow", "PROXIES" }
                    h2 { "节点速览" }
                }
                span { class: "count-badge", "{nodes().len()} 个节点" }
            }
            if nodes().is_empty() {
                EmptyNodes {}
            } else {
                div { class: "node-list compact-list",
                    for node in nodes().into_iter().take(4) {
                        NodeRow { node, selected_tag, latency_results }
                    }
                }
            }
        }
    }
}

#[component]
fn SystemProxyToggle(
    core_running: bool,
    core_action_busy: bool,
    mut system_proxy: Signal<SystemProxyLoadState>,
    mut system_proxy_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let proxy_busy = system_proxy_busy();
    let proxy = system_proxy_view(system_proxy(), proxy_busy);
    let SystemProxyView {
        label: proxy_label,
        detail: proxy_detail,
        enabled: proxy_enabled,
        supported: proxy_supported,
        ready: proxy_ready,
        loading: proxy_loading,
        busy: _,
    } = proxy;

    rsx! {
        label {
            class: "overview-proxy-toggle",
            title: "{proxy_detail}",
            div {
                strong { "系统代理" }
                small { "{proxy_label}" }
            }
            if proxy_loading || proxy_busy {
                span {
                    class: if proxy_enabled { "switch switch-loading active" } else { "switch switch-loading" },
                    aria_busy: "true",
                    aria_label: if proxy_busy { "正在设置系统代理" } else { "正在读取系统代理状态" },
                    span { class: "spinner" }
                }
            } else {
                input {
                    r#type: "checkbox",
                    aria_label: "系统代理",
                    checked: proxy_enabled,
                    disabled: core_action_busy
                        || !proxy_ready
                        || !proxy_supported
                        || (!proxy_enabled && !core_running),
                    onchange: move |event| {
                        let enabled = event.checked();
                        let blocked = core_action_busy || system_proxy_busy();
                        if !blocked {
                            system_proxy_busy.set(true);
                        }
                        async move {
                            if blocked {
                                return;
                            }
                            match api::set_system_proxy(enabled).await {
                                Ok(status) => {
                                    system_proxy.set(SystemProxyLoadState::Ready(status));
                                    if enabled {
                                        toast.success("系统代理已启用");
                                    } else {
                                        toast.success("系统代理已关闭");
                                    }
                                }
                                Err(error) => {
                                    toast.error(format!("系统代理设置失败: {error}"));
                                    match api::system_proxy_status().await {
                                        Ok(status) => {
                                            system_proxy.set(SystemProxyLoadState::Ready(status));
                                        }
                                        Err(refresh_error) => {
                                            system_proxy.set(SystemProxyLoadState::Failed(
                                                refresh_error.to_string(),
                                            ));
                                        }
                                    }
                                }
                            }
                            system_proxy_busy.set(false);
                        }
                    },
                }
                span { class: "switch" }
            }
        }
    }
}

/// Height of one stacked bar segment. The chart keeps a floor so an idle
/// second still reads as a bar instead of collapsing to nothing.
fn traffic_bar_height(sample: u64, peak: u64) -> String {
    if sample == 0 {
        return "2px".to_string();
    }
    let percent = 10 + sample.saturating_mul(84).saturating_div(peak.max(1));
    format!("{}%", percent.min(88))
}

fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

fn format_data_rate(bytes_per_second: u64) -> String {
    format!("{}/s", format_data_amount(bytes_per_second))
}

/// `7890` is the fixed mixed port the embedded core always listens on.
fn proxy_port_label(allow_lan: bool) -> &'static str {
    if allow_lan {
        "局域网代理 7890"
    } else {
        "本地代理 7890"
    }
}

fn format_data_amount(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else if bytes < GIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    }
}

/// Derived display state of the system proxy control, shared by the desktop
/// toggle row and the phone quick toggle tile.
struct SystemProxyView {
    label: &'static str,
    detail: String,
    enabled: bool,
    supported: bool,
    ready: bool,
    loading: bool,
    busy: bool,
}

fn system_proxy_view(state: SystemProxyLoadState, busy: bool) -> SystemProxyView {
    let (status, loading, error) = match state {
        SystemProxyLoadState::Loading => (None, true, None),
        SystemProxyLoadState::Ready(status) => (Some(status), false, None),
        SystemProxyLoadState::Failed(error) => (None, false, Some(error)),
    };
    let enabled = status.as_ref().is_some_and(|status| status.enabled);
    let supported = status.as_ref().is_some_and(|status| status.supported);
    let label = if busy {
        "设置中"
    } else if loading {
        "读取中"
    } else if enabled {
        "已启用"
    } else if error.is_some() {
        "读取失败"
    } else if !supported {
        "不可用"
    } else {
        "未启用"
    };
    let detail = if busy {
        if enabled {
            "正在关闭系统代理".to_string()
        } else {
            "正在应用到系统网络服务".to_string()
        }
    } else {
        status
            .as_ref()
            .map(|status| status.detail.clone())
            .or(error)
            .unwrap_or_else(|| "正在读取系统代理状态".to_string())
    };
    SystemProxyView {
        label,
        detail,
        enabled,
        supported,
        ready: status.is_some(),
        loading,
        busy,
    }
}

/// One tile of the phone quick-toggle grid: icon plus status dot on the first
/// row, name and monospace value below, matching the design's 2x2 grid.
#[component]
fn QuickToggle(
    icon: String,
    title: String,
    value: String,
    checked: bool,
    busy: bool,
    disabled: bool,
    hint: String,
    on_toggle: EventHandler<bool>,
) -> Element {
    let control = if busy {
        rsx! {
            span {
                class: if checked { "switch switch-loading active" } else { "switch switch-loading" },
                aria_busy: "true",
                aria_label: "{title}",
                span { class: "spinner" }
            }
        }
    } else {
        rsx! {
            input {
                r#type: "checkbox",
                aria_label: "{title}",
                checked,
                disabled,
                onchange: move |event| on_toggle.call(event.checked()),
            }
            span { class: "switch" }
        }
    };
    rsx! {
        label { class: if checked { "quick-toggle active" } else { "quick-toggle" }, title: "{hint}",
            span { class: "quick-toggle-icon",
                if icon == "network" {
                    Icon { icon: LdNetwork, width: 16, height: 16 }
                } else if icon == "wifi" {
                    Icon { icon: LdWifi, width: 16, height: 16 }
                } else if icon == "moon" {
                    if checked {
                        Icon { icon: LdMoon, width: 16, height: 16 }
                    } else {
                        Icon { icon: LdSun, width: 16, height: 16 }
                    }
                } else {
                    Icon { icon: LdGlobe, width: 16, height: 16 }
                }
                span { class: if checked { "quick-toggle-dot on" } else { "quick-toggle-dot" } }
            }
            {control}
            span { class: "quick-toggle-text",
                strong { "{title}" }
                small { "{value}" }
            }
        }
    }
}

/// Phone-only 2x2 grid: the design keeps TUN, LAN, theme, and system proxy one
/// tap away from the power button. Desktop keeps those in the settings page.
#[component]
fn OverviewQuickToggles(
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    proxy_server_nameservers: Vec<String>,
    connected: Signal<bool>,
    core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    tunnel_mode: Signal<TunnelMode>,
    mut tun_enabled: Signal<bool>,
    mut allow_lan: Signal<bool>,
    mut dark_mode: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    group_selections: Signal<HashMap<String, String>>,
    system_proxy: Signal<SystemProxyLoadState>,
    system_proxy_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let tun_busy = use_signal(|| false);
    let runtime = RuntimeSignals {
        status: CoreStatusSignals {
            connected,
            core_restarting,
            core_state,
            core_version,
            core_note,
        },
        core_busy,
        system_proxy_busy,
        runtime_action_busy,
    };
    let proxy = system_proxy_view(system_proxy(), system_proxy_busy());
    let runtime_settings = RuntimeSettings {
        tunnel_mode,
        tun_enabled,
        allow_lan,
        custom_rules,
        dns_overrides,
        config_script_enabled,
        config_script,
    };
    let busy = runtime.busy();
    let core_running = connected();
    let show_tun = !cfg!(target_os = "android");
    // Each tile handler needs its own copy of the nameserver list.
    let tun_nameservers = proxy_server_nameservers.clone();

    rsx! {
        section { class: "quick-toggles glass-surface",
            div { class: "quick-toggles-heading",
                div {
                    strong { "快捷开关" }
                    p { class: "eyebrow", "QUICK TOGGLES" }
                }
            }
            div { class: "quick-toggle-grid",
                if show_tun {
                    QuickToggle {
                        icon: "network".to_string(),
                        title: "TUN 模式".to_string(),
                        value: if tun_busy() {
                            "处理中".to_string()
                        } else if tun_enabled() {
                            "已启用".to_string()
                        } else {
                            "未启用".to_string()
                        },
                        hint: "系统级网络接管".to_string(),
                        checked: tun_enabled(),
                        busy: tun_busy(),
                        disabled: busy || nodes().is_empty(),
                        on_toggle: move |enabled| {
                            let request = build_connection_request(
                                nodes(),
                                selected_tag(),
                                tun_nameservers.clone(),
                                runtime_settings,
                                group_selections(),
                            );
                            spawn_tun_toggle(
                                enabled,
                                request,
                                tun_enabled,
                                tun_busy,
                                runtime,
                                toast,
                            );
                        },
                    }
                }
                QuickToggle {
                    icon: "wifi".to_string(),
                    title: "局域网".to_string(),
                    value: if busy {
                        "处理中".to_string()
                    } else if allow_lan() {
                        "0.0.0.0:7890".to_string()
                    } else {
                        "仅本机".to_string()
                    },
                    hint: "允许同一局域网设备使用此代理".to_string(),
                    checked: allow_lan(),
                    busy: false,
                    disabled: busy || nodes().is_empty(),
                    on_toggle: move |enabled| {
                        let request = build_connection_request(
                            nodes(),
                            selected_tag(),
                            proxy_server_nameservers.clone(),
                            runtime_settings,
                            group_selections(),
                        );
                        spawn_allow_lan_toggle(
                            enabled,
                            request,
                            allow_lan,
                            runtime,
                            toast,
                        );
                    },
                }
                QuickToggle {
                    icon: "moon".to_string(),
                    title: "深色".to_string(),
                    value: if dark_mode() { "已开启".to_string() } else { "已关闭".to_string() },
                    hint: "切换界面配色".to_string(),
                    checked: dark_mode(),
                    busy: false,
                    disabled: false,
                    on_toggle: move |enabled| dark_mode.set(enabled),
                }
                QuickToggle {
                    icon: "globe".to_string(),
                    title: "系统代理".to_string(),
                    value: proxy.label.to_string(),
                    hint: proxy.detail,
                    checked: proxy.enabled,
                    busy: proxy.busy || proxy.loading,
                    disabled: busy
                        || !proxy.ready
                        || !proxy.supported
                        || (!proxy.enabled && !core_running),
                    on_toggle: move |enabled| {
                        spawn_system_proxy_toggle(
                            enabled,
                            system_proxy,
                            system_proxy_busy,
                            busy,
                            toast,
                        );
                    },
                }
            }
        }
    }
}

#[component]
fn MetricCard(label: String, value: String, icon: String, unit: String) -> Element {
    rsx! {
        div { class: "metric-card glass-surface",
            p {
                if icon == "timer" {
                    Icon { icon: LdTimer, width: 15, height: 15 }
                } else if icon == "zap" {
                    Icon { icon: LdWifi, width: 15, height: 15 }
                } else {
                    Icon { icon: LdGitFork, width: 15, height: 15 }
                }
                "{label}"
            }
            strong { "{value}" small { "{unit}" } }
        }
    }
}

#[component]
fn NodesView(
    groups: Vec<ProxyGroup>,
    groups_loading: bool,
    groups_error: Option<String>,
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
    all_count: usize,
    subscriptions: Vec<Subscription>,
    mut group_selections: Signal<HashMap<String, String>>,
    connected: Signal<bool>,
    core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    system_proxy_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    selected_tag: Signal<String>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    mut search: Signal<String>,
    mut latency_results: Signal<HashMap<String, NodeLatency>>,
    mut latency_busy: Signal<bool>,
    mut import_open: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let needle = search().trim().to_ascii_lowercase();
    let latency_snapshot = latency_results();
    // The design lists every proxy group as its own section instead of a
    // master/detail split, so one scroll shows the whole selector tree.
    let groups_with_members = groups
        .iter()
        .map(|group| {
            let mut members = proxy_group_members(group, &groups, &nodes, &needle);
            sort_proxy_group_members(&mut members, &latency_snapshot);
            let selected = group_selections()
                .get(&group.tag)
                .filter(|selected| group.outbounds.iter().any(|item| item == *selected))
                .cloned()
                .unwrap_or_else(|| group.selected.clone());
            (group.clone(), members, selected)
        })
        .filter(|(_, members, _)| !members.is_empty())
        .collect::<Vec<_>>();
    let listed_member_count = groups_with_members
        .iter()
        .map(|(_, members, _)| members.len())
        .sum::<usize>();
    let tested_count = latency_snapshot
        .values()
        .filter(|result| result.latency_ms.is_some())
        .count();
    let focus = node_focus_details(
        &groups,
        &nodes,
        &subscriptions,
        &selected_tag(),
        &latency_snapshot,
    );

    rsx! {
        section { class: "workspace-section glass-surface",
            if all_count == 0 {
                div { class: "large-empty",
                    EmptyNodes {}
                    button {
                        class: "primary-button",
                        onclick: move |_| import_open.set(true),
                        Icon { icon: LdPlus, width: 17, height: 17 }
                        "添加订阅"
                    }
                }
            } else if groups_loading {
                div { class: "large-empty",
                    span { class: "spinner large" }
                    strong { "正在生成代理组" }
                }
            } else if let Some(error) = groups_error {
                div { class: "large-empty",
                    Icon { icon: LdCircleAlert, width: 28, height: 28 }
                    strong { "代理组生成失败" }
                    p { "{error}" }
                }
            } else if groups.is_empty() {
                div { class: "large-empty",
                    Icon { icon: LdRoute, width: 28, height: 28 }
                    strong { "配置中没有代理组" }
                }
            } else {
                div { class: "nodes-toolbar",
                    label { class: "search-field nodes-search",
                        Icon { icon: LdSearch, width: 17, height: 17 }
                        input {
                            value: search,
                            placeholder: "搜索节点、地址或协议",
                            oninput: move |event| search.set(event.value()),
                        }
                    }
                                    button {
                                        class: "secondary-button compact",
                                        title: if latency_busy() { "正在刷新延迟" } else { "刷新延迟" },
                                        disabled: latency_busy() || nodes.is_empty(),
                                        onclick: move |_| {
                                            let probe_nodes = nodes.clone();
                                            let cache_nodes = nodes.clone();
                                            let proxy_server_nameservers =
                                                proxy_server_nameservers.clone();
                                            async move {
                                                latency_busy.set(true);
                                                let mut failures = 0;
                                                let session_id = match api::start_node_latency(
                                                    probe_nodes,
                                                    proxy_server_nameservers.clone(),
                                                ).await {
                                                    Ok(session_id) => session_id,
                                                    Err(error) => {
                                                        toast.error(format!("启动测速失败: {error}"));
                                                        latency_busy.set(false);
                                                        return;
                                                    }
                                                };
                                                let completed = loop {
                                                    match api::poll_node_latency(session_id).await {
                                                        Ok(snapshot) => {
                                                            failures += snapshot
                                                                .results
                                                                .iter()
                                                                .filter(|result| result.latency_ms.is_none())
                                                                .count();
                                                            if !snapshot.results.is_empty() {
                                                                let mut current_results = latency_results.write();
                                                                for result in snapshot.results {
                                                                    current_results.insert(result.tag.clone(), result);
                                                                }
                                                            }
                                                            if snapshot.done {
                                                                break snapshot.completed;
                                                            }
                                                        }
                                                        Err(error) => {
                                                            toast.error(format!("读取测速结果失败: {error}"));
                                                            latency_busy.set(false);
                                                            return;
                                                        }
                                                    }
                                                    wait_for_latency_poll().await;
                                                };
                                                if failures == 0 {
                                                    toast.success(format!("已完成 {completed} 个节点测速"));
                                                } else {
                                                    toast.info(format!(
                                                        "延迟刷新完成，{completed} 个节点中 {failures} 个不可用"
                                                    ));
                                                }
                                                let latency_snapshot = latency_results();
                                                persist_latency_cache(&cache_nodes, &latency_snapshot)
                                                    .await;
                                                latency_busy.set(false);
                                            }
                                        },
                                        if latency_busy() {
                                            span { class: "spinner" }
                                        } else {
                                            Icon { icon: LdRefreshCw, width: 16, height: 16 }
                                        }
                                        span { class: "nodes-refresh-label", "刷新延迟" }
                                    }
                    div { class: "nodes-toolbar-meta",
                        span { class: "nodes-toolbar-count",
                            "{groups_with_members.len()} 个分组 · {listed_member_count} 个节点"
                        }
                        small {
                            if tested_count == 0 {
                                "尚未测试延迟"
                            } else {
                                "已测速 {tested_count} 个节点"
                            }
                        }
                    }
                }
                if groups_with_members.is_empty() {
                    div { class: "large-empty group-empty",
                        Icon { icon: LdSearch, width: 28, height: 28 }
                        strong { "没有匹配的节点" }
                        p { "换个关键词试试，节点名、地址和协议都可以搜索。" }
                    }
                } else {
                    div { class: "proxy-group-sections",
                        for (group, members, selected) in groups_with_members {
                            div { class: "proxy-group-section", key: "{group.tag}",
                                div { class: "proxy-group-section-header",
                                    span {
                                        class: if group.kind == ProxyGroupKind::UrlTest {
                                            "group-kind-mark auto"
                                        } else {
                                            "group-kind-mark"
                                        },
                                        if group.kind == ProxyGroupKind::UrlTest {
                                            Icon { icon: LdGauge, width: 17, height: 17 }
                                        } else {
                                            Icon { icon: LdLayers, width: 17, height: 17 }
                                        }
                                    }
                                    div { class: "proxy-group-section-copy",
                                        strong { "{group.tag}" }
                                        small {
                                            "{proxy_group_kind_label(group.kind)} · {members.len()} 节点 · 当前 "
                                            {outbound_display_name(&selected, &nodes)}
                                        }
                                    }
                                }
                                div { class: "node-table-header",
                                    span { "协议" }
                                    span { "节点" }
                                    span { "类型" }
                                    span { "延迟" }
                                    span { "" }
                                }
                                div { class: "node-list proxy-member-list",
                                    for member in members {
                                        ProxyGroupMemberRow {
                                            group_tag: group.tag.clone(),
                                            group_kind: group.kind,
                                            member,
                                            selected: selected.clone(),
                                            connected,
                                            core_busy,
                                            core_restarting,
                                            system_proxy_busy,
                                            runtime_action_busy,
                                            core_state,
                                            core_version,
                                            core_note,
                                            nodes: nodes.clone(),
                                            proxy_server_nameservers: proxy_server_nameservers.clone(),
                                            selected_tag,
                                            tunnel_mode,
                                            tun_enabled,
                                            allow_lan,
                                            custom_rules,
                                            dns_overrides,
                                            config_script_enabled,
                                            config_script,
                                            group_selections,
                                            latency_results,
                                        }
                                    }
                                }
                            }
                        }
                    }
                  // The design keeps one detail card under the group tables so the
                  // focused node stays readable on phones, where rows are compacted.
                  if let Some(focus) = focus {
                      div { class: "node-focus",
                          div { class: "node-focus-header",
                              Icon { icon: LdInfo, width: 15, height: 15 }
                              strong { "节点信息" }
                              small { "· {focus.name}" }
                          }
                          div { class: "node-focus-rows",
                              div { class: "node-focus-row",
                                  span { "协议" }
                                  code { "{focus.protocol}" }
                              }
                              div { class: "node-focus-row",
                                  span { "地址" }
                                  code { "{focus.endpoint}" }
                              }
                              div { class: "node-focus-row",
                                  span { "所属分组" }
                                  code { "{focus.group_label}" }
                              }
                              div { class: "node-focus-row",
                                  span { "当前状态" }
                                  code { class: "node-focus-status {focus.status_class}", "{focus.status}" }
                              }
                              div { class: "node-focus-row",
                                  span { "来源" }
                                  code { "{focus.source}" }
                              }
                          }
                      }
                  }
                }
            }
        }
    }
}

/// Details shown for the node the core is currently using. Every field comes
/// from the parsed profile: the source is resolved by matching the namespaced
/// node tag back to its subscription, so nothing is guessed.
#[derive(Debug, Clone, PartialEq)]
struct NodeFocusDetails {
    name: String,
    protocol: String,
    endpoint: String,
    group_label: String,
    status: String,
    status_class: &'static str,
    source: String,
}

fn node_focus_details(
    groups: &[ProxyGroup],
    nodes: &[ProxyNode],
    subscriptions: &[Subscription],
    selected_tag: &str,
    latency_results: &HashMap<String, NodeLatency>,
) -> Option<NodeFocusDetails> {
    let node = nodes
        .iter()
        .find(|candidate| candidate.tag == selected_tag)?;
    let owner = groups
        .iter()
        .find(|group| group.outbounds.iter().any(|tag| tag == selected_tag))
        .map(|group| format!("{} · {}", group.tag, proxy_group_kind_label(group.kind)))
        .unwrap_or_else(|| "未归组".to_string());
    let source = subscriptions
        .iter()
        // Stored nodes are already namespaced by `apply_subscription_report`, so
        // the tag can be matched directly without re-prefixing it.
        .find(|subscription| {
            subscription
                .nodes
                .iter()
                .any(|scoped| scoped.tag == node.tag)
        })
        .map(|subscription| source_label(&subscription.source))
        .unwrap_or_else(|| "本地内容".to_string());
    let latency = latency_results.get(&node.tag);
    let (status, status_class) = match latency.map(|entry| entry.latency_ms) {
        Some(Some(_)) => ("已选中 · 可用", "ready"),
        Some(None) => ("已选中 · 探测失败", "failed"),
        None => ("已选中 · 未测试", "pending"),
    };

    Some(NodeFocusDetails {
        name: node.name.clone(),
        protocol: node.protocol.label().to_string(),
        endpoint: node.endpoint(),
        group_label: owner,
        status: status.to_string(),
        status_class,
        source,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct ProxyGroupMember {
    tag: String,
    node: Option<ProxyNode>,
    nested_kind: Option<ProxyGroupKind>,
}

/// Members of one proxy group after the free-text filter. Members can be real
/// nodes or nested groups, so both are resolved against the shared lists.
fn proxy_group_members(
    group: &ProxyGroup,
    groups: &[ProxyGroup],
    nodes: &[ProxyNode],
    needle: &str,
) -> Vec<ProxyGroupMember> {
    group
        .outbounds
        .iter()
        .filter_map(|tag| {
            let node = nodes.iter().find(|node| node.tag == *tag).cloned();
            let nested_kind = groups
                .iter()
                .find(|candidate| candidate.tag == *tag)
                .map(|candidate| candidate.kind);
            let display_name = node
                .as_ref()
                .map(|node| node.name.as_str())
                .unwrap_or(tag.as_str());
            (needle.is_empty()
                || display_name.to_ascii_lowercase().contains(needle)
                || tag.to_ascii_lowercase().contains(needle)
                || node.as_ref().is_some_and(|node| {
                    node.server.to_ascii_lowercase().contains(needle)
                        || node.protocol.label().to_ascii_lowercase().contains(needle)
                }))
            .then(|| ProxyGroupMember {
                tag: tag.clone(),
                node,
                nested_kind,
            })
        })
        .collect()
}

#[component]
fn ProxyGroupMemberRow(
    group_tag: String,
    group_kind: ProxyGroupKind,
    member: ProxyGroupMember,
    selected: String,
    connected: Signal<bool>,
    core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    system_proxy_busy: Signal<bool>,
    mut runtime_action_busy: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
    mut selected_tag: Signal<String>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    mut group_selections: Signal<HashMap<String, String>>,
    latency_results: Signal<HashMap<String, NodeLatency>>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let is_selected = group_kind == ProxyGroupKind::Selector && selected == member.tag;
    let selectable = group_kind == ProxyGroupKind::Selector;
    let member_tag = member.tag.clone();
    let target_group = group_tag.clone();
    let (latency_label, latency_class, latency_title) = member
        .node
        .as_ref()
        .map(|node| format_latency(latency_results().get(&node.tag)))
        .unwrap_or_else(|| ("".to_string(), "latency pending", String::new()));
    let member_kind_label = member
        .nested_kind
        .map(proxy_group_kind_label)
        .unwrap_or_else(|| match member.tag.as_str() {
            "direct" => "直连",
            "block" => "拦截",
            _ => "代理节点",
        });

    rsx! {
        button {
            class: if is_selected { "node-row group-member-row selected" } else { "node-row group-member-row" },
            disabled: !selectable
                || core_busy()
                || core_restarting()
                || system_proxy_busy()
                || runtime_action_busy(),
            onclick: move |_| {
                let group = target_group.clone();
                let outbound = member_tag.clone();
                let nodes = nodes.clone();
                let proxy_server_nameservers = proxy_server_nameservers.clone();
                let blocked = core_busy()
                    || core_restarting()
                    || system_proxy_busy()
                    || runtime_action_busy();
                if !blocked {
                    runtime_action_busy.set(true);
                }
                async move {
                    if blocked {
                        return;
                    }
                    let previous_tag = selected_tag();
                    let previous_selections = group_selections();
                    let mut next_selections = previous_selections.clone();
                    next_selections.insert(group.clone(), outbound.clone());
                    if group == "proxy" {
                        selected_tag.set(outbound.clone());
                    }
                    if connected() {
                        match api::select_proxy_group(group.clone(), outbound.clone()).await {
                            Ok(()) => {
                                group_selections.set(next_selections);
                                toast.success(format!("{group} 已切换到 {outbound}"));
                            }
                            Err(_) => {
                                group_selections.set(next_selections.clone());
                                let request = build_connection_request(
                                    nodes,
                                    selected_tag(),
                                    proxy_server_nameservers,
                                    RuntimeSettings {
                                        tunnel_mode,
                                        tun_enabled,
                                        allow_lan,
                                        custom_rules,
                                        dns_overrides,
                                        config_script_enabled,
                                        config_script,
                                    },
                                    next_selections,
                                );
                                let restarted = restart_core_from_ui(
                                    request,
                                    CoreStatusSignals {
                                        connected,
                                        core_restarting,
                                        core_state,
                                        core_version,
                                        core_note,
                                    },
                                    toast,
                                    format!("{group} 已切换到 {outbound}，内核已重启"),
                                )
                                .await;
                                if !restarted {
                                    selected_tag.set(previous_tag);
                                    group_selections.set(previous_selections);
                                }
                            }
                        }
                        runtime_action_busy.set(false);
                        return;
                    }
                    group_selections.set(next_selections);
                    toast.success(format!("{group} 已切换到 {outbound}"));
                    runtime_action_busy.set(false);
                }
            },
            if let Some(ref node) = member.node {
                span { class: protocol_class(node.protocol), "{protocol_abbreviation(node.protocol)}" }
                span { class: "node-main",
                    strong { "{node.name}" }
                    small { "{node.endpoint()}" }
                }
                span { class: "protocol-label", "{node.protocol.label()}" }
                LatencyCell {
                    result: member.node.as_ref().and_then(|node| latency_results().get(&node.tag).cloned()),
                    latency_label,
                    latency_class,
                    latency_title,
                }
            } else {
                span { class: if member.nested_kind == Some(ProxyGroupKind::UrlTest) { "group-kind-mark auto" } else { "group-kind-mark" },
                    if member.tag == "direct" {
                        Icon { icon: LdWifi, width: 17, height: 17 }
                    } else if member.tag == "block" {
                        Icon { icon: LdShieldCheck, width: 17, height: 17 }
                    } else if member.nested_kind == Some(ProxyGroupKind::UrlTest) {
                        Icon { icon: LdGauge, width: 17, height: 17 }
                    } else {
                        Icon { icon: LdRoute, width: 17, height: 17 }
                    }
                }
                span { class: "node-main",
                    strong { "{member.tag}" }
                    small { "{member_kind_label}" }
                }
                span { class: "protocol-label", "{member_kind_label}" }
                span { class: "latency pending", "" }
            }
            if is_selected {
                span { class: "selected-check", Icon { icon: LdCircleCheck, width: 19, height: 19 } }
            } else if selectable {
                span { class: "row-chevron", Icon { icon: LdChevronRight, width: 18, height: 18 } }
            } else {
                span { class: "auto-indicator", Icon { icon: LdGauge, width: 17, height: 17 } }
            }
        }
    }
}

fn proxy_group_kind_label(kind: ProxyGroupKind) -> &'static str {
    match kind {
        ProxyGroupKind::Selector => "手动选择",
        ProxyGroupKind::UrlTest => "自动测速",
    }
}

fn outbound_display_name(tag: &str, nodes: &[ProxyNode]) -> String {
    nodes
        .iter()
        .find(|node| node.tag == tag)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| match tag {
            "direct" => "直连".to_string(),
            "block" => "拦截".to_string(),
            _ => tag.to_string(),
        })
}

#[component]
fn NodeRow(
    node: ProxyNode,
    mut selected_tag: Signal<String>,
    latency_results: Signal<HashMap<String, NodeLatency>>,
) -> Element {
    let selected = selected_tag() == node.tag;
    let tag = node.tag.clone();
    let (latency_label, latency_class, latency_title) =
        format_latency(latency_results().get(&node.tag));
    rsx! {
        button {
            class: if selected { "node-row selected" } else { "node-row" },
            onclick: move |_| selected_tag.set(tag.clone()),
            span { class: protocol_class(node.protocol), "{protocol_abbreviation(node.protocol)}" }
            span { class: "node-main",
                strong { "{node.name}" }
                // The overview preview keeps one mono line: endpoint then protocol.
                small { "{node.endpoint()} · {node.protocol.label()}" }
            }
            span { class: "protocol-label" }
            LatencyCell {
                result: latency_results().get(&node.tag).cloned(),
                latency_label,
                latency_class,
                latency_title,
            }
            if selected {
                span { class: "selected-check", Icon { icon: LdCircleCheck, width: 19, height: 19 } }
            } else {
                span { class: "row-chevron", Icon { icon: LdChevronRight, width: 18, height: 18 } }
            }
        }
    }
}

/// Latency column: meter bar plus the monospace millisecond figure from the
/// design. The meter only fills once a probe really returned a value.
#[component]
fn LatencyCell(
    result: Option<NodeLatency>,
    latency_label: String,
    latency_class: &'static str,
    latency_title: String,
) -> Element {
    let (fill, tone) = latency_meter(result.as_ref());
    rsx! {
        span { class: "latency-cell", title: latency_title,
            span { class: "latency-meter", aria_hidden: "true",
                span { class: "latency-meter-fill {tone}", style: "width: {fill}%" }
            }
            span { class: latency_class, "{latency_label}" }
        }
    }
}

fn restore_cached_latencies(
    nodes: &[ProxyNode],
    entries: Vec<LatencyCacheEntry>,
) -> HashMap<String, NodeLatency> {
    let cached_by_tag = entries
        .iter()
        .map(|(tag, endpoint, latency_ms)| ((tag.as_str(), endpoint.as_str()), *latency_ms))
        .collect::<HashMap<_, _>>();
    let cached_by_endpoint = entries
        .iter()
        .map(|(_, endpoint, latency_ms)| (endpoint.as_str(), *latency_ms))
        .collect::<HashMap<_, _>>();

    nodes
        .iter()
        .filter_map(|node| {
            let endpoint = node.endpoint();
            cached_by_tag
                .get(&(node.tag.as_str(), endpoint.as_str()))
                .or_else(|| cached_by_endpoint.get(endpoint.as_str()))
                .map(|latency_ms| {
                    (
                        node.tag.clone(),
                        NodeLatency {
                            tag: node.tag.clone(),
                            latency_ms: Some(*latency_ms),
                            error: None,
                        },
                    )
                })
        })
        .collect()
}

fn latency_cache_entries(
    nodes: &[ProxyNode],
    results: &HashMap<String, NodeLatency>,
) -> Vec<LatencyCacheEntry> {
    nodes
        .iter()
        .filter_map(|node| {
            results
                .get(&node.tag)
                .and_then(|result| result.latency_ms)
                .map(|latency_ms| (node.tag.clone(), node.endpoint(), latency_ms))
        })
        .collect()
}

async fn persist_latency_cache(nodes: &[ProxyNode], results: &HashMap<String, NodeLatency>) {
    let eval = document::eval(&save_latency_cache_script());
    if eval.send(latency_cache_entries(nodes, results)).is_ok() {
        let _ = eval.await;
    }
}

fn format_latency(result: Option<&NodeLatency>) -> (String, &'static str, String) {
    match result {
        Some(NodeLatency {
            latency_ms: Some(latency),
            ..
        }) => (
            format!("{latency} ms"),
            "latency success",
            "节点可用".to_string(),
        ),
        Some(NodeLatency {
            error: Some(error), ..
        }) => ("失败".to_string(), "latency error", error.clone()),
        _ => (
            "-- ms".to_string(),
            "latency pending",
            "尚未探测".to_string(),
        ),
    }
}

/// Latency meter shown next to the millisecond figure: the bar shrinks as the
/// probe gets slower, and stays empty until a node is actually measured.
fn latency_meter(result: Option<&NodeLatency>) -> (u8, &'static str) {
    match result.and_then(|latency| latency.latency_ms) {
        Some(ms) if ms < 80 => (100, "fast"),
        Some(ms) if ms < 180 => (76, "fast"),
        Some(ms) if ms < 350 => (52, "medium"),
        Some(_) => (28, "slow"),
        None => (0, "empty"),
    }
}

#[component]
fn EmptyNodes() -> Element {
    rsx! {
        div { class: "empty-state",
            span { class: "empty-icon", Icon { icon: LdGlobe, width: 26, height: 26 } }
            div {
                strong { "还没有节点" }
                p { "添加订阅后会显示在这里" }
            }
        }
    }
}

#[component]
fn SubscriptionsView(
    subscriptions: Signal<Vec<Subscription>>,
    active_subscription_id: Signal<Option<u64>>,
    mut import_open: Signal<bool>,
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    refresh_busy: Signal<Option<RefreshTarget>>,
    connected: Signal<bool>,
    core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    system_proxy_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    group_selections: Signal<HashMap<String, String>>,
) -> Element {
    let toast = use_context::<ToastManager>();
    rsx! {
        section { class: "workspace-section glass-surface",
            div { class: "workspace-toolbar",
                div {
                    p { class: "eyebrow", "SUBSCRIPTIONS" }
                    h2 { "订阅管理" }
                    span { "{subscriptions().len()} 个订阅" }
                }
                div { class: "toolbar-controls subscription-actions",
                    button {
                        class: "icon-button glass-control",
                        title: "刷新全部订阅",
                        disabled: subscriptions().is_empty() || refresh_busy().is_some(),
                        onclick: move |_| {
                            let sources: Vec<(u64, String)> = subscriptions()
                                .into_iter()
                                .map(|subscription| (subscription.id, subscription.source))
                                .collect();
                            async move {
                                refresh_busy.set(Some(RefreshTarget::All));
                                let mut refreshed = 0;
                                let mut failed = 0;
                                for (subscription_id, source) in sources {
                                    match api::preview_subscription(source).await {
                                        Ok(report) => {
                                            let result = {
                                                let mut stored = subscriptions.write();
                                                apply_subscription_report(&mut stored, subscription_id, report)
                                            };
                                            match result {
                                                Ok(_) => refreshed += 1,
                                                Err(_) => failed += 1,
                                            }
                                        }
                                        Err(_) => failed += 1,
                                    }
                                }
                                let active_nodes = collect_subscription_nodes(
                                    &subscriptions(),
                                    active_subscription_id(),
                                );
                                let next_tag =
                                    select_available_tag(&active_nodes, &selected_tag());
                                nodes.set(active_nodes);
                                selected_tag.set(next_tag);
                                if failed == 0 {
                                    toast.success(format!("已刷新 {refreshed} 个订阅"));
                                } else {
                                    toast.info(format!("已刷新 {refreshed} 个订阅，{failed} 个失败"));
                                }
                                refresh_busy.set(None);
                            }
                        },
                        if refresh_busy() == Some(RefreshTarget::All) {
                            span { class: "spinner" }
                        } else {
                            Icon { icon: LdRefreshCw, width: 17, height: 17 }
                        }
                    }
                    button {
                        class: "primary-button",
                        onclick: move |_| import_open.set(true),
                        Icon { icon: LdPlus, width: 17, height: 17 }
                        "添加订阅"
                    }
                }
            }
            if subscriptions().is_empty() {
                div { class: "large-empty subscriptions-empty",
                    span { class: "empty-icon", Icon { icon: LdRadioTower, width: 28, height: 28 } }
                    strong { "暂无订阅" }
                }
            } else {
                div { class: "subscription-list",
                    for subscription in subscriptions() {
                        SubscriptionRow {
                            subscription,
                            subscriptions,
                            active_subscription_id,
                            nodes,
                            selected_tag,
                            refresh_busy,
                            connected,
                            core_busy,
                            core_restarting,
                            system_proxy_busy,
                            runtime_action_busy,
                            core_state,
                            core_version,
                            core_note,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            custom_rules,
        dns_overrides,
                            config_script_enabled,
                            config_script,
                            group_selections,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SubscriptionRow(
    subscription: Subscription,
    mut subscriptions: Signal<Vec<Subscription>>,
    mut active_subscription_id: Signal<Option<u64>>,
    mut nodes: Signal<Vec<ProxyNode>>,
    mut selected_tag: Signal<String>,
    mut refresh_busy: Signal<Option<RefreshTarget>>,
    connected: Signal<bool>,
    core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    system_proxy_busy: Signal<bool>,
    mut runtime_action_busy: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    group_selections: Signal<HashMap<String, String>>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let subscription_id = subscription.id;
    let subscription_source = subscription.source.clone();
    let display_source = source_label(&subscription.source);
    let refreshing = refresh_busy() == Some(RefreshTarget::One(subscription_id));
    let active = active_subscription_id() == Some(subscription_id);

    rsx! {
        div { class: if active { "subscription-row active" } else { "subscription-row" },
            div { class: "subscription-main",
                strong { "{subscription.name}" }
                small { "{display_source}" }
            }
            div { class: "subscription-stats",
                strong { "{subscription.node_count()}" }
                small { "节点" }
            }
            if subscription.rejected_count > 0 {
                span { class: "warning-badge", "忽略 {subscription.rejected_count}" }
            }
            button {
                class: if active { "subscription-use-button active" } else { "subscription-use-button" },
                disabled: active
                    || refresh_busy().is_some()
                    || core_busy()
                    || core_restarting()
                    || system_proxy_busy()
                    || runtime_action_busy(),
                onclick: move |_| {
                    let blocked = core_busy()
                        || core_restarting()
                        || system_proxy_busy()
                        || runtime_action_busy();
                    if !blocked {
                        runtime_action_busy.set(true);
                    }
                    async move {
                        if blocked {
                            return;
                        }
                        let previous_subscription_id = active_subscription_id();
                        let previous_nodes = nodes();
                        let previous_tag = selected_tag();
                        let previous_selections = group_selections();
                        active_subscription_id.set(Some(subscription_id));
                        let active_nodes =
                            collect_subscription_nodes(&subscriptions(), Some(subscription_id));
                        let next_tag = select_available_tag(&active_nodes, "");
                        let mut next_selections = previous_selections.clone();
                        if next_tag.is_empty() {
                            next_selections.remove("proxy");
                        } else {
                            next_selections.insert("proxy".to_string(), next_tag.clone());
                        }
                        nodes.set(active_nodes.clone());
                        selected_tag.set(next_tag.clone());
                        group_selections.set(next_selections.clone());
                        if connected() {
                            let request = build_connection_request(
                                active_nodes,
                                next_tag,
                                active_subscription_proxy_server_nameservers(
                                    &subscriptions(),
                                    Some(subscription_id),
                                ),
                                RuntimeSettings {
                                    tunnel_mode,
                                    tun_enabled,
                                    allow_lan,
                                    custom_rules,
                                    dns_overrides,
                                    config_script_enabled,
                                    config_script,
                                },
                                next_selections,
                            );
                            let restarted = restart_core_from_ui(
                                request,
                                CoreStatusSignals {
                                    connected,
                                    core_restarting,
                                    core_state,
                                    core_version,
                                    core_note,
                                },
                                toast,
                                "已切换订阅，内核已重启",
                            )
                            .await;
                            if !restarted {
                                active_subscription_id.set(previous_subscription_id);
                                nodes.set(previous_nodes);
                                selected_tag.set(previous_tag);
                                group_selections.set(previous_selections);
                            }
                        } else {
                            toast.success("已切换订阅");
                        }
                        runtime_action_busy.set(false);
                    }
                },
                if active {
                    Icon { icon: LdCircleCheck, width: 15, height: 15 }
                    "当前使用"
                } else {
                    "使用"
                }
            }
            button {
                class: "icon-button subscription-refresh-button",
                title: "刷新订阅",
                disabled: refresh_busy().is_some(),
                onclick: move |_| {
                    let source = subscription_source.clone();
                    async move {
                    refresh_busy.set(Some(RefreshTarget::One(subscription_id)));
                    match api::preview_subscription(source).await {
                        Ok(report) => {
                            let result = {
                                let mut stored = subscriptions.write();
                                apply_subscription_report(&mut stored, subscription_id, report)
                            };
                            match result {
                                Ok(count) => {
                                    if active_subscription_id() == Some(subscription_id) {
                                        let active_nodes = collect_subscription_nodes(
                                            &subscriptions(),
                                            Some(subscription_id),
                                        );
                                        let next_tag = select_available_tag(
                                            &active_nodes,
                                            &selected_tag(),
                                        );
                                        nodes.set(active_nodes);
                                        selected_tag.set(next_tag);
                                    }
                                    toast.success(format!("已刷新 {count} 个节点"));
                                }
                                Err(reason) => {
                                    toast.error(format!("刷新失败: {reason}"));
                                }
                            }
                        }
                        Err(error) => toast.error(format!("刷新失败: {error}")),
                    }
                    refresh_busy.set(None);
                    }
                },
                if refreshing {
                    span { class: "spinner" }
                } else {
                    Icon { icon: LdRefreshCw, width: 17, height: 17 }
                }
            }
            button {
                class: "icon-button danger subscription-delete-button",
                title: "删除订阅",
                disabled: refresh_busy().is_some(),
                onclick: move |_| {
                    let deleting_active = active_subscription_id() == Some(subscription_id);
                    subscriptions.write().retain(|item| item.id != subscription_id);
                    let next_active_id = if deleting_active {
                        subscriptions().first().map(|item| item.id)
                    } else {
                        active_subscription_id()
                    };
                    active_subscription_id.set(next_active_id);
                    let active_nodes =
                        collect_subscription_nodes(&subscriptions(), next_active_id);
                    let requested_tag = if deleting_active {
                        String::new()
                    } else {
                        selected_tag()
                    };
                    let next_tag = select_available_tag(&active_nodes, &requested_tag);
                    nodes.set(active_nodes);
                    selected_tag.set(next_tag);
                    toast.success("订阅已删除");
                },
                Icon { icon: LdTrash2, width: 17, height: 17 }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RulesTab {
    Routing,
    Dns,
}

#[component]
fn RulesView(
    mut rules: Signal<Vec<CustomRule>>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    connected: Signal<bool>,
    tunnel_mode: Signal<TunnelMode>,
    mut rule_sets_busy: Signal<bool>,
    config_script_enabled: Signal<bool>,
    mut active_tab: Signal<RulesTab>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let mut editor_open = use_signal(|| false);
    let mut editing_id = use_signal(|| None::<u64>);
    let mut draft_match = use_signal(|| CustomRuleMatch::DomainSuffix);
    let mut draft_action = use_signal(|| CustomRuleAction::Direct);
    let mut draft_value = use_signal(String::new);
    let mut editor_error = use_signal(|| None::<String>);
    let mut process_picker_open = use_signal(|| false);
    let mut process_picker_loaded = use_signal(|| false);
    let mut process_picker_error = use_signal(|| None::<String>);
    let process_picker_list = use_signal(Vec::<api::RunningProcess>::new);
    let include_process_rules = process_rules_available();
    let all_stored = rules();
    let stored_rule_count = all_stored.len();
    let stored = all_stored
        .into_iter()
        .filter(|rule| custom_rule_visible(rule.match_type, include_process_rules))
        .collect::<Vec<_>>();
    let enabled_count = stored.iter().filter(|rule| rule.enabled).count();
    let total = stored.len();
    let rule_mode_active = tunnel_mode() == TunnelMode::Rule;
    let override_active = config_script_enabled();

    let mut open_new_editor = move || {
        editing_id.set(None);
        draft_match.set(CustomRuleMatch::DomainSuffix);
        draft_action.set(CustomRuleAction::Direct);
        draft_value.set(String::new());
        editor_error.set(None);
        editor_open.set(true);
    };
    let on_edit = move |rule: CustomRule| {
        editing_id.set(Some(rule.id));
        draft_match.set(rule.match_type);
        draft_action.set(rule.action);
        draft_value.set(rule.value);
        editor_error.set(None);
        editor_open.set(true);
    };
    let placeholder = custom_rule_placeholder(draft_match());
    let dns_rules = dns_overrides();
    let dns_total = dns_rules.len();
    let dns_enabled_count = dns_rules.iter().filter(|rule| rule.enabled).count();
    let mut dns_domain = use_signal(String::new);
    let mut dns_server = use_signal(String::new);
    let dns_error = use_signal(|| None::<String>);

    rsx! {
        section {
            class: if override_active {
                "workspace-section rules-workspace rules-override-active glass-surface"
            } else {
                "workspace-section rules-workspace glass-surface"
            },
            div { class: "workspace-toolbar rules-toolbar",
                div {
                    div { class: "section-title-line",
                        if active_tab() == RulesTab::Routing {
                            h2 { "自定义分流" }
                            span {
                                class: if override_active {
                                    "status-badge pending"
                                } else if rule_mode_active {
                                    "status-badge online"
                                } else {
                                    "status-badge"
                                },
                                if override_active {
                                    "脚本覆写"
                                } else if rule_mode_active {
                                    "规则模式"
                                } else {
                                    "未启用"
                                }
                            }
                        } else {
                            h2 { "专用 DNS" }
                            span {
                                class: if dns_enabled_count > 0 {
                                    "status-badge ready"
                                } else {
                                    "status-badge"
                                },
                                if dns_enabled_count > 0 { "已启用" } else { "未启用" }
                            }
                        }
                    }
                    if active_tab() == RulesTab::Routing {
                        p { "{enabled_count} 条启用 · {total} 条规则" }
                    } else {
                        p { "命中域名后缀用专用 DNS 解析并直连，任何分流模式都生效" }
                    }
                }
                div { class: "toolbar-controls rules-toolbar-actions",
                    div { class: "segmented-control rules-tab-control", aria_label: "规则类型",
                        button {
                            class: if active_tab() == RulesTab::Routing { "active" } else { "" },
                            onclick: move |_| active_tab.set(RulesTab::Routing),
                            "分流规则 {total}"
                        }
                        button {
                            class: if active_tab() == RulesTab::Dns { "active" } else { "" },
                            onclick: move |_| active_tab.set(RulesTab::Dns),
                            "专用 DNS {dns_total}"
                        }
                    }
                    if active_tab() == RulesTab::Routing {
                        button {
                            class: "secondary-button compact",
                            title: "立即更新分流规则",
                            aria_busy: rule_sets_busy(),
                            disabled: rule_sets_busy() || override_active,
                            onclick: move |_| async move {
                                rule_sets_busy.set(true);
                                match api::update_rule_sets(true).await {
                                    Ok(_) => toast.success("分流规则已更新"),
                                    Err(error) => toast.error(format!("分流规则更新失败: {error}")),
                                }
                                rule_sets_busy.set(false);
                            },
                            if rule_sets_busy() {
                                span { class: "spinner" }
                            } else {
                                Icon { icon: LdRefreshCw, width: 17, height: 17 }
                            }
                            span { class: "rules-refresh-label", "更新规则" }
                        }
                        button {
                            class: "primary-button compact",
                            disabled: override_active || stored_rule_count >= MAX_CUSTOM_RULES,
                            onclick: move |_| open_new_editor(),
                            Icon { icon: LdPlus, width: 17, height: 17 }
                            span { "添加规则" }
                        }
                    }
                }
            }

            if active_tab() == RulesTab::Dns {
                div { class: "dns-list",
                    div { class: "dns-table-header",
                        span { "序号" }
                        span { class: "dns-col-action", "解析动作" }
                        span { "域名后缀" }
                        span { "DNS 服务器" }
                        span { "启用" }
                        span { "操作" }
                    }
                    if dns_rules.is_empty() {
                        div { class: "dns-empty-row",
                            Icon { icon: LdGlobe, width: 18, height: 18 }
                            strong { "尚未配置专用 DNS" }
                            small { "为指定域名后缀固定 DNS，避免公共 DNS 解析到错误地址" }
                        }
                    }
                    for (index, rule) in dns_rules.into_iter().enumerate() {
                        div {
                            class: if rule.enabled { "dns-row" } else { "dns-row disabled" },
                            key: "{rule.id}",
                            span { class: "dns-index", "{index + 1:02}" }
                            span { class: "dns-rule-action",
                                Icon { icon: LdGlobe, width: 14, height: 14 }
                                "专用 DNS"
                            }
                            span { class: "dns-rule-domain", title: "{rule.domain_suffix}", "{rule.domain_suffix}" }
                            span { class: "dns-rule-server", title: "{rule.server}", "{rule.server}" }
                            label {
                                class: "rule-enabled-toggle",
                                title: if rule.enabled { "停用这条规则" } else { "启用这条规则" },
                                input {
                                    r#type: "checkbox",
                                    aria_label: if rule.enabled { "停用 {rule.domain_suffix} 的专用 DNS" } else { "启用 {rule.domain_suffix} 的专用 DNS" },
                                    checked: rule.enabled,
                                    onchange: move |event| {
                                        let checked = event.checked();
                                        dns_overrides.write().iter_mut().for_each(|item| {
                                            if item.id == rule.id {
                                                item.enabled = checked;
                                            }
                                        });
                                        notify_rule_change(connected(), toast);
                                    },
                                }
                                span { class: "switch" }
                            }
                            button {
                                class: "icon-button danger",
                                aria_label: "删除 {rule.domain_suffix} 的专用 DNS",
                                title: "删除",
                                onclick: move |_| {
                                    dns_overrides.write().retain(|item| item.id != rule.id);
                                    notify_rule_change(connected(), toast);
                                },
                                Icon { icon: LdTrash2, width: 16, height: 16 }
                            }
                        }
                    }
                    // Inline "add row" pinned to the bottom of the list, per design.
                    div { class: "dns-add-row",
                        label { class: "dns-add-field",
                            span { class: "sr-only", "域名后缀" }
                            input {
                                placeholder: "添加域名后缀，例如 et.net",
                                aria_label: "域名后缀",
                                value: "{dns_domain}",
                                oninput: move |event| dns_domain.set(event.value()),
                                onkeydown: move |event| {
                                    if event.key() == Key::Enter {
                                        add_dns_override_rule(
                                            dns_domain,
                                            dns_server,
                                            dns_overrides,
                                            dns_error,
                                            connected(),
                                            toast,
                                        );
                                    }
                                },
                            }
                        }
                        label { class: "dns-add-field",
                            span { class: "sr-only", "DNS 服务器" }
                            input {
                                placeholder: "DNS 服务器，例如 100.100.100.101",
                                aria_label: "DNS 服务器",
                                value: "{dns_server}",
                                oninput: move |event| dns_server.set(event.value()),
                                onkeydown: move |event| {
                                    if event.key() == Key::Enter {
                                        add_dns_override_rule(
                                            dns_domain,
                                            dns_server,
                                            dns_overrides,
                                            dns_error,
                                            connected(),
                                            toast,
                                        );
                                    }
                                },
                            }
                        }
                        button {
                            class: "secondary-button compact dns-add-button",
                            title: "添加专用 DNS 规则",
                            onclick: move |_| {
                                add_dns_override_rule(
                                    dns_domain,
                                    dns_server,
                                    dns_overrides,
                                    dns_error,
                                    connected(),
                                    toast,
                                );
                            },
                            Icon { icon: LdPlus, width: 15, height: 15 }
                            "添加"
                        }
                    }
                }
                if let Some(message) = dns_error() {
                    div { class: "inline-note", role: "alert",
                        Icon { icon: LdCircleAlert, width: 16, height: 16 }
                        span { "{message}" }
                    }
                }
            }

            if active_tab() == RulesTab::Routing {
            if override_active {
                div { class: "rule-override-notice", role: "status",
                    Icon { icon: LdScrollText, width: 17, height: 17 }
                    div {
                        strong { "JavaScript 配置覆写已启用" }
                        span { "自定义分流规则暂不可用" }
                    }
                }
            }

            if !override_active && include_process_rules {
                div { class: "process-rule-hint",
                    Icon { icon: LdInfo, width: 15, height: 15 }
                    span { "进程分流需开启 TUN 才能覆盖所有进程；仅系统代理模式下，进程规则只作用于走代理的进程。" }
                }
            }

            if editor_open() && !override_active {
                div { class: "rule-editor",
                    div { class: "rule-editor-grid",
                        div { class: "rule-field",
                            span { "动作" }
                            RuleSelect {
                                label: "动作".to_string(),
                                value: custom_rule_action_value(draft_action()).to_string(),
                                options: vec![
                                    RuleSelectOption::new("direct", "直连"),
                                    RuleSelectOption::new("proxy", "代理"),
                                    RuleSelectOption::new("block", "拦截"),
                                ],
                                on_select: move |value: String| {
                                    draft_action.set(parse_custom_rule_action(&value));
                                },
                            }
                        }
                        div { class: "rule-field",
                            span { "匹配类型" }
                            RuleSelect {
                                label: "匹配类型".to_string(),
                                value: custom_rule_match_value(draft_match()).to_string(),
                                options: custom_rule_match_options(include_process_rules),
                                on_select: move |value: String| {
                                    draft_match.set(parse_custom_rule_match(&value));
                                    editor_error.set(None);
                                },
                            }
                        }
                        if include_process_rules
                            && (draft_match() == CustomRuleMatch::ProcessName
                                || draft_match() == CustomRuleMatch::ProcessPath)
                        {
                            div { class: "rule-field",
                                span { "匹配内容" }
                                div { class: "rule-value-row",
                                    input {
                                        value: draft_value,
                                        placeholder,
                                        spellcheck: "false",
                                        oninput: move |event| {
                                            draft_value.set(event.value());
                                            editor_error.set(None);
                                        },
                                        onkeydown: move |event| {
                                            if event.key() == Key::Enter {
                                                match save_custom_rule(
                                                    &mut rules.write(),
                                                    editing_id(),
                                                    draft_match(),
                                                    draft_action(),
                                                    &draft_value(),
                                                ) {
                                                    Ok(()) => {
                                                        editor_open.set(false);
                                                        notify_rule_change(connected(), toast);
                                                    }
                                                    Err(error) => editor_error.set(Some(error)),
                                                }
                                            }
                                        },
                                    }
                                    button {
                                        r#type: "button",
                                        class: "secondary-button compact process-pick-button",
                                        onclick: move |_| {
                                            process_picker_open.set(true);
                                            process_picker_loaded.set(false);
                                            process_picker_error.set(None);
                                            load_running_processes(
                                                process_picker_list,
                                                process_picker_loaded,
                                                process_picker_error,
                                            );
                                        },
                                        Icon { icon: LdListFilter, width: 15, height: 15 }
                                        "选择进程"
                                    }
                                }
                            }
                        } else {
                        label { class: "rule-field",
                            span { "匹配内容" }
                            input {
                                value: draft_value,
                                placeholder,
                                spellcheck: "false",
                                oninput: move |event| {
                                    draft_value.set(event.value());
                                    editor_error.set(None);
                                },
                                onkeydown: move |event| {
                                    if event.key() == Key::Enter {
                                        match save_custom_rule(
                                            &mut rules.write(),
                                            editing_id(),
                                            draft_match(),
                                            draft_action(),
                                            &draft_value(),
                                        ) {
                                            Ok(()) => {
                                                editor_open.set(false);
                                                notify_rule_change(connected(), toast);
                                            }
                                            Err(error) => editor_error.set(Some(error)),
                                        }
                                    }
                                },
                            }
                        }
                        }
                    }
                    if let Some(error) = editor_error() {
                        div { class: "field-error",
                            Icon { icon: LdCircleAlert, width: 15, height: 15 }
                            span { "{error}" }
                        }
                    }
                    div { class: "rule-editor-actions",
                        button {
                            class: "secondary-button",
                            onclick: move |_| {
                                editor_open.set(false);
                                editor_error.set(None);
                            },
                            "取消"
                        }
                        button {
                            class: "primary-button",
                            onclick: move |_| {
                                match save_custom_rule(
                                    &mut rules.write(),
                                    editing_id(),
                                    draft_match(),
                                    draft_action(),
                                    &draft_value(),
                                ) {
                                    Ok(()) => {
                                        editor_open.set(false);
                                        notify_rule_change(connected(), toast);
                                    }
                                    Err(error) => editor_error.set(Some(error)),
                                }
                            },
                            Icon { icon: LdSave, width: 16, height: 16 }
                            if editing_id().is_some() { "保存" } else { "添加" }
                        }
                    }
                }
            }

            if process_picker_open() {
                ProcessPicker {
                    list: process_picker_list,
                    loaded: process_picker_loaded,
                    error: process_picker_error,
                    on_close: move |_| process_picker_open.set(false),
                    on_pick: move |process: api::RunningProcess| {
                        let value = if draft_match() == CustomRuleMatch::ProcessPath {
                            process.exe_path.clone().unwrap_or_else(|| process.name.clone())
                        } else {
                            process.name.clone()
                        };
                        draft_value.set(value);
                        editor_error.set(None);
                        process_picker_open.set(false);
                    },
                }
            }

            if stored.is_empty() {
                div { class: "large-empty rules-empty",
                    span { class: "empty-icon", Icon { icon: LdListFilter, width: 28, height: 28 } }
                    strong { "暂无自定义规则" }
                }
            } else {
                div { class: "rule-list",
                    div { class: "rule-table-header",
                        span { "序号" }
                        span { "动作" }
                        span { "匹配内容 / 类型" }
                        span { "启用" }
                        span { "操作" }
                    }
                    for (index, rule) in stored.into_iter().enumerate() {
                        RuleListItem {
                            key: "{rule.id}",
                            rule,
                            index,
                            total,
                            rules,
                            connected: connected(),
                            locked: override_active,
                            include_process_rules,
                            on_edit,
                        }
                    }
                }
            }
            }
        }
    }
}

fn load_running_processes(
    mut list: Signal<Vec<api::RunningProcess>>,
    mut loaded: Signal<bool>,
    mut error: Signal<Option<String>>,
) {
    spawn(async move {
        match api::list_running_processes().await {
            Ok(processes) => {
                list.set(processes);
                loaded.set(true);
            }
            Err(load_error) => {
                error.set(Some(load_error.to_string()));
            }
        }
    });
}

#[component]
fn ProcessPicker(
    list: Signal<Vec<api::RunningProcess>>,
    loaded: Signal<bool>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    mut on_pick: EventHandler<api::RunningProcess>,
) -> Element {
    let mut query = use_signal(String::new);
    let filtered = list
        .iter()
        .filter(|process| {
            let needle = query().to_lowercase();
            needle.is_empty()
                || process.name.to_lowercase().contains(&needle)
                || process
                    .exe_path
                    .as_deref()
                    .map(|path| path.to_lowercase().contains(&needle))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    rsx! {
        div { class: "process-picker-backdrop", aria_hidden: "true", onclick: move |_| on_close.call(()) }
        div { class: "process-picker", role: "dialog", aria_label: "选择进程",
            div { class: "process-picker-header",
                h3 { "选择进程" }
                button {
                    r#type: "button",
                    class: "icon-button",
                    aria_label: "关闭",
                    onclick: move |_| on_close.call(()),
                    Icon { icon: LdX, width: 17, height: 17 }
                }
            }
            div { class: "process-picker-search",
                input {
                    value: query,
                    placeholder: "搜索进程名或路径…",
                    spellcheck: "false",
                    oninput: move |event| query.set(event.value()),
                }
            }
            if let Some(message) = error() {
                div { class: "process-picker-status error", "{message}" }
            } else if !loaded() {
                div { class: "process-picker-status", span { class: "spinner" } "正在读取进程列表…" }
            } else if filtered.is_empty() {
                div { class: "process-picker-status", "没有匹配的进程" }
            } else {
                div { class: "process-picker-list", role: "listbox",
                    for process in filtered {
                        {
                            let name = process.name.clone();
                            let path = process.exe_path.clone().unwrap_or_default();
                            let picked = process.clone();
                            rsx! {
                                button {
                                    key: "{process.pid}",
                                    r#type: "button",
                                    class: "process-picker-item",
                                    role: "option",
                                    onclick: move |_| on_pick.call(picked.clone()),
                                    div { class: "process-picker-item-main",
                                        span { class: "process-picker-item-name", "{name}" }
                                        if !path.is_empty() {
                                            span { class: "process-picker-item-path", "{path}" }
                                        }
                                    }
                                    span { class: "process-picker-item-pid", "PID {process.pid}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RuleSelect(
    label: String,
    value: String,
    options: Vec<RuleSelectOption>,
    on_select: EventHandler<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let selected_label = options
        .iter()
        .find(|option| option.value == value)
        .map(|option| option.label.clone())
        .unwrap_or_else(|| value.clone());
    let trigger_label = label.clone();
    let menu_label = label;

    rsx! {
        div { class: if open() { "rule-select-control open" } else { "rule-select-control" },
            button {
                r#type: "button",
                class: "rule-select-trigger",
                aria_label: trigger_label,
                aria_haspopup: "listbox",
                aria_expanded: if open() { "true" } else { "false" },
                onclick: move |event| {
                    event.stop_propagation();
                    open.set(!open());
                },
                onkeydown: move |event| {
                    match event.key() {
                        Key::ArrowDown | Key::ArrowUp => {
                            event.prevent_default();
                            open.set(true);
                        }
                        Key::Escape => {
                            event.prevent_default();
                            open.set(false);
                        }
                        _ => {}
                    }
                },
                span { "{selected_label}" }
                Icon { icon: LdChevronDown, width: 16, height: 16 }
            }
            if open() {
                div {
                    class: "rule-select-backdrop",
                    aria_hidden: "true",
                    onclick: move |_| open.set(false),
                }
                div { class: "rule-select-menu", role: "listbox", aria_label: menu_label,
                    for option in options {
                        {
                            let option_value = option.value.clone();
                            let selected = option.value == value;
                            rsx! {
                                button {
                                    key: "{option.value}",
                                    r#type: "button",
                                    role: "option",
                                    aria_selected: if selected { "true" } else { "false" },
                                    class: if selected { "rule-select-option selected" } else { "rule-select-option" },
                                    onclick: move |event| {
                                        event.stop_propagation();
                                        on_select.call(option_value.clone());
                                        open.set(false);
                                    },
                                    span { "{option.label}" }
                                    if selected {
                                        Icon { icon: LdCheck, width: 15, height: 15 }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RuleListItem(
    rule: CustomRule,
    index: usize,
    total: usize,
    mut rules: Signal<Vec<CustomRule>>,
    connected: bool,
    locked: bool,
    include_process_rules: bool,
    on_edit: EventHandler<CustomRule>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let rule_id = rule.id;
    let edit_rule = rule.clone();
    let enabled = rule.enabled;
    let action = rule.action;
    let match_type = rule.match_type;
    let priority = index + 1;

    rsx! {
        div {
            class: if locked {
                "rule-row locked"
            } else if enabled {
                "rule-row"
            } else {
                "rule-row disabled"
            },
            span { class: "rule-priority", "{priority:02}" }
            span { class: custom_rule_action_class(action),
                match action {
                    CustomRuleAction::Direct => rsx! { Icon { icon: LdRoute, width: 15, height: 15 } },
                    CustomRuleAction::Proxy => rsx! { Icon { icon: LdShieldCheck, width: 15, height: 15 } },
                    CustomRuleAction::Block => rsx! { Icon { icon: LdBan, width: 15, height: 15 } },
                }
                {custom_rule_action_label(action)}
            }
            div { class: "rule-main",
                strong { title: rule.value.clone(), "{rule.value}" }
                small { "{custom_rule_match_label(match_type)}" }
            }
            label { class: "rule-enabled-toggle", title: if enabled { "停用规则" } else { "启用规则" },
                input {
                    r#type: "checkbox",
                    checked: enabled,
                    disabled: locked,
                    onchange: move |event| {
                        if let Some(stored) = rules.write().iter_mut().find(|item| item.id == rule_id) {
                            stored.enabled = event.checked();
                            notify_rule_change(connected, toast);
                        }
                    },
                }
                span { class: "switch" }
            }
            div { class: "rule-row-actions",
                button {
                    class: "icon-button",
                    title: "上移",
                    disabled: locked || index == 0,
                    onclick: move |_| {
                        move_custom_rule(
                            &mut rules.write(),
                            rule_id,
                            -1,
                            include_process_rules,
                        );
                        notify_rule_change(connected, toast);
                    },
                    Icon { icon: LdArrowUp, width: 16, height: 16 }
                }
                button {
                    class: "icon-button",
                    title: "下移",
                    disabled: locked || index + 1 >= total,
                    onclick: move |_| {
                        move_custom_rule(
                            &mut rules.write(),
                            rule_id,
                            1,
                            include_process_rules,
                        );
                        notify_rule_change(connected, toast);
                    },
                    Icon { icon: LdArrowDown, width: 16, height: 16 }
                }
                button {
                    class: "icon-button",
                    title: "编辑规则",
                    disabled: locked,
                    onclick: move |_| on_edit.call(edit_rule.clone()),
                    Icon { icon: LdPencil, width: 16, height: 16 }
                }
                button {
                    class: "icon-button danger",
                    title: "删除规则",
                    disabled: locked,
                    onclick: move |_| {
                        rules.write().retain(|item| item.id != rule_id);
                        notify_rule_change(connected, toast);
                    },
                    Icon { icon: LdTrash2, width: 16, height: 16 }
                }
            }
        }
    }
}

fn save_custom_rule(
    rules: &mut Vec<CustomRule>,
    editing_id: Option<u64>,
    match_type: CustomRuleMatch,
    action: CustomRuleAction,
    value: &str,
) -> Result<(), String> {
    if editing_id.is_none() && rules.len() >= MAX_CUSTOM_RULES {
        return Err(format!("最多添加 {MAX_CUSTOM_RULES} 条规则"));
    }
    let id = editing_id
        .or_else(|| next_custom_rule_id(rules))
        .ok_or_else(|| "无法生成规则 ID".to_string())?;
    let enabled = rules
        .iter()
        .find(|rule| rule.id == id)
        .map(|rule| rule.enabled)
        .unwrap_or(true);
    let candidate = CustomRule {
        id,
        enabled,
        match_type,
        value: value.to_string(),
        action,
    }
    .normalized()
    .map_err(|error| error.to_string())?;

    let mut updated = rules.clone();
    if let Some(existing) = updated.iter_mut().find(|rule| rule.id == id) {
        *existing = candidate;
    } else if editing_id.is_some() {
        return Err("要编辑的规则不存在".to_string());
    } else {
        updated.push(candidate);
    }
    validate_custom_rules(&updated).map_err(|error| error.to_string())?;
    *rules = updated;
    Ok(())
}

fn connection_custom_rules(
    config_script_enabled: bool,
    config_script: &str,
    custom_rules: Vec<CustomRule>,
) -> Vec<CustomRule> {
    connection_custom_rules_for_platform(
        config_script_enabled,
        config_script,
        custom_rules,
        process_rules_available(),
    )
}

fn connection_custom_rules_for_platform(
    config_script_enabled: bool,
    config_script: &str,
    custom_rules: Vec<CustomRule>,
    include_process_rules: bool,
) -> Vec<CustomRule> {
    if config_script_enabled && !config_script.trim().is_empty() {
        Vec::new()
    } else {
        custom_rules
            .into_iter()
            .filter(|rule| custom_rule_visible(rule.match_type, include_process_rules))
            .collect()
    }
}

fn next_custom_rule_id(rules: &[CustomRule]) -> Option<u64> {
    (1..=(rules.len() as u64 + 1)).find(|id| rules.iter().all(|rule| rule.id != *id))
}

fn move_custom_rule(rules: &mut [CustomRule], id: u64, offset: isize, include_process_rules: bool) {
    let visible_indices = rules
        .iter()
        .enumerate()
        .filter_map(|(index, rule)| {
            custom_rule_visible(rule.match_type, include_process_rules).then_some(index)
        })
        .collect::<Vec<_>>();
    let Some(visible_index) = visible_indices
        .iter()
        .position(|index| rules[*index].id == id)
    else {
        return;
    };
    let Some(target) = visible_index.checked_add_signed(offset) else {
        return;
    };
    if let Some(target_index) = visible_indices.get(target) {
        rules.swap(visible_indices[visible_index], *target_index);
    }
}

fn notify_rule_change(connected: bool, toast: ToastManager) {
    toast.success(if connected {
        "规则已保存，重新连接后生效"
    } else {
        "规则已保存"
    });
}

fn custom_rule_action_value(action: CustomRuleAction) -> &'static str {
    match action {
        CustomRuleAction::Direct => "direct",
        CustomRuleAction::Proxy => "proxy",
        CustomRuleAction::Block => "block",
    }
}

fn parse_custom_rule_action(value: &str) -> CustomRuleAction {
    match value {
        "proxy" => CustomRuleAction::Proxy,
        "block" => CustomRuleAction::Block,
        _ => CustomRuleAction::Direct,
    }
}

fn custom_rule_match_value(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "domain",
        CustomRuleMatch::DomainSuffix => "domain_suffix",
        CustomRuleMatch::DomainKeyword => "domain_keyword",
        CustomRuleMatch::IpCidr => "ip_cidr",
        CustomRuleMatch::ProcessName => "process_name",
        CustomRuleMatch::ProcessPath => "process_path",
    }
}

const fn process_rules_available() -> bool {
    !cfg!(any(target_os = "android", target_os = "ios"))
}

fn custom_rule_match_options(include_process_rules: bool) -> Vec<RuleSelectOption> {
    let mut options = vec![
        RuleSelectOption::new("domain", "精确域名"),
        RuleSelectOption::new("domain_suffix", "域名后缀"),
        RuleSelectOption::new("domain_keyword", "域名关键字"),
        RuleSelectOption::new("ip_cidr", "IP CIDR"),
    ];
    if include_process_rules {
        options.push(RuleSelectOption::new("process_name", "进程名"));
        options.push(RuleSelectOption::new("process_path", "进程路径"));
    }
    options
}

fn custom_rule_visible(match_type: CustomRuleMatch, include_process_rules: bool) -> bool {
    include_process_rules
        || !matches!(
            match_type,
            CustomRuleMatch::ProcessName | CustomRuleMatch::ProcessPath
        )
}

fn parse_custom_rule_match(value: &str) -> CustomRuleMatch {
    match value {
        "domain" => CustomRuleMatch::Domain,
        "domain_keyword" => CustomRuleMatch::DomainKeyword,
        "ip_cidr" => CustomRuleMatch::IpCidr,
        "process_name" => CustomRuleMatch::ProcessName,
        "process_path" => CustomRuleMatch::ProcessPath,
        _ => CustomRuleMatch::DomainSuffix,
    }
}

fn custom_rule_match_label(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "精确域名",
        CustomRuleMatch::DomainSuffix => "域名后缀",
        CustomRuleMatch::DomainKeyword => "域名关键字",
        CustomRuleMatch::IpCidr => "IP CIDR",
        CustomRuleMatch::ProcessName => "进程名",
        CustomRuleMatch::ProcessPath => "进程路径",
    }
}

fn custom_rule_placeholder(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "api.example.com",
        CustomRuleMatch::DomainSuffix => "example.com",
        CustomRuleMatch::DomainKeyword => "google",
        CustomRuleMatch::IpCidr => "203.0.113.0/24",
        CustomRuleMatch::ProcessName => "Telegram",
        CustomRuleMatch::ProcessPath => "/Applications/Telegram.app/Contents/MacOS/Telegram",
    }
}

fn custom_rule_action_label(action: CustomRuleAction) -> &'static str {
    match action {
        CustomRuleAction::Direct => "直连",
        CustomRuleAction::Proxy => "代理",
        CustomRuleAction::Block => "拦截",
    }
}

fn custom_rule_action_class(action: CustomRuleAction) -> &'static str {
    match action {
        CustomRuleAction::Direct => "rule-action direct",
        CustomRuleAction::Proxy => "rule-action proxy",
        CustomRuleAction::Block => "rule-action block",
    }
}

#[component]
fn LogsView(
    mut logs: Signal<Vec<CoreLogEntry>>,
    connected: Signal<bool>,
    mut collection_paused: Signal<bool>,
    core_version: Signal<Option<String>>,
    mut filter: Signal<LogFilter>,
    mut search: Signal<String>,
    on_start: EventHandler<()>,
) -> Element {
    let stored = logs();
    let route_count = stored.iter().filter(|entry| entry.route.is_some()).count();
    let direct_count = stored
        .iter()
        .filter(|entry| {
            entry.route.as_ref().map(|route| route.decision) == Some(RouteDecision::Direct)
        })
        .count();
    let proxy_count = stored
        .iter()
        .filter(|entry| {
            entry.route.as_ref().map(|route| route.decision) == Some(RouteDecision::Proxy)
        })
        .count();
    let block_count = stored
        .iter()
        .filter(|entry| {
            entry.route.as_ref().map(|route| route.decision) == Some(RouteDecision::Block)
        })
        .count();
    let raw_count = stored.len() - route_count;
    let query = search().trim().to_ascii_lowercase();
    let visible_logs = visible_log_entries(&stored, filter(), &query);
    let visible_count = visible_logs.len();
    // The collector only runs while the core is up and this page is open, so the
    // chip has three real states instead of a fake pause button.
    let collector_class = if !connected() {
        "log-collection-toggle stopped"
    } else if collection_paused() {
        "log-collection-toggle paused"
    } else {
        "log-collection-toggle collecting"
    };
    let collector_label = if !connected() {
        "已停止"
    } else if collection_paused() {
        "已暂停"
    } else {
        "采集中"
    };

    rsx! {
        // Count strip doubles as the filter: each chip is one decision tally,
        // the active one carries the outline, exactly like the design.
        div { class: "log-stats", aria_label: "日志统计", role: "status",
            for (label, value, tone, chip_filter) in [
                ("全部路由", route_count, "total", LogFilter::Routes),
                ("代理", proxy_count, "proxy", LogFilter::Proxy),
                ("直连", direct_count, "direct", LogFilter::Direct),
                ("拦截", block_count, "block", LogFilter::Block),
                ("内核日志", raw_count, "raw", LogFilter::Raw),
            ] {
                button {
                    class: if filter() == chip_filter { format!("log-stat active {tone}") } else { format!("log-stat {tone}") },
                    key: "{label}",
                    aria_pressed: filter() == chip_filter,
                    onclick: move |_| filter.set(chip_filter),
                    span { class: "log-stat-bar {tone}", aria_hidden: "true" }
                    div {
                        span { class: "log-stat-label", "{label}" }
                        strong { class: "log-stat-value", "{value}" }
                    }
                }
            }
        }
        div { class: "log-controls",
            label { class: "search-field log-search",
                Icon { icon: LdSearch, width: 17, height: 17 }
                input {
                    value: search,
                    placeholder: "搜索域名、IP、节点",
                    oninput: move |event| search.set(event.value()),
                }
            }
            div { class: "log-toolbar-actions",
                button {
                    class: collector_class,
                    title: if connected() {
                        if collection_paused() { "恢复采集内核日志" } else { "暂停采集内核日志" }
                    } else {
                        "内核未运行，日志采集已停止"
                    },
                    disabled: !connected(),
                    onclick: move |_| collection_paused.toggle(),
                    if connected() && collection_paused() {
                        Icon { icon: LdPlay, width: 14, height: 14 }
                    } else if connected() {
                        Icon { icon: LdPause, width: 14, height: 14 }
                    } else {
                        span { class: "status-badge-dot" }
                    }
                    span { class: "log-collection-label", "{collector_label}" }
                }
                span { class: "log-controls-hint", "缓冲区 {stored.len()} 条记录" }
            }
        }
        section { class: "workspace-section log-workspace glass-surface",
            if visible_logs.is_empty() {
                div { class: "large-empty log-empty",
                    span { class: "empty-icon", Icon { icon: LdScrollText, width: 27, height: 27 } }
                    strong {
                        if connected() { "暂无匹配日志" } else { "连接后显示日志" }
                    }
                    p {
                        "启动代理后，每条连接的目标域名、命中链路与动作都会实时写入。当前 {route_count} 条路由 · {stored.len()} 条记录。"
                    }
                    if !connected() {
                        button {
                            class: "primary-button compact log-start-button",
                            onclick: move |_| on_start.call(()),
                            Icon { icon: LdPlay, width: 16, height: 16 }
                            span { "启动代理开始采集" }
                        }
                    }
                    small { class: "log-empty-hint", "{core_build_hint(core_version(), connected())}" }
                }
            } else {
                div { class: "log-list", aria_label: "内核日志",
                    div { class: "log-table-header",
                        span { "时间" }
                        span { "动作" }
                        span { "目标" }
                        span { class: "log-col-chain", "命中链路" }
                        span { "出口节点" }
                    }
                    for entry in visible_logs {
                        LogRow { entry }
                    }
                }
                div { class: "log-list-footer", "当前显示 {visible_count} 条" }
            }
        }
    }
}

/// Flatten the visible log entries into CSV cells, oldest first. Only fields the
/// core really reports are exported: the design's per-connection byte counts and
/// timings are not part of the log model, so they stay out of the file.
fn log_export_rows(entries: &[CoreLogEntry]) -> Vec<Vec<String>> {
    let mut rows = vec![vec![
        "时间".to_string(),
        "级别".to_string(),
        "动作".to_string(),
        "目标".to_string(),
        "类型".to_string(),
        "端口".to_string(),
        "出口类型".to_string(),
        "出口节点".to_string(),
        "分组链路".to_string(),
        "来源 IP".to_string(),
        "原始消息".to_string(),
    ]];
    for entry in entries.iter().rev() {
        match entry.route.as_ref() {
            Some(route) => rows.push(vec![
                entry.timestamp.clone(),
                entry.level.clone(),
                route_decision_label(route.decision).to_string(),
                route.host.clone(),
                route_target_kind_label(route.target_kind).to_string(),
                route
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                route.outbound_type.clone(),
                route.outbound_tag.clone(),
                route_group_chain(route),
                route.source_ip.clone().unwrap_or_else(|| "-".to_string()),
                entry.message.clone(),
            ]),
            None => rows.push(vec![
                entry.timestamp.clone(),
                entry.level.clone(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                entry.message.clone(),
            ]),
        }
    }
    rows
}

/// Script that turns the received rows into a CSV blob and clicks a temporary
/// download anchor. The BOM keeps Excel from mangling the Chinese headers.
fn download_csv_script(file_name: &str) -> String {
    r#"
const rows = await dioxus.recv();
const csv = rows
    .map((row) => row.map((cell) => `"${String(cell).replace(/"/g, '""')}"`).join(','))
    .join('\r\n');
const blob = new Blob([`\ufeff${csv}`], { type: 'text/csv;charset=utf-8' });
const url = URL.createObjectURL(blob);
const anchor = document.createElement('a');
anchor.href = url;
anchor.download = __FILE_NAME__;
document.body.appendChild(anchor);
anchor.click();
anchor.remove();
setTimeout(() => URL.revokeObjectURL(url), 1_000);
await dioxus.send(true);
"#
    .replace("__FILE_NAME__", &format!("{file_name:?}"))
}

async fn download_csv(file_name: &str, rows: Vec<Vec<String>>) -> Result<(), String> {
    let eval = document::eval(&download_csv_script(file_name));
    eval.send(rows).map_err(|error| error.to_string())?;
    eval.await.map(|_| ()).map_err(|error| error.to_string())
}

/// Subtitle under the Kitty Pro wordmark: which sing-box build is embedded.
fn about_build_label(core_version: Option<String>, platform: &str) -> String {
    match core_version {
        Some(version) => format!("sing-box {version} · {platform}"),
        None => format!("sing-box 未就绪 · {platform}"),
    }
}

/// Plain-text diagnostic blob the user can paste into a bug report. Nothing in
/// here is invented: every line mirrors a state the UI already shows.
#[allow(clippy::too_many_arguments)]
fn build_diagnostic_report(
    core_state: String,
    core_version: Option<String>,
    platform: &str,
    tunnel_mode: TunnelMode,
    tun_enabled: bool,
    allow_lan: bool,
    node_count: usize,
    subscription_count: usize,
    rule_count: usize,
    dns_count: usize,
) -> String {
    format!(
        "Kitty Pro 诊断信息\n\
         界面平台: {platform}\n\
         内核状态: {state_label} ({core_state})\n\
         内核版本: {core_version}\n\
         分流模式: {mode}\n\
         TUN: {tun} · 局域网: {lan} ({endpoint})\n\
         节点: {node_count} · 订阅: {subscription_count}\n\
         自定义规则: {rule_count} · 专用 DNS: {dns_count}",
        state_label = core_state_label(&core_state),
        core_version = core_version.unwrap_or_else(|| "未检测到版本".to_string()),
        mode = mode_label(tunnel_mode),
        tun = if tun_enabled { "开启" } else { "关闭" },
        lan = if allow_lan { "开启" } else { "关闭" },
        endpoint = proxy_port_label(allow_lan),
    )
}

/// Script that writes the received string to the clipboard. Falls back to a
/// hidden textarea + execCommand so it still works on non-secure origins.
fn copy_to_clipboard_script() -> String {
    r#"
const text = await dioxus.recv();
try {
    await navigator.clipboard.writeText(text);
} catch (_) {
    const area = document.createElement('textarea');
    area.value = text;
    area.setAttribute('readonly', '');
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.appendChild(area);
    area.select();
    document.execCommand('copy');
    area.remove();
}
await dioxus.send(true);
"#
    .to_string()
}

async fn copy_to_clipboard(text: String) -> Result<(), String> {
    let eval = document::eval(&copy_to_clipboard_script());
    eval.send(text).map_err(|error| error.to_string())?;
    eval.await.map(|_| ()).map_err(|error| error.to_string())
}

/// Footer hint of the log empty state: which core is idle, and whether the
/// collector is waiting for a connection.
fn core_build_hint(core_version: Option<String>, connected: bool) -> String {
    let build = core_version.unwrap_or_else(|| "sing-box".to_string());
    if connected {
        format!("内核 {build} · 日志采集中")
    } else {
        format!("内核 {build} · 日志采集待启动")
    }
}

#[component]
fn LogRow(entry: CoreLogEntry) -> Element {
    let level_class = format!("log-level {}", entry.level);
    let level_label = entry.level.to_ascii_uppercase();
    let route = entry.route.clone();
    let source_ip = route.as_ref().and_then(|route| route.source_ip.clone());
    let has_route = route.is_some();

    rsx! {
        div { class: if has_route { "log-row route-entry" } else { "log-row raw-entry" },
            div { class: "log-origin",
                time { class: "log-time", "{entry.timestamp}" }
                if let Some(source_ip) = source_ip {
                    small { class: "log-source", title: source_ip.clone(), "{source_ip}" }
                }
            }
            if let Some(route) = route {
                div { class: "route-decision-cell",
                    span {
                        class: match route.decision {
                            RouteDecision::Direct => "route-decision direct",
                            RouteDecision::Proxy => "route-decision proxy",
                            RouteDecision::Block => "route-decision block",
                        },
                        {route_decision_label(route.decision)}
                    }
                }
                div { class: "log-target", title: route.target.clone(),
                    strong { "{route.host}" }
                    small {
                        {route_target_kind_label(route.target_kind)}
                        if let Some(port) = route.port {
                            " · {port}"
                        }
                    }
                }
                // The core only reports the outbound chain, never a rule name, so
                // this column lists the real group hops instead of guessing a rule.
                div { class: "log-chain",
                    {route_chain_tags(&route)}
                }
                div { class: "log-outbound", title: route.outbound_tag.clone(),
                    strong { "{route.outbound_tag}" }
                    small { "{route.outbound_type}" }
                }
                code { class: "log-message", "{entry.message}" }
            } else {
                span { class: level_class, "{level_label}" }
                code { class: "log-message raw-message", "{entry.message}" }
            }
        }
    }
}

fn log_matches_filter(entry: &CoreLogEntry, filter: LogFilter) -> bool {
    match filter {
        LogFilter::Routes => entry.route.is_some(),
        LogFilter::Raw => entry.route.is_none(),
        LogFilter::Direct => {
            entry.route.as_ref().map(|route| route.decision) == Some(RouteDecision::Direct)
        }
        LogFilter::Proxy => {
            entry.route.as_ref().map(|route| route.decision) == Some(RouteDecision::Proxy)
        }
        LogFilter::Block => {
            entry.route.as_ref().map(|route| route.decision) == Some(RouteDecision::Block)
        }
    }
}

/// Newest first, after the chip filter and the free-text query. Shared with the
/// topbar export so the CSV always matches what the page shows.
fn visible_log_entries(
    stored: &[CoreLogEntry],
    filter: LogFilter,
    query: &str,
) -> Vec<CoreLogEntry> {
    stored
        .iter()
        .rev()
        .filter(|entry| log_matches_filter(entry, filter))
        .filter(|entry| log_matches_search(entry, query))
        .cloned()
        .collect()
}

fn log_matches_search(entry: &CoreLogEntry, query: &str) -> bool {
    if query.is_empty() || entry.message.to_ascii_lowercase().contains(query) {
        return true;
    }
    entry.route.as_ref().is_some_and(|route| {
        route.host.to_ascii_lowercase().contains(query)
            || route.outbound_tag.to_ascii_lowercase().contains(query)
            || route.outbound_type.to_ascii_lowercase().contains(query)
            || route
                .source_ip
                .as_ref()
                .is_some_and(|source_ip| source_ip.to_ascii_lowercase().contains(query))
            || route
                .outbound_chain
                .iter()
                .any(|tag| tag.to_ascii_lowercase().contains(query))
    })
}

/// Group hops the core really reported for this connection, rendered as tags.
/// sing-box logs no rule name, so the chain is the closest honest answer to the
/// design's "匹配规则" column; a direct hop without a chain renders as "-".
fn route_chain_tags(route: &RouteLogDetail) -> Element {
    let hops = route
        .outbound_chain
        .iter()
        .filter(|tag| tag.as_str() != route.outbound_tag)
        .cloned()
        .collect::<Vec<_>>();
    if hops.is_empty() {
        return rsx! { span { class: "log-chain-empty", "-" } };
    }
    rsx! {
        for hop in hops {
            span { class: "log-chain-tag", key: "{hop}", title: "{hop}", "{hop}" }
        }
    }
}

fn route_group_chain(route: &RouteLogDetail) -> String {
    route
        .outbound_chain
        .iter()
        .filter(|tag| tag.as_str() != route.outbound_tag)
        .cloned()
        .collect::<Vec<_>>()
        .join(" → ")
}

fn route_decision_label(decision: RouteDecision) -> &'static str {
    match decision {
        RouteDecision::Direct => "直连",
        RouteDecision::Proxy => "代理",
        RouteDecision::Block => "拦截",
    }
}

fn route_target_kind_label(kind: RouteTargetKind) -> &'static str {
    match kind {
        RouteTargetKind::Domain => "域名",
        RouteTargetKind::Ip => "IP",
    }
}

#[component]
fn SyncSettings(
    mut subscriptions: Signal<Vec<Subscription>>,
    mut active_subscription_id: Signal<Option<u64>>,
    mut nodes: Signal<Vec<ProxyNode>>,
    mut selected_tag: Signal<String>,
    mut custom_rules: Signal<Vec<CustomRule>>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    mut group_selections: Signal<HashMap<String, String>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    dark_mode: Signal<bool>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let mut config = use_signal(SyncConfig::default);
    let mut loaded = use_signal(|| false);
    let mut busy = use_signal(|| None::<SyncAction>);
    let mut confirm = use_signal(|| None::<SyncAction>);

    use_effect(move || {
        spawn(async move {
            match api::load_sync_config().await {
                Ok(saved) => config.set(saved),
                Err(error) => toast.error(format!("同步配置读取失败: {error}")),
            }
            loaded.set(true);
        });
    });

    let current = config();
    let provider = current.provider;
    let sync_enabled = current.enabled;
    let sync_busy = busy().is_some();
    let last_sync_at = current.last_sync_at;

    rsx! {
        section { class: "settings-section sync-settings glass-surface",
            div { class: "section-heading",
                div {
                    p { class: "eyebrow", "REMOTE SYNC" }
                    h2 { "规则与订阅同步" }
                }
                span {
                    class: if last_sync_at > 0 { "status-badge ready" } else if sync_enabled { "status-badge pending" } else { "status-badge" },
                    if !loaded() { "读取中" } else if last_sync_at > 0 { "已同步" } else if sync_enabled { "待同步" } else { "未启用" }
                }
            }

            div { class: "sync-provider-row",
                div { class: "segmented-control sync-provider-control", aria_label: "同步存储类型",
                    button {
                        class: if provider == SyncProviderKind::WebDav { "active" } else { "" },
                        disabled: sync_busy,
                        onclick: move |_| config.write().provider = SyncProviderKind::WebDav,
                        "WebDAV"
                    }
                    button {
                        class: if provider == SyncProviderKind::S3 { "active" } else { "" },
                        disabled: sync_busy,
                        onclick: move |_| config.write().provider = SyncProviderKind::S3,
                        "S3 兼容"
                    }
                }
                label { class: "setting-row toggle-row sync-enable-row",
                    span { class: "setting-icon", Icon { icon: LdCloud, width: 19, height: 19 } }
                    div {
                        strong { "远端同步" }
                        small { if sync_enabled { "已启用" } else { "已停用" } }
                    }
                    input {
                        r#type: "checkbox",
                        checked: sync_enabled,
                        disabled: sync_busy,
                        onchange: move |event| config.write().enabled = event.checked(),
                    }
                    span { class: "switch" }
                }
            }

            div { class: "inline-note sync-security-note",
                Icon { icon: LdInfo, width: 16, height: 16 }
                span { "远端文件包含订阅来源和自定义规则" }
            }

            div { class: "sync-form-grid",
                // Each provider owns its own address field so switching tabs can
                // never show or save an address typed for the other provider.
                if provider == SyncProviderKind::WebDav {
                    label { class: "sync-field sync-endpoint-field",
                        span { "WebDAV 地址" }
                        input {
                            value: current.webdav_endpoint,
                            disabled: sync_busy,
                            placeholder: "https://dav.example.com/dav/files/user",
                            spellcheck: "false",
                            oninput: move |event| config.write().webdav_endpoint = event.value(),
                        }
                    }
                } else {
                    label { class: "sync-field sync-endpoint-field",
                        span { "S3 Endpoint" }
                        input {
                            value: current.s3_endpoint,
                            disabled: sync_busy,
                            placeholder: "留空使用 AWS，或填写 https://s3.example.com",
                            spellcheck: "false",
                            oninput: move |event| config.write().s3_endpoint = event.value(),
                        }
                    }
                }
                label { class: "sync-field",
                    span { "远程文件" }
                    input {
                        value: current.path,
                        disabled: sync_busy,
                        placeholder: "kitty-pro-sync.json",
                        spellcheck: "false",
                        oninput: move |event| config.write().path = event.value(),
                    }
                }

                if provider == SyncProviderKind::WebDav {
                    label { class: "sync-field",
                        span { "用户名" }
                        input {
                            value: current.username,
                            disabled: sync_busy,
                            autocomplete: "username",
                            oninput: move |event| config.write().username = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "密码 / 应用密码" }
                        input {
                            r#type: "password",
                            value: current.password,
                            disabled: sync_busy,
                            autocomplete: "current-password",
                            oninput: move |event| config.write().password = event.value(),
                        }
                    }
                } else {
                    label { class: "sync-field",
                        span { "Bucket" }
                        input {
                            value: current.bucket,
                            disabled: sync_busy,
                            spellcheck: "false",
                            oninput: move |event| config.write().bucket = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "Region" }
                        input {
                            value: current.region,
                            disabled: sync_busy,
                            placeholder: "us-east-1",
                            spellcheck: "false",
                            oninput: move |event| config.write().region = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "Access Key" }
                        input {
                            value: current.access_key,
                            disabled: sync_busy,
                            autocomplete: "off",
                            spellcheck: "false",
                            oninput: move |event| config.write().access_key = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "Secret Key" }
                        input {
                            r#type: "password",
                            value: current.secret_key,
                            disabled: sync_busy,
                            autocomplete: "off",
                            oninput: move |event| config.write().secret_key = event.value(),
                        }
                    }
                }
            }

            div { class: "sync-actions",
                button {
                    class: "secondary-button compact",
                    disabled: !loaded() || sync_busy,
                    onclick: move |_| {
                        confirm.set(None);
                        let value = config();
                        async move {
                            busy.set(Some(SyncAction::Save));
                            match api::save_sync_config(value).await {
                                Ok(saved) => {
                                    config.set(saved);
                                    toast.success("同步配置已保存");
                                }
                                Err(error) => toast.error(format!("同步配置保存失败: {error}")),
                            }
                            busy.set(None);
                        }
                    },
                    if busy() == Some(SyncAction::Save) { span { class: "spinner" } }
                    else { Icon { icon: LdSave, width: 16, height: 16 } }
                    "保存"
                }
                button {
                    class: "secondary-button compact",
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    title: "使用远程规则和订阅替换本机数据",
                    onclick: move |_| async move {
                        if confirm() != Some(SyncAction::Pull) {
                            confirm.set(Some(SyncAction::Pull));
                            return;
                        }
                        confirm.set(None);
                        let value = config();
                        busy.set(Some(SyncAction::Pull));
                        match api::sync_pull(value.clone()).await {
                            Ok(downloaded) => {
                                let snapshot = downloaded.snapshot;
                                let profile = AppProfile {
                                    subscriptions: subscriptions(),
                                    active_subscription_id: active_subscription_id(),
                                    selected_tag: selected_tag(),
                                    tunnel_mode: tunnel_mode(),
                                    tun_enabled: tun_enabled(),
                                    allow_lan: allow_lan(),
                                    dark_mode: dark_mode(),
                                    custom_rules: custom_rules(),
                                    config_script_enabled: config_script_enabled(),
                                    config_script: config_script(),
                                    group_selections: group_selections(),
                                    ..AppProfile::default()
                                };
                                let result = api::commit_sync_pull(
                                    value,
                                    profile,
                                    snapshot.clone(),
                                    downloaded.remote_revision,
                                )
                                .await;
                                let result = match result {
                                    Ok(result) => result,
                                    Err(error) => {
                                        toast.error(format!(
                                            "远端数据已下载，但保存到本机失败: {error}"
                                        ));
                                        busy.set(None);
                                        return;
                                    }
                                };
                                apply_sync_snapshot(
                                    snapshot,
                                    SyncTargetSignals {
                                        subscriptions,
                                        active_subscription_id,
                                        nodes,
                                        selected_tag,
                                        custom_rules,
                                        dns_overrides,
                                        group_selections,
                                    },
                                );
                                if result.checkpoint_saved {
                                    let mut current = config.write();
                                    current.last_sync_at = result.updated_at;
                                    current.last_remote_revision = result.remote_revision.clone();
                                }
                                toast.success(format!(
                                    "已下载 {} 个订阅和 {} 条规则{}",
                                    result.subscription_count,
                                    result.rule_count,
                                    if result.subscription_count > 0 {
                                        "，订阅需刷新后使用"
                                    } else {
                                        ""
                                    }
                                ));
                                if let Some(warning) = result.warning {
                                    toast.info(warning);
                                }
                            }
                            Err(error) => toast.error(format!("远端下载失败: {error}")),
                        }
                        busy.set(None);
                    },
                    if busy() == Some(SyncAction::Pull) { span { class: "spinner" } }
                    else { Icon { icon: LdCloudDownload, width: 17, height: 17 } }
                    if confirm() == Some(SyncAction::Pull) { "确认覆盖本机" } else { "下载覆盖本机" }
                }
                button {
                    class: "primary-button compact",
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    onclick: move |_| {
                        confirm.set(None);
                        let value = config();
                        let snapshot = build_sync_snapshot(
                            &subscriptions(),
                            &custom_rules(),
                            &dns_overrides(),
                            value.last_sync_at,
                        );
                        async move {
                            busy.set(Some(SyncAction::Push));
                            let result = api::sync_push(value, snapshot, false).await;
                            match result {
                                Ok(result) => {
                                    if result.checkpoint_saved {
                                        let mut current = config.write();
                                        current.last_sync_at = result.updated_at;
                                        current.last_remote_revision = result.remote_revision.clone();
                                    }
                                    toast.success(format!(
                                        "已上传 {} 个订阅和 {} 条规则",
                                        result.subscription_count, result.rule_count
                                    ));
                                    if let Some(warning) = result.warning {
                                        toast.info(warning);
                                    }
                                }
                                Err(error) => toast.error(format!("远端上传失败: {error}")),
                            }
                            busy.set(None);
                        }
                    },
                    if busy() == Some(SyncAction::Push) { span { class: "spinner" } }
                    else { Icon { icon: LdCloudUpload, width: 17, height: 17 } }
                    "上传"
                }
                button {
                    class: "secondary-button compact danger-text-button",
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    title: "忽略远端版本并使用本机数据覆盖",
                    onclick: move |_| async move {
                        if confirm() != Some(SyncAction::ForcePush) {
                            confirm.set(Some(SyncAction::ForcePush));
                            return;
                        }
                        confirm.set(None);
                        let value = config();
                        let snapshot = build_sync_snapshot(
                            &subscriptions(),
                            &custom_rules(),
                            &dns_overrides(),
                            value.last_sync_at,
                        );
                        busy.set(Some(SyncAction::ForcePush));
                        let result = api::sync_push(value, snapshot, true).await;
                        match result {
                            Ok(result) => {
                                if result.checkpoint_saved {
                                    let mut current = config.write();
                                    current.last_sync_at = result.updated_at;
                                    current.last_remote_revision = result.remote_revision.clone();
                                }
                                toast.success("已覆盖远端同步文件");
                                if let Some(warning) = result.warning {
                                    toast.info(warning);
                                }
                            }
                            Err(error) => toast.error(format!("覆盖远端失败: {error}")),
                        }
                        busy.set(None);
                    },
                    if busy() == Some(SyncAction::ForcePush) { span { class: "spinner" } }
                    else { Icon { icon: LdCloudUpload, width: 17, height: 17 } }
                    if confirm() == Some(SyncAction::ForcePush) { "确认覆盖远端" } else { "覆盖远端" }
                }
            }
        }
    }
}

fn build_sync_snapshot(
    subscriptions: &[Subscription],
    custom_rules: &[CustomRule],
    dns_overrides: &[DnsOverride],
    updated_at: u64,
) -> SyncSnapshot {
    SyncSnapshot {
        format: proxy_core::SYNC_SNAPSHOT_FORMAT.to_string(),
        version: SYNC_SNAPSHOT_VERSION,
        updated_at,
        subscriptions: subscriptions.iter().map(SyncSubscription::from).collect(),
        custom_rules: custom_rules.to_vec(),
        dns_overrides: dns_overrides.to_vec(),
    }
}

/// Mutable signals a pulled snapshot writes back into, grouped to keep
/// `apply_sync_snapshot` within the clippy argument budget.
#[derive(Clone, Copy)]
struct SyncTargetSignals {
    subscriptions: Signal<Vec<Subscription>>,
    active_subscription_id: Signal<Option<u64>>,
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    group_selections: Signal<HashMap<String, String>>,
}

fn apply_sync_snapshot(snapshot: SyncSnapshot, targets: SyncTargetSignals) {
    let SyncTargetSignals {
        mut subscriptions,
        mut active_subscription_id,
        mut nodes,
        mut selected_tag,
        mut custom_rules,
        mut dns_overrides,
        mut group_selections,
    } = targets;
    subscriptions.set(
        snapshot
            .subscriptions
            .into_iter()
            .map(Subscription::from)
            .collect(),
    );
    active_subscription_id.set(None);
    nodes.set(Vec::new());
    selected_tag.set(String::new());
    custom_rules.set(snapshot.custom_rules);
    dns_overrides.set(snapshot.dns_overrides);
    group_selections.set(HashMap::new());
}

#[cfg(all(
    feature = "native-desktop",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
fn desktop_autostart_settings() -> Element {
    if use_context::<DesktopAutostartCapability>().0 {
        rsx! { DesktopAutostartSettings {} }
    } else {
        rsx! {}
    }
}

#[cfg(not(all(
    feature = "native-desktop",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
)))]
fn desktop_autostart_settings() -> Element {
    let _ = use_context::<DesktopAutostartCapability>().0;
    rsx! {}
}

#[cfg(all(
    feature = "native-desktop",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
#[component]
fn DesktopAutostartSettings() -> Element {
    let mut enabled = use_signal(|| false);
    let mut ready = use_signal(|| false);
    let mut busy = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);
    use_future(move || async move {
        match api::autostart::status().await {
            Ok(value) => {
                enabled.set(value);
                ready.set(true);
            }
            Err(err) => error.set(Some(err.to_string())),
        }
        busy.set(false);
    });
    rsx! {
        section { class: "settings-section autostart-settings glass-surface",
            div { class: "section-heading", h2 { "桌面启动" } }
            label { class: "setting-row toggle-row",
                div {
                    strong { "用户登录时启动" }
                    small { "仅启动 Kitty Pro，不自动连接代理或开启 TUN；默认关闭" }
                }
                input {
                    r#type: "checkbox",
                    aria_label: "用户登录时启动 Kitty Pro",
                    checked: enabled(),
                    disabled: busy() || !ready(),
                    onclick: move |event| {
                        event.prevent_default();
                        if busy() || !ready() { return; }
                        let target = !enabled();
                        busy.set(true);
                        error.set(None);
                        spawn(async move {
                            match api::autostart::set_enabled(target).await {
                                Ok(value) => enabled.set(value),
                                Err(err) => {
                                    error.set(Some(err.to_string()));
                                    // 设置可能已部分生效，回读真实状态避免开关显示反向。
                                    // 只有回读值与操作目标一致才算真正成功；不一致时保留
                                    // 失败提示，否则用户会看到开关静默弹回且没有任何说明。
                                    if let Ok(value) = api::autostart::status().await {
                                        enabled.set(value);
                                        if value == target {
                                            error.set(None);
                                        }
                                    }
                                }
                            }
                            busy.set(false);
                        });
                    },
                }
                span { class: "switch" }
            }
            if busy() {
                div { class: "inline-note", aria_busy: "true",
                    span { class: "spinner" }
                    span { "正在读取或更新系统启动项…" }
                }
            }
            if let Some(message) = error() {
                div { class: "inline-note", role: "alert",
                    Icon { icon: LdCircleAlert, width: 16, height: 16 }
                    span { "{message}" }
                }
                div { class: "rule-editor-actions",
                    button {
                        class: "secondary-button compact",
                        disabled: busy(),
                        onclick: move |_| {
                            if busy() { return; }
                            busy.set(true);
                            error.set(None);
                            spawn(async move {
                                match api::autostart::status().await {
                                    Ok(value) => { enabled.set(value); ready.set(true); }
                                    Err(err) => { ready.set(false); error.set(Some(err.to_string())); }
                                }
                                busy.set(false);
                            });
                        },
                        Icon { icon: LdRefreshCw, width: 15, height: 15 }
                        "重新读取系统状态"
                    }
                }
            }
        }
    }
}

fn add_dns_override_rule(
    mut domain: Signal<String>,
    mut server: Signal<String>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    mut error: Signal<Option<String>>,
    connected: bool,
    toast: ToastManager,
) {
    match proxy_core::normalize_dns_override(&domain(), &server()) {
        Ok((domain_suffix, dns_server)) => {
            let mut rules = dns_overrides.write();
            if rules.iter().any(|item| item.domain_suffix == domain_suffix) {
                error.set(Some(format!("已存在 {domain_suffix} 的专用 DNS")));
                return;
            }
            let id = rules.iter().map(|item| item.id).max().unwrap_or(0) + 1;
            rules.push(DnsOverride {
                id,
                enabled: true,
                domain_suffix,
                server: dns_server,
            });
            domain.set(String::new());
            server.set(String::new());
            error.set(None);
            drop(rules);
            notify_rule_change(connected, toast);
        }
        Err(message) => error.set(Some(message)),
    }
}

#[component]
fn SettingsView(
    platform: String,
    mut core_state: Signal<String>,
    mut core_version: Signal<Option<String>>,
    mut core_note: Signal<Option<String>>,
    mut connected: Signal<bool>,
    core_busy: Signal<bool>,
    mut core_restarting: Signal<bool>,
    system_proxy_busy: Signal<bool>,
    mut runtime_action_busy: Signal<bool>,
    mut tunnel_mode: Signal<TunnelMode>,
    mut tun_enabled: Signal<bool>,
    mut allow_lan: Signal<bool>,
    mut dark_mode: Signal<bool>,
    mut nodes: Signal<Vec<ProxyNode>>,
    mut selected_tag: Signal<String>,
    mut subscriptions: Signal<Vec<Subscription>>,
    mut active_subscription_id: Signal<Option<u64>>,
    proxy_server_nameservers: Vec<String>,
    mut custom_rules: Signal<Vec<CustomRule>>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    mut config_script_enabled: Signal<bool>,
    mut config_script: Signal<String>,
    mut group_selections: Signal<HashMap<String, String>>,
) -> Element {
    let mut script_check_busy = use_signal(|| false);
    let mut script_editor_open = use_signal(|| false);
    let mut script_draft = use_signal(String::new);
    let mut copy_busy = use_signal(|| false);
    let mut copy_env_busy = use_signal(|| false);
    let tun_busy = use_signal(|| false);
    let toast = use_context::<ToastManager>();
    let tun_proxy_server_nameservers = proxy_server_nameservers.clone();
    let allow_lan_proxy_server_nameservers = proxy_server_nameservers.clone();
    let script_proxy_server_nameservers = proxy_server_nameservers;
    let autostart_settings = desktop_autostart_settings();

    rsx! {
        div { class: "settings-grid",
            // Design row A: the core card (runtime + JS override) sits next to
            // the routing card, so both fit above the fold on a laptop screen.
            section { class: "settings-section core-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "CORE" }
                        h2 { "内核" }
                    }
                    span { class: status_badge_class(&core_state()), "{core_state_label(&core_state())}" }
                }
                div { class: "setting-row",
                    span { class: "setting-icon", Icon { icon: LdShieldCheck, width: 19, height: 19 } }
                    div {
                        strong { "sing-box" }
                        small { {core_version().unwrap_or_else(|| "未检测到版本".to_string())} }
                    }
                    span { class: "setting-value", "{platform}" }
                }
                if let Some(note) = core_note() {
                    div { class: "inline-note",
                        Icon { icon: LdInfo, width: 16, height: 16 }
                        span { "{note}" }
                    }
                }
                div { class: "setting-row script-setting-row",
                    span { class: "setting-icon", Icon { icon: LdScrollText, width: 19, height: 19 } }
                    div {
                        strong { "JavaScript 配置覆写" }
                        small {
                            if config_script().trim().is_empty() {
                                "main(config) · 尚未配置"
                            } else {
                                "main(config) · QuickJS · {config_script().chars().count()} 字符"
                            }
                        }
                    }
                    button {
                        class: "icon-button",
                        title: "编辑 JavaScript 配置覆写",
                        aria_label: "编辑 JavaScript 配置覆写",
                        onclick: move |_| {
                            script_draft.set(config_script());
                            script_editor_open.set(true);
                        },
                        Icon { icon: LdPencil, width: 17, height: 17 }
                    }
                    label { class: "compact-toggle", title: "启用 JavaScript 配置覆写",
                        input {
                            r#type: "checkbox",
                            aria_label: "启用 JavaScript 配置覆写",
                            checked: config_script_enabled,
                            disabled: config_script().trim().is_empty(),
                            onchange: move |event| config_script_enabled.set(event.checked()),
                        }
                        span { class: "switch" }
                    }
                }
            }

            // Design row A pairs routing with the core card.
            section { class: "settings-section routing-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "ROUTING" }
                        h2 { "路由模式" }
                    }
                }
                div { class: "segmented-control wide", aria_label: "分流模式",
                    button { class: if tunnel_mode() == TunnelMode::Rule { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Rule), "规则模式" }
                    button { class: if tunnel_mode() == TunnelMode::Global { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Global), "全局代理" }
                    button { class: if tunnel_mode() == TunnelMode::Direct { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Direct), "全局直连" }
                }
                if !cfg!(target_os = "android") {
                label { class: "setting-row toggle-row",
                    span { class: "setting-icon", Icon { icon: LdNetwork, width: 19, height: 19 } }
                    div {
                        strong { "TUN 模式" }
                        small { "系统级网络接管" }
                    }
                    if tun_busy() {
                        span {
                            class: if tun_enabled() { "switch switch-loading active" } else { "switch switch-loading" },
                            aria_busy: "true",
                            aria_label: if tun_enabled() { "正在关闭 TUN 模式" } else { "正在准备 TUN 权限" },
                            span { class: "spinner" }
                        }
                    } else {
                        input {
                            r#type: "checkbox",
                            checked: tun_enabled,
                            disabled: core_busy()
                                || core_restarting()
                                || system_proxy_busy()
                                || runtime_action_busy(),
                            onclick: move |event| {
                                event.prevent_default();
                                let enabled = !tun_enabled();
                                let request = build_connection_request(
                                    nodes(),
                                    selected_tag(),
                                    tun_proxy_server_nameservers.clone(),
                                    RuntimeSettings {
                                        tunnel_mode,
                                        tun_enabled,
                                        allow_lan,
                                        custom_rules,
                                        dns_overrides,
                                        config_script_enabled,
                                        config_script,
                                    },
                                    group_selections(),
                                );
                                spawn_tun_toggle(
                                    enabled,
                                    request,
                                    tun_enabled,
                                    tun_busy,
                                    RuntimeSignals {
                                        status: CoreStatusSignals {
                                            connected,
                                            core_restarting,
                                            core_state,
                                            core_version,
                                            core_note,
                                        },
                                        core_busy,
                                        system_proxy_busy,
                                        runtime_action_busy,
                                    },
                                    toast,
                                );
                            },
                        }
                        span { class: "switch" }
                    }
                }
                }
                label {
                    class: "setting-row toggle-row",
                    title: "允许同一局域网设备直接使用此代理",
                    span { class: "setting-icon", Icon { icon: LdWifi, width: 19, height: 19 } }
                    div {
                        strong { "允许局域网连接" }
                        small {
                            if allow_lan() { "0.0.0.0:7890 · 无认证" }
                            else { "127.0.0.1:7890" }
                        }
                    }
                    input {
                        r#type: "checkbox",
                        aria_label: "允许局域网连接",
                        checked: allow_lan,
                        disabled: core_busy()
                            || core_restarting()
                            || system_proxy_busy()
                            || runtime_action_busy(),
                        onchange: move |event| {
                                let enabled = event.checked();
                                let request = build_connection_request(
                                    nodes(),
                                    selected_tag(),
                                    allow_lan_proxy_server_nameservers.clone(),
                                    RuntimeSettings {
                                        tunnel_mode,
                                        tun_enabled,
                                        allow_lan,
                                        custom_rules,
                                        dns_overrides,
                                        config_script_enabled,
                                        config_script,
                                    },
                                    group_selections(),
                                );
                                spawn_allow_lan_toggle(
                                    enabled,
                                    request,
                                    allow_lan,
                                    RuntimeSignals {
                                        status: CoreStatusSignals {
                                            connected,
                                            core_restarting,
                                            core_state,
                                            core_version,
                                            core_note,
                                        },
                                        core_busy,
                                        system_proxy_busy,
                                        runtime_action_busy,
                                    },
                                    toast,
                                );
                            },
                    }
                    span { class: "switch" }
                }
            }


            SyncSettings {
                subscriptions,
                active_subscription_id,
                nodes,
                selected_tag,
                custom_rules,
                dns_overrides,
                group_selections,
                tunnel_mode,
                tun_enabled,
                allow_lan,
                dark_mode,
                config_script_enabled,
                config_script,
            }

            section { class: "settings-section appearance-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "APPEARANCE" }
                        h2 { "外观" }
                    }
                }
                label { class: "setting-row toggle-row",
                    span { class: "setting-icon",
                        if dark_mode() { Icon { icon: LdMoon, width: 19, height: 19 } }
                        else { Icon { icon: LdSun, width: 19, height: 19 } }
                    }
                    div {
                        strong { "深色模式" }
                        small { if dark_mode() { "已启用" } else { "跟随浅色外观" } }
                    }
                    input {
                        r#type: "checkbox",
                        checked: dark_mode,
                        onchange: move |event| dark_mode.set(event.checked()),
                    }
                    span { class: "switch" }
                }
                div { class: "setting-row",
                    span { class: "setting-icon", Icon { icon: LdLanguages, width: 19, height: 19 } }
                    div {
                        strong { "语言" }
                        small { "界面语言" }
                    }
                    span { class: "setting-value", "简体中文" }
                }
            }

            section { class: "settings-section about-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "ABOUT" }
                        h2 { "诊断与关于" }
                    }
                }
                div { class: "about-identity",
                    span { class: "about-mark", aria_hidden: "true",
                        dangerous_inner_html: BRAND_ICON_SVG,
                    }
                    div {
                        strong { "Kitty Pro" }
                        small { {about_build_label(core_version(), &platform)} }
                    }
                }
                div { class: "about-facts",
                    div { class: "about-fact",
                        span { "内核状态" }
                        strong { {core_state_label(&core_state())} }
                    }
                    div { class: "about-fact",
                        span { "节点 / 订阅" }
                        strong { "{nodes().len()} / {subscriptions().len()}" }
                    }
                    div { class: "about-fact",
                        span { "自定义规则" }
                        strong { "{custom_rules().len()}" }
                    }
                    div { class: "about-fact",
                        span { "专用 DNS" }
                        strong { "{dns_overrides().len()}" }
                    }
                }
                div { class: "topbar-actions about-actions",
                    button {
                        class: "secondary-button compact",
                        title: "复制内核与配置诊断信息",
                        disabled: copy_busy(),
                        onclick: move |_| {
                            let payload = build_diagnostic_report(
                                core_state(),
                                core_version(),
                                &platform,
                                tunnel_mode(),
                                tun_enabled(),
                                allow_lan(),
                                nodes().len(),
                                subscriptions().len(),
                                custom_rules().len(),
                                dns_overrides().len(),
                            );
                            copy_busy.set(true);
                            async move {
                                match copy_to_clipboard(payload).await {
                                    Ok(()) => toast.success("诊断信息已复制到剪贴板"),
                                    Err(error) => toast.error(format!("复制失败: {error}")),
                                }
                                copy_busy.set(false);
                            }
                        },
                        if copy_busy() {
                            span { class: "spinner" }
                        } else {
                            Icon { icon: LdClipboardCopy, width: 16, height: 16 }
                        }
                        span { "复制诊断信息" }
                    }
                    button {
                        class: "secondary-button compact",
                        title: "复制代理环境变量",
                        disabled: copy_env_busy(),
                        onclick: move |_| {
                            copy_env_busy.set(true);
                            async move {
                                match api::copy_proxy_environment_variables().await {
                                    Ok(()) => toast.success("代理环境变量已复制"),
                                    Err(error) => {
                                        toast.error(format!("复制环境变量失败: {error}"));
                                    }
                                }
                                copy_env_busy.set(false);
                            }
                        },
                        if copy_env_busy() {
                            span { class: "spinner" }
                        } else {
                            Icon { icon: LdTerminal, width: 16, height: 16 }
                        }
                        span { "复制环境变量" }
                    }
                }
            }

            {autostart_settings}
        }

        if script_editor_open() {
            div {
                class: "modal-backdrop",
                role: "presentation",
                onclick: move |_| {
                    if !script_check_busy() {
                        script_editor_open.set(false);
                    }
                },
                div {
                    class: "modal script-modal glass-modal",
                    role: "dialog",
                    aria_modal: "true",
                    aria_label: "编辑 JavaScript 配置覆写",
                    onclick: move |event| event.stop_propagation(),
                    div { class: "modal-header",
                        div {
                            p { class: "eyebrow", "CONFIG SCRIPT" }
                            h2 { "JavaScript 配置覆写" }
                        }
                        button {
                            class: "icon-button",
                            title: "关闭",
                            aria_label: "关闭脚本编辑器",
                            disabled: script_check_busy(),
                            onclick: move |_| script_editor_open.set(false),
                            Icon { icon: LdX, width: 19, height: 19 }
                        }
                    }
                    textarea {
                        class: "script-editor",
                        aria_label: "JavaScript 配置覆写脚本",
                        spellcheck: "false",
                        placeholder: "function main(config) {{\n  return config;\n}}",
                        value: script_draft,
                        oninput: move |event| script_draft.set(event.value()),
                    }
                    div { class: "modal-actions",
                        button {
                            class: "secondary-button",
                            disabled: script_check_busy(),
                            onclick: move |_| script_editor_open.set(false),
                            "取消"
                        }
                        button {
                            class: "secondary-button",
                            disabled: script_check_busy() || script_draft().trim().is_empty(),
                            onclick: move |_| {
                                let proxy_server_nameservers =
                                    script_proxy_server_nameservers.clone();
                                async move {
                                script_check_busy.set(true);
                                let request = ConnectionRequest {
                                    nodes: nodes(),
                                    selected_tag: selected_tag(),
                                    proxy_server_nameservers: proxy_server_nameservers.clone(),
                                    mode: tunnel_mode(),
                                    tun: tun_enabled(),
                                    allow_lan: allow_lan(),
                                    custom_rules: Vec::new(),
                                    dns_overrides: dns_overrides(),
                                    config_script: Some(script_draft()),
                                    group_selections: group_selections(),
                                };
                                match api::validate_config_script(request).await {
                                    Ok(()) => toast.success("配置脚本校验通过"),
                                    Err(error) => {
                                        toast.error(format!("配置脚本校验失败: {error}"));
                                    }
                                }
                                script_check_busy.set(false);
                                }
                            },
                            if script_check_busy() {
                                span { class: "spinner" }
                            } else {
                                Icon { icon: LdCircleCheck, width: 17, height: 17 }
                            }
                            "校验"
                        }
                        button {
                            class: "primary-button",
                            disabled: script_check_busy(),
                            onclick: move |_| {
                                let value = script_draft();
                                if value.trim().is_empty() {
                                    config_script_enabled.set(false);
                                }
                                config_script.set(value);
                                script_editor_open.set(false);
                                toast.success("配置脚本已保存");
                            },
                            Icon { icon: LdSave, width: 16, height: 16 }
                            "保存"
                        }
                    }
                }
            }
        }
    }
}

fn protocol_abbreviation(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::AnyTls => "AT",
        ProxyProtocol::Hysteria2 => "HY",
        ProxyProtocol::Vmess => "VM",
        ProxyProtocol::Vless => "VL",
        ProxyProtocol::Trojan => "TR",
        ProxyProtocol::Shadowsocks => "SS",
        ProxyProtocol::Http => "HT",
        ProxyProtocol::Socks5 => "SO",
    }
}

fn protocol_class(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::AnyTls => "protocol-mark anytls",
        ProxyProtocol::Hysteria2 => "protocol-mark hy2",
        ProxyProtocol::Vmess => "protocol-mark vmess",
        ProxyProtocol::Vless => "protocol-mark vless",
        ProxyProtocol::Trojan => "protocol-mark trojan",
        ProxyProtocol::Shadowsocks => "protocol-mark shadowsocks",
        ProxyProtocol::Http => "protocol-mark http",
        ProxyProtocol::Socks5 => "protocol-mark socks5",
    }
}

fn mode_label(mode: TunnelMode) -> &'static str {
    match mode {
        TunnelMode::Rule => "规则模式",
        TunnelMode::Global => "全局模式",
        TunnelMode::Direct => "直连模式",
    }
}

fn core_state_label(state: &str) -> &'static str {
    match state {
        "running" => "内核运行中",
        "stopped" => "内核已就绪",
        "checking" => "正在检测",
        _ => "内核不可用",
    }
}

fn status_dot_class(state: &str) -> &'static str {
    match state {
        "running" => "status-dot online",
        "stopped" => "status-dot ready",
        _ => "status-dot offline",
    }
}

fn status_badge_class(state: &str) -> &'static str {
    match state {
        "running" => "status-badge online",
        "stopped" => "status-badge ready",
        _ => "status-badge offline",
    }
}

/// Topbar chip: the design keeps a connected core and an idle-but-ready core in
/// the same amber family, and only escalates to coral when the core is gone.
fn connection_chip_class(connected: bool, state: &str) -> &'static str {
    match (connected, state) {
        (true, _) => "status-badge online",
        (false, "unavailable") => "status-badge offline",
        _ => "status-badge pending",
    }
}

fn connection_chip_label(connected: bool, state: &str) -> &'static str {
    match (connected, state) {
        (true, _) => "已连接",
        (false, "unavailable") => "内核不可用",
        (false, "checking") => "检测中",
        _ => "未连接",
    }
}

fn next_subscription_id(subscriptions: &[Subscription]) -> u64 {
    subscriptions
        .iter()
        .map(|subscription| subscription.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn namespace_nodes(subscription_id: u64, mut nodes: Vec<ProxyNode>) -> Vec<ProxyNode> {
    for node in &mut nodes {
        node.tag = format!("subscription-{subscription_id}-{}", node.tag);
    }
    nodes
}

fn apply_subscription_report(
    subscriptions: &mut [Subscription],
    subscription_id: u64,
    report: ParseReport,
) -> Result<usize, String> {
    if report.nodes.is_empty() {
        return Err(report
            .rejected
            .first()
            .map(|issue| issue.reason.clone())
            .unwrap_or_else(|| "没有找到可用节点".to_string()));
    }
    let ParseReport {
        nodes,
        rejected,
        proxy_server_nameservers,
    } = report;
    let count = nodes.len();
    let rejected_count = rejected.len();
    let subscription = subscriptions
        .iter_mut()
        .find(|subscription| subscription.id == subscription_id)
        .ok_or_else(|| "订阅已不存在".to_string())?;
    subscription.nodes = namespace_nodes(subscription_id, nodes);
    subscription.proxy_server_nameservers = proxy_server_nameservers;
    subscription.rejected_count = rejected_count;
    Ok(count)
}

#[cfg(test)]
fn collect_nodes(subscriptions: &[Subscription]) -> Vec<ProxyNode> {
    subscriptions
        .iter()
        .flat_map(|subscription| subscription.nodes.iter().cloned())
        .collect()
}

fn resolve_active_subscription_id(
    subscriptions: &[Subscription],
    requested_id: Option<u64>,
    selected_tag: &str,
) -> Option<u64> {
    requested_id
        .filter(|id| {
            subscriptions
                .iter()
                .any(|subscription| subscription.id == *id)
        })
        .or_else(|| {
            subscriptions
                .iter()
                .find(|subscription| {
                    subscription
                        .nodes
                        .iter()
                        .any(|node| node.tag == selected_tag)
                })
                .map(|subscription| subscription.id)
        })
        .or_else(|| subscriptions.first().map(|subscription| subscription.id))
}

fn collect_subscription_nodes(
    subscriptions: &[Subscription],
    subscription_id: Option<u64>,
) -> Vec<ProxyNode> {
    subscription_id
        .and_then(|id| {
            subscriptions
                .iter()
                .find(|subscription| subscription.id == id)
        })
        .map(|subscription| subscription.nodes.clone())
        .unwrap_or_default()
}

fn active_subscription_proxy_server_nameservers(
    subscriptions: &[Subscription],
    subscription_id: Option<u64>,
) -> Vec<String> {
    subscription_id
        .and_then(|id| {
            subscriptions
                .iter()
                .find(|subscription| subscription.id == id)
        })
        .map(|subscription| subscription.proxy_server_nameservers.clone())
        .unwrap_or_default()
}

fn sort_proxy_group_members(
    members: &mut [ProxyGroupMember],
    results: &HashMap<String, NodeLatency>,
) {
    members.sort_by_key(|member| {
        member
            .node
            .as_ref()
            .and_then(|node| results.get(&node.tag))
            .map(|result| match result.latency_ms {
                Some(latency) => (0, latency),
                None => (2, u64::MAX),
            })
            .unwrap_or((1, u64::MAX))
    });
}

fn select_available_tag(nodes: &[ProxyNode], requested_tag: &str) -> String {
    if nodes.iter().any(|node| node.tag == requested_tag) {
        requested_tag.to_string()
    } else {
        nodes
            .first()
            .map(|node| node.tag.clone())
            .unwrap_or_default()
    }
}

fn source_label(source: &str) -> String {
    let trimmed = source.trim();
    if let Some((scheme, remainder)) = trimmed.split_once("://") {
        if matches!(scheme, "http" | "https") {
            let authority = remainder.split(['/', '?', '#']).next().unwrap_or(remainder);
            let host = authority.rsplit('@').next().unwrap_or(authority);
            return format!("{scheme}://{host}");
        }
    }
    "本地内容".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_subscriptions_get_distinct_node_tags() {
        let node = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge",
        )
        .nodes
        .pop()
        .expect("fixture should parse");
        let first = namespace_nodes(1, vec![node.clone()]);
        let second = namespace_nodes(2, vec![node]);
        let nodes = collect_nodes(&[
            Subscription {
                id: 1,
                name: "One".to_string(),
                source: "one".to_string(),
                nodes: first,
                proxy_server_nameservers: Vec::new(),
                rejected_count: 0,
            },
            Subscription {
                id: 2,
                name: "Two".to_string(),
                source: "two".to_string(),
                nodes: second,
                proxy_server_nameservers: Vec::new(),
                rejected_count: 0,
            },
        ]);

        assert_eq!(nodes.len(), 2);
        assert_ne!(nodes[0].tag, nodes[1].tag);
        assert_eq!(select_available_tag(&nodes, "missing"), nodes[0].tag);
    }

    #[test]
    fn active_subscription_follows_selection_and_limits_nodes() {
        let node = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge",
        )
        .nodes
        .pop()
        .expect("fixture should parse");
        let subscriptions = vec![
            Subscription {
                id: 1,
                name: "One".to_string(),
                source: "one".to_string(),
                nodes: namespace_nodes(1, vec![node.clone()]),
                proxy_server_nameservers: Vec::new(),
                rejected_count: 0,
            },
            Subscription {
                id: 2,
                name: "Two".to_string(),
                source: "two".to_string(),
                nodes: namespace_nodes(2, vec![node]),
                proxy_server_nameservers: Vec::new(),
                rejected_count: 0,
            },
        ];
        let second_tag = subscriptions[1].nodes[0].tag.clone();

        assert_eq!(
            resolve_active_subscription_id(&subscriptions, None, &second_tag),
            Some(2)
        );
        let active_nodes = collect_subscription_nodes(&subscriptions, Some(2));
        assert_eq!(active_nodes.len(), 1);
        assert_eq!(active_nodes[0].tag, second_tag);

        let remaining = vec![subscriptions[0].clone()];
        let fallback_id = resolve_active_subscription_id(&remaining, Some(2), &second_tag);
        assert_eq!(fallback_id, Some(1));
        assert_eq!(collect_subscription_nodes(&remaining, fallback_id).len(), 1);
    }

    #[test]
    fn source_label_hides_subscription_tokens() {
        assert_eq!(
            source_label("https://user:pass@sub.example.com/path?token=secret"),
            "https://sub.example.com"
        );
    }

    #[test]
    fn node_focus_resolves_owner_source_and_status() {
        let parsed =
            proxy_core::parse_subscription("trojan://password@edge.example.com:443#Edge").nodes;
        let subscriptions = vec![Subscription {
            id: 7,
            name: "Primary".to_string(),
            source: "https://sub.example.com/path?token=secret".to_string(),
            nodes: namespace_nodes(7, parsed.clone()),
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        }];
        let node = &subscriptions[0].nodes[0];
        let groups = vec![ProxyGroup {
            tag: "proxy".to_string(),
            kind: ProxyGroupKind::Selector,
            outbounds: vec![node.tag.clone()],
            selected: node.tag.clone(),
        }];
        let mut latencies = HashMap::new();
        latencies.insert(
            node.tag.clone(),
            NodeLatency {
                tag: node.tag.clone(),
                latency_ms: Some(42),
                error: None,
            },
        );

        let focus = node_focus_details(
            &groups,
            &subscriptions[0].nodes,
            &subscriptions,
            &node.tag,
            &latencies,
        )
        .expect("focused node should resolve");

        assert_eq!(focus.name, "Edge");
        assert_eq!(focus.protocol, "Trojan");
        assert_eq!(focus.endpoint, "edge.example.com:443");
        assert_eq!(focus.group_label, "proxy · 手动选择");
        assert_eq!(focus.status, "已选中 · 可用");
        assert_eq!(focus.source, "https://sub.example.com");

        let ungrouped = node_focus_details(
            &[],
            &subscriptions[0].nodes,
            &[],
            &node.tag,
            &HashMap::new(),
        )
        .expect("an ungrouped node still has details");
        assert_eq!(ungrouped.group_label, "未归组");
        assert_eq!(ungrouped.status, "已选中 · 未测试");
        assert_eq!(ungrouped.source, "本地内容");
    }

    #[test]
    fn empty_refresh_does_not_replace_existing_nodes() {
        let original = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@old.example.com:443#Old",
        )
        .nodes
        .pop()
        .expect("fixture should parse");
        let mut subscriptions = vec![Subscription {
            id: 3,
            name: "Primary".to_string(),
            source: "https://example.com/subscription".to_string(),
            nodes: vec![original.clone()],
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        }];

        let result = apply_subscription_report(
            &mut subscriptions,
            3,
            ParseReport {
                nodes: Vec::new(),
                rejected: vec![proxy_core::ParseIssue {
                    line: 1,
                    reason: "无效内容".to_string(),
                }],
                proxy_server_nameservers: Vec::new(),
            },
        );

        assert_eq!(result, Err("无效内容".to_string()));
        assert_eq!(subscriptions[0].nodes, vec![original]);
    }

    #[test]
    fn successful_refresh_replaces_nodes_with_a_scoped_tag() {
        let original = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@old.example.com:443#Old",
        )
        .nodes
        .pop()
        .expect("fixture should parse");
        let replacement =
            proxy_core::parse_subscription("trojan://password@new.example.com:443#New")
                .nodes
                .pop()
                .expect("fixture should parse");
        let mut subscriptions = vec![Subscription {
            id: 9,
            name: "Primary".to_string(),
            source: "https://example.com/subscription".to_string(),
            nodes: vec![original],
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        }];

        assert_eq!(
            apply_subscription_report(
                &mut subscriptions,
                9,
                ParseReport {
                    nodes: vec![replacement],
                    rejected: Vec::new(),
                    proxy_server_nameservers: vec![
                        "https://resolver.example.com/dns-query".to_string(),
                    ],
                },
            ),
            Ok(1)
        );
        assert_eq!(subscriptions[0].nodes.len(), 1);
        assert!(subscriptions[0].nodes[0].tag.starts_with("subscription-9-"));
        assert_eq!(
            subscriptions[0].proxy_server_nameservers,
            vec!["https://resolver.example.com/dns-query"]
        );
    }

    #[test]
    fn latency_labels_distinguish_success_and_failure() {
        let success = NodeLatency {
            tag: "edge".to_string(),
            latency_ms: Some(321),
            error: None,
        };
        let failure = NodeLatency {
            tag: "offline".to_string(),
            latency_ms: None,
            error: Some("探测超时或节点不可用".to_string()),
        };

        assert_eq!(format_latency(Some(&success)).0, "321 ms");
        assert_eq!(format_latency(Some(&success)).1, "latency success");
        assert_eq!(format_latency(Some(&failure)).0, "失败");
        assert_eq!(format_latency(Some(&failure)).1, "latency error");
    }

    #[test]
    fn latency_cache_keeps_successes_for_unchanged_endpoints() {
        let node = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge",
        )
        .nodes
        .pop()
        .expect("fixture should parse");
        let results = HashMap::from([
            (
                node.tag.clone(),
                NodeLatency {
                    tag: node.tag.clone(),
                    latency_ms: Some(128),
                    error: None,
                },
            ),
            (
                "offline".to_string(),
                NodeLatency {
                    tag: "offline".to_string(),
                    latency_ms: None,
                    error: Some("timeout".to_string()),
                },
            ),
        ]);

        let entries = latency_cache_entries(std::slice::from_ref(&node), &results);
        assert_eq!(entries, vec![(node.tag.clone(), node.endpoint(), 128)]);

        let restored = restore_cached_latencies(std::slice::from_ref(&node), entries.clone());
        assert_eq!(
            restored.get(&node.tag).and_then(|item| item.latency_ms),
            Some(128)
        );

        let mut renamed_node = node.clone();
        renamed_node.tag = "renamed-edge".to_string();
        let restored_renamed =
            restore_cached_latencies(std::slice::from_ref(&renamed_node), entries.clone());
        assert_eq!(
            restored_renamed
                .get(&renamed_node.tag)
                .and_then(|item| item.latency_ms),
            Some(128)
        );

        let mut changed_node = node;
        changed_node.server = "new-edge.example.com".to_string();
        assert!(restore_cached_latencies(&[changed_node], entries).is_empty());
    }

    #[test]
    fn latency_results_sort_success_before_pending_and_failure() {
        let mut nodes = proxy_core::parse_subscription(
            "trojan://password@slow.example.com:443#Slow\n\
             trojan://password@pending.example.com:443#Pending\n\
             trojan://password@fast.example.com:443#Fast\n\
             trojan://password@failed.example.com:443#Failed",
        )
        .nodes;
        let mut members = nodes
            .drain(..)
            .map(|node| ProxyGroupMember {
                tag: node.tag.clone(),
                node: Some(node),
                nested_kind: None,
            })
            .collect::<Vec<_>>();
        let mut results = HashMap::new();
        results.insert(
            members[0].tag.clone(),
            NodeLatency {
                tag: members[0].tag.clone(),
                latency_ms: Some(300),
                error: None,
            },
        );
        results.insert(
            members[2].tag.clone(),
            NodeLatency {
                tag: members[2].tag.clone(),
                latency_ms: Some(80),
                error: None,
            },
        );
        results.insert(
            members[3].tag.clone(),
            NodeLatency {
                tag: members[3].tag.clone(),
                latency_ms: None,
                error: Some("timeout".to_string()),
            },
        );

        sort_proxy_group_members(&mut members, &results);

        let names = members
            .iter()
            .filter_map(|member| member.node.as_ref().map(|node| node.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["Fast", "Slow", "Pending", "Failed"]);
    }

    #[test]
    fn formats_traffic_rates_for_display() {
        assert_eq!(format_data_rate(0), "0 B/s");
        assert_eq!(format_data_rate(1_536), "1.5 KiB/s");
        assert_eq!(format_data_amount(2 * 1024 * 1024), "2.0 MiB");
        assert_eq!(traffic_bar_height(0, 100), "2px");
        assert_eq!(traffic_bar_height(100, 100), "88%");
        assert_eq!(format_uptime(0), "00:00");
        assert_eq!(format_uptime(75), "01:15");
        assert_eq!(format_uptime(3_725), "1:02:05");
    }

    #[test]
    fn saves_normalized_custom_rules_and_rejects_duplicates() {
        let mut rules = Vec::new();
        assert_eq!(
            save_custom_rule(
                &mut rules,
                None,
                CustomRuleMatch::DomainSuffix,
                CustomRuleAction::Proxy,
                "*.Example.COM.",
            ),
            Ok(())
        );
        assert_eq!(rules[0].id, 1);
        assert_eq!(rules[0].value, "example.com");

        assert_eq!(
            save_custom_rule(
                &mut rules,
                None,
                CustomRuleMatch::DomainSuffix,
                CustomRuleAction::Direct,
                "example.com",
            ),
            Err("已存在相同的匹配规则".to_string())
        );
    }

    #[test]
    fn mobile_rule_options_hide_process_matches() {
        let options = custom_rule_match_options(false);
        let values = options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            vec!["domain", "domain_suffix", "domain_keyword", "ip_cidr"]
        );
        assert!(!values.contains(&"process_name"));
        assert!(!values.contains(&"process_path"));
        assert!(!custom_rule_visible(CustomRuleMatch::ProcessName, false));
        assert!(custom_rule_visible(CustomRuleMatch::Domain, false));
    }

    #[test]
    fn script_override_excludes_custom_rules_from_connection_config() {
        let rules = vec![CustomRule {
            id: 1,
            enabled: true,
            match_type: CustomRuleMatch::Domain,
            value: "example.com".to_string(),
            action: CustomRuleAction::Proxy,
        }];

        assert!(connection_custom_rules(
            true,
            "function main(config) { return config; }",
            rules.clone()
        )
        .is_empty());
        assert_eq!(connection_custom_rules(false, "", rules.clone()), rules);
    }

    #[test]
    fn mobile_connection_excludes_preserved_process_rules() {
        let rules = vec![
            CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::Domain,
                value: "example.com".to_string(),
                action: CustomRuleAction::Proxy,
            },
            CustomRule {
                id: 2,
                enabled: true,
                match_type: CustomRuleMatch::ProcessName,
                value: "Telegram".to_string(),
                action: CustomRuleAction::Block,
            },
        ];

        let filtered = connection_custom_rules_for_platform(false, "", rules, false);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].match_type, CustomRuleMatch::Domain);
    }

    #[test]
    fn editing_and_moving_custom_rules_preserves_state() {
        let mut rules = vec![
            CustomRule {
                id: 1,
                enabled: false,
                match_type: CustomRuleMatch::Domain,
                value: "one.example".to_string(),
                action: CustomRuleAction::Direct,
            },
            CustomRule {
                id: 2,
                enabled: true,
                match_type: CustomRuleMatch::IpCidr,
                value: "203.0.113.0/24".to_string(),
                action: CustomRuleAction::Block,
            },
        ];

        assert!(save_custom_rule(
            &mut rules,
            Some(1),
            CustomRuleMatch::DomainKeyword,
            CustomRuleAction::Proxy,
            "Media",
        )
        .is_ok());
        assert!(!rules[0].enabled);
        assert_eq!(rules[0].value, "media");

        move_custom_rule(&mut rules, 2, -1, true);
        assert_eq!(
            rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
            vec![2, 1]
        );
        move_custom_rule(&mut rules, 2, -1, true);
        assert_eq!(
            rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn moving_visible_mobile_rules_preserves_hidden_process_rules() {
        let mut rules = vec![
            CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::Domain,
                value: "one.example".to_string(),
                action: CustomRuleAction::Direct,
            },
            CustomRule {
                id: 2,
                enabled: true,
                match_type: CustomRuleMatch::ProcessName,
                value: "Telegram".to_string(),
                action: CustomRuleAction::Proxy,
            },
            CustomRule {
                id: 3,
                enabled: true,
                match_type: CustomRuleMatch::Domain,
                value: "three.example".to_string(),
                action: CustomRuleAction::Block,
            },
        ];

        move_custom_rule(&mut rules, 3, -1, false);
        assert_eq!(
            rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
    }

    fn route_entry(sequence: u64) -> CoreLogEntry {
        CoreLogEntry {
            sequence,
            timestamp: "12:00:00".to_string(),
            level: "info".to_string(),
            message: "outbound/proxy[AI]: connection to example.com:443".to_string(),
            route: Some(RouteLogDetail {
                decision: RouteDecision::Proxy,
                target: "example.com:443".to_string(),
                host: "example.com".to_string(),
                port: Some(443),
                target_kind: RouteTargetKind::Domain,
                outbound_type: "proxy".to_string(),
                outbound_tag: "AI".to_string(),
                outbound_chain: vec!["proxy".to_string(), "AI".to_string()],
                source_ip: Some("127.0.0.1".to_string()),
            }),
        }
    }

    #[test]
    fn csv_export_keeps_header_and_route_columns() {
        let rows = log_export_rows(&[
            route_entry(1),
            CoreLogEntry {
                sequence: 2,
                timestamp: "12:00:01".to_string(),
                level: "warn".to_string(),
                message: "unsupported config field".to_string(),
                route: None,
            },
        ]);
        assert_eq!(rows[0][0], "时间");
        assert_eq!(rows[0].len(), rows[1].len());
        // Newest first, mirroring the on-screen order.
        assert_eq!(rows[1][0], "12:00:01");
        assert_eq!(rows[1][2], "-");
        assert_eq!(rows[2][2], "代理");
        assert_eq!(rows[2][3], "example.com");
        // The chain column keeps only the group hops; the outbound itself is
        // exported separately, so "AI" must not be repeated here.
        assert_eq!(rows[2][8], "proxy");
        assert_eq!(rows[2][7], "AI");
    }

    #[test]
    fn diagnostic_report_lists_only_known_state() {
        let report = build_diagnostic_report(
            "running".to_string(),
            Some("1.13.14".to_string()),
            "macos",
            TunnelMode::Rule,
            true,
            false,
            3,
            2,
            10,
            1,
        );
        assert!(report.contains("内核版本: 1.13.14"));
        assert!(report.contains("TUN: 开启 · 局域网: 关闭 (本地代理 7890)"));
        assert!(report.contains("节点: 3 · 订阅: 2"));
        assert!(!report.contains("{"));
    }
}
