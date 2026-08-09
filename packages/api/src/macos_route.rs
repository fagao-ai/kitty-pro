use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

const KITTY_TUN_IPV4: Ipv4Addr = Ipv4Addr::new(172, 19, 0, 1);
const KITTY_TUN_IPV6: Ipv6Addr = Ipv6Addr::new(0xfdfe, 0xdcba, 0x9876, 0, 0, 0, 0, 1);
const DEFAULT_ROUTE_PROBE_IPV4: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);
const DEFAULT_ROUTE_PROBE_IPV6: Ipv6Addr = Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1);

pub(crate) fn pin_non_default_outbound_sources(config: &mut Value) -> Result<(), String> {
    pin_non_default_outbound_sources_with(config, route_source)
}

fn pin_non_default_outbound_sources_with<F>(
    config: &mut Value,
    mut resolve_source: F,
) -> Result<(), String>
where
    F: FnMut(IpAddr, u16) -> Result<IpAddr, String>,
{
    if !config_uses_tun(config) {
        return Ok(());
    }

    let Some(outbounds) = config.get_mut("outbounds").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let mut default_ipv4_source: Option<Result<IpAddr, String>> = None;
    let mut default_ipv6_source: Option<Result<IpAddr, String>> = None;
    for outbound in outbounds {
        let Some(object) = outbound.as_object_mut() else {
            continue;
        };
        if object.contains_key("bind_interface")
            || object.contains_key("inet4_bind_address")
            || object.contains_key("inet6_bind_address")
            || object.contains_key("network_strategy")
            || object.contains_key("network_type")
            || object.contains_key("fallback_network_type")
        {
            continue;
        }
        let Some(target) = object
            .get("server")
            .and_then(Value::as_str)
            .and_then(|server| server.parse::<IpAddr>().ok())
        else {
            continue;
        };

        let port = object
            .get("server_port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0)
            .unwrap_or(9);
        let source = resolve_source(target, port)
            .map_err(|error| format!("无法确认代理上游 {target}:{port} 的系统路由: {error}"))?;
        if source.is_ipv4() != target.is_ipv4() || source.is_unspecified() {
            return Err(format!(
                "代理上游 {target}:{port} 返回了无效的源地址 {source}"
            ));
        }
        if source == IpAddr::V4(KITTY_TUN_IPV4) || source == IpAddr::V6(KITTY_TUN_IPV6) {
            return Err(format!(
                "代理上游 {target}:{port} 会重新进入 Kitty TUN，已取消切换"
            ));
        }

        // auto_detect_interface protects ordinary outbound sockets from the
        // new TUN routes, but it also overrides an existing split route. Pin
        // only endpoints macOS routed differently before TUN startup.
        let default_source = match target {
            IpAddr::V4(_) => default_ipv4_source
                .get_or_insert_with(|| resolve_source(IpAddr::V4(DEFAULT_ROUTE_PROBE_IPV4), 9))
                .as_ref(),
            IpAddr::V6(_) => default_ipv6_source
                .get_or_insert_with(|| resolve_source(IpAddr::V6(DEFAULT_ROUTE_PROBE_IPV6), 9))
                .as_ref(),
        };
        if matches!(default_source, Ok(default) if *default == source) {
            continue;
        }

        let bind_key = match source {
            IpAddr::V4(_) => "inet4_bind_address",
            IpAddr::V6(_) => "inet6_bind_address",
        };
        object.insert(bind_key.to_string(), Value::String(source.to_string()));
    }
    Ok(())
}

fn config_uses_tun(config: &Value) -> bool {
    config
        .get("inbounds")
        .and_then(Value::as_array)
        .is_some_and(|inbounds| {
            inbounds
                .iter()
                .any(|inbound| inbound.get("type").and_then(Value::as_str) == Some("tun"))
        })
}

fn route_source(target: IpAddr, port: u16) -> Result<IpAddr, String> {
    let bind_address = match target {
        IpAddr::V4(_) => "0.0.0.0:0",
        IpAddr::V6(_) => "[::]:0",
    };
    let socket = UdpSocket::bind(bind_address)
        .map_err(|error| format!("创建只读路由探测套接字失败: {error}"))?;
    // UDP connect selects a route and local address without sending a packet.
    socket
        .connect(SocketAddr::new(target, port))
        .map_err(|error| format!("查询系统路由失败: {error}"))?;
    socket
        .local_addr()
        .map(|address| address.ip())
        .map_err(|error| format!("读取系统选择的源地址失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tun_config() -> Value {
        json!({
            "inbounds": [{ "type": "tun" }],
            "outbounds": [
                { "type": "http", "tag": "vpn-a", "server": "100.64.0.2", "server_port": 11080 },
                { "type": "socks", "tag": "public", "server": "198.51.100.8", "server_port": 1080 },
                { "type": "http", "tag": "vpn-b", "server": "10.20.30.40", "server_port": 8080 },
                { "type": "socks", "tag": "local", "server": "127.0.0.1", "server_port": 1081 },
                { "type": "selector", "tag": "proxy", "outbounds": ["vpn-a", "public", "vpn-b", "local"] }
            ]
        })
    }

    fn routed_source(target: IpAddr, _: u16) -> Result<IpAddr, String> {
        match target {
            IpAddr::V4(address) if address == DEFAULT_ROUTE_PROBE_IPV4 => {
                Ok(IpAddr::V4(Ipv4Addr::new(192, 168, 50, 207)))
            }
            IpAddr::V4(address) if address == Ipv4Addr::new(100, 64, 0, 2) => {
                Ok(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 249)))
            }
            IpAddr::V4(address) if address == Ipv4Addr::new(10, 20, 30, 40) => {
                Ok(IpAddr::V4(Ipv4Addr::new(10, 8, 0, 2)))
            }
            IpAddr::V4(address) if address == Ipv4Addr::LOCALHOST => {
                Ok(IpAddr::V4(Ipv4Addr::LOCALHOST))
            }
            IpAddr::V4(_) => Ok(IpAddr::V4(Ipv4Addr::new(192, 168, 50, 207))),
            IpAddr::V6(address) if address == DEFAULT_ROUTE_PROBE_IPV6 => {
                Ok("2001:db8:1::20".parse().unwrap())
            }
            IpAddr::V6(_) => Ok("2001:db8:1::20".parse().unwrap()),
        }
    }

    #[test]
    fn pins_every_non_default_candidate_including_unselected_outbounds() {
        let mut config = tun_config();

        pin_non_default_outbound_sources_with(&mut config, routed_source).unwrap();

        assert_eq!(config["outbounds"][0]["inet4_bind_address"], "100.64.0.249");
        assert!(config["outbounds"][1].get("inet4_bind_address").is_none());
        assert_eq!(config["outbounds"][2]["inet4_bind_address"], "10.8.0.2");
        assert_eq!(config["outbounds"][3]["inet4_bind_address"], "127.0.0.1");
    }

    #[test]
    fn keeps_explicit_outbound_routing_controls() {
        let mut config = tun_config();
        config["outbounds"][0]["bind_interface"] = json!("utun9");

        pin_non_default_outbound_sources_with(&mut config, routed_source).unwrap();

        assert_eq!(config["outbounds"][0]["bind_interface"], "utun9");
        assert!(config["outbounds"][0].get("inet4_bind_address").is_none());
    }

    #[test]
    fn leaves_default_route_endpoints_on_auto_detection() {
        let mut config = tun_config();

        pin_non_default_outbound_sources_with(&mut config, |target, _| match target {
            IpAddr::V4(address) if address == Ipv4Addr::LOCALHOST => {
                Ok(IpAddr::V4(Ipv4Addr::LOCALHOST))
            }
            IpAddr::V4(_) => Ok(IpAddr::V4(Ipv4Addr::new(192, 168, 50, 207))),
            IpAddr::V6(_) => Ok("2001:db8:1::20".parse().unwrap()),
        })
        .unwrap();

        assert!(config["outbounds"][0].get("inet4_bind_address").is_none());
        assert!(config["outbounds"][1].get("inet4_bind_address").is_none());
        assert!(config["outbounds"][2].get("inet4_bind_address").is_none());
        assert_eq!(config["outbounds"][3]["inet4_bind_address"], "127.0.0.1");
    }

    #[test]
    fn pins_ipv6_when_only_a_specific_route_is_available() {
        let mut config = tun_config();
        config["outbounds"][1]["server"] = json!("fd00:1234::10");

        pin_non_default_outbound_sources_with(&mut config, |target, port| match target {
            IpAddr::V4(_) => routed_source(target, port),
            IpAddr::V6(address) if address == DEFAULT_ROUTE_PROBE_IPV6 => {
                Err("no default IPv6 route".to_string())
            }
            IpAddr::V6(_) => Ok("fd00:1234::2".parse().unwrap()),
        })
        .unwrap();

        assert_eq!(config["outbounds"][1]["inet6_bind_address"], "fd00:1234::2");
    }

    #[test]
    fn rejects_routes_that_resolve_back_to_kitty_tun() {
        let mut config = tun_config();

        let error = pin_non_default_outbound_sources_with(&mut config, |_, _| {
            Ok(IpAddr::V4(KITTY_TUN_IPV4))
        })
        .unwrap_err();

        assert!(error.contains("会重新进入 Kitty TUN"));
    }

    #[test]
    fn non_tun_config_does_not_probe_routes() {
        let mut config = tun_config();
        config["inbounds"] = json!([{ "type": "mixed" }]);

        pin_non_default_outbound_sources_with(&mut config, |_, _| {
            panic!("route lookup must not run without TUN")
        })
        .unwrap();
    }
}
