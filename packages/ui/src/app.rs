use api::{CoreTraffic, NodeLatency, SystemProxyStatus};
use dioxus::prelude::*;
use dioxus_free_icons::icons::ld_icons::{
    LdActivity, LdArrowDown, LdArrowUp, LdChevronRight, LdCircleAlert, LdCircleCheck, LdClock3,
    LdGauge, LdGlobe, LdInfo, LdLanguages, LdMoon, LdNetwork, LdPlus, LdPower, LdRadioTower,
    LdRefreshCw, LdRoute, LdSearch, LdServer, LdSettings, LdShieldCheck, LdSun, LdTrash2, LdWifi,
    LdX, LdZap,
};
use dioxus_free_icons::Icon;
use proxy_core::{
    AppProfile, ConnectionRequest, ParseReport, ProxyNode, ProxyProtocol, Subscription, TunnelMode,
};
use std::collections::HashMap;

const APP_CSS: Asset = asset!("/assets/styling/app.css");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    Overview,
    Nodes,
    Subscriptions,
    Settings,
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

#[cfg(target_arch = "wasm32")]
async fn wait_for_traffic_tick() {
    gloo_timers::future::TimeoutFuture::new(1_000).await;
}

#[cfg(not(target_arch = "wasm32"))]
async fn wait_for_traffic_tick() {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

impl AppView {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Nodes => "节点",
            Self::Subscriptions => "订阅",
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
    let mut core_state = use_signal(|| "checking".to_string());
    let mut core_version = use_signal(|| None::<String>);
    let mut core_note = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<String>);
    let mut nodes = use_signal(Vec::<ProxyNode>::new);
    let mut selected_tag = use_signal(String::new);
    let mut subscriptions = use_signal(Vec::<Subscription>::new);
    let mut tunnel_mode = use_signal(|| TunnelMode::Rule);
    let mut tun_enabled = use_signal(|| false);
    let mut import_open = use_signal(|| false);
    let mut import_name = use_signal(String::new);
    let mut import_source = use_signal(String::new);
    let mut import_busy = use_signal(|| false);
    let mut import_error = use_signal(|| None::<String>);
    let search = use_signal(String::new);
    let refresh_busy = use_signal(|| None::<RefreshTarget>);
    let latency_results = use_signal(HashMap::<String, NodeLatency>::new);
    let latency_busy = use_signal(|| false);
    let mut traffic = use_signal(TrafficDisplay::default);
    let mut profile_loaded = use_signal(|| false);
    let mut system_proxy = use_signal(|| SystemProxyLoadState::Loading);
    let system_proxy_busy = use_signal(|| false);

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
            match api::load_profile().await {
                Ok(profile) => {
                    let restored_nodes = collect_nodes(&profile.subscriptions);
                    let restored_tag = select_available_tag(&restored_nodes, &profile.selected_tag);
                    subscriptions.set(profile.subscriptions);
                    nodes.set(restored_nodes);
                    selected_tag.set(restored_tag);
                    tunnel_mode.set(profile.tunnel_mode);
                    tun_enabled.set(profile.tun_enabled);
                    dark_mode.set(profile.dark_mode);
                    profile_loaded.set(true);
                }
                Err(error) => notice.set(Some(format!("无法恢复本地配置: {error}"))),
            }
        });
    });

    use_effect(move || {
        if !profile_loaded() {
            return;
        }
        let profile = AppProfile {
            subscriptions: subscriptions(),
            selected_tag: selected_tag(),
            tunnel_mode: tunnel_mode(),
            tun_enabled: tun_enabled(),
            dark_mode: dark_mode(),
            ..AppProfile::default()
        };
        spawn(async move {
            if let Err(error) = api::save_profile(profile).await {
                notice.set(Some(format!("本地配置保存失败: {error}")));
            }
        });
    });

    use_effect(move || {
        if core_state() != "running" {
            traffic.set(TrafficDisplay::default());
            return;
        }
        spawn(async move {
            let mut previous = None::<CoreTraffic>;
            let mut history = Vec::new();
            while core_state() == "running" {
                if let Ok(current) = api::core_traffic().await {
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

    let current_view = active_view();
    let filtered_nodes: Vec<ProxyNode> = {
        let needle = search().to_ascii_lowercase();
        nodes()
            .into_iter()
            .filter(|node| {
                needle.is_empty()
                    || node.name.to_ascii_lowercase().contains(&needle)
                    || node.server.to_ascii_lowercase().contains(&needle)
                    || node.protocol.label().to_ascii_lowercase().contains(&needle)
            })
            .collect()
    };
    let root_class = if dark_mode() {
        "proxy-app theme-dark"
    } else {
        "proxy-app"
    };
    let connection_allowed = !nodes().is_empty() && core_state() != "unavailable";

    rsx! {
        document::Link { rel: "stylesheet", href: APP_CSS }
        div { class: root_class,
            div { class: "ambient-grid" }
            aside { class: "sidebar glass-surface",
                Brand {}
                nav { class: "primary-nav", aria_label: "主导航",
                    NavItem { view: AppView::Overview, active_view }
                    NavItem { view: AppView::Nodes, active_view }
                    NavItem { view: AppView::Subscriptions, active_view }
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
                    div {
                        p { class: "eyebrow", "KITTY PRO" }
                        h1 { "{current_view.title()}" }
                    }
                    div { class: "topbar-actions",
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

                if let Some(message) = notice() {
                    div { class: "notice-bar glass-surface",
                        Icon { icon: LdInfo, width: 17, height: 17 }
                        span { "{message}" }
                        button {
                            class: "bare-icon",
                            title: "关闭",
                            onclick: move |_| notice.set(None),
                            Icon { icon: LdX, width: 16, height: 16 }
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
                            core_state,
                            core_note,
                            tunnel_mode,
                            tun_enabled,
                            connection_allowed,
                            latency_results,
                            traffic,
                            notice,
                        }
                    },
                    AppView::Nodes => rsx! {
                        NodesView {
                            nodes: filtered_nodes,
                            probe_nodes: nodes(),
                            all_count: nodes().len(),
                            selected_tag,
                            search,
                            latency_results,
                            latency_busy,
                            import_open,
                            notice,
                        }
                    },
                    AppView::Subscriptions => rsx! {
                        SubscriptionsView {
                            subscriptions,
                            import_open,
                            nodes,
                            selected_tag,
                            refresh_busy,
                            notice,
                        }
                    },
                    AppView::Settings => rsx! {
                        SettingsView {
                            platform: platform.clone(),
                            core_state,
                            core_version,
                            core_note,
                            tunnel_mode,
                            tun_enabled,
                            dark_mode,
                            system_proxy,
                            system_proxy_busy,
                            notice,
                        }
                    },
                }
            }

            nav { class: "mobile-nav glass-surface", aria_label: "移动端导航",
                NavItem { view: AppView::Overview, active_view }
                NavItem { view: AppView::Nodes, active_view }
                NavItem { view: AppView::Subscriptions, active_view }
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
                                    placeholder: "https://...",
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
                                                name,
                                                source,
                                                nodes: parsed_nodes,
                                                rejected_count: rejected,
                                            });
                                            let merged_nodes = collect_nodes(&subscriptions());
                                            let next_tag = select_available_tag(&merged_nodes, &selected_tag());
                                            nodes.set(merged_nodes);
                                            selected_tag.set(next_tag);
                                            notice.set(Some(format!("已导入 {count} 个节点")));
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
        }
    }
}

#[component]
fn Brand() -> Element {
    rsx! {
        div { class: "brand",
            div { class: "brand-mark", aria_hidden: "true",
                Icon { icon: LdShieldCheck, width: 24, height: 24 }
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
    mut core_state: Signal<String>,
    core_note: Signal<Option<String>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    connection_allowed: bool,
    latency_results: Signal<HashMap<String, NodeLatency>>,
    traffic: Signal<TrafficDisplay>,
    mut notice: Signal<Option<String>>,
) -> Element {
    let selected_node = nodes().into_iter().find(|node| node.tag == selected_tag());
    let status_title = if connected() {
        "已连接"
    } else {
        "未连接"
    };
    let node_name = selected_node
        .as_ref()
        .map(|node| node.name.as_str())
        .unwrap_or("未选择节点");
    let mode_name = mode_label(tunnel_mode());
    let note = core_note().unwrap_or_else(|| {
        if core_state() == "checking" {
            "正在检查 sing-box".to_string()
        } else {
            "sing-box 已就绪".to_string()
        }
    });
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
            section { class: if connected() { "connection-panel glass-surface connected" } else { "connection-panel glass-surface" },
                div { class: "connection-copy",
                    div { class: "status-line",
                        span { class: if connected() { "live-dot" } else { "idle-dot" } }
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
                            "本地代理 7890"
                        }
                    }
                }
                button {
                    class: if connected() { "power-button active" } else { "power-button" },
                    title: if connected() { "断开连接" } else { "建立连接" },
                    disabled: core_busy() || (!connected() && !connection_allowed),
                    onclick: move |_| async move {
                        let target = !connected();
                        core_busy.set(true);
                        let request = target.then(|| ConnectionRequest {
                            nodes: nodes(),
                            selected_tag: selected_tag(),
                            mode: tunnel_mode(),
                            tun: tun_enabled(),
                        });
                        match api::set_core_enabled(target, request).await {
                            Ok(status) => {
                                connected.set(status.state == "running");
                                core_state.set(status.state);
                                notice.set(Some(if target {
                                    "sing-box 已启动".to_string()
                                } else {
                                    "连接已断开".to_string()
                                }));
                            }
                            Err(error) => notice.set(Some(error.to_string())),
                        }
                        core_busy.set(false);
                    },
                    if core_busy() {
                        span { class: "spinner large" }
                    } else {
                        Icon { icon: LdPower, width: 32, height: 32 }
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
                    }
                    div {
                        span { class: "metric-icon upload", Icon { icon: LdArrowUp, width: 16, height: 16 } }
                        p { "上传" }
                        strong {
                            title: "累计上传 {format_data_amount(traffic.upload_total)}",
                            "{format_data_rate(traffic.upload_bytes_per_second)}"
                        }
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
    nodes: Vec<ProxyNode>,
    probe_nodes: Vec<ProxyNode>,
    all_count: usize,
    selected_tag: Signal<String>,
    mut search: Signal<String>,
    mut latency_results: Signal<HashMap<String, NodeLatency>>,
    mut latency_busy: Signal<bool>,
    mut import_open: Signal<bool>,
    mut notice: Signal<Option<String>>,
) -> Element {
    rsx! {
        section { class: "workspace-section glass-surface",
            div { class: "workspace-toolbar",
                div {
                    p { class: "eyebrow", "PROXY NODES" }
                    h2 { "全部节点" }
                    span { "{all_count} 个节点" }
                }
                div { class: "toolbar-controls subscription-actions",
                    label { class: "search-field",
                        Icon { icon: LdSearch, width: 17, height: 17 }
                        input {
                            value: search,
                            placeholder: "搜索节点",
                            oninput: move |event| search.set(event.value()),
                        }
                    }
                    button {
                        class: "icon-button glass-control",
                        title: if latency_busy() { "正在刷新延迟" } else { "刷新延迟" },
                        disabled: latency_busy() || probe_nodes.is_empty(),
                        onclick: move |_| {
                            let probe_nodes = probe_nodes.clone();
                            async move {
                                latency_busy.set(true);
                                match api::measure_node_latency(probe_nodes).await {
                                    Ok(results) => {
                                        let failures = results
                                            .iter()
                                            .filter(|result| result.latency_ms.is_none())
                                            .count();
                                        latency_results.set(
                                            results
                                                .into_iter()
                                                .map(|result| (result.tag.clone(), result))
                                                .collect(),
                                        );
                                        notice.set(Some(if failures == 0 {
                                            "节点延迟已刷新".to_string()
                                        } else {
                                            format!("延迟刷新完成，{failures} 个节点不可用")
                                        }));
                                    }
                                    Err(error) => notice.set(Some(format!("延迟探测失败: {error}"))),
                                }
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
            } else if nodes.is_empty() {
                div { class: "large-empty",
                    Icon { icon: LdSearch, width: 28, height: 28 }
                    strong { "没有匹配的节点" }
                }
            } else {
                div { class: "node-list",
                    for node in nodes {
                        NodeRow { node, selected_tag, latency_results }
                    }
                }
            }
        }
    }
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
    mut import_open: Signal<bool>,
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    refresh_busy: Signal<Option<RefreshTarget>>,
    notice: Signal<Option<String>>,
) -> Element {
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
                                let merged_nodes = collect_nodes(&subscriptions());
                                let next_tag = select_available_tag(&merged_nodes, &selected_tag());
                                nodes.set(merged_nodes);
                                selected_tag.set(next_tag);
                                notice.set(Some(if failed == 0 {
                                    format!("已刷新 {refreshed} 个订阅")
                                } else {
                                    format!("已刷新 {refreshed} 个订阅，{failed} 个失败")
                                }));
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
                            nodes,
                            selected_tag,
                            refresh_busy,
                            notice,
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
    mut nodes: Signal<Vec<ProxyNode>>,
    mut selected_tag: Signal<String>,
    mut refresh_busy: Signal<Option<RefreshTarget>>,
    mut notice: Signal<Option<String>>,
) -> Element {
    let subscription_id = subscription.id;
    let subscription_source = subscription.source.clone();
    let display_source = source_label(&subscription.source);
    let refreshing = refresh_busy() == Some(RefreshTarget::One(subscription_id));

    rsx! {
        div { class: "subscription-row",
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
                class: "icon-button",
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
                                    let merged_nodes = collect_nodes(&subscriptions());
                                    let next_tag =
                                        select_available_tag(&merged_nodes, &selected_tag());
                                    nodes.set(merged_nodes);
                                    selected_tag.set(next_tag);
                                    notice.set(Some(format!("已刷新 {count} 个节点")));
                                }
                                Err(reason) => {
                                    notice.set(Some(format!("刷新失败: {reason}")));
                                }
                            }
                        }
                        Err(error) => notice.set(Some(format!("刷新失败: {error}"))),
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
                class: "icon-button danger",
                title: "删除订阅",
                disabled: refresh_busy().is_some(),
                onclick: move |_| {
                    subscriptions.write().retain(|item| item.id != subscription_id);
                    let merged_nodes = collect_nodes(&subscriptions());
                    let next_tag = select_available_tag(&merged_nodes, &selected_tag());
                    nodes.set(merged_nodes);
                    selected_tag.set(next_tag);
                    notice.set(Some("订阅已删除".to_string()));
                },
                Icon { icon: LdTrash2, width: 17, height: 17 }
            }
        }
    }
}

#[component]
fn SettingsView(
    platform: String,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    mut tunnel_mode: Signal<TunnelMode>,
    mut tun_enabled: Signal<bool>,
    mut dark_mode: Signal<bool>,
    mut system_proxy: Signal<SystemProxyLoadState>,
    mut system_proxy_busy: Signal<bool>,
    mut notice: Signal<Option<String>>,
) -> Element {
    let (proxy_status, proxy_loading, proxy_error) = match system_proxy() {
        SystemProxyLoadState::Loading => (None, true, None),
        SystemProxyLoadState::Ready(status) => (Some(status), false, None),
        SystemProxyLoadState::Failed(error) => (None, false, Some(error)),
    };
    let proxy_ready = proxy_status.is_some();
    let proxy_supported = proxy_status.as_ref().is_some_and(|status| status.supported);
    let proxy_enabled = proxy_status.as_ref().is_some_and(|status| status.enabled);
    let proxy_detail = proxy_status
        .as_ref()
        .map(|status| status.detail.clone())
        .unwrap_or_else(|| {
            if proxy_loading {
                "正在读取系统代理状态".to_string()
            } else {
                "无法读取系统代理状态".to_string()
            }
        });
    let enable_allowed = core_state() == "running";

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

            section { class: "settings-section glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "SYSTEM PROXY" }
                        h2 { "系统代理" }
                    }
                    span {
                        class: if proxy_enabled { "status-badge online" } else { "status-badge" },
                        if proxy_loading { "读取中" }
                        else if proxy_enabled { "已启用" }
                        else if proxy_error.is_some() { "读取失败" }
                        else { "未启用" }
                    }
                }
                label { class: "setting-row toggle-row",
                    span { class: "setting-icon", Icon { icon: LdRoute, width: 19, height: 19 } }
                    div {
                        strong { "使用本地 mixed 代理" }
                        small { "127.0.0.1:7890；{proxy_detail}" }
                    }
                    if proxy_loading {
                        span { class: "switch switch-loading",
                            span { class: "spinner" }
                        }
                    } else {
                        input {
                            r#type: "checkbox",
                            checked: proxy_enabled,
                            disabled: system_proxy_busy() || !proxy_ready || !proxy_supported || (!proxy_enabled && !enable_allowed),
                            onchange: move |event| {
                                let enabled = event.checked();
                                async move {
                                    system_proxy_busy.set(true);
                                    match api::set_system_proxy(enabled).await {
                                        Ok(status) => {
                                            system_proxy.set(SystemProxyLoadState::Ready(status));
                                            notice.set(Some(if enabled {
                                                "系统代理已启用".to_string()
                                            } else {
                                                "系统代理已恢复为启用前的设置".to_string()
                                            }));
                                        }
                                        Err(error) => {
                                            notice.set(Some(format!("系统代理设置失败: {error}")));
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
                if let Some(error) = proxy_error {
                    div { class: "inline-note",
                        Icon { icon: LdInfo, width: 16, height: 16 }
                        span { "无法读取系统代理状态：{error}" }
                    }
                } else if proxy_ready && !proxy_supported {
                    div { class: "inline-note",
                        Icon { icon: LdInfo, width: 16, height: 16 }
                        span { "当前平台尚未提供系统代理适配。" }
                    }
                } else if proxy_ready && !proxy_enabled && !enable_allowed {
                    div { class: "inline-note",
                        Icon { icon: LdInfo, width: 16, height: 16 }
                        span { "请先在概览页建立连接，再启用系统代理。" }
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
    }
}

fn protocol_abbreviation(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Hysteria2 => "HY",
        ProxyProtocol::Vmess => "VM",
        ProxyProtocol::Vless => "VL",
        ProxyProtocol::Trojan => "TR",
        ProxyProtocol::Shadowsocks => "SS",
    }
}

fn protocol_class(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::Hysteria2 => "protocol-mark hy2",
        ProxyProtocol::Vmess => "protocol-mark vmess",
        ProxyProtocol::Vless => "protocol-mark vless",
        ProxyProtocol::Trojan => "protocol-mark trojan",
        ProxyProtocol::Shadowsocks => "protocol-mark shadowsocks",
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

fn collect_nodes(subscriptions: &[Subscription]) -> Vec<ProxyNode> {
    subscriptions
        .iter()
        .flat_map(|subscription| subscription.nodes.iter().cloned())
        .collect()
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
    fn formats_traffic_rates_for_display() {
        assert_eq!(format_data_rate(0), "0 B/s");
        assert_eq!(format_data_rate(1_536), "1.5 KiB/s");
        assert_eq!(format_data_amount(2 * 1024 * 1024), "2.0 MiB");
        assert_eq!(traffic_chart_height(50, 100), "50%");
    }
}
