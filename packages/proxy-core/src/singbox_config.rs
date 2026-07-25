use crate::{ConnectionRequest, ProxyAuth, ProxyNode, ProxyProtocol, TunnelMode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingBoxOptions {
    pub mixed_port: u16,
    pub listen: String,
    pub log_level: String,
    /// Optional loopback-only endpoint used internally to read traffic counters.
    pub traffic_api_port: Option<u16>,
    /// Per-process authentication for the internal traffic endpoint.
    pub traffic_api_secret: Option<String>,
}

impl Default for SingBoxOptions {
    fn default() -> Self {
        Self {
            mixed_port: 7890,
            listen: "127.0.0.1".to_string(),
            log_level: "info".to_string(),
            traffic_api_port: None,
            traffic_api_secret: None,
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
        inbounds.push(json!({
            "type": "tun",
            "tag": "tun-in",
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "auto_route": true,
            "strict_route": true,
            "stack": "system",
        }));
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
    let rules = match request.mode {
        TunnelMode::Rule => vec![
            json!({ "port": 53, "action": "hijack-dns" }),
            json!({ "ip_is_private": true, "outbound": "direct" }),
        ],
        TunnelMode::Global | TunnelMode::Direct => {
            vec![json!({ "port": 53, "action": "hijack-dns" })]
        }
    };

    let mut config = json!({
        "log": {
            "level": options.log_level,
            "timestamp": true,
        },
        "dns": {
            "servers": [{
                "type": "tls",
                "tag": "dns-direct",
                "server": "1.1.1.1",
            }],
            "final": "dns-direct",
        },
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": {
            "auto_detect_interface": true,
            "rules": rules,
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

fn node_to_outbound(node: &ProxyNode) -> Value {
    let mut object = Map::new();
    object.insert(
        "type".to_string(),
        json!(match node.protocol {
            ProxyProtocol::Hysteria2 => "hysteria2",
            ProxyProtocol::Vmess => "vmess",
            ProxyProtocol::Vless => "vless",
            ProxyProtocol::Trojan => "trojan",
            ProxyProtocol::Shadowsocks => "shadowsocks",
        }),
    );
    object.insert("tag".to_string(), json!(node.tag));
    object.insert("server".to_string(), json!(node.server));
    object.insert("server_port".to_string(), json!(node.port));

    match &node.auth {
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
    fn builds_selector_and_protocol_outbounds() {
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
        assert_eq!(config["route"]["rules"][0]["port"], 53);
        assert_eq!(config["dns"]["final"], "dns-direct");
        assert_eq!(
            config["experimental"]["clash_api"]["external_controller"],
            "127.0.0.1:17891"
        );
    }
}
