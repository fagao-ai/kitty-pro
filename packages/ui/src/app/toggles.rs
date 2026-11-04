//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

#[component]
pub(crate) fn SystemProxyToggle(
    core_running: bool,
    core_action_busy: bool,
    mut system_proxy: Signal<SystemProxyLoadState>,
    mut system_proxy_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let proxy_busy = system_proxy_busy();
    let proxy = system_proxy_view(system_proxy(), proxy_busy);
    let SystemProxyView {
        label: proxy_label,
        detail: proxy_detail,
        enabled: proxy_enabled,
        supported: proxy_supported,
        ready: proxy_ready,
        loading: proxy_loading,
        busy: _,
    } = proxy;

    rsx! {
        label {
            class: "overview-proxy-toggle",
            title: "{proxy_detail}",
            div {
                strong { "系统代理" }
                small { "{proxy_label}" }
            }
            if proxy_loading || proxy_busy {
                span {
                    class: if proxy_enabled { "switch switch-loading active" } else { "switch switch-loading" },
                    aria_busy: "true",
                    aria_label: if proxy_busy { "正在设置系统代理" } else { "正在读取系统代理状态" },
                    span { class: "spinner" }
                }
            } else {
                input {
                    r#type: "checkbox",
                    aria_label: "系统代理",
                    checked: proxy_enabled,
                    disabled: core_action_busy
                        || !proxy_ready
                        || !proxy_supported
                        || (!proxy_enabled && !core_running),
                    onchange: move |event| {
                        let enabled = event.checked();
                        let blocked = core_action_busy || system_proxy_busy();
                        if !blocked {
                            system_proxy_busy.set(true);
                        }
                        async move {
                            if blocked {
                                return;
                            }
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
                    },
                }
                span { class: "switch" }
            }
        }
    }
}

/// Derived display state of the system proxy control, shared by the desktop
/// toggle row and the phone quick toggle tile.
pub(crate) struct SystemProxyView {
    pub(crate) label: &'static str,
    pub(crate) detail: String,
    pub(crate) enabled: bool,
    pub(crate) supported: bool,
    pub(crate) ready: bool,
    pub(crate) loading: bool,
    pub(crate) busy: bool,
}

pub(crate) fn system_proxy_view(state: SystemProxyLoadState, busy: bool) -> SystemProxyView {
    let (status, loading, error) = match state {
        SystemProxyLoadState::Loading => (None, true, None),
        SystemProxyLoadState::Ready(status) => (Some(status), false, None),
        SystemProxyLoadState::Failed(error) => (None, false, Some(error)),
    };
    let enabled = status.as_ref().is_some_and(|status| status.enabled);
    let supported = status.as_ref().is_some_and(|status| status.supported);
    let label = if busy {
        "设置中"
    } else if loading {
        "读取中"
    } else if enabled {
        "已启用"
    } else if error.is_some() {
        "读取失败"
    } else if !supported {
        "不可用"
    } else {
        "未启用"
    };
    let detail = if busy {
        if enabled {
            "正在关闭系统代理".to_string()
        } else {
            "正在应用到系统网络服务".to_string()
        }
    } else {
        status
            .as_ref()
            .map(|status| status.detail.clone())
            .or(error)
            .unwrap_or_else(|| "正在读取系统代理状态".to_string())
    };
    SystemProxyView {
        label,
        detail,
        enabled,
        supported,
        ready: status.is_some(),
        loading,
        busy,
    }
}

/// One tile of the phone quick-toggle grid: icon plus status dot on the first
/// row, name and monospace value below, matching the design's 2x2 grid.
#[component]
pub(crate) fn QuickToggle(
    icon: String,
    title: String,
    value: String,
    checked: bool,
    busy: bool,
    disabled: bool,
    hint: String,
    /// What applying this switch costs (restarts the core, touches the OS...)
    /// so four identical-looking tiles do not hide three different impacts.
    #[props(default)]
    cost: String,
    /// Why the switch cannot be used right now. A disabled toggle with a
    /// static hint reads as broken; the reason makes it a step, not a wall.
    #[props(default)]
    disabled_reason: String,
    on_toggle: EventHandler<bool>,
) -> Element {
    let control = if busy {
        rsx! {
            span {
                class: if checked { "switch switch-loading active" } else { "switch switch-loading" },
                aria_busy: "true",
                aria_label: "{title}",
                span { class: "spinner" }
            }
        }
    } else {
        rsx! {
            input {
                r#type: "checkbox",
                aria_label: "{title}",
                checked,
                disabled,
                onchange: move |event| on_toggle.call(event.checked()),
            }
            span { class: "switch" }
        }
    };
    let reason_shown = disabled && !busy && !disabled_reason.is_empty();
    let shown_hint = if reason_shown {
        disabled_reason.clone()
    } else {
        hint.clone()
    };
    rsx! {
        label {
            class: if disabled {
                if checked { "quick-toggle active disabled" } else { "quick-toggle disabled" }
            } else if checked {
                "quick-toggle active"
            } else {
                "quick-toggle"
            },
            title: "{shown_hint}",
            span { class: "quick-toggle-icon",
                if icon == "network" {
                    Icon { icon: LdNetwork, width: 16, height: 16 }
                } else if icon == "wifi" {
                    Icon { icon: LdWifi, width: 16, height: 16 }
                } else if icon == "moon" {
                    if checked {
                        Icon { icon: LdMoon, width: 16, height: 16 }
                    } else {
                        Icon { icon: LdSun, width: 16, height: 16 }
                    }
                } else {
                    Icon { icon: LdGlobe, width: 16, height: 16 }
                }
                span { class: if checked { "quick-toggle-dot on" } else { "quick-toggle-dot" } }
            }
            {control}
            span { class: "quick-toggle-text",
                strong { "{title}" }
                small { "{value}" }
                if !cost.is_empty() {
                    small { class: "quick-toggle-cost", "{cost}" }
                }
            }
        }
    }
}

/// Why a runtime quick-toggle is currently inert, in one place so TUN and LAN
/// stop silently refusing the tap while showing an unrelated feature hint.
pub(crate) fn quick_toggle_disabled_reason(busy: bool, node_count: usize) -> String {
    if busy {
        "其它网络操作正在进行".to_string()
    } else if node_count == 0 {
        "请先添加订阅".to_string()
    } else {
        String::new()
    }
}

/// Phone-only 2x2 grid: the design keeps TUN, LAN, theme, and system proxy one
/// tap away from the power button. Desktop keeps those in the settings page.
#[component]
pub(crate) fn OverviewQuickToggles(
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    proxy_server_nameservers: Vec<String>,
    connected: Signal<bool>,
    core_busy: Signal<bool>,
    core_restarting: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    tunnel_mode: Signal<TunnelMode>,
    mut tun_enabled: Signal<bool>,
    mut allow_lan: Signal<bool>,
    mut dark_mode: Signal<bool>,
    custom_rules: Signal<Vec<CustomRule>>,
    dns_overrides: Signal<Vec<DnsOverride>>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
    group_selections: Signal<HashMap<String, String>>,
    system_proxy: Signal<SystemProxyLoadState>,
    system_proxy_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
    tun_busy: Signal<bool>,
    lan_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    // Provided by ProxyApp; cleared centrally when a restart applies the config.
    let config_pending = use_context::<ConfigPending>().0;
    let runtime = RuntimeSignals {
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
    };
    let proxy = system_proxy_view(system_proxy(), system_proxy_busy());
    let runtime_settings = RuntimeSettings {
        tunnel_mode,
        tun_enabled,
        allow_lan,
        custom_rules,
        dns_overrides,
        config_script_enabled,
        config_script,
    };
    let busy = runtime.busy();
    let core_running = connected();
    let show_tun = !cfg!(target_os = "android");
    // Each tile handler needs its own copy of the nameserver list.
    let tun_nameservers = proxy_server_nameservers.clone();

    rsx! {
        section { class: "quick-toggles glass-surface",
            div { class: "quick-toggles-heading",
                div {
                    strong { "快捷开关" }
                    p { class: "eyebrow", "QUICK TOGGLES" }
                }
            }
            div { class: "quick-toggle-grid",
                if show_tun {
                    QuickToggle {
                        icon: "network".to_string(),
                        title: "TUN 模式".to_string(),
                        value: if tun_busy() {
                            "处理中".to_string()
                        } else if tun_enabled() {
                            "已启用".to_string()
                        } else {
                            "未启用".to_string()
                        },
                        hint: "系统级网络接管".to_string(),
                        cost: if core_running { "切换会重启内核".to_string() } else { "需要管理员授权".to_string() },
                        disabled_reason: quick_toggle_disabled_reason(busy, nodes().len()),
                        checked: tun_enabled(),
                        busy: tun_busy(),
                        disabled: busy || nodes().is_empty(),
                        on_toggle: move |enabled| {
                            let request = build_connection_request(
                                nodes(),
                                selected_tag(),
                                tun_nameservers.clone(),
                                runtime_settings,
                                group_selections(),
                            );
                            spawn_tun_toggle(
                                enabled,
                                request,
                                tun_enabled,
                                tun_busy,
                                runtime,
                                toast,
                            );
                        },
                    }
                }
                QuickToggle {
                    icon: "wifi".to_string(),
                    title: "局域网".to_string(),
                    value: if lan_busy() {
                        "处理中".to_string()
                    } else if allow_lan() {
                        "0.0.0.0:7890".to_string()
                    } else {
                        "仅本机".to_string()
                    },
                    hint: "允许同一局域网设备使用此代理".to_string(),
                    cost: if core_running { "切换会重启内核".to_string() } else { "下次启动生效".to_string() },
                    disabled_reason: quick_toggle_disabled_reason(busy, nodes().len()),
                    checked: allow_lan(),
                    busy: lan_busy(),
                    disabled: busy || nodes().is_empty(),
                    on_toggle: move |enabled| {
                        let request = build_connection_request(
                            nodes(),
                            selected_tag(),
                            proxy_server_nameservers.clone(),
                            runtime_settings,
                            group_selections(),
                        );
                        spawn_allow_lan_toggle(
                            enabled,
                            request,
                            allow_lan,
                            lan_busy,
                            runtime,
                            toast,
                        );
                    },
                }
                QuickToggle {
                    icon: "moon".to_string(),
                    title: "深色".to_string(),
                    value: if dark_mode() { "已开启".to_string() } else { "已关闭".to_string() },
                    hint: "切换界面配色".to_string(),
                    checked: dark_mode(),
                    busy: false,
                    disabled: false,
                    on_toggle: move |enabled| dark_mode.set(enabled),
                }
                QuickToggle {
                    icon: "globe".to_string(),
                    title: "系统代理".to_string(),
                    value: proxy.label.to_string(),
                    hint: proxy.detail,
                    cost: "修改操作系统代理设置".to_string(),
                    disabled_reason: if busy {
                        "其它网络操作正在进行".to_string()
                    } else if !proxy.supported {
                        "当前平台不支持系统代理".to_string()
                    } else if !proxy.ready {
                        "系统代理状态读取中或不可用".to_string()
                    } else if !proxy.enabled && !core_running {
                        "先启动代理，才能开启系统代理".to_string()
                    } else {
                        String::new()
                    },
                    checked: proxy.enabled,
                    busy: proxy.busy || proxy.loading,
                    disabled: busy
                        || !proxy.ready
                        || !proxy.supported
                        || (!proxy.enabled && !core_running),
                    on_toggle: move |enabled| {
                        spawn_system_proxy_toggle(
                            enabled,
                            system_proxy,
                            system_proxy_busy,
                            busy,
                            toast,
                        );
                    },
                }
            }
        }
    }
}
