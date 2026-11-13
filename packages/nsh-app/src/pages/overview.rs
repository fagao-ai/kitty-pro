//! Overview: the connect control, the figures that describe the running core, and the
//! traffic of the last minute.

use crate::domain;
use crate::i18n;
use crate::store::{AppStore, SystemProxy};
use crate::ui::{controls, theme, Icon};
use nsh_base::theme as base_theme;
use nsh_core::prelude::*;

/// Bars in the live traffic chart: one per poll over the last minute.
const HISTORY_LEN: usize = 60;

pub fn overview(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(crate::shell::page_scroll(
            view()
                .flex_col()
                .gap_4()
                .child(connection_panel(store))
                .child(metrics_row(store))
                .child(traffic_card(store)),
        ))
        .child(controls::floating_layer(connect_action(store)))
}

/// Start and stop the core: 总览's one action.
///
/// Icon-only and named by the verb the press performs (启动代理, 断开, 正在启动…), so the state
/// the user is checking is written on the control that changes it. The name also stays a stable
/// selector while the glyph does the pointing.
fn connect_action(store: AppStore) -> impl IntoElement {
    controls::floating_action_state(
        // A binding, not a value: this body runs once, so reading `connected()` here would
        // freeze the glyph at whatever it was on the first frame.
        move || {
            if store.connected() {
                Icon::Power
            } else {
                Icon::Rocket
            }
        },
        move || crate::shell::connect_label(store).into(),
        // Lit means the core is up. The disc is the thing a user glances at to answer "is it
        // running?", and `app.css` lights its power button on exactly that fact.
        move || store.connected(),
        // With nothing to route and no core to talk to, connecting cannot succeed, so the
        // control says so instead of failing on press. Disconnect stays available.
        move || {
            store.core_action_busy()
                || (!store.connected()
                    && (store.nodes().is_empty() || store.core_state() == "unavailable"))
        },
        move || store.toggle_core(!store.connected()),
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
        // Connecting is not in this card any more: it is the dock's primary control, so it is
        // reachable from 节点 or 设置 without walking back here first. What stays is everything
        // that reads as an attribute of the connection rather than an act on it.
        // Gated on what the platform answered rather than on a build flag, so a browser
        // driving a desktop server keeps the switch it needs. See `SystemProxy::is_offered`.
        .child(when(
            move || store.system_proxy().is_offered(),
            move || system_proxy_toggle(store),
        ))
        .child(probe_row(store))
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
            text(move || i18n::tr(status_title(store)))
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || base_theme().card_foreground),
        )
        .child(
            text(move || format!("· {}", i18n::tr(domain::mode_label(store.tunnel_mode()))))
                .text_xs()
                .text_color(theme::text_soft()),
        )
}

/// The card's own state title. The verb of the connect action lives on the dock's button, in
/// `shell::connect_label`, where the control now is.
fn status_title(store: AppStore) -> &'static str {
    if store.system_proxy_busy() {
        "系统代理设置中"
    } else if store.core_action_busy() {
        "处理中"
    } else if store.connected() {
        "已连接"
    } else {
        "未连接"
    }
}

/// The switch that decides whether anything uses the port the core is listening on.
///
/// It is the difference between "connected" and "your traffic is going through it", and it
/// sits beside the connect control rather than on Settings because turning it on is part of
/// connecting, not a preference to go and find.
///
/// A parked switch says why. The one case that matters is trying to enable it before a core
/// is running: the setting would point the machine at a port with nothing behind it.
fn system_proxy_toggle(store: AppStore) -> impl IntoElement {
    let status = computed(move || store.system_proxy());
    let enabled = move || status.with(SystemProxy::is_enabled);
    // The backend's own gate, asked before the press rather than after it: a switch the
    // platform cannot honour, or that has no answer to act on, says so instead of failing.
    let disabled = move || {
        store.core_action_busy()
            || store.system_proxy_busy()
            || !status.with(SystemProxy::is_ready)
            || !status.with(SystemProxy::is_supported)
            || (!enabled() && !store.connected())
    };
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(move || i18n::tr("系统代理"))
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || i18n::tr(&proxy_note(store)))
                        .text_xs()
                        .text_color(theme::text_faint()),
                ),
        )
        .child(controls::switch_state(
            move || Some("系统代理".into()),
            enabled,
            disabled,
            move |next| store.set_system_proxy(next),
        ))
}

/// What the backend said about the operating system, or why it has not said it yet.
fn proxy_note(store: AppStore) -> String {
    store.system_proxy().note(
        store.connected(),
        store.core_action_busy(),
        store.system_proxy_busy(),
    )
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
                    text(move || i18n::tr("内核不可用，请重新检测；若仍失败，请检查服务是否运行、端口是否被占用。"))
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
    // The shared note, which is the app's one owner of "a sentence and the action that recovers
    // from it" — the sync card's failure above and the subscription card's hint already use it,
    // and it gives the sentence `flex_1` with a zero floor, which is the guard `probe_row` below
    // carries by hand. Assembled here instead, this row was a bare `justify_between` of the two:
    // a flex item's floor is its content size, so the sentence kept its intrinsic width and the
    // row laid 重新连接 off the right edge. Measured at 390pt in English — where the sentence asks
    // for 436pt against the 126pt it asks for in Chinese — the button reached x 584 of 390, so
    // the one action that applies the pending change was both unreadable and unreachable. The
    // note's frames are what `a_note_with_its_action_fits_a_phone` measures, which is why this
    // went to the control rather than to a third copy of that guard: this row needs a running
    // core, and the suite starts none.
    controls::inline_note_with(
        theme::Tint::Warning,
        Icon::CircleAlert,
        "本地配置已改动，运行中的内核还是旧的。",
        Some(controls::secondary_button("重新连接", move || {
            store.restart_core()
        })),
    )
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
                .child(
                    text(move || i18n::tr(label))
                        .text_xs()
                        .text_color(theme::text_faint()),
                ),
        )
        .child(metric_value(store, kind))
}

/// A figure and the line that explains it. Both are closures: this function runs once,
/// and a value read here would sit at whatever the profile held on the first frame.
///
/// `i18n::tr` is applied to both: each helper returns a key (`空闲`, `等待导入订阅`, a formatted
/// figure whose words are ours), and a closure that reads a key without looking it up is the
/// subtle version of the frozen-label bug — it re-runs correctly on every poll and still says the
/// same untranslated thing.
fn metric_value(store: AppStore, kind: MetricKind) -> impl IntoElement {
    view()
        .flex_col()
        .gap_0p5()
        .child(
            text(move || i18n::tr(&metric_figure(store, kind)))
                .text_size(px(22.0))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(theme::font_display())
                .text_color(move || base_theme().card_foreground),
        )
        .child(
            text(move || i18n::tr(metric_note(store, kind)))
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
                text(move || i18n::tr(&(chart_status(&history.get(), store.connected()))))
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

/// The port the core listens on, and whether a measurement is running. The act itself is the
/// dock's; this row keeps the two facts a user reads next to the traffic — the endpoint, and
/// that a probe is in flight — because the dock's button changes to say the same thing and a
/// glance at the chart is where that gets checked.
fn measure_row(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(move || domain::proxy_port_label(store.allow_lan(), store.language()))
                .text_xs()
                .font_family(theme::font_mono())
                .text_color(theme::text_faint()),
        )
        .child(when(
            move || store.latency_busy(),
            || {
                text(move || i18n::tr("正在测量"))
                    .text_xs()
                    .text_color(theme::text_faint())
            },
        ))
}

/// Does traffic actually get out?
///
/// The connection state cannot answer this. A running core with a listening port is a different
/// claim from traffic reaching the internet: the exit can be reachable enough to answer a latency
/// ping and useless for a real connection, and a provider can be up while the account behind it
/// is not. The probe dials out through the port, so it walks the whole chain the user depends on.
///
/// It sits with the connect control rather than among the figures because its answer is an
/// action's result, and it stays parked while the core is down: with nothing behind the port the
/// only honest answer is a connection refusal, which reads as "your proxy is broken" when the
/// truth is "you have not started it".
fn probe_row(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    // `flex_1` with a zero floor, exactly as the switch row above does it:
                    // without both, this label keeps its intrinsic width and the row lays the
                    // button off the right edge of a phone. `min_w(px(0.0))` is the part that
                    // matters — a flex item's floor is its content size, so `flex_1` alone
                    // cannot shrink a line below the text's own width. The label wraps rather
                    // than truncates because there is no tooltip here to recover what an
                    // ellipsis took, and the sentence is the reason the button is parked.
                    view().flex_1().min_w(px(0.0)).child(
                        text(move || i18n::tr(probe_hint(store)))
                            .text_xs()
                            .text_color(theme::text_faint()),
                    ),
                )
                .child(controls::secondary_button_state(
                    move || {
                        if store.probe_busy() {
                            "正在探测出口"
                        } else {
                            "测试连通"
                        }
                    },
                    // Busy, or nothing behind the port to probe through.
                    move || store.probe_busy() || !store.connected(),
                    move || store.probe_connectivity(),
                )),
        )
        .child(when(
            move || store.probe_result().is_some(),
            move || probe_result_line(store),
        ))
}

/// What the row says while it waits to be useful. A parked control names its own reason, as the
/// system-proxy switch does: an inert button with no explanation reads as a broken one, and here
/// the reason is a step the user has not taken yet rather than a fault.
fn probe_hint(store: AppStore) -> &'static str {
    if !store.connected() {
        "先启动代理，才能探测出口"
    } else if store.probe_busy() {
        "正在通过代理拨测外部地址"
    } else {
        "从代理拨一次外部地址，确认流量真的出得去"
    }
}

/// The last answer, kept on screen until the next press. A notice ages out while the user is
/// still working out what to do about a failure, and the reason is usually the useful part.
///
/// The whole note is rebuilt per answer rather than tinted through a binding, because
/// `inline_note` takes its color and glyph as values: the probe finishes on a press, so a
/// rebuild here costs nothing and reads clearer than a memo carried for the sake of one row.
fn probe_result_line(store: AppStore) -> impl IntoElement {
    dynamic(move || match store.probe_result() {
        Some(outcome) if outcome.ok => controls::inline_note(
            theme::Tint::Success,
            Icon::CircleCheck,
            SharedString::from(i18n::fill("连通正常：{}", &[&(outcome.detail)])),
        )
        .into_element(),
        Some(outcome) => controls::inline_note(
            theme::Tint::Danger,
            Icon::CircleAlert,
            SharedString::from(i18n::fill("出口不可用：{}", &[&(outcome.detail)])),
        )
        .into_element(),
        None => view().into_element(),
    })
}

fn chart_status(window: &crate::store::TrafficWindow, connected: bool) -> String {
    if window.active_connections > 0 {
        i18n::fill("{} 个活动连接", &[&(window.active_connections)])
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
        .child(
            text(move || i18n::tr(label))
                .text_xs()
                .text_color(theme::text_faint()),
        )
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
