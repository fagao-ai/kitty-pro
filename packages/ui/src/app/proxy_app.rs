//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

#[component]
pub fn ProxyApp(
    platform: String,
    /// Package version of the shell that mounted this UI (`CARGO_PKG_VERSION`).
    /// The update card compares it against the newest published release, so each
    /// shell reports its own build rather than the version of the shared `ui`
    /// crate it links.
    app_version: String,
    desktop_tray: Option<DesktopTrayBridge>,
) -> Element {
    use_context_provider(|| DesktopAutostartCapability(desktop_tray.is_some()));
    // One update check per app run, shared so the launch toast, the settings nav
    // dot, and the settings card all read the same answer.
    let mut update = use_context_provider(|| UpdateMonitor {
        status: Signal::new(None),
        saved: Signal::new(api::update::UpdateSettings::default()),
        busy: Signal::new(None),
        ready: Signal::new(false),
        announced: Signal::new(None),
        error: Signal::new(None),
        platform: Signal::new(platform.clone()),
        version: Signal::new(app_version.clone()),
    });
    let active_view = use_signal(|| AppView::Overview);
    let mut dark_mode = use_signal(|| false);
    let mut connected = use_signal(|| false);
    let core_busy = use_signal(|| false);
    let mut runtime_action_busy = use_signal(|| false);
    let core_restarting = use_signal(|| false);
    let mut core_state = use_signal(|| "checking".to_string());
    let mut core_version = use_signal(|| None::<String>);
    let mut core_note = use_signal(|| None::<String>);
    let toast = ToastManager {
        messages: use_signal(Vec::<ToastMessage>::new),
        next_id: use_signal(|| 0),
        tick: use_signal(|| 0u64),
    };
    use_context_provider(|| toast);
    use_effect(move || {
        toast.start_sweeper();
    });
    // "Local config drifted from what the running core applied." Set by
    // refreshes/imports/deletes/rule edits while connected; cleared by the
    // restart or start that ships the current config to the core.
    let mut config_pending = use_signal(|| false);
    use_context_provider(|| ConfigPending(config_pending));
    let mut nodes = use_signal(Vec::<ProxyNode>::new);
    let mut selected_tag = use_signal(String::new);
    let mut subscriptions = use_signal(Vec::<Subscription>::new);
    let mut custom_rules = use_signal(Vec::<CustomRule>::new);
    let mut dns_overrides = use_signal(Vec::<DnsOverride>::new);
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
    // Seconds since the core reached `running`; None while it is stopped. A plain
    // tick counter keeps this off platform clock APIs (wasm has none).
    let mut uptime_seconds = use_signal(|| None::<u64>);
    let mut uptime_generation = use_signal(|| 0_u32);
    let mut core_log_cursor = use_signal(|| 0_u64);
    let mut log_poll_generation = use_signal(|| 0_u64);
    let mut log_collection_paused = use_signal(|| false);
    let mut profile_loaded = use_signal(|| false);
    // Per-switch busy flags live at the root so the "处理中" spinner follows
    // the operation across pages: a local `use_signal` reset when the view
    // remounted, leaving a disabled switch with no explanation.
    let tun_busy = use_signal(|| false);
    let lan_busy = use_signal(|| false);
    let mut system_proxy = use_signal(|| SystemProxyLoadState::Loading);
    let mut system_proxy_busy = use_signal(|| false);
    // Log page state lives here so the chip filter and the search box stay in
    // sync with the topbar eyebrow that the rules tab also drives.
    let log_filter = use_signal(|| LogFilter::Routes);
    let log_search = use_signal(String::new);
    let rules_tab = use_signal(|| RulesTab::Routing);

    use_effect(move || {
        let Some(mut bridge) = desktop_tray else {
            return;
        };
        let (system_proxy_ready, system_proxy_supported, system_proxy_enabled) =
            match system_proxy() {
                SystemProxyLoadState::Ready(status) => (true, status.supported, status.enabled),
                SystemProxyLoadState::Loading | SystemProxyLoadState::Failed(_) => {
                    (false, false, false)
                }
            };
        bridge.state.set(DesktopTrayState {
            ready: profile_loaded(),
            busy: core_busy() || core_restarting() || system_proxy_busy() || runtime_action_busy(),
            connected: connected(),
            system_proxy_ready,
            system_proxy_supported,
            system_proxy_enabled,
            active_subscription_id: active_subscription_id(),
            selected_tag: selected_tag(),
            subscriptions: subscriptions()
                .into_iter()
                .map(|subscription| DesktopTraySubscription {
                    id: subscription.id,
                    name: subscription.name,
                    nodes: subscription
                        .nodes
                        .into_iter()
                        .map(|node| DesktopTrayNode {
                            tag: node.tag,
                            name: node.name,
                        })
                        .collect(),
                })
                .collect(),
        });
    });

    use_effect(move || {
        let Some(mut bridge) = desktop_tray else {
            return;
        };
        let Some(command) = (bridge.command)() else {
            return;
        };
        bridge.command.set(None);

        if !profile_loaded()
            || core_busy()
            || core_restarting()
            || system_proxy_busy()
            || runtime_action_busy()
        {
            return;
        }

        runtime_action_busy.set(true);
        spawn(async move {
            match command {
                DesktopTrayCommand::SelectSubscription(subscription_id) => {
                    if active_subscription_id() == Some(subscription_id) {
                        runtime_action_busy.set(false);
                        return;
                    }
                    if !subscriptions()
                        .iter()
                        .any(|subscription| subscription.id == subscription_id)
                    {
                        toast.error("托盘选择的订阅已不存在");
                        runtime_action_busy.set(false);
                        return;
                    }

                    let previous_subscription_id = active_subscription_id();
                    let previous_nodes = nodes();
                    let previous_tag = selected_tag();
                    let previous_selections = group_selections();
                    let active_nodes =
                        collect_subscription_nodes(&subscriptions(), Some(subscription_id));
                    let next_tag = select_available_tag(&active_nodes, "");
                    let mut next_selections = previous_selections.clone();
                    prune_group_selections(&mut next_selections, |_| false);
                    if next_tag.is_empty() {
                        next_selections.remove("proxy");
                    } else {
                        next_selections.insert("proxy".to_string(), next_tag.clone());
                    }

                    active_subscription_id.set(Some(subscription_id));
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
                        if !restart_core_from_ui(
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
                            "已从托盘切换订阅，内核已重启",
                        )
                        .await
                        {
                            active_subscription_id.set(previous_subscription_id);
                            nodes.set(previous_nodes);
                            selected_tag.set(previous_tag);
                            group_selections.set(previous_selections);
                        }
                    } else {
                        toast.success("已从托盘切换订阅");
                    }
                }
                DesktopTrayCommand::SelectNode(outbound) => {
                    if selected_tag() == outbound {
                        runtime_action_busy.set(false);
                        return;
                    }
                    let active_nodes = nodes();
                    if !active_nodes.iter().any(|node| node.tag == outbound) {
                        toast.error("托盘选择的节点已不存在");
                        runtime_action_busy.set(false);
                        return;
                    }

                    let previous_tag = selected_tag();
                    let previous_selections = group_selections();
                    let mut next_selections = previous_selections.clone();
                    next_selections.insert("proxy".to_string(), outbound.clone());
                    selected_tag.set(outbound.clone());

                    if connected() {
                        // TUN mode forwards this selector call to its existing
                        // privileged helper, so a normal node switch does not
                        // prompt for elevation or interrupt the tunnel.
                        if api::select_proxy_group("proxy".to_string(), outbound.clone())
                            .await
                            .is_ok()
                        {
                            group_selections.set(next_selections);
                            toast.success(format!("已从托盘切换到 {outbound}"));
                        } else {
                            // restart_core preflights the candidate and reuses
                            // a live TUN helper before stopping the old core.
                            group_selections.set(next_selections.clone());
                            let request = build_connection_request(
                                active_nodes,
                                outbound.clone(),
                                active_subscription_proxy_server_nameservers(
                                    &subscriptions(),
                                    active_subscription_id(),
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
                            if !restart_core_from_ui(
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
                                format!("已从托盘切换到 {outbound}，内核已重启"),
                            )
                            .await
                            {
                                selected_tag.set(previous_tag);
                                group_selections.set(previous_selections);
                            }
                        }
                    } else {
                        group_selections.set(next_selections);
                        toast.success(format!("已从托盘选择 {outbound}"));
                    }
                }
                DesktopTrayCommand::ToggleSystemProxy => match system_proxy() {
                    SystemProxyLoadState::Ready(status) if status.supported => {
                        let enabled = !status.enabled;
                        system_proxy_busy.set(true);
                        match api::set_system_proxy(enabled).await {
                            Ok(status) => {
                                system_proxy.set(SystemProxyLoadState::Ready(status));
                                if enabled {
                                    toast.success("系统代理已启用");
                                } else {
                                    toast.success("系统代理已关闭");
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
                    SystemProxyLoadState::Ready(_) => {
                        toast.error("当前平台不支持系统代理");
                    }
                    SystemProxyLoadState::Loading => {
                        toast.info("系统代理状态仍在读取");
                    }
                    SystemProxyLoadState::Failed(error) => {
                        toast.error(format!("系统代理状态不可用: {error}"));
                    }
                },
                DesktopTrayCommand::CopyEnvironmentVariables => {
                    match api::copy_proxy_environment_variables().await {
                        Ok(()) => toast.success("代理环境变量已复制"),
                        Err(error) => toast.error(format!("复制环境变量失败: {error}")),
                    }
                }
            }
            runtime_action_busy.set(false);
        });
    });

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
                    // One truth: a failed first read means "not connected",
                    // not the half-update that left the chip on 已连接.
                    connected.set(false);
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
                        // `connected` and `core_state` are one truth, not two:
                        // an errored poll must move both, or the topbar chip
                        // keeps saying 已连接 while the sidebar reads 内核不可用
                        // and the next power click toggles from a stale value.
                        connected.set(false);
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

    // Update check on launch: read what the last run remembered, then let the
    // server decide whether the day is up. A check the user did not ask for only
    // ever speaks up about a release worth installing - "already latest" is the
    // card's business, and a failure is the card's business too - so an offline
    // start costs the user nothing but a request they enabled.
    use_effect(move || {
        spawn(async move {
            if let Ok(saved) = api::update::load_update_settings().await {
                update.saved.set(saved);
            }
            update.ready.set(true);
            update.refresh(false, toast, true).await;
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
                    dns_overrides.set(profile.dns_overrides);
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
                Err(error) => {
                    // Start from an empty-but-editable profile. Gating
                    // `profile_loaded` on success only meant every later edit
                    // (subscriptions, rules, theme) silently never reached the
                    // disk and vanished on restart, while the tray ignored
                    // commands with no feedback. The backend kept a copy of
                    // the unreadable file beside the profile, so editing from
                    // empty costs nothing that cannot be recovered.
                    profile_loaded.set(true);
                    toast.error(format!(
                        "无法恢复本地配置，已从空配置开始；原文件已备份为 profile.corrupt.json（{error}）"
                    ));
                }
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
            dns_overrides: dns_overrides(),
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
            let mut download_history = traffic.peek().download_history.clone();
            let mut upload_history = traffic.peek().upload_history.clone();
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
                    download_history.push(download_bytes_per_second);
                    upload_history.push(upload_bytes_per_second);
                    while download_history.len() > TRAFFIC_HISTORY_LEN {
                        download_history.remove(0);
                        upload_history.remove(0);
                    }
                    traffic.set(TrafficDisplay {
                        upload_bytes_per_second,
                        download_bytes_per_second,
                        upload_total: current.upload_total,
                        download_total: current.download_total,
                        active_connections: current.active_connections,
                        download_history: download_history.clone(),
                        upload_history: upload_history.clone(),
                    });
                    previous = Some(current);
                }
                wait_for_traffic_tick().await;
            }
        });
    });

    use_effect(move || {
        let is_connected = connected();
        let generation = {
            let mut current = uptime_generation.write();
            *current = current.wrapping_add(1);
            *current
        };
        if !is_connected {
            uptime_seconds.set(None);
            return;
        }
        uptime_seconds.set(Some(0));
        let mut uptime_seconds = uptime_seconds;
        spawn(async move {
            let mut seconds = 0_u64;
            while *uptime_generation.peek() == generation && *connected.peek() {
                wait_for_traffic_tick().await;
                seconds = seconds.saturating_add(1);
                uptime_seconds.set(Some(seconds));
            }
        });
    });

    use_effect(move || {
        let enabled =
            core_state() == "running" && active_view() == AppView::Logs && !log_collection_paused();
        spawn(async move {
            if let Err(error) = api::set_core_log_collection(enabled).await {
                // The chip reads `log_collection_paused` as the truth; swallowing
                // the error left it showing 采集中 while the backend refused to
                // collect. Park the switch on the real outcome and say so.
                if enabled {
                    log_collection_paused.set(true);
                    toast.error(format!("无法开启日志采集: {error}"));
                }
            }
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
            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                &subscriptions(),
                active_subscription_id(),
            ),
            mode: tunnel_mode(),
            tun: tun_enabled(),
            allow_lan: allow_lan(),
            custom_rules: connection_custom_rules(
                config_script_enabled(),
                &config_script(),
                custom_rules(),
            ),
            dns_overrides: dns_overrides(),
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
    let eyebrow = match current_view {
        AppView::Rules if rules_tab() == RulesTab::Dns => "RULES / 专用 DNS",
        other => other.eyebrow(),
    };
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
                Brand { kicker: brand_kicker(core_version()) }
                div { class: "nav-group-label", "MAIN" }
                nav { class: "primary-nav", aria_label: "主导航",
                    NavItem { view: AppView::Overview, active_view }
                    NavItem {
                        view: AppView::Nodes,
                        active_view,
                        count: nodes().len().to_string(),
                    }
                    NavItem {
                        view: AppView::Subscriptions,
                        active_view,
                        count: subscriptions().len().to_string(),
                    }
                    NavItem {
                        view: AppView::Rules,
                        active_view,
                        count: (custom_rules().len() + dns_overrides().len()).to_string(),
                    }
                    NavItem {
                        view: AppView::Logs,
                        active_view,
                        count: core_logs().len().to_string(),
                    }
                    NavItem {
                        view: AppView::Settings,
                        active_view,
                        dot: update.has_actionable_update(),
                    }
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
                    div { class: "topbar-titles",
                        p { class: "eyebrow", "{eyebrow}" }
                        h1 { "{current_view.title()}" }
                    }
                    div { class: "topbar-actions",
                        div { class: connection_chip_class(connected(), &core_state()),
                            span { class: "status-badge-dot" }
                            {connection_chip_label(connected(), &core_state())}
                        }
                        RestartCoreButton {
                            status: CoreStatusSignals {
                                connected,
                                core_restarting,
                                core_state,
                                core_version,
                                core_note,
                                system_proxy,
                                config_pending,
                            },
                            settings: RuntimeSettings {
                                tunnel_mode,
                                tun_enabled,
                                allow_lan,
                                custom_rules,
                                dns_overrides,
                                config_script_enabled,
                                config_script,
                            },
                            nodes,
                            selected_tag,
                            subscriptions,
                            active_subscription_id,
                            group_selections,
                            core_busy,
                            runtime_action_busy,
                            system_proxy_busy,
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
                        // The design keeps the destructive action in the page header
                        // so it is reachable without scrolling the log list.
                        if current_view == AppView::Logs {
                            button {
                                class: "secondary-button log-clear-button",
                                title: "清空当前缓冲区中的日志",
                                disabled: core_logs().is_empty(),
                                onclick: move |_| core_logs.write().clear(),
                                Icon { icon: LdTrash2, width: 15, height: 15 }
                                span { "清空日志" }
                            }
                        }
                    }
                }

                match current_view {
                    AppView::Overview => rsx! {
                        OverviewView {
                            nodes,
                            selected_tag,
                            active_view,
                            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                                &subscriptions(),
                                active_subscription_id(),
                            ),
                            connected,
                            core_busy,
                            core_restarting,
                            core_state,
                            core_note,
                            core_version,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            custom_rules,
                            dns_overrides,
                            connection_allowed,
                            latency_results,
                            traffic,
                            uptime_seconds,
                            subscription_count: subscriptions().len(),
                            config_script_enabled,
                            config_script,
                            group_selections,
                            system_proxy,
                            system_proxy_busy,
                            runtime_action_busy,
                            dark_mode,
                            tun_busy,
                            lan_busy,
                        }
                    },
                    AppView::Nodes => rsx! {
                        NodesView {
                            groups: proxy_groups,
                            groups_loading: proxy_groups_loading,
                            groups_error: proxy_groups_error,
                            nodes: nodes(),
                            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                                &subscriptions(),
                                active_subscription_id(),
                            ),
                            all_count: nodes().len(),
                            subscriptions: subscriptions(),
                            group_selections,
                            connected,
                            core_busy,
                            core_restarting,
                            system_proxy_busy,
                            runtime_action_busy,
                            core_state,
                            core_version,
                            core_note,
                            system_proxy,
                            selected_tag,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            custom_rules,
        dns_overrides,
                            config_script_enabled,
                            config_script,
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
                            connected,
                            core_busy,
                            core_restarting,
                            system_proxy_busy,
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
                            runtime_action_busy,
                        }
                    },
                    AppView::Rules => rsx! {
                        RulesView {
                            rules: custom_rules,
                            dns_overrides,
                            active_tab: rules_tab,
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
                            core_version,
                            filter: log_filter,
                            search: log_search,
                            start_allowed: connection_allowed,
                            start_busy: core_busy()
                                || core_restarting()
                                || system_proxy_busy()
                                || runtime_action_busy(),
                            on_start: move |_| {
                                let target = !connected();
                                if !target || nodes().is_empty() {
                                    return;
                                }
                                spawn_core_toggle(
                                    build_connection_request(
                                        nodes(),
                                        selected_tag(),
                                        active_subscription_proxy_server_nameservers(
                                            &subscriptions(),
                                            active_subscription_id(),
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
                                        group_selections(),
                                    )
                                    .into(),
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
                        }
                    },
                    AppView::Settings => rsx! {
                        SettingsView {
                            platform: platform.clone(),
                            app_version: app_version.clone(),
                            tun_busy,
                            lan_busy,
                            core_state,
                            core_version,
                            core_note,
                            system_proxy,
                            connected,
                            core_busy,
                            core_restarting,
                            system_proxy_busy,
                            tunnel_mode,
                            tun_enabled,
                            allow_lan,
                            dark_mode,
                            nodes,
                            selected_tag,
                            subscriptions,
                            active_subscription_id,
                            proxy_server_nameservers: active_subscription_proxy_server_nameservers(
                                &subscriptions(),
                                active_subscription_id(),
                            ),
                            custom_rules,
        dns_overrides,
                            config_script_enabled,
                            config_script,
                            group_selections,
                            runtime_action_busy,
                        }
                    },
                }
            }

            nav { class: "mobile-nav glass-surface", aria_label: "移动端导航",
                // The phone tab bar stays count-free: six slots leave no room for chips.
                NavItem { view: AppView::Overview, active_view, count: None::<String> }
                NavItem { view: AppView::Nodes, active_view, count: None::<String> }
                NavItem { view: AppView::Subscriptions, active_view, count: None::<String> }
                NavItem { view: AppView::Rules, active_view, count: None::<String> }
                NavItem { view: AppView::Logs, active_view, count: None::<String> }
                NavItem {
                    view: AppView::Settings,
                    active_view,
                    count: None::<String>,
                    dot: update.has_actionable_update(),
                }
            }

            if import_open() {
                DialogShell {
                    panel_id: "import-dialog".to_string(),
                    class: "modal glass-modal",
                    label: "添加订阅".to_string(),
                    dismissible: !import_busy(),
                    on_close: move |_| import_open.set(false),
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
                                placeholder: "订阅地址，或代理 URL（HTTP/HTTPS/SOCKS5）",
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
                            disabled: import_busy()
                                || core_busy()
                                || core_restarting()
                                || import_source().trim().is_empty(),
                            onclick: move |_| async move {
                                import_busy.set(true);
                                import_error.set(None);
                                let source = import_source();
                                match api::preview_subscription(source.clone()).await {
                                    Ok(report) if !report.nodes.is_empty() => {
                                        let ParseReport {
                                            nodes: report_nodes,
                                            rejected,
                                            proxy_server_nameservers,
                                        } = report;
                                        let count = report_nodes.len();
                                        let rejected = rejected.len();
                                        let id = next_subscription_id(&subscriptions());
                                        let parsed_nodes = namespace_nodes(id, report_nodes);
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
                                            proxy_server_nameservers,
                                            rejected_count: rejected,
                                        });
                                        if connected() {
                                            // The new subscription is not what
                                            // the core runs; switching to it (or
                                            // restarting) is a separate action,
                                            // so flag the drift like refresh does.
                                            config_pending.set(true);
                                            toast.success(format!(
                                                "已导入 {name}，共 {count} 个节点；当前连接未变化"
                                            ));
                                        } else {
                                            active_subscription_id.set(Some(id));
                                            let active_nodes = collect_subscription_nodes(
                                                &subscriptions(),
                                                Some(id),
                                            );
                                            let next_tag = select_available_tag(&active_nodes, "");
                                            let mut next_selections = group_selections();
                                            if next_tag.is_empty() {
                                                next_selections.remove("proxy");
                                            } else {
                                                next_selections
                                                    .insert("proxy".to_string(), next_tag.clone());
                                            }
                                            nodes.set(active_nodes);
                                            selected_tag.set(next_tag);
                                            group_selections.set(next_selections);
                                            toast.success(format!("已导入并切换到 {name}，共 {count} 个节点"));
                                        }
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

            ToastViewport {}
        }
    }
}
