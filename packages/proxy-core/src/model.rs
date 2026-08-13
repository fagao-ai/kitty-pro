use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use thiserror::Error;

pub const MAX_CUSTOM_RULES: usize = 256;
pub const SYNC_SNAPSHOT_FORMAT: &str = "kitty-pro-sync";
pub const SYNC_SNAPSHOT_VERSION: u32 = 2;
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
    #[serde(alias = "mixed")]
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
    #[serde(default)]
    pub proxy_server_nameservers: Vec<String>,
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
    ProcessName,
    ProcessPath,
}

impl CustomRuleMatch {
    pub fn singbox_field(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::DomainSuffix => "domain_suffix",
            Self::DomainKeyword => "domain_keyword",
            Self::IpCidr => "ip_cidr",
            Self::ProcessName => "process_name",
            Self::ProcessPath => "process_path",
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
    #[error("进程名不能为空或包含控制字符，请直接填进程名（如 Telegram）")]
    InvalidProcessName,
    #[error("进程路径必须是绝对路径，如 /Applications/Telegram.app/Contents/MacOS/Telegram 或 C:\\Program Files\\Telegram\\Telegram.exe")]
    InvalidProcessPath,
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
        CustomRuleMatch::ProcessName => {
            let name = value
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .ok_or(CustomRuleValidationError::InvalidProcessName)?;
            if name.chars().any(char::is_control) {
                return Err(CustomRuleValidationError::InvalidProcessName);
            }
            Ok(name.to_string())
        }
        CustomRuleMatch::ProcessPath => {
            if value.chars().any(char::is_control) || !is_portable_absolute_path(value) {
                return Err(CustomRuleValidationError::InvalidProcessPath);
            }
            Ok(trim_process_path(value))
        }
    }
}

fn is_portable_absolute_path(value: &str) -> bool {
    if value.starts_with('/') {
        return true;
    }

    let bytes = value.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let unc_absolute = value.starts_with("\\\\")
        && value[2..]
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .take(2)
            .count()
            == 2;
    drive_absolute || unc_absolute
}

fn trim_process_path(value: &str) -> String {
    let trimmed = value.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return "/".to_string();
    }
    if trimmed.len() == 2
        && trimmed.as_bytes()[0].is_ascii_alphabetic()
        && trimmed.as_bytes()[1] == b':'
    {
        return value[..3].to_string();
    }
    trimmed.to_string()
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
    pub proxy_server_nameservers: Vec<String>,
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
    pub proxy_server_nameservers: Vec<String>,
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

/// The portable part of a profile shared between devices.
///
/// Device-local networking and UI preferences deliberately stay out of this
/// structure. `updated_at` is a Unix timestamp used for optimistic conflict
/// detection by the remote storage adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSubscription {
    pub id: u64,
    pub name: String,
    pub source: String,
}

impl From<&Subscription> for SyncSubscription {
    fn from(subscription: &Subscription) -> Self {
        Self {
            id: subscription.id,
            name: subscription.name.clone(),
            source: subscription.source.clone(),
        }
    }
}

impl From<SyncSubscription> for Subscription {
    fn from(subscription: SyncSubscription) -> Self {
        Self {
            id: subscription.id,
            name: subscription.name,
            source: subscription.source,
            nodes: Vec::new(),
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSnapshot {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub subscriptions: Vec<SyncSubscription>,
    #[serde(default)]
    pub custom_rules: Vec<CustomRule>,
}

impl SyncSnapshot {
    pub fn from_profile(profile: &AppProfile, updated_at: u64) -> Self {
        Self {
            format: default_sync_snapshot_format(),
            version: default_sync_snapshot_version(),
            updated_at,
            subscriptions: profile
                .subscriptions
                .iter()
                .map(SyncSubscription::from)
                .collect(),
            custom_rules: profile.custom_rules.clone(),
        }
    }

    pub fn apply_to_profile(&self, profile: &mut AppProfile) {
        profile.subscriptions = self
            .subscriptions
            .iter()
            .cloned()
            .map(Subscription::from)
            .collect();
        profile.active_subscription_id = None;
        profile.selected_tag.clear();
        profile.custom_rules = self.custom_rules.clone();
        profile.group_selections.clear();
    }
}

fn default_sync_snapshot_format() -> String {
    SYNC_SNAPSHOT_FORMAT.to_string()
}

const fn default_sync_snapshot_version() -> u32 {
    SYNC_SNAPSHOT_VERSION
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
                proxy_server_nameservers: vec!["https://resolver.example.com/dns-query".to_string()],
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
        assert_eq!(
            restored.subscriptions[0].proxy_server_nameservers,
            vec!["https://resolver.example.com/dns-query"]
        );
        assert_eq!(restored.custom_rules.len(), 1);
    }

    #[test]
    fn sync_snapshot_only_contains_subscription_sources_and_rules() {
        let node = crate::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=tls#Edge",
        )
        .nodes
        .pop()
        .expect("fixture should parse");
        let source = AppProfile {
            subscriptions: vec![Subscription {
                id: 7,
                name: "Primary".to_string(),
                source: "https://example.com/subscription?token=secret".to_string(),
                nodes: vec![node],
                proxy_server_nameservers: vec!["https://resolver.example/dns-query".to_string()],
                rejected_count: 2,
            }],
            active_subscription_id: Some(7),
            selected_tag: "subscription-7-node".to_string(),
            dark_mode: true,
            tun_enabled: true,
            allow_lan: true,
            custom_rules: vec![CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "example.com".to_string(),
                action: CustomRuleAction::Proxy,
            }],
            group_selections: HashMap::from([("proxy".to_string(), "edge".to_string())]),
            ..AppProfile::default()
        };
        let snapshot = SyncSnapshot::from_profile(&source, 42);
        let mut target = AppProfile {
            dark_mode: false,
            tun_enabled: false,
            allow_lan: false,
            ..AppProfile::default()
        };

        snapshot.apply_to_profile(&mut target);

        assert_eq!(snapshot.format, SYNC_SNAPSHOT_FORMAT);
        assert_eq!(snapshot.version, SYNC_SNAPSHOT_VERSION);
        assert_eq!(snapshot.updated_at, 42);
        assert_eq!(target.subscriptions.len(), 1);
        assert!(target.subscriptions[0].nodes.is_empty());
        assert!(target.subscriptions[0].proxy_server_nameservers.is_empty());
        assert_eq!(target.active_subscription_id, None);
        assert!(target.selected_tag.is_empty());
        assert_eq!(target.custom_rules, source.custom_rules);
        assert!(target.group_selections.is_empty());
        assert!(!target.dark_mode);
        assert!(!target.tun_enabled);
        assert!(!target.allow_lan);

        let serialized = serde_json::to_value(&snapshot).expect("snapshot should serialize");
        assert!(serialized.get("active_subscription_id").is_none());
        assert!(serialized.get("selected_tag").is_none());
        assert!(serialized.get("group_selections").is_none());
        assert!(serialized["subscriptions"][0].get("nodes").is_none());
    }

    #[test]
    fn older_profiles_keep_lan_access_disabled() {
        let restored: AppProfile = serde_json::from_str(
            r#"{
                "subscriptions": [{
                    "id": 1,
                    "name": "Legacy",
                    "source": "legacy",
                    "nodes": []
                }]
            }"#,
        )
        .expect("profile should deserialize");

        assert!(!restored.allow_lan);
        assert!(restored.subscriptions[0]
            .proxy_server_nameservers
            .is_empty());
    }

    #[test]
    fn transient_mixed_proxy_profiles_migrate_to_socks5() {
        let restored: ProxyNode = serde_json::from_str(
            r#"{
                "tag": "company",
                "name": "Company",
                "protocol": "mixed",
                "server": "100.64.0.2",
                "port": 11080,
                "auth": { "kind": "none" }
            }"#,
        )
        .expect("transient mixed proxy node should remain readable");

        assert_eq!(restored.protocol, ProxyProtocol::Socks5);
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
    fn process_rules_use_portable_path_semantics() {
        assert_eq!(
            normalize_custom_rule_value(
                CustomRuleMatch::ProcessName,
                r"C:\Program Files\Telegram Desktop\Telegram.exe",
            ),
            Ok("Telegram.exe".to_string())
        );
        assert_eq!(
            normalize_custom_rule_value(
                CustomRuleMatch::ProcessName,
                "/Applications/Telegram.app/Contents/MacOS/Telegram",
            ),
            Ok("Telegram".to_string())
        );
        assert_eq!(
            normalize_custom_rule_value(
                CustomRuleMatch::ProcessPath,
                r"C:\Program Files\Telegram Desktop\Telegram.exe",
            ),
            Ok(r"C:\Program Files\Telegram Desktop\Telegram.exe".to_string())
        );
        assert_eq!(
            normalize_custom_rule_value(
                CustomRuleMatch::ProcessPath,
                "/Applications/Telegram.app/Contents/MacOS/Telegram/",
            ),
            Ok("/Applications/Telegram.app/Contents/MacOS/Telegram".to_string())
        );
        assert_eq!(
            normalize_custom_rule_value(CustomRuleMatch::ProcessPath, "/"),
            Ok("/".to_string())
        );
        assert!(normalize_custom_rule_value(CustomRuleMatch::ProcessName, "/").is_err());
        assert!(normalize_custom_rule_value(CustomRuleMatch::ProcessPath, "Telegram").is_err());
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
