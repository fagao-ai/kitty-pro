use super::*;

#[test]
fn age_labels_measure_against_the_supplied_clock() {
    // The card runs in the browser, where there is no clock to read, so an
    // age is always "then" subtracted from the "now" the check came back with.
    assert_eq!(age_label(1_000, 1_000).as_deref(), Some("刚刚"));
    assert_eq!(age_label(1_000, 1_900).as_deref(), Some("15 分钟前"));
    assert_eq!(age_label(1_000, 200_000).as_deref(), Some("2 天前"));
    // A timestamp ahead of the answering clock must not underflow.
    assert_eq!(age_label(5_000, 1_000).as_deref(), Some("刚刚"));
    // "Never checked" must not read as "a moment ago".
    assert_eq!(age_label(0, 1_000).as_deref(), Some("从未检查"));
    // No answering clock means no age at all, rather than a wrong one.
    assert_eq!(age_label(1_000, 0), None);
}

#[test]
fn the_auto_check_row_names_its_schedule_without_inventing_a_time() {
    assert_eq!(
        auto_check_label(true, 1_000, 1_000 + 3 * 86_400),
        "每天一次 · 3 天前"
    );
    assert_eq!(auto_check_label(true, 0, 1_000), "每天一次 · 从未检查");
    // Before the first answer there is no clock, so the row promises only
    // what it can keep.
    assert_eq!(auto_check_label(true, 0, 0), "每天一次 · 启动时检查");
    assert_eq!(
        auto_check_label(false, 1_000, 1_000),
        "仅手动检查，启动时不再联网"
    );
}

#[test]
fn release_ages_read_the_formats_github_sends() {
    let now = 1_793_704_246 + 3 * 86_400;
    assert_eq!(
        release_age_label(Some("2026-11-03T11:10:46Z"), now),
        "3 天前"
    );
    // An unparseable stamp says so instead of inventing a date.
    for value in [None, Some(""), Some("yesterday"), Some("2026-11-03")] {
        assert_eq!(
            release_age_label(value, now),
            "发布时间未知",
            "{value:?} should not produce a date"
        );
    }
    // So does a stamp with no clock to measure it against.
    assert_eq!(
        release_age_label(Some("2026-11-03T11:10:46Z"), 0),
        "发布时间未知"
    );
}

#[test]
fn a_release_named_after_its_tag_does_not_repeat_it() {
    // This project titles releases exactly like their tags, so the version
    // line would otherwise read "v1.0.9 / v1.0.9 · 13 天前".
    assert_eq!(
        release_subtitle("v1.0.9", "1.0.9", "13 天前".into()),
        "13 天前"
    );
    assert_eq!(
        release_subtitle("V1.0.9", "1.0.9", "13 天前".into()),
        "13 天前"
    );
    // A human title carries information the version does not, so both stay.
    assert_eq!(
        release_subtitle("Hotfix release", "1.0.9", "13 天前".into()),
        "v1.0.9 · 13 天前"
    );
}

#[test]
fn elapsed_time_uses_the_largest_recent_unit() {
    assert_eq!(relative_age(0), "刚刚");
    assert_eq!(relative_age(45), "45 秒前");
    assert_eq!(relative_age(59), "59 秒前");
    assert_eq!(relative_age(60), "1 分钟前");
    assert_eq!(relative_age(3_600), "1 小时前");
    assert_eq!(relative_age(86_400), "1 天前");
    assert_eq!(relative_age(200_000), "2 天前");
}

#[test]
fn asset_sizes_read_in_the_units_a_download_is_judged_in() {
    // The card prints a size the user weighs a download against, so it uses
    // the same formatter the traffic stats use rather than a second one with
    // its own rounding - `2 KiB` in one place and `2.0 KiB` in another is how
    // two helpers start disagreeing about the same byte count.
    assert_eq!(format_data_amount(0), "0 B");
    assert_eq!(format_data_amount(1_023), "1023 B");
    // The real .dmg, .msi and .apk sizes all land on one decimal of MiB.
    assert_eq!(format_data_amount(23_351_810), "22.3 MiB");
    assert_eq!(format_data_amount(30_306_304), "28.9 MiB");
    assert_eq!(format_data_amount(2 * 1024 * 1024), "2.0 MiB");
    assert_eq!(format_data_amount(3 * 1024 * 1024 * 1024), "3.0 GiB");
}

#[test]
fn the_badge_wording_follows_the_check_state() {
    assert_eq!(
        update_badge_label(api::update::AppUpdateState::Available, false, true, false),
        "有更新"
    );
    assert_eq!(
        update_badge_label(api::update::AppUpdateState::UpToDate, false, true, false),
        "已是最新"
    );
    // While a check is in flight the badge says so, whatever it last found.
    assert_eq!(
        update_badge_label(api::update::AppUpdateState::UpToDate, true, true, false),
        "检查中"
    );
    assert_eq!(
        update_badge_label(api::update::AppUpdateState::Idle, false, false, false),
        "读取中"
    );
    assert_eq!(
        update_badge_label(api::update::AppUpdateState::Failed, false, true, false),
        "检查失败"
    );
    assert_eq!(
        update_badge_label(api::update::AppUpdateState::Unsupported, false, true, false),
        "不支持"
    );
    // A dismissed release: the header must not keep advertising an update the
    // card already said it would leave alone.
    assert_eq!(
        update_badge_label(api::update::AppUpdateState::Available, false, true, true),
        "已忽略"
    );
}

#[test]
fn only_an_actionable_update_colours_the_badge() {
    assert_eq!(
        update_badge_class(api::update::AppUpdateState::Available, true),
        "status-badge pending"
    );
    // A dismissed release keeps the neutral badge: the colour is the nag.
    assert_eq!(
        update_badge_class(api::update::AppUpdateState::Available, false),
        "status-badge"
    );
    assert_eq!(
        update_badge_class(api::update::AppUpdateState::UpToDate, false),
        "status-badge ready"
    );
    assert_eq!(
        update_badge_class(api::update::AppUpdateState::Failed, false),
        "status-badge offline"
    );
}

#[test]
fn a_check_that_never_answered_still_shows_the_failure() {
    // Suppressing the background toast is only honest while the card reports
    // the failure. Without this mapping a check that died before producing an
    // `AppUpdateStatus` - unreadable state file, server unreachable - would
    // keep saying "尚未检查" and the error would exist nowhere at all.
    use api::update::{AppUpdateState, AppUpdateStatus};
    let answer = |state: AppUpdateState| AppUpdateStatus {
        state,
        current_version: "1.0.0".to_string(),
        latest: None,
        last_check_at: 0,
        snoozed: false,
        target: "桌面版 · macos · aarch64".to_string(),
        error: None,
        server_time: 0,
    };

    assert_eq!(
        resolve_display_state(None, Some("state file unreadable")),
        AppUpdateState::Failed
    );
    // Whatever the badge said before the check is not what it says after a
    // failure, and a real answer still wins over a stale error.
    assert_eq!(resolve_display_state(None, None), AppUpdateState::Idle);
    assert_eq!(
        resolve_display_state(Some(&answer(AppUpdateState::UpToDate)), Some("late")),
        AppUpdateState::UpToDate
    );
    assert_eq!(
        resolve_display_state(Some(&answer(AppUpdateState::Available)), None),
        AppUpdateState::Available
    );

    // And the failed badge reads as a failure rather than a placeholder.
    assert_eq!(
        update_badge_label(AppUpdateState::Failed, false, true, false),
        "检查失败"
    );
}

#[test]
fn separate_subscriptions_get_distinct_node_tags() {
    let node = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge",
    )
    .nodes
    .pop()
    .expect("fixture should parse");
    let first = namespace_nodes(1, vec![node.clone()]);
    let second = namespace_nodes(2, vec![node]);
    let nodes = collect_nodes(&[
        Subscription {
            id: 1,
            name: "One".to_string(),
            source: "one".to_string(),
            nodes: first,
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        },
        Subscription {
            id: 2,
            name: "Two".to_string(),
            source: "two".to_string(),
            nodes: second,
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        },
    ]);

    assert_eq!(nodes.len(), 2);
    assert_ne!(nodes[0].tag, nodes[1].tag);
    assert_eq!(select_available_tag(&nodes, "missing"), nodes[0].tag);
}

#[test]
fn active_subscription_follows_selection_and_limits_nodes() {
    let node = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge",
    )
    .nodes
    .pop()
    .expect("fixture should parse");
    let subscriptions = vec![
        Subscription {
            id: 1,
            name: "One".to_string(),
            source: "one".to_string(),
            nodes: namespace_nodes(1, vec![node.clone()]),
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        },
        Subscription {
            id: 2,
            name: "Two".to_string(),
            source: "two".to_string(),
            nodes: namespace_nodes(2, vec![node]),
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        },
    ];
    let second_tag = subscriptions[1].nodes[0].tag.clone();

    assert_eq!(
        resolve_active_subscription_id(&subscriptions, None, &second_tag),
        Some(2)
    );
    let active_nodes = collect_subscription_nodes(&subscriptions, Some(2));
    assert_eq!(active_nodes.len(), 1);
    assert_eq!(active_nodes[0].tag, second_tag);

    let remaining = vec![subscriptions[0].clone()];
    let fallback_id = resolve_active_subscription_id(&remaining, Some(2), &second_tag);
    assert_eq!(fallback_id, Some(1));
    assert_eq!(collect_subscription_nodes(&remaining, fallback_id).len(), 1);
}

#[test]
fn source_label_hides_subscription_tokens() {
    assert_eq!(
        source_label("https://user:pass@sub.example.com/path?token=secret"),
        "https://sub.example.com"
    );
}

fn saved_request(tun: bool, allow_lan: bool) -> ConnectionRequest {
    let nodes = proxy_core::parse_subscription("trojan://password@edge.example.com:443#Edge").nodes;
    ConnectionRequest {
        selected_tag: nodes[0].tag.clone(),
        nodes,
        proxy_server_nameservers: Vec::new(),
        mode: TunnelMode::Rule,
        tun,
        allow_lan,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: None,
        group_selections: HashMap::new(),
    }
}

/// Both toggles build their request from signals that still hold the value
/// being switched away from, so the request has to be stamped with the new
/// one. Getting this wrong reads as "TUN 权限预检需要启用 TUN 的连接配置"
/// with the core stopped, and as a core restarted with the switch unmoved
/// while running.
#[test]
fn toggles_stamp_their_new_value_onto_the_request() {
    // Off -> on: the saved request still says false.
    let request = request_with_tun(saved_request(false, false), true);
    assert!(request.tun);
    assert!(!request.allow_lan, "TUN must not disturb LAN");

    // On -> off: the saved request still says true.
    let request = request_with_tun(saved_request(true, true), false);
    assert!(!request.tun);
    assert!(request.allow_lan);

    let request = request_with_allow_lan(saved_request(true, false), true);
    assert!(request.allow_lan);
    assert!(request.tun, "LAN must not disturb TUN");

    let request = request_with_allow_lan(saved_request(true, true), false);
    assert!(!request.allow_lan);
    assert!(request.tun);
}

#[test]
fn node_focus_resolves_owner_source_and_status() {
    let parsed =
        proxy_core::parse_subscription("trojan://password@edge.example.com:443#Edge").nodes;
    let subscriptions = vec![Subscription {
        id: 7,
        name: "Primary".to_string(),
        source: "https://sub.example.com/path?token=secret".to_string(),
        nodes: namespace_nodes(7, parsed.clone()),
        proxy_server_nameservers: Vec::new(),
        rejected_count: 0,
    }];
    let node = &subscriptions[0].nodes[0];
    let groups = vec![ProxyGroup {
        tag: "proxy".to_string(),
        kind: ProxyGroupKind::Selector,
        outbounds: vec![node.tag.clone()],
        selected: node.tag.clone(),
    }];
    let mut latencies = HashMap::new();
    latencies.insert(
        node.tag.clone(),
        NodeLatency {
            tag: node.tag.clone(),
            latency_ms: Some(42),
            error: None,
        },
    );

    let focus = node_focus_details(
        &groups,
        &subscriptions[0].nodes,
        &subscriptions,
        &node.tag,
        &latencies,
    )
    .expect("focused node should resolve");

    assert_eq!(focus.name, "Edge");
    assert_eq!(focus.protocol, "Trojan");
    assert_eq!(focus.endpoint, "edge.example.com:443");
    assert_eq!(focus.group_label, "proxy · 手动选择");
    assert_eq!(focus.status, "已选中 · 可用");
    assert_eq!(focus.source, "https://sub.example.com");

    let ungrouped = node_focus_details(
        &[],
        &subscriptions[0].nodes,
        &[],
        &node.tag,
        &HashMap::new(),
    )
    .expect("an ungrouped node still has details");
    assert_eq!(ungrouped.group_label, "未归组");
    assert_eq!(ungrouped.status, "已选中 · 未测试");
    assert_eq!(ungrouped.source, "本地内容");
}

#[test]
fn empty_refresh_does_not_replace_existing_nodes() {
    let original = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@old.example.com:443#Old",
    )
    .nodes
    .pop()
    .expect("fixture should parse");
    let mut subscriptions = vec![Subscription {
        id: 3,
        name: "Primary".to_string(),
        source: "https://example.com/subscription".to_string(),
        nodes: vec![original.clone()],
        proxy_server_nameservers: Vec::new(),
        rejected_count: 0,
    }];

    let result = apply_subscription_report(
        &mut subscriptions,
        3,
        ParseReport {
            nodes: Vec::new(),
            rejected: vec![proxy_core::ParseIssue {
                line: 1,
                reason: "无效内容".to_string(),
            }],
            proxy_server_nameservers: Vec::new(),
        },
    );

    assert_eq!(result, Err("无效内容".to_string()));
    assert_eq!(subscriptions[0].nodes, vec![original]);
}

#[test]
fn successful_refresh_replaces_nodes_with_a_scoped_tag() {
    let original = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@old.example.com:443#Old",
    )
    .nodes
    .pop()
    .expect("fixture should parse");
    let replacement = proxy_core::parse_subscription("trojan://password@new.example.com:443#New")
        .nodes
        .pop()
        .expect("fixture should parse");
    let mut subscriptions = vec![Subscription {
        id: 9,
        name: "Primary".to_string(),
        source: "https://example.com/subscription".to_string(),
        nodes: vec![original],
        proxy_server_nameservers: Vec::new(),
        rejected_count: 0,
    }];

    assert_eq!(
        apply_subscription_report(
            &mut subscriptions,
            9,
            ParseReport {
                nodes: vec![replacement],
                rejected: Vec::new(),
                proxy_server_nameservers: vec![
                    "https://resolver.example.com/dns-query".to_string(),
                ],
            },
        ),
        Ok(1)
    );
    assert_eq!(subscriptions[0].nodes.len(), 1);
    assert!(subscriptions[0].nodes[0].tag.starts_with("subscription-9-"));
    assert_eq!(
        subscriptions[0].proxy_server_nameservers,
        vec!["https://resolver.example.com/dns-query"]
    );
}

#[test]
fn latency_labels_distinguish_success_and_failure() {
    let success = NodeLatency {
        tag: "edge".to_string(),
        latency_ms: Some(321),
        error: None,
    };
    let failure = NodeLatency {
        tag: "offline".to_string(),
        latency_ms: None,
        error: Some("探测超时或节点不可用".to_string()),
    };

    assert_eq!(format_latency(Some(&success)).0, "321 ms");
    assert_eq!(format_latency(Some(&success)).1, "latency success");
    assert_eq!(format_latency(Some(&failure)).0, "失败");
    assert_eq!(format_latency(Some(&failure)).1, "latency error");
}

#[test]
fn latency_cache_keeps_successes_for_unchanged_endpoints() {
    let node = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge",
    )
    .nodes
    .pop()
    .expect("fixture should parse");
    let results = HashMap::from([
        (
            node.tag.clone(),
            NodeLatency {
                tag: node.tag.clone(),
                latency_ms: Some(128),
                error: None,
            },
        ),
        (
            "offline".to_string(),
            NodeLatency {
                tag: "offline".to_string(),
                latency_ms: None,
                error: Some("timeout".to_string()),
            },
        ),
    ]);

    let entries = latency_cache_entries(std::slice::from_ref(&node), &results);
    assert_eq!(entries, vec![(node.tag.clone(), node.endpoint(), 128)]);

    let restored = restore_cached_latencies(std::slice::from_ref(&node), entries.clone());
    assert_eq!(
        restored.get(&node.tag).and_then(|item| item.latency_ms),
        Some(128)
    );

    // A renamed node must NOT inherit the cached number by endpoint
    // alone: subscriptions from different providers share exit addresses,
    // and the old endpoint-only fallback pasted one node's latency onto
    // another provider's node ("已测速" values that were never measured).
    let mut renamed_node = node.clone();
    renamed_node.tag = "renamed-edge".to_string();
    let restored_renamed =
        restore_cached_latencies(std::slice::from_ref(&renamed_node), entries.clone());
    assert!(restored_renamed.is_empty());

    let mut changed_node = node;
    changed_node.server = "new-edge.example.com".to_string();
    assert!(restore_cached_latencies(&[changed_node], entries).is_empty());
}

#[test]
fn latency_results_sort_success_before_pending_and_failure() {
    let mut nodes = proxy_core::parse_subscription(
        "trojan://password@slow.example.com:443#Slow\n\
         trojan://password@pending.example.com:443#Pending\n\
         trojan://password@fast.example.com:443#Fast\n\
         trojan://password@failed.example.com:443#Failed",
    )
    .nodes;
    let mut members = nodes
        .drain(..)
        .map(|node| ProxyGroupMember {
            tag: node.tag.clone(),
            node: Some(node),
            nested_kind: None,
        })
        .collect::<Vec<_>>();
    let mut results = HashMap::new();
    results.insert(
        members[0].tag.clone(),
        NodeLatency {
            tag: members[0].tag.clone(),
            latency_ms: Some(300),
            error: None,
        },
    );
    results.insert(
        members[2].tag.clone(),
        NodeLatency {
            tag: members[2].tag.clone(),
            latency_ms: Some(80),
            error: None,
        },
    );
    results.insert(
        members[3].tag.clone(),
        NodeLatency {
            tag: members[3].tag.clone(),
            latency_ms: None,
            error: Some("timeout".to_string()),
        },
    );

    sort_proxy_group_members(&mut members, &results);

    let names = members
        .iter()
        .filter_map(|member| member.node.as_ref().map(|node| node.name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Fast", "Slow", "Pending", "Failed"]);
}

#[test]
fn formats_traffic_rates_for_display() {
    assert_eq!(format_data_rate(0), "0 B/s");
    assert_eq!(format_data_rate(1_536), "1.5 KiB/s");
    assert_eq!(format_data_amount(2 * 1024 * 1024), "2.0 MiB");
    assert_eq!(traffic_bar_height(0, 100), "2px");
    assert_eq!(traffic_bar_height(100, 100), "88%");
    assert_eq!(format_uptime(0), "00:00");
    assert_eq!(format_uptime(75), "01:15");
    assert_eq!(format_uptime(3_725), "1:02:05");
}

#[test]
fn saves_normalized_custom_rules_and_rejects_duplicates() {
    let mut rules = Vec::new();
    assert_eq!(
        save_custom_rule(
            &mut rules,
            None,
            CustomRuleMatch::DomainSuffix,
            CustomRuleAction::Proxy,
            "*.Example.COM.",
        ),
        Ok(())
    );
    assert_eq!(rules[0].id, 1);
    assert_eq!(rules[0].value, "example.com");

    assert_eq!(
        save_custom_rule(
            &mut rules,
            None,
            CustomRuleMatch::DomainSuffix,
            CustomRuleAction::Direct,
            "example.com",
        ),
        Err("已存在相同的匹配规则".to_string())
    );
}

#[test]
fn mobile_rule_options_hide_process_matches() {
    let options = custom_rule_match_options(false);
    let values = options
        .iter()
        .map(|option| option.value.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        values,
        vec!["domain", "domain_suffix", "domain_keyword", "ip_cidr"]
    );
    assert!(!values.contains(&"process_name"));
    assert!(!values.contains(&"process_path"));
    assert!(!custom_rule_visible(CustomRuleMatch::ProcessName, false));
    assert!(custom_rule_visible(CustomRuleMatch::Domain, false));
}

#[test]
fn script_override_excludes_custom_rules_from_connection_config() {
    let rules = vec![CustomRule {
        id: 1,
        enabled: true,
        match_type: CustomRuleMatch::Domain,
        value: "example.com".to_string(),
        action: CustomRuleAction::Proxy,
    }];

    assert!(connection_custom_rules(
        true,
        "function main(config) { return config; }",
        rules.clone()
    )
    .is_empty());
    assert_eq!(connection_custom_rules(false, "", rules.clone()), rules);
}

#[test]
fn mobile_connection_excludes_preserved_process_rules() {
    let rules = vec![
        CustomRule {
            id: 1,
            enabled: true,
            match_type: CustomRuleMatch::Domain,
            value: "example.com".to_string(),
            action: CustomRuleAction::Proxy,
        },
        CustomRule {
            id: 2,
            enabled: true,
            match_type: CustomRuleMatch::ProcessName,
            value: "Telegram".to_string(),
            action: CustomRuleAction::Block,
        },
    ];

    let filtered = connection_custom_rules_for_platform(false, "", rules, false);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].match_type, CustomRuleMatch::Domain);
}

#[test]
fn editing_and_moving_custom_rules_preserves_state() {
    let mut rules = vec![
        CustomRule {
            id: 1,
            enabled: false,
            match_type: CustomRuleMatch::Domain,
            value: "one.example".to_string(),
            action: CustomRuleAction::Direct,
        },
        CustomRule {
            id: 2,
            enabled: true,
            match_type: CustomRuleMatch::IpCidr,
            value: "203.0.113.0/24".to_string(),
            action: CustomRuleAction::Block,
        },
    ];

    assert!(save_custom_rule(
        &mut rules,
        Some(1),
        CustomRuleMatch::DomainKeyword,
        CustomRuleAction::Proxy,
        "Media",
    )
    .is_ok());
    assert!(!rules[0].enabled);
    assert_eq!(rules[0].value, "media");

    move_custom_rule(&mut rules, 2, -1, true);
    assert_eq!(
        rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
        vec![2, 1]
    );
    move_custom_rule(&mut rules, 2, -1, true);
    assert_eq!(
        rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
        vec![2, 1]
    );
}

#[test]
fn moving_visible_mobile_rules_preserves_hidden_process_rules() {
    let mut rules = vec![
        CustomRule {
            id: 1,
            enabled: true,
            match_type: CustomRuleMatch::Domain,
            value: "one.example".to_string(),
            action: CustomRuleAction::Direct,
        },
        CustomRule {
            id: 2,
            enabled: true,
            match_type: CustomRuleMatch::ProcessName,
            value: "Telegram".to_string(),
            action: CustomRuleAction::Proxy,
        },
        CustomRule {
            id: 3,
            enabled: true,
            match_type: CustomRuleMatch::Domain,
            value: "three.example".to_string(),
            action: CustomRuleAction::Block,
        },
    ];

    move_custom_rule(&mut rules, 3, -1, false);
    assert_eq!(
        rules.iter().map(|rule| rule.id).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
}

#[test]
fn diagnostic_report_lists_only_known_state() {
    let report = build_diagnostic_report(
        "running".to_string(),
        Some("1.13.14".to_string()),
        "macos",
        TunnelMode::Rule,
        true,
        false,
        3,
        2,
        10,
        1,
    );
    assert!(report.contains("内核版本: 1.13.14"));
    assert!(report.contains("TUN: 开启 · 局域网: 关闭 (本地代理 7890)"));
    assert!(report.contains("节点: 3 · 订阅: 2"));
    assert!(!report.contains("{"));
}

/// A remote written before DNS overrides existed carries no section, and a
/// pull keeps the local rules. The summary must not claim "0 条专用 DNS",
/// which reads as if the download had deleted them.
#[test]
fn dns_clause_distinguishes_absent_from_empty() {
    assert_eq!(dns_sync_clause(Some(3)), "3 条专用 DNS");
    assert_eq!(dns_sync_clause(Some(0)), "0 条专用 DNS");
    let absent = dns_sync_clause(None);
    assert!(!absent.starts_with('0'), "{absent}");
    assert!(absent.contains("保留本机"), "{absent}");
}

#[test]
fn missed_log_count_ignores_rows_the_reader_already_saw() {
    let entry = |sequence: u64| CoreLogEntry {
        sequence,
        timestamp: String::new(),
        level: "info".to_string(),
        message: String::new(),
        route: None,
    };
    let entries = [entry(7), entry(8), entry(9)];

    assert_eq!(log_entries_newer_than(&entries, 9), 0);
    assert_eq!(log_entries_newer_than(&entries, 8), 1);
    assert_eq!(log_entries_newer_than(&entries, 6), 3);
    // A buffer trimmed at the tail still reports what arrived, because the
    // count walks sequences rather than the buffer length.
    assert_eq!(log_entries_newer_than(&entries, 0), 3);
}

/// Every dialog in the app renders through `DialogShell`, which is what owns
/// Escape, the Tab ring and focus restore. A hand-rolled backdrop re-introduces
/// the bug where Escape did nothing and focus fell through to `document.body`.
#[test]
fn dialogs_all_use_the_shared_shell() {
    // Scan only the component source: this test quotes the very patterns it
    // counts, so reading itself back would inflate every total.
    let source = APP_SOURCE;

    assert_eq!(
        source.matches(r#"class: "modal-backdrop""#).count(),
        1,
        "only DialogShell may render a backdrop"
    );
    assert_eq!(
        source.matches(r#"role: "dialog""#).count(),
        1,
        "only DialogShell may declare a dialog role"
    );
    assert_eq!(
        source.matches("DialogShell {").count(),
        3,
        "import dialog, process picker and script editor all need the shell"
    );
    assert!(
        source.contains("Key::Escape"),
        "the shell must dismiss on Escape"
    );
}

/// F22：桌面侧栏把 6 个真实 `AppView` 导航项分成「连接 / 配置 / 运维」三组，
/// 每组带中文 eyebrow 标题。同步 / 更新是设置页内的功能，不是独立视图，因此
/// 不出现在分组里（不编造不存在的视图）；窄屏 tabbar 保持平铺、不分组。
#[test]
fn sidebar_groups_navigation_under_chinese_headings() {
    let source = APP_SOURCE;
    assert_eq!(
        source.matches("NavGroup {").count(),
        3,
        "桌面侧栏应分成三组导航"
    );
    for title in ["连接", "配置", "运维"] {
        assert!(
            source.contains(&format!("NavGroup {{ title: \"{title}\".to_string()")),
            "导航分组标题「{title}」应存在"
        );
    }
    assert!(
        source.contains(r#"p { class: "eyebrow nav-group-label""#),
        "分组标题应使用现有 eyebrow 类"
    );
}

/// `100vh` minus a hand-picked chrome height is the root cause of the resize
/// bugs: the moment the window is a different height the card is either short
/// of the fold or adds phantom page scroll. Only a dialog's `max-height` may
/// reference the viewport, and then as a cap rather than a target.
#[test]
fn layout_does_not_guess_the_viewport_chrome_height() {
    for (number, line) in css_lines() {
        let trimmed = line.trim();
        if !trimmed.contains("calc(100vh") || trimmed.starts_with("/*") || trimmed.contains(" `") {
            continue;
        }
        assert!(
            trimmed.starts_with("max-height:"),
            "line {number} sizes layout off the viewport height: {trimmed}"
        );
    }
}

/// The console card must be the flex child that absorbs the viewport, and the
/// row list must be the scroller inside it. Without both, rows past the card's
/// bottom are clipped by `overflow: hidden` and no wheel or touch gesture can
/// reach them.
#[test]
fn log_console_scrolls_its_rows_internally() {
    let workspace = css_rule(".log-workspace");
    assert!(
        workspace.contains("display: flex"),
        ".log-workspace must be a column"
    );
    assert!(
        workspace.contains("flex: 1 1 0"),
        ".log-workspace must absorb the remainder"
    );
    assert!(
        workspace.contains("overflow: hidden"),
        "the card clips its own scroller"
    );

    let list = css_rule(".log-workspace .log-list");
    assert!(
        list.contains("overflow-y: auto"),
        "the row list must scroll"
    );
    assert!(
        list.contains("min-height: 0"),
        "a flex scroller needs a zero floor"
    );

    let header = css_rule(".log-workspace .log-table-header");
    assert!(
        header.contains("position: sticky"),
        "captions stay pinned over the rows"
    );
    assert!(
        header.contains("var(--bg)"),
        "a translucent sticky header lets scrolled rows bleed through"
    );
}

/// `.script-modal` and `.modal` share one specificity, so declaration order
/// decides the width: the script editor silently shrank to the import dialog's
/// 510px when `.script-modal` sat earlier in the file.
#[test]
fn script_modal_widens_the_shared_modal() {
    let modal = SHEET
        .find("\n.modal {")
        .expect("the shared modal rule must exist");
    let script = SHEET
        .find("\n.script-modal {")
        .expect("the script modal width override must exist");
    assert!(
        script > modal,
        ".script-modal has to follow .modal or the 880px width loses"
    );
}

/// The console footer reserves the chip slot so a batch cannot jump the
/// layout, and the `<small>` in the chart footnote has to opt out of the
/// browser default (0.833em of a 10px parent measured 8.33px).
#[test]
fn small_text_and_follow_slot_stay_legible() {
    assert!(
        css_rule(".chart-foot small").contains("font-size: inherit"),
        "UA-default <small> shrinks below 9px inside .chart-foot"
    );
    assert!(css_rule(".log-follow-state").contains("min-height"));
}

/// The power control is the only affordance for starting and stopping the
/// core, and it used to render the same accent fill and the same glow in both
/// states - on phones the two rules carried a byte-identical `box-shadow`, so
/// "lit" told you nothing. The stopped state has to stay neutral and only
/// `.active` may wear the accent, in every breakpoint that styles the button.
#[test]
fn power_button_only_lights_up_when_running() {
    // Bodies exclude the selector line, so name the rule in the message.
    let stopped = css_rules(".power-button");
    let running = css_rules(".power-button.active");
    assert_eq!(stopped.len(), 2, "base and phone rule");
    assert_eq!(running.len(), 2, "base and phone rule");

    for rule in &stopped {
        assert!(
            !rule.contains("var(--accent-strong)"),
            ".power-button is the stopped state and must not wear the running fill: {rule}"
        );
    }
    // Only the base rule sets the fill; the phone rule deliberately overrides
    // the shadow alone and inherits it.
    assert!(
        running[0].contains("background: var(--accent-strong)"),
        ".active has to carry the running fill"
    );

    // The bug was specifically the phone pair sharing one shadow.
    let phone_shadow = |rules: &[&'static str]| -> Option<&'static str> {
        rules[1]
            .lines()
            .find_map(|line| line.trim().strip_prefix("box-shadow:"))
            .map(|value| value.trim().trim_end_matches(';').trim())
    };
    assert_ne!(
        phone_shadow(&stopped),
        phone_shadow(&running),
        "the phone rules need distinct shadows"
    );
    assert_eq!(
        phone_shadow(&stopped),
        Some("none"),
        "the phone stopped state must not glow"
    );
}

/// The stylesheet as authored, so the line numbers in failures match the file
/// on disk (`APP_CSS` prefixes it with the inlined fonts).
const SHEET: &str = include_str!("../../assets/styling/app.css");
/// The dialogs are declared in RSX, so the guard reads the component source.
/// All component sources concatenated: the shell used to be one `app.rs`,
/// now it is the `app/` module minus the tests themselves (this test
/// quotes the very patterns it counts).
const APP_SOURCE: &str = concat!(
    include_str!("tray.rs"),
    include_str!("runtime.rs"),
    include_str!("latency.rs"),
    include_str!("toast.rs"),
    include_str!("types.rs"),
    include_str!("proxy_app.rs"),
    include_str!("chrome.rs"),
    include_str!("overview.rs"),
    include_str!("toggles.rs"),
    include_str!("nodes.rs"),
    include_str!("subscriptions.rs"),
    include_str!("rules.rs"),
    include_str!("logs.rs"),
    include_str!("diagnostics.rs"),
    include_str!("sync.rs"),
    include_str!("updates.rs"),
    include_str!("settings.rs"),
    include_str!("domain.rs"),
);

fn css_lines() -> impl Iterator<Item = (usize, &'static str)> {
    SHEET
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
}

/// Body of the first rule whose selector appears at the start of a line.
fn css_rule(selector: &str) -> &'static str {
    css_rules(selector)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("missing CSS rule: {selector}"))
}

/// Every rule with this selector, in file order, so a media-query override
/// cannot hide behind the base rule.
///
/// The body ends at the rule's own closing brace, matched by brace depth
/// rather than by hunting for a `\n}`: rules inside a `@media` block close on
/// an indented line, and a plain column-zero search there swallows every rule
/// that follows - a guard so wide it could only ever pass by luck.
fn css_rules(selector: &str) -> Vec<&'static str> {
    let needle = format!("{selector} {{");
    let mut rules = Vec::new();
    let mut offset = 0;
    while let Some(found) = SHEET[offset..].find(&needle) {
        let start = offset + found + needle.len();
        let mut depth = 1usize;
        let mut cursor = start;
        while cursor < SHEET.len() {
            match SHEET.as_bytes()[cursor] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            cursor += 1;
        }
        assert!(depth == 0, "unterminated CSS rule: {selector}");
        rules.push(&SHEET[start..cursor]);
        offset = cursor;
    }
    assert!(!rules.is_empty(), "missing CSS rule: {selector}");
    rules
}
