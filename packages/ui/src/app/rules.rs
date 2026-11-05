//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RulesTab {
    Routing,
    Dns,
}

#[component]
pub(crate) fn RulesView(
    mut rules: Signal<Vec<CustomRule>>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    connected: Signal<bool>,
    tunnel_mode: Signal<TunnelMode>,
    mut rule_sets_busy: Signal<bool>,
    config_script_enabled: Signal<bool>,
    mut active_tab: Signal<RulesTab>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let config_pending = use_context::<ConfigPending>();
    let mut editor_open = use_signal(|| false);
    let mut editing_id = use_signal(|| None::<u64>);
    let mut draft_match = use_signal(|| CustomRuleMatch::DomainSuffix);
    let mut draft_action = use_signal(|| CustomRuleAction::Direct);
    let mut draft_value = use_signal(String::new);
    let mut editor_error = use_signal(|| None::<String>);
    let mut confirm_rule_delete = use_signal(|| None::<u64>);
    let mut confirm_dns_delete = use_signal(|| None::<u64>);
    let mut process_picker_open = use_signal(|| false);
    let mut process_picker_loaded = use_signal(|| false);
    let mut process_picker_error = use_signal(|| None::<String>);
    let process_picker_list = use_signal(Vec::<api::RunningProcess>::new);
    // U7: an armed delete must not stay dangerous forever. Re-arming (or a
    // fresh arm on another row) bumps a generation so only the newest timer
    // may clear the armed state; tab switches clear it explicitly below.
    let mut confirm_generation = use_signal(|| 0_u64);
    use_effect(move || {
        if confirm_rule_delete().is_none() && confirm_dns_delete().is_none() {
            return;
        }
        let generation = *confirm_generation.peek() + 1;
        confirm_generation.set(generation);
        spawn(async move {
            // ~4s: long enough to confirm, short enough that a forgotten
            // armed button cannot delete on a much later click.
            for _ in 0..4 {
                wait_for_traffic_tick().await;
            }
            if *confirm_generation.peek() != generation {
                return;
            }
            confirm_rule_delete.set(None);
            confirm_dns_delete.set(None);
        });
    });
    let include_process_rules = process_rules_available();
    // U10: cloning + filter/collect of the rule Vec used to run on every
    // render. The memo re-runs only when `rules()` changes, so unrelated
    // re-renders (tab switch, busy flag, toast tick) reuse the cached list.
    let rule_stats = use_memo(move || {
        let all_stored = rules();
        let stored_rule_count = all_stored.len();
        let stored = all_stored
            .into_iter()
            .filter(|rule| custom_rule_visible(rule.match_type, include_process_rules))
            .collect::<Vec<_>>();
        let enabled_count = stored.iter().filter(|rule| rule.enabled).count();
        let total = stored.len();
        (stored, stored_rule_count, enabled_count, total)
    });
    let (stored, stored_rule_count, enabled_count, total) = rule_stats();
    let rule_mode_active = tunnel_mode() == TunnelMode::Rule;
    let override_active = config_script_enabled();
    // Rules stored but not shown on this platform (process rules on Android).
    let hidden_process_rules = stored_rule_count.saturating_sub(total);

    let mut open_new_editor = move || {
        confirm_rule_delete.set(None);
        editing_id.set(None);
        draft_match.set(CustomRuleMatch::DomainSuffix);
        draft_action.set(CustomRuleAction::Direct);
        draft_value.set(String::new());
        editor_error.set(None);
        editor_open.set(true);
    };
    let on_edit = move |rule: CustomRule| {
        confirm_rule_delete.set(None);
        editing_id.set(Some(rule.id));
        draft_match.set(rule.match_type);
        draft_action.set(rule.action);
        draft_value.set(rule.value);
        editor_error.set(None);
        editor_open.set(true);
    };
    let placeholder = custom_rule_placeholder(draft_match());
    // U10: the DNS tab's counts get the same memo treatment as the routing tab.
    let dns_stats = use_memo(move || {
        let dns_rules = dns_overrides();
        let dns_total = dns_rules.len();
        let dns_enabled_count = dns_rules.iter().filter(|rule| rule.enabled).count();
        (dns_rules, dns_total, dns_enabled_count)
    });
    let (dns_rules, dns_total, dns_enabled_count) = dns_stats();
    let mut dns_domain = use_signal(String::new);
    let mut dns_server = use_signal(String::new);
    let mut dns_error = use_signal(|| None::<String>);
    // The DNS tab adds rules through the same inline dialog as routing so both
    // tabs keep an identical toolbar and list height when switching.
    let mut dns_editor_open = use_signal(|| false);

    let mut open_dns_editor = move || {
        dns_domain.set(String::new());
        dns_server.set(String::new());
        dns_error.set(None);
        dns_editor_open.set(true);
    };

    rsx! {
        section {
            class: if override_active {
                "workspace-section rules-workspace rules-override-active glass-surface"
            } else {
                "workspace-section rules-workspace glass-surface"
            },
            div { class: "workspace-toolbar rules-toolbar",
                div {
                    div { class: "section-title-line",
                        if active_tab() == RulesTab::Routing {
                            h2 { "自定义分流" }
                            span {
                                class: if override_active {
                                    "status-badge pending"
                                } else if rule_mode_active {
                                    "status-badge online"
                                } else {
                                    "status-badge"
                                },
                                if override_active {
                                    "脚本覆写"
                                } else if rule_mode_active {
                                    "规则模式"
                                } else {
                                    "未启用"
                                }
                            }
                        } else {
                            h2 { "专用 DNS" }
                            span {
                                class: if dns_enabled_count > 0 {
                                    "status-badge ready"
                                } else {
                                    "status-badge"
                                },
                                if dns_enabled_count > 0 { "已启用" } else { "未启用" }
                            }
                        }
                    }
                    if active_tab() == RulesTab::Routing {
                        // The rule-set refresh lives with the subtitle rather than
                        // the toolbar so the tab control never slides sideways.
                        div { class: "rules-sub-row",
                            p { "{enabled_count} 条启用 · {total} 条规则" }
                            button {
                                class: "rules-refresh-button",
                                title: "立即更新分流规则",
                                aria_label: "更新分流规则",
                                aria_busy: rule_sets_busy(),
                                disabled: rule_sets_busy() || override_active,
                                onclick: move |_| {
                                    // U9: arm the guard synchronously, before
                                    // the spawn, so a double click cannot fire
                                    // two overlapping update_rule_sets calls.
                                    if *rule_sets_busy.peek() {
                                        return;
                                    }
                                    rule_sets_busy.set(true);
                                    spawn(async move {
                                        match api::update_rule_sets(true).await {
                                            Ok(_) => toast.success("分流规则已更新"),
                                            Err(error) => toast.error(format!("分流规则更新失败: {error}")),
                                        }
                                        rule_sets_busy.set(false);
                                    });
                                },
                                if rule_sets_busy() {
                                    span { class: "spinner" }
                                } else {
                                    Icon { icon: LdRefreshCw, width: 13, height: 13 }
                                }
                                span { class: "rules-refresh-label", "更新规则" }
                            }
                        }
                    } else {
                        div { class: "rules-sub-row",
                            p { "命中域名后缀用专用 DNS 解析并直连，任何分流模式都生效" }
                        }
                    }
                }
                div { class: "toolbar-controls rules-toolbar-actions",
                    // U5: a real group with pressed state, not class-only selection.
                    SegmentedControl {
                        options: vec![
                            ("routing".to_string(), format!("分流规则 {total}")),
                            ("dns".to_string(), format!("专用 DNS {dns_total}")),
                        ],
                        value: if active_tab() == RulesTab::Routing {
                            "routing".to_string()
                        } else {
                            "dns".to_string()
                        },
                        aria_label: "规则类型".to_string(),
                        extra_class: "rules-tab-control".to_string(),
                        onchange: move |next| {
                            // U7: switching views cancels any armed delete.
                            confirm_rule_delete.set(None);
                            confirm_dns_delete.set(None);
                            if next == "dns" {
                                active_tab.set(RulesTab::Dns);
                            } else {
                                active_tab.set(RulesTab::Routing);
                            }
                        },
                    }
                    // Both tabs render exactly this control pair, so switching
                    // tabs moves nothing: only the label counts change.
                    button {
                        class: "primary-button compact",
                        disabled: if active_tab() == RulesTab::Routing {
                            override_active || stored_rule_count >= MAX_CUSTOM_RULES
                        } else {
                            dns_total >= MAX_CUSTOM_RULES
                        },
                        onclick: move |_| {
                            if active_tab() == RulesTab::Routing {
                                open_new_editor();
                            } else {
                                open_dns_editor();
                            }
                        },
                        Icon { icon: LdPlus, width: 17, height: 17 }
                        span { "添加规则" }
                    }
                }
            }

            if active_tab() == RulesTab::Dns {
                // Same inline dialog as the routing tab: the toolbar and the list
                // keep identical geometry when the two tabs are switched.
                if dns_editor_open() {
                    div { class: "rule-editor",
                        div { class: "rule-editor-grid dns-editor-grid",
                            label { class: "rule-field",
                                span { "域名后缀" }
                                input {
                                    value: dns_domain,
                                    placeholder: "example.com",
                                    spellcheck: "false",
                                    oninput: move |event| {
                                        dns_domain.set(event.value());
                                        dns_error.set(None);
                                    },
                                    onkeydown: move |event| {
                                        if event.key() == Key::Enter
                                            && commit_dns_override(
                                                dns_domain,
                                                dns_server,
                                                dns_overrides,
                                                dns_error,
                                                connected(),
                                                toast,
                                                config_pending,
                                            )
                                        {
                                            dns_editor_open.set(false);
                                        }
                                    },
                                }
                            }
                            label { class: "rule-field",
                                span { "DNS 服务器" }
                                input {
                                    value: dns_server,
                                    placeholder: "223.5.5.5",
                                    spellcheck: "false",
                                    oninput: move |event| {
                                        dns_server.set(event.value());
                                        dns_error.set(None);
                                    },
                                    onkeydown: move |event| {
                                        if event.key() == Key::Enter
                                            && commit_dns_override(
                                                dns_domain,
                                                dns_server,
                                                dns_overrides,
                                                dns_error,
                                                connected(),
                                                toast,
                                                config_pending,
                                            )
                                        {
                                            dns_editor_open.set(false);
                                        }
                                    },
                                }
                            }
                        }
                        if let Some(error) = dns_error() {
                            div { class: "field-error", role: "alert",
                                Icon { icon: LdCircleAlert, width: 15, height: 15 }
                                span { "{error}" }
                            }
                        }
                        div { class: "rule-editor-actions",
                            button {
                                class: "secondary-button",
                                onclick: move |_| {
                                    dns_editor_open.set(false);
                                    dns_error.set(None);
                                    dns_domain.set(String::new());
                                    dns_server.set(String::new());
                                },
                                "取消"
                            }
                            button {
                                class: "primary-button",
                                onclick: move |_| {
                                    if commit_dns_override(
                                        dns_domain,
                                        dns_server,
                                        dns_overrides,
                                        dns_error,
                                        connected(),
                                        toast,
                                        config_pending,
                                    ) {
                                        dns_editor_open.set(false);
                                    }
                                },
                                Icon { icon: LdSave, width: 16, height: 16 }
                                "添加"
                            }
                        }
                    }
                }
                div { class: "dns-list",
                    div { class: "dns-table-header",
                        span { "序号" }
                        span { class: "dns-col-action", "解析动作" }
                        span { "域名后缀" }
                        span { "DNS 服务器" }
                        span { "启用" }
                        span { "操作" }
                    }
                    if dns_rules.is_empty() {
                        div { class: "dns-empty-row",
                            Icon { icon: LdGlobe, width: 18, height: 18 }
                            strong { "尚未配置专用 DNS" }
                            small { "为指定域名后缀固定 DNS，避免公共 DNS 解析到错误地址" }
                        }
                    }
                    for (index, rule) in dns_rules.into_iter().enumerate() {
                        div {
                            class: if rule.enabled { "dns-row" } else { "dns-row disabled" },
                            key: "{rule.id}",
                            span { class: "dns-index", "{index + 1:02}" }
                            span { class: "dns-rule-action",
                                Icon { icon: LdGlobe, width: 14, height: 14 }
                                "专用 DNS"
                            }
                            span { class: "dns-rule-domain", title: "{rule.domain_suffix}", "{rule.domain_suffix}" }
                            span { class: "dns-rule-server", title: "{rule.server}", "{rule.server}" }
                            label {
                                class: "rule-enabled-toggle",
                                title: if rule.enabled { "停用这条规则" } else { "启用这条规则" },
                                Switch {
                                    checked: rule.enabled,
                                    label: "专用 DNS 规则开关".to_string(),
                                    aria_label: Some(if rule.enabled {
                                        format!("停用 {} 的专用 DNS", rule.domain_suffix)
                                    } else {
                                        format!("启用 {} 的专用 DNS", rule.domain_suffix)
                                    }),
                                    onchange: move |checked| {
                                        dns_overrides.write().iter_mut().for_each(|item| {
                                            if item.id == rule.id {
                                                item.enabled = checked;
                                            }
                                        });
                                        notify_rule_change(connected(), toast, config_pending);
                                    },
                                }
                            }
                            button {
                                class: if confirm_dns_delete() == Some(rule.id) {
                                    "icon-button danger armed"
                                } else {
                                    "icon-button danger"
                                },
                                aria_label: if confirm_dns_delete() == Some(rule.id) {
                                    "再次点击确认删除该专用 DNS"
                                } else {
                                    "删除 {rule.domain_suffix} 的专用 DNS"
                                },
                                title: if confirm_dns_delete() == Some(rule.id) {
                                    "再次点击确认删除"
                                } else {
                                    "删除"
                                },
                                onclick: move |_| {
                                    if confirm_dns_delete() != Some(rule.id) {
                                        confirm_dns_delete.set(Some(rule.id));
                                        return;
                                    }
                                    confirm_dns_delete.set(None);
                                    dns_overrides.write().retain(|item| item.id != rule.id);
                                    notify_rule_change(connected(), toast, config_pending);
                                },
                                Icon { icon: LdTrash2, width: 16, height: 16 }
                            }
                        }
                    }
                }
            }

            if active_tab() == RulesTab::Routing {
            if override_active {
                div { class: "rule-override-notice", role: "status",
                    Icon { icon: LdScrollText, width: 17, height: 17 }
                    div {
                        strong { "JavaScript 配置覆写已启用" }
                        span { "自定义分流规则暂不可用" }
                    }
                }
            }

            if hidden_process_rules > 0 && !override_active {
                // Process rules saved on a desktop sync to Android where they
                // are hidden, yet they still count against MAX_CUSTOM_RULES.
                // Without this the limit hits with a half-empty-looking list.
                div { class: "inline-note info",
                    Icon { icon: LdInfo, width: 16, height: 16 }
                    span { "另有 {hidden_process_rules} 条进程规则在当前平台不可见，但仍计入规则总数并随配置同步。" }
                }
            }

            if !override_active && include_process_rules {
                div { class: "process-rule-hint",
                    Icon { icon: LdInfo, width: 15, height: 15 }
                    span { "进程分流需开启 TUN 才能覆盖所有进程；仅系统代理模式下，进程规则只作用于走代理的进程。" }
                }
            }

            if editor_open() && !override_active {
                div { class: "rule-editor",
                    div { class: "rule-editor-grid",
                        div { class: "rule-field",
                            span { "动作" }
                            RuleSelect {
                                label: "动作".to_string(),
                                value: custom_rule_action_value(draft_action()).to_string(),
                                options: vec![
                                    RuleSelectOption::new("direct", "直连"),
                                    RuleSelectOption::new("proxy", "代理"),
                                    RuleSelectOption::new("block", "拦截"),
                                ],
                                on_select: move |value: String| {
                                    draft_action.set(parse_custom_rule_action(&value));
                                },
                            }
                        }
                        div { class: "rule-field",
                            span { "匹配类型" }
                            RuleSelect {
                                label: "匹配类型".to_string(),
                                value: custom_rule_match_value(draft_match()).to_string(),
                                options: custom_rule_match_options(include_process_rules),
                                on_select: move |value: String| {
                                    draft_match.set(parse_custom_rule_match(&value));
                                    editor_error.set(None);
                                },
                            }
                        }
                        if include_process_rules
                            && (draft_match() == CustomRuleMatch::ProcessName
                                || draft_match() == CustomRuleMatch::ProcessPath)
                        {
                            div { class: "rule-field",
                                span { "匹配内容" }
                                div { class: "rule-value-row",
                                    input {
                                        value: draft_value,
                                        placeholder,
                                        spellcheck: "false",
                                        oninput: move |event| {
                                            draft_value.set(event.value());
                                            editor_error.set(None);
                                        },
                                        onkeydown: move |event| {
                                            if event.key() == Key::Enter {
                                                match save_custom_rule(
                                                    &mut rules.write(),
                                                    editing_id(),
                                                    draft_match(),
                                                    draft_action(),
                                                    &draft_value(),
                                                ) {
                                                    Ok(()) => {
                                                        editor_open.set(false);
                                                        notify_rule_change(connected(), toast, config_pending);
                                                    }
                                                    Err(error) => editor_error.set(Some(error)),
                                                }
                                            }
                                        },
                                    }
                                    button {
                                        r#type: "button",
                                        class: "secondary-button compact process-pick-button",
                                        onclick: move |_| {
                                            process_picker_open.set(true);
                                            process_picker_loaded.set(false);
                                            process_picker_error.set(None);
                                            load_running_processes(
                                                process_picker_list,
                                                process_picker_loaded,
                                                process_picker_error,
                                            );
                                        },
                                        Icon { icon: LdListFilter, width: 15, height: 15 }
                                        "选择进程"
                                    }
                                }
                            }
                        } else {
                        label { class: "rule-field",
                            span { "匹配内容" }
                            input {
                                value: draft_value,
                                placeholder,
                                spellcheck: "false",
                                oninput: move |event| {
                                    draft_value.set(event.value());
                                    editor_error.set(None);
                                },
                                onkeydown: move |event| {
                                    if event.key() == Key::Enter {
                                        match save_custom_rule(
                                            &mut rules.write(),
                                            editing_id(),
                                            draft_match(),
                                            draft_action(),
                                            &draft_value(),
                                        ) {
                                            Ok(()) => {
                                                editor_open.set(false);
                                                notify_rule_change(connected(), toast, config_pending);
                                            }
                                            Err(error) => editor_error.set(Some(error)),
                                        }
                                    }
                                },
                            }
                        }
                        }
                    }
                    if let Some(error) = editor_error() {
                        div { class: "field-error", role: "alert",
                            Icon { icon: LdCircleAlert, width: 15, height: 15 }
                            span { "{error}" }
                        }
                    }
                    // F17 / U22: rule actions ("直连/代理/拦截") are per-rule
                    // match outcomes, not the global "直连/全局" routing mode.
                    div { class: "inline-note info",
                        Icon { icon: LdInfo, width: 16, height: 16 }
                        span { "此处指单条规则的匹配动作，与全局分流模式不同" }
                    }
                    div { class: "rule-editor-actions",
                        button {
                            class: "secondary-button",
                            onclick: move |_| {
                                // Drop the draft too: leaving `editing_id` and
                                // `draft_value` filled made "取消" a hide, not
                                // an undo - the editor reopened with the
                                // abandoned edit still loaded.
                                editor_open.set(false);
                                editor_error.set(None);
                                editing_id.set(None);
                                draft_value.set(String::new());
                            },
                            "取消"
                        }
                        button {
                            class: "primary-button",
                            onclick: move |_| {
                                match save_custom_rule(
                                    &mut rules.write(),
                                    editing_id(),
                                    draft_match(),
                                    draft_action(),
                                    &draft_value(),
                                ) {
                                    Ok(()) => {
                                        editor_open.set(false);
                                        notify_rule_change(connected(), toast, config_pending);
                                    }
                                    Err(error) => editor_error.set(Some(error)),
                                }
                            },
                            Icon { icon: LdSave, width: 16, height: 16 }
                            if editing_id().is_some() { "保存" } else { "添加" }
                        }
                    }
                }
            }

            if stored.is_empty() {
                div { class: "large-empty rules-empty", role: "status",
                    span { class: "empty-icon", Icon { icon: LdListFilter, width: 28, height: 28 } }
                    strong { "暂无自定义规则" }
                }
            } else {
                div { class: "rule-list",
                    div { class: "rule-table-header",
                        span { "序号" }
                        span { "动作" }
                        span { "匹配内容 / 类型" }
                        span { "启用" }
                        span { "操作" }
                    }
                    for (index, rule) in stored.into_iter().enumerate() {
                        RuleListItem {
                            key: "{rule.id}",
                            rule,
                            index,
                            total,
                            rules,
                            connected: connected(),
                            locked: override_active,
                            include_process_rules,
                            on_edit,
                            confirm_delete: confirm_rule_delete,
                        }
                    }
                }
            }
            }
        }

        // The picker has to live outside the card: `.glass-surface` gives the
        // rule card a `backdrop-filter`, which makes it the containing block for
        // every `position: fixed` descendant, so a picker rendered inside it was
        // centred on the card instead of the viewport - and with a long rule list
        // the card scrolls, which flung the dialog out of reach entirely.
        if process_picker_open() {
            ProcessPicker {
                list: process_picker_list,
                loaded: process_picker_loaded,
                error: process_picker_error,
                on_close: move |_| process_picker_open.set(false),
                on_pick: move |process: api::RunningProcess| {
                    let value = if draft_match() == CustomRuleMatch::ProcessPath {
                        process.exe_path.clone().unwrap_or_else(|| process.name.clone())
                    } else {
                        process.name.clone()
                    };
                    draft_value.set(value);
                    editor_error.set(None);
                    process_picker_open.set(false);
                },
            }
        }
    }
}

pub(crate) fn load_running_processes(
    mut list: Signal<Vec<api::RunningProcess>>,
    mut loaded: Signal<bool>,
    mut error: Signal<Option<String>>,
) {
    spawn(async move {
        match api::list_running_processes().await {
            Ok(processes) => {
                list.set(processes);
                loaded.set(true);
            }
            Err(load_error) => {
                error.set(Some(load_error.to_string()));
            }
        }
    });
}

#[component]
pub(crate) fn ProcessPicker(
    list: Signal<Vec<api::RunningProcess>>,
    loaded: Signal<bool>,
    error: Signal<Option<String>>,
    on_close: EventHandler<()>,
    mut on_pick: EventHandler<api::RunningProcess>,
) -> Element {
    let mut query = use_signal(String::new);
    let filtered = list
        .iter()
        .filter(|process| {
            let needle = query().to_lowercase();
            needle.is_empty()
                || process.name.to_lowercase().contains(&needle)
                || process
                    .exe_path
                    .as_deref()
                    .map(|path| path.to_lowercase().contains(&needle))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    rsx! {
        DialogShell {
            panel_id: "process-picker-dialog".to_string(),
            class: "process-picker glass-modal",
            label: "选择进程".to_string(),
            dismissible: true,
            on_close,
            div { class: "process-picker-header",
                h3 { "选择进程" }
                button {
                    r#type: "button",
                    class: "icon-button",
                    aria_label: "关闭",
                    onclick: move |_| on_close.call(()),
                    Icon { icon: LdX, width: 17, height: 17 }
                }
            }
            div { class: "process-picker-search",
                input {
                    value: query,
                    aria_label: "搜索进程名或路径",
                    placeholder: "搜索进程名或路径…",
                    spellcheck: "false",
                    oninput: move |event| query.set(event.value()),
                }
            }
            if let Some(message) = error() {
                div { class: "process-picker-status error", role: "alert", "{message}" }
            } else if !loaded() {
                div { class: "process-picker-status", role: "status", aria_live: "polite",
                    span { class: "spinner", aria_hidden: "true" }
                    "正在读取进程列表…"
                }
            } else if filtered.is_empty() {
                div { class: "process-picker-status", role: "status", "没有匹配的进程" }
            } else {
                div { class: "process-picker-list", role: "listbox",
                    for process in filtered {
                        {
                            let name = process.name.clone();
                            let path = process.exe_path.clone().unwrap_or_default();
                            let picked = process.clone();
                            rsx! {
                                button {
                                    key: "{process.pid}",
                                    r#type: "button",
                                    class: "process-picker-item",
                                    role: "option",
                                    onclick: move |_| on_pick.call(picked.clone()),
                                    div { class: "process-picker-item-main",
                                        span { class: "process-picker-item-name", "{name}" }
                                        if !path.is_empty() {
                                            span { class: "process-picker-item-path", "{path}" }
                                        }
                                    }
                                    span { class: "process-picker-item-pid", "PID {process.pid}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn RuleSelect(
    label: String,
    value: String,
    options: Vec<RuleSelectOption>,
    on_select: EventHandler<String>,
) -> Element {
    let mut open = use_signal(|| false);
    // U24: roving active option for Home/End/Enter + aria-activedescendant.
    // Limitation: options stay Tab-reachable buttons with no roving visual
    // focus ring (adding one would need a new CSS class), so keyboard users
    // rely on the option's own focus and aria-selected instead.
    let mut active_index = use_signal(|| 0_usize);
    let selected_label = options
        .iter()
        .find(|option| option.value == value)
        .map(|option| option.label.clone())
        .unwrap_or_else(|| value.clone());
    let selected_index = options
        .iter()
        .position(|option| option.value == value)
        .unwrap_or(0);
    let option_count = options.len();
    let trigger_label = label.clone();
    let menu_label = label.clone();
    // Ids are scoped by the (unique) label so two selects on the page never
    // hand aria-activedescendant the same id.
    let id_base = format!("rule-select-{}", label.replace(' ', "-"));
    let handler_options = options.clone();

    rsx! {
        div { class: if open() { "rule-select-control open" } else { "rule-select-control" },
            button {
                r#type: "button",
                class: "rule-select-trigger",
                // U24: the accessible name must name the current value too;
                // "动作" / "匹配类型" alone swallowed the selection.
                aria_label: format!("{trigger_label}：{selected_label}"),
                aria_haspopup: "listbox",
                aria_expanded: if open() { "true" } else { "false" },
                aria_activedescendant: if open() {
                    Some(format!("{id_base}-{}", active_index()))
                } else {
                    None
                },
                onclick: move |event| {
                    event.stop_propagation();
                    let next = !open();
                    open.set(next);
                    if next {
                        active_index.set(selected_index);
                    }
                },
                onkeydown: move |event| {
                    match event.key() {
                        Key::ArrowDown => {
                            event.prevent_default();
                            open.set(true);
                            active_index.set((active_index() + 1).min(option_count.saturating_sub(1)));
                        }
                        Key::ArrowUp => {
                            event.prevent_default();
                            open.set(true);
                            active_index.set(active_index().saturating_sub(1));
                        }
                        Key::Home => {
                            event.prevent_default();
                            open.set(true);
                            active_index.set(0);
                        }
                        Key::End => {
                            event.prevent_default();
                            open.set(true);
                            active_index.set(option_count.saturating_sub(1));
                        }
                        Key::Enter => {
                            event.prevent_default();
                            if !open() {
                                open.set(true);
                                active_index.set(selected_index);
                            } else if let Some(option) = handler_options.get(active_index()) {
                                on_select.call(option.value.clone());
                                open.set(false);
                            }
                        }
                        Key::Escape => {
                            event.prevent_default();
                            // Only the menu may close: without this the dialog
                            // shell above also sees Escape and takes the whole
                            // editor with it.
                            event.stop_propagation();
                            open.set(false);
                        }
                        _ => {}
                    }
                },
                span { "{selected_label}" }
                Icon { icon: LdChevronDown, width: 16, height: 16 }
            }
            if open() {
                div {
                    class: "rule-select-backdrop",
                    aria_hidden: "true",
                    onclick: move |_| open.set(false),
                }
                div { class: "rule-select-menu", role: "listbox", aria_label: menu_label,
                    for (index, option) in options.into_iter().enumerate() {
                        {
                            let option_value = option.value.clone();
                            let selected = option.value == value;
                            let option_id = format!("{id_base}-{index}");
                            rsx! {
                                button {
                                    key: "{option.value}",
                                    id: option_id,
                                    r#type: "button",
                                    role: "option",
                                    aria_selected: if selected { "true" } else { "false" },
                                    class: if selected { "rule-select-option selected" } else { "rule-select-option" },
                                    onclick: move |event| {
                                        event.stop_propagation();
                                        on_select.call(option_value.clone());
                                        open.set(false);
                                    },
                                    span { "{option.label}" }
                                    if selected {
                                        Icon { icon: LdCheck, width: 15, height: 15 }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn RuleListItem(
    rule: CustomRule,
    index: usize,
    total: usize,
    mut rules: Signal<Vec<CustomRule>>,
    connected: bool,
    locked: bool,
    include_process_rules: bool,
    on_edit: EventHandler<CustomRule>,
    /// Rule id currently armed for delete (see SubscriptionRow's note on the
    /// inline two-step pattern; rules have no undo either).
    mut confirm_delete: Signal<Option<u64>>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let config_pending = use_context::<ConfigPending>();
    let rule_id = rule.id;
    let edit_rule = rule.clone();
    let enabled = rule.enabled;
    let action = rule.action;
    let match_type = rule.match_type;
    let priority = index + 1;

    rsx! {
        div {
            class: if locked {
                "rule-row locked"
            } else if enabled {
                "rule-row"
            } else {
                "rule-row disabled"
            },
            span { class: "rule-priority", "{priority:02}" }
            span { class: custom_rule_action_class(action),
                match action {
                    CustomRuleAction::Direct => rsx! { Icon { icon: LdRoute, width: 15, height: 15 } },
                    CustomRuleAction::Proxy => rsx! { Icon { icon: LdShieldCheck, width: 15, height: 15 } },
                    CustomRuleAction::Block => rsx! { Icon { icon: LdBan, width: 15, height: 15 } },
                }
                {custom_rule_action_label(action)}
            }
            div { class: "rule-main",
                strong { title: rule.value.clone(), "{rule.value}" }
                small { "{custom_rule_match_label(match_type)}" }
            }
            label { class: "rule-enabled-toggle", title: if enabled { "停用规则" } else { "启用规则" },
                Switch {
                    checked: enabled,
                    label: if enabled { "停用规则".to_string() } else { "启用规则".to_string() },
                    disabled: locked,
                    onchange: move |checked| {
                        if let Some(stored) = rules.write().iter_mut().find(|item| item.id == rule_id) {
                            stored.enabled = checked;
                            notify_rule_change(connected, toast, config_pending);
                        }
                    },
                }
            }
            div { class: "rule-row-actions",
                button {
                    class: "icon-button",
                    title: "上移",
                    aria_label: "上移这条规则",
                    disabled: locked || index == 0,
                    onclick: move |_| {
                        move_custom_rule(
                            &mut rules.write(),
                            rule_id,
                            -1,
                            include_process_rules,
                        );
                        notify_rule_change(connected, toast, config_pending);
                    },
                    Icon { icon: LdArrowUp, width: 16, height: 16 }
                }
                button {
                    class: "icon-button",
                    title: "下移",
                    aria_label: "下移这条规则",
                    disabled: locked || index + 1 >= total,
                    onclick: move |_| {
                        move_custom_rule(
                            &mut rules.write(),
                            rule_id,
                            1,
                            include_process_rules,
                        );
                        notify_rule_change(connected, toast, config_pending);
                    },
                    Icon { icon: LdArrowDown, width: 16, height: 16 }
                }
                button {
                    class: "icon-button",
                    title: "编辑规则",
                    aria_label: "编辑这条规则",
                    disabled: locked,
                    onclick: move |_| on_edit.call(edit_rule.clone()),
                    Icon { icon: LdPencil, width: 16, height: 16 }
                }
                button {
                    class: if confirm_delete() == Some(rule_id) {
                        "icon-button danger armed"
                    } else {
                        "icon-button danger"
                    },
                    aria_label: if confirm_delete() == Some(rule_id) {
                        "再次点击确认删除规则"
                    } else {
                        "删除规则"
                    },
                    title: if confirm_delete() == Some(rule_id) {
                        "再次点击确认删除"
                    } else {
                        "删除规则"
                    },
                    disabled: locked,
                    onclick: move |_| {
                        if confirm_delete() != Some(rule_id) {
                            confirm_delete.set(Some(rule_id));
                            return;
                        }
                        confirm_delete.set(None);
                        rules.write().retain(|item| item.id != rule_id);
                        notify_rule_change(connected, toast, config_pending);
                    },
                    Icon { icon: LdTrash2, width: 16, height: 16 }
                }
            }
        }
    }
}

pub(crate) fn save_custom_rule(
    rules: &mut Vec<CustomRule>,
    editing_id: Option<u64>,
    match_type: CustomRuleMatch,
    action: CustomRuleAction,
    value: &str,
) -> Result<(), String> {
    if editing_id.is_none() && rules.len() >= MAX_CUSTOM_RULES {
        return Err(format!("最多添加 {MAX_CUSTOM_RULES} 条规则"));
    }
    let id = editing_id
        .or_else(|| next_custom_rule_id(rules))
        .ok_or_else(|| "无法生成规则 ID".to_string())?;
    let enabled = rules
        .iter()
        .find(|rule| rule.id == id)
        .map(|rule| rule.enabled)
        .unwrap_or(true);
    let candidate = CustomRule {
        id,
        enabled,
        match_type,
        value: value.to_string(),
        action,
    }
    .normalized()
    .map_err(|error| error.to_string())?;

    let mut updated = rules.clone();
    if let Some(existing) = updated.iter_mut().find(|rule| rule.id == id) {
        *existing = candidate;
    } else if editing_id.is_some() {
        return Err("要编辑的规则不存在".to_string());
    } else {
        updated.push(candidate);
    }
    validate_custom_rules(&updated).map_err(|error| error.to_string())?;
    *rules = updated;
    Ok(())
}

pub(crate) fn connection_custom_rules(
    config_script_enabled: bool,
    config_script: &str,
    custom_rules: Vec<CustomRule>,
) -> Vec<CustomRule> {
    connection_custom_rules_for_platform(
        config_script_enabled,
        config_script,
        custom_rules,
        process_rules_available(),
    )
}

pub(crate) fn connection_custom_rules_for_platform(
    config_script_enabled: bool,
    config_script: &str,
    custom_rules: Vec<CustomRule>,
    include_process_rules: bool,
) -> Vec<CustomRule> {
    if config_script_enabled && !config_script.trim().is_empty() {
        Vec::new()
    } else {
        custom_rules
            .into_iter()
            .filter(|rule| custom_rule_visible(rule.match_type, include_process_rules))
            .collect()
    }
}

pub(crate) fn next_custom_rule_id(rules: &[CustomRule]) -> Option<u64> {
    (1..=(rules.len() as u64 + 1)).find(|id| rules.iter().all(|rule| rule.id != *id))
}

pub(crate) fn move_custom_rule(
    rules: &mut [CustomRule],
    id: u64,
    offset: isize,
    include_process_rules: bool,
) {
    let visible_indices = rules
        .iter()
        .enumerate()
        .filter_map(|(index, rule)| {
            custom_rule_visible(rule.match_type, include_process_rules).then_some(index)
        })
        .collect::<Vec<_>>();
    let Some(visible_index) = visible_indices
        .iter()
        .position(|index| rules[*index].id == id)
    else {
        return;
    };
    let Some(target) = visible_index.checked_add_signed(offset) else {
        return;
    };
    if let Some(target_index) = visible_indices.get(target) {
        rules.swap(visible_indices[visible_index], *target_index);
    }
}

pub(crate) fn notify_rule_change(connected: bool, toast: ToastManager, mut pending: ConfigPending) {
    if connected {
        // Rules/DNS edits only reach the core through a (re)start, so the
        // overview keeps a 重新连接 affordance up until one happens.
        pending.0.set(true);
    }
    toast.success(if connected {
        "规则已保存，重新连接后生效"
    } else {
        "规则已保存"
    });
}

pub(crate) fn custom_rule_action_value(action: CustomRuleAction) -> &'static str {
    match action {
        CustomRuleAction::Direct => "direct",
        CustomRuleAction::Proxy => "proxy",
        CustomRuleAction::Block => "block",
    }
}

pub(crate) fn parse_custom_rule_action(value: &str) -> CustomRuleAction {
    match value {
        "proxy" => CustomRuleAction::Proxy,
        "block" => CustomRuleAction::Block,
        _ => CustomRuleAction::Direct,
    }
}

pub(crate) fn custom_rule_match_value(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "domain",
        CustomRuleMatch::DomainSuffix => "domain_suffix",
        CustomRuleMatch::DomainKeyword => "domain_keyword",
        CustomRuleMatch::IpCidr => "ip_cidr",
        CustomRuleMatch::ProcessName => "process_name",
        CustomRuleMatch::ProcessPath => "process_path",
    }
}

pub(crate) const fn process_rules_available() -> bool {
    !cfg!(any(target_os = "android", target_os = "ios"))
}

pub(crate) fn custom_rule_match_options(include_process_rules: bool) -> Vec<RuleSelectOption> {
    let mut options = vec![
        RuleSelectOption::new("domain", "精确域名"),
        RuleSelectOption::new("domain_suffix", "域名后缀"),
        RuleSelectOption::new("domain_keyword", "域名关键字"),
        RuleSelectOption::new("ip_cidr", "IP CIDR"),
    ];
    if include_process_rules {
        options.push(RuleSelectOption::new("process_name", "进程名"));
        options.push(RuleSelectOption::new("process_path", "进程路径"));
    }
    options
}

pub(crate) fn custom_rule_visible(
    match_type: CustomRuleMatch,
    include_process_rules: bool,
) -> bool {
    include_process_rules
        || !matches!(
            match_type,
            CustomRuleMatch::ProcessName | CustomRuleMatch::ProcessPath
        )
}

pub(crate) fn parse_custom_rule_match(value: &str) -> CustomRuleMatch {
    match value {
        "domain" => CustomRuleMatch::Domain,
        "domain_keyword" => CustomRuleMatch::DomainKeyword,
        "ip_cidr" => CustomRuleMatch::IpCidr,
        "process_name" => CustomRuleMatch::ProcessName,
        "process_path" => CustomRuleMatch::ProcessPath,
        _ => CustomRuleMatch::DomainSuffix,
    }
}

pub(crate) fn custom_rule_match_label(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "精确域名",
        CustomRuleMatch::DomainSuffix => "域名后缀",
        CustomRuleMatch::DomainKeyword => "域名关键字",
        CustomRuleMatch::IpCidr => "IP CIDR",
        CustomRuleMatch::ProcessName => "进程名",
        CustomRuleMatch::ProcessPath => "进程路径",
    }
}

pub(crate) fn custom_rule_placeholder(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "api.example.com",
        CustomRuleMatch::DomainSuffix => "example.com",
        CustomRuleMatch::DomainKeyword => "google",
        CustomRuleMatch::IpCidr => "203.0.113.0/24",
        CustomRuleMatch::ProcessName => "Telegram",
        CustomRuleMatch::ProcessPath => "/Applications/Telegram.app/Contents/MacOS/Telegram",
    }
}

pub(crate) fn custom_rule_action_label(action: CustomRuleAction) -> &'static str {
    match action {
        CustomRuleAction::Direct => "直连",
        CustomRuleAction::Proxy => "代理",
        CustomRuleAction::Block => "拦截",
    }
}

pub(crate) fn custom_rule_action_class(action: CustomRuleAction) -> &'static str {
    match action {
        CustomRuleAction::Direct => "rule-action direct",
        CustomRuleAction::Proxy => "rule-action proxy",
        CustomRuleAction::Block => "rule-action block",
    }
}
