use crate::{
    normalize_custom_rule_value, ConnectionRequest, CustomRule, CustomRuleAction, CustomRuleMatch,
    ProxyAuth, ProxyGroup, ProxyGroupKind, ProxyNode, ProxyProtocol, TunnelMode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use url::Url;

pub const CHINA_GEOSITE_RULE_SET_URL: &str =
    "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geosite/cn.srs";
pub const CHINA_GEOIP_RULE_SET_URL: &str =
    "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geoip/cn.srs";
const SECURE_DNS_SERVER: &str = "1.1.1.1";
const SECURE_DNS_SERVER_NAME: &str = "cloudflare-dns.com";
const PROXY_ENDPOINT_DNS_SERVER: &str = "1.12.12.12";
const PROXY_ENDPOINT_DNS_SERVER_NAME: &str = "doh.pub";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSetCachePaths {
    pub geosite: String,
    pub geoip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingBoxOptions {
    pub mixed_port: u16,
    pub listen: String,
    pub log_level: String,
    /// Optional loopback-only endpoint used internally to read traffic counters.
    pub traffic_api_port: Option<u16>,
    /// Per-process authentication for the internal traffic endpoint.
    pub traffic_api_secret: Option<String>,
    /// Validated local rule-set files. Local rule sets are watched by
    /// sing-box and reloaded when the updater atomically replaces them.
    pub rule_set_cache: Option<RuleSetCachePaths>,
    /// Persistent sing-box cache used for remote rule sets and, in TUN mode,
    /// stable FakeIP-to-domain mappings across core restarts.
    pub cache_file: Option<String>,
}

impl Default for SingBoxOptions {
    fn default() -> Self {
        Self {
            mixed_port: 7890,
            listen: "127.0.0.1".to_string(),
            log_level: "info".to_string(),
            traffic_api_port: None,
            traffic_api_secret: None,
            rule_set_cache: None,
            cache_file: None,
        }
    }
}

pub fn build_singbox_config(request: &ConnectionRequest, options: &SingBoxOptions) -> Value {
    let mut inbounds = vec![json!({
        "type": "mixed",
        "tag": "mixed-in",
        "listen": options.listen,
        "listen_port": options.mixed_port,
    })];
    if request.tun {
        let mut tun_inbound = json!({
            "type": "tun",
            "tag": "tun-in",
            // Keep both address families captured. FakeIP below restores the
            // original domain before dialing, so an IPv4-only upstream does
            // not receive a browser-selected IPv6 literal.
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "auto_route": true,
            "strict_route": true,
            "stack": "system",
            // Without an explicit MTU the system stack inherits the utun
            // maximum (65535), so clients negotiate huge TCP segments and
            // throughput collapses to ~1 Mbps through the userspace forwarder
            // (WeChat image uploads crawl while small text packets are fine).
            // Match the physical link so MSS negotiation stays normal.
            "mtu": 1500,
        });
        let route_exclusions = proxy_endpoint_routes(request);
        if !route_exclusions.is_empty() {
            tun_inbound["route_exclude_address"] = json!(route_exclusions);
        }
        inbounds.push(tun_inbound);
    }

    let mut outbounds: Vec<Value> = request.nodes.iter().map(node_to_outbound).collect();
    let node_tags: Vec<&str> = request.nodes.iter().map(|node| node.tag.as_str()).collect();
    let selected = if request
        .nodes
        .iter()
        .any(|node| node.tag == request.selected_tag)
    {
        request.selected_tag.as_str()
    } else {
        node_tags.first().copied().unwrap_or("direct")
    };
    outbounds.push(json!({
        "type": "selector",
        "tag": "proxy",
        "outbounds": node_tags,
        "default": selected,
    }));
    outbounds.push(json!({ "type": "direct", "tag": "direct" }));
    outbounds.push(json!({ "type": "block", "tag": "block" }));

    let route_final = match request.mode {
        TunnelMode::Direct => "direct",
        TunnelMode::Rule | TunnelMode::Global => "proxy",
    };
    let mut rules = vec![json!({ "port": 53, "action": "hijack-dns" })];
    if request.mode == TunnelMode::Rule {
        rules.push(json!({ "action": "sniff" }));
        rules.push(json!({ "ip_is_private": true, "action": "route", "outbound": "direct" }));
        rules.extend(
            request
                .custom_rules
                .iter()
                .filter(|rule| rule.enabled)
                .filter_map(custom_rule_to_value),
        );
        rules.push(json!({ "rule_set": "geosite-cn", "action": "route", "outbound": "direct" }));
        rules.push(json!({ "rule_set": "geoip-cn", "action": "route", "outbound": "direct" }));
    }
    let rule_sets = match (request.mode, options.rule_set_cache.as_ref()) {
        (TunnelMode::Rule, Some(paths)) => vec![
            json!({
                "type": "local",
                "tag": "geosite-cn",
                "format": "binary",
                "path": paths.geosite,
            }),
            json!({
                "type": "local",
                "tag": "geoip-cn",
                "format": "binary",
                "path": paths.geoip,
            }),
        ],
        (TunnelMode::Rule, None) => vec![
            json!({
                "type": "remote",
                "tag": "geosite-cn",
                "format": "binary",
                "url": CHINA_GEOSITE_RULE_SET_URL,
                "download_detour": "direct",
                "update_interval": "168h",
            }),
            json!({
                "type": "remote",
                "tag": "geoip-cn",
                "format": "binary",
                "url": CHINA_GEOIP_RULE_SET_URL,
                "download_detour": "direct",
                "update_interval": "168h",
            }),
        ],
        (TunnelMode::Global | TunnelMode::Direct, _) => Vec::new(),
    };
    let (dns, subscription_dns) = build_dns_config(request);
    let default_domain_resolver = if request.mode == TunnelMode::Direct {
        "dns-direct"
    } else if subscription_dns {
        "dns-subscription"
    } else {
        "dns-bootstrap"
    };

    let mut config = json!({
        "log": {
            "level": options.log_level,
            "timestamp": true,
        },
        "dns": dns,
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": {
            // Mixed/system-proxy connections must follow host routes so
            // tailnets and other VPN interfaces remain reachable. Desktop
            // TUN mode still pins outbounds to avoid re-entering its own TUN.
            "auto_detect_interface": request.tun,
            // Proxy endpoints and rule-set downloads must resolve through a
            // direct encrypted resolver without trusting the system DNS.
            "default_domain_resolver": default_domain_resolver,
            "rules": rules,
            "rule_set": rule_sets,
            "final": route_final,
        },
    });
    let mut experimental = Map::new();
    if let Some(path) = options.cache_file.as_deref() {
        experimental.insert(
            "cache_file".to_string(),
            json!({
                "enabled": true,
                "path": path,
                "store_fakeip": request.tun && request.mode != TunnelMode::Direct,
            }),
        );
    }
    if let Some(port) = options.traffic_api_port {
        experimental.insert(
            "clash_api".to_string(),
            json!({
                "external_controller": format!("127.0.0.1:{port}"),
                "secret": options.traffic_api_secret,
            }),
        );
    }
    if !experimental.is_empty() {
        config["experimental"] = Value::Object(experimental);
    }
    config
}

fn build_dns_config(request: &ConnectionRequest) -> (Value, bool) {
    let mut servers = vec![json!({
        "type": "local",
        "tag": "dns-direct",
    })];
    if request.mode == TunnelMode::Direct {
        return (
            json!({
                "servers": servers,
                "final": "dns-direct",
            }),
            false,
        );
    }

    servers.push(json!({
        "type": "https",
        "tag": "dns-bootstrap",
        "server": PROXY_ENDPOINT_DNS_SERVER,
        "server_port": 443,
        "path": "/dns-query",
        "tls": {
            "enabled": true,
            "server_name": PROXY_ENDPOINT_DNS_SERVER_NAME,
        },
    }));
    let subscription_dns = request
        .proxy_server_nameservers
        .iter()
        .find_map(|server| subscription_https_dns_server(server));
    let has_subscription_dns = subscription_dns.is_some();
    if let Some(server) = subscription_dns {
        servers.push(server);
    }
    servers.push(json!({
        "type": "https",
        "tag": "dns-proxy",
        "server": SECURE_DNS_SERVER,
        "server_port": 443,
        "path": "/dns-query",
        "tls": {
            "enabled": true,
            "server_name": SECURE_DNS_SERVER_NAME,
        },
        "detour": "proxy",
    }));

    let mut rules = vec![
        json!({
            "query_type": ["PTR"],
            "action": "route",
            "server": "dns-direct",
        }),
        json!({
            "domain_suffix": ["lan", "local", "home.arpa"],
            "action": "route",
            "server": "dns-direct",
        }),
    ];
    if request.mode == TunnelMode::Rule {
        rules.extend(
            request
                .custom_rules
                .iter()
                .filter(|rule| rule.enabled && rule.action == CustomRuleAction::Direct)
                .filter_map(custom_direct_dns_rule_to_value),
        );
        // Chinese/domestic domains are answered only with IPv4. WeChat and
        // other Tencent apps prefer IPv6 for media uploads; if the host has
        // no real IPv6 connectivity the direct IPv6 dial fails (image send
        // breaks) while IPv4 text traffic keeps working.
        rules.push(json!({
            "rule_set": "geosite-cn",
            "action": "route",
            "server": "dns-direct",
            "strategy": "ipv4_only",
        }));
    }

    let use_fakeip = request.tun;
    if use_fakeip {
        servers.push(json!({
            "type": "fakeip",
            "tag": "dns-fakeip",
            "inet4_range": "198.18.0.0/15",
            "inet6_range": "fc00::/18",
        }));
        rules.push(json!({
            "query_type": ["A", "AAAA"],
            "action": "route",
            "server": "dns-fakeip",
        }));
    }

    let mut dns = json!({
        "servers": servers,
        "rules": rules,
        "final": "dns-proxy",
    });
    if use_fakeip {
        dns["independent_cache"] = json!(true);
    }
    (dns, has_subscription_dns)
}

fn subscription_https_dns_server(value: &str) -> Option<Value> {
    let url = Url::parse(value.trim()).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let server = url.host_str()?.to_string();
    let path = if url.path().is_empty() {
        "/dns-query"
    } else {
        url.path()
    };
    let mut options = json!({
        "type": "https",
        "tag": "dns-subscription",
        "server": server,
        "server_port": url.port().unwrap_or(443),
        "path": path,
        "tls": {
            "enabled": true,
            "server_name": server,
        },
    });
    if server.parse::<IpAddr>().is_err() {
        options["domain_resolver"] = json!("dns-bootstrap");
    }
    Some(options)
}

fn custom_direct_dns_rule_to_value(rule: &CustomRule) -> Option<Value> {
    if rule.match_type == CustomRuleMatch::IpCidr {
        return None;
    }
    let value = normalize_custom_rule_value(rule.match_type, &rule.value).ok()?;
    Some(json!({
        rule.match_type.singbox_field(): [value],
        "action": "route",
        "server": "dns-direct",
        // Direct egress needs the host's real connectivity, so only IPv4 is
        // handed out. Many domestic apps (WeChat media uploads) prefer IPv6
        // and fail when the host has no IPv6 route.
        "strategy": "ipv4_only",
    }))
}

fn custom_rule_to_value(rule: &CustomRule) -> Option<Value> {
    let value = normalize_custom_rule_value(rule.match_type, &rule.value).ok()?;
    let mut object = Map::new();
    object.insert(rule.match_type.singbox_field().to_string(), json!([value]));
    object.insert("action".to_string(), json!("route"));
    object.insert("outbound".to_string(), json!(rule.action.outbound()));
    Some(Value::Object(object))
}

pub fn apply_proxy_group_selections(config: &mut Value, selections: &HashMap<String, String>) {
    let Some(outbounds) = config.get_mut("outbounds").and_then(Value::as_array_mut) else {
        return;
    };
    for outbound in outbounds {
        if outbound.get("type").and_then(Value::as_str) != Some("selector") {
            continue;
        }
        let Some(tag) = outbound.get("tag").and_then(Value::as_str) else {
            continue;
        };
        let Some(selected) = selections.get(tag) else {
            continue;
        };
        let valid = outbound
            .get("outbounds")
            .and_then(Value::as_array)
            .is_some_and(|outbounds| {
                outbounds
                    .iter()
                    .any(|value| value.as_str() == Some(selected))
            });
        if valid {
            outbound["default"] = Value::String(selected.clone());
        }
    }
}

pub fn extract_proxy_groups(config: &Value) -> Vec<ProxyGroup> {
    config
        .get("outbounds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|outbound| {
            let kind = match outbound.get("type").and_then(Value::as_str)? {
                "selector" => ProxyGroupKind::Selector,
                "urltest" => ProxyGroupKind::UrlTest,
                _ => return None,
            };
            let tag = outbound.get("tag").and_then(Value::as_str)?.to_string();
            let outbounds = outbound
                .get("outbounds")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            if outbounds.is_empty() {
                return None;
            }
            let selected = outbound
                .get("default")
                .and_then(Value::as_str)
                .filter(|selected| outbounds.iter().any(|item| item == selected))
                .unwrap_or(&outbounds[0])
                .to_string();
            Some(ProxyGroup {
                tag,
                kind,
                outbounds,
                selected,
            })
        })
        .collect()
}

fn node_to_outbound(node: &ProxyNode) -> Value {
    let mut object = Map::new();
    object.insert(
        "type".to_string(),
        json!(match node.protocol {
            ProxyProtocol::AnyTls => "anytls",
            ProxyProtocol::Hysteria2 => "hysteria2",
            ProxyProtocol::Vmess => "vmess",
            ProxyProtocol::Vless => "vless",
            ProxyProtocol::Trojan => "trojan",
            ProxyProtocol::Shadowsocks => "shadowsocks",
            ProxyProtocol::Http => "http",
            ProxyProtocol::Socks5 => "socks",
        }),
    );
    object.insert("tag".to_string(), json!(node.tag));
    object.insert("server".to_string(), json!(node.server));
    object.insert("server_port".to_string(), json!(node.port));

    match &node.auth {
        ProxyAuth::None => {}
        ProxyAuth::UserPassword { username, password } => {
            object.insert("username".to_string(), json!(username));
            object.insert("password".to_string(), json!(password));
        }
        ProxyAuth::Password { password } => {
            object.insert("password".to_string(), json!(password));
        }
        ProxyAuth::Uuid {
            uuid,
            alter_id,
            flow,
        } => {
            object.insert("uuid".to_string(), json!(uuid));
            if node.protocol == ProxyProtocol::Vmess {
                object.insert("security".to_string(), json!("auto"));
                object.insert("alter_id".to_string(), json!(alter_id.unwrap_or(0)));
            }
            if let Some(flow) = flow {
                object.insert("flow".to_string(), json!(flow));
            }
        }
        ProxyAuth::Shadowsocks { method, password } => {
            object.insert("method".to_string(), json!(method));
            object.insert("password".to_string(), json!(password));
        }
    }

    if node.protocol == ProxyProtocol::Socks5 {
        object.insert("version".to_string(), json!("5"));
    }

    if node.protocol == ProxyProtocol::Hysteria2 {
        if let Some(options) = &node.hysteria2 {
            if let Some(up_mbps) = options.up_mbps {
                object.insert("up_mbps".to_string(), json!(up_mbps));
            }
            if let Some(down_mbps) = options.down_mbps {
                object.insert("down_mbps".to_string(), json!(down_mbps));
            }
            if let (Some(kind), Some(password)) = (&options.obfs, &options.obfs_password) {
                object.insert(
                    "obfs".to_string(),
                    json!({ "type": kind, "password": password }),
                );
            }
        }
    }

    if node.tls.enabled {
        let mut tls = Map::new();
        tls.insert("enabled".to_string(), json!(true));
        tls.insert("insecure".to_string(), json!(node.tls.insecure));
        if let Some(server_name) = &node.tls.server_name {
            tls.insert("server_name".to_string(), json!(server_name));
        }
        if !node.tls.alpn.is_empty() {
            tls.insert("alpn".to_string(), json!(node.tls.alpn));
        }
        if let Some(fingerprint) = &node.tls.fingerprint {
            tls.insert(
                "utls".to_string(),
                json!({ "enabled": true, "fingerprint": fingerprint }),
            );
        }
        if let Some(public_key) = &node.tls.reality_public_key {
            tls.insert(
                "reality".to_string(),
                json!({
                    "enabled": true,
                    "public_key": public_key,
                    "short_id": node.tls.reality_short_id.clone().unwrap_or_default(),
                }),
            );
        }
        object.insert("tls".to_string(), Value::Object(tls));
    }

    let transport = transport_to_value(node);
    if !transport.is_null() {
        object.insert("transport".to_string(), transport);
    }
    Value::Object(object)
}

// Selectors can switch at runtime without rebuilding TUN routes, so every
// literal proxy endpoint must bypass the TUN from startup.
fn proxy_endpoint_routes(request: &ConnectionRequest) -> Vec<String> {
    let mut seen = HashSet::new();
    request
        .nodes
        .iter()
        .filter_map(|node| node.server.parse::<IpAddr>().ok())
        .filter(|address| seen.insert(*address))
        .map(|address| match address {
            IpAddr::V4(address) => format!("{address}/32"),
            IpAddr::V6(address) => format!("{address}/128"),
        })
        .collect()
}

fn transport_to_value(node: &ProxyNode) -> Value {
    match node.transport.kind.as_str() {
        "ws" | "websocket" => json!({
            "type": "ws",
            "path": node.transport.path.clone().unwrap_or_else(|| "/".to_string()),
            "headers": node.transport.host.as_ref().map(|host| json!({ "Host": host })),
        }),
        "grpc" => json!({
            "type": "grpc",
            "service_name": node.transport.service_name.clone().unwrap_or_default(),
        }),
        "http" | "h2" => json!({
            "type": "http",
            "host": node.transport.host.as_ref().map(|host| vec![host]),
            "path": node.transport.path.clone().unwrap_or_else(|| "/".to_string()),
        }),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_subscription;

    #[test]
    fn builds_anytls_outbound() {
        let nodes = parse_subscription(
            "anytls://secret@edge.example.com:443?type=tcp&insecure=0&fp=chrome&sni=origin.example.com#AnyTLS",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Global,
            tun: false,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());
        let outbound = &config["outbounds"][0];
        assert_eq!(outbound["type"], "anytls");
        assert_eq!(outbound["password"], "secret");
        assert_eq!(outbound["tls"]["enabled"], true);
        assert_eq!(outbound["tls"]["server_name"], "origin.example.com");
        assert_eq!(outbound["tls"]["utls"]["fingerprint"], "chrome");
        assert!(outbound.get("transport").is_none());
    }

    #[test]
    fn builds_selector_protocol_outbounds_and_china_split_rules() {
        let nodes = parse_subscription(
            "hysteria2://secret@hy.example.com:443?sni=hy.example.com#Fast\n\
vless://11111111-1111-1111-1111-111111111111@vl.example.com:443?type=ws&security=tls&sni=vl.example.com#Backup",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: true,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(
            &request,
            &SingBoxOptions {
                traffic_api_port: Some(17891),
                traffic_api_secret: Some("test-secret".to_string()),
                ..SingBoxOptions::default()
            },
        );

        assert_eq!(config["outbounds"][0]["type"], "hysteria2");
        assert_eq!(config["outbounds"][2]["type"], "selector");
        assert_eq!(config["inbounds"][1]["type"], "tun");
        assert_eq!(
            config["inbounds"][1]["address"],
            json!(["172.19.0.1/30", "fdfe:dcba:9876::1/126"])
        );
        assert_eq!(config["inbounds"][1]["mtu"], 1500);
        assert!(config["dns"].get("strategy").is_none());
        assert_eq!(config["route"]["final"], "proxy");
        assert_eq!(config["route"]["auto_detect_interface"], true);
        assert_eq!(config["route"]["rules"][0]["port"], 53);
        assert_eq!(config["route"]["rules"][1]["action"], "sniff");
        assert_eq!(
            config["route"]["rules"][2]["ip_is_private"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(config["route"]["rules"][3]["rule_set"], "geosite-cn");
        assert_eq!(config["route"]["rules"][4]["rule_set"], "geoip-cn");
        assert_eq!(config["route"]["rule_set"][0]["type"], "remote");
        assert_eq!(config["route"]["rule_set"][0]["format"], "binary");
        assert_eq!(config["route"]["rule_set"][0]["download_detour"], "direct");
        assert_eq!(
            config["route"]["rule_set"][1]["url"],
            CHINA_GEOIP_RULE_SET_URL
        );
        assert_eq!(config["dns"]["final"], "dns-proxy");
        assert_eq!(config["dns"]["servers"][0]["type"], "local");
        assert_eq!(config["dns"]["servers"][1]["type"], "https");
        assert_eq!(config["dns"]["servers"][1]["tag"], "dns-bootstrap");
        assert_eq!(config["dns"]["servers"][1]["server"], "1.12.12.12");
        assert_eq!(config["dns"]["servers"][1]["tls"]["server_name"], "doh.pub");
        assert!(config["dns"]["servers"][1].get("detour").is_none());
        assert_eq!(config["dns"]["servers"][2]["server"], "1.1.1.1");
        assert_eq!(config["dns"]["servers"][2]["detour"], "proxy");
        assert_eq!(config["dns"]["servers"][3]["type"], "fakeip");
        assert_eq!(config["dns"]["servers"][3]["tag"], "dns-fakeip");
        assert_eq!(config["dns"]["servers"][3]["inet4_range"], "198.18.0.0/15");
        assert_eq!(config["dns"]["servers"][3]["inet6_range"], "fc00::/18");
        assert_eq!(config["dns"]["rules"][2]["rule_set"], "geosite-cn");
        assert_eq!(config["dns"]["rules"][2]["server"], "dns-direct");
        assert_eq!(config["dns"]["rules"][2]["strategy"], "ipv4_only");
        assert_eq!(
            config["dns"]["rules"][3]["query_type"],
            json!(["A", "AAAA"])
        );
        assert_eq!(config["dns"]["rules"][3]["server"], "dns-fakeip");
        assert_eq!(config["dns"]["independent_cache"], true);
        assert_eq!(config["route"]["default_domain_resolver"], "dns-bootstrap");
        assert_eq!(
            config["experimental"]["clash_api"]["external_controller"],
            "127.0.0.1:17891"
        );
    }

    #[test]
    fn global_and_direct_modes_do_not_load_split_rule_sets() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;

        for (mode, expected_final) in [
            (TunnelMode::Global, "proxy"),
            (TunnelMode::Direct, "direct"),
        ] {
            let request = ConnectionRequest {
                selected_tag: nodes[0].tag.clone(),
                nodes: nodes.clone(),
                proxy_server_nameservers: Vec::new(),
                mode,
                tun: false,
                allow_lan: false,
                custom_rules: Vec::new(),
                config_script: None,
                group_selections: HashMap::new(),
            };
            let config = build_singbox_config(&request, &SingBoxOptions::default());

            assert_eq!(config["route"]["final"], expected_final);
            assert_eq!(config["route"]["auto_detect_interface"], false);
            assert!(config["dns"].get("strategy").is_none());
            assert_eq!(config["route"]["rules"].as_array().map(Vec::len), Some(1));
            assert!(config["route"]["rule_set"]
                .as_array()
                .is_some_and(Vec::is_empty));
            if mode == TunnelMode::Direct {
                assert_eq!(config["dns"]["final"], "dns-direct");
                assert_eq!(config["dns"]["servers"].as_array().map(Vec::len), Some(1));
                assert_eq!(config["route"]["default_domain_resolver"], "dns-direct");
            } else {
                assert_eq!(config["dns"]["final"], "dns-proxy");
                assert_eq!(config["dns"]["servers"].as_array().map(Vec::len), Some(3));
                assert_eq!(config["route"]["default_domain_resolver"], "dns-bootstrap");
            }
        }
    }

    #[test]
    fn subscription_dns_resolves_proxy_endpoints_with_bootstrap_dns() {
        let nodes = parse_subscription(
            "anytls://secret@bilibili-image-cdn.juhazf.cn:50033?sni=example.com#AnyTLS",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: vec![
                "https://cdn.ookkzz.com/message-chat/hello-cn".to_string()
            ],
            mode: TunnelMode::Global,
            tun: false,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());
        let server = &config["dns"]["servers"][2];

        assert_eq!(
            config["route"]["default_domain_resolver"],
            "dns-subscription"
        );
        assert_eq!(server["tag"], "dns-subscription");
        assert_eq!(server["server"], "cdn.ookkzz.com");
        assert_eq!(server["path"], "/message-chat/hello-cn");
        assert_eq!(server["domain_resolver"], "dns-bootstrap");
        assert_eq!(config["dns"]["servers"][3]["tag"], "dns-proxy");
    }

    #[test]
    fn invalid_subscription_dns_falls_back_to_bootstrap_dns() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: vec!["http://resolver.example.com/dns-query".to_string()],
            mode: TunnelMode::Global,
            tun: false,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());

        assert_eq!(config["route"]["default_domain_resolver"], "dns-bootstrap");
        assert_eq!(config["dns"]["servers"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn direct_domain_rules_use_local_dns_before_secure_dns_fallback() {
        let nodes = parse_subscription(
            "hysteria2://secret@155.248.218.187:10086?sni=bing.com&insecure=1#HY2",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: true,
            allow_lan: false,
            custom_rules: vec![
                CustomRule {
                    id: 1,
                    enabled: true,
                    match_type: CustomRuleMatch::DomainSuffix,
                    value: "*.corp.example".to_string(),
                    action: CustomRuleAction::Direct,
                },
                CustomRule {
                    id: 2,
                    enabled: true,
                    match_type: CustomRuleMatch::IpCidr,
                    value: "100.64.0.0/16".to_string(),
                    action: CustomRuleAction::Direct,
                },
            ],
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());
        let dns_rules = config["dns"]["rules"]
            .as_array()
            .expect("DNS rules should be an array");

        assert_eq!(dns_rules[2]["domain_suffix"], json!(["corp.example"]));
        assert_eq!(dns_rules[2]["server"], "dns-direct");
        assert_eq!(dns_rules[2]["strategy"], "ipv4_only");
        assert_eq!(dns_rules[3]["rule_set"], "geosite-cn");
        assert_eq!(dns_rules[3]["server"], "dns-direct");
        assert_eq!(dns_rules[3]["strategy"], "ipv4_only");
        assert_eq!(dns_rules[4]["server"], "dns-fakeip");
        assert_eq!(dns_rules.len(), 5);
    }

    #[test]
    fn direct_dns_is_ipv4_only_but_proxied_fakeip_keeps_both_families() {
        // WeChat prefers IPv6 for media uploads. Domains routed DIRECT must
        // never receive AAAA answers, otherwise the direct IPv6 dial fails on
        // hosts without real IPv6 connectivity (image send breaks). Proxied
        // domains keep the fake-ip AAAA range because the proxy dials them.
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: true,
            allow_lan: false,
            custom_rules: vec![CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "corp.example".to_string(),
                action: CustomRuleAction::Direct,
            }],
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());
        let rules = config["dns"]["rules"]
            .as_array()
            .expect("DNS rules should be an array");

        assert_eq!(rules[2]["domain_suffix"], json!(["corp.example"]));
        assert_eq!(rules[2]["server"], "dns-direct");
        assert_eq!(rules[2]["strategy"], "ipv4_only");
        assert_eq!(rules[3]["rule_set"], "geosite-cn");
        assert_eq!(rules[3]["server"], "dns-direct");
        assert_eq!(rules[3]["strategy"], "ipv4_only");
        let fakeip = rules
            .iter()
            .find(|rule| rule["server"] == "dns-fakeip")
            .expect("fakeip DNS rule should exist");
        assert!(fakeip.get("strategy").is_none());
        assert_eq!(config["dns"]["servers"][3]["inet6_range"], "fc00::/18");
    }

    #[test]
    fn fakeip_is_enabled_only_for_proxy_tun_configs() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;
        let mut request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Global,
            tun: true,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let options = SingBoxOptions {
            cache_file: Some("/app-data/cache/sing-box.db".to_string()),
            ..SingBoxOptions::default()
        };
        let config = build_singbox_config(&request, &options);
        assert!(config["dns"]["servers"]
            .as_array()
            .is_some_and(|servers| { servers.iter().any(|server| server["tag"] == "dns-fakeip") }));
        assert_eq!(config["dns"]["independent_cache"], true);
        assert_eq!(config["experimental"]["cache_file"]["enabled"], true);
        assert_eq!(
            config["experimental"]["cache_file"]["path"],
            "/app-data/cache/sing-box.db"
        );
        assert_eq!(config["experimental"]["cache_file"]["store_fakeip"], true);

        request.tun = false;
        let config = build_singbox_config(&request, &options);
        assert!(config["dns"]["servers"]
            .as_array()
            .is_some_and(|servers| { servers.iter().all(|server| server["tag"] != "dns-fakeip") }));
        assert!(config["dns"].get("independent_cache").is_none());
        assert_eq!(config["experimental"]["cache_file"]["enabled"], true);
        assert_eq!(config["experimental"]["cache_file"]["store_fakeip"], false);

        request.tun = true;
        request.mode = TunnelMode::Direct;
        let config = build_singbox_config(&request, &options);
        assert_eq!(config["dns"]["final"], "dns-direct");
        assert_eq!(config["dns"]["servers"].as_array().map(Vec::len), Some(1));
        assert_eq!(config["experimental"]["cache_file"]["enabled"], true);
        assert_eq!(config["experimental"]["cache_file"]["store_fakeip"], false);
        assert!(config["dns"].get("independent_cache").is_none());
    }

    #[test]
    fn uses_validated_local_rule_set_cache_when_available() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: false,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };
        let config = build_singbox_config(
            &request,
            &SingBoxOptions {
                rule_set_cache: Some(RuleSetCachePaths {
                    geosite: "/app-data/rules/geosite-cn.srs".to_string(),
                    geoip: "/app-data/rules/geoip-cn.srs".to_string(),
                }),
                ..SingBoxOptions::default()
            },
        );

        assert_eq!(config["route"]["rule_set"][0]["type"], "local");
        assert_eq!(
            config["route"]["rule_set"][0]["path"],
            "/app-data/rules/geosite-cn.srs"
        );
        assert_eq!(config["route"]["rule_set"][1]["type"], "local");
        assert_eq!(
            config["route"]["rule_set"][1]["path"],
            "/app-data/rules/geoip-cn.srs"
        );
        assert!(config["route"]["rule_set"][0]["url"].is_null());
    }

    #[test]
    fn custom_rules_keep_priority_and_map_to_native_singbox_fields() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: false,
            allow_lan: false,
            custom_rules: vec![
                CustomRule {
                    id: 1,
                    enabled: true,
                    match_type: crate::CustomRuleMatch::DomainSuffix,
                    value: "*.Example.com".to_string(),
                    action: crate::CustomRuleAction::Proxy,
                },
                CustomRule {
                    id: 2,
                    enabled: true,
                    match_type: crate::CustomRuleMatch::IpCidr,
                    value: "203.0.113.9/24".to_string(),
                    action: crate::CustomRuleAction::Block,
                },
                CustomRule {
                    id: 3,
                    enabled: false,
                    match_type: crate::CustomRuleMatch::DomainKeyword,
                    value: "disabled".to_string(),
                    action: crate::CustomRuleAction::Direct,
                },
            ],
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());
        let rules = config["route"]["rules"]
            .as_array()
            .expect("route rules should be an array");

        assert_eq!(rules[2]["ip_is_private"], true);
        assert_eq!(rules[3]["domain_suffix"], json!(["example.com"]));
        assert_eq!(rules[3]["outbound"], "proxy");
        assert_eq!(rules[4]["ip_cidr"], json!(["203.0.113.0/24"]));
        assert_eq!(rules[4]["outbound"], "block");
        assert_eq!(rules[5]["rule_set"], "geosite-cn");
        assert_eq!(rules.len(), 7);
    }

    #[test]
    fn process_rules_map_to_process_name_and_process_path_fields() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: false,
            allow_lan: false,
            custom_rules: vec![
                CustomRule {
                    id: 1,
                    enabled: true,
                    match_type: crate::CustomRuleMatch::ProcessName,
                    value: "Telegram".to_string(),
                    action: crate::CustomRuleAction::Direct,
                },
                CustomRule {
                    id: 2,
                    enabled: true,
                    match_type: crate::CustomRuleMatch::ProcessPath,
                    value: "/Applications/Telegram.app/Contents/MacOS/Telegram".to_string(),
                    action: crate::CustomRuleAction::Proxy,
                },
            ],
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());
        let rules = config["route"]["rules"]
            .as_array()
            .expect("route rules should be an array");

        assert_eq!(rules[3]["process_name"], json!(["Telegram"]));
        assert_eq!(rules[3]["outbound"], "direct");
        assert_eq!(
            rules[4]["process_path"],
            json!(["/Applications/Telegram.app/Contents/MacOS/Telegram"])
        );
        assert_eq!(rules[4]["outbound"], "proxy");
        assert_eq!(rules.len(), 7);

        let request_global = ConnectionRequest {
            mode: TunnelMode::Global,
            ..request
        };
        let config_global = build_singbox_config(&request_global, &SingBoxOptions::default());
        assert_eq!(
            config_global["route"]["rules"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn process_name_normalization_strips_path_to_basename() {
        let normalized = crate::normalize_custom_rule_value(
            crate::CustomRuleMatch::ProcessName,
            "/Applications/Telegram.app/Contents/MacOS/Telegram",
        )
        .expect("path should normalize to basename");
        assert_eq!(normalized, "Telegram");

        assert!(
            crate::normalize_custom_rule_value(crate::CustomRuleMatch::ProcessName, "").is_err()
        );
        assert!(
            crate::normalize_custom_rule_value(crate::CustomRuleMatch::ProcessName, "a b c")
                .is_ok()
        );

        let normalized_path = crate::normalize_custom_rule_value(
            crate::CustomRuleMatch::ProcessPath,
            "/Applications/Telegram.app/",
        )
        .expect("absolute path should be accepted");
        assert_eq!(normalized_path, "/Applications/Telegram.app");
        assert!(crate::normalize_custom_rule_value(
            crate::CustomRuleMatch::ProcessPath,
            "Telegram"
        )
        .is_err());
    }

    #[test]
    fn custom_rules_are_inactive_outside_rule_mode() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Global,
            tun: false,
            allow_lan: false,
            custom_rules: vec![CustomRule {
                id: 1,
                enabled: true,
                match_type: crate::CustomRuleMatch::Domain,
                value: "example.com".to_string(),
                action: crate::CustomRuleAction::Block,
            }],
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());
        assert_eq!(config["route"]["rules"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn builds_http_and_socks_outbounds_and_excludes_all_ip_endpoints_from_tun() {
        let nodes = parse_subscription(
            "http://100.64.0.2:11080#Company\n\
socks5://alice:secret@127.0.0.1:1080#Socks",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Rule,
            tun: true,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());

        assert_eq!(config["outbounds"][0]["type"], "http");
        assert!(config["outbounds"][0].get("username").is_none());
        assert_eq!(config["outbounds"][1]["type"], "socks");
        assert_eq!(config["outbounds"][1]["version"], "5");
        assert_eq!(config["outbounds"][1]["username"], "alice");
        assert_eq!(
            config["inbounds"][1]["route_exclude_address"],
            json!(["100.64.0.2/32", "127.0.0.1/32"])
        );
    }

    #[test]
    fn tun_excludes_literal_endpoints_for_runtime_selection_without_inventing_domain_routes() {
        let mut nodes = parse_subscription(
            "http://proxy.example.com:8080#Domain\n\
http://192.0.2.10:8080#IPv4\n\
http://198.51.100.20:8080#IPv6\n\
http://192.0.2.10:8081#Duplicate",
        )
        .nodes;
        nodes[2].server = "2001:db8::7".to_string();
        let mut request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Global,
            tun: true,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());

        assert_eq!(
            config["inbounds"][1]["route_exclude_address"],
            json!(["192.0.2.10/32", "2001:db8::7/128"])
        );

        request.tun = false;
        let config = build_singbox_config(&request, &SingBoxOptions::default());
        assert_eq!(config["inbounds"].as_array().map(Vec::len), Some(1));
        assert!(config["inbounds"][0].get("route_exclude_address").is_none());
    }

    #[test]
    fn builds_https_proxy_as_http_outbound_with_tls() {
        let nodes =
            parse_subscription("https://alice:secret@proxy.example.com:8443?insecure=1#Secure")
                .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            proxy_server_nameservers: Vec::new(),
            mode: TunnelMode::Global,
            tun: false,
            allow_lan: false,
            custom_rules: Vec::new(),
            config_script: None,
            group_selections: HashMap::new(),
        };

        let config = build_singbox_config(&request, &SingBoxOptions::default());

        assert_eq!(config["outbounds"][0]["type"], "http");
        assert_eq!(config["outbounds"][0]["tls"]["enabled"], true);
        assert_eq!(config["outbounds"][0]["tls"]["insecure"], true);
    }

    #[test]
    fn extracts_groups_and_applies_selector_defaults() {
        let mut config = json!({
            "outbounds": [
                {"type": "vless", "tag": "node-a"},
                {
                    "type": "selector",
                    "tag": "AI",
                    "outbounds": ["node-a", "direct"],
                    "default": "node-a"
                },
                {
                    "type": "urltest",
                    "tag": "Auto",
                    "outbounds": ["node-a"]
                }
            ]
        });
        apply_proxy_group_selections(
            &mut config,
            &HashMap::from([("AI".to_string(), "direct".to_string())]),
        );

        let groups = extract_proxy_groups(&config);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].tag, "AI");
        assert_eq!(groups[0].selected, "direct");
        assert_eq!(groups[1].kind, ProxyGroupKind::UrlTest);
        assert_eq!(groups[1].selected, "node-a");
    }
}
