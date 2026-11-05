//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

#[component]
pub(crate) fn NodesView(
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
    system_proxy: Signal<SystemProxyLoadState>,
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
    // U11: share the big, rarely-changing tables through Rc handles. The old
    // code cloned the whole `nodes` Vec once per member row (O(n²)); memos and
    // rows now take a cheap Rc clone instead of a full copy.
    let groups = Rc::new(groups);
    let nodes = Rc::new(nodes);
    let subscriptions = Rc::new(subscriptions);
    let proxy_server_nameservers = Rc::new(proxy_server_nameservers);
    // F20: cancellation flag for the latency poll loop.
    let mut latency_cancel = use_signal(|| false);
    // Dedicated cheap handles for the refresh click handler: a `move` event
    // closure must not steal the Rc tables the rest of the RSX still needs.
    // The owned Vec copies are taken on click, not on every render.
    let refresh_nodes = nodes.clone();
    let refresh_nameservers = proxy_server_nameservers.clone();

    // The body still needs to know whether the search box is empty (for the
    // empty-state copy). The memos below re-read the signals themselves so
    // they stay tracked dependencies and only recompute on a real input change.
    let needle_empty = search().trim().is_empty();

    // U10: one memo for the whole group/member derivation. It tracks search(),
    // group_selections() and latency_results(), so an unrelated re-render
    // (e.g. the 1 Hz traffic tick) reuses the cached tree instead of doing the
    // per-outbound linear finds again.
    let groups_for_memo = groups.clone();
    let nodes_for_memo = nodes.clone();
    let groups_with_members = use_memo(move || {
        let needle = search().trim().to_ascii_lowercase();
        let latency_snapshot = latency_results();
        let selections = group_selections();
        groups_for_memo
            .iter()
            .map(|group| {
                let mut members =
                    proxy_group_members(group, &groups_for_memo, &nodes_for_memo, &needle);
                sort_proxy_group_members(&mut members, &latency_snapshot);
                let selected = selections
                    .get(&group.tag)
                    .filter(|selected| group.outbounds.iter().any(|item| item == *selected))
                    .cloned()
                    .unwrap_or_else(|| group.selected.clone());
                (group.clone(), members, selected)
            })
            .filter(|(_, members, _)| !members.is_empty())
            .collect::<Vec<_>>()
    });

    // U10: the two counters derive from the memo above plus the latency map.
    let nodes_for_stats = nodes.clone();
    let node_stats = use_memo(move || {
        let latency_snapshot = latency_results();
        let listed_member_count = groups_with_members
            .read()
            .iter()
            .map(|(_, members, _)| members.len())
            .sum::<usize>();
        // Count only nodes that exist in the current subscription: after a
        // switch the map still holds the previous provider's results, and the
        // old count claimed "已测速 30 个节点" above a list of `-- ms` rows.
        let listed_tags: HashMap<&str, ()> = nodes_for_stats
            .iter()
            .map(|node| (node.tag.as_str(), ()))
            .collect();
        let tested_count = latency_snapshot
            .iter()
            .filter(|(tag, result)| {
                result.latency_ms.is_some() && listed_tags.contains_key(tag.as_str())
            })
            .count();
        (listed_member_count, tested_count)
    });

    // U10: the focus card resolves several linear finds; memoize it too.
    let groups_for_focus = groups.clone();
    let nodes_for_focus = nodes.clone();
    let subs_for_focus = subscriptions.clone();
    let focus = use_memo(move || {
        node_focus_details(
            &groups_for_focus,
            &nodes_for_focus,
            &subs_for_focus,
            &selected_tag(),
            &latency_results(),
        )
    });

    // Read once into plain values for the RSX below.
    let (listed_member_count, tested_count) = node_stats();
    let focus = focus();
    let groups_with_members = groups_with_members();

    rsx! {
        section { class: "workspace-section glass-surface",
            if all_count == 0 {
                div { class: "large-empty", role: "status",
                    EmptyNodes {}
                    button {
                        class: "primary-button",
                        onclick: move |_| import_open.set(true),
                        Icon { icon: LdPlus, width: 17, height: 17 }
                        "添加订阅"
                    }
                }
            } else if groups_loading {
                div { class: "large-empty", role: "status",
                    span { class: "spinner large", aria_hidden: "true" }
                    strong { "正在生成代理组" }
                }
            } else if let Some(error) = groups_error {
                div { class: "large-empty", role: "alert",
                    Icon { icon: LdCircleAlert, width: 28, height: 28 }
                    strong { "代理组生成失败" }
                    p { "{error}" }
                }
            } else if groups.is_empty() {
                div { class: "large-empty", role: "status",
                    Icon { icon: LdRoute, width: 28, height: 28 }
                    strong { "配置中没有代理组" }
                }
            } else {
                div { class: "nodes-toolbar",
                    label { class: "search-field nodes-search",
                        Icon { icon: LdSearch, width: 17, height: 17 }
                        input {
                            value: search,
                            aria_label: "搜索节点、地址或协议",
                            placeholder: "搜索节点、地址或协议",
                            oninput: move |event| search.set(event.value()),
                        }
                    }
                                    button {
                                        class: "secondary-button compact",
                                        title: if latency_busy() { "正在刷新延迟" } else { "刷新延迟" },
                                        aria_label: "刷新节点延迟",
                                        aria_busy: latency_busy(),
                                        disabled: latency_busy() || nodes.is_empty(),
                                        onclick: move |_| {
                                            // U9: arm synchronously, before the
                                            // spawn, so a double click cannot
                                            // start two overlapping sessions.
                                            if *latency_busy.peek() {
                                                return;
                                            }
                                            latency_busy.set(true);
                                            latency_cancel.set(false);
                                            let probe_nodes = (*refresh_nodes).clone();
                                            let cache_nodes = (*refresh_nodes).clone();
                                            let proxy_server_nameservers =
                                                (*refresh_nameservers).clone();
                                            spawn(async move {
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
                                                // F20: bound the poll by ~60s
                                                // (600 × the 100ms tick) and
                                                // stop early on user cancel.
                                                const MAX_LATENCY_POLLS: usize = 600;
                                                let mut polls = 0usize;
                                                let mut cancelled = false;
                                                let mut timed_out = false;
                                                let completed = loop {
                                                    if *latency_cancel.peek() {
                                                        cancelled = true;
                                                        break 0;
                                                    }
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
                                                            // Keep what already
                                                            // arrived: dropping
                                                            // the batch meant a
                                                            // late failure threw
                                                            // away a full run of
                                                            // measurements.
                                                            let partial = latency_results();
                                                            persist_latency_cache(
                                                                &cache_nodes,
                                                                &partial,
                                                            )
                                                            .await;
                                                            toast.error(format!("读取测速结果失败: {error}"));
                                                            latency_busy.set(false);
                                                            return;
                                                        }
                                                    }
                                                    polls += 1;
                                                    if polls >= MAX_LATENCY_POLLS {
                                                        timed_out = true;
                                                        break 0;
                                                    }
                                                    wait_for_latency_poll().await;
                                                };
                                                if cancelled {
                                                    toast.info("已停止测速，保留已返回的结果");
                                                } else if timed_out {
                                                    toast.info("测速超时，已保留已返回的结果");
                                                } else if failures == 0 {
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
                                            });
                                        },
                                        if latency_busy() {
                                            span { class: "spinner", aria_hidden: "true" }
                                        } else {
                                            Icon { icon: LdRefreshCw, width: 16, height: 16 }
                                        }
                                        span { class: "nodes-refresh-label", "刷新延迟" }
                                    }
                                    // F20: an explicit stop control; the loop
                                    // above polls `latency_cancel` each tick.
                                    if latency_busy() {
                                        button {
                                            class: "secondary-button compact",
                                            title: "停止测速",
                                            aria_label: "停止测速",
                                            onclick: move |_| latency_cancel.set(true),
                                            Icon { icon: LdX, width: 16, height: 16 }
                                            span { class: "nodes-refresh-label", "停止测速" }
                                        }
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
                    div { class: "large-empty group-empty", role: "status",
                        Icon { icon: LdSearch, width: 28, height: 28 }
                        strong {
                            if needle_empty { "配置中没有代理组" } else { "没有匹配的节点" }
                        }
                        p {
                            // F17: no internal "selector/urltest" jargon.
                            if needle_empty {
                                "该订阅没有分组信息，可直接在下方选择节点"
                            } else {
                                "换个关键词试试，节点名、地址和协议都可以搜索。"
                            }
                        }
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
                                        aria_hidden: "true",
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
                                            key: "{member.tag}",
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
                                            system_proxy,
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
                          // F20: a selected node whose last probe failed is
                          // still selectable, but must say so out loud.
                          if focus.status_class == "failed" {
                              div { class: "inline-note info", role: "status", style: "color: #e5484d",
                                  Icon { icon: LdCircleAlert, width: 15, height: 15 }
                                  span { "该节点最近一次探测失败，可能无法连接；建议重新测速或改选其它节点。" }
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
pub(crate) struct NodeFocusDetails {
    pub(crate) name: String,
    pub(crate) protocol: String,
    pub(crate) endpoint: String,
    pub(crate) group_label: String,
    pub(crate) status: String,
    pub(crate) status_class: &'static str,
    pub(crate) source: String,
}

pub(crate) fn node_focus_details(
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
    // Q1：focus.source 是「展示用」字段，必须经 source_label 脱敏后再进 RSX
    // （nodes.rs 中 `code { "{focus.source}" }` 会把它写进 DOM）。source_label
    // 只保留 scheme+host，去掉 userinfo 与 query/fragment/path，因此含 token /
    // 密码的完整订阅 URL 不会泄露给用户。这里只替换展示值；传给内核 / 下载 /
    // 解析器的真实 subscription.source 原样保留，不受影响。
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
pub(crate) struct ProxyGroupMember {
    pub(crate) tag: String,
    pub(crate) node: Option<ProxyNode>,
    pub(crate) nested_kind: Option<ProxyGroupKind>,
}

/// Members of one proxy group after the free-text filter. Members can be real
/// nodes or nested groups, so both are resolved against the shared lists.
pub(crate) fn proxy_group_members(
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
pub(crate) fn ProxyGroupMemberRow(
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
    system_proxy: Signal<SystemProxyLoadState>,
    // U11: shared Rc tables instead of a full Vec clone for every member row.
    nodes: Rc<Vec<ProxyNode>>,
    proxy_server_nameservers: Rc<Vec<String>>,
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
    // Provided by ProxyApp; cleared centrally when a restart applies the config.
    let config_pending = use_context::<ConfigPending>().0;
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
                // Take owned Vecs only on click; the Rc handles stay out of the
                // async task (which must not carry a non-Send Rc).
                let nodes = (*nodes).clone();
                let proxy_server_nameservers = (*proxy_server_nameservers).clone();
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
                                        system_proxy,
                                        config_pending,
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
                    aria_hidden: "true",
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
                span { class: "selected-check", aria_hidden: "true", Icon { icon: LdCircleCheck, width: 19, height: 19 } }
            } else if selectable {
                span { class: "row-chevron", aria_hidden: "true", Icon { icon: LdChevronRight, width: 18, height: 18 } }
            } else {
                span { class: "auto-indicator", aria_hidden: "true", Icon { icon: LdGauge, width: 17, height: 17 } }
            }
        }
    }
}

pub(crate) fn proxy_group_kind_label(kind: ProxyGroupKind) -> &'static str {
    match kind {
        ProxyGroupKind::Selector => "手动选择",
        ProxyGroupKind::UrlTest => "自动测速",
    }
}

pub(crate) fn outbound_display_name(tag: &str, nodes: &[ProxyNode]) -> String {
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

/// The overview's four-row node preview. This is a *preview*, not a selector:
/// writing `selected_tag` here only relabels the header while the core keeps
/// running the old node (and the next start sends the new `selected_tag` with
/// the stale `group_selections`), so a tap opens the nodes page where picking
/// actually applies. The check mark still shows which node is live.
#[component]
pub(crate) fn NodeRow(
    node: ProxyNode,
    selected_tag: Signal<String>,
    latency_results: Signal<HashMap<String, NodeLatency>>,
    mut on_open: EventHandler<()>,
) -> Element {
    let selected = selected_tag() == node.tag;
    let (latency_label, latency_class, latency_title) =
        format_latency(latency_results().get(&node.tag));
    rsx! {
        button {
            class: if selected { "node-row selected" } else { "node-row" },
            title: "在节点页切换",
            onclick: move |_| on_open.call(()),
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
                span { class: "selected-check", aria_hidden: "true", Icon { icon: LdCircleCheck, width: 19, height: 19 } }
            } else {
                span { class: "row-chevron", aria_hidden: "true", Icon { icon: LdChevronRight, width: 18, height: 18 } }
            }
        }
    }
}

/// Latency column: meter bar plus the monospace millisecond figure from the
/// design. The meter only fills once a probe really returned a value.
#[component]
pub(crate) fn LatencyCell(
    result: Option<NodeLatency>,
    latency_label: String,
    latency_class: &'static str,
    latency_title: String,
) -> Element {
    let (fill, tone) = latency_meter(result.as_ref());
    // F20: flag probes that came back with no measurement at all.
    let probe_failed = result
        .as_ref()
        .is_some_and(|entry| entry.latency_ms.is_none());
    rsx! {
        span { class: "latency-cell", title: latency_title,
            span { class: "latency-meter", aria_hidden: "true",
                span { class: "latency-meter-fill {tone}", style: "width: {fill}%" }
            }
            if probe_failed {
                // Existing class + inline style (no new CSS class). The mark is
                // decorative: the "失败" label and the tooltip already name it.
                span { class: "latency error", style: "color: #e5484d",
                    aria_hidden: "true", title: "该节点探测失败", "⚠"
                }
            }
            span { class: latency_class, "{latency_label}" }
        }
    }
}

#[component]
pub(crate) fn EmptyNodes() -> Element {
    rsx! {
        div { class: "empty-state",
            span { class: "empty-icon", aria_hidden: "true", Icon { icon: LdGlobe, width: 26, height: 26 } }
            div {
                strong { "还没有节点" }
                p { "添加订阅后会显示在这里" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_source_never_leaks_subscription_credentials() {
        // Q1：展示用脱敏函数必须去掉 userinfo / query / fragment，只保留
        // scheme+host，不能把含 token / 密码的完整订阅 URL 写进 DOM。
        let sensitive = "https://user:s3cr3t@sub.example.com/path?token=TOKEN123&x=1#frag";
        let label = source_label(sensitive);
        assert_eq!(label, "https://sub.example.com");
        assert!(!label.contains("s3cr3t"));
        assert!(!label.contains("TOKEN123"));
        assert!(!label.contains("user:"));
        assert!(!label.contains("token"));
    }

    #[test]
    fn node_focus_details_source_is_redacted_before_render() {
        // 端到端：node_focus_details 产出的 source 字段（即 RSX 渲染的
        // "{focus.source}"）不含订阅 URL 里的凭据；真实 subscription.source
        // 保持原样，仍可用于下载 / 刷新。
        let parsed =
            proxy_core::parse_subscription("trojan://password@edge.example.com:443#Edge").nodes;
        let node = parsed[0].clone();
        let secret_url = "https://alice:p%40ss@sub.example.com/sub?token=SECRET99";
        let subscriptions = vec![Subscription {
            id: 1,
            name: "Primary".to_string(),
            source: secret_url.to_string(),
            nodes: vec![node.clone()],
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        }];
        let groups = vec![ProxyGroup {
            tag: "proxy".to_string(),
            kind: ProxyGroupKind::Selector,
            outbounds: vec![node.tag.clone()],
            selected: node.tag.clone(),
        }];
        let focus = node_focus_details(
            &groups,
            &subscriptions[0].nodes,
            &subscriptions,
            &node.tag,
            &HashMap::new(),
        )
        .expect("focused node should resolve");

        assert_eq!(focus.source, "https://sub.example.com");
        assert!(!focus.source.contains("SECRET99"));
        assert!(!focus.source.contains("alice"));
        assert!(!focus.source.contains("token"));
        // 真实值未被展示函数改写。
        assert_eq!(subscriptions[0].source, secret_url);
    }
}
