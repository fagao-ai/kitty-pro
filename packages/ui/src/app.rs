use api::{
    CoreLogEntry, CoreTraffic, NodeLatency, RouteDecision, RouteLogDetail, RouteTargetKind,
    SystemProxyStatus,
};
use dioxus::prelude::*;
use dioxus_free_icons::icons::ld_icons::{
    LdActivity, LdArrowDown, LdArrowUp, LdBan, LdCheck, LdChevronDown, LdChevronRight,
    LdCircleAlert, LdCircleCheck, LdClock3, LdGauge, LdGlobe, LdInfo, LdLanguages, LdListFilter,
    LdMoon, LdNetwork, LdPause, LdPencil, LdPlay, LdPlus, LdPower, LdRadioTower, LdRefreshCw,
    LdRoute, LdSave, LdScrollText, LdSearch, LdServer, LdSettings, LdShieldCheck, LdSun, LdTrash2,
    LdWifi, LdX, LdZap,
};
use dioxus_free_icons::Icon;
use proxy_core::{
    validate_custom_rules, AppProfile, ConnectionRequest, CustomRule, CustomRuleAction,
    CustomRuleMatch, ParseReport, ProxyGroup, ProxyGroupKind, ProxyNode, ProxyProtocol,
    Subscription, TunnelMode, MAX_CUSTOM_RULES,
};
use std::collections::HashMap;

use crate::APP_CSS;

const BRAND_ICON_SVG: &str = include_str!("../assets/kitty-pro.svg");
const ANDROID_VPN_WAITING_NOTICE: &str = "正在等待 Android VPN 授权或启动服务";
const RULE_SET_UPDATE_CHECK_SECS: u64 = 6 * 60 * 60;
const TOAST_DISMISS_MILLIS: u64 = 4_500;
const LATENCY_CACHE_KEY: &str = "kitty-pro.node-latency.v1";

type LatencyCacheEntry = (String, String, u64);

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
}

#[derive(Clone, Copy)]
struct ToastManager {
    messages: Signal<Vec<ToastMessage>>,
    next_id: Signal<u64>,
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
            stored.push(ToastMessage { id, kind, message });
        }
        spawn(async move {
            wait_for_toast_dismiss().await;
            messages.write().retain(|toast| toast.id != id);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFilter {
    Routes,
    All,
    Direct,
    Proxy,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshTarget {
    One(u64),
    All,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrafficDisplay {
    upload_bytes_per_second: u64,
    download_bytes_per_second: u64,
    upload_total: u64,
    download_total: u64,
    active_connections: u32,
    history: Vec<u64>,
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
async fn wait_for_toast_dismiss() {
    gloo_timers::future::TimeoutFuture::new(TOAST_DISMISS_MILLIS as u32).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn wait_for_toast_dismiss() {
    tokio::time::sleep(std::time::Duration::from_millis(TOAST_DISMISS_MILLIS)).await;
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
}

#[component]
pub fn ProxyApp(platform: String) -> Element {
    let active_view = use_signal(|| AppView::Overview);
    let mut dark_mode = use_signal(|| false);
    let mut connected = use_signal(|| false);
    let core_busy = use_signal(|| false);
    let mut core_restarting = use_signal(|| false);
    let mut core_state = use_signal(|| "checking".to_string());
    let mut core_version = use_signal(|| None::<String>);
    let mut core_note = use_signal(|| None::<String>);
    let toast = ToastManager {
        messages: use_signal(Vec::<ToastMessage>::new),
        next_id: use_signal(|| 0),
    };
    use_context_provider(|| toast);
    let mut nodes = use_signal(Vec::<ProxyNode>::new);
    let mut selected_tag = use_signal(String::new);
    let mut subscriptions = use_signal(Vec::<Subscription>::new);
    let mut custom_rules = use_signal(Vec::<CustomRule>::new);
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
    let mut core_log_cursor = use_signal(|| 0_u64);
    let mut log_poll_generation = use_signal(|| 0_u64);
    let log_collection_paused = use_signal(|| false);
    let mut profile_loaded = use_signal(|| false);
    let mut system_proxy = use_signal(|| SystemProxyLoadState::Loading);
    let system_proxy_busy = use_signal(|| false);

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
            let mut history = traffic.peek().history.clone();
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
                    let combined_rate =
                        upload_bytes_per_second.saturating_add(download_bytes_per_second);
                    history.push(combined_rate);
                    if history.len() > 12 {
                        history.remove(0);
                    }
                    traffic.set(TrafficDisplay {
                        upload_bytes_per_second,
                        download_bytes_per_second,
                        upload_total: current.upload_total,
                        download_total: current.download_total,
                        active_connections: current.active_connections,
                        history: history.clone(),
                    });
                    previous = Some(current);
                }
                wait_for_traffic_tick().await;
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
            mode: tunnel_mode(),
            tun: tun_enabled(),
            allow_lan: allow_lan(),
            custom_rules: connection_custom_rules(
                config_script_enabled(),
                &config_script(),
                custom_rules(),
            ),
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
                Brand {}
                nav { class: "primary-nav", aria_label: "主导航",
                    NavItem { view: AppView::Overview, active_view }
                    NavItem { view: AppView::Nodes, active_view }
                    NavItem { view: AppView::Subscriptions, active_view }
                    NavItem { view: AppView::Rules, active_view }
                    NavItem { view: AppView::Logs, active_view }
                    NavItem { view: AppView::Settings, active_view }
                }
                div { class: "sidebar-footer",
                    div { class: "core-chip",
                        span { class: status_dot_class(&core_state()) }
                        div {
                            strong { {core_state_label(&core_state())} }
                            small { {core_version().unwrap_or_else(|| platform.clone())} }
                        }
                        button {
                            class: "core-restart-button",
                            title: if core_restarting() { "正在重启内核" } else { "重启内核" },
                            aria_label: "重启内核",
                            aria_busy: core_restarting(),
                            disabled: !connected() || core_busy() || core_restarting(),
                            onclick: move |_| async move {
                                core_restarting.set(true);
                                let request = ConnectionRequest {
                                    nodes: nodes(),
                                    selected_tag: selected_tag(),
                                    mode: tunnel_mode(),
                                    tun: tun_enabled(),
                                    allow_lan: allow_lan(),
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
                                };
                                match api::restart_core(request).await {
                                    Ok(status) => {
                                        let is_running = status.state == "running";
                                        connected.set(is_running);
                                        core_state.set(status.state);
                                        core_version.set(status.version);
                                        core_note.set(status.note);
                                        if is_running {
                                            toast.success("sing-box 内核已重启");
                                        } else {
                                            toast.info(ANDROID_VPN_WAITING_NOTICE);
                                        }
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
                                    }
                                }
                                core_restarting.set(false);
                            },
                            if core_restarting() {
                                span { class: "spinner" }
                            } else {
                                Icon { icon: LdRefreshCw, width: 15, height: 15 }
                            }
                        }
                    }
                }
            }

            main { class: "main-content",
                header { class: "topbar",
                    div {
                        p { class: "eyebrow", "KITTY PRO" }
                        h1 { "{current_view.title()}" }
                    }
                    div { class: "topbar-actions",
                        button {
                            class: "icon-button glass-control mobile-core-restart-button",
                            title: if core_restarting() { "正在重启内核" } else { "重启内核" },
                            aria_label: "重启内核",
                            aria_busy: core_restarting(),
                            disabled: !connected() || core_busy() || core_restarting(),
                            onclick: move |_| async move {
                                core_restarting.set(true);
                                let request = ConnectionRequest {
                                    nodes: nodes(),
                                    selected_tag: selected_tag(),
                                    mode: tunnel_mode(),
                                    tun: tun_enabled(),
                                    allow_lan: allow_lan(),
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
                                };
                                match api::restart_core(request).await {
                                    Ok(status) => {
                                        let is_running = status.state == "running";
                                        connected.set(is_running);
                                        core_state.set(status.state);
                                        core_version.set(status.version);
                                        core_note.set(status.note);
                                        if is_running {
                                            toast.success("sing-box 内核已重启");
                                        } else {
                                            toast.info(ANDROID_VPN_WAITING_NOTICE);
                                        }
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
                                    }
                                }
                                core_restarting.set(false);
                            },
                            if core_restarting() {
                                span { class: "spinner" }
                            } else {
                                Icon { icon: LdRefreshCw, width: 18, height: 18 }
                            }
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
                    }
                }

                match current_view {
                    AppView::Overview => rsx! {
                        OverviewView {
                            nodes,
                            selected_tag,
                            connected,
                            core_busy,
                            core_restarting,
                            core_state,
                            core_note,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            custom_rules,
                            connection_allowed,
                            latency_results,
                            traffic,
                            config_script_enabled,
                            config_script,
                            group_selections,
                            system_proxy,
                            system_proxy_busy,
                        }
                    },
                    AppView::Nodes => rsx! {
                        NodesView {
                            groups: proxy_groups,
                            groups_loading: proxy_groups_loading,
                            groups_error: proxy_groups_error,
                            nodes: nodes(),
                            all_count: nodes().len(),
                            group_selections,
                            connected,
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
                        }
                    },
                    AppView::Rules => rsx! {
                        RulesView {
                            rules: custom_rules,
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
                        }
                    },
                    AppView::Settings => rsx! {
                        SettingsView {
                            platform: platform.clone(),
                            core_state,
                            core_version,
                            core_note,
                            connected,
                            core_restarting,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            dark_mode,
                            nodes,
                            selected_tag,
                            custom_rules,
                            config_script_enabled,
                            config_script,
                            group_selections,
                        }
                    },
                }
            }

            nav { class: "mobile-nav glass-surface", aria_label: "移动端导航",
                NavItem { view: AppView::Overview, active_view }
                NavItem { view: AppView::Nodes, active_view }
                NavItem { view: AppView::Subscriptions, active_view }
                NavItem { view: AppView::Rules, active_view }
                NavItem { view: AppView::Logs, active_view }
                NavItem { view: AppView::Settings, active_view }
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
                                    placeholder: "订阅地址，或 http://host:port#节点名称",
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
                                disabled: import_busy() || import_source().trim().is_empty(),
                                onclick: move |_| async move {
                                    import_busy.set(true);
                                    import_error.set(None);
                                    let source = import_source();
                                    match api::preview_subscription(source.clone()).await {
                                        Ok(report) if !report.nodes.is_empty() => {
                                            let count = report.nodes.len();
                                            let rejected = report.rejected.len();
                                            let id = next_subscription_id(&subscriptions());
                                            let parsed_nodes = namespace_nodes(id, report.nodes);
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
                                                rejected_count: rejected,
                                            });
                                            active_subscription_id.set(Some(id));
                                            let active_nodes = collect_subscription_nodes(
                                                &subscriptions(),
                                                Some(id),
                                            );
                                            let next_tag = select_available_tag(&active_nodes, "");
                                            nodes.set(active_nodes);
                                            selected_tag.set(next_tag);
                                            toast.success(format!("已导入并切换到 {name}，共 {count} 个节点"));
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
fn Brand() -> Element {
    rsx! {
        div { class: "brand",
            div {
                class: "brand-mark",
                aria_hidden: "true",
                dangerous_inner_html: BRAND_ICON_SVG,
            }
            div {
                strong { "Kitty Pro" }
                small { "SING-BOX CLIENT" }
            }
        }
    }
}

#[component]
fn NavItem(view: AppView, mut active_view: Signal<AppView>) -> Element {
    let active = active_view() == view;
    rsx! {
        button {
            class: if active { "nav-item active" } else { "nav-item" },
            aria_current: if active { "page" },
            onclick: move |_| active_view.set(view),
            match view {
                AppView::Overview => rsx! { Icon { icon: LdGauge, width: 20, height: 20 } },
                AppView::Nodes => rsx! { Icon { icon: LdServer, width: 20, height: 20 } },
                AppView::Subscriptions => rsx! { Icon { icon: LdRadioTower, width: 20, height: 20 } },
                AppView::Rules => rsx! { Icon { icon: LdListFilter, width: 20, height: 20 } },
                AppView::Logs => rsx! { Icon { icon: LdScrollText, width: 20, height: 20 } },
                AppView::Settings => rsx! { Icon { icon: LdSettings, width: 20, height: 20 } },
            }
            span { "{view.title()}" }
        }
    }
}

#[component]
fn OverviewView(
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    mut connected: Signal<bool>,
    mut core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    mut core_state: Signal<String>,
    mut core_note: Signal<Option<String>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    connection_allowed: bool,
    latency_results: Signal<HashMap<String, NodeLatency>>,
    traffic: Signal<TrafficDisplay>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    group_selections: Signal<HashMap<String, String>>,
    system_proxy: Signal<SystemProxyLoadState>,
    system_proxy_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let selected_node = nodes().into_iter().find(|node| node.tag == selected_tag());
    let is_connected = connected();
    let is_core_busy = core_busy();
    let is_core_restarting = core_restarting();
    let is_core_action_busy = is_core_busy || is_core_restarting;
    let status_title = if is_core_restarting {
        "正在重启"
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
    let chart_peak = traffic.history.iter().copied().max().unwrap_or(1).max(1);
    let chart_history = traffic.history.clone();

    rsx! {
        div { class: "overview-grid",
            section { class: if is_core_action_busy { "connection-panel glass-surface busy" } else if is_connected { "connection-panel glass-surface connected" } else { "connection-panel glass-surface" },
                div { class: "connection-copy",
                    div { class: "status-line", role: "status", aria_live: "polite",
                        span { class: if is_core_action_busy { "busy-dot" } else if is_connected { "live-dot" } else { "idle-dot" } }
                        span { "{status_title}" }
                    }
                    h2 { "{node_name}" }
                    p { "{note}" }
                    div { class: "connection-meta",
                        span {
                            Icon { icon: LdRoute, width: 15, height: 15 }
                            "{mode_name}"
                        }
                        span {
                            Icon { icon: LdWifi, width: 15, height: 15 }
                            if allow_lan() { "局域网代理 7890" } else { "本地代理 7890" }
                        }
                    }
                }
                div { class: "connection-controls",
                    button {
                        class: if is_core_action_busy && is_connected { "power-button active loading" } else if is_core_action_busy { "power-button loading" } else if is_connected { "power-button active" } else { "power-button" },
                        title: if is_core_restarting { "正在重启内核" } else if is_core_busy { "正在切换连接状态" } else if is_connected { "断开连接" } else { "建立连接" },
                        aria_label: if is_connected { "停止内核" } else { "启动内核" },
                        aria_busy: is_core_action_busy,
                        disabled: is_core_action_busy || (!is_connected && !connection_allowed),
                        onclick: move |_| async move {
                            let target = !connected();
                            core_busy.set(true);
                            let request = target.then(|| ConnectionRequest {
                                nodes: nodes(),
                                selected_tag: selected_tag(),
                                mode: tunnel_mode(),
                                tun: tun_enabled(),
                                allow_lan: allow_lan(),
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
                            match api::set_core_enabled(target, request).await {
                                Ok(status) => {
                                    let is_running = status.state == "running";
                                    connected.set(is_running);
                                    core_state.set(status.state);
                                    core_note.set(status.note);
                                    if !target {
                                        toast.success("连接已断开");
                                    } else if is_running {
                                        toast.success("sing-box 已启动");
                                    } else {
                                        toast.info(ANDROID_VPN_WAITING_NOTICE);
                                    }
                                }
                                Err(error) => toast.error(error.to_string()),
                            }
                            core_busy.set(false);
                        },
                        if is_core_action_busy {
                            span { class: "spinner large" }
                        } else {
                            Icon { icon: LdPower, width: 32, height: 32 }
                        }
                    }
                    SystemProxyToggle {
                        core_running: is_connected,
                        system_proxy,
                        system_proxy_busy,
                    }
                }
            }

            section { class: "traffic-panel glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "SESSION" }
                        h2 { "实时流量" }
                    }
                    Icon { icon: LdActivity, width: 20, height: 20 }
                }
                div { class: "traffic-values",
                    div {
                        span { class: "metric-icon download", Icon { icon: LdArrowDown, width: 16, height: 16 } }
                        p { "下载" }
                        strong {
                            title: "累计下载 {format_data_amount(traffic.download_total)}",
                            "{format_data_rate(traffic.download_bytes_per_second)}"
                        }
                        small { class: "traffic-total", "累计 {format_data_amount(traffic.download_total)}" }
                    }
                    div {
                        span { class: "metric-icon upload", Icon { icon: LdArrowUp, width: 16, height: 16 } }
                        p { "上传" }
                        strong {
                            title: "累计上传 {format_data_amount(traffic.upload_total)}",
                            "{format_data_rate(traffic.upload_bytes_per_second)}"
                        }
                        small { class: "traffic-total", "累计 {format_data_amount(traffic.upload_total)}" }
                    }
                }
                div { class: "traffic-chart",
                    for sample in chart_history {
                        span { style: "height: {traffic_chart_height(sample, chart_peak)}" }
                    }
                    if traffic.active_connections == 0 {
                        small { class: "traffic-status", "等待代理流量" }
                    } else {
                        small { class: "traffic-status", "{traffic.active_connections} 个活动连接" }
                    }
                }
            }
        }

        div { class: "stats-grid",
            MetricCard { label: "节点", value: nodes().len().to_string(), icon: "server" }
            MetricCard {
                label: "延迟",
                value: selected_latency
                    .map(|latency| format!("{latency} ms"))
                    .unwrap_or_else(|| "-- ms".to_string()),
                icon: "zap",
            }
            MetricCard { label: "运行时间", value: if connected() { "刚刚".to_string() } else { "--".to_string() }, icon: "clock" }
        }

        section { class: "list-section glass-surface",
            div { class: "section-heading",
                div {
                    p { class: "eyebrow", "PROXIES" }
                    h2 { "节点速览" }
                }
                span { class: "count-badge", "{nodes().len()}" }
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
    mut system_proxy: Signal<SystemProxyLoadState>,
    mut system_proxy_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let (proxy_status, proxy_loading, proxy_error) = match system_proxy() {
        SystemProxyLoadState::Loading => (None, true, None),
        SystemProxyLoadState::Ready(status) => (Some(status), false, None),
        SystemProxyLoadState::Failed(error) => (None, false, Some(error)),
    };
    let proxy_ready = proxy_status.is_some();
    let proxy_supported = proxy_status.as_ref().is_some_and(|status| status.supported);
    let proxy_enabled = proxy_status.as_ref().is_some_and(|status| status.enabled);
    let proxy_busy = system_proxy_busy();
    let proxy_label = if proxy_busy {
        "设置中"
    } else if proxy_loading {
        "读取中"
    } else if proxy_enabled {
        "已启用"
    } else if proxy_error.is_some() {
        "读取失败"
    } else if !proxy_supported {
        "不可用"
    } else {
        "未启用"
    };
    let proxy_detail = if proxy_busy {
        if proxy_enabled {
            "正在恢复启用前的系统代理设置".to_string()
        } else {
            "正在应用到系统网络服务".to_string()
        }
    } else {
        proxy_status
            .as_ref()
            .map(|status| status.detail.clone())
            .or(proxy_error)
            .unwrap_or_else(|| "正在读取系统代理状态".to_string())
    };

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
                    disabled: !proxy_ready || !proxy_supported || (!proxy_enabled && !core_running),
                    onchange: move |event| {
                        let enabled = event.checked();
                        async move {
                            system_proxy_busy.set(true);
                            match api::set_system_proxy(enabled).await {
                                Ok(status) => {
                                    system_proxy.set(SystemProxyLoadState::Ready(status));
                                    if enabled {
                                        toast.success("系统代理已启用");
                                    } else {
                                        toast.success("系统代理已恢复为启用前的设置");
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

fn traffic_chart_height(sample: u64, peak: u64) -> String {
    let percent = 12 + sample.saturating_mul(76).saturating_div(peak.max(1));
    format!("{}%", percent.min(88))
}

fn format_data_rate(bytes_per_second: u64) -> String {
    format!("{}/s", format_data_amount(bytes_per_second))
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

#[component]
fn MetricCard(label: String, value: String, icon: String) -> Element {
    rsx! {
        div { class: "metric-card glass-surface",
            span { class: "metric-symbol",
                if icon == "server" {
                    Icon { icon: LdNetwork, width: 19, height: 19 }
                } else if icon == "zap" {
                    Icon { icon: LdZap, width: 19, height: 19 }
                } else {
                    Icon { icon: LdClock3, width: 19, height: 19 }
                }
            }
            div {
                p { "{label}" }
                strong { "{value}" }
            }
        }
    }
}

#[component]
fn NodesView(
    groups: Vec<ProxyGroup>,
    groups_loading: bool,
    groups_error: Option<String>,
    nodes: Vec<ProxyNode>,
    all_count: usize,
    mut group_selections: Signal<HashMap<String, String>>,
    connected: Signal<bool>,
    mut search: Signal<String>,
    mut latency_results: Signal<HashMap<String, NodeLatency>>,
    mut latency_busy: Signal<bool>,
    mut import_open: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let active_group = use_signal(String::new);
    let requested_group = active_group();
    let current_group = groups
        .iter()
        .find(|group| group.tag == requested_group)
        .or_else(|| groups.first())
        .cloned();
    let current_group_tag = current_group
        .as_ref()
        .map(|group| group.tag.clone())
        .unwrap_or_default();
    let current_group_kind = current_group.as_ref().map(|group| group.kind);
    let current_selected = current_group.as_ref().map(|group| {
        group_selections()
            .get(&group.tag)
            .filter(|selected| group.outbounds.iter().any(|item| item == *selected))
            .cloned()
            .unwrap_or_else(|| group.selected.clone())
    });
    let needle = search().trim().to_ascii_lowercase();
    let latency_snapshot = latency_results();
    let mut members = current_group
        .as_ref()
        .map(|group| {
            group
                .outbounds
                .iter()
                .filter_map(|tag| {
                    let node = nodes.iter().find(|node| node.tag == *tag).cloned();
                    let nested_kind = groups
                        .iter()
                        .find(|candidate| candidate.tag == *tag)
                        .map(|candidate| candidate.kind);
                    let display_name = node.as_ref().map(|node| node.name.as_str()).unwrap_or(tag);
                    (needle.is_empty()
                        || display_name.to_ascii_lowercase().contains(&needle)
                        || tag.to_ascii_lowercase().contains(&needle)
                        || node.as_ref().is_some_and(|node| {
                            node.server.to_ascii_lowercase().contains(&needle)
                                || node.protocol.label().to_ascii_lowercase().contains(&needle)
                        }))
                    .then(|| ProxyGroupMember {
                        tag: tag.clone(),
                        node,
                        nested_kind,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    sort_proxy_group_members(&mut members, &latency_snapshot);
    let current_selected_label = current_selected
        .as_deref()
        .map(|tag| outbound_display_name(tag, &nodes))
        .unwrap_or_else(|| "未选择".to_string());

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
                div { class: "proxy-groups-layout",
                    aside { class: "proxy-group-list", aria_label: "代理组",
                        div { class: "proxy-group-list-heading",
                            p { class: "eyebrow", "PROXY GROUPS" }
                            strong { "{groups.len()} 个分组" }
                        }
                        for group in groups.clone() {
                            ProxyGroupTab {
                                group,
                                nodes: nodes.clone(),
                                active: current_group_tag.clone(),
                                group_selections,
                                active_group,
                            }
                        }
                    }
                    div { class: "proxy-group-members",
                        div { class: "workspace-toolbar proxy-group-toolbar",
                            div {
                                p { class: "eyebrow", {current_group_kind.map(proxy_group_kind_label).unwrap_or("PROXY GROUP")} }
                                h2 { "{current_group_tag}" }
                                span {
                                    if current_group_kind == Some(ProxyGroupKind::UrlTest) {
                                        "自动测速 · {members.len()} 个候选"
                                    } else {
                                        "当前：{current_selected_label}"
                                    }
                                }
                            }
                            div { class: "toolbar-controls subscription-actions",
                                label { class: "search-field",
                                    Icon { icon: LdSearch, width: 17, height: 17 }
                                    input {
                                        value: search,
                                        placeholder: "搜索当前分组",
                                        oninput: move |event| search.set(event.value()),
                                    }
                                }
                                button {
                                    class: "icon-button glass-control",
                                    title: if latency_busy() { "正在刷新延迟" } else { "刷新延迟" },
                                    disabled: latency_busy() || nodes.is_empty(),
                                    onclick: move |_| {
                                        let probe_nodes = nodes.clone();
                                        let cache_nodes = nodes.clone();
                                        async move {
                                            latency_busy.set(true);
                                            let mut failures = 0;
                                            let session_id = match api::start_node_latency(probe_nodes).await {
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
                                            persist_latency_cache(
                                                &cache_nodes,
                                                &latency_results.peek(),
                                            )
                                            .await;
                                            latency_busy.set(false);
                                        }
                                    },
                                    if latency_busy() {
                                        span { class: "spinner" }
                                    } else {
                                        Icon { icon: LdRefreshCw, width: 18, height: 18 }
                                    }
                                }
                            }
                        }
                        if members.is_empty() {
                            div { class: "large-empty group-empty",
                                Icon { icon: LdSearch, width: 28, height: 28 }
                                strong { "当前分组没有匹配项" }
                            }
                        } else {
                            div { class: "node-list proxy-member-list",
                                for member in members {
                                    ProxyGroupMemberRow {
                                        group_tag: current_group_tag.clone(),
                                        group_kind: current_group_kind.unwrap_or(ProxyGroupKind::Selector),
                                        member,
                                        selected: current_selected.clone().unwrap_or_default(),
                                        connected,
                                        group_selections,
                                        latency_results,
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

#[derive(Debug, Clone, PartialEq)]
struct ProxyGroupMember {
    tag: String,
    node: Option<ProxyNode>,
    nested_kind: Option<ProxyGroupKind>,
}

#[component]
fn ProxyGroupTab(
    group: ProxyGroup,
    nodes: Vec<ProxyNode>,
    active: String,
    group_selections: Signal<HashMap<String, String>>,
    mut active_group: Signal<String>,
) -> Element {
    let tag = group.tag.clone();
    let selected = group_selections()
        .get(&group.tag)
        .filter(|selected| group.outbounds.iter().any(|item| item == *selected))
        .cloned()
        .unwrap_or_else(|| group.selected.clone());
    let detail = if group.kind == ProxyGroupKind::UrlTest {
        format!("自动测速 · {} 项", group.outbounds.len())
    } else {
        format!("当前：{}", outbound_display_name(&selected, &nodes))
    };
    rsx! {
        button {
            class: if active == group.tag { "proxy-group-tab active" } else { "proxy-group-tab" },
            onclick: move |_| active_group.set(tag.clone()),
            span { class: if group.kind == ProxyGroupKind::UrlTest { "group-kind-mark auto" } else { "group-kind-mark" },
                if group.kind == ProxyGroupKind::UrlTest {
                    Icon { icon: LdGauge, width: 17, height: 17 }
                } else {
                    Icon { icon: LdRoute, width: 17, height: 17 }
                }
            }
            span { class: "proxy-group-tab-copy",
                strong { "{group.tag}" }
                small { "{detail}" }
            }
            Icon { icon: LdChevronRight, width: 16, height: 16 }
        }
    }
}

#[component]
fn ProxyGroupMemberRow(
    group_tag: String,
    group_kind: ProxyGroupKind,
    member: ProxyGroupMember,
    selected: String,
    connected: Signal<bool>,
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
            disabled: !selectable,
            onclick: move |_| {
                let group = target_group.clone();
                let outbound = member_tag.clone();
                async move {
                    if connected() {
                        if let Err(error) = api::select_proxy_group(group.clone(), outbound.clone()).await {
                            toast.error(format!("切换 {group} 失败: {error}"));
                            return;
                        }
                    }
                    group_selections.write().insert(group.clone(), outbound.clone());
                    toast.success(format!("{group} 已切换到 {outbound}"));
                }
            },
            if let Some(node) = member.node {
                span { class: protocol_class(node.protocol), "{protocol_abbreviation(node.protocol)}" }
                span { class: "node-main",
                    strong { "{node.name}" }
                    small { "{node.endpoint()}" }
                }
                span { class: "protocol-label", "{node.protocol.label()}" }
                span { class: latency_class, title: latency_title, "{latency_label}" }
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
                small { "{node.endpoint()}" }
            }
            span { class: "protocol-label", "{node.protocol.label()}" }
            span { class: latency_class, title: latency_title, "{latency_label}" }
            if selected {
                span { class: "selected-check", Icon { icon: LdCircleCheck, width: 19, height: 19 } }
            } else {
                span { class: "row-chevron", Icon { icon: LdChevronRight, width: 18, height: 18 } }
            }
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
                div { class: "toolbar-controls",
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
) -> Element {
    let toast = use_context::<ToastManager>();
    let subscription_id = subscription.id;
    let subscription_source = subscription.source.clone();
    let display_source = source_label(&subscription.source);
    let refreshing = refresh_busy() == Some(RefreshTarget::One(subscription_id));
    let active = active_subscription_id() == Some(subscription_id);

    rsx! {
        div { class: if active { "subscription-row active" } else { "subscription-row" },
            span { class: "subscription-icon", Icon { icon: LdRadioTower, width: 20, height: 20 } }
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
                disabled: active || refresh_busy().is_some(),
                onclick: move |_| {
                    active_subscription_id.set(Some(subscription_id));
                    let active_nodes =
                        collect_subscription_nodes(&subscriptions(), Some(subscription_id));
                    let next_tag = select_available_tag(&active_nodes, "");
                    nodes.set(active_nodes);
                    selected_tag.set(next_tag);
                    toast.success("已切换订阅");
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

#[component]
fn RulesView(
    mut rules: Signal<Vec<CustomRule>>,
    connected: Signal<bool>,
    tunnel_mode: Signal<TunnelMode>,
    mut rule_sets_busy: Signal<bool>,
    config_script_enabled: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let mut editor_open = use_signal(|| false);
    let mut editing_id = use_signal(|| None::<u64>);
    let mut draft_match = use_signal(|| CustomRuleMatch::DomainSuffix);
    let mut draft_action = use_signal(|| CustomRuleAction::Direct);
    let mut draft_value = use_signal(String::new);
    let mut editor_error = use_signal(|| None::<String>);
    let stored = rules();
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
                    }
                    p { "{enabled_count} 条启用 · {total} 条规则" }
                }
                div { class: "toolbar-controls rules-toolbar-actions",
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
                        disabled: override_active || total >= MAX_CUSTOM_RULES,
                        onclick: move |_| open_new_editor(),
                        Icon { icon: LdPlus, width: 17, height: 17 }
                        span { "添加规则" }
                    }
                }
            }

            if override_active {
                div { class: "rule-override-notice", role: "status",
                    Icon { icon: LdScrollText, width: 17, height: 17 }
                    div {
                        strong { "JavaScript 配置覆写已启用" }
                        span { "自定义分流规则暂不可用" }
                    }
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
                                options: vec![
                                    RuleSelectOption::new("domain", "精确域名"),
                                    RuleSelectOption::new("domain_suffix", "域名后缀"),
                                    RuleSelectOption::new("domain_keyword", "域名关键字"),
                                    RuleSelectOption::new("ip_cidr", "IP CIDR"),
                                ],
                                on_select: move |value: String| {
                                    draft_match.set(parse_custom_rule_match(&value));
                                    editor_error.set(None);
                                },
                            }
                        }
                        label { class: "rule-field rule-value-field",
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

            if stored.is_empty() {
                div { class: "large-empty rules-empty",
                    span { class: "empty-icon", Icon { icon: LdListFilter, width: 28, height: 28 } }
                    strong { "暂无自定义规则" }
                }
            } else {
                div { class: "rule-list",
                    for (index, rule) in stored.into_iter().enumerate() {
                        RuleListItem {
                            key: "{rule.id}",
                            rule,
                            index,
                            total,
                            rules,
                            connected: connected(),
                            locked: override_active,
                            on_edit,
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
                        move_custom_rule(&mut rules.write(), rule_id, -1);
                        notify_rule_change(connected, toast);
                    },
                    Icon { icon: LdArrowUp, width: 16, height: 16 }
                }
                button {
                    class: "icon-button",
                    title: "下移",
                    disabled: locked || index + 1 >= total,
                    onclick: move |_| {
                        move_custom_rule(&mut rules.write(), rule_id, 1);
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
    if config_script_enabled && !config_script.trim().is_empty() {
        Vec::new()
    } else {
        custom_rules
    }
}

fn next_custom_rule_id(rules: &[CustomRule]) -> Option<u64> {
    (1..=(rules.len() as u64 + 1)).find(|id| rules.iter().all(|rule| rule.id != *id))
}

fn move_custom_rule(rules: &mut [CustomRule], id: u64, offset: isize) {
    let Some(index) = rules.iter().position(|rule| rule.id == id) else {
        return;
    };
    let Some(target) = index.checked_add_signed(offset) else {
        return;
    };
    if target < rules.len() {
        rules.swap(index, target);
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
    }
}

fn parse_custom_rule_match(value: &str) -> CustomRuleMatch {
    match value {
        "domain" => CustomRuleMatch::Domain,
        "domain_keyword" => CustomRuleMatch::DomainKeyword,
        "ip_cidr" => CustomRuleMatch::IpCidr,
        _ => CustomRuleMatch::DomainSuffix,
    }
}

fn custom_rule_match_label(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "精确域名",
        CustomRuleMatch::DomainSuffix => "域名后缀",
        CustomRuleMatch::DomainKeyword => "域名关键字",
        CustomRuleMatch::IpCidr => "IP CIDR",
    }
}

fn custom_rule_placeholder(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "api.example.com",
        CustomRuleMatch::DomainSuffix => "example.com",
        CustomRuleMatch::DomainKeyword => "google",
        CustomRuleMatch::IpCidr => "203.0.113.0/24",
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
) -> Element {
    let mut filter = use_signal(|| LogFilter::Routes);
    let mut search = use_signal(String::new);
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
    let query = search().trim().to_ascii_lowercase();
    let visible_logs: Vec<CoreLogEntry> = stored
        .iter()
        .rev()
        .filter(|entry| log_matches_filter(entry, filter()))
        .filter(|entry| log_matches_search(entry, &query))
        .cloned()
        .collect();
    let visible_count = visible_logs.len();

    rsx! {
        section { class: "workspace-section log-workspace glass-surface",
            div { class: "workspace-toolbar log-toolbar",
                div {
                    p { class: "eyebrow", "CORE LOGS" }
                    h2 { "连接日志" }
                    span { "{route_count} 条路由 · {stored.len()} 条记录" }
                }
                div { class: "toolbar-controls log-toolbar-actions",
                    button {
                        class: if !connected() {
                            "log-collection-toggle stopped"
                        } else if collection_paused() {
                            "log-collection-toggle paused"
                        } else {
                            "log-collection-toggle collecting"
                        },
                        title: if !connected() {
                            "内核未运行"
                        } else if collection_paused() {
                            "继续采集日志"
                        } else {
                            "暂停采集日志"
                        },
                        aria_pressed: collection_paused(),
                        disabled: !connected(),
                        onclick: move |_| {
                            let paused = collection_paused();
                            collection_paused.set(!paused);
                        },
                        if connected() && !collection_paused() {
                            Icon { icon: LdPause, width: 14, height: 14 }
                            span { class: "log-collection-label", "实时采集" }
                        } else if connected() {
                            Icon { icon: LdPlay, width: 14, height: 14 }
                            span { class: "log-collection-label", "已暂停" }
                        } else {
                            Icon { icon: LdPause, width: 14, height: 14 }
                            span { class: "log-collection-label", "已停止" }
                        }
                    }
                    button {
                        class: "icon-button glass-control danger",
                        title: "清空日志",
                        disabled: stored.is_empty(),
                        onclick: move |_| logs.write().clear(),
                        Icon { icon: LdTrash2, width: 17, height: 17 }
                    }
                }
            }
            div { class: "log-controls",
                div { class: "log-segmented-control", aria_label: "日志筛选",
                    button {
                        class: if filter() == LogFilter::Routes { "active" } else { "" },
                        onclick: move |_| filter.set(LogFilter::Routes),
                        "路由 {route_count}"
                    }
                    button {
                        class: if filter() == LogFilter::All { "active" } else { "" },
                        onclick: move |_| filter.set(LogFilter::All),
                        "全部 {stored.len()}"
                    }
                    button {
                        class: if filter() == LogFilter::Direct { "active" } else { "" },
                        onclick: move |_| filter.set(LogFilter::Direct),
                        "直连 {direct_count}"
                    }
                    button {
                        class: if filter() == LogFilter::Proxy { "active" } else { "" },
                        onclick: move |_| filter.set(LogFilter::Proxy),
                        "代理 {proxy_count}"
                    }
                    button {
                        class: if filter() == LogFilter::Block { "active" } else { "" },
                        onclick: move |_| filter.set(LogFilter::Block),
                        "拦截 {block_count}"
                    }
                }
                label { class: "search-field log-search",
                    Icon { icon: LdSearch, width: 17, height: 17 }
                    input {
                        value: search,
                        placeholder: "搜索域名、IP、节点",
                        oninput: move |event| search.set(event.value()),
                    }
                }
            }
            if visible_logs.is_empty() {
                div { class: "large-empty log-empty",
                    span { class: "empty-icon", Icon { icon: LdScrollText, width: 27, height: 27 } }
                    strong {
                        if connected() { "暂无匹配日志" } else { "连接后显示日志" }
                    }
                }
            } else {
                div { class: "log-list", aria_label: "内核日志",
                    div { class: "log-table-header",
                        span { "时间 / 来源" }
                        span { "路由" }
                        span { "目标" }
                        span { "出口" }
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
                    if let Some(group) = route_primary_group(&route) {
                        small {
                            class: match route.decision {
                                RouteDecision::Direct => "route-decision-group direct",
                                RouteDecision::Proxy => "route-decision-group proxy",
                                RouteDecision::Block => "route-decision-group block",
                            },
                            title: route_group_chain(&route),
                            "{group}"
                        }
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
        LogFilter::All => true,
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

fn route_primary_group(route: &RouteLogDetail) -> Option<&str> {
    route
        .outbound_chain
        .iter()
        .find(|tag| {
            tag.as_str() != route.outbound_tag && !matches!(tag.as_str(), "direct" | "block")
        })
        .map(String::as_str)
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
fn SettingsView(
    platform: String,
    mut core_state: Signal<String>,
    mut core_version: Signal<Option<String>>,
    mut core_note: Signal<Option<String>>,
    mut connected: Signal<bool>,
    mut core_restarting: Signal<bool>,
    mut tunnel_mode: Signal<TunnelMode>,
    mut tun_enabled: Signal<bool>,
    mut allow_lan: Signal<bool>,
    mut dark_mode: Signal<bool>,
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    custom_rules: Signal<Vec<CustomRule>>,
    mut config_script_enabled: Signal<bool>,
    mut config_script: Signal<String>,
    group_selections: Signal<HashMap<String, String>>,
) -> Element {
    let mut script_check_busy = use_signal(|| false);
    let mut script_editor_open = use_signal(|| false);
    let mut script_draft = use_signal(String::new);
    let toast = use_context::<ToastManager>();

    rsx! {
        div { class: "settings-grid",
            section { class: "settings-section glass-surface",
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
            }

            section { class: "settings-section script-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "CONFIG SCRIPT" }
                        h2 { "JavaScript 配置覆写" }
                    }
                    span {
                        class: if config_script_enabled() { "status-badge ready" } else { "status-badge" },
                        if config_script_enabled() { "已启用" } else { "未启用" }
                    }
                }
                div { class: "setting-row script-setting-row",
                    span { class: "setting-icon", Icon { icon: LdScrollText, width: 19, height: 19 } }
                    div {
                        strong { "main(config)" }
                        small {
                            if config_script().trim().is_empty() {
                                "尚未配置"
                            } else {
                                "QuickJS · 已配置 {config_script().chars().count()} 个字符"
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

            section { class: "settings-section glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "ROUTING" }
                        h2 { "路由模式" }
                    }
                }
                div { class: "segmented-control wide",
                    button { class: if tunnel_mode() == TunnelMode::Rule { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Rule), "规则" }
                    button { class: if tunnel_mode() == TunnelMode::Global { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Global), "全局" }
                    button { class: if tunnel_mode() == TunnelMode::Direct { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Direct), "直连" }
                }
                label { class: "setting-row toggle-row",
                    span { class: "setting-icon", Icon { icon: LdNetwork, width: 19, height: 19 } }
                    div {
                        strong { "TUN 模式" }
                        small { "系统级网络接管" }
                    }
                    input {
                        r#type: "checkbox",
                        checked: tun_enabled,
                        onchange: move |event| tun_enabled.set(event.checked()),
                    }
                    span { class: "switch" }
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
                        disabled: core_restarting(),
                        onchange: move |event| async move {
                            let enabled = event.checked();
                            allow_lan.set(enabled);
                            if !connected() {
                                toast.info(if enabled {
                                    "局域网连接将在下次启动内核时生效"
                                } else {
                                    "仅本机访问将在下次启动内核时生效"
                                });
                                return;
                            }

                            core_restarting.set(true);
                            let request = ConnectionRequest {
                                nodes: nodes(),
                                selected_tag: selected_tag(),
                                mode: tunnel_mode(),
                                tun: tun_enabled(),
                                allow_lan: enabled,
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
                            };
                            match api::restart_core(request).await {
                                Ok(status) => {
                                    let is_running = status.state == "running";
                                    connected.set(is_running);
                                    core_state.set(status.state);
                                    core_version.set(status.version);
                                    core_note.set(status.note);
                                    toast.success(if enabled {
                                        "已允许局域网连接，sing-box 内核已重启"
                                    } else {
                                        "已恢复仅本机访问，sing-box 内核已重启"
                                    });
                                }
                                Err(error) => {
                                    connected.set(false);
                                    core_state.set("stopped".to_string());
                                    toast.error(format!("应用局域网监听设置失败: {error}"));
                                }
                            }
                            core_restarting.set(false);
                        },
                    }
                    span { class: "switch" }
                }
            }

            section { class: "settings-section glass-surface",
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
                            onclick: move |_| async move {
                                script_check_busy.set(true);
                                let request = ConnectionRequest {
                                    nodes: nodes(),
                                    selected_tag: selected_tag(),
                                    mode: tunnel_mode(),
                                    tun: tun_enabled(),
                                    allow_lan: allow_lan(),
                                    custom_rules: Vec::new(),
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
    let count = report.nodes.len();
    let rejected_count = report.rejected.len();
    let subscription = subscriptions
        .iter_mut()
        .find(|subscription| subscription.id == subscription_id)
        .ok_or_else(|| "订阅已不存在".to_string())?;
    subscription.nodes = namespace_nodes(subscription_id, report.nodes);
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
                rejected_count: 0,
            },
            Subscription {
                id: 2,
                name: "Two".to_string(),
                source: "two".to_string(),
                nodes: second,
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
                rejected_count: 0,
            },
            Subscription {
                id: 2,
                name: "Two".to_string(),
                source: "two".to_string(),
                nodes: namespace_nodes(2, vec![node]),
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
            rejected_count: 0,
        }];

        assert_eq!(
            apply_subscription_report(
                &mut subscriptions,
                9,
                ParseReport {
                    nodes: vec![replacement],
                    rejected: Vec::new(),
                },
            ),
            Ok(1)
        );
        assert_eq!(subscriptions[0].nodes.len(), 1);
        assert!(subscriptions[0].nodes[0].tag.starts_with("subscription-9-"));
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
        assert_eq!(traffic_chart_height(50, 100), "50%");
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

        move_custom_rule(&mut rules, 2, -1);
        assert_eq!(
            rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
            vec![2, 1]
        );
        move_custom_rule(&mut rules, 2, -1);
        assert_eq!(
            rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
            vec![2, 1]
        );
    }
}
