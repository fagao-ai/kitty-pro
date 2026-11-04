//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

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
    let mut config = use_signal(SyncConfig::default);
    let loaded = use_signal(|| false);
    let load_error = use_signal(|| None::<String>);
    let mut load_attempt = use_signal(|| 0_u32);
    let mut busy = use_signal(|| None::<SyncAction>);
    let mut confirm = use_signal(|| None::<SyncAction>);

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
                div { class: "segmented-control sync-provider-control", aria_label: "同步存储类型",
                    button {
                        class: if provider == SyncProviderKind::WebDav { "active" } else { "" },
                        disabled: sync_busy,
                        onclick: move |_| config.write().provider = SyncProviderKind::WebDav,
                        "WebDAV"
                    }
                    button {
                        class: if provider == SyncProviderKind::S3 { "active" } else { "" },
                        disabled: sync_busy,
                        onclick: move |_| config.write().provider = SyncProviderKind::S3,
                        "S3 兼容"
                    }
                }
                label { class: "setting-row toggle-row sync-enable-row",
                    span { class: "setting-icon", Icon { icon: LdCloud, width: 19, height: 19 } }
                    div {
                        strong { "远端同步" }
                        small { if sync_enabled { "已启用" } else { "已停用" } }
                    }
                    input {
                        r#type: "checkbox",
                        checked: sync_enabled,
                        disabled: sync_busy,
                        onchange: move |event| config.write().enabled = event.checked(),
                    }
                    span { class: "switch" }
                }
            }

            div { class: "inline-note sync-security-note",
                Icon { icon: LdInfo, width: 16, height: 16 }
                span { "远端内容包含订阅来源（含令牌）、自定义规则、专用 DNS 与配置脚本" }
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
                    onclick: move |_| {
                        confirm.set(None);
                        let value = config();
                        async move {
                            busy.set(Some(SyncAction::Save));
                            match api::save_sync_config(value).await {
                                Ok(saved) => {
                                    config.set(saved);
                                    toast.success("同步配置已保存");
                                }
                                Err(error) => toast.error(format!("同步配置保存失败: {error}")),
                            }
                            busy.set(None);
                        }
                    },
                    if busy() == Some(SyncAction::Save) { span { class: "spinner" } }
                    else { Icon { icon: LdSave, width: 16, height: 16 } }
                    "保存"
                }
                button {
                    class: "secondary-button compact",
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    title: "使用远程规则和订阅替换本机数据",
                    onclick: move |_| async move {
                        if confirm() != Some(SyncAction::Pull) {
                            confirm.set(Some(SyncAction::Pull));
                            return;
                        }
                        confirm.set(None);
                        let value = config();
                        busy.set(Some(SyncAction::Pull));
                        match api::sync_pull(value.clone()).await {
                            Ok(downloaded) => {
                                let snapshot = downloaded.snapshot;
                                // Every field the snapshot is allowed to keep must
                                // be handed over explicitly: `..Default::default()`
                                // quietly drops anything not listed, and the drop is
                                // then persisted - which is how a pull erased the
                                // local DNS rules.
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
                                let result = api::commit_sync_pull(
                                    value,
                                    profile,
                                    snapshot.clone(),
                                    downloaded.remote_revision,
                                )
                                .await;
                                let result = match result {
                                    Ok(result) => result,
                                    Err(error) => {
                                        toast.error(format!(
                                            "远端数据已下载，但保存到本机失败: {error}"
                                        ));
                                        busy.set(None);
                                        return;
                                    }
                                };
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
                                    },
                                );
                                if result.checkpoint_saved {
                                    let mut current = config.write();
                                    current.last_sync_at = result.updated_at;
                                    current.last_remote_revision = result.remote_revision.clone();
                                }
                                toast.success(format!(
                                    "已下载 {} 个订阅、{} 条分流规则、{}{}",
                                    result.subscription_count,
                                    result.rule_count,
                                    dns_sync_clause(result.dns_rule_count),
                                    if result.subscription_count > 0 {
                                        "，订阅需刷新后使用"
                                    } else {
                                        ""
                                    }
                                ));
                                if let Some(warning) = result.warning {
                                    toast.info(warning);
                                }
                            }
                            Err(error) => toast.error(format!("远端下载失败: {error}")),
                        }
                        busy.set(None);
                    },
                    if busy() == Some(SyncAction::Pull) { span { class: "spinner" } }
                    else { Icon { icon: LdCloudDownload, width: 17, height: 17 } }
                    if confirm() == Some(SyncAction::Pull) { "确认覆盖本机" } else { "下载覆盖本机" }
                }
                button {
                    class: "primary-button compact",
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    onclick: move |_| {
                        confirm.set(None);
                        let value = config();
                        let snapshot = build_sync_snapshot(
                            &subscriptions(),
                            &custom_rules(),
                            &dns_overrides(),
                            config_script_enabled(),
                            &config_script(),
                            value.last_sync_at,
                        );
                        async move {
                            busy.set(Some(SyncAction::Push));
                            let result = api::sync_push(value, snapshot, false).await;
                            match result {
                                Ok(result) => {
                                    if result.checkpoint_saved {
                                        let mut current = config.write();
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
                                }
                                Err(error) => toast.error(format!("远端上传失败: {error}")),
                            }
                            busy.set(None);
                        }
                    },
                    if busy() == Some(SyncAction::Push) { span { class: "spinner" } }
                    else { Icon { icon: LdCloudUpload, width: 17, height: 17 } }
                    "上传"
                }
                button {
                    class: "secondary-button compact danger-text-button",
                    disabled: !loaded() || sync_busy || !sync_enabled,
                    title: "忽略远端版本并使用本机数据覆盖",
                    onclick: move |_| async move {
                        if confirm() != Some(SyncAction::ForcePush) {
                            confirm.set(Some(SyncAction::ForcePush));
                            return;
                        }
                        confirm.set(None);
                        let value = config();
                        let snapshot = build_sync_snapshot(
                            &subscriptions(),
                            &custom_rules(),
                            &dns_overrides(),
                            config_script_enabled(),
                            &config_script(),
                            value.last_sync_at,
                        );
                        busy.set(Some(SyncAction::ForcePush));
                        let result = api::sync_push(value, snapshot, true).await;
                        match result {
                            Ok(result) => {
                                if result.checkpoint_saved {
                                    let mut current = config.write();
                                    current.last_sync_at = result.updated_at;
                                    current.last_remote_revision = result.remote_revision.clone();
                                }
                                toast.success("已覆盖远端同步文件");
                                if let Some(warning) = result.warning {
                                    toast.info(warning);
                                }
                            }
                            Err(error) => toast.error(format!("覆盖远端失败: {error}")),
                        }
                        busy.set(None);
                    },
                    if busy() == Some(SyncAction::ForcePush) { span { class: "spinner" } }
                    else { Icon { icon: LdCloudUpload, width: 17, height: 17 } }
                    if confirm() == Some(SyncAction::ForcePush) { "确认覆盖远端" } else { "覆盖远端" }
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
    } = targets;
    subscriptions.set(
        snapshot
            .subscriptions
            .into_iter()
            .map(Subscription::from)
            .collect(),
    );
    active_subscription_id.set(None);
    nodes.set(Vec::new());
    selected_tag.set(String::new());
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
    group_selections.set(HashMap::new());
}
