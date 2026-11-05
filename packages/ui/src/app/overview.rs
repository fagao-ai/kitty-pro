//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

/// Bars in the live traffic chart: one per second over the last minute.
pub(crate) const TRAFFIC_HISTORY_LEN: usize = 60;

#[component]
pub(crate) fn OverviewView(
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    mut active_view: Signal<AppView>,
    proxy_server_nameservers: Vec<String>,
    mut connected: Signal<bool>,
    mut core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    mut core_state: Signal<String>,
    mut core_note: Signal<Option<String>>,
    mut core_version: Signal<Option<String>>,
    mut tunnel_mode: Signal<TunnelMode>,
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
    tun_busy: Signal<bool>,
    lan_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    // Read here for the "重新连接" banner; the restart paths clear it centrally.
    let config_pending = use_context::<ConfigPending>().0;
    // F8: exit-connectivity self-check result, guarded by a synchronous busy
    // flag so a second click cannot start a second probe.
    let mut probe_busy = use_signal(|| false);
    let mut probe_result = use_signal(|| None::<(bool, String)>);
    let mut core_recheck_busy = use_signal(|| false);
    let selected_node = nodes().into_iter().find(|node| node.tag == selected_tag());
    let is_connected = connected();
    let is_core_busy = core_busy();
    let is_core_restarting = core_restarting();
    let is_system_proxy_busy = system_proxy_busy();
    let is_runtime_action_busy = runtime_action_busy();
    let is_core_action_busy =
        is_core_busy || is_core_restarting || is_system_proxy_busy || is_runtime_action_busy;
    let status_title = if tun_busy() {
        "正在申请网络权限"
    } else if is_core_restarting {
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
    let note = if tun_busy() {
        "正在申请网络权限，请在系统弹窗中授权".to_string()
    } else if is_core_restarting {
        "正在重启 sing-box 内核，请稍候".to_string()
    } else if is_system_proxy_busy {
        "正在设置系统代理，请稍候".to_string()
    } else if is_runtime_action_busy {
        "正在应用运行时设置，请稍候".to_string()
    } else if is_core_busy {
        if is_connected {
            "正在停止 sing-box 内核".to_string()
        } else {
            "正在启动内核，首次使用可能需要系统授权，请稍候".to_string()
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
    // The pending-reload banner's handler runs after the power button's
    // closure already moved `proxy_server_nameservers` (both are `move`
    // closures over the same prop), so it needs its own copy.
    let banner_nameservers = proxy_server_nameservers.clone();

    // F22：连接卡折叠的次要信息展示串。单独构造，避免在 rsx 字符串里嵌套
    // 引号，也避免把后面 MetricCard 还要用的 `uptime_label` 提前移动。
    let core_version_label = core_version().unwrap_or_else(|| "未知".to_string());
    let uptime_display = format!("{uptime_label} {uptime_unit}");
    let dns_override_label = format!("{} 条", dns_overrides().len());
    let node_count_label = format!("{} / {}", nodes().len(), subscription_count);

    rsx! {
        div { class: "overview-grid",
            section { class: if is_core_action_busy { "connection-panel glass-surface busy" } else if is_connected { "connection-panel glass-surface connected" } else { "connection-panel glass-surface" },
                div { class: "connection-copy",
                    div { class: "status-line", role: "status", aria_live: "polite",
                        span { class: if is_core_action_busy { "busy-dot" } else if is_connected { "live-dot" } else { "idle-dot" } }
                        span { "{status_title} · {mode_name}" }
                    }
                    // F8 的提示语义已由并排可见的「测试连通」按钮承载；这行
                    // 常驻文字在 700/768 高的折叠预算里放不下（layout 守卫），
                    // 运行中不再显示，异常态提示保留在下方。
                    // U4/U17/U20: explicit first-load / failure states. The
                    // ProxyApp heartbeat times "checking" out to "unavailable".
                    if core_state() == "checking" {
                        div { class: "inline-note info", role: "status",
                            Icon { icon: LdInfo, width: 16, height: 16 }
                            span { "正在检查内核…" }
                        }
                    }
                    if core_state() == "unavailable" {
                        div { class: "inline-note error", role: "alert",
                            Icon { icon: LdCircleAlert, width: 16, height: 16 }
                            span { "内核不可用，请点「重新检测内核」重试；若仍失败，请检查服务是否运行、端口是否被占用、平台是否支持。" }
                        }
                        div { class: "rule-editor-actions",
                            button {
                                class: "secondary-button compact",
                                disabled: core_recheck_busy(),
                                onclick: move |_| {
                                    if *core_recheck_busy.peek() { return; }
                                    core_recheck_busy.set(true);
                                    spawn(async move {
                                        match api::core_status().await {
                                            Ok(status) => {
                                                connected.set(status.state == "running");
                                                core_state.set(status.state);
                                                core_version.set(status.version);
                                                core_note.set(status.note);
                                            }
                                            Err(error) => {
                                                connected.set(false);
                                                core_state.set("unavailable".to_string());
                                                core_note.set(Some(error.to_string()));
                                            }
                                        }
                                        core_recheck_busy.set(false);
                                    });
                                },
                                Icon { icon: LdRefreshCw, width: 15, height: 15 }
                                "重新检测内核"
                            }
                        }
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
                    // F14: keep the mode switch on the connection card itself.
                    div { class: "segmented-control", role: "group", aria_label: "分流模式",
                        button { class: if tunnel_mode() == TunnelMode::Rule { "active" }, aria_pressed: tunnel_mode() == TunnelMode::Rule, onclick: move |_| { tunnel_mode.set(TunnelMode::Rule); flag_config_pending(connected(), config_pending, toast, "路由模式已切换，重新连接后生效"); }, "规则模式" }
                        button { class: if tunnel_mode() == TunnelMode::Global { "active" }, aria_pressed: tunnel_mode() == TunnelMode::Global, onclick: move |_| { tunnel_mode.set(TunnelMode::Global); flag_config_pending(connected(), config_pending, toast, "路由模式已切换，重新连接后生效"); }, "全局代理" }
                        button { class: if tunnel_mode() == TunnelMode::Direct { "active" }, aria_pressed: tunnel_mode() == TunnelMode::Direct, onclick: move |_| { tunnel_mode.set(TunnelMode::Direct); flag_config_pending(connected(), config_pending, toast, "路由模式已切换，重新连接后生效"); }, "全局直连" }
                    }
                    // F22 折叠预算：连接卡必须整体留在 700px 窗口的折叠线内
                    // （layout 守卫）。原先「切换节点」「测试连通」和 details
                    // 摘要各占一行，现合并为一行；F8 测试连通、切换节点入口、
                    // F22 次要信息折叠全部保留，只压缩垂直空间。
                    div { class: "connection-aux-row",
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
                        // F8: probe the proxy exit, not just the core process.
                        button {
                            class: "secondary-button compact",
                            aria_busy: probe_busy(),
                            disabled: probe_busy(),
                            onclick: move |_| {
                                if *probe_busy.peek() { return; }
                                probe_busy.set(true);
                                probe_result.set(None);
                                spawn(async move {
                                    let outcome = match api::probe_connectivity().await {
                                        Ok(probe) => (probe.ok, probe.detail),
                                        Err(error) => (false, error.to_string()),
                                    };
                                    probe_result.set(Some(outcome));
                                    probe_busy.set(false);
                                });
                            },
                            if probe_busy() {
                                span { class: "spinner" }
                            } else {
                                Icon { icon: LdGauge, width: 15, height: 15 }
                            }
                            span { "测试连通" }
                        }
                        // F22：把版本号 / 运行时长 / DNS 覆盖数等次要信息折叠进
                        // details/summary，摘要与两个次级按钮同行；展开内容在
                        // 行下方自然流出，不影响折叠预算。
                        details {
                            class: "connection-details",
                            summary { "详细信息" }
                            // 展开/收起的显隐与网格布局都在 app.css 的
                            // `.connection-details > div` 规则里：closed 时必须
                            // display:none，否则内容会穿透 details 盒子参与布局。
                            div {
                                span { "内核版本：{core_version_label}" }
                                span { "运行时长：{uptime_display}" }
                                span { "专用 DNS：{dns_override_label}" }
                                span { "节点 / 订阅：{node_count_label}" }
                            }
                        }
                    }
                    if let Some((ok, detail)) = probe_result() {
                        if ok {
                            div { class: "inline-note", role: "status",
                                Icon { icon: LdCircleCheck, width: 16, height: 16 }
                                span { "连通正常：{detail}" }
                            }
                        } else {
                            div { class: "inline-note error", role: "alert",
                                Icon { icon: LdCircleAlert, width: 16, height: 16 }
                                span { "连通失败：{detail} · 请检查节点与出口后点「测试连通」重试" }
                            }
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
                                        system_proxy,
                                        config_pending,
                                    },
                                    core_busy,
                                    system_proxy_busy,
                                    runtime_action_busy,
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

            // Local edits (refresh, import, rule changes) reach the core only
            // through a restart; keep the restart affordance right here instead
            // of telling the user to "reconnect" without a way to do it.
            if config_pending() && is_connected && !is_core_action_busy {
                div { class: "inline-note action pending-reload-banner", role: "status",
                    Icon { icon: LdRefreshCw, width: 16, height: 16 }
                    span { "配置已更改，当前连接仍在使用旧配置。" }
                    button {
                        class: "secondary-button compact",
                        onclick: move |_| {
                            let nameservers = banner_nameservers.clone();
                            async move {
                            let request = ConnectionRequest {
                                nodes: nodes(),
                                selected_tag: selected_tag(),
                                proxy_server_nameservers: nameservers,
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
                            };
                            restart_core_from_ui(
                                request,
                                CoreStatusSignals {
                                    connected,
                                    core_restarting,
                                    core_state,
                                    core_version,
                                    core_note,
                                    system_proxy,
                                    config_pending,
                                },
                                toast,
                                "已应用最新配置",
                            )
                            .await;
                            }
                        },
                        Icon { icon: LdRefreshCw, width: 15, height: 15 }
                        span { "重新连接" }
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
                    tun_busy,
                    lan_busy,
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
                        NodeRow {
                            node,
                            selected_tag,
                            latency_results,
                            on_open: move |_| active_view.set(AppView::Nodes),
                        }
                    }
                }
            }
        }
    }
}

/// Height of one stacked bar segment. The chart keeps a floor so an idle
/// second still reads as a bar instead of collapsing to nothing.
pub(crate) fn traffic_bar_height(sample: u64, peak: u64) -> String {
    if sample == 0 {
        return "2px".to_string();
    }
    let percent = 10 + sample.saturating_mul(84).saturating_div(peak.max(1));
    format!("{}%", percent.min(88))
}

pub(crate) fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

pub(crate) fn format_data_rate(bytes_per_second: u64) -> String {
    format!("{}/s", format_data_amount(bytes_per_second))
}

/// `7890` is the fixed mixed port the embedded core always listens on.
pub(crate) fn proxy_port_label(allow_lan: bool) -> &'static str {
    if allow_lan {
        "局域网代理 7890"
    } else {
        "本地代理 7890"
    }
}

pub(crate) fn format_data_amount(bytes: u64) -> String {
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
pub(crate) fn MetricCard(label: String, value: String, icon: String, unit: String) -> Element {
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
