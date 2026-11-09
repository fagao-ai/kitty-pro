//! Overview: the connect control, the figures that describe the running core, and the
//! traffic of the last minute.

use crate::domain;
use crate::store::{AppStore, CoreBusy};
use crate::ui::{controls, theme, Icon};
use nsh_base::theme as base_theme;
use nsh_core::prelude::*;

/// Bars in the live traffic chart: one per poll over the last minute.
const HISTORY_LEN: usize = 60;

pub fn overview(store: AppStore) -> impl IntoElement {
    crate::shell::page_scroll(
        view()
            .flex_col()
            .gap_4()
            .child(connection_panel(store))
            .child(metrics_row(store))
            .child(traffic_card(store)),
    )
}

/// The one control that matters: whether traffic is flowing right now.
fn connection_panel(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_4()
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .style_with(move |s| {
            if store.connected() {
                s.border_color(move || domain::core_tint(true, "running").wash())
            } else {
                s
            }
        })
        .child(status_line(store))
        // Say which node the traffic would go through. "已连接" without a name is only
        // half an answer, and it is the first thing a user checks after picking one.
        .child(when(
            move || store.selected_node().is_some(),
            move || selected_node_line(store),
        ))
        .child(when(
            move || store.core_state() == "unavailable",
            move || unavailable_note(store),
        ))
        .child(controls::hero_button(
            move || action_label(store),
            // A binding, not a value: the body of this function runs once, so reading
            // `connected()` here would freeze the icon at whatever it was on the first
            // frame and leave a connected app offering to connect again.
            move || {
                if store.connected() {
                    Icon::Power
                } else {
                    Icon::Rocket
                }
            },
            // With nothing to route and no core to talk to, connecting cannot succeed, so
            // the control says so instead of failing on press. Disconnect stays available.
            move || {
                store.core_action_busy()
                    || (!store.connected()
                        && (store.nodes().is_empty() || store.core_state() == "unavailable"))
            },
            // The same applies to the decision itself. Capturing it would make the second
            // press repeat the first action.
            move || store.toggle_core(!store.connected()),
        ))
        .child(when(
            move || store.config_pending(),
            move || pending_banner(store),
        ))
}

/// The live status: state, mode, and what the app is doing about it.
fn status_line(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(controls::status_dot_tinted(move || {
            domain::core_tint(store.connected(), &store.core_state())
        }))
        .child(
            text(move || status_title(store))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || base_theme().card_foreground),
        )
        .child(
            text(move || format!("· {}", domain::mode_label(store.tunnel_mode())))
                .text_xs()
                .text_color(theme::text_soft()),
        )
}

/// The button's verb says what the press will do, including while it is doing it.
fn action_label(store: AppStore) -> String {
    match store.busy() {
        CoreBusy::Toggling if store.connected() => "正在断开".to_owned(),
        CoreBusy::Toggling => "正在启动".to_owned(),
        CoreBusy::Restarting => "正在重启".to_owned(),
        CoreBusy::Tun => "正在申请网络权限".to_owned(),
        CoreBusy::Idle if store.connected() => "断开".to_owned(),
        CoreBusy::Idle => "启动代理".to_owned(),
    }
}

fn status_title(store: AppStore) -> &'static str {
    if store.core_action_busy() {
        "处理中"
    } else if store.connected() {
        "已连接"
    } else {
        "未连接"
    }
}

/// The chosen node's name, address and protocol.
fn selected_node_line(store: AppStore) -> impl IntoElement {
    let node = computed(move || store.selected_node());
    view()
        .flex_col()
        .gap_0p5()
        .child(
            text(move || {
                node.with(|node| node.as_ref().map(|n| n.name.clone()).unwrap_or_default())
            })
            .text_size(px(16.0))
            .font_weight(FontWeight::SEMIBOLD)
            .number_of_lines(1)
            .text_color(move || base_theme().card_foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || {
                        node.with(|node| {
                            node.as_ref()
                                .map(|n| format!("{}:{}", n.server, n.port))
                                .unwrap_or_default()
                        })
                    })
                    .text_xs()
                    .font_family(theme::font_mono())
                    .text_color(theme::text_soft()),
                )
                .child(
                    text(move || {
                        node.with(|node| {
                            node.as_ref()
                                .map(|n| n.protocol.label().to_owned())
                                .unwrap_or_default()
                        })
                    })
                    .text_xs()
                    .text_color(theme::text_faint()),
                ),
        )
}

/// A core that will not report is the one failure the user can act on, so the
/// note names the retry rather than only the problem.
fn unavailable_note(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_3()
        .p_3()
        .rounded(theme::radius_tile())
        .bg(move || domain::core_tint(false, "unavailable").wash())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    controls::icon(Icon::CircleAlert, 16.0)
                        .text_color(move || domain::core_tint(false, "unavailable").foreground()),
                )
                .child(
                    text("内核不可用，请重新检测；若仍失败，请检查服务是否运行、端口是否被占用。")
                        .text_xs()
                        .text_color(move || domain::core_tint(false, "unavailable").foreground()),
                ),
        )
        .child(view().flex_row().gap_2().child(controls::secondary_button(
            "重新检测内核",
            move || store.refresh_status(),
        )))
}

/// Local edits the running core has not taken yet. Naming the gap is what makes
/// "connected" trustworthy: the chip alone would overstate it.
fn pending_banner(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .p_3()
        .rounded(theme::radius_tile())
        .bg(move || theme::Tint::Warning.wash())
        .child(
            text("本地配置已改动，运行中的内核还是旧的。")
                .text_xs()
                .text_color(move || theme::Tint::Warning.foreground()),
        )
        .child(controls::secondary_button("重新连接", move || {
            store.restart_core()
        }))
}

/// The four figures worth a glance. Each is a plain label over a value; none is
/// interactive, so none is styled as though it were.
fn metrics_row(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_4()
        .child(metric_card(store, MetricKind::Uptime))
        .child(metric_card(store, MetricKind::Latency))
        .child(metric_card(store, MetricKind::Nodes))
        .child(metric_card(store, MetricKind::Connections))
}

#[derive(Clone, Copy)]
enum MetricKind {
    Uptime,
    Latency,
    Nodes,
    Connections,
}

fn metric_card(store: AppStore, kind: MetricKind) -> impl IntoElement {
    let (label, glyph) = match kind {
        MetricKind::Uptime => ("运行时长", Icon::Timer),
        MetricKind::Latency => ("节点延迟", Icon::Gauge),
        MetricKind::Nodes => ("可用节点", Icon::GitFork),
        MetricKind::Connections => ("活动连接", Icon::Network),
    };
    view()
        .flex_1()
        .min_w(px(168.0))
        .flex_col()
        .gap_3()
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    controls::icon(glyph, 15.0).text_color(move || base_theme().muted_foreground),
                )
                .child(text(label).text_xs().text_color(theme::text_faint())),
        )
        .child(metric_value(store, kind))
}

/// A figure and the line that explains it. Both are closures: this function runs once,
/// and a value read here would sit at whatever the profile held on the first frame.
fn metric_value(store: AppStore, kind: MetricKind) -> impl IntoElement {
    view()
        .flex_col()
        .gap_0p5()
        .child(
            text(move || metric_figure(store, kind))
                .text_size(px(22.0))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(theme::font_display())
                .text_color(move || base_theme().card_foreground),
        )
        .child(
            text(move || metric_note(store, kind))
                .text_xs()
                .text_color(theme::text_faint()),
        )
}

fn metric_figure(store: AppStore, kind: MetricKind) -> String {
    match kind {
        MetricKind::Uptime => store
            .uptime_seconds()
            .map(domain::format_uptime)
            .unwrap_or_else(|| "--".to_owned()),
        MetricKind::Latency => match store.selected_node() {
            None => "—".to_owned(),
            Some(node) => match store.latency_for(&node.tag) {
                Some(result) => {
                    domain::format_latency(result.latency_ms, result.error.as_deref()).0
                }
                None if store.latency_busy() => "探测中".to_owned(),
                None => "未测试".to_owned(),
            },
        },
        MetricKind::Nodes => format!("{}", store.nodes().len()),
        MetricKind::Connections => format!("{}", store.traffic().active_connections),
    }
}

fn metric_note(store: AppStore, kind: MetricKind) -> &'static str {
    match kind {
        MetricKind::Uptime => {
            if store.uptime_seconds().is_some() {
                "运行中"
            } else {
                "未启动"
            }
        }
        MetricKind::Latency => match store.selected_node() {
            None => "没有可用节点",
            Some(node) => match store.latency_for(&node.tag) {
                Some(_) => "最近一次探测",
                None if store.latency_busy() => "正在逐个测量",
                None => "尚未测量",
            },
        },
        MetricKind::Nodes => {
            if store.subscription_count() > 0 {
                "订阅内节点"
            } else {
                "等待导入订阅"
            }
        }
        MetricKind::Connections => {
            if store.traffic().active_connections > 0 {
                "正在转发"
            } else {
                "空闲"
            }
        }
    }
}

/// Download and upload over the last minute. The bars are relative to the peak
/// in the window, so a quiet minute still shows its own shape.
fn traffic_card(store: AppStore) -> impl IntoElement {
    let history = computed(move || store.traffic());
    view()
        .flex_col()
        .gap_4()
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(controls::section_header(
            "近一分钟流量",
            Some(
                text(move || chart_status(&history.get(), store.connected()))
                    .text_xs()
                    .text_color(theme::text_faint()),
            ),
        ))
        .child(bars(history))
        .child(rate_row(store))
        .child(when(
            move || !store.nodes().is_empty(),
            move || measure_row(store),
        ))
}

/// The port the core listens on, and the way to refresh the one figure that cannot be
/// inferred from traffic. Naming the endpoint is what makes "已连接" checkable by hand.
fn measure_row(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(move || domain::proxy_port_label(store.allow_lan()))
                .text_xs()
                .font_family(theme::font_mono())
                .text_color(theme::text_faint()),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(when(
                    move || store.latency_busy(),
                    || text("正在测量").text_xs().text_color(theme::text_faint()),
                ))
                .child(controls::secondary_button(
                    move || {
                        if store.latency_busy() {
                            "测量中"
                        } else {
                            "刷新延迟"
                        }
                    },
                    move || store.measure_latency(),
                )),
        )
}

fn chart_status(window: &crate::store::TrafficWindow, connected: bool) -> String {
    if window.active_connections > 0 {
        format!("{} 个活动连接", window.active_connections)
    } else if connected {
        "等待代理流量".to_owned()
    } else {
        "内核未启动".to_owned()
    }
}

fn bars(history: Memo<crate::store::TrafficWindow>) -> impl IntoElement {
    let peak = computed(move || {
        let window = history.get();
        window
            .download_history
            .iter()
            .chain(window.upload_history.iter())
            .copied()
            .max()
            .unwrap_or(1)
            .max(1)
    });
    view()
        .h(px(72.0))
        .w_full()
        .flex_row()
        .items_end()
        .gap_0p5()
        .children((0..HISTORY_LEN).map(move |index| {
            view()
                .flex_1()
                .flex_col()
                .items_center()
                .justify_end()
                .gap_0p5()
                .child(bar(history, peak, index, true, theme::Tint::Info, 28.0))
                .child(bar(history, peak, index, false, theme::Tint::Success, 44.0))
        }))
}

/// One bar of one column. The height is a binding over the window memo, so a new sample
/// moves the bars without rebuilding the sixty columns around them; reading `history`
/// outside a closure here would freeze the chart at its first frame.
fn bar(
    history: Memo<crate::store::TrafficWindow>,
    peak: Memo<u64>,
    index: usize,
    upload: bool,
    tint: theme::Tint,
    max_height: f32,
) -> impl IntoElement {
    view()
        .w_full()
        .h(move || {
            let window = history.get();
            let samples = if upload {
                &window.upload_history
            } else {
                &window.download_history
            };
            // Columns are fixed at HISTORY_LEN; samples arrive from the right, so a
            // partly-filled window leaves stubs at the left rather than shifting.
            let offset = index + HISTORY_LEN - samples.len();
            let value = samples.get(offset).copied().unwrap_or(0);
            let height = (value as f32 / peak.get().max(1) as f32) * max_height;
            px(height.max(2.0))
        })
        .rounded(px(2.0))
        .bg(move || tint.foreground())
        .opacity(0.85)
}

fn rate_row(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_4()
        .child(legend_dot(theme::Tint::Success, "下载"))
        .child(legend_dot(theme::Tint::Info, "上传"))
        .child(
            view()
                .flex_1()
                .flex_row()
                .items_center()
                .justify_end()
                .gap_4()
                .child(rate(store, true))
                .child(rate(store, false)),
        )
}

fn legend_dot(tint: theme::Tint, label: &'static str) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(controls::status_dot_tinted(move || tint))
        .child(text(label).text_xs().text_color(theme::text_faint()))
}

/// The live rate, not the cumulative total: a chart of per-second samples is only
/// readable next to the same unit.
fn rate(store: AppStore, download: bool) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(
            controls::icon(
                if download {
                    Icon::ArrowDown
                } else {
                    Icon::ArrowUp
                },
                13.0,
            )
            .text_color(move || base_theme().muted_foreground),
        )
        .child(
            text(move || {
                let window = store.traffic();
                domain::format_data_rate(if download {
                    window.download_per_second
                } else {
                    window.upload_per_second
                })
            })
            .text_xs()
            .font_family(theme::font_mono())
            .text_color(move || base_theme().card_foreground),
        )
}
