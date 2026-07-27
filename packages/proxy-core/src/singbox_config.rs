use crate::{
    normalize_custom_rule_value, ConnectionRequest, CustomRule, ProxyAuth, ProxyGroup,
    ProxyGroupKind, ProxyNode, ProxyProtocol, TunnelMode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::net::IpAddr;

pub const CHINA_GEOSITE_RULE_SET_URL: &str =
    "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geosite/cn.srs";
pub const CHINA_GEOIP_RULE_SET_URL: &str =
    "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@sing/geo/geoip/cn.srs";

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
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "auto_route": true,
            "strict_route": true,
            "stack": "system",
        });
        let route_exclusions = selected_proxy_endpoint_routes(request);
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

    let mut config = json!({
        "log": {
            "level": options.log_level,
            "timestamp": true,
        },
        "dns": {
            "servers": [{
                "type": "local",
                "tag": "dns-direct",
            }],
            "final": "dns-direct",
        },
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": {
            // Mixed/system-proxy connections must follow host routes so
            // tailnets and other VPN interfaces remain reachable. Desktop
            // TUN mode still pins outbounds to avoid re-entering its own TUN.
            "auto_detect_interface": request.tun,
            "rules": rules,
            "rule_set": rule_sets,
            "final": route_final,
        },
    });
    if let Some(port) = options.traffic_api_port {
        config["experimental"] = json!({
            "clash_api": {
                "external_controller": format!("127.0.0.1:{port}"),
                "secret": options.traffic_api_secret,
            },
        });
    }
    config
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

fn selected_proxy_endpoint_routes(request: &ConnectionRequest) -> Vec<String> {
    request
        .nodes
        .iter()
        .find(|node| node.tag == request.selected_tag)
        .or_else(|| request.nodes.first())
        .and_then(|node| node.server.parse::<IpAddr>().ok())
        .map(|address| match address {
            IpAddr::V4(address) => format!("{address}/32"),
            IpAddr::V6(address) => format!("{address}/128"),
        })
        .into_iter()
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
            mode: TunnelMode::Global,
            tun: false,
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
            mode: TunnelMode::Rule,
            tun: true,
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
        assert_eq!(config["dns"]["final"], "dns-direct");
        assert_eq!(config["dns"]["servers"][0]["type"], "local");
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
                mode,
                tun: false,
                custom_rules: Vec::new(),
                config_script: None,
                group_selections: HashMap::new(),
            };
            let config = build_singbox_config(&request, &SingBoxOptions::default());

            assert_eq!(config["route"]["final"], expected_final);
            assert_eq!(config["route"]["auto_detect_interface"], false);
            assert_eq!(config["route"]["rules"].as_array().map(Vec::len), Some(1));
            assert!(config["route"]["rule_set"]
                .as_array()
                .is_some_and(Vec::is_empty));
        }
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
            mode: TunnelMode::Rule,
            tun: false,
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
            mode: TunnelMode::Rule,
            tun: false,
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
    fn custom_rules_are_inactive_outside_rule_mode() {
        let nodes = parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@vl.example.com:443#Node",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            mode: TunnelMode::Global,
            tun: false,
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
    fn builds_http_and_socks_outbounds_and_excludes_ip_endpoints_from_tun() {
        let nodes = parse_subscription(
            "http://100.64.0.2:11080#Company\n\
socks5://alice:secret@127.0.0.1:1080#Socks",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            mode: TunnelMode::Rule,
            tun: true,
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
            json!(["100.64.0.2/32"])
        );
    }

    #[test]
    fn builds_https_proxy_as_http_outbound_with_tls() {
        let nodes =
            parse_subscription("https://alice:secret@proxy.example.com:8443?insecure=1#Secure")
                .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            mode: TunnelMode::Global,
            tun: false,
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
