//! Copy and units shared by every screen.
//!
//! Pure functions with no engine dependency, kept identical to the Dioxus UI so a
//! translated screen shows the same text. `cargo test` exercises them without a surface.

use crate::ui::theme::Tint;
use api::{RouteDecision, RouteTargetKind};
use proxy_core::{
    CustomRuleAction, CustomRuleMatch, ParseReport, ProxyGroup, ProxyGroupKind, ProxyNode,
    ProxyProtocol, Subscription, TunnelMode,
};
use std::collections::HashMap;

/// The core's fixed mixed port; the label only ever changes with LAN sharing.
pub const PROXY_PORT: u16 = 7890;

/// This build's version, as the about card and the diagnostic text name it.
///
/// Read from the crate rather than written down: the Dioxus shells pass
/// `env!("CARGO_PKG_VERSION")` of their own entry point, and an NSH build is one crate that
/// serves every window, so the same expression here reports the same kind of fact.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The interface language, said the way the original says it.
///
/// A constant rather than a setting: nothing in either front end switches language, and a row
/// that offered a menu of one would promise a feature the app does not have.
pub const LANGUAGE_LABEL: &str = "简体中文（暂不支持切换）";

/// Two-letter protocol marks for the node row's badge.
pub fn protocol_abbreviation(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::AnyTls => "AT",
        ProxyProtocol::Hysteria2 => "HY",
        ProxyProtocol::Vmess => "VM",
        ProxyProtocol::Vless => "VL",
        ProxyProtocol::Trojan => "TR",
        ProxyProtocol::Shadowsocks => "SS",
        ProxyProtocol::Http => "HT",
        ProxyProtocol::Socks5 => "SO",
    }
}

/// The status family a protocol belongs to, so groups read at a glance.
pub fn protocol_tint(protocol: ProxyProtocol) -> Tint {
    match protocol {
        ProxyProtocol::AnyTls | ProxyProtocol::Hysteria2 => Tint::Success,
        ProxyProtocol::Vmess | ProxyProtocol::Vless => Tint::Info,
        ProxyProtocol::Trojan => Tint::Dns,
        ProxyProtocol::Shadowsocks => Tint::Warning,
        ProxyProtocol::Http | ProxyProtocol::Socks5 => Tint::Danger,
    }
}

pub fn mode_label(mode: TunnelMode) -> &'static str {
    match mode {
        TunnelMode::Rule => "规则模式",
        TunnelMode::Global => "全局模式",
        TunnelMode::Direct => "直连模式",
    }
}

pub fn core_state_label(state: &str) -> &'static str {
    match state {
        "running" => "内核运行中",
        "stopped" => "内核已就绪",
        "checking" => "正在检测",
        _ => "内核不可用",
    }
}

/// The status family behind the core label: connected and idle share amber, and
/// only a missing core escalates to the danger color.
pub fn core_tint(connected: bool, state: &str) -> Tint {
    match (connected, state) {
        (true, _) => Tint::Success,
        (false, "unavailable") => Tint::Danger,
        _ => Tint::Warning,
    }
}

pub fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

pub fn format_data_amount(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else if bytes < GIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    }
}

pub fn format_data_rate(bytes_per_second: u64) -> String {
    format!("{}/s", format_data_amount(bytes_per_second))
}

pub fn proxy_port_label(allow_lan: bool) -> String {
    if allow_lan {
        format!("局域网代理 {PROXY_PORT}")
    } else {
        format!("本地代理 {PROXY_PORT}")
    }
}

/// Latency text plus the status family it deserves: a miss reads as a failure.
pub fn format_latency(latency_ms: Option<u64>, error: Option<&str>) -> (String, Tint) {
    match (latency_ms, error) {
        (Some(0), _) => ("超时".to_owned(), Tint::Danger),
        (Some(ms), _) => {
            let tint = if ms < 150 {
                Tint::Success
            } else if ms < 400 {
                Tint::Warning
            } else {
                Tint::Danger
            };
            (format!("{ms} ms"), tint)
        }
        (None, Some(_)) => ("失败".to_owned(), Tint::Danger),
        (None, None) => ("—".to_owned(), Tint::Info),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_switches_to_hours_past_one() {
        assert_eq!(format_uptime(0), "00:00");
        assert_eq!(format_uptime(65), "01:05");
        assert_eq!(format_uptime(3_661), "1:01:01");
    }

    #[test]
    fn data_amount_walks_the_binary_scale() {
        assert_eq!(format_data_amount(512), "512 B");
        assert_eq!(format_data_amount(2_048), "2.0 KiB");
        assert_eq!(format_data_rate(2_048), "2.0 KiB/s");
        assert_eq!(format_data_amount(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn a_zero_latency_reads_as_a_timeout_not_zero_ms() {
        assert_eq!(
            format_latency(Some(0), None),
            ("超时".to_owned(), Tint::Danger)
        );
        assert_eq!(
            format_latency(Some(80), None),
            ("80 ms".to_owned(), Tint::Success)
        );
        assert_eq!(
            format_latency(None, Some("refused")),
            ("失败".to_owned(), Tint::Danger)
        );
    }

    #[test]
    fn only_a_missing_core_escalates_to_danger() {
        assert_eq!(core_tint(true, "stopped"), Tint::Success);
        assert_eq!(core_tint(false, "stopped"), Tint::Warning);
        assert_eq!(core_tint(false, "unavailable"), Tint::Danger);
    }

    #[test]
    fn a_subscription_label_keeps_a_host_and_drops_credentials() {
        assert_eq!(
            source_label("https://user:pass@example.test/sub?token=1"),
            "https://example.test"
        );
        // Pasted node links have no host to name them by.
        assert_eq!(source_label("ss://a:b@host.test:8388#one"), "本地内容");
    }

    #[test]
    fn two_providers_sharing_a_tag_stay_distinct() {
        let first = namespace_nodes(1, one_node());
        let second = namespace_nodes(2, one_node());
        // The parser slugifies the fragment, so both providers hand back the same tag,
        // which is exactly the collision the namespace is for.
        assert_eq!(first[0].tag, "subscription-1-edge-1");
        assert_eq!(second[0].tag, "subscription-2-edge-1");
    }

    /// A refresh that returns nothing has to fail rather than empty the subscription: a
    /// provider that went quiet would otherwise take the only usable node list with it.
    #[test]
    fn a_refresh_returning_nothing_leaves_the_nodes_alone() {
        let parsed = parse_one_node();
        let mut subscriptions = vec![Subscription {
            id: 1,
            name: "实验室".to_owned(),
            source: "https://example.test/sub".to_owned(),
            nodes: namespace_nodes(1, parsed.nodes.clone()),
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        }];
        let empty = proxy_core::ParseReport {
            nodes: Vec::new(),
            rejected: vec![proxy_core::ParseIssue {
                line: 1,
                reason: "不支持的协议".to_owned(),
            }],
            proxy_server_nameservers: Vec::new(),
        };
        let reason = apply_subscription_report(&mut subscriptions, 1, empty)
            .expect_err("no nodes must be a failure");
        assert_eq!(reason, "不支持的协议");
        assert_eq!(subscriptions[0].node_count(), 1);

        apply_subscription_report(
            &mut subscriptions,
            1,
            proxy_core::ParseReport {
                nodes: parsed.nodes,
                rejected: Vec::new(),
                proxy_server_nameservers: vec!["223.5.5.5".to_owned()],
            },
        )
        .expect("a good report applies");
        assert_eq!(subscriptions[0].proxy_server_nameservers, ["223.5.5.5"]);
    }

    /// A pulled or hand-edited file can name a subscription that no longer exists. Falling
    /// back is what keeps the app startable; taking the stored id at face value would leave
    /// an empty node list and no way to get one.
    #[test]
    fn a_missing_active_subscription_falls_back_to_the_selected_node() {
        let subscriptions = vec![
            subscription(1, &["subscription-1-A"]),
            subscription(2, &["subscription-2-B"]),
        ];
        assert_eq!(
            resolve_active_subscription_id(&subscriptions, Some(9), "subscription-2-B"),
            Some(2)
        );
        assert_eq!(
            resolve_active_subscription_id(&subscriptions, None, ""),
            Some(1),
            "with nothing remembered, the first is what a user would expect"
        );
        assert_eq!(
            resolve_active_subscription_id(&subscriptions, Some(2), "subscription-2-B"),
            Some(2)
        );
    }

    #[test]
    fn a_selection_that_survives_a_refresh_is_kept() {
        let nodes = vec![node("subscription-1-A"), node("subscription-1-B")];
        assert_eq!(
            select_available_tag(&nodes, "subscription-1-B"),
            "subscription-1-B"
        );
        assert_eq!(
            select_available_tag(&nodes, "subscription-1-gone"),
            "subscription-1-A",
            "a removed node falls back to the first rather than to nothing"
        );
        assert_eq!(select_available_tag(&[], "anything"), "");
    }

    /// Only the namespaced outbounds are pruned. `direct` and a provider's own group names
    /// are real choices that have nothing to do with a node list.
    #[test]
    fn a_stale_outbound_is_dropped_and_a_group_name_is_not() {
        let mut selections = HashMap::from([
            ("proxy".to_owned(), "subscription-1-A".to_owned()),
            ("group".to_owned(), "美国流量".to_owned()),
            ("other".to_owned(), "direct".to_owned()),
        ]);
        prune_stale_group_selections(&mut selections, &["subscription-2-B".to_owned()]);
        assert_eq!(
            selections,
            HashMap::from([
                ("group".to_owned(), "美国流量".to_owned()),
                ("other".to_owned(), "direct".to_owned()),
            ])
        );
    }

    #[test]
    fn the_log_filters_partition_the_buffer() {
        let entries = vec![
            raw(1, "level info"),
            routed(2, RouteDecision::Proxy, "api.example.com", "proxy"),
            routed(3, RouteDecision::Direct, "lan.local", "direct"),
            routed(4, RouteDecision::Block, "ads.example.com", "block"),
        ];
        assert_eq!(
            visible_log_entries(&entries, LogFilter::Routes, "").len(),
            3
        );
        assert_eq!(visible_log_entries(&entries, LogFilter::Raw, "").len(), 1);
        assert_eq!(visible_log_entries(&entries, LogFilter::Proxy, "").len(), 1);
        // Newest first, so the console opens on the last thing that happened.
        assert_eq!(
            visible_log_entries(&entries, LogFilter::Routes, "")[0].sequence,
            4
        );
        // A search reaches the host, not the English message.
        assert_eq!(
            visible_log_entries(&entries, LogFilter::Routes, "api.example").len(),
            1
        );
        assert!(visible_log_entries(&entries, LogFilter::Routes, "nothing").is_empty());
    }

    /// Counting sequences rather than the buffer's length is what keeps the trim at 500 rows
    /// from reporting fewer missed entries than there were.
    #[test]
    fn missed_rows_are_counted_by_sequence_not_length() {
        let entries = vec![raw(10, "a"), raw(11, "b"), raw(12, "c")];
        assert_eq!(log_entries_newer_than(&entries, 11), 1);
        assert_eq!(log_entries_newer_than(&entries, 12), 0);
        assert_eq!(log_entries_newer_than(&entries, 0), 3);
    }

    #[test]
    fn the_chain_lists_hops_but_not_the_exit() {
        let detail = route(RouteDecision::Proxy, "example.com");
        let mut detail = detail;
        detail.outbound_chain = vec!["代理出口".to_owned(), "出口节点".to_owned()];
        detail.outbound_tag = "出口节点".to_owned();
        assert_eq!(route_chain_hops(&detail), ["代理出口"]);
    }

    /// What reaches the core is not what the list shows. A script that took over owns the
    /// route section, and a process rule the OS cannot report is a rule the user believes is
    /// working, so both come out of the request.
    #[test]
    fn the_request_drops_rules_the_core_cannot_honour() {
        let rules = vec![
            rule(CustomRuleMatch::DomainSuffix, "a.test"),
            rule(CustomRuleMatch::ProcessName, "Telegram"),
        ];
        assert_eq!(
            connection_custom_rules(false, "", rules.clone(), true).len(),
            2,
            "everything applies where process rules work"
        );
        assert_eq!(
            connection_custom_rules(false, "", rules.clone(), false)
                .iter()
                .map(|rule| rule.match_type)
                .collect::<Vec<_>>(),
            vec![CustomRuleMatch::DomainSuffix],
        );
        assert!(
            connection_custom_rules(true, "function main(c) { return c; }", rules.clone(), true)
                .is_empty(),
            "a live script owns the route section"
        );
        assert_eq!(
            connection_custom_rules(true, "   ", rules, true).len(),
            2,
            "an enabled but empty script applies nothing"
        );
    }

    fn rule(match_type: CustomRuleMatch, value: &str) -> proxy_core::CustomRule {
        proxy_core::CustomRule {
            id: 1,
            enabled: true,
            match_type,
            value: value.to_owned(),
            action: CustomRuleAction::Proxy,
        }
    }

    /// The headless test cannot assert this from a live app: whether the machine is already
    /// proxied is not the app's to decide, so the wording depends on state the test does not
    /// own. Every branch is here instead.
    #[test]
    fn a_parked_proxy_switch_says_why() {
        let ready = |enabled: bool, supported: bool| {
            SystemProxy::Ready(api::SystemProxyStatus {
                supported,
                enabled,
                detail: "已设置 macOS 系统代理 127.0.0.1:7890".to_owned(),
            })
        };
        assert_eq!(
            SystemProxy::Loading.note(false, false, false),
            "正在读取系统代理状态"
        );
        assert_eq!(
            SystemProxy::Failed("读取失败".to_owned()).note(false, false, false),
            "读取失败"
        );
        assert_eq!(
            ready(false, false).note(true, false, false),
            "当前平台不支持系统代理"
        );
        assert_eq!(
            ready(false, true).note(false, false, false),
            "先启动代理，才能开启系统代理"
        );
        // While the core is being started there is nothing to tell the user yet, and the
        // machine's own answer is the honest line.
        assert_eq!(
            ready(false, true).note(false, true, false),
            "已设置 macOS 系统代理 127.0.0.1:7890"
        );
        assert_eq!(
            ready(true, true).note(false, false, false),
            "已设置 macOS 系统代理 127.0.0.1:7890"
        );
        assert_eq!(
            ready(false, true).note(false, false, true),
            "正在应用到系统网络服务"
        );
        assert!(ready(false, false).is_ready());
        assert!(!SystemProxy::Loading.is_ready());
        assert!(!SystemProxy::Loading.is_supported());
        assert!(ready(true, true).is_enabled());
    }

    #[test]
    fn rule_ids_reuse_a_gap_rather_than_growing_forever() {
        let rules = vec![
            proxy_core::CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "a.test".to_owned(),
                action: CustomRuleAction::Proxy,
            },
            proxy_core::CustomRule {
                id: 3,
                enabled: true,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "b.test".to_owned(),
                action: CustomRuleAction::Proxy,
            },
        ];
        assert_eq!(next_custom_rule_id(&rules), Some(2));
        assert_eq!(next_dns_override_id(&[]), 1);
        assert_eq!(
            next_dns_override_id(&[proxy_core::DnsOverride {
                id: 4,
                enabled: true,
                domain_suffix: "a.test".to_owned(),
                server: "1.1.1.1".to_owned(),
            }]),
            5
        );
    }

    fn subscription(id: u64, tags: &[&str]) -> Subscription {
        Subscription {
            id,
            name: format!("订阅 {id}"),
            source: format!("https://{id}.test/sub"),
            nodes: tags.iter().map(|tag| node(tag)).collect(),
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        }
    }

    fn node(tag: &str) -> ProxyNode {
        ProxyNode {
            tag: tag.to_owned(),
            name: tag.to_owned(),
            protocol: ProxyProtocol::Vless,
            server: "edge.example.test".to_owned(),
            port: 443,
            auth: proxy_core::ProxyAuth::Uuid {
                uuid: "11111111-1111-1111-1111-111111111111".to_owned(),
                alter_id: None,
                flow: None,
            },
            transport: proxy_core::TransportOptions::default(),
            tls: proxy_core::TlsOptions {
                enabled: true,
                ..Default::default()
            },
            hysteria2: None,
        }
    }

    /// One link, parsed by the real parser, so the node a test hands around has the shape
    /// the app actually gets from a provider.
    fn parse_one_node() -> proxy_core::ParseReport {
        let report = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=tls#Edge",
        );
        assert_eq!(report.nodes.len(), 1, "{:?}", report.rejected);
        report
    }

    fn one_node() -> Vec<ProxyNode> {
        parse_one_node().nodes
    }

    fn raw(sequence: u64, message: &str) -> api::CoreLogEntry {
        api::CoreLogEntry {
            sequence,
            timestamp: "12:00:00".to_owned(),
            level: "info".to_owned(),
            message: message.to_owned(),
            route: None,
        }
    }

    fn route(decision: RouteDecision, host: &str) -> api::RouteLogDetail {
        api::RouteLogDetail {
            decision,
            target: host.to_owned(),
            host: host.to_owned(),
            port: Some(443),
            target_kind: RouteTargetKind::Domain,
            outbound_type: "direct".to_owned(),
            outbound_tag: "direct".to_owned(),
            outbound_chain: Vec::new(),
            source_ip: None,
        }
    }

    fn routed(sequence: u64, decision: RouteDecision, host: &str, _: &str) -> api::CoreLogEntry {
        api::CoreLogEntry {
            sequence,
            timestamp: "12:00:00".to_owned(),
            level: "info".to_owned(),
            message: format!("connection to {host}"),
            route: Some(route(decision, host)),
        }
    }
}

/// The id for a subscription being added: one past the highest in use, so a
/// deleted subscription's namespaced tags are never reused by a new one.
pub fn next_subscription_id(subscriptions: &[Subscription]) -> u64 {
    subscriptions
        .iter()
        .map(|subscription| subscription.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// Qualify every node tag with its subscription. Two providers can ship nodes
/// with the same tag, and the core addresses nodes by tag alone.
pub fn namespace_nodes(subscription_id: u64, mut nodes: Vec<ProxyNode>) -> Vec<ProxyNode> {
    for node in &mut nodes {
        node.tag = format!("subscription-{subscription_id}-{}", node.tag);
    }
    nodes
}

/// Fold a fresh parse into the subscription it came from. An empty result is an
/// error naming why, so a provider that went quiet reads as a failure rather
/// than as a subscription that legitimately has no nodes.
pub fn apply_subscription_report(
    subscriptions: &mut [Subscription],
    subscription_id: u64,
    report: ParseReport,
) -> Result<usize, String> {
    if report.nodes.is_empty() {
        return Err(report
            .rejected
            .first()
            .map(|issue| issue.reason.clone())
            .unwrap_or_else(|| "没有找到可用节点".to_string()));
    }
    let count = report.nodes.len();
    let rejected_count = report.rejected.len();
    let subscription = subscriptions
        .iter_mut()
        .find(|subscription| subscription.id == subscription_id)
        .ok_or_else(|| "订阅已不存在".to_string())?;
    subscription.nodes = namespace_nodes(subscription_id, report.nodes);
    subscription.proxy_server_nameservers = report.proxy_server_nameservers;
    subscription.rejected_count = rejected_count;
    Ok(count)
}

/// The nodes of one subscription. Only the active subscription's nodes reach the
/// core, which is what makes switching subscriptions change the exit.
pub fn subscription_nodes(
    subscriptions: &[Subscription],
    subscription_id: Option<u64>,
) -> Vec<ProxyNode> {
    subscription_id
        .and_then(|id| {
            subscriptions
                .iter()
                .find(|subscription| subscription.id == id)
        })
        .map(|subscription| subscription.nodes.clone())
        .unwrap_or_default()
}

/// Keep a remembered active subscription only while it still exists, then fall
/// back to the one that owns the selected node, then to the first. A pulled
/// snapshot leaves the id unset, and without a fallback the app could never
/// start again.
pub fn resolve_active_subscription_id(
    subscriptions: &[Subscription],
    requested_id: Option<u64>,
    selected_tag: &str,
) -> Option<u64> {
    requested_id
        .filter(|id| {
            subscriptions
                .iter()
                .any(|subscription| subscription.id == *id)
        })
        .or_else(|| {
            subscriptions
                .iter()
                .find(|subscription| {
                    subscription
                        .nodes
                        .iter()
                        .any(|node| node.tag == selected_tag)
                })
                .map(|subscription| subscription.id)
        })
        .or_else(|| subscriptions.first().map(|subscription| subscription.id))
}

/// The tag to select: what was asked for if it still exists, else the first.
pub fn select_available_tag(nodes: &[ProxyNode], requested_tag: &str) -> String {
    if nodes.iter().any(|node| node.tag == requested_tag) {
        requested_tag.to_owned()
    } else {
        nodes
            .first()
            .map(|node| node.tag.clone())
            .unwrap_or_default()
    }
}

/// Drop remembered proxy-group selections whose outbound no longer exists.
/// Refreshing or deleting a subscription rewrites namespaced tags, and a stale
/// outbound would otherwise ride along in every later request while the list
/// showed a defaulted choice. Choices that are not subscription nodes (built-in
/// group names, `direct`) are left alone.
pub fn prune_stale_group_selections(
    selections: &mut HashMap<String, String>,
    live_tags: &[String],
) {
    selections.retain(|_, outbound| {
        !outbound.starts_with("subscription-") || live_tags.iter().any(|tag| tag == outbound)
    });
}

pub fn all_live_node_tags(subscriptions: &[Subscription]) -> Vec<String> {
    subscriptions
        .iter()
        .flat_map(|subscription| subscription.nodes.iter())
        .map(|node| node.tag.clone())
        .collect()
}

/// Keep the built-in `proxy` group's remembered choice in step with the chosen node.
///
/// The two are one decision written in two places, and the engine reads the second one last:
/// `build_singbox_config` sets the `proxy` selector's `default` from `selected_tag`, and
/// `apply_proxy_group_selections` then overwrites it with `group_selections["proxy"]`. A pick
/// that moved only the first left the config pointing at the node the user had just replaced —
/// the list showed the new node, the core kept dialing the old one, and nothing said so.
pub fn sync_proxy_group_selection(selections: &mut HashMap<String, String>, selected_tag: &str) {
    if selected_tag.is_empty() {
        selections.remove("proxy");
    } else {
        selections.insert("proxy".to_owned(), selected_tag.to_owned());
    }
}

/// How a proxy group picks, in the words a user reads rather than the engine's.
pub fn proxy_group_kind_label(kind: ProxyGroupKind) -> &'static str {
    match kind {
        ProxyGroupKind::Selector => "手动选择",
        ProxyGroupKind::UrlTest => "自动测速",
    }
}

/// What an outbound is called where it is shown. A member that is not one of the subscription's
/// nodes is one of the engine's own outbounds, and naming those is the point: `direct` in a
/// list reads as a broken node unless it says 直连.
pub fn outbound_display_name(tag: &str, nodes: &[ProxyNode]) -> String {
    nodes
        .iter()
        .find(|node| node.tag == tag)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| match tag {
            "direct" => "直连".to_owned(),
            "block" => "拦截".to_owned(),
            other => other.to_owned(),
        })
}

/// One entry of a group's member list: a node, or a bare outbound (another group, `direct`,
/// `block`) that has no node behind it and so no protocol, address or measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyGroupMember {
    pub tag: String,
    pub node: Option<ProxyNode>,
    pub nested_kind: Option<ProxyGroupKind>,
}

/// Which outbound a group is on: the remembered choice while it still names a member, the
/// config's own default otherwise. A remembered choice that no longer exists is dropped rather
/// than shown, because listing it would claim a selection the core silently ignored.
pub fn group_selected(group: &ProxyGroup, selections: &HashMap<String, String>) -> String {
    selections
        .get(&group.tag)
        .filter(|outbound| group.outbounds.iter().any(|item| item == *outbound))
        .cloned()
        .unwrap_or_else(|| group.selected.clone())
}

/// The groups to list, each with the members the filter leaves and the outbound it is on.
///
/// A group whose members are all filtered out is dropped rather than drawn as a heading over
/// nothing: with a search box in reach, an empty section reads as a broken subscription.
pub fn visible_proxy_groups(
    groups: &[ProxyGroup],
    nodes: &[ProxyNode],
    selections: &HashMap<String, String>,
    latency: &HashMap<String, api::NodeLatency>,
    needle: &str,
) -> Vec<(ProxyGroup, Vec<ProxyGroupMember>, String)> {
    let needle = needle.trim().to_lowercase();
    groups
        .iter()
        .map(|group| {
            let mut members = group_members(group, groups, nodes, &needle);
            sort_group_members(&mut members, latency);
            (group.clone(), members, group_selected(group, selections))
        })
        .filter(|(_, members, _)| !members.is_empty())
        .collect()
}

fn group_members(
    group: &ProxyGroup,
    groups: &[ProxyGroup],
    nodes: &[ProxyNode],
    needle: &str,
) -> Vec<ProxyGroupMember> {
    group
        .outbounds
        .iter()
        .filter_map(|tag| {
            let node = nodes.iter().find(|node| &node.tag == tag).cloned();
            let nested_kind = groups
                .iter()
                .find(|candidate| &candidate.tag == tag)
                .map(|candidate| candidate.kind);
            let haystacks: Vec<String> = match &node {
                Some(node) => vec![
                    node.name.to_lowercase(),
                    node.server.to_lowercase(),
                    format!("{:?}", node.protocol).to_lowercase(),
                    node.protocol.label().to_lowercase(),
                    tag.to_lowercase(),
                ],
                None => vec![tag.to_lowercase()],
            };
            let matches = needle.is_empty() || haystacks.iter().any(|field| field.contains(needle));
            matches.then(|| ProxyGroupMember {
                tag: tag.clone(),
                node,
                nested_kind,
            })
        })
        .collect()
}

/// Measured nodes first, fastest first, then the unmeasured, then the failures. A group of
/// thirty nodes is opened to find one that works, so the ones already known to answer sit at
/// the top and a node that failed a probe is not recommended by being left in the middle.
pub fn sort_group_members(
    members: &mut [ProxyGroupMember],
    latency: &HashMap<String, api::NodeLatency>,
) {
    members.sort_by_key(|member| {
        member
            .node
            .as_ref()
            .and_then(|node| latency.get(&node.tag))
            .map(|result| match result.latency_ms {
                Some(latency) => (0, latency),
                None => (2, u64::MAX),
            })
            .unwrap_or((1, u64::MAX))
    });
}

/// Whether a group's choice can be moved by a press. An `urltest` group picks its own member
/// by measuring it; offering it as a list of buttons would let the user make a selection the
/// engine overrules within its check interval.
pub const fn group_is_selectable(kind: ProxyGroupKind) -> bool {
    matches!(kind, ProxyGroupKind::Selector)
}

/// A subscription's display name when the user did not give one: the URL's
/// scheme and host, with credentials stripped, or a label for pasted content.
pub fn source_label(source: &str) -> String {
    let trimmed = source.trim();
    if let Some((scheme, remainder)) = trimmed.split_once("://") {
        if matches!(scheme, "http" | "https") {
            let authority = remainder.split(['/', '?', '#']).next().unwrap_or(remainder);
            let host = authority.rsplit('@').next().unwrap_or(authority);
            return format!("{scheme}://{host}");
        }
    }
    "本地内容".to_string()
}

/// How a custom rule matches. Labels come from the domain rather than the page,
/// so the list, the editor and the log agree on the wording.
pub fn custom_rule_match_label(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "精确域名",
        CustomRuleMatch::DomainSuffix => "域名后缀",
        CustomRuleMatch::DomainKeyword => "域名关键字",
        CustomRuleMatch::IpCidr => "IP CIDR",
        CustomRuleMatch::ProcessName => "进程名",
        CustomRuleMatch::ProcessPath => "进程路径",
    }
}

/// The example shown in the rule's value field, which is what teaches the
/// expected form without a paragraph of explanation.
pub fn custom_rule_placeholder(match_type: CustomRuleMatch) -> &'static str {
    match match_type {
        CustomRuleMatch::Domain => "api.example.com",
        CustomRuleMatch::DomainSuffix => "example.com",
        CustomRuleMatch::DomainKeyword => "google",
        CustomRuleMatch::IpCidr => "203.0.113.0/24",
        CustomRuleMatch::ProcessName => "Telegram",
        CustomRuleMatch::ProcessPath => "/Applications/Telegram.app/Contents/MacOS/Telegram",
    }
}

pub fn custom_rule_action_label(action: CustomRuleAction) -> &'static str {
    match action {
        CustomRuleAction::Direct => "直连",
        CustomRuleAction::Proxy => "代理",
        CustomRuleAction::Block => "拦截",
    }
}

/// Whether a rule of this kind can ever match here. Process rules need the OS to report
/// the owning process of a connection, which iOS and Android cannot; a rule that can only
/// sit unused is hidden rather than listed as dead weight. `process_rules_supported` comes
/// from the caller, so this stays a pure function a test can drive without an engine.
pub fn custom_rule_available(match_type: CustomRuleMatch, process_rules_supported: bool) -> bool {
    match match_type {
        CustomRuleMatch::ProcessName | CustomRuleMatch::ProcessPath => process_rules_supported,
        _ => true,
    }
}

/// The status family a rule's action belongs to. A rule's action and a log entry's decision
/// are the same three outcomes, so they wear the same three colors: the list, the editor and
/// the console then read as one vocabulary.
pub fn custom_rule_action_tint(action: CustomRuleAction) -> Tint {
    match action {
        CustomRuleAction::Direct => Tint::Info,
        CustomRuleAction::Proxy => Tint::Success,
        CustomRuleAction::Block => Tint::Danger,
    }
}

/// Where a routed connection went. These three words are the whole point of the
/// log page, so they must match the wording used by the rules.
pub fn route_decision_label(decision: RouteDecision) -> &'static str {
    match decision {
        RouteDecision::Direct => "直连",
        RouteDecision::Proxy => "代理",
        RouteDecision::Block => "拦截",
    }
}

pub fn route_decision_tint(decision: RouteDecision) -> Tint {
    match decision {
        RouteDecision::Direct => Tint::Info,
        RouteDecision::Proxy => Tint::Success,
        RouteDecision::Block => Tint::Danger,
    }
}

pub fn route_target_kind_label(kind: RouteTargetKind) -> &'static str {
    match kind {
        RouteTargetKind::Domain => "域名",
        RouteTargetKind::Ip => "IP",
    }
}

/// What the log page shows. `Routes` and `Raw` split the buffer by whether the core said
/// where a connection went; the other three are the decisions themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFilter {
    Routes,
    Direct,
    Proxy,
    Block,
    Raw,
}

impl LogFilter {
    pub fn label(self) -> &'static str {
        match self {
            LogFilter::Routes => "全部路由",
            LogFilter::Direct => "直连",
            LogFilter::Proxy => "代理",
            LogFilter::Block => "拦截",
            LogFilter::Raw => "内核日志",
        }
    }

    pub const ALL: [LogFilter; 5] = [
        LogFilter::Routes,
        LogFilter::Proxy,
        LogFilter::Direct,
        LogFilter::Block,
        LogFilter::Raw,
    ];
}

pub fn log_matches_filter(entry: &api::CoreLogEntry, filter: LogFilter) -> bool {
    match filter {
        LogFilter::Routes => entry.route.is_some(),
        LogFilter::Raw => entry.route.is_none(),
        LogFilter::Direct => decision_of(entry) == Some(RouteDecision::Direct),
        LogFilter::Proxy => decision_of(entry) == Some(RouteDecision::Proxy),
        LogFilter::Block => decision_of(entry) == Some(RouteDecision::Block),
    }
}

fn decision_of(entry: &api::CoreLogEntry) -> Option<RouteDecision> {
    entry.route.as_ref().map(|route| route.decision)
}

/// Whether a search finds this row. The host, the exit and the chain are searched as well as
/// the message, because "telegram" is what a user types and it appears in the host, not the
/// English log line.
pub fn log_matches_search(entry: &api::CoreLogEntry, query: &str) -> bool {
    if query.is_empty() || entry.message.to_ascii_lowercase().contains(query) {
        return true;
    }
    entry.route.as_ref().is_some_and(|route| {
        route.host.to_ascii_lowercase().contains(query)
            || route.outbound_tag.to_ascii_lowercase().contains(query)
            || route.outbound_type.to_ascii_lowercase().contains(query)
            || route
                .source_ip
                .as_ref()
                .is_some_and(|ip| ip.to_ascii_lowercase().contains(query))
            || route
                .outbound_chain
                .iter()
                .any(|tag| tag.to_ascii_lowercase().contains(query))
    })
}

/// Newest first: the console reads down from the most recent connection, and a reader who
/// arrives wants the last thing that happened rather than the oldest row still in the buffer.
pub fn visible_log_entries(
    stored: &[api::CoreLogEntry],
    filter: LogFilter,
    query: &str,
) -> Vec<api::CoreLogEntry> {
    stored
        .iter()
        .rev()
        .filter(|entry| log_matches_filter(entry, filter))
        .filter(|entry| log_matches_search(entry, query))
        .cloned()
        .collect()
}

/// How many rows arrived since `previous`. Counting sequences rather than the buffer's
/// length keeps the trim at 500 rows from under-reporting what a reader who scrolled away has
/// missed.
pub fn log_entries_newer_than(entries: &[api::CoreLogEntry], previous: u64) -> usize {
    entries
        .iter()
        .filter(|entry| entry.sequence > previous)
        .count()
}

/// The hops a connection took before its exit. The core reports the chain, never a rule name,
/// so the page lists the real group hops rather than guessing at a rule; the exit itself is
/// shown in its own column, so it is dropped here to avoid saying it twice.
pub fn route_chain_hops(route: &api::RouteLogDetail) -> Vec<String> {
    route
        .outbound_chain
        .iter()
        .filter(|tag| tag.as_str() != route.outbound_tag.as_str())
        .cloned()
        .collect()
}

/// The rules a connection should be built with.
///
/// Two reductions, both of which change what the core does:
///
/// - A config script that is on and non-empty owns the whole route section, so the app's own
///   rules are dropped rather than layered under it. Sending both would have the script and
///   the list argue about the same connection.
/// - A process rule can only match where the OS reports the owning process. Sending one that
///   cannot match is not harmless on a platform that ignores the field: it is a rule the user
///   believes is working.
pub fn connection_custom_rules(
    config_script_enabled: bool,
    config_script: &str,
    custom_rules: Vec<proxy_core::CustomRule>,
    process_rules_supported: bool,
) -> Vec<proxy_core::CustomRule> {
    if config_script_enabled && !config_script.trim().is_empty() {
        return Vec::new();
    }
    custom_rules
        .into_iter()
        .filter(|rule| custom_rule_available(rule.match_type, process_rules_supported))
        .collect()
}

/// What the backend says about the operating system's proxy setting.
///
/// Read rather than remembered: the OS proxy changes underneath the app whenever the core
/// goes down, and a switch holding a remembered value would tell the user their traffic was
/// proxied when the port behind it had already closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemProxy {
    /// Not read yet.
    Loading,
    Ready(api::SystemProxyStatus),
    /// The read itself failed, with the reason.
    Failed(String),
}

impl SystemProxy {
    /// The line under the switch: what the setting is, and when it cannot be changed, why
    /// not. A parked switch with no reason reads as a broken control.
    pub fn note(&self, core_running: bool, core_action_busy: bool, setting_busy: bool) -> String {
        if setting_busy {
            return "正在应用到系统网络服务".to_owned();
        }
        if cfg!(target_os = "android") {
            // Android's VPN owns every route, so the backend's "not implemented" would read
            // as a broken feature rather than a platform that does not need the switch.
            return "Android 由 VPN 接管，无需系统代理".to_owned();
        }
        match self {
            SystemProxy::Loading => "正在读取系统代理状态".to_owned(),
            SystemProxy::Failed(error) => error.clone(),
            SystemProxy::Ready(status) => {
                if !status.supported {
                    return "当前平台不支持系统代理".to_owned();
                }
                // The machine's own words, except for the one case the user can act on:
                // turning this on with nothing behind the port sends the machine's traffic to
                // a closed listener.
                if status.enabled || core_running || core_action_busy {
                    return status.detail.clone();
                }
                "先启动代理，才能开启系统代理".to_owned()
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        matches!(self, SystemProxy::Ready(status) if status.enabled)
    }

    /// Whether the switch has an answer to act on. Unread and unreadable both park it, since
    /// writing a setting whose state is unknown can leave the machine misconfigured.
    pub fn is_ready(&self) -> bool {
        matches!(self, SystemProxy::Ready(_))
    }

    /// Whether the platform can be pointed at the port at all.
    pub fn is_supported(&self) -> bool {
        matches!(self, SystemProxy::Ready(status) if status.supported)
    }
}

/// The first id missing from the list, so a delete followed by an add does not hand out an
/// id a log row still refers to.
pub fn next_custom_rule_id(rules: &[proxy_core::CustomRule]) -> Option<u64> {
    (1..=(rules.len() as u64 + 1)).find(|id| rules.iter().all(|rule| rule.id != *id))
}

/// The same for the DNS overrides, which are numbered separately: a rule and an override
/// sharing an id is harmless because they are never looked up together.
pub fn next_dns_override_id(overrides: &[proxy_core::DnsOverride]) -> u64 {
    overrides.iter().map(|item| item.id).max().unwrap_or(0) + 1
}

/// Whether this build can match on the owning process. iOS and Android cannot report it,
/// so the option is withheld rather than offered dead.
pub const fn process_rules_supported() -> bool {
    !cfg!(any(target_os = "android", target_os = "ios"))
}

/// The core's version, or the words that say there isn't one.
///
/// Shared by the about card and the diagnostic text so the two can never disagree about
/// whether the version is known.
pub fn core_version_label(core_version: Option<&str>) -> String {
    match core_version {
        Some(version) => version.to_owned(),
        None => "未检测到版本".to_owned(),
    }
}

/// The line under the app's name: which build of the app, which core, which platform.
///
/// The three facts a bug report opens with, in the order anyone reading it needs them.
pub fn about_build_label(app_version: &str, core_version: Option<&str>, platform: &str) -> String {
    let core = match core_version {
        Some(version) => format!("sing-box {version}"),
        None => "sing-box 未就绪".to_owned(),
    };
    format!("v{app_version} · {core} · {platform}")
}

/// Everything the diagnostic text reports, read off the store in one go.
///
/// A struct rather than ten arguments: the report is assembled once, from signals, and a
/// list of fields says what it covers better than a call site that runs past the edge of
/// the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticFacts {
    pub app_version: String,
    pub platform: String,
    pub core_state: String,
    pub core_version: Option<String>,
    pub tunnel_mode: TunnelMode,
    pub tun_enabled: bool,
    pub allow_lan: bool,
    pub node_count: usize,
    pub subscription_count: usize,
    pub rule_count: usize,
    pub dns_count: usize,
}

/// The plain text to paste into a bug report.
///
/// Every line mirrors something the screens already show — nothing here is invented or
/// measured on the spot, so the text cannot claim a state the app has not reported. The
/// core's raw state string travels beside its label because a developer reading the report
/// wants `unavailable`, not `内核不可用`.
pub fn diagnostic_report(facts: &DiagnosticFacts) -> String {
    format!(
        "Kitty Pro 诊断信息\n\
         界面平台: {platform}\n\
         内核状态: {state_label} ({core_state})\n\
         内核版本: {core_version}\n\
         分流模式: {mode}\n\
         TUN: {tun} · 局域网: {lan} ({endpoint})\n\
         节点: {node_count} · 订阅: {subscription_count}\n\
         自定义规则: {rule_count} · 专用 DNS: {dns_count}",
        platform = facts.platform,
        state_label = core_state_label(&facts.core_state),
        core_state = facts.core_state,
        core_version = core_version_label(facts.core_version.as_deref()),
        mode = mode_label(facts.tunnel_mode),
        tun = if facts.tun_enabled {
            "开启"
        } else {
            "关闭"
        },
        lan = if facts.allow_lan { "开启" } else { "关闭" },
        endpoint = proxy_port_label(facts.allow_lan),
        node_count = facts.node_count,
        subscription_count = facts.subscription_count,
        rule_count = facts.rule_count,
        dns_count = facts.dns_count,
    )
}

/// A card of the settings page, named by what it holds rather than by its heading.
///
/// The search filters by card and not by row: a page that hid half of a card reads as a
/// broken layout, and the card is the unit a user chose to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    Appearance,
    Mode,
    Scope,
    Session,
    Sync,
    Updates,
    Script,
    Links,
    About,
}

impl SettingsSection {
    /// The words this card answers to, in the page's own vocabulary plus the English a user
    /// may type when the Chinese term is on the tip of their tongue.
    ///
    /// A substring table rather than a search index: the page has seven cards, and the
    /// cheapest honest rule is whether the typed word appears among the card's names.
    fn keywords(self) -> &'static str {
        match self {
            SettingsSection::Appearance => "外观 深色 浅色 主题 语言 配色 appearance 简体中文",
            SettingsSection::Mode => "流量走向 分流 模式 规则模式 全局模式 直连模式 routing",
            SettingsSection::Scope => "接管方式 tun 局域网 允许局域网 系统代理 网络 权限 scope",
            SettingsSection::Session => {
                "启动与终端 自动启动 开机 启动 登录 登录项 环境变量 终端 命令行 autostart shell"
            }
            SettingsSection::Sync => {
                "同步 云同步 远端 上传 下载 覆盖 webdav s3 bucket 拉取 推送 快照 配置 sync 存储 冲突"
            }
            SettingsSection::Updates => {
                "更新 升级 版本 检查 下载 发布 发行 忽略 有新版本 update 自动检查 github"
            }
            SettingsSection::Script => {
                "配置脚本 内核 sing-box javascript 配置覆写 脚本 quickjs core"
            }
            SettingsSection::Links => "配置 订阅 规则 日志 管理",
            SettingsSection::About => {
                "关于 诊断 内核状态 内核版本 节点 订阅 规则 dns 复制 about 版本"
            }
        }
    }

    /// Whether a card shows for this query. An empty query shows everything, and matching is
    /// on the lowercased string both sides, so `TUN` and `tun` behave the same.
    ///
    /// The direction matters: the *query* must appear inside the card's words, so typing a
    /// whole sentence finds nothing rather than matching its first word.
    pub fn matches(self, keyword: &str) -> bool {
        keyword.is_empty() || self.keywords().contains(keyword)
    }
}

/// A query in the form every filter on this app compares against.
///
/// Trimmed and lowercased in one place: the settings cards, the process picker and a node
/// search all match by substring, and two of them normalising while one does not is how a
/// trailing space makes a list look empty.
pub fn search_keyword(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Which cards survive the query, in page order.
///
/// Takes the box's raw text and normalises it here, so a caller cannot reach the cards with an
/// untrimmed query: a search box holding only spaces is an empty search, and treating it as a
/// filter would blank the page for someone who typed nothing.
pub fn visible_settings_sections(query: &str) -> Vec<SettingsSection> {
    use SettingsSection as S;
    let keyword = search_keyword(query);
    [
        S::Appearance,
        S::Mode,
        S::Scope,
        S::Session,
        S::Sync,
        S::Updates,
        S::Script,
        S::Links,
        S::About,
    ]
    .into_iter()
    .filter(|section| section.matches(&keyword))
    .collect()
}

/// The name a process is matched by, for the rule editor's picker.
///
/// A path rule wants the path the OS reported; where it reported none, the name is the only
/// thing worth filling in, and an empty field would be a worse answer than a partial one.
pub fn process_rule_value(process: &api::RunningProcess, match_type: CustomRuleMatch) -> String {
    match match_type {
        CustomRuleMatch::ProcessPath => process
            .exe_path
            .clone()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| process.name.clone()),
        _ => process.name.clone(),
    }
}

/// Whether a process answers to this query: by name, or by the path it was launched from.
pub fn process_matches(process: &api::RunningProcess, keyword: &str) -> bool {
    if keyword.is_empty() {
        return true;
    }
    process.name.to_lowercase().contains(keyword)
        || process
            .exe_path
            .as_deref()
            .is_some_and(|path| path.to_lowercase().contains(keyword))
}

/// What the remote-sync card says about the last exchange with the remote.
///
/// Three states and not two: an enabled sync that has never run still has work to do, and
/// calling that 已同步 would send the user away believing their other devices were current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    /// The card is still reading `sync.json`.
    Loading,
    /// Off, so neither button does anything yet.
    Disabled,
    /// On, and nothing has been pushed or pulled from this device yet.
    Pending,
    /// At least one exchange completed, at `updated_at`.
    Synced(u64),
}

impl SyncStatus {
    pub fn label(self) -> &'static str {
        match self {
            SyncStatus::Loading => "读取中",
            SyncStatus::Disabled => "未启用",
            SyncStatus::Pending => "待同步",
            SyncStatus::Synced(_) => "已同步",
        }
    }

    /// The status family the badge wears: a completed sync is the good state, an enabled but
    /// never-run one is the warning that something is outstanding, and off is neither.
    pub fn tint(self) -> Tint {
        match self {
            SyncStatus::Synced(_) => Tint::Success,
            SyncStatus::Pending => Tint::Warning,
            SyncStatus::Loading | SyncStatus::Disabled => Tint::Neutral,
        }
    }

    /// When the last exchange happened, said in local terms rather than as a raw epoch number.
    ///
    /// Days rather than hours because a sync baseline is something a user checks on the order
    /// of weekly, and `4320 小时前` is unreadable at exactly the scale that matters.
    pub fn since_label(self, now: u64) -> Option<String> {
        let SyncStatus::Synced(at) = self else {
            return None;
        };
        let ago = now.saturating_sub(at);
        const DAY: u64 = 86_400;
        Some(if ago < 60 {
            "刚刚".to_owned()
        } else if ago < DAY {
            format!("{} 小时前", ago / 3_600)
        } else {
            format!("{} 天前", ago / DAY)
        })
    }
}

/// The remote's contents in the words a replacement decision needs.
///
/// A pull overwrites local subscriptions and rules, so the confirm line has to count both sides;
/// a remote that has never been read reports `None` counts and says so rather than claiming zero,
/// which would read as "the remote is empty" right before the user replaced their own work with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncCounts {
    pub remote_subscriptions: usize,
    pub remote_rules: usize,
}

/// The summary under 下载覆盖本机 once it has been armed.
pub fn pull_confirm_summary(
    remote: Option<SyncCounts>,
    local_subscriptions: usize,
    local_rules: usize,
) -> String {
    match remote {
        Some(counts) => format!(
            "将用远端 {} 个订阅 / {} 条规则替换本机 {local_subscriptions} 个订阅 / {local_rules} 条规则",
            counts.remote_subscriptions, counts.remote_rules
        ),
        None => format!(
            "将用远端配置替换本机 {local_subscriptions} 个订阅 / {local_rules} 条规则（远端数量需下载后确认）"
        ),
    }
}

/// What one sync exchange carried, turned into the sentence the notice reads.
///
/// `dns_count` is `Option` for the same reason the backend's is: an older remote document has no
/// DNS section at all, and the local list survives that case. Saying "0 条专用 DNS" would report
/// an erasure that did not happen.
pub fn sync_result_clause(
    subscription_count: usize,
    rule_count: usize,
    dns_count: Option<usize>,
) -> String {
    let dns = match dns_count {
        Some(count) => format!("{count} 条专用 DNS"),
        None => "远端未含专用 DNS（保留本机）".to_owned(),
    };
    format!("已同步 {subscription_count} 个订阅、{rule_count} 条分流规则、{dns}")
}

/// Turn the backend's optimistic-concurrency wording into a sentence with an action in it.
///
/// The remote answers a rejected push with ETag and baseline jargon. The user cannot act on that,
/// and the raw text stays available under 详情 for whoever can, so this maps each known shape of
/// refusal onto the one move that resolves it.
pub fn classify_sync_error(raw: &str) -> (String, bool) {
    if raw.contains("已被其他设备修改") || raw.contains("已被其它设备修改") {
        (
            "远端配置已被其它设备更新，请先下载覆盖本机或明确覆盖远端".to_owned(),
            true,
        )
    } else if raw.contains("ETag") || raw.contains("etag") {
        (
            "远端存储不支持安全上传，请改用「覆盖远端」".to_owned(),
            true,
        )
    } else if raw.contains("基线") {
        (
            "本机尚无该远端目标的同步基线，请先下载覆盖本机或覆盖远端".to_owned(),
            true,
        )
    } else if raw.contains("revision") || raw.contains("Revision") {
        (
            "远端版本校验未通过，请下载覆盖本机或覆盖远端".to_owned(),
            true,
        )
    } else {
        (raw.to_owned(), false)
    }
}

/// Which fields a provider's form holds.
///
/// Two providers, two disjoint address sets, on purpose: `webdav_endpoint` and `s3_endpoint` are
/// separate stored fields precisely so switching the segment can never save an address typed for
/// the other one. The enum is that decision made visible in the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncProviderForm {
    WebDav,
    S3,
}

impl SyncProviderForm {
    pub fn from_kind(kind: api::SyncProviderKind) -> Self {
        match kind {
            api::SyncProviderKind::WebDav => SyncProviderForm::WebDav,
            api::SyncProviderKind::S3 => SyncProviderForm::S3,
        }
    }

    /// The segment's label. Both are proper nouns of the storage being spoken to, so they stay
    /// untranslated on a Chinese page, as they do in the Dioxus shell.
    pub fn label(self) -> &'static str {
        match self {
            SyncProviderForm::WebDav => "WebDAV",
            SyncProviderForm::S3 => "S3 兼容",
        }
    }

    /// Whether the address field names a public service or a self-hosted one, which is the one
    /// thing the placeholder has to say.
    pub fn endpoint_label(self) -> &'static str {
        match self {
            SyncProviderForm::WebDav => "WebDAV 地址",
            SyncProviderForm::S3 => "S3 Endpoint",
        }
    }

    pub fn endpoint_placeholder(self) -> &'static str {
        match self {
            SyncProviderForm::WebDav => "https://dav.example.com/dav/files/user",
            // S3's empty endpoint is a meaningful value: it selects AWS. The placeholder has to
            // say that, or a user leaves it blank believing nothing was chosen.
            SyncProviderForm::S3 => "留空使用 AWS，或填写 https://s3.example.com",
        }
    }

    /// The credential fields, in the order a user fills them.
    ///
    /// Each names the stored field it edits, which is what lets one setter own all nine: the form
    /// iterates this list rather than keeping its own copy of which label writes where, and those
    /// two copies drifting is how a typed password ends up saved into the username.
    pub fn fields(self) -> Vec<SyncFieldSpec> {
        match self {
            SyncProviderForm::WebDav => vec![
                SyncFieldSpec::new(SyncField::Username, "用户名", "", false),
                SyncFieldSpec::new(
                    SyncField::Password,
                    "密码 / 应用密码",
                    "网盘的应用密码比主密码更安全",
                    true,
                ),
            ],
            SyncProviderForm::S3 => vec![
                SyncFieldSpec::new(SyncField::Bucket, "Bucket", "", false),
                SyncFieldSpec::new(SyncField::Region, "Region", "us-east-1", false),
                SyncFieldSpec::new(SyncField::AccessKey, "Access Key", "", false),
                SyncFieldSpec::new(SyncField::SecretKey, "Secret Key", "", true),
            ],
        }
    }
}

/// One field of the sync form: which stored value it edits, what it is called, and whether it is
/// a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFieldSpec {
    pub field: SyncField,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub secret: bool,
}

impl SyncFieldSpec {
    fn new(field: SyncField, label: &'static str, placeholder: &'static str, secret: bool) -> Self {
        Self {
            field,
            label,
            placeholder,
            secret,
        }
    }
}

/// Which stored field a sync form input edits.
///
/// An enum rather than nine closures because the form and the store then agree on one list of
/// names: the page cannot write a field the config does not have, and adding a provider field is
/// one variant plus one row in `SyncProviderForm::fields`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncField {
    WebDavEndpoint,
    S3Endpoint,
    Path,
    Username,
    Password,
    Bucket,
    Region,
    AccessKey,
    SecretKey,
}

impl SyncField {
    pub fn read(self, config: &api::SyncConfig) -> &str {
        match self {
            SyncField::WebDavEndpoint => &config.webdav_endpoint,
            SyncField::S3Endpoint => &config.s3_endpoint,
            SyncField::Path => &config.path,
            SyncField::Username => &config.username,
            SyncField::Password => &config.password,
            SyncField::Bucket => &config.bucket,
            SyncField::Region => &config.region,
            SyncField::AccessKey => &config.access_key,
            SyncField::SecretKey => &config.secret_key,
        }
    }

    pub fn write(self, config: &mut api::SyncConfig, value: &str) {
        let slot = match self {
            SyncField::WebDavEndpoint => &mut config.webdav_endpoint,
            SyncField::S3Endpoint => &mut config.s3_endpoint,
            SyncField::Path => &mut config.path,
            SyncField::Username => &mut config.username,
            SyncField::Password => &mut config.password,
            SyncField::Bucket => &mut config.bucket,
            SyncField::Region => &mut config.region,
            SyncField::AccessKey => &mut config.access_key,
            SyncField::SecretKey => &mut config.secret_key,
        };
        *slot = value.to_owned();
    }
}

/// Whether a sync target is complete enough to try.
///
/// Checked here rather than left to the remote, because an address with no path writes to the
/// storage root and a path with no address cannot be dialed at all — both fail after a request
/// the user paid for, and both are visible before one is sent.
pub fn sync_target_incomplete(
    form: SyncProviderForm,
    config: &api::SyncConfig,
) -> Option<&'static str> {
    if config.path.trim().is_empty() {
        return Some("远程路径");
    }
    let address = match form {
        SyncProviderForm::WebDav => &config.webdav_endpoint,
        // S3's blank endpoint is AWS, which is a complete answer rather than a missing one.
        SyncProviderForm::S3 => {
            if config.bucket.trim().is_empty() {
                return Some("Bucket");
            }
            if config.access_key.trim().is_empty() || config.secret_key.trim().is_empty() {
                return Some("Access Key / Secret Key");
            }
            return None;
        }
    };
    if address.trim().is_empty() {
        return Some(form.endpoint_label());
    }
    if config.username.trim().is_empty() || config.password.trim().is_empty() {
        return Some("用户名 / 密码");
    }
    None
}

/// What the update card's badge says.
///
/// `checking` and `loaded` come before the state because both describe the card's own knowledge,
/// which is what the user is asking about when the badge is the only thing on screen: 已是最新
/// printed over a card that has not read anything yet is a claim the app cannot support.
pub fn update_badge_label(
    state: api::update::AppUpdateState,
    checking: bool,
    loaded: bool,
    snoozed: bool,
) -> &'static str {
    use api::update::AppUpdateState as S;
    if checking {
        "检查中"
    } else if !loaded {
        "读取中"
    } else if snoozed {
        // Otherwise the header would read 有更新 over a card that says 已忽略.
        "已忽略"
    } else {
        match state {
            S::Available => "有更新",
            S::UpToDate => "已是最新",
            S::Unsupported => "不支持",
            S::Failed => "检查失败",
            S::Idle => "尚未检查",
        }
    }
}

/// The status to show: the backend's answer if there is one, else a failure if the last attempt
/// failed, else "not asked yet".
///
/// The three are genuinely different and the card renders them differently, which is why the
/// store keeps the error apart from the status rather than storing one `AppUpdateState::Failed`.
pub fn update_display_state(
    answer: Option<&api::update::AppUpdateStatus>,
    failure: Option<&str>,
) -> api::update::AppUpdateState {
    match answer {
        Some(answer) => answer.state,
        None if failure.is_some() => api::update::AppUpdateState::Failed,
        None => api::update::AppUpdateState::Idle,
    }
}

/// Elapsed time in the largest unit that still reads as "recent".
pub fn relative_age(seconds_ago: u64) -> String {
    if seconds_ago == 0 {
        return "刚刚".to_owned();
    }
    const MINUTE: u64 = 60;
    const HOUR: u64 = MINUTE * 60;
    const DAY: u64 = HOUR * 24;
    if seconds_ago < MINUTE {
        format!("{seconds_ago} 秒前")
    } else if seconds_ago < HOUR {
        format!("{} 分钟前", seconds_ago / MINUTE)
    } else if seconds_ago < DAY {
        format!("{} 小时前", seconds_ago / HOUR)
    } else {
        format!("{} 天前", seconds_ago / DAY)
    }
}

/// Age of an event stamped `then`, measured against `now`.
///
/// `None` while the answering clock is unknown. Both numbers come from the backend: the machine
/// that contacted the release feed is the only one whose clock means anything for a timestamp it
/// wrote, and a client with no usable clock would otherwise render "从未检查" forever. Zero for
/// `then` is "never", not the epoch.
pub fn age_label(then: u64, now: u64) -> Option<String> {
    if now == 0 {
        return None;
    }
    Some(if then == 0 {
        "从未检查".to_owned()
    } else {
        relative_age(now.saturating_sub(then))
    })
}

/// The auto-check row's subtitle: how often it runs, and when it last did.
pub fn auto_check_label(auto_check: bool, last_check_at: u64, server_time: u64) -> String {
    if !auto_check {
        return "仅手动检查，启动时不再联网".to_owned();
    }
    match age_label(last_check_at, server_time) {
        Some(age) => format!("每天一次 · {age}"),
        None => "每天一次 · 启动时检查".to_owned(),
    }
}

/// The release line: the title, then the version and age — unless the title already *is* the
/// version.
///
/// A release can be named anything and this project names them exactly like their tags, so
/// printing both lines reads "v1.0.9 / v1.0.9 · 13 天前". The version earns its own place only
/// when it adds something.
pub fn release_subtitle(title: &str, version: &str, age: String) -> String {
    let trimmed = title.trim().trim_start_matches(['v', 'V']);
    if trimmed.eq_ignore_ascii_case(version.trim()) {
        age
    } else {
        format!("v{version} · {age}")
    }
}

/// Whether a status is an update the user has not already declined.
pub fn update_is_actionable(answer: Option<&api::update::AppUpdateStatus>) -> bool {
    answer.is_some_and(|answer| {
        answer.state == api::update::AppUpdateState::Available && !answer.snoozed
    })
}

/// Whether a check should be announced, and the version to name in the notice.
///
/// A launch check stays quiet unless there is something to act on — the card reading 已是最新 is
/// enough, and a notice for it would be noise on every start. And one release is announced once:
/// the daily re-check finds the same release every time it runs, so without this the same notice
/// would arrive again on the next launch of the same week.
pub fn update_announcement(
    announced: Option<&str>,
    answer: Option<&api::update::AppUpdateStatus>,
) -> Option<String> {
    if !update_is_actionable(answer) {
        return None;
    }
    let version = answer?.latest.as_ref()?.version.clone();
    (announced != Some(version.as_str())).then_some(version)
}

/// What a page's floating action looks like, given whether its work is live.
///
/// The original app draws its power button twice: an inset surface with a short shadow while
/// nothing runs, the accent fill with a glow and a halo once it does. That is worth copying for
/// a reason beyond taste — a disc that is orange either way spends the one colour in the app
/// that could answer "is it running?", and answers nothing.
///
/// The rule lives here rather than in the view so both halves can be tested: a real lit state
/// needs a running core, which no headless test starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscSkin {
    /// Neutral fill, one short shadow to sit the disc on the page.
    Idle,
    /// Accent fill, its glow, and the 6pt halo that reads as "lit".
    Live,
}

impl DiscSkin {
    /// The page's state decides the skin: 总览 asks whether the core is up, 节点 whether a
    /// sweep is in flight, 日志 whether the feed is flowing.
    pub fn for_state(live: bool) -> Self {
        if live {
            DiscSkin::Live
        } else {
            DiscSkin::Idle
        }
    }

    /// The theme role the fill comes from. Kept a role name so no color literal appears here.
    pub fn fill_role(self) -> &'static str {
        match self {
            DiscSkin::Live => "primary",
            DiscSkin::Idle => "muted",
        }
    }

    /// The role the glyph takes, chosen so both states clear AA against their own fill.
    pub fn glyph_role(self) -> &'static str {
        match self {
            DiscSkin::Live => "primary-foreground",
            DiscSkin::Idle => "text-soft",
        }
    }

    /// How many shadow layers the disc carries: the lit one has the glow and the halo, the idle
    /// one only its own lift.
    pub fn shadow_layers(self) -> usize {
        match self {
            DiscSkin::Live => 2,
            DiscSkin::Idle => 1,
        }
    }
}

#[cfg(test)]
mod disc_skin_tests {
    use super::DiscSkin;

    /// The whole point of the two skins is that they differ, in fill and in weight both.
    #[test]
    fn a_live_disc_is_painted_differently_from_an_idle_one() {
        assert_ne!(DiscSkin::Idle.fill_role(), DiscSkin::Live.fill_role());
        assert_ne!(DiscSkin::Idle.glyph_role(), DiscSkin::Live.glyph_role());
        assert_eq!(DiscSkin::Idle.shadow_layers(), 1);
        assert_eq!(DiscSkin::Live.shadow_layers(), 2);
    }

    #[test]
    fn the_state_maps_to_the_skin_in_one_direction_only() {
        assert_eq!(DiscSkin::for_state(false), DiscSkin::Idle);
        assert_eq!(DiscSkin::for_state(true), DiscSkin::Live);
    }

    /// The accent carries the live state and nothing else does: an idle disc must not be able
    /// to claim a running core by borrowing its colour.
    #[test]
    fn only_the_live_skin_wears_the_accent() {
        assert_eq!(DiscSkin::Live.fill_role(), "primary");
        assert_ne!(DiscSkin::Idle.fill_role(), "primary");
    }
}

/// The size of a switch, and the distance its thumb travels.
///
/// `packages/ui` states these once (`.switch` / `.switch::after` in `app.css`) and every
/// toggle reuses them; the NSH control has no look of its own, so the same numbers are read
/// from here instead of being repeated at each of the ten call sites — where a changed `inset`
/// would silently stop matching `travel`, which is exactly the bug this struct exists to
/// prevent. It was a real one: the thumb never moved, so every switch on screen looked on and
/// off at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchGeometry {
    /// The switch's own height: the track plus a point of press area.
    pub box_height: f32,
    /// The switch's width. The track fills it, so this is the layout the rows reserve.
    pub box_width: f32,
    pub track_height: f32,
    pub track_width: f32,
    pub track_radius: f32,
    /// The gap between the track's border and the thumb, on either side.
    pub inset: f32,
    /// The thumb's side. It is a square, and round.
    pub thumb: f32,
}

impl Default for SwitchGeometry {
    fn default() -> Self {
        Self {
            box_height: 23.0,
            box_width: 40.0,
            track_height: 23.0,
            track_width: 40.0,
            track_radius: 12.0,
            inset: 3.0,
            thumb: 16.0,
        }
    }
}

impl SwitchGeometry {
    /// How far the thumb slides when the switch is turned on: the track's inside width, minus
    /// the thumb, minus both insets. Derived rather than written down, because the number that
    /// looks right is the one that leaves the thumb exactly `inset` from the far end — 16pt on
    /// the original's 40×23 track with a 16pt thumb and a 1pt border.
    pub fn travel(self) -> f32 {
        let inside = (self.track_width - 2.0).max(0.0);
        (inside - self.thumb - 2.0 * self.inset).max(0.0)
    }

    /// Whether the thumb still sits inside the track in both states: the slide plus the thumb
    /// plus the far inset must not exceed the track's inside width, and the thumb must be
    /// shorter than the track's inside height, which is where `items_center` puts it. A track
    /// whose numbers are edited into an impossible shape is a layout bug worth failing a test
    /// over, since it draws as a thumb clipped at the end of the track.
    pub fn thumb_fits(self) -> bool {
        self.travel() + self.thumb + self.inset <= (self.track_width - 2.0).max(0.0)
            && self.thumb <= (self.track_height - 2.0).max(0.0)
    }
}

#[cfg(test)]
mod switch_geometry_tests {
    use super::SwitchGeometry;

    #[test]
    fn the_default_switch_matches_the_originals_measurements() {
        // `.switch` is 40x23 with a 12px radius; `.switch::after` is 16px, 3px from the
        // border, and moves 16px. These are the values `app.css` carries, so a change here
        // is a deliberate redesign rather than a slip.
        let geo = SwitchGeometry::default();
        assert_eq!((geo.track_width, geo.track_height), (40.0, 23.0));
        assert_eq!((geo.thumb, geo.inset, geo.track_radius), (16.0, 3.0, 12.0));
        assert_eq!(geo.travel(), 16.0);
    }

    #[test]
    fn travel_is_the_room_left_between_the_two_insets() {
        // A wider track buys the thumb a longer slide, point for point.
        let wide = SwitchGeometry {
            track_width: 52.0,
            ..SwitchGeometry::default()
        };
        assert_eq!(wide.travel(), 28.0);
        // A bigger thumb takes the same room back.
        let big = SwitchGeometry {
            track_width: 52.0,
            thumb: 20.0,
            ..SwitchGeometry::default()
        };
        assert_eq!(big.travel(), 24.0);
        // The thumb starts `inset` from the near end, so the checked far edge sits `inset`
        // from the far end — the same gap on both sides, which is what makes it look centred.
        assert_eq!(
            big.inset + big.travel() + big.thumb,
            big.track_width - 2.0 - big.inset
        );
    }

    #[test]
    fn a_track_too_small_for_its_thumb_never_yields_a_negative_slide() {
        let geo = SwitchGeometry {
            track_width: 20.0,
            thumb: 24.0,
            ..SwitchGeometry::default()
        };
        assert_eq!(geo.travel(), 0.0);
        assert!(!geo.thumb_fits());
    }

    #[test]
    fn every_state_pair_of_the_default_geometry_is_drawable() {
        assert!(SwitchGeometry::default().thumb_fits());
    }
}

#[cfg(test)]
mod group_tests {

    use super::*;

    fn node(tag: &str, name: &str) -> ProxyNode {
        ProxyNode {
            tag: tag.to_owned(),
            name: name.to_owned(),
            protocol: ProxyProtocol::Vless,
            server: "edge.example.test".to_owned(),
            port: 443,
            auth: proxy_core::ProxyAuth::Uuid {
                uuid: "11111111-1111-1111-1111-111111111111".to_owned(),
                alter_id: None,
                flow: None,
            },
            transport: proxy_core::TransportOptions::default(),
            tls: proxy_core::TlsOptions::default(),
            hysteria2: None,
        }
    }

    fn group(tag: &str, kind: ProxyGroupKind, outbounds: &[&str], selected: &str) -> ProxyGroup {
        ProxyGroup {
            tag: tag.to_owned(),
            kind,
            outbounds: outbounds.iter().map(|tag| (*tag).to_owned()).collect(),
            selected: selected.to_owned(),
        }
    }

    fn latency(tag: &str, latency_ms: Option<u64>) -> api::NodeLatency {
        api::NodeLatency {
            tag: tag.to_owned(),
            latency_ms,
            error: latency_ms
                .map(|_| None)
                .unwrap_or(Some("timeout".to_owned())),
        }
    }

    /// The bug this closes: `selected_tag` is what the list shows and what the config's
    /// `default` is built from, but `apply_proxy_group_selections` then overwrites that default
    /// with `group_selections["proxy"]`. A pick that moved only the first left the core dialing
    /// the node the user had just replaced.
    #[test]
    fn a_pick_moves_the_remembered_proxy_group_choice_with_it() {
        let mut selections = HashMap::new();
        sync_proxy_group_selection(&mut selections, "subscription-1-a");
        assert_eq!(
            selections.get("proxy").map(String::as_str),
            Some("subscription-1-a")
        );

        sync_proxy_group_selection(&mut selections, "subscription-1-b");
        assert_eq!(
            selections.get("proxy").map(String::as_str),
            Some("subscription-1-b"),
            "a second pick left the first choice remembered, which the engine applies last"
        );
    }

    /// With nothing selected there is no choice to remember, and leaving an empty string would
    /// make `apply_proxy_group_selections` hunt for a member named "" (it rejects it, but the
    /// entry would survive a save and reappear in the profile forever).
    #[test]
    fn clearing_the_selection_clears_the_group_entry() {
        let mut selections = HashMap::from([("proxy".to_owned(), "a".to_owned())]);
        sync_proxy_group_selection(&mut selections, "");
        assert!(!selections.contains_key("proxy"));
    }

    /// The `proxy` entry is derived from `selected_tag`, so a hand-written map must not be able
    /// to keep a choice the node list no longer offers while the list shows something else.
    #[test]
    fn other_groups_keep_their_own_choice() {
        let mut selections = HashMap::from([
            ("proxy".to_owned(), "a".to_owned()),
            ("auto".to_owned(), "b".to_owned()),
        ]);
        sync_proxy_group_selection(&mut selections, "c");
        assert_eq!(selections.get("proxy").map(String::as_str), Some("c"));
        assert_eq!(selections.get("auto").map(String::as_str), Some("b"));
    }

    #[test]
    fn a_remembered_choice_only_counts_while_it_names_a_member() {
        let group = group("proxy", ProxyGroupKind::Selector, &["a", "b"], "a");
        let live = HashMap::from([("proxy".to_owned(), "b".to_owned())]);
        assert_eq!(group_selected(&group, &live), "b");

        // A choice from before a refresh, naming a node that is gone: the config's own default
        // is the honest answer, and it is also what the core is actually using.
        let stale = HashMap::from([("proxy".to_owned(), "gone".to_owned())]);
        assert_eq!(group_selected(&group, &stale), "a");
        assert_eq!(group_selected(&group, &HashMap::new()), "a");
    }

    #[test]
    fn members_keep_their_kind_and_resolve_to_nodes() {
        let nodes = vec![node("a", "东京 01"), node("b", "大阪 02")];
        let groups = vec![
            group(
                "proxy",
                ProxyGroupKind::Selector,
                &["a", "b", "direct"],
                "a",
            ),
            group("fast", ProxyGroupKind::UrlTest, &["a"], "a"),
        ];
        let members = visible_proxy_groups(&groups, &nodes, &HashMap::new(), &HashMap::new(), "")
            .into_iter()
            .find(|(group, _, _)| group.tag == "proxy")
            .expect("the proxy group is listed")
            .1;
        assert_eq!(members.len(), 3);
        assert_eq!(
            members[0].node.as_ref().map(|node| node.name.as_str()),
            Some("东京 01")
        );
        // A nested group is a member without a node: it has a kind, and no address to show.
        assert_eq!(members[2].tag, "direct");
        assert_eq!(members[2].node, None);
        assert_eq!(members[2].nested_kind, None);
        assert_eq!(
            members
                .iter()
                .find(|member| member.tag == "fast")
                .map(|member| member.nested_kind),
            None,
            "`fast` is not a member of `proxy`, so it is not listed as one"
        );
        // The urltest group lists only its own member, and the search text finds a node by the
        // name its provider gave it rather than by the namespaced tag.
        let named = visible_proxy_groups(&groups, &nodes, &HashMap::new(), &HashMap::new(), "大阪")
            .into_iter()
            .find(|(group, _, _)| group.tag == "proxy")
            .expect("a match keeps the group");
        assert_eq!(named.1.len(), 1);
        assert_eq!(named.1[0].tag, "b");
    }

    #[test]
    fn a_group_whose_members_are_all_filtered_out_is_not_listed() {
        let nodes = vec![node("a", "东京 01")];
        let groups = vec![group("proxy", ProxyGroupKind::Selector, &["a"], "a")];
        assert!(
            visible_proxy_groups(&groups, &nodes, &HashMap::new(), &HashMap::new(), "nope")
                .is_empty()
        );
    }

    #[test]
    fn measured_nodes_outrank_unmeasured_and_failures() {
        let nodes = vec![
            node("a", "a"),
            node("b", "b"),
            node("c", "c"),
            node("d", "d"),
        ];
        let group = group(
            "proxy",
            ProxyGroupKind::Selector,
            &["a", "b", "c", "d"],
            "a",
        );
        let latency = HashMap::from([
            ("a".to_owned(), latency("a", Some(300))),
            ("b".to_owned(), latency("b", Some(40))),
            ("c".to_owned(), latency("c", None)),
        ]);
        let groups = [group.clone()];
        let mut members = group_members(&group, &groups, &nodes, "");
        sort_group_members(&mut members, &latency);
        assert_eq!(
            members.iter().map(|m| m.tag.as_str()).collect::<Vec<_>>(),
            vec!["b", "a", "d", "c"],
            "fastest first, then unmeasured, then the one that failed"
        );
    }

    #[test]
    fn an_automatic_group_is_not_offered_as_a_choice() {
        assert!(group_is_selectable(ProxyGroupKind::Selector));
        assert!(!group_is_selectable(ProxyGroupKind::UrlTest));
    }

    #[test]
    fn engine_outbounds_are_named_not_dumped() {
        let nodes = vec![node("a", "东京 01")];
        assert_eq!(outbound_display_name("a", &nodes), "东京 01");
        assert_eq!(outbound_display_name("direct", &nodes), "直连");
        assert_eq!(outbound_display_name("block", &nodes), "拦截");
        // A nested group has no friendlier name than its tag, which is what the provider chose.
        assert_eq!(outbound_display_name("HK", &nodes), "HK");
    }

    /// The coupling `sync_proxy_group_selection` exists for, shown against the engine rather
    /// than asserted about it. `api`'s config builder runs two steps in this order: it asks
    /// `build_singbox_config` for a config, whose `proxy` selector defaults to `selected_tag`,
    /// and then runs `apply_proxy_group_selections`, which overwrites that default with
    /// `group_selections["proxy"]`. A pick that moved only the first produced a config pointing
    /// back at the node it had just replaced — and the second step wins, so the stale field is
    /// the one the core obeys. If that order ever changes, the first assertion fails and says so.
    #[test]
    fn a_stale_group_choice_overrides_the_selection_the_engine_builds() {
        let nodes = vec![node("a", "东京 01"), node("b", "大阪 02")];
        let request = proxy_core::ConnectionRequest {
            nodes: nodes.clone(),
            selected_tag: "b".to_owned(),
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: false,
            allow_lan: false,
            custom_rules: Vec::new(),
            dns_overrides: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };
        let options = proxy_core::SingBoxOptions::default();
        let proxy_default = |selections: HashMap<String, String>| {
            let mut request = request.clone();
            request.group_selections = selections;
            let mut config = proxy_core::build_singbox_config(&request, &options);
            proxy_core::apply_proxy_group_selections(&mut config, &request.group_selections);
            proxy_core::extract_proxy_groups(&config)
                .into_iter()
                .find(|group| group.tag == "proxy")
                .expect("a node list always yields the built-in proxy group")
                .selected
        };

        // Both fields on the new node, which is what the app writes: the answer is the choice.
        let mut agreed = HashMap::new();
        sync_proxy_group_selection(&mut agreed, "b");
        assert_eq!(proxy_default(agreed), "b");

        // Only `selected_tag` moved, the shape a pick left behind before this was fixed. The
        // remembered entry wins, so the core would dial the node the user had just replaced.
        assert_eq!(
            proxy_default(HashMap::from([("proxy".to_owned(), "a".to_owned())])),
            "a",
            "the engine stopped letting the remembered group choice override the selection, so \
             `sync_proxy_group_selection` is no longer needed and this test should say so"
        );
    }

    /// The wording is what a Chinese-reading user sees, and the internal `selector`/`urltest`
    /// must never appear: the original calls this out as F17.
    #[test]
    fn group_kinds_are_labelled_without_engine_jargon() {
        assert_eq!(proxy_group_kind_label(ProxyGroupKind::Selector), "手动选择");
        assert_eq!(proxy_group_kind_label(ProxyGroupKind::UrlTest), "自动测速");
    }

    /// The search matches the protocol word as well as the name, which is how "ss" finds a
    /// Shadowsocks node whose display name says nothing about it.
    #[test]
    fn the_search_reads_the_protocol_as_well_as_the_name() {
        let mut ss = node("subscription-1-ss", "隐藏节点");
        ss.protocol = ProxyProtocol::Shadowsocks;
        let groups = vec![group(
            "proxy",
            ProxyGroupKind::Selector,
            &["subscription-1-ss"],
            "subscription-1-ss",
        )];
        let hits = visible_proxy_groups(&groups, &[ss], &HashMap::new(), &HashMap::new(), "shadow");
        assert_eq!(hits.len(), 1, "searching the protocol must find the node");
    }
}

#[cfg(test)]
mod diagnostics_tests {
    use super::*;

    fn facts() -> DiagnosticFacts {
        DiagnosticFacts {
            app_version: "0.1.0".to_owned(),
            platform: "Desktop".to_owned(),
            core_state: "running".to_owned(),
            core_version: Some("1.13.14".to_owned()),
            tunnel_mode: TunnelMode::Rule,
            tun_enabled: true,
            allow_lan: false,
            node_count: 12,
            subscription_count: 2,
            rule_count: 3,
            dns_count: 1,
        }
    }

    #[test]
    fn the_report_names_every_state_it_can_be_asked_about() {
        let report = diagnostic_report(&facts());
        assert_eq!(
            report,
            "Kitty Pro 诊断信息\n\
             界面平台: Desktop\n\
             内核状态: 内核运行中 (running)\n\
             内核版本: 1.13.14\n\
             分流模式: 规则模式\n\
             TUN: 开启 · 局域网: 关闭 (本地代理 7890)\n\
             节点: 12 · 订阅: 2\n\
             自定义规则: 3 · 专用 DNS: 1"
        );
    }

    /// An unknown version is the common case before the core has answered, and the one place
    /// the report must not print `None`.
    #[test]
    fn an_unread_core_version_is_said_rather_than_left_blank() {
        let mut quiet = facts();
        quiet.core_version = None;
        quiet.core_state = "unavailable".to_owned();
        let report = diagnostic_report(&quiet);
        assert!(report.contains("内核版本: 未检测到版本"), "{report}");
        assert!(report.contains("内核不可用 (unavailable)"), "{report}");
        assert!(
            !report.contains("None"),
            "the report is pasted into a bug report; a Rust `None` there is a lie about the \
             app's own words"
        );
    }

    #[test]
    fn the_build_line_falls_back_when_the_core_is_not_ready() {
        assert_eq!(
            about_build_label("0.1.0", Some("1.13.14"), "Desktop"),
            "v0.1.0 · sing-box 1.13.14 · Desktop"
        );
        assert_eq!(
            about_build_label("0.1.0", None, "Mobile"),
            "v0.1.0 · sing-box 未就绪 · Mobile"
        );
    }

    /// LAN sharing changes both the switch's words and the port the report names, so the two
    /// cannot drift apart and blame the wrong listener.
    #[test]
    fn the_report_names_the_endpoint_the_switch_implies() {
        let mut lan = facts();
        lan.allow_lan = true;
        assert!(diagnostic_report(&lan).contains("局域网: 开启 (局域网代理 7890)"));
        assert!(diagnostic_report(&facts()).contains("局域网: 关闭 (本地代理 7890)"));
    }

    #[test]
    fn a_card_answers_to_its_own_words_and_to_english() {
        assert!(SettingsSection::Scope.matches("tun"));
        assert!(SettingsSection::Scope.matches("TUN".to_lowercase().as_str()));
        assert!(SettingsSection::About.matches("诊断"));
        assert!(SettingsSection::About.matches("dns"));
        assert!(!SettingsSection::About.matches("tun"));
        assert!(!SettingsSection::Appearance.matches("诊断"));
    }

    /// An empty query is not a filter: it has to leave the page as it was.
    #[test]
    fn an_empty_query_keeps_the_whole_page() {
        assert_eq!(visible_settings_sections("").len(), 9);
        assert_eq!(visible_settings_sections("   ").len(), 9);
    }

    /// The query is matched inside the card's words, never the other way round: typing a whole
    /// sentence must find nothing rather than whatever its first word names.
    #[test]
    fn a_longer_query_narrows_instead_of_loosening() {
        assert_eq!(
            visible_settings_sections("诊断"),
            vec![SettingsSection::About]
        );
        assert!(visible_settings_sections("诊断 内核 版本 全部").is_empty());
    }

    #[test]
    fn a_query_is_compared_trimmed_and_lowercased() {
        assert_eq!(search_keyword("  TUN "), "tun");
        assert!(SettingsSection::Scope.matches(&search_keyword("  TUN ")));
    }

    /// Everything the search cannot find empties the page, which is the state the page has to
    /// announce rather than leave blank.
    #[test]
    fn a_word_no_card_claims_shows_nothing() {
        assert!(visible_settings_sections("打印机").is_empty());
    }

    fn process(name: &str, path: Option<&str>) -> api::RunningProcess {
        api::RunningProcess {
            pid: 42,
            name: name.to_owned(),
            exe_path: path.map(str::to_owned),
        }
    }

    #[test]
    fn a_name_rule_takes_the_name_and_a_path_rule_the_path() {
        let telegram = process(
            "Telegram",
            Some("/Applications/Telegram.app/Contents/MacOS/Telegram"),
        );
        assert_eq!(
            process_rule_value(&telegram, CustomRuleMatch::ProcessName),
            "Telegram"
        );
        assert_eq!(
            process_rule_value(&telegram, CustomRuleMatch::ProcessPath),
            "/Applications/Telegram.app/Contents/MacOS/Telegram"
        );
        // A domain rule is not this picker's business, but it must not come back empty either.
        assert_eq!(
            process_rule_value(&telegram, CustomRuleMatch::Domain),
            "Telegram"
        );
    }

    /// Where the OS reports no path, a path rule still gets the only name there is: an empty
    /// field would be a worse answer than a partial one, and the user can see what landed.
    #[test]
    fn a_process_without_a_reported_path_falls_back_to_its_name() {
        let bare = process("kernel_task", None);
        assert_eq!(
            process_rule_value(&bare, CustomRuleMatch::ProcessPath),
            "kernel_task"
        );
        let blank = process("loginwindow", Some("   "));
        assert_eq!(
            process_rule_value(&blank, CustomRuleMatch::ProcessPath),
            "loginwindow"
        );
    }

    #[test]
    fn a_process_is_found_by_name_or_by_path() {
        let telegram = process(
            "Telegram",
            Some("/Applications/Telegram.app/Contents/MacOS/Telegram"),
        );
        assert!(process_matches(&telegram, "tele"));
        assert!(process_matches(&telegram, "macos"));
        assert!(!process_matches(&telegram, "safari"));
        assert!(process_matches(&telegram, ""));
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use api::SyncConfig;

    fn webdav() -> SyncConfig {
        SyncConfig {
            enabled: true,
            provider: api::SyncProviderKind::WebDav,
            webdav_endpoint: "https://dav.example.test/dav".to_owned(),
            path: "kitty-pro-sync.json".to_owned(),
            username: "hezz".to_owned(),
            password: "app-pass".to_owned(),
            ..SyncConfig::default()
        }
    }

    fn s3() -> SyncConfig {
        SyncConfig {
            enabled: true,
            provider: api::SyncProviderKind::S3,
            bucket: "kitty".to_owned(),
            access_key: "AKIA".to_owned(),
            secret_key: "shh".to_owned(),
            path: "backups/kitty.json".to_owned(),
            ..SyncConfig::default()
        }
    }

    /// A blank path would write to the storage root and a blank address cannot be dialled at
    /// all; both are knowable before the request, so the button parks and names the field.
    #[test]
    fn an_incomplete_target_names_the_field_that_is_missing() {
        let form = SyncProviderForm::WebDav;
        assert_eq!(sync_target_incomplete(form, &webdav()), None);

        let mut no_path = webdav();
        no_path.path = "  ".to_owned();
        assert_eq!(sync_target_incomplete(form, &no_path), Some("远程路径"));

        let mut no_address = webdav();
        no_address.webdav_endpoint = String::new();
        assert_eq!(
            sync_target_incomplete(form, &no_address),
            Some("WebDAV 地址")
        );

        let mut no_password = webdav();
        no_password.password = String::new();
        assert_eq!(
            sync_target_incomplete(form, &no_password),
            Some("用户名 / 密码")
        );
    }

    /// S3's blank endpoint is a real answer — it means AWS — so it must not be reported as
    /// missing the way a blank WebDAV address is.
    #[test]
    fn s3_treats_a_blank_endpoint_as_aws() {
        assert_eq!(sync_target_incomplete(SyncProviderForm::S3, &s3()), None);

        let mut no_keys = s3();
        no_keys.secret_key = String::new();
        assert_eq!(
            sync_target_incomplete(SyncProviderForm::S3, &no_keys),
            Some("Access Key / Secret Key")
        );
    }

    /// Switching providers shows a different field set, and each set writes a different stored
    /// field: this is the pairing that must never invert.
    #[test]
    fn each_provider_offers_only_its_own_fields() {
        let webdav_fields: Vec<SyncField> = SyncProviderForm::WebDav
            .fields()
            .into_iter()
            .map(|spec| spec.field)
            .collect();
        assert_eq!(
            webdav_fields,
            vec![SyncField::Username, SyncField::Password]
        );

        let s3_fields: Vec<SyncField> = SyncProviderForm::S3
            .fields()
            .into_iter()
            .map(|spec| spec.field)
            .collect();
        assert_eq!(
            s3_fields,
            vec![
                SyncField::Bucket,
                SyncField::Region,
                SyncField::AccessKey,
                SyncField::SecretKey
            ]
        );
    }

    /// The secret fields are the ones that hide what is typed. Marking an address as secret would
    /// make it unreadable while typing; marking a token as plain is a leak.
    #[test]
    fn only_the_two_secrets_are_masked() {
        let masked: Vec<SyncField> = [SyncProviderForm::WebDav, SyncProviderForm::S3]
            .into_iter()
            .flat_map(|form| form.fields())
            .filter(|spec| spec.secret)
            .map(|spec| spec.field)
            .collect();
        assert_eq!(masked, vec![SyncField::Password, SyncField::SecretKey]);
    }

    /// Read and write have to agree on which slot each variant owns, or a typed value lands in a
    /// different field than the one it was typed into.
    #[test]
    fn every_field_round_trips_through_its_own_slot() {
        let all = [
            SyncField::WebDavEndpoint,
            SyncField::S3Endpoint,
            SyncField::Path,
            SyncField::Username,
            SyncField::Password,
            SyncField::Bucket,
            SyncField::Region,
            SyncField::AccessKey,
            SyncField::SecretKey,
        ];
        for field in all {
            let mut config = SyncConfig::default();
            field.write(&mut config, "written");
            assert_eq!(field.read(&config), "written", "{field:?} read elsewhere");
            for other in all {
                let holds_what_was_written = other.read(&config) == "written";
                assert_eq!(
                    holds_what_was_written,
                    other == field,
                    "{field:?} wrote into {other:?}"
                );
            }
        }
    }

    #[test]
    fn a_field_write_leaves_the_others_alone() {
        let mut config = webdav();
        SyncField::Region.write(&mut config, "eu-west-1");
        assert_eq!(config.region, "eu-west-1");
        assert_eq!(config.username, "hezz");
        assert_eq!(config.password, "app-pass");
        assert_eq!(config.webdav_endpoint, "https://dav.example.test/dav");
    }

    /// Synced, pending and off are three different answers to "is my other device current", and
    /// reading an off card as 已同步 is the one that sends the user away wrong.
    #[test]
    fn the_status_distinguishes_off_from_never_run_from_synced() {
        assert_eq!(SyncStatus::Synced(1_700_000_000).label(), "已同步");
        assert_eq!(SyncStatus::Pending.label(), "待同步");
        assert_eq!(SyncStatus::Disabled.label(), "未启用");
        assert_eq!(SyncStatus::Loading.label(), "读取中");
        assert_ne!(SyncStatus::Pending.tint(), SyncStatus::Disabled.tint());
    }

    #[test]
    fn the_age_is_said_in_the_unit_the_user_thinks_in() {
        let now = 1_700_000_000;
        assert_eq!(
            SyncStatus::Synced(now - 10).since_label(now).as_deref(),
            Some("刚刚")
        );
        assert_eq!(
            SyncStatus::Synced(now - 7_200).since_label(now).as_deref(),
            Some("2 小时前")
        );
        assert_eq!(
            SyncStatus::Synced(now - 3 * 86_400)
                .since_label(now)
                .as_deref(),
            Some("3 天前")
        );
        // A clock that went backwards (an NTP correction) must not print a huge age.
        assert_eq!(
            SyncStatus::Synced(now + 500).since_label(now).as_deref(),
            Some("刚刚")
        );
        assert_eq!(SyncStatus::Pending.since_label(now), None);
    }

    #[test]
    fn the_pull_summary_counts_both_sides() {
        let remote = Some(SyncCounts {
            remote_subscriptions: 4,
            remote_rules: 9,
        });
        assert_eq!(
            pull_confirm_summary(remote, 2, 3),
            "将用远端 4 个订阅 / 9 条规则替换本机 2 个订阅 / 3 条规则"
        );
    }

    /// The remote has not been read yet, so its size is unknown. Saying "0 个订阅" here would
    /// tell the user the remote is empty seconds before overwriting their work with it.
    #[test]
    fn an_unread_remote_says_unknown_rather_than_zero() {
        let summary = pull_confirm_summary(None, 2, 3);
        assert!(summary.contains("需下载后确认"), "{summary}");
        assert!(!summary.contains("远端 0"), "{summary}");
    }

    /// A remote document from before the DNS section shipped carries no DNS list at all, which is
    /// not the same fact as "the remote has none": the first keeps the local list, the second
    /// empties it.
    #[test]
    fn a_remote_without_dns_says_so_rather_than_reporting_zero() {
        assert_eq!(
            sync_result_clause(3, 5, Some(0)),
            "已同步 3 个订阅、5 条分流规则、0 条专用 DNS"
        );
        assert!(sync_result_clause(3, 5, None).contains("远端未含专用 DNS（保留本机）"));
    }

    /// The backend refuses a stale push in ETag and baseline language. Nobody can act on that, so
    /// each known shape is mapped onto the move that resolves it and flagged as a conflict, which
    /// is what opens the panel with the two buttons.
    #[test]
    fn a_backend_refusal_becomes_a_move_the_user_can_take() {
        let (friendly, conflict) = classify_sync_error("远端文件已被其他设备修改");
        assert!(conflict);
        assert!(friendly.contains("下载覆盖本机"), "{friendly}");

        let (_, conflict) = classify_sync_error("If-Match failed: no ETag returned");
        assert!(conflict);

        let (_, conflict) = classify_sync_error("本机尚无同步基线");
        assert!(conflict);

        let (_, conflict) = classify_sync_error("remote revision mismatch");
        assert!(conflict);
    }

    /// Anything else is an ordinary failure and must stay one: raising a panel for a DNS
    /// resolution error would offer 覆盖远端 as the cure for a problem it cannot fix.
    #[test]
    fn an_ordinary_failure_stays_ordinary() {
        let (friendly, conflict) = classify_sync_error("连接超时");
        assert!(!conflict);
        assert_eq!(friendly, "连接超时");
    }

    /// Both providers' field sets must be reachable from the stored kind, or switching to a
    /// provider leaves its fields hidden and its values uneditable.
    #[test]
    fn the_form_follows_the_stored_provider() {
        assert_eq!(
            SyncProviderForm::from_kind(api::SyncProviderKind::S3),
            SyncProviderForm::S3
        );
        assert_eq!(
            SyncProviderForm::from_kind(api::SyncProviderKind::WebDav),
            SyncProviderForm::WebDav
        );
        assert_eq!(SyncProviderForm::S3.label(), "S3 兼容");
    }

    /// The sync card is findable, and it is the only card that answers to 云同步.
    #[test]
    fn the_sync_card_is_what_a_search_for_sync_finds() {
        assert_eq!(
            visible_settings_sections("云同步"),
            vec![SettingsSection::Sync]
        );
        assert_eq!(
            visible_settings_sections("webdav"),
            vec![SettingsSection::Sync]
        );
        assert!(SettingsSection::Sync.matches("冲突"));
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    use api::update::{AppRelease, AppUpdateState, AppUpdateStatus};

    fn status(state: AppUpdateState, snoozed: bool) -> AppUpdateStatus {
        AppUpdateStatus {
            state,
            current_version: "0.1.0".to_owned(),
            latest: Some(AppRelease {
                version: "9.9.9".to_owned(),
                title: "v9.9.9".to_owned(),
                url: "https://github.example.test/releases/9.9.9".to_owned(),
                notes: None,
                published_at: None,
                prerelease: false,
                asset: None,
                asset_size: 0,
            }),
            last_check_at: 1_700_000_000,
            snoozed,
            target: "desktop · macos · aarch64".to_owned(),
            error: None,
            server_time: 1_700_000_060,
        }
    }

    /// A check that has not answered yet and a check that answered "you are current" are
    /// different facts, and the badge must not blur them: 已是最新 printed before anything was
    /// read is the claim that keeps someone off a fix.
    #[test]
    fn an_unread_card_never_claims_the_build_is_current() {
        assert_eq!(
            update_badge_label(AppUpdateState::UpToDate, false, false, false),
            "读取中"
        );
        assert_eq!(
            update_badge_label(AppUpdateState::Idle, false, true, false),
            "尚未检查"
        );
        assert_eq!(
            update_badge_label(AppUpdateState::UpToDate, false, true, false),
            "已是最新"
        );
    }

    /// While a check is running the badge says so whatever the previous answer was, and a
    /// dismissed release stops being announced — otherwise the header reads 有更新 over a card
    /// whose button the user already pressed.
    #[test]
    fn checking_and_snoozing_outrank_the_stored_state() {
        assert_eq!(
            update_badge_label(AppUpdateState::Available, true, true, false),
            "检查中"
        );
        assert_eq!(
            update_badge_label(AppUpdateState::Available, false, true, true),
            "已忽略"
        );
        assert_eq!(
            update_badge_label(AppUpdateState::Available, false, true, false),
            "有更新"
        );
        assert_eq!(
            update_badge_label(AppUpdateState::Failed, false, true, false),
            "检查失败"
        );
    }

    /// The state shown is the backend's answer when there is one; only its absence falls through
    /// to the transport failure, and only the absence of both reads as "not asked yet".
    #[test]
    fn an_answer_outranks_a_transport_failure() {
        let answer = status(AppUpdateState::Available, false);
        assert_eq!(
            update_display_state(Some(&answer), Some("timeout")),
            AppUpdateState::Available
        );
        assert_eq!(
            update_display_state(None, Some("timeout")),
            AppUpdateState::Failed
        );
        assert_eq!(update_display_state(None, None), AppUpdateState::Idle);
    }

    /// A release the user declined is not an update to act on, and an answer with no release at
    /// all must not be treated as one either.
    #[test]
    fn only_an_undismissed_release_is_actionable() {
        assert!(update_is_actionable(Some(&status(
            AppUpdateState::Available,
            false
        ))));
        assert!(!update_is_actionable(Some(&status(
            AppUpdateState::Available,
            true
        ))));
        assert!(!update_is_actionable(Some(&status(
            AppUpdateState::UpToDate,
            false
        ))));
        assert!(!update_is_actionable(None));
    }

    #[test]
    fn an_age_uses_the_largest_unit_that_still_reads_as_recent() {
        assert_eq!(relative_age(0), "刚刚");
        assert_eq!(relative_age(45), "45 秒前");
        assert_eq!(relative_age(120), "2 分钟前");
        assert_eq!(relative_age(7_200), "2 小时前");
        assert_eq!(relative_age(172_800), "2 天前");
        // One hour reads in minutes rather than as "0 小时前".
        assert_eq!(relative_age(3_600), "1 小时前");
    }

    /// `then == 0` means "never", not the epoch; `now == 0` means the answering clock is unknown
    /// and no age can be stated at all.
    #[test]
    fn never_is_not_the_epoch() {
        assert_eq!(age_label(0, 1_700_000_000).as_deref(), Some("从未检查"));
        assert_eq!(age_label(1_700_000_000, 0), None);
        assert_eq!(age_label(0, 0), None);
        assert_eq!(
            age_label(1_700_000_000 - 120, 1_700_000_000).as_deref(),
            Some("2 分钟前")
        );
    }

    /// A clock that moved backwards must not print an absurd age.
    #[test]
    fn a_future_stamp_does_not_underflow() {
        assert_eq!(
            age_label(1_700_000_500, 1_700_000_000).as_deref(),
            Some("刚刚")
        );
    }

    /// With no clock to measure against the row still has to explain itself, rather than showing
    /// an empty subtitle under a switch the user is deciding about.
    #[test]
    fn the_auto_check_row_always_says_something() {
        assert_eq!(
            auto_check_label(false, 1_700_000_000, 1_700_000_060),
            "仅手动检查，启动时不再联网"
        );
        assert_eq!(
            auto_check_label(true, 1_700_000_000, 1_700_000_060),
            "每天一次 · 1 分钟前"
        );
        assert_eq!(auto_check_label(true, 0, 0), "每天一次 · 启动时检查");
    }

    /// Titles are whatever the publisher typed. When the title already is the version, printing
    /// both reads "v1.0.9 / v1.0.9 · 13 天前".
    #[test]
    fn the_release_line_drops_a_version_the_title_already_gave() {
        assert_eq!(
            release_subtitle("v1.0.9", "1.0.9", "13 天前".to_owned()),
            "13 天前"
        );
        assert_eq!(
            release_subtitle("V1.0.9", "1.0.9", "13 天前".to_owned()),
            "13 天前"
        );
        assert_eq!(
            release_subtitle("October build", "1.0.9", "13 天前".to_owned()),
            "v1.0.9 · 13 天前"
        );
    }

    /// A notice is for something to act on. A card that says 已是最新 needs no interruption, and
    /// a release the user already declined must not ask a second time.
    #[test]
    fn only_an_undismissed_release_is_announced() {
        let available = status(AppUpdateState::Available, false);
        assert_eq!(
            update_announcement(None, Some(&available)).as_deref(),
            Some("9.9.9")
        );
        assert_eq!(
            update_announcement(None, Some(&status(AppUpdateState::UpToDate, false))),
            None
        );
        assert_eq!(
            update_announcement(None, Some(&status(AppUpdateState::Available, true))),
            None
        );
        assert_eq!(update_announcement(None, None), None);
    }

    /// The daily re-check finds the same release every time it runs. Without this the same notice
    /// would arrive on every launch of the same week, which is how a reminder becomes the thing
    /// the user learns to dismiss without reading.
    #[test]
    fn one_release_is_announced_once() {
        let available = status(AppUpdateState::Available, false);
        assert_eq!(
            update_announcement(Some("9.9.9"), Some(&available)),
            None,
            "the announced release was announced again"
        );
        // A newer release after the one already dismissed does earn a notice.
        assert_eq!(
            update_announcement(Some("9.9.8"), Some(&available)).as_deref(),
            Some("9.9.9")
        );
    }

    /// The card answers to what a user types after an announcement, and to nothing else — 关于
    /// belongs to the about card, and two cards claiming it would both surface on one query.
    #[test]
    fn the_update_card_answers_to_update_words_only() {
        assert!(visible_settings_sections("更新").contains(&SettingsSection::Updates));
        assert_eq!(
            visible_settings_sections("github"),
            vec![SettingsSection::Updates]
        );
        assert!(SettingsSection::Updates.matches("忽略"));
        assert!(!SettingsSection::Updates.matches("webdav"));
        assert!(visible_settings_sections("关于").contains(&SettingsSection::About));
    }
}
