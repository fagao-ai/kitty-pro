//! Copy and units shared by every screen.
//!
//! Pure functions with no engine dependency, kept identical to the Dioxus UI so a
//! translated screen shows the same text. `cargo test` exercises them without a surface.

use crate::ui::theme::Tint;
use api::{RouteDecision, RouteTargetKind};
use proxy_core::{
    CustomRuleAction, CustomRuleMatch, ParseReport, ProxyNode, ProxyProtocol, Subscription,
    TunnelMode,
};
use std::collections::HashMap;

/// The core's fixed mixed port; the label only ever changes with LAN sharing.
pub const PROXY_PORT: u16 = 7890;

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
