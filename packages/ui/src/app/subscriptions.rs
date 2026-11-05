//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

/// F19: a monotone arm token shared with every row through context so the
/// 3 s auto-expiry can tell a fresh arm from a stale one. Kept out of the props
/// so `SubscriptionRow`'s signature (and `ProxyApp`'s wiring) stays unchanged.
#[derive(Clone, Copy)]
pub(crate) struct ConfirmArm(pub(crate) Signal<u64>);

/// U9: one busy slot per refresh target. A finishing single-row refresh must
/// not clear the marker for an in-flight "refresh all" (and vice versa). The
/// existing `Signal<Option<RefreshTarget>>` prop stays the display marker;
/// `pending` holds one entry per actually-running refresh.
#[derive(Clone, Copy)]
pub(crate) struct RefreshGuard {
    pub(crate) display: Signal<Option<RefreshTarget>>,
    pub(crate) pending: Signal<Vec<RefreshTarget>>,
}

impl RefreshGuard {
    /// Synchronous claim (peek-based, so a double click cannot slip through
    /// the render tick). Returns false when this target is already running.
    pub(crate) fn claim(self, target: RefreshTarget) -> bool {
        if self.pending.peek().contains(&target) {
            return false;
        }
        let mut pending = self.pending;
        pending.write().push(target);
        let mut display = self.display;
        display.set(Some(target));
        true
    }

    /// Release only this target's slot; the display marker falls back to the
    /// remaining in-flight target, or `None` when the last one finishes.
    pub(crate) fn release(self, target: RefreshTarget) {
        {
            let mut pending = self.pending;
            pending.write().retain(|item| *item != target);
        }
        let mut display = self.display;
        display.set(self.pending.peek().last().copied());
    }
}

/// F19: a shared 3 s wait for two-step confirmations (wasm-safe, mirrors the
/// timers in `types.rs`). Used by the subscription rows and the sync card.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn wait_for_confirm_expiry() {
    gloo_timers::future::TimeoutFuture::new(3_000).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn wait_for_confirm_expiry() {
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
}

/// F18: one prune implementation shared by the single-row refresh and the
/// refresh-all path. `live_tags` must be every node tag that still exists after
/// the refresh; stale namespaced outbounds are dropped so they cannot be sent
/// with the next `ConnectionRequest.group_selections`. Non-node choices
/// (built-in group names, `direct`, ...) are left alone.
pub(crate) fn prune_stale_group_selections(
    selections: &mut HashMap<String, String>,
    live_tags: &[String],
) {
    prune_group_selections(selections, |outbound| {
        !outbound.starts_with("subscription-") || live_tags.iter().any(|tag| tag == outbound)
    });
}

/// F18: every node tag that still exists across all subscriptions, used as the
/// `live_tags` input so both refresh paths prune against the same full set.
pub(crate) fn all_live_node_tags(subscriptions: &[Subscription]) -> Vec<String> {
    subscriptions
        .iter()
        .flat_map(|subscription| subscription.nodes.iter().map(|node| node.tag.clone()))
        .collect()
}

/// Signals `refresh_all_subscriptions` writes back into. Bundled so the
/// subscription page (F18/U9) and the sync page (F5/F6) call one function.
#[derive(Clone, Copy)]
pub(crate) struct RefreshAllSignals {
    pub(crate) subscriptions: Signal<Vec<Subscription>>,
    pub(crate) active_subscription_id: Signal<Option<u64>>,
    pub(crate) nodes: Signal<Vec<ProxyNode>>,
    pub(crate) selected_tag: Signal<String>,
    pub(crate) group_selections: Signal<HashMap<String, String>>,
    pub(crate) toast: ToastManager,
}

/// Refresh every subscription, then rebuild the derived state with the same
/// domain helpers the single-row refresh uses. Returns how many sources
/// refreshed cleanly and emits the success/failure toast.
pub(crate) async fn refresh_all_subscriptions(
    sources: Vec<(u64, String)>,
    signals: RefreshAllSignals,
) -> usize {
    let RefreshAllSignals {
        mut subscriptions,
        mut active_subscription_id,
        mut nodes,
        mut selected_tag,
        mut group_selections,
        toast,
    } = signals;
    let mut refreshed = 0;
    // Name the failed subscriptions: "1 个失败" gave no idea WHICH provider
    // is down, and a user with four subscriptions cannot act on that.
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
                    Ok(_) => refreshed += 1,
                    Err(reason) => failures.push(format!("{name}: {reason}")),
                }
            }
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    // F5/F6: a pulled snapshot leaves `active_subscription_id = None` and an
    // empty node list, so `connection_allowed` can never become true again.
    // Resolve a real active subscription and rebuild nodes/tag/selections.
    let next_active_id =
        resolve_active_subscription_id(&subscriptions(), active_subscription_id(), &selected_tag());
    let active_nodes = collect_subscription_nodes(&subscriptions(), next_active_id);
    let next_tag = select_available_tag(&active_nodes, &selected_tag());
    // F18: the exact same prune the single-row refresh runs, so a removed
    // outbound can never linger and ride along in the next ConnectionRequest.
    let mut selections = group_selections();
    prune_stale_group_selections(&mut selections, &all_live_node_tags(&subscriptions()));
    group_selections.set(selections);
    active_subscription_id.set(next_active_id);
    nodes.set(active_nodes);
    selected_tag.set(next_tag);
    if failures.is_empty() {
        toast.success("订阅已刷新，可重新连接或启动");
    } else {
        toast.error(format!(
            "已刷新 {refreshed} 个订阅，{} 个失败：{}",
            failures.len(),
            failures.join("；")
        ));
    }
    refreshed
}

/// U9: claim synchronously, then release on every exit path. Shared by the
/// toolbar "refresh all" button and the empty-node recovery button.
pub(crate) fn start_refresh_all(
    guard: RefreshGuard,
    signals: RefreshAllSignals,
    connected: Signal<bool>,
    config_pending: ConfigPending,
) {
    if !guard.claim(RefreshTarget::All) {
        return;
    }
    let subs_signal = signals.subscriptions;
    let sources: Vec<(u64, String)> = subs_signal()
        .into_iter()
        .map(|item| (item.id, item.source))
        .collect();
    spawn(async move {
        let refreshed = refresh_all_subscriptions(sources, signals).await;
        // A refresh while connected still leaves the core on the old node set;
        // flag the drift instead of pretending the new nodes are live.
        if refreshed > 0 && connected() {
            let mut pending = config_pending.0;
            pending.set(true);
        }
        guard.release(RefreshTarget::All);
    });
}

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
    let config_pending = use_context::<ConfigPending>();
    let mut confirm_delete = use_signal(|| None::<u64>);
    let confirm_arm = use_signal(|| 0u64);
    use_context_provider(|| ConfirmArm(confirm_arm));

    // U10: derive the list once; the memo only changes when the subscription
    // vec actually changes, instead of cloning it on every render.
    #[allow(clippy::redundant_closure)]
    let subscription_rows = use_memo(move || subscriptions());
    // U9: one busy slot per target, provided to the rows through context.
    let refresh_pending = use_signal(Vec::<RefreshTarget>::new);
    let refresh_guard = RefreshGuard {
        display: refresh_busy,
        pending: refresh_pending,
    };
    use_context_provider(|| refresh_guard);

    // F19: a two-step delete arm expires after 3 s. `confirm_arm` is bumped on
    // every arm so a re-arm of the same row is a fresh arm and a pending timer
    // cannot cancel it early. Switching pages unmounts this component, which
    // resets the arm by itself.
    use_effect(move || {
        let arm = confirm_arm();
        if confirm_delete().is_none() {
            return;
        }
        spawn(async move {
            wait_for_confirm_expiry().await;
            if *confirm_arm.peek() == arm {
                confirm_delete.set(None);
            }
        });
    });

    let refresh_args = RefreshAllSignals {
        subscriptions,
        active_subscription_id,
        nodes,
        selected_tag,
        group_selections,
        toast,
    };
    let rows = subscription_rows();

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
                        aria_label: "刷新全部订阅",
                        aria_busy: refresh_busy().is_some(),
                        disabled: rows.is_empty() || refresh_busy().is_some(),
                        onclick: move |_| {
                            // F19: any other interaction cancels an armed delete.
                            confirm_delete.set(None);
                            start_refresh_all(refresh_guard, refresh_args, connected, config_pending);
                        },
                        if refresh_busy() == Some(RefreshTarget::All) {
                            span { class: "spinner" }
                        } else {
                            Icon { icon: LdRefreshCw, width: 17, height: 17 }
                        }
                    }
                    button {
                        class: "primary-button",
                        onclick: move |_| {
                            confirm_delete.set(None);
                            import_open.set(true);
                        },
                        Icon { icon: LdPlus, width: 17, height: 17 }
                        "添加订阅"
                    }
                }
            }
            if rows.is_empty() {
                // U20/F11: an empty state needs a CTA and a purpose, not just
                // a label. `import_open` is the same signal the topbar uses.
                div { class: "large-empty subscriptions-empty",
                    span { class: "empty-icon", Icon { icon: LdRadioTower, width: 28, height: 28 } }
                    strong { role: "status", "暂无订阅" }
                    p { class: "inline-note",
                        "订阅用于导入代理节点；添加后即可在节点页挑选线路、回到概览连接。也可点击右上角「添加订阅」。"
                    }
                    button {
                        class: "primary-button compact",
                        onclick: move |_| {
                            confirm_delete.set(None);
                            import_open.set(true);
                        },
                        Icon { icon: LdPlus, width: 17, height: 17 }
                        "添加订阅"
                    }
                }
            } else {
                // F5/F10: a pulled snapshot (or a failed refresh) can leave
                // subscriptions present but zero usable nodes. Give a real
                // "next step" instead of only a toast.
                if nodes().is_empty() {
                    div { class: "inline-note sync-security-note",
                        Icon { icon: LdInfo, width: 16, height: 16 }
                        span { "订阅已更新但节点尚未解析，连接前请先刷新全部订阅。" }
                        button {
                            class: "secondary-button compact",
                            disabled: refresh_busy().is_some(),
                            onclick: move |_| {
                                confirm_delete.set(None);
                                start_refresh_all(refresh_guard, refresh_args, connected, config_pending);
                            },
                            "一键刷新全部订阅并恢复使用"
                        }
                    }
                }
                div { class: "subscription-list",
                    // U11: a stable key keeps row state aligned when the list
                    // is reordered or a row is removed.
                    for subscription in rows.iter().cloned() {
                        SubscriptionRow {
                            key: "{subscription.id}",
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
    let confirm_arm = use_context::<ConfirmArm>().0;
    let refresh_guard = use_context::<RefreshGuard>();
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
                    // F19: any other click cancels a pending delete arm.
                    confirm_delete.set(None);
                    // U9: peek-based re-entry guard, set synchronously.
                    if *runtime_action_busy.peek() {
                        return;
                    }
                    let blocked = core_busy()
                        || core_restarting()
                        || system_proxy_busy()
                        || runtime_action_busy();
                    if blocked {
                        return;
                    }
                    runtime_action_busy.set(true);
                    spawn(async move {
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
                    });
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
                aria_label: "刷新订阅",
                aria_busy: refreshing,
                disabled: refresh_busy().is_some(),
                onclick: move |_| {
                    // F19: any other click cancels a pending delete arm.
                    confirm_delete.set(None);
                    // U9: synchronous claim + peek-based re-entry guard.
                    if !refresh_guard.claim(RefreshTarget::One(subscription_id)) {
                        return;
                    }
                    let source = subscription_source.clone();
                    spawn(async move {
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
                                            // F18: the exact same prune the
                                            // refresh-all path runs, against
                                            // the full live node set.
                                            let mut selections = group_selections();
                                            prune_stale_group_selections(
                                                &mut selections,
                                                &all_live_node_tags(&subscriptions()),
                                            );
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
                        // U9: release on every exit path.
                        refresh_guard.release(RefreshTarget::One(subscription_id));
                    });
                },
                if refreshing {
                    span { class: "spinner" }
                } else {
                    Icon { icon: LdRefreshCw, width: 17, height: 17 }
                }
            }
            ConfirmButton {
                label: "删除订阅".to_string(),
                confirm_label: "再次点击确认删除订阅".to_string(),
                disabled: refresh_busy().is_some(),
                // F19: the old title promised "点击其它位置取消" but nothing
                // implemented it; other controls still cancel via the shared
                // arm generation, and the second tap is worded honestly.
                title: Some("删除订阅".to_string()),
                extra_class: "icon-button danger subscription-delete-button".to_string(),
                compact: true,
                onarm: move |armed: bool| {
                    // Keep the ConfirmArm generation and the list-level arm
                    // signal in step with the button's own two-step state.
                    if armed {
                        let next_arm = *confirm_arm.peek() + 1;
                        let mut arm = confirm_arm;
                        arm.set(next_arm);
                    }
                    confirm_delete.set(None);
                },
                onconfirm: move |_| {
                    let deleting_active = active_subscription_id() == Some(subscription_id);
                    subscriptions.write().retain(|item| item.id != subscription_id);
                    let mut selections = group_selections();
                    prune_stale_group_selections(
                        &mut selections,
                        &all_live_node_tags(&subscriptions()),
                    );
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
            }
        }
    }
}
