//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

#[component]
pub(crate) fn SubscriptionsView(
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
    system_proxy: Signal<SystemProxyLoadState>,
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
    let mut config_pending = use_context::<ConfigPending>();
    let mut confirm_delete = use_signal(|| None::<u64>);
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
                            confirm_delete.set(None);
                            let sources: Vec<(u64, String)> = subscriptions()
                                .into_iter()
                                .map(|subscription| (subscription.id, subscription.source))
                                .collect();
                            async move {
                                refresh_busy.set(Some(RefreshTarget::All));
                                let mut refreshed = 0;
                                // Name the failed subscriptions: "1 个失败" gave
                                // no idea WHICH provider is down, and a user
                                // with four subscriptions cannot act on that.
                                let mut failures: Vec<String> = Vec::new();
                                for (subscription_id, source) in sources {
                                    let name = subscriptions()
                                        .iter()
                                        .find(|item| item.id == subscription_id)
                                        .map(|item| item.name.clone())
                                        .unwrap_or_else(|| format!("#{subscription_id}"));
                                    match api::preview_subscription(source).await {
                                        Ok(report) => {
                                            let result = {
                                                let mut stored = subscriptions.write();
                                                apply_subscription_report(&mut stored, subscription_id, report)
                                            };
                                            match result {
                                                Ok(_) => {
                                                    refreshed += 1;
                                                    if connected() {
                                                        config_pending.0.set(true);
                                                    }
                                                }
                                                Err(reason) => failures.push(format!("{name}: {reason}")),
                                            }
                                        }
                                        Err(error) => failures.push(format!("{name}: {error}")),
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
                                if failures.is_empty() {
                                    toast.success(format!("已刷新 {refreshed} 个订阅"));
                                } else {
                                    toast.error(format!(
                                        "已刷新 {refreshed} 个订阅，{} 个失败：{}",
                                        failures.len(),
                                        failures.join("；")
                                    ));
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
                            confirm_delete,
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
                            system_proxy,
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
pub(crate) fn SubscriptionRow(
    subscription: Subscription,
    /// Row id currently armed for delete confirmation (tap trash again to
    /// confirm). Destructive one-tap deletes with no undo need a second step;
    /// an inline arm/disarm keeps it lighter than a dialog.
    mut confirm_delete: Signal<Option<u64>>,
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
    system_proxy: Signal<SystemProxyLoadState>,
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
    // Provided by ProxyApp; cleared centrally when a restart applies the config.
    let mut config_pending = use_context::<ConfigPending>().0;
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
                        // Other groups belong to the old subscription's config;
                        // carrying them over resurrects stale outbounds.
                        let mut next_selections = previous_selections.clone();
                        prune_group_selections(&mut next_selections, |_| false);
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
                                    system_proxy,
                                    config_pending,
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
                                    let touched_active = active_subscription_id()
                                        == Some(subscription_id);
                                    if touched_active {
                                        let active_nodes = collect_subscription_nodes(
                                            &subscriptions(),
                                            Some(subscription_id),
                                        );
                                        let next_tag = select_available_tag(
                                            &active_nodes,
                                            &selected_tag(),
                                        );
                                        // Outbounds of THIS subscription that the
                                        // refresh removed would otherwise stay
                                        // remembered in group_selections and go
                                        // out with the next start.
                                        let prefix = format!("subscription-{subscription_id}-");
                                        let live_tags = active_nodes
                                            .iter()
                                            .map(|node| node.tag.clone())
                                            .collect::<Vec<_>>();
                                        let mut selections = group_selections();
                                        prune_group_selections(&mut selections, |outbound| {
                                            !outbound.starts_with(&prefix)
                                                || live_tags.iter().any(|tag| tag == outbound)
                                        });
                                        group_selections.set(selections);
                                        nodes.set(active_nodes);
                                        selected_tag.set(next_tag);
                                        // A refresh never restarts the core:
                                        // while connected, the running config
                                        // still holds the old node set, so flag
                                        // the drift instead of pretending.
                                        if connected() {
                                            config_pending.set(true);
                                        }
                                    }
                                    toast.success(if touched_active && connected() {
                                        format!("已刷新 {count} 个节点，重新连接后生效")
                                    } else {
                                        format!("已刷新 {count} 个节点")
                                    });
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
                class: if confirm_delete() == Some(subscription_id) {
                    "icon-button danger armed subscription-delete-button"
                } else {
                    "icon-button danger subscription-delete-button"
                },
                aria_label: if confirm_delete() == Some(subscription_id) {
                    "再次点击确认删除订阅"
                } else {
                    "删除订阅"
                },
                title: if confirm_delete() == Some(subscription_id) {
                    "再次点击确认删除；点击其它位置取消"
                } else {
                    "删除订阅"
                },
                disabled: refresh_busy().is_some(),
                onclick: move |_| {
                    if confirm_delete() != Some(subscription_id) {
                        confirm_delete.set(Some(subscription_id));
                        return;
                    }
                    confirm_delete.set(None);
                    let deleting_active = active_subscription_id() == Some(subscription_id);
                    subscriptions.write().retain(|item| item.id != subscription_id);
                    let prefix = format!("subscription-{subscription_id}-");
                    let mut selections = group_selections();
                    prune_group_selections(&mut selections, |outbound| {
                        !outbound.starts_with(&prefix)
                    });
                    group_selections.set(selections);
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
                    // The core still runs the deleted subscription's nodes
                    // until a restart; flag the drift and name the new active
                    // subscription when the delete silently swapped one.
                    if connected() {
                        config_pending.set(true);
                    }
                    let swapped_to = deleting_active.then(|| {
                        subscriptions()
                            .iter()
                            .find(|item| Some(item.id) == next_active_id)
                            .map(|item| item.name.clone())
                    });
                    let mut message = "订阅已删除".to_string();
                    if let Some(Some(name)) = swapped_to {
                        message.push_str(&format!("，当前订阅已切换到「{name}」"));
                    }
                    if connected() {
                        message.push_str("，重新连接后生效");
                    }
                    toast.success(message);
                },
                Icon { icon: LdTrash2, width: 17, height: 17 }
            }
        }
    }
}
