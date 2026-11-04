//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

#[cfg(all(
    feature = "native-desktop",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
pub(crate) fn desktop_autostart_settings() -> Element {
    if use_context::<DesktopAutostartCapability>().0 {
        rsx! { DesktopAutostartSettings {} }
    } else {
        rsx! {}
    }
}

#[cfg(not(all(
    feature = "native-desktop",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
)))]
pub(crate) fn desktop_autostart_settings() -> Element {
    let _ = use_context::<DesktopAutostartCapability>().0;
    rsx! {}
}

#[cfg(all(
    feature = "native-desktop",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
))]
#[component]
pub(crate) fn DesktopAutostartSettings() -> Element {
    let mut enabled = use_signal(|| false);
    let mut ready = use_signal(|| false);
    let mut busy = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);
    use_future(move || async move {
        match api::autostart::status().await {
            Ok(value) => {
                enabled.set(value);
                ready.set(true);
            }
            Err(err) => error.set(Some(err.to_string())),
        }
        busy.set(false);
    });
    rsx! {
        section { class: "settings-section autostart-settings glass-surface",
            div { class: "section-heading", h2 { "桌面启动" } }
            label { class: "setting-row toggle-row",
                div {
                    strong { "用户登录时启动" }
                    small { "仅启动 Kitty Pro，不自动连接代理或开启 TUN；默认关闭" }
                }
                input {
                    r#type: "checkbox",
                    aria_label: "用户登录时启动 Kitty Pro",
                    checked: enabled(),
                    disabled: busy() || !ready(),
                    onclick: move |event| {
                        event.prevent_default();
                        if busy() || !ready() { return; }
                        let target = !enabled();
                        busy.set(true);
                        error.set(None);
                        spawn(async move {
                            match api::autostart::set_enabled(target).await {
                                Ok(value) => enabled.set(value),
                                Err(err) => {
                                    error.set(Some(err.to_string()));
                                    // 设置可能已部分生效，回读真实状态避免开关显示反向。
                                    // 只有回读值与操作目标一致才算真正成功；不一致时保留
                                    // 失败提示，否则用户会看到开关静默弹回且没有任何说明。
                                    if let Ok(value) = api::autostart::status().await {
                                        enabled.set(value);
                                        if value == target {
                                            error.set(None);
                                        }
                                    }
                                }
                            }
                            busy.set(false);
                        });
                    },
                }
                span { class: "switch" }
            }
            if busy() {
                div { class: "inline-note", aria_busy: "true",
                    span { class: "spinner" }
                    span { "正在读取或更新系统启动项…" }
                }
            }
            if let Some(message) = error() {
                div { class: "inline-note", role: "alert",
                    Icon { icon: LdCircleAlert, width: 16, height: 16 }
                    span { "{message}" }
                }
                div { class: "rule-editor-actions",
                    button {
                        class: "secondary-button compact",
                        disabled: busy(),
                        onclick: move |_| {
                            if busy() { return; }
                            busy.set(true);
                            error.set(None);
                            spawn(async move {
                                match api::autostart::status().await {
                                    Ok(value) => { enabled.set(value); ready.set(true); }
                                    Err(err) => { ready.set(false); error.set(Some(err.to_string())); }
                                }
                                busy.set(false);
                            });
                        },
                        Icon { icon: LdRefreshCw, width: 15, height: 15 }
                        "重新读取系统状态"
                    }
                }
            }
        }
    }
}

/// Validate and append one DNS override. Returns whether the dialog may close;
/// failures stay open and surface their message inside the dialog.
pub(crate) fn commit_dns_override(
    mut domain: Signal<String>,
    mut server: Signal<String>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    mut error: Signal<Option<String>>,
    connected: bool,
    toast: ToastManager,
    pending: ConfigPending,
) -> bool {
    match proxy_core::normalize_dns_override(&domain(), &server()) {
        Ok((domain_suffix, dns_server)) => {
            let mut rules = dns_overrides.write();
            if rules.iter().any(|item| item.domain_suffix == domain_suffix) {
                error.set(Some(format!("已存在 {domain_suffix} 的专用 DNS")));
                return false;
            }
            if rules.len() >= MAX_CUSTOM_RULES {
                error.set(Some(format!("最多添加 {MAX_CUSTOM_RULES} 条规则")));
                return false;
            }
            let id = rules.iter().map(|item| item.id).max().unwrap_or(0) + 1;
            rules.push(DnsOverride {
                id,
                enabled: true,
                domain_suffix,
                server: dns_server,
            });
            domain.set(String::new());
            server.set(String::new());
            error.set(None);
            drop(rules);
            notify_rule_change(connected, toast, pending);
            true
        }
        Err(message) => {
            error.set(Some(message));
            false
        }
    }
}

#[component]
pub(crate) fn SettingsView(
    platform: String,
    app_version: String,
    tun_busy: Signal<bool>,
    lan_busy: Signal<bool>,
    mut core_state: Signal<String>,
    mut core_version: Signal<Option<String>>,
    mut core_note: Signal<Option<String>>,
    system_proxy: Signal<SystemProxyLoadState>,
    mut connected: Signal<bool>,
    core_busy: Signal<bool>,
    mut core_restarting: Signal<bool>,
    system_proxy_busy: Signal<bool>,
    mut runtime_action_busy: Signal<bool>,
    mut tunnel_mode: Signal<TunnelMode>,
    mut tun_enabled: Signal<bool>,
    mut allow_lan: Signal<bool>,
    mut dark_mode: Signal<bool>,
    mut nodes: Signal<Vec<ProxyNode>>,
    mut selected_tag: Signal<String>,
    mut subscriptions: Signal<Vec<Subscription>>,
    mut active_subscription_id: Signal<Option<u64>>,
    proxy_server_nameservers: Vec<String>,
    mut custom_rules: Signal<Vec<CustomRule>>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    mut config_script_enabled: Signal<bool>,
    mut config_script: Signal<String>,
    mut group_selections: Signal<HashMap<String, String>>,
) -> Element {
    let mut script_check_busy = use_signal(|| false);
    let mut script_editor_open = use_signal(|| false);
    let mut script_draft = use_signal(String::new);
    let mut copy_busy = use_signal(|| false);
    let mut copy_env_busy = use_signal(|| false);
    let toast = use_context::<ToastManager>();
    // Provided by ProxyApp; cleared centrally when a restart applies the config.
    let config_pending = use_context::<ConfigPending>().0;
    let tun_proxy_server_nameservers = proxy_server_nameservers.clone();
    let allow_lan_proxy_server_nameservers = proxy_server_nameservers.clone();
    let script_proxy_server_nameservers = proxy_server_nameservers;
    let autostart_settings = desktop_autostart_settings();

    rsx! {
        div { class: "settings-grid",
            // Design row A: the core card (runtime + JS override) sits next to
            // the routing card, so both fit above the fold on a laptop screen.
            section { class: "settings-section core-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "CORE" }
                        h2 { "内核" }
                    }
                    span { class: status_badge_class(&core_state()), "{core_state_label(&core_state())}" }
                }
                div { class: "setting-row",
                    span { class: "setting-icon", Icon { icon: LdShieldCheck, width: 19, height: 19 } }
                    div {
                        strong { "sing-box" }
                        small { {core_version().unwrap_or_else(|| "未检测到版本".to_string())} }
                    }
                    span { class: "setting-value", "{platform}" }
                }
                if let Some(note) = core_note() {
                    div { class: "inline-note",
                        Icon { icon: LdInfo, width: 16, height: 16 }
                        span { "{note}" }
                    }
                }
                div { class: "setting-row script-setting-row",
                    span { class: "setting-icon", Icon { icon: LdScrollText, width: 19, height: 19 } }
                    div {
                        strong { "JavaScript 配置覆写" }
                        small {
                            if config_script().trim().is_empty() {
                                "main(config) · 尚未配置"
                            } else {
                                "main(config) · QuickJS · {config_script().chars().count()} 字符"
                            }
                        }
                    }
                    button {
                        class: "icon-button",
                        title: "编辑 JavaScript 配置覆写",
                        aria_label: "编辑 JavaScript 配置覆写",
                        onclick: move |_| {
                            script_draft.set(config_script());
                            script_editor_open.set(true);
                        },
                        Icon { icon: LdPencil, width: 17, height: 17 }
                    }
                    label { class: "compact-toggle", title: "启用 JavaScript 配置覆写",
                        input {
                            r#type: "checkbox",
                            aria_label: "启用 JavaScript 配置覆写",
                            checked: config_script_enabled,
                            disabled: config_script().trim().is_empty(),
                            onchange: move |event| config_script_enabled.set(event.checked()),
                        }
                        span { class: "switch" }
                    }
                }
            }

            // Design row A pairs routing with the core card.
            section { class: "settings-section routing-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "ROUTING" }
                        h2 { "路由模式" }
                    }
                }
                div { class: "segmented-control wide", aria_label: "分流模式",
                    button { class: if tunnel_mode() == TunnelMode::Rule { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Rule), "规则模式" }
                    button { class: if tunnel_mode() == TunnelMode::Global { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Global), "全局代理" }
                    button { class: if tunnel_mode() == TunnelMode::Direct { "active" }, onclick: move |_| tunnel_mode.set(TunnelMode::Direct), "全局直连" }
                }
                if !cfg!(target_os = "android") {
                label { class: "setting-row toggle-row",
                    span { class: "setting-icon", Icon { icon: LdNetwork, width: 19, height: 19 } }
                    div {
                        strong { "TUN 模式" }
                        small { "系统级网络接管" }
                    }
                    if tun_busy() {
                        span {
                            class: if tun_enabled() { "switch switch-loading active" } else { "switch switch-loading" },
                            aria_busy: "true",
                            aria_label: if tun_enabled() { "正在关闭 TUN 模式" } else { "正在准备 TUN 权限" },
                            span { class: "spinner" }
                        }
                    } else {
                        input {
                            r#type: "checkbox",
                            checked: tun_enabled,
                            disabled: core_busy()
                                || core_restarting()
                                || system_proxy_busy()
                                || runtime_action_busy(),
                            onclick: move |event| {
                                event.prevent_default();
                                let enabled = !tun_enabled();
                                let request = build_connection_request(
                                    nodes(),
                                    selected_tag(),
                                    tun_proxy_server_nameservers.clone(),
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
                                );
                                spawn_tun_toggle(
                                    enabled,
                                    request,
                                    tun_enabled,
                                    tun_busy,
                                    RuntimeSignals {
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
                        span { class: "switch" }
                    }
                }
                }
                label {
                    class: "setting-row toggle-row",
                    title: "允许同一局域网设备直接使用此代理",
                    span { class: "setting-icon", Icon { icon: LdWifi, width: 19, height: 19 } }
                    div {
                        strong { "允许局域网连接" }
                        small {
                            if allow_lan() { "0.0.0.0:7890 · 无认证" }
                            else { "127.0.0.1:7890" }
                        }
                    }
                    input {
                        r#type: "checkbox",
                        aria_label: "允许局域网连接",
                        checked: allow_lan,
                        disabled: core_busy()
                            || core_restarting()
                            || system_proxy_busy()
                            || runtime_action_busy(),
                        onchange: move |event| {
                                let enabled = event.checked();
                                let request = build_connection_request(
                                    nodes(),
                                    selected_tag(),
                                    allow_lan_proxy_server_nameservers.clone(),
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
                                );
                                spawn_allow_lan_toggle(
                                    enabled,
                                    request,
                                    allow_lan,
                                    lan_busy,
                                    RuntimeSignals {
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
                    span { class: "switch" }
                }
            }


            SyncSettings {
                subscriptions,
                active_subscription_id,
                nodes,
                selected_tag,
                custom_rules,
                dns_overrides,
                group_selections,
                tunnel_mode,
                tun_enabled,
                allow_lan,
                dark_mode,
                config_script_enabled,
                config_script,
            }

            UpdateSettings {}

            section { class: "settings-section appearance-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "APPEARANCE" }
                        h2 { "外观" }
                    }
                }
                label { class: "setting-row toggle-row",
                    span { class: "setting-icon",
                        if dark_mode() { Icon { icon: LdMoon, width: 19, height: 19 } }
                        else { Icon { icon: LdSun, width: 19, height: 19 } }
                    }
                    div {
                        strong { "深色模式" }
                        small { if dark_mode() { "已启用" } else { "跟随浅色外观" } }
                    }
                    input {
                        r#type: "checkbox",
                        checked: dark_mode,
                        onchange: move |event| dark_mode.set(event.checked()),
                    }
                    span { class: "switch" }
                }
                div { class: "setting-row",
                    span { class: "setting-icon", Icon { icon: LdLanguages, width: 19, height: 19 } }
                    div {
                        strong { "语言" }
                        small { "界面语言" }
                    }
                    span { class: "setting-value", "简体中文" }
                }
            }

            section { class: "settings-section about-settings glass-surface",
                div { class: "section-heading",
                    div {
                        p { class: "eyebrow", "ABOUT" }
                        h2 { "诊断与关于" }
                    }
                }
                div { class: "about-identity",
                    span { class: "about-mark", aria_hidden: "true",
                        dangerous_inner_html: BRAND_ICON_SVG,
                    }
                    div {
                        strong { "Kitty Pro" }
                        small { {about_build_label(&app_version, core_version(), &platform)} }
                    }
                }
                div { class: "about-facts",
                    div { class: "about-fact",
                        span { "内核状态" }
                        strong { {core_state_label(&core_state())} }
                    }
                    div { class: "about-fact",
                        span { "节点 / 订阅" }
                        strong { "{nodes().len()} / {subscriptions().len()}" }
                    }
                    div { class: "about-fact",
                        span { "自定义规则" }
                        strong { "{custom_rules().len()}" }
                    }
                    div { class: "about-fact",
                        span { "专用 DNS" }
                        strong { "{dns_overrides().len()}" }
                    }
                }
                div { class: "topbar-actions about-actions",
                    button {
                        class: "secondary-button compact",
                        title: "复制内核与配置诊断信息",
                        disabled: copy_busy(),
                        onclick: move |_| {
                            let payload = build_diagnostic_report(
                                core_state(),
                                core_version(),
                                &platform,
                                tunnel_mode(),
                                tun_enabled(),
                                allow_lan(),
                                nodes().len(),
                                subscriptions().len(),
                                custom_rules().len(),
                                dns_overrides().len(),
                            );
                            copy_busy.set(true);
                            async move {
                                match copy_to_clipboard(payload).await {
                                    Ok(()) => toast.success("诊断信息已复制到剪贴板"),
                                    Err(error) => toast.error(format!("复制失败: {error}")),
                                }
                                copy_busy.set(false);
                            }
                        },
                        if copy_busy() {
                            span { class: "spinner" }
                        } else {
                            Icon { icon: LdClipboardCopy, width: 16, height: 16 }
                        }
                        span { "复制诊断信息" }
                    }
                    button {
                        class: "secondary-button compact",
                        title: if cfg!(target_arch = "wasm32") {
                            "复制到运行此服务的主机剪贴板（浏览器内无法直接访问剪贴板）"
                        } else {
                            "复制代理环境变量"
                        },
                        disabled: copy_env_busy(),
                        onclick: move |_| {
                            copy_env_busy.set(true);
                            async move {
                                match api::copy_proxy_environment_variables().await {
                                    Ok(()) if cfg!(target_arch = "wasm32") => {
                                        toast.success("代理环境变量已复制到服务器主机的剪贴板");
                                    }
                                    Ok(()) => toast.success("代理环境变量已复制"),
                                    Err(error) => {
                                        toast.error(format!("复制环境变量失败: {error}"));
                                    }
                                }
                                copy_env_busy.set(false);
                            }
                        },
                        if copy_env_busy() {
                            span { class: "spinner" }
                        } else {
                            Icon { icon: LdTerminal, width: 16, height: 16 }
                        }
                        span { "复制环境变量" }
                    }
                }
            }

            {autostart_settings}
        }

        if script_editor_open() {
            DialogShell {
                panel_id: "script-editor-dialog".to_string(),
                class: "modal script-modal glass-modal",
                label: "编辑 JavaScript 配置覆写".to_string(),
                // A validation round-trip owns the draft: closing mid-flight would
                // drop the answer the user is waiting for.
                dismissible: !script_check_busy(),
                on_close: move |_| script_editor_open.set(false),
                div { class: "modal-header",
                    div {
                        p { class: "eyebrow", "CONFIG SCRIPT" }
                        h2 { "JavaScript 配置覆写" }
                    }
                    button {
                        class: "icon-button",
                        title: "关闭",
                        aria_label: "关闭脚本编辑器",
                        disabled: script_check_busy(),
                        onclick: move |_| script_editor_open.set(false),
                        Icon { icon: LdX, width: 19, height: 19 }
                    }
                }
                textarea {
                    class: "script-editor",
                    aria_label: "JavaScript 配置覆写脚本",
                    spellcheck: "false",
                    placeholder: "function main(config) {{\n  return config;\n}}",
                    value: script_draft,
                    oninput: move |event| script_draft.set(event.value()),
                }
                div { class: "modal-actions",
                    button {
                        class: "secondary-button",
                        disabled: script_check_busy(),
                        onclick: move |_| script_editor_open.set(false),
                        "取消"
                    }
                    button {
                        class: "secondary-button",
                        disabled: script_check_busy() || script_draft().trim().is_empty(),
                        onclick: move |_| {
                            let proxy_server_nameservers =
                                script_proxy_server_nameservers.clone();
                            async move {
                            script_check_busy.set(true);
                            let request = ConnectionRequest {
                                nodes: nodes(),
                                selected_tag: selected_tag(),
                                proxy_server_nameservers: proxy_server_nameservers.clone(),
                                mode: tunnel_mode(),
                                tun: tun_enabled(),
                                allow_lan: allow_lan(),
                                custom_rules: Vec::new(),
                                dns_overrides: dns_overrides(),
                                config_script: Some(script_draft()),
                                group_selections: group_selections(),
                            };
                            match api::validate_config_script(request).await {
                                Ok(()) => toast.success(if cfg!(target_arch = "wasm32") {
                                    "配置脚本已在服务器端校验通过"
                                } else {
                                    "配置脚本校验通过"
                                }),
                                Err(error) => {
                                    toast.error(format!("配置脚本校验失败: {error}"));
                                }
                            }
                            script_check_busy.set(false);
                            }
                        },
                        if script_check_busy() {
                            span { class: "spinner" }
                        } else {
                            Icon { icon: LdCircleCheck, width: 17, height: 17 }
                        }
                        "校验"
                    }
                    button {
                        class: "primary-button",
                        disabled: script_check_busy(),
                        onclick: move |_| {
                            let value = script_draft();
                            if value.trim().is_empty() {
                                config_script_enabled.set(false);
                            }
                            config_script.set(value);
                            script_editor_open.set(false);
                            toast.success("配置脚本已保存");
                        },
                        Icon { icon: LdSave, width: 16, height: 16 }
                        "保存"
                    }
                }
            }
        }
    }
}
