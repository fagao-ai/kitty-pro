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

// YAML 解析前的输入预算（本地常量，不依赖 api 的 MAX_SUBSCRIPTION_BYTES）。
// 与订阅下载 10 MiB 上限对齐，限制行数/缩进/锚点数量以抵御 alias 展开炸弹。
const MAX_YAML_BYTES: usize = 10 * 1024 * 1024;
const MAX_YAML_LINES: usize = 200_000;
const MAX_YAML_INDENT_DEPTH: usize = 200;
const MAX_YAML_ANCHOR_ALIASES: usize = 64;

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

/// serde_yaml 在反序列化时会无上限地展开 anchor/alias，billion-laughs
/// 文档能把 10 MiB 订阅膨胀成 GB 级内存打爆进程。Clash 订阅从不依赖
/// YAML 锚点复用，因此解析前先检查是否存在“anchor 定义 + alias 引用”
/// 的组合：单独出现 `&`/`*` 字符（密码、备注里的符号）不受影响，只有
/// 定义并被引用时才拒绝。
///
/// Q3：锚点名按 token 起始字符 `&`/`*` 识别，后面的名字不再限制为 ASCII——
/// YAML 1.2 允许非 ASCII 字符作锚点名（如 `&节点` / `*节点`）。旧实现只把
/// ASCII 字母数字/`_ - .` 当作名字，遇到 `&节点` 会读出空名字而整对漏检，
/// 让 Unicode 锚点名炸弹绕过本启发式（yaml_input_budget 的数量上限也未必覆盖
/// 层数少但展开量大的文档）。名字切片按 UTF-8 字节整体推进，停在分隔符上，
/// 保证 `line[name_start..end]` 始终落在字符边界、不会 panic。
fn yaml_expansion_bomb(text: &str) -> bool {
    let mut anchors: Vec<&str> = Vec::new();
    let mut aliases: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = match line.find('#') {
            // 仅当 # 前有空白才视为注释（值内的 # 不算），足够本检查使用。
            Some(index) if index > 0 && line.as_bytes()[index - 1].is_ascii_whitespace() => {
                &line[..index]
            }
            _ => line,
        };
        let bytes = line.as_bytes();
        let mut position = 0;
        while position < bytes.len() {
            let byte = bytes[position];
            let at_token_start = position == 0
                || bytes[position - 1].is_ascii_whitespace()
                || matches!(bytes[position - 1], b'[' | b'{' | b',');
            if at_token_start && (byte == b'&' || byte == b'*') {
                let name_start = position + 1;
                let mut end = name_start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric()
                        || matches!(bytes[end], b'_' | b'-' | b'.')
                        // 允许任意非 ASCII 字节：UTF-8 多字节字符整体作为名字，
                        // 不再要求后续字符必须是 ASCII（Q3）。
                        || !bytes[end].is_ascii())
                {
                    end += 1;
                }
                if end > name_start {
                    let name = &line[name_start..end];
                    if byte == b'&' {
                        anchors.push(name);
                    } else {
                        aliases.push(name);
                    }
                    position = end;
                    continue;
                }
            }
            position += 1;
        }
    }
    !anchors.is_empty() && aliases.iter().any(|alias| anchors.contains(alias))
}

/// 仅当 `#` 位于行首或前面是空白时才视为注释（值内的 `#` 不算）。
fn strip_yaml_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] == b'#' && (index == 0 || bytes[index - 1].is_ascii_whitespace()) {
            return &line[..index];
        }
    }
    line
}

/// 判断一行是否以块标量指示符（| / > 及其 + - 数字变体）结尾。
fn is_block_scalar_indicator(trimmed: &str) -> bool {
    trimmed
        .split_whitespace()
        .last()
        .is_some_and(|token| token.starts_with('|') || token.starts_with('>'))
}

/// YAML 反序列化前的输入预算校验：在调用 serde_yaml::from_str 之前限制总字节、
/// 行数、缩进深度、锚点/别名数量，并拒绝自定义标签（!!python/object 等）与
/// 块标量之外的可疑 &/* 组合。与 yaml_expansion_bomb 启发式叠加为多层防线。
pub(crate) fn yaml_input_budget(source: &str) -> Result<(), ParseError> {
    if source.len() > MAX_YAML_BYTES {
        return Err(ParseError::InvalidYaml(format!(
            "YAML 输入超过 {MAX_YAML_BYTES} 字节安全上限"
        )));
    }
    let mut lines = 0usize;
    let mut anchor_alias_count = 0usize;
    let mut block_scalar_indent: Option<usize> = None;
    for line in source.lines() {
        lines += 1;
        if lines > MAX_YAML_LINES {
            return Err(ParseError::InvalidYaml(format!(
                "YAML 输入超过 {MAX_YAML_LINES} 行安全上限"
            )));
        }
        let indent = line.bytes().take_while(|byte| *byte == b' ').count();
        let content = strip_yaml_comment(line);
        let trimmed = content.trim();

        // 块标量内部的行是纯文本，不参与缩进/标签/锚点检查。
        if let Some(base_indent) = block_scalar_indent {
            if trimmed.is_empty() || indent > base_indent {
                continue;
            }
            block_scalar_indent = None;
        }
        if indent > MAX_YAML_INDENT_DEPTH {
            return Err(ParseError::InvalidYaml(format!(
                "YAML 缩进深度超过 {MAX_YAML_INDENT_DEPTH} 层安全上限"
            )));
        }

        let bytes = content.as_bytes();
        let mut position = 0;
        while position < bytes.len() {
            let byte = bytes[position];
            let at_token_start = position == 0
                || bytes[position - 1].is_ascii_whitespace()
                || matches!(bytes[position - 1], b'[' | b'{' | b',');
            // 拒绝自定义/危险 YAML 标签（!!python/object、!!str 等）。
            if at_token_start
                && byte == b'!'
                && position + 1 < bytes.len()
                && bytes[position + 1] == b'!'
            {
                return Err(ParseError::InvalidYaml(
                    "YAML 输入包含不允许的自定义标签".to_string(),
                ));
            }
            // 统计块标量之外的锚点/别名标记。
            if at_token_start && (byte == b'&' || byte == b'*') {
                anchor_alias_count += 1;
                if anchor_alias_count > MAX_YAML_ANCHOR_ALIASES {
                    return Err(ParseError::InvalidYaml(format!(
                        "YAML 锚点/别名数量超过 {MAX_YAML_ANCHOR_ALIASES} 个安全上限"
                    )));
                }
            }
            position += 1;
        }
        if is_block_scalar_indicator(trimmed) {
            block_scalar_indent = Some(indent);
        }
    }
    Ok(())
}

fn parse_clash_yaml(text: &str) -> Result<ParseReport, ParseError> {
    yaml_input_budget(text)?;
    if yaml_expansion_bomb(text) {
        return Err(ParseError::InvalidYaml(
            "订阅使用了 YAML 锚点引用，出于资源安全考虑已拒绝解析".to_string(),
        ));
    }
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
    fn yaml_alias_guard_rejects_bombs_and_keeps_normal_passwords() {
        // anchor 定义 + alias 引用 = 拒绝（billion-laughs 展开炸弹）。
        let bomb = "proxy: &a [{x: 1}]\nb: [*a, *a, *a]\nc: [*b, *b, *b]";
        assert!(yaml_expansion_bomb(bomb));
        // 普通订阅里密码/备注含 *、&、# 等符号不受影响。
        let ordinary = "proxies:\n  - name: 节点 & 备注 #1\n    password: p@ss*word&x#\n    server: a*b.example.com\n";
        assert!(!yaml_expansion_bomb(ordinary));
        // 只有 alias 没有 anchor 定义（如 flow 标记 `*`）也放行。
        assert!(!yaml_expansion_bomb("proxies:\n  - flow: xord***\n"));
        // alias 引用了不存在的 anchor 不触发指数展开，放行给解析器处理。
        assert!(!yaml_expansion_bomb("a: &x 1\nb: *y\n"));

        // 端到端：恶意 Clash YAML 订阅被拒绝且不会 panic。
        let report = parse_subscription("proxies: &a [1]\nother: [*a, *a]");
        assert!(report.nodes.is_empty());
        assert!(report
            .rejected
            .iter()
            .any(|issue| issue.reason.contains("锚点")));
    }

    #[test]
    fn yaml_expansion_bomb_detects_unicode_and_mixed_case_anchor_names() {
        // Q3：Unicode 锚点名必须和 ASCII 名一样被识别为 anchor+alias 组合并
        // 拒绝；旧实现只读 ASCII 名字，会把 `&节点` 读成空名字而漏检。
        assert!(yaml_expansion_bomb(
            "a: &节点 [1, 2]\nb: [*节点, *节点, *节点]"
        ));
        assert!(yaml_expansion_bomb("a: &é锚 [1]\nb: *é锚\n"));
        assert!(yaml_expansion_bomb("proxies:\n  - &名字 1\n  - *名字\n"));
        // 混合大小写：YAML 锚点名区分大小写，`&A`/`*A` 命中，`&A`/`*a` 是不同的
        // 锚点（后者是未定义 alias），不应误判为炸弹。
        assert!(yaml_expansion_bomb("a: &A 1\nb: *A\n"));
        assert!(!yaml_expansion_bomb("a: &A 1\nb: *a\n"));
        // `&a`/`*a` 出现在行首/值首（token 起始）算锚点/别名；出现在单词内部
        // （前面不是空白/流起始符）只是普通字符，不误判。
        assert!(yaml_expansion_bomb("a: &anchor 1\nb: *anchor\n"));
        assert!(!yaml_expansion_bomb("a: x&anchor 1\nb: x*anchor\n"));
        // 端到端：Unicode 炸弹被拒绝且不 panic。
        let report = parse_subscription("proxies: &节点 [1]\nother: [*节点, *节点]");
        assert!(report.nodes.is_empty());
        assert!(report
            .rejected
            .iter()
            .any(|issue| issue.reason.contains("锚点")));
    }

    #[test]
    fn yaml_anchor_guard_ignores_block_scalar_text_and_normal_passwords() {
        // 正常密码 / 备注里的 & / * 不被误伤：不在 token 起始，或没有同名引用。
        let ordinary = "proxies:\n  - name: 节点 & 备注 #1\n    password: p@ss*word&x#\n    server: a*b.example.com\n    remark: *lone-alias-without-anchor\n";
        assert!(!yaml_expansion_bomb(ordinary));
        // 块标量 | / > 内部的 *alias 文本是纯文本，不应被当作 alias 炸弹，
        // yaml_input_budget 也保持「块标量内部行不参与检查」的既有正确行为。
        let block = "\
proxies:
  - name: test
    type: vmess
    server: x.example.com
    port: 443
    uuid: 11111111-1111-1111-1111-111111111111
    payload: |
      *alias *节点 plain text
      folded > *alias *alias *alias
";
        assert!(!yaml_expansion_bomb(block));
        assert!(yaml_input_budget(block).is_ok());
        // 块标量内即便有远超 64 上限的 *alias 文本，也不计入锚点/别名预算。
        let many = format!(
            "proxies:\n  - payload: |\n{}",
            "      *alias text *节点\n".repeat(80)
        );
        assert!(yaml_input_budget(&many).is_ok());
    }

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
            "http://198.51.100.20:11080#Company\n\
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
        assert!(is_http_proxy_share_link(
            "http://198.51.100.20:11080#Company"
        ));
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
    server: 198.51.100.20
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

    #[test]
    fn yaml_input_budget_rejects_oversize_source() {
        let source = "a".repeat(MAX_YAML_BYTES + 1);
        let error = yaml_input_budget(&source).expect_err("oversize YAML must be rejected");
        assert!(error.to_string().contains("字节"));
    }

    #[test]
    fn yaml_input_budget_rejects_excessive_indent() {
        let source = format!("{}proxies: []", " ".repeat(MAX_YAML_INDENT_DEPTH + 1));
        let error = yaml_input_budget(&source).expect_err("deep indent must be rejected");
        assert!(error.to_string().contains("缩进"));
    }

    #[test]
    fn yaml_input_budget_rejects_too_many_anchors_and_aliases() {
        let mut source = String::from("proxies:\n");
        for _ in 0..(MAX_YAML_ANCHOR_ALIASES + 1) {
            source.push_str("  - &anchor x\n");
        }
        let error = yaml_input_budget(&source).expect_err("too many anchors must be rejected");
        assert!(error.to_string().contains("锚点"));
    }

    #[test]
    fn yaml_input_budget_rejects_custom_tags() {
        let error = yaml_input_budget("proxies: !!python/object/apply:os.system ['x']\n")
            .expect_err("custom YAML tags must be rejected");
        assert!(error.to_string().contains("标签"));
    }

    #[test]
    fn yaml_input_budget_is_wired_into_clash_yaml_parsing() {
        // 端到端：带自定义标签的 Clash 订阅在 serde_yaml 解析前即被拒绝。
        let report = parse_subscription("proxies: !!python/object/apply:os.system ['x']\n");
        assert!(report.nodes.is_empty());
        assert!(report
            .rejected
            .iter()
            .any(|issue| issue.reason.contains("标签")));
    }

    #[test]
    fn yaml_input_budget_allows_normal_clash_yaml() {
        let source = "\
proxies:
  - name: HK
    type: vmess
    server: hk.example.com
    port: 443
    uuid: 11111111-1111-1111-1111-111111111111
    password: p@ss*word&value
";
        assert!(yaml_input_budget(source).is_ok());
        let report = parse_subscription(source);
        assert_eq!(report.nodes.len(), 1);
        assert!(report.rejected.is_empty());
    }

    // ===== T3：确定性生成式测试（纯标准库 LCG，不引入 rand/proptest）=====

    #[test]
    fn parse_subscription_never_panics_on_generated_inputs() {
        use crate::deterministic_test_support::Lcg;

        const SEED: u64 = 0x4b49_5454_5950_524f; // "KITTYPRO"
        let mut lcg = Lcg::new(SEED);
        // 固定的危险片段轮转叠加到随机字节串上：NUL、CRLF、BOM、YAML
        // 锚点/别名、!!python 标签、嵌套引号、非 base64、未闭合 IPv6。
        let hazards: &[&str] = &[
            "\0",
            "\r\n",
            "\u{feff}",
            "proxies:\n  - &a 1\n  - *a\n",
            "!!python/object/apply:os.system ['x']\n",
            "\"\"'':\n",
            "vmess://!!!not-base64!!!",
            "ss://@@@@",
            "http://[::1\n",
            "proxies: [",
        ];
        for iteration in 0..200 {
            let mut input = String::new();
            let raw_len = (lcg.next() % 512) as usize + 1;
            // from_utf8_lossy 保证是合法 &str，同时保留 NUL / 控制字符。
            input.push_str(&String::from_utf8_lossy(&lcg.bytes(raw_len)));
            input.push_str(hazards[(lcg.next() as usize) % hazards.len()]);
            input.push_str(&lcg.text(64));
            if lcg.next().is_multiple_of(8) {
                // 超长输入：只验证不 panic、不要求解析成功。
                input.push_str(&"A".repeat(10 * 1024 + (lcg.next() % 256) as usize));
            }
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                parse_subscription(&input)
            }));
            let Ok(report) = outcome else {
                panic!("seed={SEED:#x} iteration={iteration}: parse_subscription panicked");
            };
            // parse_subscription 返回 ParseReport（不是 Result）；"Err 情形"即
            // rejected 非空/无有效节点。成功解析出的任意节点都必须满足不变量。
            for node in &report.nodes {
                assert!(
                    !node.tag.trim().is_empty(),
                    "seed={SEED:#x} iteration={iteration}: 节点 tag 为空"
                );
                assert!(
                    !node.tag.chars().any(char::is_control),
                    "seed={SEED:#x} iteration={iteration}: 节点 tag 含控制字符 {:?}",
                    node.tag
                );
                assert!(
                    !node.server.is_empty(),
                    "seed={SEED:#x} iteration={iteration}: 节点 address 为空"
                );
            }
        }
    }

    #[test]
    fn yaml_input_budget_generated_hazards_and_normal_clash() {
        use crate::deterministic_test_support::Lcg;

        const SEED: u64 = 0x5941_4d4c_4255_4447; // "YAMLBUDG"
        let mut lcg = Lcg::new(SEED);
        for iteration in 0..200 {
            let variant = iteration % 5;
            let source = match variant {
                // 10 MiB 以上
                0 => "a".repeat(MAX_YAML_BYTES + 1 + (lcg.next() % 2048) as usize),
                // 200 层以上缩进
                1 => format!(
                    "{}proxies: []",
                    " ".repeat(MAX_YAML_INDENT_DEPTH + 1 + (lcg.next() % 64) as usize)
                ),
                // 超过 64 个 anchor/alias
                2 => {
                    let count = MAX_YAML_ANCHOR_ALIASES + 1 + (lcg.next() % 32) as usize;
                    let mut text = String::from("proxies:\n");
                    for _ in 0..count {
                        text.push_str("  - &anchor x\n");
                    }
                    text
                }
                // 自定义/危险 YAML 标签。key 必须是纯字母数字：随机字符里
                // 的 '#' 会把整行变成注释、'\n' 会拆行，那样 `!!` 根本不是
                // 标签，测的就不是危险输入了。
                3 => {
                    let key: String = lcg
                        .text(12)
                        .chars()
                        .filter(|character| character.is_ascii_alphanumeric())
                        .collect();
                    format!(
                        "proxies:\n  - name: {key}\n    payload: !!python/object/apply:os.system ['x']\n"
                    )
                }
                // 正常 Clash YAML（随机但安全的 name）
                _ => {
                    let name: String = lcg
                        .text(16)
                        .chars()
                        .filter(|character| character.is_ascii_alphanumeric())
                        .collect();
                    format!(
                        "proxies:\n  - name: {name}\n    type: vmess\n    server: edge.example.com\n    port: 443\n    uuid: 11111111-1111-1111-1111-111111111111\n"
                    )
                }
            };
            if variant < 4 {
                assert!(
                    yaml_input_budget(&source).is_err(),
                    "seed={SEED:#x} iteration={iteration} variant={variant}: 危险 YAML 必须被拒绝"
                );
            } else {
                assert!(
                    yaml_input_budget(&source).is_ok(),
                    "seed={SEED:#x} iteration={iteration}: 正常 Clash YAML 必须通过"
                );
                let report = parse_subscription(&source);
                assert_eq!(
                    report.nodes.len(),
                    1,
                    "seed={SEED:#x} iteration={iteration}: 正常 Clash YAML 应解析出 1 个节点"
                );
            }
        }
    }
}
