//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

/// F15: turn the backend's optimistic-concurrency jargon into plain language.
/// Returns `(friendly, is_conflict)`; the raw text is kept for a folded "详情"
/// so ETag / 基线 / revision detail is still available to whoever needs it.
pub(crate) fn classify_sync_error(raw: &str) -> (String, bool) {
    if raw.contains("已被其他设备修改") || raw.contains("已被其它设备修改") {
        (
            "远端配置已被其它设备更新，请先下载覆盖本机或明确覆盖远端".to_string(),
            true,
        )
    } else if raw.contains("ETag") || raw.contains("etag") {
        (
            "远端存储不支持安全上传，请改用「覆盖远端」".to_string(),
            true,
        )
    } else if raw.contains("基线") {
        (
            "本机尚无该远端目标的同步基线，请先下载覆盖本机或覆盖远端".to_string(),
            true,
        )
    } else if raw.contains("revision") || raw.contains("Revision") {
        (
            "远端版本校验未通过，请下载覆盖本机或覆盖远端".to_string(),
            true,
        )
    } else {
        (raw.to_string(), false)
    }
}

/// F15: report a sync error in plain words; known conflicts also surface an
/// inline panel with the two resolving actions.
pub(crate) fn report_sync_error(
    toast: ToastManager,
    conflict: Signal<Option<(String, String)>>,
    prefix: &str,
    error: impl std::fmt::Display,
) {
    let raw = error.to_string();
    let (friendly, is_conflict) = classify_sync_error(&raw);
    toast.error(format!("{prefix}: {friendly}"));
    if is_conflict {
        let mut signal = conflict;
        signal.set(Some((friendly, raw)));
    }
}

/// F15: the destructive "download overwrites local" confirm summary. Remote
/// counts are only known after a previous pull, hence the graceful fallback.
pub(crate) fn pull_confirm_summary(
    remote_counts: Option<(usize, usize)>,
    local_subscriptions: usize,
    local_rules: usize,
) -> String {
    match remote_counts {
        Some((n, m)) => format!(
            "将用远端 {n} 个订阅 / {m} 条规则替换本机 {local_subscriptions} 个订阅 / {local_rules} 条规则"
        ),
        None => format!(
            "将用远端配置替换本机 {local_subscriptions} 个订阅 / {local_rules} 条规则（远端数量需下载后确认）"
        ),
    }
}

/// F15/U9: signals the pull/push flows need, bundled so the main buttons and
/// the inline conflict actions call exactly the same code.
#[derive(Clone, Copy)]
pub(crate) struct SyncFlowSignals {
    pub(crate) config: Signal<SyncConfig>,
    pub(crate) subscriptions: Signal<Vec<Subscription>>,
    pub(crate) active_subscription_id: Signal<Option<u64>>,
    pub(crate) nodes: Signal<Vec<ProxyNode>>,
    pub(crate) selected_tag: Signal<String>,
    pub(crate) custom_rules: Signal<Vec<CustomRule>>,
    pub(crate) dns_overrides: Signal<Vec<DnsOverride>>,
    pub(crate) group_selections: Signal<HashMap<String, String>>,
    pub(crate) config_script_enabled: Signal<bool>,
    pub(crate) config_script: Signal<String>,
    pub(crate) tunnel_mode: Signal<TunnelMode>,
    pub(crate) tun_enabled: Signal<bool>,
    pub(crate) allow_lan: Signal<bool>,
    pub(crate) dark_mode: Signal<bool>,
    pub(crate) config_pending: ConfigPending,
    pub(crate) toast: ToastManager,
    pub(crate) remote_counts: Signal<Option<(usize, usize)>>,
    pub(crate) conflict: Signal<Option<(String, String)>>,
}

/// U9/F5: synchronously claim the "refresh all to recover" action, shared by
/// both the post-pull hint and the empty-node hint.
pub(crate) fn start_restore_all(
    signals: RefreshAllSignals,
    mut restore_busy: Signal<bool>,
    busy: Signal<Option<SyncAction>>,
) {
    if *restore_busy.peek() || busy.peek().is_some() {
        return;
    }
    restore_busy.set(true);
    let subs_signal = signals.subscriptions;
    let sources: Vec<(u64, String)> = subs_signal()
        .into_iter()
        .map(|item| (item.id, item.source))
        .collect();
    spawn(async move {
        refresh_all_subscriptions(sources, signals).await;
        restore_busy.set(false);
    });
}

/// Pull the remote snapshot and apply it. The caller owns `busy` and clears it
/// on every exit path (including the early returns below).
pub(crate) async fn run_sync_pull(flow: SyncFlowSignals) {
    let SyncFlowSignals {
        config,
        subscriptions,
        active_subscription_id,
        nodes,
        selected_tag,
        custom_rules,
        dns_overrides,
        group_selections,
        config_script_enabled,
        config_script,
        tunnel_mode,
        tun_enabled,
        allow_lan,
        dark_mode,
        config_pending,
        toast,
        remote_counts,
        conflict,
    } = flow;
    let value = config();
    let downloaded = match api::sync_pull(value.clone()).await {
        Ok(downloaded) => downloaded,
        Err(error) => {
            report_sync_error(toast, conflict, "远端下载失败", error);
            return;
        }
    };
    let snapshot = downloaded.snapshot;
    // Every field the snapshot is allowed to keep must be handed over
    // explicitly: `..Default::default()` quietly drops anything not listed,
    // and the drop is then persisted - which is how a pull erased the local
    // DNS rules.
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
    let result =
        match api::commit_sync_pull(value, profile, snapshot.clone(), downloaded.remote_revision)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                toast.error(format!("远端数据已下载，但保存到本机失败: {error}"));
                return;
            }
        };
    // F15: remember the remote snapshot size for the two-step confirm summary.
    let mut counts = remote_counts;
    counts.set(Some((
        snapshot.subscriptions.len(),
        snapshot.custom_rules.len(),
    )));
    apply_sync_snapshot(
        snapshot,
        SyncTargetSignals {
            subscriptions,
            active_subscription_id,
            nodes,
            selected_tag,
            custom_rules,
            dns_overrides,
            group_selections,
            config_script_enabled,
            config_script,
            config_pending,
        },
    );
    if result.checkpoint_saved {
        let mut cfg = config;
        let mut current = cfg.write();
        current.last_sync_at = result.updated_at;
        current.last_remote_revision = result.remote_revision.clone();
    }
    // F6: when the snapshot already carried parsed nodes they were restored in
    // `apply_sync_snapshot`, so only point at the recovery action for legacy
    // snapshots that left the node list empty.
    let needs_refresh = nodes().is_empty() && result.subscription_count > 0;
    toast.success(format!(
        "已下载 {} 个订阅、{} 条分流规则、{}{}",
        result.subscription_count,
        result.rule_count,
        dns_sync_clause(result.dns_rule_count),
        if needs_refresh {
            " · 该快照不含节点，请点「一键刷新全部订阅并恢复使用」"
        } else {
            " · 节点已恢复，重新连接后生效"
        }
    ));
    if let Some(warning) = result.warning {
        toast.info(warning);
    }
    let mut conflict_signal = conflict;
    conflict_signal.set(None);
}

/// Normal (safe) upload: the backend only writes when the local baseline
/// matches the remote revision.
pub(crate) async fn run_sync_push(flow: SyncFlowSignals) {
    let SyncFlowSignals {
        config,
        subscriptions,
        custom_rules,
        dns_overrides,
        config_script_enabled,
        config_script,
        toast,
        conflict,
        ..
    } = flow;
    let value = config();
    let snapshot = build_sync_snapshot(
        &subscriptions(),
        &custom_rules(),
        &dns_overrides(),
        config_script_enabled(),
        &config_script(),
        value.last_sync_at,
    );
    match api::sync_push(value, snapshot, false).await {
        Ok(result) => {
            if result.checkpoint_saved {
                let mut cfg = config;
                let mut current = cfg.write();
                current.last_sync_at = result.updated_at;
                current.last_remote_revision = result.remote_revision.clone();
            }
            toast.success(format!(
                "已上传 {} 个订阅、{} 条分流规则、{}",
                result.subscription_count,
                result.rule_count,
                dns_sync_clause(result.dns_rule_count)
            ));
            if let Some(warning) = result.warning {
                toast.info(warning);
            }
            let mut signal = conflict;
            signal.set(None);
        }
        Err(error) => report_sync_error(toast, conflict, "远端上传失败", error),
    }
}

/// Force upload: ignore the remote revision, used to resolve conflicts or for
/// the first upload to an existing remote target.
pub(crate) async fn run_sync_force_push(flow: SyncFlowSignals) {
    let SyncFlowSignals {
        config,
        subscriptions,
        custom_rules,
        dns_overrides,
        config_script_enabled,
        config_script,
        toast,
        conflict,
        ..
    } = flow;
    let value = config();
    let snapshot = build_sync_snapshot(
        &subscriptions(),
        &custom_rules(),
        &dns_overrides(),
        config_script_enabled(),
        &config_script(),
        value.last_sync_at,
    );
    match api::sync_push(value, snapshot, true).await {
        Ok(result) => {
            if result.checkpoint_saved {
                let mut cfg = config;
                let mut current = cfg.write();
                current.last_sync_at = result.updated_at;
                current.last_remote_revision = result.remote_revision.clone();
            }
            toast.success("已覆盖远端同步文件");
            if let Some(warning) = result.warning {
                toast.info(warning);
            }
            let mut signal = conflict;
            signal.set(None);
        }
        Err(error) => report_sync_error(toast, conflict, "覆盖远端失败", error),
    }
}

#[component]
pub(crate) fn SyncSettings(
    mut subscriptions: Signal<Vec<Subscription>>,
    mut active_subscription_id: Signal<Option<u64>>,
    mut nodes: Signal<Vec<ProxyNode>>,
    mut selected_tag: Signal<String>,
    mut custom_rules: Signal<Vec<CustomRule>>,
    mut dns_overrides: Signal<Vec<DnsOverride>>,
    mut group_selections: Signal<HashMap<String, String>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    dark_mode: Signal<bool>,
    config_script_enabled: Signal<bool>,
    config_script: Signal<String>,
) -> Element {
    let toast = use_context::<ToastManager>();
    // F5: `ConfigPending` lives in context; `SyncSettings` has no `connected`
    // prop and its props signature must not change, so see the note in
    // `apply_sync_snapshot` for how the drift flag is handled.
    let config_pending = use_context::<ConfigPending>();
    let mut config = use_signal(SyncConfig::default);
    let loaded = use_signal(|| false);
    let load_error = use_signal(|| None::<String>);
    let mut load_attempt = use_signal(|| 0_u32);
    let mut busy = use_signal(|| None::<SyncAction>);
    // F19: one arm per action. Arming Pull can no longer leak into ForcePush,
    // and switching actions cancels the previous arm.
    let mut confirm_pull = use_signal(|| false);
    let mut confirm_force_push = use_signal(|| false);
    let confirm_arm = use_signal(|| 0u64);
    let conflict = use_signal(|| None::<(String, String)>);
    let remote_counts = use_signal(|| None::<(usize, usize)>);
    let restore_busy = use_signal(|| false);

    use_effect(move || {
        let _ = load_attempt();
        let mut loaded = loaded;
        let mut config = config;
        let mut load_error = load_error;
        spawn(async move {
            match api::load_sync_config().await {
                Ok(saved) => {
                    config.set(saved);
                    load_error.set(None);
                    loaded.set(true);
                }
                // A failed read must NOT unlock the buttons: the form still
                // holds SyncConfig::default() and saving it would overwrite the
                // real sync.json with an empty config. Stay gated, show the
                // reason, and offer a retry (the effect only re-runs when
                // `load_attempt` bumps - there is no dependency array here).
                Err(error) => {
                    loaded.set(false);
                    load_error.set(Some(error.to_string()));
                }
            }
        });
    });

    // F19: a two-step confirm expires after 3 s; the generation token keeps a
    // re-arm from being cancelled by an older timer.
    use_effect(move || {
        let arm = confirm_arm();
        if !confirm_pull() && !confirm_force_push() {
            return;
        }
        spawn(async move {
            wait_for_confirm_expiry().await;
            if *confirm_arm.peek() == arm {
                confirm_pull.set(false);
                confirm_force_push.set(false);
            }
        });
    });

    let flow = SyncFlowSignals {
        config,
        subscriptions,
        active_subscription_id,
        nodes,
        selected_tag,
        custom_rules,
        dns_overrides,
        group_selections,
        config_script_enabled,
        config_script,
        tunnel_mode,
        tun_enabled,
        allow_lan,
        dark_mode,
        config_pending,
        toast,
        remote_counts,
        conflict,
    };
    // F5/F6: the same "refresh all" signals the subscription page uses, so the
    // post-pull recovery can restore active/nodes/tag/selections.
    let restore_args = RefreshAllSignals {
        subscriptions,
        active_subscription_id,
        nodes,
        selected_tag,
        group_selections,
        toast,
    };

    let current = config();
    let provider = current.provider;
    let sync_enabled = current.enabled;
    let sync_busy = busy().is_some();
    let last_sync_at = current.last_sync_at;

    rsx! {
        section { class: "settings-section sync-settings glass-surface",
            div { class: "section-heading",
                div {
                    p { class: "eyebrow", "REMOTE SYNC" }
                    h2 { "规则与订阅同步" }
                }
                span {
                    class: if last_sync_at > 0 { "status-badge ready" } else if sync_enabled { "status-badge pending" } else { "status-badge" },
                    role: "status",
                    if !loaded() { "读取中" } else if last_sync_at > 0 { "已同步" } else if sync_enabled { "待同步" } else { "未启用" }
                }
            }

            if let Some(error) = load_error() {
                div { class: "load-error-panel", role: "alert",
                    Icon { icon: LdCircleAlert, width: 17, height: 17 }
                    div { class: "load-error-copy",
                        strong { "同步配置读取失败" }
                        small { "为避免用空配置覆盖已保存的设置，所有同步操作已停用。{error}" }
                    }
                    button {
                        class: "secondary-button compact",
                        onclick: move |_| {
                            let next = *load_attempt.peek() + 1;
                            load_attempt.set(next);
                        },
                        Icon { icon: LdRefreshCw, width: 15, height: 15 }
                        "重试"
                    }
                }
            }

            div { class: "sync-provider-row",
                // U5: a role makes the aria_label effective, and aria_pressed
                // tells assistive tech which segment is selected.
                SegmentedControl {
                    options: vec![
                        ("webdav".to_string(), "WebDAV".to_string()),
                        ("s3".to_string(), "S3 兼容".to_string()),
                    ],
                    value: if provider == SyncProviderKind::WebDav {
                        "webdav".to_string()
                    } else {
                        "s3".to_string()
                    },
                    aria_label: "同步存储类型".to_string(),
                    extra_class: "sync-provider-control".to_string(),
                    disabled: sync_busy,
                    onchange: move |next| {
                        if next == "s3" {
                            config.write().provider = SyncProviderKind::S3;
                        } else {
                            config.write().provider = SyncProviderKind::WebDav;
                        }
                    },
                }
                label { class: "setting-row toggle-row sync-enable-row",
                    span { class: "setting-icon", Icon { icon: LdCloud, width: 19, height: 19 } }
                    div {
                        strong { "远端同步" }
                        small { if sync_enabled { "已启用" } else { "已停用" } }
                    }
                    Switch {
                        checked: sync_enabled,
                        label: "远端同步".to_string(),
                        disabled: sync_busy,
                        onchange: move |enabled| config.write().enabled = enabled,
                    }
                }
            }

            div { class: "inline-note sync-security-note",
                Icon { icon: LdInfo, width: 16, height: 16 }
                span { "远端内容包含订阅来源（含令牌）、自定义规则、专用 DNS 与配置脚本" }
            }

            // F15: plain-language explanation of ETag conditional writes,
            // baseline resets and two-step confirmation, folded with existing
            // classes only (no new CSS).
            details { class: "inline-note sync-security-note",
                summary { "同步如何避免覆盖别人的改动？（点击展开说明）" }
                p { "普通「上传」使用 ETag 条件写：只有本机记录的基线版本与远端当前版本一致时才允许写入，避免悄悄盖掉其它设备的更新。" }
                p { "「下载覆盖本机」用远端配置替换本机订阅与规则并重置基线；「覆盖远端」忽略版本直接写入，用于解决冲突或首次上传，同样会重置基线。" }
                p { "这两个会替换数据的按钮都需要二次点击确认，确认状态 3 秒后自动失效。" }
            }

            div { class: "sync-form-grid",
                // Each provider owns its own address field so switching tabs can
                // never show or save an address typed for the other provider.
                if provider == SyncProviderKind::WebDav {
                    label { class: "sync-field sync-endpoint-field",
                        span { "WebDAV 地址" }
                        input {
                            value: current.webdav_endpoint,
                            disabled: sync_busy,
                            placeholder: "https://dav.example.com/dav/files/user",
                            spellcheck: "false",
                            oninput: move |event| config.write().webdav_endpoint = event.value(),
                        }
                    }
                } else {
                    label { class: "sync-field sync-endpoint-field",
                        span { "S3 Endpoint" }
                        input {
                            value: current.s3_endpoint,
                            disabled: sync_busy,
                            placeholder: "留空使用 AWS，或填写 https://s3.example.com",
                            spellcheck: "false",
                            oninput: move |event| config.write().s3_endpoint = event.value(),
                        }
                    }
                }
                label { class: "sync-field",
                    span { "远程路径" }
                    input {
                        value: current.path,
                        disabled: sync_busy,
                        placeholder: "kitty-pro-sync.json",
                        spellcheck: "false",
                        oninput: move |event| config.write().path = event.value(),
                    }
                    // The field no longer names one file: the content lands in a
                    // folder beside it, so show the derived prefix instead of
                    // leaving the user to wonder where the objects went.
                    small { class: "sync-field-hint",
                        "内容写入同级目录 /{proxy_core::sync_bundle_directory(&current.path)}/"
                    }
                }

                if provider == SyncProviderKind::WebDav {
                    label { class: "sync-field",
                        span { "用户名" }
                        input {
                            value: current.username,
                            disabled: sync_busy,
                            autocomplete: "username",
                            oninput: move |event| config.write().username = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "密码 / 应用密码" }
                        input {
                            r#type: "password",
                            value: current.password,
                            disabled: sync_busy,
                            autocomplete: "current-password",
                            oninput: move |event| config.write().password = event.value(),
                        }
                    }
                } else {
                    label { class: "sync-field",
                        span { "Bucket" }
                        input {
                            value: current.bucket,
                            disabled: sync_busy,
                            spellcheck: "false",
                            oninput: move |event| config.write().bucket = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "Region" }
                        input {
                            value: current.region,
                            disabled: sync_busy,
                            placeholder: "us-east-1",
                            spellcheck: "false",
                            oninput: move |event| config.write().region = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "Access Key" }
                        input {
                            value: current.access_key,
                            disabled: sync_busy,
                            autocomplete: "off",
                            spellcheck: "false",
                            oninput: move |event| config.write().access_key = event.value(),
                        }
                    }
                    label { class: "sync-field",
                        span { "Secret Key" }
                        input {
                            r#type: "password",
                            value: current.secret_key,
                            disabled: sync_busy,
                            autocomplete: "off",
                            oninput: move |event| config.write().secret_key = event.value(),
                        }
                    }
                }
            }

            div { class: "sync-actions",
                button {
                    class: "secondary-button compact",
                    disabled: !loaded() || sync_busy,
                    aria_busy: busy() == Some(SyncAction::Save),
                    title: "保存：仅保存本机同步设置，不上传也不下载",
                    onclick: move |_| {
                        // U9: synchronous set + peek-based re-entry guard.
                        if busy.peek().is_some() { return; }
                        confirm_pull.set(false);
                        confirm_force_push.set(false);
                        busy.set(Some(SyncAction::Save));
                        let value = config();
                        spawn(async move {
                            match api::save_sync_config(value).await {
                                Ok(saved) => {
                                    config.set(saved);
                                    toast.success("同步配置已保存");
                                }
                                Err(error) => {
                                    report_sync_error(toast, conflict, "同步配置保存失败", error)
                                }
                            }
                            busy.set(None);
                        });
                    },
                    if busy() == Some(SyncAction::Save) { span { class: "spinner" } }
                    else { Icon { icon: LdSave, width: 16, height: 16 } }
                    "保存"
                }
                ConfirmButton {
                    label: "下载覆盖本机".to_string(),
                    confirm_label: "确认覆盖本机".to_string(),
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    title: Some("下载覆盖本机：用远端配置替换本机订阅与规则".to_string()),
                    extra_class: "secondary-button compact".to_string(),
                    onarm: move |armed: bool| {
                        // F19: per-action arm; switching actions cancels the
                        // other and bumps the generation that guards expiry.
                        confirm_pull.set(armed);
                        if armed {
                            confirm_force_push.set(false);
                            let next = *confirm_arm.peek() + 1;
                            let mut arm = confirm_arm;
                            arm.set(next);
                        }
                    },
                    onconfirm: move |_| {
                        if busy.peek().is_some() { return; }
                        busy.set(Some(SyncAction::Pull));
                        spawn(async move {
                            run_sync_pull(flow).await;
                            busy.set(None);
                        });
                    },
                }
                button {
                    class: "primary-button compact",
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    aria_busy: busy() == Some(SyncAction::Push),
                    title: "上传：仅在本机基线匹配远端版本时安全上传",
                    onclick: move |_| {
                        if busy.peek().is_some() { return; }
                        confirm_pull.set(false);
                        confirm_force_push.set(false);
                        busy.set(Some(SyncAction::Push));
                        spawn(async move {
                            run_sync_push(flow).await;
                            busy.set(None);
                        });
                    },
                    if busy() == Some(SyncAction::Push) { span { class: "spinner" } }
                    else { Icon { icon: LdCloudUpload, width: 17, height: 17 } }
                    "上传"
                }
                ConfirmButton {
                    label: "覆盖远端".to_string(),
                    confirm_label: "确认覆盖远端".to_string(),
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    title: Some("覆盖远端：忽略远端版本，用于解决冲突或首次上传".to_string()),
                    extra_class: "secondary-button compact danger-text-button".to_string(),
                    onarm: move |armed: bool| {
                        // F19: per-action arm; switching actions cancels the
                        // other and bumps the generation that guards expiry.
                        confirm_force_push.set(armed);
                        if armed {
                            confirm_pull.set(false);
                            let next = *confirm_arm.peek() + 1;
                            let mut arm = confirm_arm;
                            arm.set(next);
                        }
                    },
                    onconfirm: move |_| {
                        if busy.peek().is_some() { return; }
                        busy.set(Some(SyncAction::ForcePush));
                        spawn(async move {
                            run_sync_force_push(flow).await;
                            busy.set(None);
                        });
                    },
                }
                // F5/F6: right where a successful pull leaves zero nodes,
                // offer the one-click recovery (the empty-node hint below is
                // the same action, for the state reached from any page).
                if nodes().is_empty() && !subscriptions().is_empty() {
                    button {
                        class: "secondary-button compact",
                        disabled: sync_busy || restore_busy(),
                        onclick: move |_| start_restore_all(restore_args, restore_busy, busy),
                        Icon { icon: LdRefreshCw, width: 16, height: 16 }
                        "刷新全部订阅并恢复使用"
                    }
                }
            }

            // F15: the second click on "download overwrites local" shows a
            // concrete diff summary before anything is replaced.
            if confirm_pull() {
                div { class: "inline-note",
                    "{pull_confirm_summary(remote_counts(), subscriptions().len(), custom_rules().len())}"
                }
            }

            // F15: conflict/ETag/基线 errors get an in-place resolution panel
            // with the two actions, plus the raw backend detail folded away.
            if let Some((friendly, raw)) = conflict() {
                div { class: "inline-note sync-security-note", role: "alert",
                    Icon { icon: LdCircleAlert, width: 16, height: 16 }
                    div {
                        span { "{friendly}" }
                        details {
                            summary { "详情" }
                            p { class: "inline-note", "{raw}" }
                        }
                        div {
                            button {
                                class: "secondary-button compact",
                                disabled: sync_busy,
                                onclick: move |_| {
                                    if busy.peek().is_some() { return; }
                                    confirm_pull.set(false);
                                    confirm_force_push.set(false);
                                    busy.set(Some(SyncAction::Pull));
                                    spawn(async move {
                                        run_sync_pull(flow).await;
                                        busy.set(None);
                                    });
                                },
                                "下载覆盖本机"
                            }
                            button {
                                class: "secondary-button compact danger-text-button",
                                disabled: sync_busy,
                                onclick: move |_| {
                                    if busy.peek().is_some() { return; }
                                    confirm_pull.set(false);
                                    confirm_force_push.set(false);
                                    busy.set(Some(SyncAction::ForcePush));
                                    spawn(async move {
                                        run_sync_force_push(flow).await;
                                        busy.set(None);
                                    });
                                },
                                "覆盖远端"
                            }
                        }
                    }
                }
            }

            // F5/F6/F10: after a pull (and whenever subscriptions exist but
            // nodes are empty) the core cannot start. Give a real next step
            // instead of only a toast.
            if nodes().is_empty() && !subscriptions().is_empty() {
                div { class: "inline-note sync-security-note",
                    Icon { icon: LdInfo, width: 16, height: 16 }
                    span { "订阅已更新但节点尚未解析：为避免连接时无节点可用，请刷新全部订阅恢复使用。" }
                    button {
                        class: "secondary-button compact",
                        disabled: sync_busy || restore_busy(),
                        onclick: move |_| start_restore_all(restore_args, restore_busy, busy),
                        "一键刷新全部订阅并恢复使用"
                    }
                }
            }
        }
    }
}

pub(crate) fn build_sync_snapshot(
    subscriptions: &[Subscription],
    custom_rules: &[CustomRule],
    dns_overrides: &[DnsOverride],
    config_script_enabled: bool,
    config_script: &str,
    updated_at: u64,
) -> SyncSnapshot {
    SyncSnapshot {
        format: proxy_core::SYNC_SNAPSHOT_FORMAT.to_string(),
        version: SYNC_SNAPSHOT_VERSION,
        updated_at,
        subscriptions: subscriptions.iter().map(SyncSubscription::from).collect(),
        custom_rules: custom_rules.to_vec(),
        dns_overrides: Some(dns_overrides.to_vec()),
        config_script: Some(SyncConfigScript {
            enabled: config_script_enabled,
            script: config_script.to_string(),
        }),
    }
}

/// Mutable signals a pulled snapshot writes back into, grouped to keep
/// `apply_sync_snapshot` within the clippy argument budget.
#[derive(Clone, Copy)]
pub(crate) struct SyncTargetSignals {
    pub(crate) subscriptions: Signal<Vec<Subscription>>,
    pub(crate) active_subscription_id: Signal<Option<u64>>,
    pub(crate) nodes: Signal<Vec<ProxyNode>>,
    pub(crate) selected_tag: Signal<String>,
    pub(crate) custom_rules: Signal<Vec<CustomRule>>,
    pub(crate) dns_overrides: Signal<Vec<DnsOverride>>,
    pub(crate) group_selections: Signal<HashMap<String, String>>,
    pub(crate) config_script_enabled: Signal<bool>,
    pub(crate) config_script: Signal<String>,
    pub(crate) config_pending: ConfigPending,
}

pub(crate) fn apply_sync_snapshot(snapshot: SyncSnapshot, targets: SyncTargetSignals) {
    let SyncTargetSignals {
        mut subscriptions,
        mut active_subscription_id,
        mut nodes,
        mut selected_tag,
        mut custom_rules,
        mut dns_overrides,
        mut group_selections,
        mut config_script_enabled,
        mut config_script,
        config_pending,
    } = targets;
    // F6: modern snapshots carry each subscription's last parsed nodes. Restore
    // them immediately instead of blanking the list and forcing a network
    // refresh. Snapshots written before this change have no `nodes` field
    // (`#[serde(default)]`), so they still fall through to the empty-node
    // recovery hint below.
    let synced_subscriptions: Vec<Subscription> = snapshot
        .subscriptions
        .into_iter()
        .map(Subscription::from)
        .collect();
    let active_id = resolve_active_subscription_id(&synced_subscriptions, None, "");
    let restored_nodes = collect_subscription_nodes(&synced_subscriptions, active_id);
    let restored_tag = select_available_tag(&restored_nodes, "");
    let mut restored_selections = HashMap::new();
    if !restored_tag.is_empty() {
        restored_selections.insert("proxy".to_string(), restored_tag.clone());
    }
    subscriptions.set(synced_subscriptions);
    active_subscription_id.set(active_id);
    nodes.set(restored_nodes);
    selected_tag.set(restored_tag);
    custom_rules.set(snapshot.custom_rules);
    // A remote written before DNS overrides existed carries no section; keep the
    // local rules rather than deleting them.
    if let Some(overrides) = snapshot.dns_overrides {
        dns_overrides.set(overrides);
    }
    // Same rule for the JS override: `commit_sync_pull` writes the snapshot's
    // script into the disk profile, so the signals must follow it too -
    // otherwise the UI shows the old script, the save effect persists that
    // stale view over the profile, and the pull quietly rolls itself back.
    if let Some(script) = snapshot.config_script {
        config_script_enabled.set(script.enabled);
        config_script.set(script.script);
    }
    group_selections.set(restored_selections);
    // F5: a connected core keeps serving the old config after a pull, so flag
    // the drift. `SyncSettings` has no `connected` prop (changing its props
    // would break ProxyApp's wiring), and the overview's "重新连接" banner is
    // itself gated by `is_connected`, so a disconnected pull cannot raise a
    // false banner. The next successful restart clears this in
    // `restart_core_from_ui`.
    let mut pending = config_pending.0;
    pending.set(true);
}
