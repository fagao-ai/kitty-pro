use serde_json::Value;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::process::Command;

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
    route_source_with(
        target,
        port,
        route_source_via_udp,
        route_source_without_kitty_tun,
    )
}

fn route_source_with<U, F>(
    target: IpAddr,
    port: u16,
    udp_source: U,
    fallback_source: F,
) -> Result<IpAddr, String>
where
    U: FnOnce(IpAddr, u16) -> Result<IpAddr, String>,
    F: FnOnce(IpAddr) -> Result<IpAddr, String>,
{
    let source = udp_source(target, port)?;
    if !is_kitty_tun_source(&source) {
        return Ok(source);
    }

    // A candidate config is built while the old Kitty TUN is still running.
    // UDP route selection therefore sees utun5 even though the proxy endpoint
    // must be reached through the underlying interface. Ask macOS for the
    // best route on every non-Kitty interface before treating this as a loop.
    fallback_source(target).or(Ok(source))
}

fn route_source_via_udp(target: IpAddr, port: u16) -> Result<IpAddr, String> {
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

fn route_source_without_kitty_tun(target: IpAddr) -> Result<IpAddr, String> {
    let interfaces = command_stdout("/sbin/ifconfig", &["-l"])?
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let default_interface = route_info("default", None)
        .ok()
        .map(|route| route.interface);
    let target_text = target.to_string();
    let candidates = interfaces
        .into_iter()
        .filter_map(|interface| {
            let addresses = interface_addresses(&interface).ok()?;
            let route = route_info(&target_text, Some(&interface)).ok()?;
            Some(InterfaceRoute {
                interface,
                addresses,
                route,
            })
        })
        .collect::<Vec<_>>();

    select_route_source(target, default_interface.as_deref(), &candidates)
        .ok_or_else(|| "无法找到 Kitty TUN 之外的可用路由源地址".to_string())
}

#[derive(Debug, PartialEq, Eq)]
struct RouteInfo {
    interface: String,
    destination: String,
}

impl RouteInfo {
    fn is_specific(&self) -> bool {
        self.destination != "default"
    }
}

#[derive(Debug)]
struct InterfaceRoute {
    interface: String,
    addresses: Vec<IpAddr>,
    route: RouteInfo,
}

fn select_route_source(
    target: IpAddr,
    default_interface: Option<&str>,
    candidates: &[InterfaceRoute],
) -> Option<IpAddr> {
    let mut default_source = None;

    for candidate in candidates {
        if candidate.addresses.iter().any(is_kitty_tun_source)
            || candidate.route.interface != candidate.interface
        {
            continue;
        }
        let Some(source) = candidate
            .addresses
            .iter()
            .copied()
            .find(|address| is_usable_source(*address, target))
        else {
            continue;
        };
        if candidate.route.is_specific() {
            return Some(source);
        }
        if default_interface == Some(candidate.interface.as_str()) {
            default_source = Some(source);
        }
    }

    default_source
}

fn command_stdout(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("执行 {program} 失败: {error}"))?;
    if !output.status.success() {
        return Err(format!("{program} 返回失败状态 {}", output.status));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("解析 {program} 输出失败: {error}"))
}

fn route_info(target: &str, interface: Option<&str>) -> Result<RouteInfo, String> {
    let mut args = vec!["-n", "get"];
    if let Some(interface) = interface {
        args.extend(["-ifscope", interface]);
    }
    args.push(target);
    let output = command_stdout("/sbin/route", &args)?;
    parse_route_info(&output).ok_or_else(|| format!("/sbin/route 未返回 {target} 的完整路由"))
}

fn parse_route_info(output: &str) -> Option<RouteInfo> {
    let field = |name: &str| {
        output.lines().find_map(|line| {
            line.trim()
                .strip_prefix(name)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    };
    Some(RouteInfo {
        interface: field("interface:")?.to_string(),
        destination: field("destination:")?.to_string(),
    })
}

fn interface_addresses(interface: &str) -> Result<Vec<IpAddr>, String> {
    let output = command_stdout("/sbin/ifconfig", &[interface])?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let kind = fields.next()?;
            if kind != "inet" && kind != "inet6" {
                return None;
            }
            fields
                .next()?
                .split('%')
                .next()
                .and_then(|address| address.parse().ok())
        })
        .collect())
}

fn is_kitty_tun_source(address: &IpAddr) -> bool {
    *address == IpAddr::V4(KITTY_TUN_IPV4) || *address == IpAddr::V6(KITTY_TUN_IPV6)
}

fn is_usable_source(address: IpAddr, target: IpAddr) -> bool {
    if address.is_ipv4() != target.is_ipv4()
        || address.is_unspecified()
        || address.is_multicast()
        || address.is_loopback() != target.is_loopback()
    {
        return false;
    }
    !matches!(address, IpAddr::V6(address) if address.is_unicast_link_local())
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

    fn interface_route(interface: &str, addresses: &[&str], destination: &str) -> InterfaceRoute {
        InterfaceRoute {
            interface: interface.to_string(),
            addresses: addresses
                .iter()
                .map(|address| address.parse().unwrap())
                .collect(),
            route: RouteInfo {
                interface: interface.to_string(),
                destination: destination.to_string(),
            },
        }
    }

    #[test]
    fn parses_macos_route_output() {
        let route = parse_route_info(
            "   route to: 155.248.218.187\n\
             destination: default\n\
             gateway: 192.168.50.1\n\
             interface: en1\n",
        )
        .unwrap();

        assert_eq!(
            route,
            RouteInfo {
                interface: "en1".to_string(),
                destination: "default".to_string(),
            }
        );
    }

    #[test]
    fn selects_physical_default_route_while_kitty_tun_is_running() {
        let target = "155.248.218.187".parse().unwrap();
        let candidates = vec![
            interface_route("utun4", &["100.64.0.249"], "default"),
            interface_route("utun5", &["172.19.0.1"], "152.0.0.0"),
            interface_route("en1", &["192.168.50.13"], "default"),
        ];

        assert_eq!(
            select_route_source(target, Some("en1"), &candidates),
            Some("192.168.50.13".parse().unwrap())
        );
    }

    #[test]
    fn preserves_a_more_specific_route_from_another_vpn() {
        let target = "100.64.0.2".parse().unwrap();
        let candidates = vec![
            interface_route("en1", &["192.168.50.13"], "default"),
            interface_route("utun4", &["100.64.0.249"], "100.64.0.0"),
            interface_route("utun5", &["172.19.0.1"], "96.0.0.0"),
        ];

        assert_eq!(
            select_route_source(target, Some("en1"), &candidates),
            Some("100.64.0.249".parse().unwrap())
        );
    }

    #[test]
    fn never_selects_the_kitty_tun_interface() {
        let target = "155.248.218.187".parse().unwrap();
        let candidates = vec![interface_route(
            "utun5",
            &["172.19.0.1", "fdfe:dcba:9876::1"],
            "152.0.0.0",
        )];

        assert_eq!(
            select_route_source(target, Some("utun5"), &candidates),
            None
        );
    }

    #[test]
    fn ignores_ipv6_link_local_source_addresses() {
        let target = "2001:db8:2::10".parse().unwrap();
        let candidates = vec![interface_route(
            "en1",
            &["fe80::824:882c:44d5:116d", "2001:db8:1::20"],
            "default",
        )];

        assert_eq!(
            select_route_source(target, Some("en1"), &candidates),
            Some("2001:db8:1::20".parse().unwrap())
        );
    }

    #[test]
    fn failed_fallback_keeps_the_kitty_source_for_loop_rejection() {
        let target = "155.248.218.187".parse().unwrap();
        let source = route_source_with(
            target,
            10086,
            |_, _| Ok(IpAddr::V4(KITTY_TUN_IPV4)),
            |_| Err("no non-Kitty route".to_string()),
        )
        .unwrap();

        assert_eq!(source, IpAddr::V4(KITTY_TUN_IPV4));
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
