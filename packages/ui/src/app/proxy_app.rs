//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

/// 首屏状态读取的容错次数：心跳间隔约 1s，累计失败约 8 次（≈8s）仍读不到
/// 内核就落到明确的错误态，避免 `core_state` 永远停在 `"checking"`。
const CORE_STATUS_FIRST_READ_ATTEMPTS: u32 = 8;

/// 跨平台短延迟（防抖 / 确认复位）：wasm 走 gloo-timers，native 走 tokio。
async fn delay_ms(millis: u32) {
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(millis).await;
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(std::time::Duration::from_millis(millis as u64)).await;
}

/// F13：统一取消 / 关闭导入对话框。递增代际让在途解析结果失效，并复位
/// busy / open，保证迟到的结果不会重新打开对话框或覆盖新状态。
fn cancel_import(mut generation: Signal<u64>, mut busy: Signal<bool>, mut open: Signal<bool>) {
    let mut current = generation.write();
    *current = current.wrapping_add(1);
    busy.set(false);
    open.set(false);
}

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
    // U14：本次会话是否手动切换过主题。手动切换后系统明暗偏好不再覆盖用户选择。
    let mut theme_user_overridden = use_signal(|| false);
    // 从磁盘恢复的 dark_mode 也是一次显式选择，优先级高于系统偏好；
    // 防止 profile 加载与系统主题 eval 的异步竞态把用户保存的主题覆盖掉。
    let mut theme_restored_from_profile = use_signal(|| false);
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
    // U14：首屏跟随系统明暗偏好。仅在用户本次会话尚未手动切换主题时应用，
    // 且只在挂载后执行一次：effect 主体不订阅任何 signal，所有读取与判断都
    // 放进 `spawn(async move { ... })` 的异步块里。
    // 跨目标说明：`document::eval` 在 wasm32 与桌面 / 移动 webview 均可用；
    // 本项目 `chrome.rs` 的 DialogShell 焦点脚本、下方延迟缓存恢复都已无条件
    // 调用该 API（含 `recv::<T>().await`），native 与 wasm 均可编译。因此这里
    // 不使用 `#[cfg(target_arch = "wasm32")]` 门控——只对 wasm32 开放反而会让
    // 桌面 webview 失去主题跟随能力；已保存配置 / 其它页面主题开关若在 eval
    // 异步窗口内改动了 dark_mode，也不会被这次系统值覆盖。
    use_effect(move || {
        spawn(async move {
            // 用户本次会话已手动切换过主题 → 直接返回，尊重用户选择。
            if theme_user_overridden() || theme_restored_from_profile() {
                return;
            }
            let mut eval = document::eval(
                "dioxus.send(window.matchMedia('(prefers-color-scheme: dark)').matches);",
            );
            let Ok(prefers_dark) = eval.recv::<bool>().await else {
                // 无 DOM / eval 失败时保持原默认值，不阻断首屏。
                return;
            };
            // eval 为异步：返回期间用户可能已手动切换，或 profile 已恢复完成，
            // 二次校验后再写入，保证任何显式选择都不会被系统偏好覆盖。
            if !theme_user_overridden() && !theme_restored_from_profile() {
                dark_mode.set(prefers_dark);
            }
        });
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

    // U13：profile 保存代际序号，用于丢弃防抖窗口内的过期快照。
    let mut save_generation = use_signal(|| 0u64);
    // F11：首次引导卡关闭后，本次会话不再显示。
    let mut onboarding_dismissed = use_signal(|| false);
    // F12：导入对话框「支持的格式」说明的展开状态。
    let mut import_formats_open = use_signal(|| false);
    // F13：每次导入 / 取消都自增，让在途解析结果失效，避免迟到的结果
    // 重新打开对话框或覆盖新状态。
    let mut import_generation = use_signal(|| 0u64);
    // U14：清空日志两步确认的就绪状态与自动复位代际。
    let mut clear_logs_armed = use_signal(|| false);
    let mut clear_logs_arm_generation = use_signal(|| 0u64);

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

    // 全平台统一的 core_status 心跳：桌面/Web 以前只在启动时读一次，核心
    // 意外停止后会永久显示「已连接」；现在和 Android 一样按心跳持续回读，
    // `connected` / `core_state` / `core_version` / `core_note` 始终同一真相。
    // U23：核心状态心跳属于「常驻」轮询，任何视图下都必须持续运行，绝不做
    // active_view 门控；其余非核心轮询（流量 / 日志 / 规则集更新 / 代理组预览）
    // 才按「对应视图可见才发请求」门控，切到别的视图即停发请求。
    use_effect(move || {
        spawn(async move {
            let mut failed_attempts: u32 = 0;
            // 是否曾成功读到一次状态：读到之后再失败说明核心在运行中掉线，
            // 必须立刻落到不可用，不给首屏的容错窗口。
            let mut first_read_done = false;
            loop {
                match api::core_status().await {
                    Ok(status) => {
                        first_read_done = true;
                        failed_attempts = 0;
                        let is_running = status.state == "running";
                        connected.set(is_running);
                        #[cfg(target_os = "android")]
                        {
                            if is_running && toast.contains(ANDROID_VPN_WAITING_NOTICE) {
                                toast.dismiss_message(ANDROID_VPN_WAITING_NOTICE);
                                toast.success("Android VPN 已连接");
                            }
                        }
                        core_state.set(status.state);
                        core_version.set(status.version);
                        // 覆盖首屏「正在连接内核…」的等待提示。
                        core_note.set(status.note);
                    }
                    Err(error) => {
                        connected.set(false);
                        if first_read_done {
                            core_state.set("unavailable".to_string());
                            core_note.set(Some(error.to_string()));
                        } else {
                            failed_attempts = failed_attempts.saturating_add(1);
                            if failed_attempts >= CORE_STATUS_FIRST_READ_ATTEMPTS {
                                core_state.set("unavailable".to_string());
                                core_note.set(Some(format!(
                                    "无法连接内核，请检查服务是否运行（{error}）"
                                )));
                            } else {
                                core_note.set(Some(format!(
                                    "正在连接内核，第 {failed_attempts} 次重试…（{error}）"
                                )));
                            }
                        }
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

    // U23：规则集更新检查（6h 一次）为「按需」轮询——仅当「规则」视图可见时
    // 才发请求；其它视图跳过本次请求、继续等待下一个 tick，不删除轮询本身。
    use_effect(move || {
        spawn(async move {
            loop {
                if active_view() == AppView::Rules {
                    let _ = api::update_rule_sets(false).await;
                }
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
                    theme_restored_from_profile.set(true);
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

    // U13：防抖 + 代际。快照在 effect 同步阶段一次性构造好，async 里不再
    // 读任何 signal（否则延迟期间的变化会被重新订阅）。延迟结束后只有
    // 代际仍是最新才真正落盘，过期的快照直接丢弃，避免 last-write-wins。
    use_effect(move || {
        if !profile_loaded() {
            return;
        }
        let generation = {
            let mut current = save_generation.write();
            *current = current.wrapping_add(1);
            *current
        };
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
            delay_ms(300).await;
            if *save_generation.peek() != generation {
                return;
            }
            if let Err(error) = api::save_profile(profile).await {
                toast.error(format!("本地配置保存失败: {error}"));
            }
        });
    });

    // U14：清空日志确认态 3 秒后自动复位。代际校验保证「复位 → 再次 armed」
    // 时旧定时器不会提前把新的确认态清掉。
    use_effect(move || {
        if !clear_logs_armed() {
            return;
        }
        let generation = {
            let mut current = clear_logs_arm_generation.write();
            *current = current.wrapping_add(1);
            *current
        };
        spawn(async move {
            delay_ms(3_000).await;
            if *clear_logs_arm_generation.peek() == generation {
                clear_logs_armed.set(false);
            }
        });
    });

    // U14：切换视图（或点击其它位置）时复位确认态。
    use_effect(move || {
        let _ = active_view();
        clear_logs_armed.set(false);
    });

    // U23：流量轮询为「按需」——仅概览视图可见且内核运行时才发请求；概览
    // 不可见时同步阶段直接 return，循环内也持续用 active_view 复核。
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

    // U23：日志采集开关与日志读取轮询均为「按需」——仅日志视图可见且内核
    // 运行时才启用 / 发请求；切到其它视图会自动停用采集并停止轮询。
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

    // U23：代理组预览为「按需」——仅在节点页可见时才发起较重的预览请求；
    // 读取 active_view 让视图切换时重新求值，其它视图下直接返回空结果。
    let proxy_groups_resource = use_resource(move || {
        let nodes_visible = active_view() == AppView::Nodes;
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
            // U23：节点页不可见时不发预览请求，避免 6 个视图都触发较重计算。
            if !nodes_visible {
                return Ok(Vec::new());
            }
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
    // U6/U14/U22：让 title / aria_label / 可见文本共享同一个文案来源。
    let theme_label = if dark_mode() {
        "切换浅色外观"
    } else {
        "切换深色外观"
    };
    let clear_logs_label = if clear_logs_armed() {
        "再次点击确认清空"
    } else {
        "清空日志"
    };
    let import_formats_label = if import_formats_open() {
        "收起支持的格式"
    } else {
        "支持的格式"
    };

    rsx! {
        document::Style { "{APP_CSS}" }
        div { class: root_class,
            div { class: "ambient-grid" }
            aside { class: "sidebar glass-surface",
                Brand { kicker: brand_kicker(core_version()) }
                nav { class: "primary-nav", aria_label: "主导航",
                    // F22：6 个真实 AppView 平铺项分成 3 组。同步 / 更新是设置页
                    // 内的功能，不是独立 AppView，故不单列，避免编造不存在的视图。
                    // 窄屏 tabbar 仍平铺、不分组；这里只调整桌面侧栏 DOM 与文案。
                    NavGroup { title: "连接".to_string(),
                        NavItem { view: AppView::Overview, active_view }
                        NavItem {
                            view: AppView::Nodes,
                            active_view,
                            count: nodes().len().to_string(),
                        }
                    }
                    NavGroup { title: "配置".to_string(),
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
                            view: AppView::Settings,
                            active_view,
                            dot: update.has_actionable_update(),
                        }
                    }
                    NavGroup { title: "运维".to_string(),
                        NavItem {
                            view: AppView::Logs,
                            active_view,
                            count: core_logs().len().to_string(),
                        }
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
                            title: theme_label,
                            aria_label: theme_label,
                            aria_pressed: dark_mode(),
                            onclick: move |_| {
                                // U14：本次会话用户已手动选择主题，系统偏好不再覆盖。
                                theme_user_overridden.set(true);
                                dark_mode.toggle();
                            },
                            if dark_mode() {
                                Icon { icon: LdSun, width: 19, height: 19 }
                            } else {
                                Icon { icon: LdMoon, width: 19, height: 19 }
                            }
                        }
                        if matches!(current_view, AppView::Overview | AppView::Nodes | AppView::Subscriptions) {
                            button {
                                class: "primary-button compact",
                                aria_label: "添加订阅",
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
                                title: clear_logs_label,
                                aria_label: clear_logs_label,
                                disabled: core_logs().is_empty(),
                                onclick: move |_| {
                                    if clear_logs_armed() {
                                        core_logs.write().clear();
                                        clear_logs_armed.set(false);
                                    } else {
                                        clear_logs_armed.set(true);
                                    }
                                },
                                Icon { icon: LdTrash2, width: 15, height: 15 }
                                span { "{clear_logs_label}" }
                            }
                        }
                    }
                }

                // F11：首次引导。仅在配置已加载且没有任何订阅时出现，关闭后
                // 本次会话不再显示。只用既有 CSS 类 + 少量内联 style。
                if profile_loaded() && subscriptions().is_empty() && !onboarding_dismissed() {
                    div {
                        class: "glass-surface",
                        style: "margin: 16px 0; padding: 18px; display: flex; flex-direction: column; gap: 12px;",
                        div { class: "section-heading",
                            h2 { "开始使用 Kitty Pro" }
                        }
                        ol {
                            style: "margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 6px;",
                            li { "第一步 · 添加订阅：粘贴订阅链接或代理分享内容" }
                            li { "第二步 · 选择节点：在节点页挑选一条可用线路" }
                            li { "第三步 · 启动代理：回到概览点击电源按钮连接" }
                        }
                        p { class: "inline-note", "完成以上三步即可开始使用代理；引导卡可随时关闭。" }
                        div {
                            style: "display: flex; gap: 10px; flex-wrap: wrap;",
                            button {
                                class: "primary-button",
                                aria_label: "添加订阅",
                                onclick: move |_| {
                                    import_error.set(None);
                                    import_open.set(true);
                                },
                                "添加订阅"
                            }
                            button {
                                class: "secondary-button",
                                onclick: move |_| onboarding_dismissed.set(true),
                                "不再显示"
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
                    // F13：解析期间也允许关闭 / 取消，在途结果用代际丢弃。
                    dismissible: true,
                    on_close: move |_| cancel_import(import_generation, import_busy, import_open),
                    div { class: "modal-header",
                        div {
                            p { class: "eyebrow", "SUBSCRIPTIONS / 添加订阅" }
                            h2 { "添加订阅" }
                        }
                        button {
                            class: "icon-button",
                            title: "关闭",
                            aria_label: "关闭",
                            onclick: move |_| cancel_import(import_generation, import_busy, import_open),
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
                        // F12：字段为空时给出浅色提示，而不是只让按钮变灰。
                        if import_source().trim().is_empty() {
                            p { class: "inline-note", "粘贴订阅链接、Clash YAML 或单节点 URL 后即可导入。" }
                        }
                        // F12：可折叠的「支持的格式」说明。
                        div {
                            style: "display: flex; flex-direction: column; gap: 8px;",
                            button {
                                class: "secondary-button",
                                aria_expanded: "{import_formats_open()}",
                                onclick: move |_| import_formats_open.toggle(),
                                "{import_formats_label}"
                            }
                            if import_formats_open() {
                                div { class: "inline-note",
                                    p { "当前支持以下格式：" }
                                    ul {
                                        style: "margin: 0; padding-left: 20px;",
                                        li { "Base64 分享链接订阅" }
                                        li { "Clash YAML 的 proxies 列表" }
                                        li { "单节点 socks5://host:port#名称" }
                                        li { "单节点 http://host:port#名称" }
                                        li { "单节点 https://user:pass@host:port#名称" }
                                        li { "Hysteria2 / VMess / VLESS / Trojan / Shadowsocks 分享链接" }
                                    }
                                }
                            }
                        }
                        if let Some(error) = import_error() {
                            div { class: "form-error",
                                Icon { icon: LdCircleAlert, width: 16, height: 16 }
                                span { "{error}" }
                            }
                            // F13：解析失败时把「支持的格式」说明直接展开。
                            button {
                                class: "secondary-button",
                                onclick: move |_| import_formats_open.set(true),
                                "查看支持格式"
                            }
                        }
                    }
                    div { class: "modal-actions",
                        button {
                            class: "secondary-button",
                            onclick: move |_| cancel_import(import_generation, import_busy, import_open),
                            "取消"
                        }
                        button {
                            class: "primary-button",
                            disabled: import_busy()
                                || core_busy()
                                || core_restarting()
                                || import_source().trim().is_empty(),
                            onclick: move |_| async move {
                                let generation = {
                                    let mut current = import_generation.write();
                                    *current = current.wrapping_add(1);
                                    *current
                                };
                                import_busy.set(true);
                                import_error.set(None);
                                let source = import_source();
                                let result = api::preview_subscription(source.clone()).await;
                                // F13：用户已关闭 / 取消 / 重新发起导入，代际
                                // 已变化，丢弃迟到结果，不重开对话框也不覆盖新状态。
                                if *import_generation.peek() != generation {
                                    return;
                                }
                                match result {
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
                                        // F4：无论是否连接都切换到新订阅，让概览
                                        // 「重新连接」横幅真正用上新订阅的节点。
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
                                        if connected() {
                                            config_pending.set(true);
                                            toast.success(format!(
                                                "已导入并切换到 {name}，共 {count} 个节点，重新连接后生效"
                                            ));
                                        } else {
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
                                "解析中（可取消）"
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
