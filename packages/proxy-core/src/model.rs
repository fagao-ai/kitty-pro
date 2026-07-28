use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use thiserror::Error;

pub const MAX_CUSTOM_RULES: usize = 256;
const MAX_CUSTOM_RULE_VALUE_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyProtocol {
    AnyTls,
    Hysteria2,
    Vmess,
    Vless,
    Trojan,
    Shadowsocks,
    Http,
    Socks5,
}

impl ProxyProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::AnyTls => "AnyTLS",
            Self::Hysteria2 => "Hysteria2",
            Self::Vmess => "VMess",
            Self::Vless => "VLESS",
            Self::Trojan => "Trojan",
            Self::Shadowsocks => "Shadowsocks",
            Self::Http => "HTTP",
            Self::Socks5 => "SOCKS5",
        }
    }
}

impl fmt::Display for ProxyProtocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProxyAuth {
    None,
    UserPassword {
        username: String,
        password: String,
    },
    Password {
        password: String,
    },
    Uuid {
        uuid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alter_id: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flow: Option<String>,
    },
    Shadowsocks {
        method: String,
        password: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportOptions {
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsOptions {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alpn: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reality_public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reality_short_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hysteria2Options {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obfs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obfs_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_mbps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_mbps: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyNode {
    pub tag: String,
    pub name: String,
    pub protocol: ProxyProtocol,
    pub server: String,
    pub port: u16,
    pub auth: ProxyAuth,
    #[serde(default)]
    pub transport: TransportOptions,
    #[serde(default)]
    pub tls: TlsOptions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hysteria2: Option<Hysteria2Options>,
}

impl ProxyNode {
    pub fn endpoint(&self) -> String {
        if self.server.contains(':') && !self.server.starts_with('[') {
            format!("[{}]:{}", self.server, self.port)
        } else {
            format!("{}:{}", self.server, self.port)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseIssue {
    pub line: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseReport {
    pub nodes: Vec<ProxyNode>,
    pub rejected: Vec<ParseIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TunnelMode {
    #[default]
    Rule,
    Global,
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomRuleAction {
    Direct,
    Proxy,
    Block,
}

impl CustomRuleAction {
    pub fn outbound(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Proxy => "proxy",
            Self::Block => "block",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomRuleMatch {
    Domain,
    DomainSuffix,
    DomainKeyword,
    IpCidr,
}

impl CustomRuleMatch {
    pub fn singbox_field(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::DomainSuffix => "domain_suffix",
            Self::DomainKeyword => "domain_keyword",
            Self::IpCidr => "ip_cidr",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomRule {
    pub id: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub match_type: CustomRuleMatch,
    pub value: String,
    pub action: CustomRuleAction,
}

impl CustomRule {
    pub fn normalized(mut self) -> Result<Self, CustomRuleValidationError> {
        self.value = normalize_custom_rule_value(self.match_type, &self.value)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CustomRuleValidationError {
    #[error("规则内容不能为空")]
    Empty,
    #[error("规则内容不能超过 {MAX_CUSTOM_RULE_VALUE_BYTES} 字节")]
    TooLong,
    #[error("域名格式无效")]
    InvalidDomain,
    #[error("域名关键字不能包含空白、路径或 URL scheme")]
    InvalidDomainKeyword,
    #[error("CIDR 格式无效，请输入类似 192.168.0.0/16 或 2001:db8::/32")]
    InvalidCidr,
    #[error("自定义规则不能超过 {MAX_CUSTOM_RULES} 条")]
    TooManyRules,
    #[error("规则 ID 必须唯一且不能为 0")]
    InvalidId,
    #[error("已存在相同的匹配规则")]
    Duplicate,
}

pub fn normalize_custom_rule_value(
    match_type: CustomRuleMatch,
    value: &str,
) -> Result<String, CustomRuleValidationError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(CustomRuleValidationError::Empty);
    }
    if value.len() > MAX_CUSTOM_RULE_VALUE_BYTES {
        return Err(CustomRuleValidationError::TooLong);
    }

    match match_type {
        CustomRuleMatch::Domain | CustomRuleMatch::DomainSuffix => {
            let value = if match_type == CustomRuleMatch::DomainSuffix {
                value
                    .strip_prefix("*.")
                    .or_else(|| value.strip_prefix('.'))
                    .unwrap_or(value)
            } else {
                value
            };
            let value = value.trim_end_matches('.');
            match url::Host::parse(value) {
                Ok(url::Host::Domain(domain)) if !domain.is_empty() => Ok(domain),
                _ => Err(CustomRuleValidationError::InvalidDomain),
            }
        }
        CustomRuleMatch::DomainKeyword => {
            if value.chars().any(char::is_whitespace)
                || value.contains("://")
                || value.contains('/')
                || value.contains('\\')
            {
                return Err(CustomRuleValidationError::InvalidDomainKeyword);
            }
            Ok(value.to_lowercase())
        }
        CustomRuleMatch::IpCidr => value
            .parse::<ipnet::IpNet>()
            .map(|network| network.trunc().to_string())
            .map_err(|_| CustomRuleValidationError::InvalidCidr),
    }
}

pub fn validate_custom_rules(rules: &[CustomRule]) -> Result<(), CustomRuleValidationError> {
    if rules.len() > MAX_CUSTOM_RULES {
        return Err(CustomRuleValidationError::TooManyRules);
    }
    let mut ids = HashSet::with_capacity(rules.len());
    let mut matches = HashSet::with_capacity(rules.len());
    for rule in rules {
        if rule.id == 0 || !ids.insert(rule.id) {
            return Err(CustomRuleValidationError::InvalidId);
        }
        let value = normalize_custom_rule_value(rule.match_type, &rule.value)?;
        if !matches.insert((rule.match_type, value)) {
            return Err(CustomRuleValidationError::Duplicate);
        }
    }
    Ok(())
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRequest {
    pub nodes: Vec<ProxyNode>,
    pub selected_tag: String,
    #[serde(default)]
    pub mode: TunnelMode,
    #[serde(default)]
    pub tun: bool,
    #[serde(default)]
    pub allow_lan: bool,
    #[serde(default)]
    pub custom_rules: Vec<CustomRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_script: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub group_selections: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyGroupKind {
    Selector,
    UrlTest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyGroup {
    pub tag: String,
    pub kind: ProxyGroupKind,
    pub outbounds: Vec<String>,
    pub selected: String,
}

/// A subscription together with its last successful parse result.
///
/// `source` keeps the original URL or inline payload so a native client can
/// refresh it without asking the user to paste it again. Consumers should not
/// render the value verbatim because subscription URLs may carry an access
/// token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: u64,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub nodes: Vec<ProxyNode>,
    #[serde(default)]
    pub rejected_count: usize,
}

impl Subscription {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// The portable, user-owned part of the application state.
///
/// Native shells persist this structure locally; the web shell receives it
/// from its configured server. Proxy credentials are not placed in browser
/// storage by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppProfile {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
    #[serde(default)]
    pub active_subscription_id: Option<u64>,
    #[serde(default)]
    pub selected_tag: String,
    #[serde(default)]
    pub tunnel_mode: TunnelMode,
    #[serde(default)]
    pub tun_enabled: bool,
    #[serde(default)]
    pub allow_lan: bool,
    #[serde(default)]
    pub dark_mode: bool,
    #[serde(default)]
    pub custom_rules: Vec<CustomRule>,
    #[serde(default)]
    pub config_script_enabled: bool,
    #[serde(default)]
    pub config_script: String,
    #[serde(default)]
    pub group_selections: HashMap<String, String>,
}

impl Default for AppProfile {
    fn default() -> Self {
        Self {
            version: default_profile_version(),
            subscriptions: Vec::new(),
            active_subscription_id: None,
            selected_tag: String::new(),
            tunnel_mode: TunnelMode::Rule,
            tun_enabled: false,
            allow_lan: false,
            dark_mode: false,
            custom_rules: Vec::new(),
            config_script_enabled: false,
            config_script: String::new(),
            group_selections: HashMap::new(),
        }
    }
}

const fn default_profile_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_round_trips_proxy_credentials() {
        let node = crate::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=tls#Edge",
        )
        .nodes
        .pop()
        .expect("fixture should parse");
        let profile = AppProfile {
            subscriptions: vec![Subscription {
                id: 7,
                name: "Primary".to_string(),
                source: "https://example.com/subscription?token=secret".to_string(),
                nodes: vec![node.clone()],
                rejected_count: 1,
            }],
            selected_tag: node.tag,
            allow_lan: true,
            dark_mode: true,
            custom_rules: vec![CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "example.com".to_string(),
                action: CustomRuleAction::Direct,
            }],
            ..AppProfile::default()
        };

        let restored: AppProfile = serde_json::from_slice(
            &serde_json::to_vec(&profile).expect("profile should serialize"),
        )
        .expect("profile should deserialize");

        assert_eq!(restored, profile);
        assert!(restored.allow_lan);
        assert_eq!(restored.subscriptions[0].node_count(), 1);
        assert_eq!(restored.custom_rules.len(), 1);
    }

    #[test]
    fn older_profiles_keep_lan_access_disabled() {
        let restored: AppProfile = serde_json::from_str("{}").expect("profile should deserialize");

        assert!(!restored.allow_lan);
    }

    #[test]
    fn custom_rule_values_are_normalized_and_validated() {
        assert_eq!(
            normalize_custom_rule_value(CustomRuleMatch::DomainSuffix, "*.Example.COM."),
            Ok("example.com".to_string())
        );
        assert_eq!(
            normalize_custom_rule_value(CustomRuleMatch::IpCidr, "192.168.8.9/24"),
            Ok("192.168.8.0/24".to_string())
        );
        assert!(
            normalize_custom_rule_value(CustomRuleMatch::Domain, "https://example.com").is_err()
        );
        assert!(normalize_custom_rule_value(CustomRuleMatch::IpCidr, "192.168.0.1").is_err());
    }

    #[test]
    fn custom_rule_ids_must_be_unique() {
        let rule = CustomRule {
            id: 7,
            enabled: true,
            match_type: CustomRuleMatch::Domain,
            value: "example.com".to_string(),
            action: CustomRuleAction::Proxy,
        };
        assert!(validate_custom_rules(std::slice::from_ref(&rule)).is_ok());
        assert_eq!(
            validate_custom_rules(&[rule.clone(), rule]),
            Err(CustomRuleValidationError::InvalidId)
        );
    }

    #[test]
    fn normalized_custom_rule_matches_must_be_unique() {
        let rules = [
            CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "*.Example.com".to_string(),
                action: CustomRuleAction::Proxy,
            },
            CustomRule {
                id: 2,
                enabled: false,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "example.com.".to_string(),
                action: CustomRuleAction::Direct,
            },
        ];

        assert_eq!(
            validate_custom_rules(&rules),
            Err(CustomRuleValidationError::Duplicate)
        );
    }
}
