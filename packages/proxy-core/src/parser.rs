use crate::{
    Hysteria2Options, ParseIssue, ParseReport, ProxyAuth, ProxyNode, ProxyProtocol, TlsOptions,
    TransportOptions,
};
use base64::{engine::general_purpose, Engine as _};
use percent_encoding::percent_decode_str;
use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Value as YamlValue};
use std::collections::HashMap;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("不支持的协议: {0}")]
    UnsupportedProtocol(String),
    #[error("链接格式无效: {0}")]
    InvalidUrl(String),
    #[error("缺少字段: {0}")]
    MissingField(&'static str),
    #[error("端口无效")]
    InvalidPort,
    #[error("Base64 内容无效")]
    InvalidBase64,
    #[error("VMess JSON 无效: {0}")]
    InvalidVmess(String),
    #[error("Clash YAML 无效: {0}")]
    InvalidYaml(String),
}

pub fn parse_subscription(input: &str) -> ParseReport {
    let text = input.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return ParseReport {
            rejected: vec![ParseIssue {
                line: 0,
                reason: "订阅内容为空".to_string(),
            }],
            ..Default::default()
        };
    }

    if looks_like_clash_yaml(text) {
        return parse_clash_yaml(text).unwrap_or_else(|error| ParseReport {
            rejected: vec![ParseIssue {
                line: 0,
                reason: error.to_string(),
            }],
            ..Default::default()
        });
    }

    let decoded;
    let lines = if contains_share_link(text) {
        text
    } else if let Some(value) = decode_base64_text(text) {
        decoded = value;
        decoded.as_str()
    } else {
        text
    };

    parse_uri_lines(lines)
}

pub fn parse_share_link(link: &str, index: usize) -> Result<ProxyNode, ParseError> {
    let scheme = link
        .split_once("://")
        .map(|(scheme, _)| scheme.to_ascii_lowercase())
        .ok_or_else(|| ParseError::InvalidUrl("缺少协议头".to_string()))?;

    match scheme.as_str() {
        "anytls" => parse_standard_url(link, index, ProxyProtocol::AnyTls),
        "hysteria2" | "hy2" => parse_standard_url(link, index, ProxyProtocol::Hysteria2),
        "vless" => parse_standard_url(link, index, ProxyProtocol::Vless),
        "trojan" => parse_standard_url(link, index, ProxyProtocol::Trojan),
        "vmess" => parse_vmess(link, index),
        "ss" => parse_shadowsocks(link, index),
        "http" => parse_proxy_url(link, index, ProxyProtocol::Http, false),
        "https" => parse_proxy_url(link, index, ProxyProtocol::Http, true),
        "socks" | "socks5" => parse_proxy_url(link, index, ProxyProtocol::Socks5, false),
        _ => Err(ParseError::UnsupportedProtocol(scheme)),
    }
}

pub fn is_http_proxy_share_link(value: &str) -> bool {
    if value.lines().count() != 1 {
        return false;
    }
    let Ok(url) = Url::parse(value.trim()) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") || !matches!(url.path(), "" | "/") {
        return false;
    }

    url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || (url.port().is_some() && url.query().is_none())
}

fn parse_uri_lines(text: &str) -> ParseReport {
    let mut report = ParseReport::default();
    for (line_index, raw_line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if !line.contains("://") {
            continue;
        }
        match parse_share_link(line, report.nodes.len()) {
            Ok(node) => report.nodes.push(node),
            Err(error) => report.rejected.push(ParseIssue {
                line: line_number,
                reason: error.to_string(),
            }),
        }
    }

    if report.nodes.is_empty() && report.rejected.is_empty() {
        report.rejected.push(ParseIssue {
            line: 0,
            reason: "没有找到受支持的节点".to_string(),
        });
    }
    uniquify_tags(&mut report.nodes);
    report
}

fn parse_standard_url(
    link: &str,
    index: usize,
    protocol: ProxyProtocol,
) -> Result<ProxyNode, ParseError> {
    let url = Url::parse(link).map_err(|error| ParseError::InvalidUrl(error.to_string()))?;
    let server = url
        .host_str()
        .filter(|value| !value.is_empty())
        .ok_or(ParseError::MissingField("server"))?
        .to_string();
    let port = url.port().ok_or(ParseError::InvalidPort)?;
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    let username = decode_component(url.username());
    let password = url.password().map(decode_component);
    let name = fragment_name(&url).unwrap_or_else(|| format!("{} {}", protocol.label(), index + 1));

    let auth = match protocol {
        ProxyProtocol::Vless => ProxyAuth::Uuid {
            uuid: required(username, "uuid")?,
            alter_id: None,
            flow: non_empty(query.get("flow").cloned()),
        },
        ProxyProtocol::AnyTls | ProxyProtocol::Trojan | ProxyProtocol::Hysteria2 => {
            ProxyAuth::Password {
                password: required(password.unwrap_or(username), "password")?,
            }
        }
        _ => return Err(ParseError::UnsupportedProtocol(protocol.to_string())),
    };

    let transport = transport_from_query(&query);
    let tls = tls_from_query(
        &query,
        matches!(protocol, ProxyProtocol::AnyTls | ProxyProtocol::Hysteria2),
    );
    let hysteria2 = (protocol == ProxyProtocol::Hysteria2).then(|| Hysteria2Options {
        obfs: non_empty(query.get("obfs").cloned()),
        obfs_password: non_empty(
            query
                .get("obfs-password")
                .or_else(|| query.get("obfsPassword"))
                .cloned(),
        ),
        up_mbps: query_u32(&query, &["upmbps", "up"]),
        down_mbps: query_u32(&query, &["downmbps", "down"]),
    });

    Ok(ProxyNode {
        tag: make_tag(&name, index),
        name,
        protocol,
        server,
        port,
        auth,
        transport,
        tls,
        hysteria2,
    })
}

fn parse_proxy_url(
    link: &str,
    index: usize,
    protocol: ProxyProtocol,
    tls_enabled: bool,
) -> Result<ProxyNode, ParseError> {
    let url = Url::parse(link).map_err(|error| ParseError::InvalidUrl(error.to_string()))?;
    let server = url
        .host_str()
        .filter(|value| !value.is_empty())
        .ok_or(ParseError::MissingField("server"))?
        .to_string();
    let port = url.port().or(match protocol {
        ProxyProtocol::Http if tls_enabled => Some(443),
        ProxyProtocol::Http => Some(80),
        ProxyProtocol::Socks5 => Some(1080),
        _ => None,
    });
    let port = port.ok_or(ParseError::InvalidPort)?;
    let username = decode_component(url.username());
    let password = url.password().map(decode_component);
    let auth = if username.is_empty() && password.is_none() {
        ProxyAuth::None
    } else {
        ProxyAuth::UserPassword {
            username,
            password: password.unwrap_or_default(),
        }
    };
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    let mut tls = TlsOptions::default();
    if tls_enabled {
        tls = tls_from_query(&query, true);
    }
    let name = fragment_name(&url).unwrap_or_else(|| format!("{} {}", protocol.label(), index + 1));

    Ok(ProxyNode {
        tag: make_tag(&name, index),
        name,
        protocol,
        server,
        port,
        auth,
        transport: TransportOptions::default(),
        tls,
        hysteria2: None,
    })
}

fn parse_vmess(link: &str, index: usize) -> Result<ProxyNode, ParseError> {
    let payload = link
        .strip_prefix("vmess://")
        .or_else(|| link.strip_prefix("VMESS://"))
        .ok_or_else(|| ParseError::InvalidUrl("VMess 协议头无效".to_string()))?
        .split('#')
        .next()
        .unwrap_or_default();
    let decoded = decode_base64_bytes(payload).ok_or(ParseError::InvalidBase64)?;
    let value: JsonValue = serde_json::from_slice(&decoded)
        .map_err(|error| ParseError::InvalidVmess(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| ParseError::InvalidVmess("根节点不是对象".to_string()))?;

    let name = json_string(object.get("ps"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("VMess {}", index + 1));
    let server = json_string(object.get("add")).ok_or(ParseError::MissingField("add"))?;
    let port = json_u16(object.get("port")).ok_or(ParseError::InvalidPort)?;
    let uuid = json_string(object.get("id")).ok_or(ParseError::MissingField("id"))?;
    let alter_id = json_u32(object.get("aid"));
    let network = json_string(object.get("net")).unwrap_or_else(|| "tcp".to_string());
    let host = non_empty(json_string(object.get("host")));
    let path = non_empty(json_string(object.get("path")));
    let service_name = (network == "grpc").then(|| path.clone()).flatten();
    let tls_value = json_string(object.get("tls")).unwrap_or_default();
    let server_name =
        non_empty(json_string(object.get("sni")).or_else(|| json_string(object.get("servername"))));

    Ok(ProxyNode {
        tag: make_tag(&name, index),
        name,
        protocol: ProxyProtocol::Vmess,
        server,
        port,
        auth: ProxyAuth::Uuid {
            uuid,
            alter_id,
            flow: None,
        },
        transport: TransportOptions {
            kind: network,
            path,
            host,
            service_name,
        },
        tls: TlsOptions {
            enabled: matches!(tls_value.as_str(), "tls" | "reality"),
            server_name,
            fingerprint: non_empty(json_string(object.get("fp"))),
            ..Default::default()
        },
        hysteria2: None,
    })
}

fn parse_shadowsocks(link: &str, index: usize) -> Result<ProxyNode, ParseError> {
    let raw = link
        .strip_prefix("ss://")
        .or_else(|| link.strip_prefix("SS://"))
        .ok_or_else(|| ParseError::InvalidUrl("Shadowsocks 协议头无效".to_string()))?;
    let (without_fragment, name) = match raw.split_once('#') {
        Some((value, fragment)) => (value, non_empty(Some(decode_component(fragment)))),
        None => (raw, None),
    };
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);

    let normalized = if without_query.contains('@') {
        let (userinfo, endpoint) = without_query
            .rsplit_once('@')
            .ok_or_else(|| ParseError::InvalidUrl("Shadowsocks 用户信息无效".to_string()))?;
        let decoded_userinfo = if userinfo.contains(':') {
            decode_component(userinfo)
        } else {
            String::from_utf8(decode_base64_bytes(userinfo).ok_or(ParseError::InvalidBase64)?)
                .map_err(|_| ParseError::InvalidBase64)?
        };
        format!("{}@{}", decoded_userinfo, endpoint)
    } else {
        String::from_utf8(decode_base64_bytes(without_query).ok_or(ParseError::InvalidBase64)?)
            .map_err(|_| ParseError::InvalidBase64)?
    };

    let url = Url::parse(&format!("ss://{}", normalized))
        .map_err(|error| ParseError::InvalidUrl(error.to_string()))?;
    let (method, password) = decode_component(url.username())
        .split_once(':')
        .map(|(method, password)| (method.to_string(), password.to_string()))
        .or_else(|| {
            let method = decode_component(url.username());
            url.password()
                .map(|password| (method, decode_component(password)))
        })
        .ok_or(ParseError::MissingField("method/password"))?;
    let server = url
        .host_str()
        .ok_or(ParseError::MissingField("server"))?
        .to_string();
    let port = url.port().ok_or(ParseError::InvalidPort)?;
    let name = name.unwrap_or_else(|| format!("Shadowsocks {}", index + 1));

    Ok(ProxyNode {
        tag: make_tag(&name, index),
        name,
        protocol: ProxyProtocol::Shadowsocks,
        server,
        port,
        auth: ProxyAuth::Shadowsocks { method, password },
        transport: TransportOptions::default(),
        tls: TlsOptions::default(),
        hysteria2: None,
    })
}

fn parse_clash_yaml(text: &str) -> Result<ParseReport, ParseError> {
    let root: YamlValue =
        serde_yaml::from_str(text).map_err(|error| ParseError::InvalidYaml(error.to_string()))?;
    let root = root
        .as_mapping()
        .ok_or_else(|| ParseError::InvalidYaml("根节点不是对象".to_string()))?;
    let proxies = yaml_get(root, "proxies")
        .and_then(YamlValue::as_sequence)
        .ok_or_else(|| ParseError::InvalidYaml("缺少 proxies 列表".to_string()))?;
    let proxy_server_nameservers = yaml_get(root, "dns")
        .and_then(YamlValue::as_mapping)
        .map(|dns| yaml_strings(dns, "proxy-server-nameserver"))
        .unwrap_or_default()
        .into_iter()
        .filter(|server| !server.trim().is_empty())
        .collect();
    let mut report = ParseReport {
        proxy_server_nameservers,
        ..Default::default()
    };

    for (index, value) in proxies.iter().enumerate() {
        match clash_proxy_to_node(value, index) {
            Ok(node) => report.nodes.push(node),
            Err(error) => report.rejected.push(ParseIssue {
                line: index + 1,
                reason: error.to_string(),
            }),
        }
    }
    uniquify_tags(&mut report.nodes);
    Ok(report)
}

fn clash_proxy_to_node(value: &YamlValue, index: usize) -> Result<ProxyNode, ParseError> {
    let mapping = value
        .as_mapping()
        .ok_or_else(|| ParseError::InvalidYaml("代理项不是对象".to_string()))?;
    let kind = yaml_string(mapping, "type")
        .ok_or(ParseError::MissingField("type"))?
        .to_ascii_lowercase();
    let protocol = match kind.as_str() {
        "anytls" => ProxyProtocol::AnyTls,
        "hysteria2" | "hy2" => ProxyProtocol::Hysteria2,
        "vmess" => ProxyProtocol::Vmess,
        "vless" => ProxyProtocol::Vless,
        "trojan" => ProxyProtocol::Trojan,
        "ss" | "shadowsocks" => ProxyProtocol::Shadowsocks,
        "http" | "https" => ProxyProtocol::Http,
        "socks" | "socks5" => ProxyProtocol::Socks5,
        _ => return Err(ParseError::UnsupportedProtocol(kind)),
    };
    let name =
        yaml_string(mapping, "name").unwrap_or_else(|| format!("{} {}", protocol, index + 1));
    let server = yaml_string(mapping, "server").ok_or(ParseError::MissingField("server"))?;
    let port = yaml_u16(mapping, "port").ok_or(ParseError::InvalidPort)?;
    let auth = match protocol {
        ProxyProtocol::Vmess | ProxyProtocol::Vless => ProxyAuth::Uuid {
            uuid: yaml_string(mapping, "uuid").ok_or(ParseError::MissingField("uuid"))?,
            alter_id: yaml_u32(mapping, "alterId").or_else(|| yaml_u32(mapping, "alter-id")),
            flow: non_empty(yaml_string(mapping, "flow")),
        },
        ProxyProtocol::AnyTls | ProxyProtocol::Trojan | ProxyProtocol::Hysteria2 => {
            ProxyAuth::Password {
                password: yaml_string(mapping, "password")
                    .or_else(|| yaml_string(mapping, "auth"))
                    .ok_or(ParseError::MissingField("password"))?,
            }
        }
        ProxyProtocol::Shadowsocks => ProxyAuth::Shadowsocks {
            method: yaml_string(mapping, "cipher").ok_or(ParseError::MissingField("cipher"))?,
            password: yaml_string(mapping, "password")
                .ok_or(ParseError::MissingField("password"))?,
        },
        ProxyProtocol::Http | ProxyProtocol::Socks5 => {
            let username = yaml_string(mapping, "username");
            let password = yaml_string(mapping, "password");
            if username.is_none() && password.is_none() {
                ProxyAuth::None
            } else {
                ProxyAuth::UserPassword {
                    username: username.unwrap_or_default(),
                    password: password.unwrap_or_default(),
                }
            }
        }
    };

    let network = yaml_string(mapping, "network").unwrap_or_else(|| "tcp".to_string());
    let (path, host) = mapping
        .get(YamlValue::String("ws-opts".to_string()))
        .and_then(YamlValue::as_mapping)
        .map(|ws| {
            let path = yaml_string(ws, "path");
            let host = ws
                .get(YamlValue::String("headers".to_string()))
                .and_then(YamlValue::as_mapping)
                .and_then(|headers| {
                    yaml_string(headers, "Host").or_else(|| yaml_string(headers, "host"))
                });
            (path, host)
        })
        .unwrap_or_default();
    let service_name = mapping
        .get(YamlValue::String("grpc-opts".to_string()))
        .and_then(YamlValue::as_mapping)
        .and_then(|grpc| yaml_string(grpc, "grpc-service-name"));
    let tls_enabled = matches!(protocol, ProxyProtocol::AnyTls | ProxyProtocol::Hysteria2)
        || (protocol == ProxyProtocol::Http && kind == "https")
        || yaml_bool(mapping, "tls").unwrap_or(false)
        || yaml_string(mapping, "security")
            .is_some_and(|value| value == "tls" || value == "reality");
    let reality = mapping
        .get(YamlValue::String("reality-opts".to_string()))
        .and_then(YamlValue::as_mapping);
    let hysteria2 = (protocol == ProxyProtocol::Hysteria2).then(|| Hysteria2Options {
        obfs: non_empty(yaml_string(mapping, "obfs")),
        obfs_password: non_empty(yaml_string(mapping, "obfs-password")),
        up_mbps: yaml_u32(mapping, "up").or_else(|| yaml_u32(mapping, "up-mbps")),
        down_mbps: yaml_u32(mapping, "down").or_else(|| yaml_u32(mapping, "down-mbps")),
    });

    Ok(ProxyNode {
        tag: make_tag(&name, index),
        name,
        protocol,
        server,
        port,
        auth,
        transport: TransportOptions {
            kind: network,
            path: non_empty(path),
            host: non_empty(host),
            service_name: non_empty(service_name),
        },
        tls: TlsOptions {
            enabled: tls_enabled,
            insecure: yaml_bool(mapping, "skip-cert-verify").unwrap_or(false),
            server_name: non_empty(
                yaml_string(mapping, "servername").or_else(|| yaml_string(mapping, "sni")),
            ),
            alpn: yaml_strings(mapping, "alpn"),
            fingerprint: non_empty(yaml_string(mapping, "client-fingerprint")),
            reality_public_key: reality.and_then(|value| yaml_string(value, "public-key")),
            reality_short_id: reality.and_then(|value| yaml_string(value, "short-id")),
        },
        hysteria2,
    })
}

fn transport_from_query(query: &HashMap<String, String>) -> TransportOptions {
    let kind = query
        .get("type")
        .or_else(|| query.get("network"))
        .cloned()
        .unwrap_or_else(|| "tcp".to_string());
    TransportOptions {
        path: non_empty(query.get("path").cloned()),
        host: non_empty(query.get("host").cloned()),
        service_name: non_empty(
            query
                .get("serviceName")
                .or_else(|| query.get("service_name"))
                .cloned(),
        ),
        kind,
    }
}

fn tls_from_query(query: &HashMap<String, String>, force_enabled: bool) -> TlsOptions {
    let security = query
        .get("security")
        .map(String::as_str)
        .unwrap_or_default();
    let alpn = query
        .get("alpn")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    TlsOptions {
        enabled: force_enabled || security == "tls" || security == "reality",
        insecure: query_bool(query, &["insecure", "allowInsecure", "skip-cert-verify"]),
        server_name: non_empty(
            query
                .get("sni")
                .or_else(|| query.get("peer"))
                .or_else(|| query.get("serverName"))
                .cloned(),
        ),
        alpn,
        fingerprint: non_empty(query.get("fp").cloned()),
        reality_public_key: non_empty(query.get("pbk").cloned()),
        reality_short_id: non_empty(query.get("sid").or_else(|| query.get("shortId")).cloned()),
    }
}

fn decode_component(value: &str) -> String {
    percent_decode_str(value).decode_utf8_lossy().into_owned()
}

fn fragment_name(url: &Url) -> Option<String> {
    non_empty(url.fragment().map(decode_component))
}

fn required(value: String, field: &'static str) -> Result<String, ParseError> {
    if value.is_empty() {
        Err(ParseError::MissingField(field))
    } else {
        Ok(value)
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn query_bool(query: &HashMap<String, String>, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        query.get(*key).is_some_and(|value| {
            matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
        })
    })
}

fn query_u32(query: &HashMap<String, String>, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        query.get(*key).and_then(|value| {
            value
                .trim_end_matches(|character: char| character.is_ascii_alphabetic())
                .parse()
                .ok()
        })
    })
}

fn json_string(value: Option<&JsonValue>) -> Option<String> {
    match value? {
        JsonValue::String(value) => Some(value.clone()),
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn json_u16(value: Option<&JsonValue>) -> Option<u16> {
    json_string(value)?.parse().ok()
}

fn json_u32(value: Option<&JsonValue>) -> Option<u32> {
    json_string(value)?.parse().ok()
}

fn decode_base64_bytes(value: &str) -> Option<Vec<u8>> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    [
        &general_purpose::STANDARD,
        &general_purpose::STANDARD_NO_PAD,
        &general_purpose::URL_SAFE,
        &general_purpose::URL_SAFE_NO_PAD,
    ]
    .into_iter()
    .find_map(|engine| engine.decode(&compact).ok())
}

fn decode_base64_text(value: &str) -> Option<String> {
    String::from_utf8(decode_base64_bytes(value)?).ok()
}

fn contains_share_link(text: &str) -> bool {
    [
        "anytls://",
        "vmess://",
        "vless://",
        "trojan://",
        "hysteria2://",
        "hy2://",
        "ss://",
        "http://",
        "https://",
        "socks://",
        "socks5://",
    ]
    .iter()
    .any(|scheme| text.to_ascii_lowercase().contains(scheme))
}

fn looks_like_clash_yaml(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with("proxies:"))
}

fn make_tag(name: &str, index: usize) -> String {
    let slug: String = name
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(36)
        .collect();
    if slug.is_empty() {
        format!("node-{}", index + 1)
    } else {
        format!("{}-{}", slug, index + 1)
    }
}

fn uniquify_tags(nodes: &mut [ProxyNode]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for node in nodes {
        let count = counts.entry(node.tag.clone()).or_default();
        if *count > 0 {
            node.tag = format!("{}-{}", node.tag, *count + 1);
        }
        *count += 1;
    }
}

fn yaml_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_string()))
}

fn yaml_string(mapping: &Mapping, key: &str) -> Option<String> {
    match yaml_get(mapping, key)? {
        YamlValue::String(value) => Some(value.clone()),
        YamlValue::Number(value) => Some(value.to_string()),
        YamlValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn yaml_u16(mapping: &Mapping, key: &str) -> Option<u16> {
    yaml_string(mapping, key)?.parse().ok()
}

fn yaml_u32(mapping: &Mapping, key: &str) -> Option<u32> {
    yaml_string(mapping, key)?
        .trim_end_matches(|character: char| character.is_ascii_alphabetic())
        .parse()
        .ok()
}

fn yaml_bool(mapping: &Mapping, key: &str) -> Option<bool> {
    match yaml_get(mapping, key)? {
        YamlValue::Bool(value) => Some(*value),
        YamlValue::String(value) => Some(matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )),
        _ => None,
    }
}

fn yaml_strings(mapping: &Mapping, key: &str) -> Vec<String> {
    match yaml_get(mapping, key) {
        Some(YamlValue::Sequence(values)) => values
            .iter()
            .filter_map(|value| value.as_str().map(ToOwned::to_owned))
            .collect(),
        Some(YamlValue::String(value)) => value
            .split(',')
            .map(|item| item.trim().to_string())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn parses_legacy_base64_subscription() {
        let vmess_json = r#"{"v":"2","ps":"Tokyo","add":"jp.example.com","port":"443","id":"11111111-1111-1111-1111-111111111111","aid":"0","net":"ws","host":"cdn.example.com","path":"/ws","tls":"tls","sni":"jp.example.com"}"#;
        let vmess = format!("vmess://{}", general_purpose::STANDARD.encode(vmess_json));
        let source = format!(
            "{}\nvless://22222222-2222-2222-2222-222222222222@us.example.com:443?type=grpc&security=reality&sni=www.example.com&pbk=public&sid=abcd#US",
            vmess
        );
        let encoded = general_purpose::STANDARD_NO_PAD.encode(source);

        let report = parse_subscription(&encoded);

        assert_eq!(report.nodes.len(), 2);
        assert!(report.rejected.is_empty());
        assert_eq!(report.nodes[0].protocol, ProxyProtocol::Vmess);
        assert_eq!(report.nodes[0].transport.kind, "ws");
        assert_eq!(
            report.nodes[1].tls.reality_public_key.as_deref(),
            Some("public")
        );
    }

    #[test]
    fn parses_hysteria2_and_trojan_links() {
        let source = "hysteria2://secret@hy.example.com:443?sni=hy.example.com&insecure=1&obfs=salamander&obfs-password=mask&upmbps=20&downmbps=100#HY2\n\
trojan://password@tr.example.com:443?type=ws&path=%2Fedge&host=cdn.example.com&security=tls&sni=tr.example.com#Trojan";
        let report = parse_subscription(source);

        assert_eq!(report.nodes.len(), 2);
        assert!(report.nodes[0].tls.insecure);
        assert_eq!(
            report.nodes[0].hysteria2.as_ref().unwrap().down_mbps,
            Some(100)
        );
        assert_eq!(report.nodes[1].transport.path.as_deref(), Some("/edge"));
    }

    #[test]
    fn parses_anytls_link() {
        let report = parse_subscription(
            "anytls://c2VjcmV0JTNEJTNE@edge.example.com:443?type=tcp&insecure=0&fp=chrome&sni=origin.example.com#Hong%20Kong",
        );

        assert_eq!(report.nodes.len(), 1);
        assert!(report.rejected.is_empty());
        let node = &report.nodes[0];
        assert_eq!(node.protocol, ProxyProtocol::AnyTls);
        assert_eq!(node.name, "Hong Kong");
        assert!(node.tls.enabled);
        assert!(!node.tls.insecure);
        assert_eq!(node.tls.server_name.as_deref(), Some("origin.example.com"));
        assert_eq!(node.tls.fingerprint.as_deref(), Some("chrome"));
        assert_eq!(
            node.auth,
            ProxyAuth::Password {
                password: "c2VjcmV0JTNEJTNE".to_string(),
            }
        );
    }

    #[test]
    fn parses_clash_yaml() {
        let source = r#"
dns:
  proxy-server-nameserver:
    - https://cdn.ookkzz.com/message-chat/hello-cn
proxies:
  - name: HK-HY2
    type: hysteria2
    server: hk.example.com
    port: 8443
    password: secret
    sni: hk.example.com
    skip-cert-verify: true
  - name: DE-VLESS
    type: vless
    server: de.example.com
    port: 443
    uuid: 33333333-3333-3333-3333-333333333333
    network: ws
    tls: true
    ws-opts:
      path: /socket
      headers:
        Host: cdn.example.com
"#;
        let report = parse_subscription(source);

        assert_eq!(report.nodes.len(), 2);
        assert!(report.rejected.is_empty());
        assert_eq!(
            report.proxy_server_nameservers,
            vec!["https://cdn.ookkzz.com/message-chat/hello-cn"]
        );
        assert_eq!(
            report.nodes[1].transport.host.as_deref(),
            Some("cdn.example.com")
        );
    }

    #[test]
    fn parses_both_shadowsocks_sip002_forms() {
        let user_info = general_purpose::URL_SAFE_NO_PAD.encode("aes-128-gcm:password");
        let full = general_purpose::URL_SAFE_NO_PAD
            .encode("chacha20-ietf-poly1305:secret@ss2.example.com:8443");
        let report = parse_subscription(&format!(
            "ss://{}@ss.example.com:443#One\nss://{}#Two",
            user_info, full
        ));

        assert_eq!(report.nodes.len(), 2);
        assert!(report.rejected.is_empty());
    }

    #[test]
    fn parses_http_https_and_socks5_proxy_links() {
        let report = parse_subscription(
            "http://100.64.0.2:11080#Company\n\
https://alice:s%40cret@proxy.example.com:8443?insecure=1#Secure\n\
socks5://bob:pass@127.0.0.1:1080#Socks",
        );

        assert_eq!(report.nodes.len(), 3);
        assert!(report.rejected.is_empty());
        assert_eq!(report.nodes[0].protocol, ProxyProtocol::Http);
        assert_eq!(report.nodes[0].auth, ProxyAuth::None);
        assert_eq!(report.nodes[1].protocol, ProxyProtocol::Http);
        assert!(report.nodes[1].tls.enabled);
        assert!(report.nodes[1].tls.insecure);
        assert_eq!(
            report.nodes[1].auth,
            ProxyAuth::UserPassword {
                username: "alice".to_string(),
                password: "s@cret".to_string(),
            }
        );
        assert_eq!(report.nodes[2].protocol, ProxyProtocol::Socks5);
    }

    #[test]
    fn distinguishes_http_proxy_links_from_subscription_urls() {
        assert!(is_http_proxy_share_link("http://100.64.0.2:11080#Company"));
        assert!(is_http_proxy_share_link(
            "https://user:pass@proxy.example.com"
        ));
        assert!(!is_http_proxy_share_link(
            "https://example.com/api/subscription?token=secret"
        ));
    }

    #[test]
    fn parses_clash_http_and_socks5_proxies() {
        let report = parse_subscription(
            r#"
proxies:
  - name: Company HTTP
    type: http
    server: 100.64.0.2
    port: 11080
  - name: Office SOCKS
    type: socks5
    server: socks.example.com
    port: 1080
    username: alice
    password: secret
"#,
        );

        assert_eq!(report.nodes.len(), 2);
        assert!(report.rejected.is_empty());
        assert_eq!(report.nodes[0].protocol, ProxyProtocol::Http);
        assert_eq!(report.nodes[0].auth, ProxyAuth::None);
        assert_eq!(report.nodes[1].protocol, ProxyProtocol::Socks5);
        assert!(matches!(
            report.nodes[1].auth,
            ProxyAuth::UserPassword { .. }
        ));
    }
}
