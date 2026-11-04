//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

pub(crate) fn protocol_abbreviation(protocol: ProxyProtocol) -> &'static str {
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

pub(crate) fn protocol_class(protocol: ProxyProtocol) -> &'static str {
    match protocol {
        ProxyProtocol::AnyTls => "protocol-mark anytls",
        ProxyProtocol::Hysteria2 => "protocol-mark hy2",
        ProxyProtocol::Vmess => "protocol-mark vmess",
        ProxyProtocol::Vless => "protocol-mark vless",
        ProxyProtocol::Trojan => "protocol-mark trojan",
        ProxyProtocol::Shadowsocks => "protocol-mark shadowsocks",
        ProxyProtocol::Http => "protocol-mark http",
        ProxyProtocol::Socks5 => "protocol-mark socks5",
    }
}

pub(crate) fn mode_label(mode: TunnelMode) -> &'static str {
    match mode {
        TunnelMode::Rule => "规则模式",
        TunnelMode::Global => "全局模式",
        TunnelMode::Direct => "直连模式",
    }
}

pub(crate) fn core_state_label(state: &str) -> &'static str {
    match state {
        "running" => "内核运行中",
        "stopped" => "内核已就绪",
        "checking" => "正在检测",
        _ => "内核不可用",
    }
}

pub(crate) fn status_dot_class(state: &str) -> &'static str {
    match state {
        "running" => "status-dot online",
        "stopped" => "status-dot ready",
        _ => "status-dot offline",
    }
}

pub(crate) fn status_badge_class(state: &str) -> &'static str {
    match state {
        "running" => "status-badge online",
        "stopped" => "status-badge ready",
        _ => "status-badge offline",
    }
}

/// Topbar chip: the design keeps a connected core and an idle-but-ready core in
/// the same amber family, and only escalates to coral when the core is gone.
pub(crate) fn connection_chip_class(connected: bool, state: &str) -> &'static str {
    match (connected, state) {
        (true, _) => "status-badge online",
        (false, "unavailable") => "status-badge offline",
        _ => "status-badge pending",
    }
}

pub(crate) fn connection_chip_label(connected: bool, state: &str) -> &'static str {
    match (connected, state) {
        (true, _) => "已连接",
        (false, "unavailable") => "内核不可用",
        (false, "checking") => "检测中",
        _ => "未连接",
    }
}

pub(crate) fn next_subscription_id(subscriptions: &[Subscription]) -> u64 {
    subscriptions
        .iter()
        .map(|subscription| subscription.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

/// Drop remembered proxy-group selections whose outbound no longer exists.
/// Switching/refreshing/deleting a subscription rewrites the namespaced node
/// tags, and the previous map was cloned whole - the stale outbound then went
/// out in every later `ConnectionRequest.group_selections` while the list
/// showed a silently different (defaulted) choice.
pub(crate) fn prune_group_selections(
    selections: &mut HashMap<String, String>,
    mut keep: impl FnMut(&str) -> bool,
) {
    selections.retain(|_, outbound| keep(outbound));
}

pub(crate) fn namespace_nodes(subscription_id: u64, mut nodes: Vec<ProxyNode>) -> Vec<ProxyNode> {
    for node in &mut nodes {
        node.tag = format!("subscription-{subscription_id}-{}", node.tag);
    }
    nodes
}

pub(crate) fn apply_subscription_report(
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
    let ParseReport {
        nodes,
        rejected,
        proxy_server_nameservers,
    } = report;
    let count = nodes.len();
    let rejected_count = rejected.len();
    let subscription = subscriptions
        .iter_mut()
        .find(|subscription| subscription.id == subscription_id)
        .ok_or_else(|| "订阅已不存在".to_string())?;
    subscription.nodes = namespace_nodes(subscription_id, nodes);
    subscription.proxy_server_nameservers = proxy_server_nameservers;
    subscription.rejected_count = rejected_count;
    Ok(count)
}

#[cfg(test)]
pub(crate) fn collect_nodes(subscriptions: &[Subscription]) -> Vec<ProxyNode> {
    subscriptions
        .iter()
        .flat_map(|subscription| subscription.nodes.iter().cloned())
        .collect()
}

pub(crate) fn resolve_active_subscription_id(
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

pub(crate) fn collect_subscription_nodes(
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

pub(crate) fn active_subscription_proxy_server_nameservers(
    subscriptions: &[Subscription],
    subscription_id: Option<u64>,
) -> Vec<String> {
    subscription_id
        .and_then(|id| {
            subscriptions
                .iter()
                .find(|subscription| subscription.id == id)
        })
        .map(|subscription| subscription.proxy_server_nameservers.clone())
        .unwrap_or_default()
}

pub(crate) fn sort_proxy_group_members(
    members: &mut [ProxyGroupMember],
    results: &HashMap<String, NodeLatency>,
) {
    members.sort_by_key(|member| {
        member
            .node
            .as_ref()
            .and_then(|node| results.get(&node.tag))
            .map(|result| match result.latency_ms {
                Some(latency) => (0, latency),
                None => (2, u64::MAX),
            })
            .unwrap_or((1, u64::MAX))
    });
}

pub(crate) fn select_available_tag(nodes: &[ProxyNode], requested_tag: &str) -> String {
    if nodes.iter().any(|node| node.tag == requested_tag) {
        requested_tag.to_string()
    } else {
        nodes
            .first()
            .map(|node| node.tag.clone())
            .unwrap_or_default()
    }
}

/// The DNS clause for a sync summary.
///
/// `None` means the remote document carried no DNS section, which is every
/// object uploaded before the feature shipped. That must not read as "0 条":
/// applying such a document *keeps* the local rules, and a user who just
/// downloaded would otherwise conclude their rules were wiped.
pub(crate) fn dns_sync_clause(dns_rule_count: Option<usize>) -> String {
    match dns_rule_count {
        Some(count) => format!("{count} 条专用 DNS"),
        None => "远端未含专用 DNS（保留本机）".to_string(),
    }
}

pub(crate) fn source_label(source: &str) -> String {
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
