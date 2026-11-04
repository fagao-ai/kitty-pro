//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

/// A reader counts as "following" while the newest row is this close to the top
/// of the console. Rows are 46px tall on desktop, so 8px only ever absorbs the
/// fractional offset a wheel/touch fling leaves behind.
pub(crate) const LOG_FOLLOW_THRESHOLD_PX: f64 = 8.0;

#[component]
pub(crate) fn LogsView(
    mut logs: Signal<Vec<CoreLogEntry>>,
    connected: Signal<bool>,
    mut collection_paused: Signal<bool>,
    core_version: Signal<Option<String>>,
    mut filter: Signal<LogFilter>,
    mut search: Signal<String>,
    /// False when the power button itself is disabled (no nodes, or the core
    /// is unavailable); the empty-state start affordance must not stay live
    /// where the overview refuses the same action.
    start_allowed: bool,
    start_busy: bool,
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

    // The console is newest-first, so "following" simply means being parked at the
    // top: prepending a row never moves the viewport (the browser anchors the
    // scroll to the row the reader is looking at), so new entries appear without
    // ever yanking the page. Scroll away and the reader keeps their place, but
    // then rows only ever arrive above the fold - the chip below is what makes
    // them reachable again.
    let mut following = use_signal(|| true);
    let mut unseen = use_signal(|| 0_u32);
    let mut newest_seen = use_signal(|| None::<u64>);
    let mut list_element = use_signal(|| None::<Rc<MountedData>>);

    use_effect(move || {
        // Read the buffer through the signal so every poll batch reruns this.
        let entries = logs.read();
        let Some(&newest) = entries.last().map(|entry| &entry.sequence) else {
            // Buffer emptied (the clear button, or the page being reopened): the
            // console has no scroll position left to be "away" from.
            if *unseen.peek() != 0 {
                unseen.set(0);
            }
            if !*following.peek() {
                following.set(true);
            }
            return;
        };
        let previous = *newest_seen.peek();
        newest_seen.set(Some(newest));
        // First batch since the page opened has nothing "missed", and a sequence
        // that goes backwards means the buffer was cleared or the core restarted.
        let Some(previous) = previous else {
            return;
        };
        if newest <= previous {
            return;
        }
        if *following.peek() {
            if *unseen.peek() != 0 {
                unseen.set(0);
            }
            return;
        }
        let added = log_entries_newer_than(&entries, previous);
        let pending = *unseen.peek();
        unseen.set(pending.saturating_add(added));
    });

    let track_follow = move |event: Event<ScrollData>| {
        let at_latest = event.scroll_top().abs() <= LOG_FOLLOW_THRESHOLD_PX;
        if *following.peek() != at_latest {
            following.set(at_latest);
        }
        if at_latest && *unseen.peek() != 0 {
            unseen.set(0);
        }
    };
    let jump_to_latest = move |_| {
        following.set(true);
        unseen.set(0);
        let Some(element) = list_element.cloned() else {
            return;
        };
        spawn(async move {
            let _ = element
                .scroll(PixelsVector2D::new(0.0, 0.0), ScrollBehavior::Instant)
                .await;
        });
    };
    let unseen_count = unseen();

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
                            disabled: !start_allowed || start_busy,
                            onclick: move |_| on_start.call(()),
                            if start_busy {
                                span { class: "spinner" }
                            } else {
                                Icon { icon: LdPlay, width: 16, height: 16 }
                            }
                            span { "启动代理开始采集" }
                        }
                    }
                    small { class: "log-empty-hint", "{core_build_hint(core_version(), connected())}" }
                }
            } else {
                div { class: "log-list", aria_label: "内核日志",
                    onscroll: track_follow,
                    onmounted: move |event| {
                        // The list mounts at the top, so it mounts following: the
                        // reader is looking at the newest row whatever state the
                        // page was left in when the console emptied and refilled.
                        following.set(true);
                        unseen.set(0);
                        list_element.set(Some(event.data()));
                    },
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
                div { class: "log-list-footer",
                    div { class: "log-follow-state", role: "status", aria_live: "polite",
                        if unseen_count > 0 {
                            button {
                                r#type: "button",
                                class: "log-follow-button",
                                title: "滚到最新一条",
                                onclick: jump_to_latest,
                                Icon { icon: LdArrowDownFromLine, width: 14, height: 14 }
                                "{unseen_count} 条新日志"
                            }
                        }
                    }
                    span { class: "log-list-count", "当前显示 {visible_count} 条" }
                }
            }
        }
    }
}

#[component]
pub(crate) fn LogRow(entry: CoreLogEntry) -> Element {
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
                // Rows stay one line on desktop, so carry the whole message in
                // the tooltip: these strings hold URLs and error details that
                // are unrecoverable once ellipsised.
                code { class: "log-message", title: entry.message.clone(), "{entry.message}" }
            } else {
                span { class: level_class, "{level_label}" }
                code { class: "log-message raw-message", title: entry.message.clone(), "{entry.message}" }
            }
        }
    }
}

pub(crate) fn log_matches_filter(entry: &CoreLogEntry, filter: LogFilter) -> bool {
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

/// How many buffered rows arrived after `previous`, i.e. what a reader who
/// scrolled away from the newest entry has missed. Counting sequences instead of
/// `entries.len()` keeps a trimmed buffer (the poll drops the oldest rows past
/// 500) from under-reporting.
pub(crate) fn log_entries_newer_than(entries: &[CoreLogEntry], previous: u64) -> u32 {
    entries
        .iter()
        .filter(|entry| entry.sequence > previous)
        .count() as u32
}

/// Newest first, after the chip filter and the free-text query: this is the one
/// list the page renders, so the counts beside the chips cannot drift from it.
pub(crate) fn visible_log_entries(
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

pub(crate) fn log_matches_search(entry: &CoreLogEntry, query: &str) -> bool {
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
pub(crate) fn route_chain_tags(route: &RouteLogDetail) -> Element {
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

pub(crate) fn route_decision_label(decision: RouteDecision) -> &'static str {
    match decision {
        RouteDecision::Direct => "直连",
        RouteDecision::Proxy => "代理",
        RouteDecision::Block => "拦截",
    }
}

pub(crate) fn route_target_kind_label(kind: RouteTargetKind) -> &'static str {
    match kind {
        RouteTargetKind::Domain => "域名",
        RouteTargetKind::Ip => "IP",
    }
}
