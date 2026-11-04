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
    tun_busy: Signal<bool>,
    lan_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    // Read here for the "重新连接" banner; the restart paths clear it centrally.
    let config_pending = use_context::<ConfigPending>().0;
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
    // The pending-reload banner's handler runs after the power button's
    // closure already moved `proxy_server_nameservers` (both are `move`
    // closures over the same prop), so it needs its own copy.
    let banner_nameservers = proxy_server_nameservers.clone();

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
