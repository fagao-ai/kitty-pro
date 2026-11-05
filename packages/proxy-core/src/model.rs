use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use thiserror::Error;

pub const MAX_CUSTOM_RULES: usize = 256;
pub const SYNC_SNAPSHOT_FORMAT: &str = "kitty-pro-sync";
pub const SYNC_SNAPSHOT_VERSION: u32 = 2;

/// The folder a configured sync path maps to, e.g. `kitty-pro-sync.json` becomes
/// `kitty-pro-sync`. Remote sync stores a bundle of objects under this prefix,
/// derived so an existing single-file setting needs no edit and the legacy
/// object stays readable. Lives here because it is wire-format knowledge and
/// the settings page renders it on every target, web included.
pub fn sync_bundle_directory(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    match trimmed.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("json") && !stem.is_empty() => {
            stem.to_string()
        }
        _ => trimmed.to_string(),
    }
}
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
pub struct DnsOverride {
    pub id: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub domain_suffix: String,
    pub server: String,
}

pub fn normalize_dns_override(
    domain_suffix: &str,
    server: &str,
) -> Result<(String, String), String> {
    let domain = normalize_custom_rule_value(CustomRuleMatch::DomainSuffix, domain_suffix)
        .map_err(|_| "域名后缀无效".to_string())?;
    let server = server.trim();
    let valid_server = server.parse::<std::net::IpAddr>().is_ok()
        || normalize_custom_rule_value(CustomRuleMatch::Domain, server).is_ok();
    if !valid_server {
        return Err("DNS 服务器必须是 IP 或域名".to_string());
    }
    Ok((domain, server.to_string()))
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
    #[serde(default)]
    pub dns_overrides: Vec<DnsOverride>,
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
    pub dns_overrides: Vec<DnsOverride>,
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
    /// Last parsed nodes travel with the snapshot so a pull restores a usable
    /// connection without re-downloading every subscription (README).
    /// `serde(default)` keeps snapshots written before this field parseable.
    #[serde(default)]
    pub nodes: Vec<ProxyNode>,
    #[serde(default)]
    pub proxy_server_nameservers: Vec<String>,
    #[serde(default)]
    pub rejected_count: usize,
}

impl From<&Subscription> for SyncSubscription {
    fn from(subscription: &Subscription) -> Self {
        Self {
            id: subscription.id,
            name: subscription.name.clone(),
            source: subscription.source.clone(),
            nodes: subscription.nodes.clone(),
            proxy_server_nameservers: subscription.proxy_server_nameservers.clone(),
            rejected_count: subscription.rejected_count,
        }
    }
}

impl From<SyncSubscription> for Subscription {
    fn from(subscription: SyncSubscription) -> Self {
        Self {
            id: subscription.id,
            name: subscription.name,
            source: subscription.source,
            nodes: subscription.nodes,
            proxy_server_nameservers: subscription.proxy_server_nameservers,
            rejected_count: subscription.rejected_count,
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
    /// `None` means the remote document carries no DNS section at all, which is
    /// every snapshot uploaded before the feature shipped. Applying one must
    /// leave the local list alone: reading "absent" as "empty" silently deletes
    /// rules the remote never had a chance to send. `Some(vec![])` stays the
    /// different, deliberate case - the remote really has none, so clear it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_overrides: Option<Vec<DnsOverride>>,
    /// Config script, with the same absent-versus-empty rule. Device-local
    /// preferences (theme, TUN, LAN, mode) still deliberately never travel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_script: Option<SyncConfigScript>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfigScript {
    pub enabled: bool,
    pub script: String,
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
            dns_overrides: Some(profile.dns_overrides.clone()),
            config_script: Some(SyncConfigScript {
                enabled: profile.config_script_enabled,
                script: profile.config_script.clone(),
            }),
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
        // Only overwrite sections the document actually carries.
        if let Some(dns_overrides) = &self.dns_overrides {
            profile.dns_overrides = dns_overrides.clone();
        }
        if let Some(config_script) = &self.config_script {
            profile.config_script_enabled = config_script.enabled;
            profile.config_script = config_script.script.clone();
        }
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
            dns_overrides: Vec::new(),
            config_script_enabled: false,
            config_script: String::new(),
            group_selections: HashMap::new(),
        }
    }
}

const fn default_profile_version() -> u32 {
    1
}

/// 当前迁移器认识的 profile schema 版本（B9）。与 `AppProfile.version`（本地
/// 文件格式版本，历史恒为 1）刻意区分：它供加载层从原始 JSON 判断数据新旧，
/// 而 `migrate_app_profile` 对内存中的 AppProfile 做保守归一化、不写回 version。
pub const CURRENT_PROFILE_SCHEMA: u32 = 3;

/// 从原始 profile JSON 读取可选的 `schema_version`，缺省/非法/负数/溢出一律
/// 返回 0（对应最旧的历史数据）。proxy-core 内部暂无生产调用点：加载与版本
/// 协商在 api 层，故此处仅作为 crate 内工具保留，压制 dead_code 以免影响
/// `clippy -D warnings` 门禁。
#[allow(dead_code)]
pub(crate) fn profile_schema_version(value: &serde_json::Value) -> u32 {
    value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .unwrap_or(0)
}

/// 脚本长度的保守上限，与 api 层 `MAX_PROFILE_CONFIG_SCRIPT_BYTES`（512 KiB）
/// 对齐；迁移时把历史遗留的超大脚本截断，避免写盘/序列化放大（REVIEW B8）。
const MAX_MIGRATION_CONFIG_SCRIPT_BYTES: usize = 512 * 1024;

/// 按字节上限安全截断字符串（不切断 UTF-8 字符边界）。
fn truncate_utf8(value: &mut String, limit: usize) {
    if value.len() <= limit {
        return;
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

/// 对内存中的 `AppProfile` 做幂等的旧数据归一化（B9 保守实现）。
/// 不新增字段、不改变结构体形状、不改任何 `pub fn` 签名；重复调用结果一致，
/// 且对已是当前版本的合法数据完全 no-op。每条规则注明它修复的历史行为。
pub fn migrate_app_profile(profile: &mut AppProfile) {
    // v0/v1（`default_profile_version()==1`）：导入失败会留下 source 为空的
    // 占位订阅，早期 UI 又允许保存 tag 为空的节点；它们会让连接入口拿到空
    // 地址、渲染出空白行。清理「数据本身为空」的项，不动合法内容。
    profile
        .subscriptions
        .retain(|subscription| !subscription.source.trim().is_empty());
    for subscription in &mut profile.subscriptions {
        subscription
            .nodes
            .retain(|node| !node.tag.trim().is_empty());
    }

    // v1：`tunnel_mode` 曾以自由文本持久化并出现过已废弃的模式取值。当前
    // `TunnelMode` 是 Rule/Global/Direct 三态枚举，serde 在反序列化阶段就拒绝
    // 未知字符串，能进入这里的必是合法值，因此无需运行时映射，显式保持原值。
    // `dark_mode` / `tun_enabled` / `allow_lan` 等 bool 同样保持原值，绝不重置。

    // v1/v2：自定义规则与配置脚本曾无长度/数量上限，旧 profile 可能写入超大
    // 字段。按现有常量收敛：`MAX_CUSTOM_RULES` 限条数、`MAX_CUSTOM_RULE_VALUE_BYTES`
    // 限每条 value、512 KiB 限脚本；幂等，且对合规数据 no-op。
    profile.custom_rules.truncate(MAX_CUSTOM_RULES);
    for rule in &mut profile.custom_rules {
        truncate_utf8(&mut rule.value, MAX_CUSTOM_RULE_VALUE_BYTES);
    }
    truncate_utf8(
        &mut profile.config_script,
        MAX_MIGRATION_CONFIG_SCRIPT_BYTES,
    );

    // v2：切换/刷新/删除订阅会重写 namespaced 节点 tag，旧 group_selections
    // 被整体 clone，残留项会随每次 ConnectionRequest 下发（F18）。与 UI 层
    // `prune_stale_group_selections` 同语义：只清理形如 `subscription-*` 且已
    // 不存在的节点选择，内置组名 / `direct` 等非节点选择一律保留。
    let live_tags: HashSet<String> = profile
        .subscriptions
        .iter()
        .flat_map(|subscription| subscription.nodes.iter())
        .map(|node| node.tag.clone())
        .collect();
    profile.group_selections.retain(|_, outbound| {
        !outbound.starts_with("subscription-") || live_tags.contains(outbound.as_str())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deterministic_test_support::Lcg;

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
    fn sync_snapshot_carries_parsed_nodes_and_rules() {
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
            dns_overrides: vec![DnsOverride {
                id: 1,
                enabled: true,
                domain_suffix: "example.net".to_string(),
                server: "192.0.2.53".to_string(),
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
        // F6: remote snapshots now carry the last parsed nodes so a pull can be
        // used immediately instead of forcing a manual refresh of every source.
        assert_eq!(target.subscriptions[0].nodes, source.subscriptions[0].nodes);
        assert_eq!(
            target.subscriptions[0].proxy_server_nameservers,
            source.subscriptions[0].proxy_server_nameservers
        );
        assert_eq!(target.subscriptions[0].rejected_count, 2);
        assert_eq!(target.active_subscription_id, None);
        assert!(target.selected_tag.is_empty());
        assert_eq!(target.custom_rules, source.custom_rules);
        assert_eq!(target.dns_overrides, source.dns_overrides);
        assert!(target.group_selections.is_empty());
        assert!(!target.dark_mode);
        assert!(!target.tun_enabled);
        assert!(!target.allow_lan);

        let serialized = serde_json::to_value(&snapshot).expect("snapshot should serialize");
        assert!(serialized.get("active_subscription_id").is_none());
        assert!(serialized.get("selected_tag").is_none());
        assert!(serialized.get("group_selections").is_none());
        assert_eq!(
            serialized["subscriptions"][0]["nodes"]
                .as_array()
                .map(|nodes| nodes.len()),
            Some(1),
            "the snapshot should carry the last parsed nodes (F6)"
        );
    }

    #[test]
    fn snapshot_without_a_dns_section_preserves_local_rules() {
        // Every remote document uploaded before DNS overrides shipped looks like
        // this. It must decode, and applying it must leave the local rules alone:
        // treating "absent" as "empty" deleted the user's rules on pull.
        let legacy = r#"{
            "format": "kitty-pro-sync",
            "version": 2,
            "updated_at": 7,
            "subscriptions": [],
            "custom_rules": []
        }"#;
        let snapshot: SyncSnapshot =
            serde_json::from_str(legacy).expect("legacy snapshot should decode");
        assert!(snapshot.dns_overrides.is_none());
        assert!(snapshot.config_script.is_none());

        let mut profile = AppProfile::default();
        profile.dns_overrides.push(DnsOverride {
            id: 1,
            enabled: true,
            domain_suffix: "example.net".to_string(),
            server: "192.0.2.53".to_string(),
        });
        profile.config_script = "function main(c) { return c; }".to_string();
        profile.config_script_enabled = true;
        snapshot.apply_to_profile(&mut profile);
        assert_eq!(profile.dns_overrides.len(), 1);
        assert!(profile.config_script_enabled);
        assert_eq!(profile.config_script, "function main(c) { return c; }");
    }

    #[test]
    fn explicit_empty_dns_section_clears_local_rules() {
        // The opposite case: a modern remote that really has no rules must clear
        // them locally, otherwise a deletion could never sync down.
        let modern = r#"{
            "format": "kitty-pro-sync",
            "version": 2,
            "updated_at": 7,
            "subscriptions": [],
            "custom_rules": [],
            "dns_overrides": [],
            "config_script": { "enabled": false, "script": "" }
        }"#;
        let snapshot: SyncSnapshot = serde_json::from_str(modern).expect("modern snapshot");
        let mut profile = AppProfile {
            dns_overrides: vec![DnsOverride {
                id: 1,
                enabled: true,
                domain_suffix: "example.net".to_string(),
                server: "192.0.2.53".to_string(),
            }],
            config_script_enabled: true,
            config_script: "stale".to_string(),
            ..AppProfile::default()
        };
        snapshot.apply_to_profile(&mut profile);
        assert!(profile.dns_overrides.is_empty());
        assert!(!profile.config_script_enabled);
        assert!(profile.config_script.is_empty());
    }

    #[test]
    fn snapshot_carries_dns_rules_and_the_config_script() {
        let source = AppProfile {
            dns_overrides: vec![DnsOverride {
                id: 4,
                enabled: false,
                domain_suffix: "example.net".to_string(),
                server: "192.0.2.53".to_string(),
            }],
            config_script_enabled: true,
            config_script: "function main(c) { return c; }".to_string(),
            ..AppProfile::default()
        };
        let snapshot = SyncSnapshot::from_profile(&source, 3);
        assert_eq!(snapshot.dns_overrides, Some(source.dns_overrides.clone()));
        assert_eq!(
            snapshot.config_script,
            Some(SyncConfigScript {
                enabled: true,
                script: "function main(c) { return c; }".to_string(),
            })
        );

        let mut target = AppProfile::default();
        snapshot.apply_to_profile(&mut target);
        assert_eq!(target.dns_overrides, source.dns_overrides);
        assert!(target.config_script_enabled);
        assert_eq!(target.config_script, source.config_script);
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
                "server": "198.51.100.20",
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

    // ===== B9：AppProfile 版本迁移 =====

    #[test]
    fn profile_schema_version_reads_optional_schema_version() {
        assert_eq!(profile_schema_version(&serde_json::json!({})), 0);
        assert_eq!(
            profile_schema_version(&serde_json::json!({ "schema_version": 0 })),
            0
        );
        assert_eq!(
            profile_schema_version(&serde_json::json!({ "schema_version": 2 })),
            2
        );
        assert_eq!(
            profile_schema_version(&serde_json::json!({
                "schema_version": CURRENT_PROFILE_SCHEMA
            })),
            CURRENT_PROFILE_SCHEMA
        );
        // 非法/字符串/负数/溢出/非对象一律回退到 0（最旧版本）。
        assert_eq!(
            profile_schema_version(&serde_json::json!({ "schema_version": "3" })),
            0
        );
        assert_eq!(
            profile_schema_version(&serde_json::json!({ "schema_version": -1 })),
            0
        );
        assert_eq!(
            profile_schema_version(&serde_json::json!({ "schema_version": u64::MAX })),
            0
        );
        assert_eq!(profile_schema_version(&serde_json::json!(null)), 0);
    }

    #[test]
    fn migrate_app_profile_normalizes_legacy_data() {
        let long_value = "x".repeat(MAX_CUSTOM_RULE_VALUE_BYTES + 200);
        let long_script = "a".repeat(MAX_MIGRATION_CONFIG_SCRIPT_BYTES + 1024);
        let legacy = serde_json::json!({
            "version": 1,
            "subscriptions": [
                { "id": 1, "name": "empty", "source": "", "nodes": [] },
                {
                    "id": 2, "name": "dirty", "source": "https://example.com/sub",
                    "nodes": [
                        { "tag": "", "name": "blank", "protocol": "vless", "server": "a.example.com", "port": 443, "auth": {"kind":"none"} },
                        { "tag": "subscription-2-ok", "name": "ok", "protocol": "vless", "server": "b.example.com", "port": 443, "auth": {"kind":"none"} }
                    ]
                }
            ],
            "custom_rules": [{ "id": 1, "match_type": "domain_keyword", "value": long_value, "action": "proxy" }],
            "config_script": long_script,
            "dark_mode": true,
            "tun_enabled": true,
            "allow_lan": true,
            "tunnel_mode": "global",
            "group_selections": {
                "proxy": "subscription-2-ok",
                "fallback": "subscription-99-ghost"
            }
        });
        let mut profile: AppProfile =
            serde_json::from_value(legacy).expect("legacy profile should decode");
        migrate_app_profile(&mut profile);

        // 空 source 订阅被清理、空 tag 节点被清理、合法节点保留。
        assert_eq!(profile.subscriptions.len(), 1);
        assert_eq!(profile.subscriptions[0].nodes.len(), 1);
        assert_eq!(profile.subscriptions[0].nodes[0].tag, "subscription-2-ok");
        // 超长字段被截断到既有上限。
        assert!(profile.custom_rules[0].value.len() <= MAX_CUSTOM_RULE_VALUE_BYTES);
        assert!(profile.config_script.len() <= MAX_MIGRATION_CONFIG_SCRIPT_BYTES);
        // 悬空 namespaced 选择被清理，指向现存节点的选择保留。
        assert_eq!(
            profile.group_selections.get("proxy").map(String::as_str),
            Some("subscription-2-ok")
        );
        assert!(!profile.group_selections.contains_key("fallback"));
        // bool 与 tunnel_mode 保持原值（绝不重置）。
        assert!(profile.dark_mode && profile.tun_enabled && profile.allow_lan);
        assert_eq!(profile.tunnel_mode, TunnelMode::Global);
    }

    #[test]
    fn migrate_app_profile_is_idempotent() {
        let long_value = "y".repeat(MAX_CUSTOM_RULE_VALUE_BYTES + 300);
        let long_script = "b".repeat(MAX_MIGRATION_CONFIG_SCRIPT_BYTES + 256);
        let legacy = serde_json::json!({
            "subscriptions": [
                { "id": 1, "name": "s", "source": "", "nodes": [] },
                { "id": 2, "name": "t", "source": "https://example.com/sub",
                  "nodes": [ { "tag": "", "name": "x", "protocol": "vless", "server": "a", "port": 443, "auth": {"kind":"none"} } ] }
            ],
            "custom_rules": [{ "id": 1, "match_type": "domain_keyword", "value": long_value, "action": "proxy" }],
            "config_script": long_script,
            "group_selections": { "proxy": "subscription-7-ghost" }
        });
        let mut profile: AppProfile =
            serde_json::from_value(legacy).expect("legacy profile should decode");
        migrate_app_profile(&mut profile);
        let once = profile.clone();
        migrate_app_profile(&mut profile);
        assert_eq!(profile, once, "迁移必须幂等：第二次调用不得改变任何字段");
    }

    #[test]
    fn migrate_app_profile_leaves_current_data_unchanged() {
        let node = ProxyNode {
            tag: "subscription-7-edge".to_string(),
            name: "Edge".to_string(),
            protocol: ProxyProtocol::Vless,
            server: "edge.example.com".to_string(),
            port: 443,
            auth: ProxyAuth::None,
            transport: TransportOptions::default(),
            tls: TlsOptions::default(),
            hysteria2: None,
        };
        let profile = AppProfile {
            subscriptions: vec![Subscription {
                id: 7,
                name: "Primary".to_string(),
                source: "https://example.com/sub".to_string(),
                nodes: vec![node],
                proxy_server_nameservers: Vec::new(),
                rejected_count: 0,
            }],
            active_subscription_id: Some(7),
            selected_tag: "subscription-7-edge".to_string(),
            tunnel_mode: TunnelMode::Global,
            tun_enabled: true,
            allow_lan: true,
            dark_mode: true,
            custom_rules: vec![CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::DomainSuffix,
                value: "example.com".to_string(),
                action: CustomRuleAction::Direct,
            }],
            config_script: "function main(c) { return c; }".to_string(),
            config_script_enabled: true,
            group_selections: HashMap::from([(
                "proxy".to_string(),
                "subscription-7-edge".to_string(),
            )]),
            ..AppProfile::default()
        };
        let mut migrated = profile.clone();
        migrate_app_profile(&mut migrated);
        assert_eq!(migrated, profile, "当前版本的合法数据不应被迁移改动");
    }

    // ===== T3：确定性生成式测试（纯标准库 LCG，不引入 rand/proptest）=====

    #[test]
    fn generated_profiles_and_snapshots_round_trip() {
        const SEED: u64 = 0x4d4f_4445_4c53_5254; // "MODELSRT"
        let mut lcg = Lcg::new(SEED);
        for iteration in 0..150 {
            // 生成「空 / 满 / 超长 / 特殊字符」组合的字段。
            let tag = format!("subscription-{}-{}", lcg.next() % 8, lcg.text(6));
            let node = ProxyNode {
                tag: tag.clone(),
                name: lcg.text(10),
                protocol: ProxyProtocol::Vless,
                server: lcg.text(12),
                port: 443,
                auth: ProxyAuth::None,
                transport: TransportOptions::default(),
                tls: TlsOptions::default(),
                hysteria2: None,
            };
            let script_len = match lcg.next() % 4 {
                0 => 0,
                1 => 16,
                2 => 2048, // 超长
                _ => 5,
            };
            let mut profile = AppProfile {
                subscriptions: vec![Subscription {
                    id: lcg.next(),
                    name: lcg.text(8),
                    source: lcg.text(20),
                    nodes: vec![node],
                    proxy_server_nameservers: vec![lcg.text(8)],
                    rejected_count: (lcg.next() % 8) as usize,
                }],
                selected_tag: tag.clone(),
                config_script: lcg.text(script_len),
                dark_mode: lcg.next().is_multiple_of(2),
                tun_enabled: lcg.next().is_multiple_of(2),
                allow_lan: lcg.next().is_multiple_of(2),
                group_selections: HashMap::from([("proxy".to_string(), tag)]),
                ..AppProfile::default()
            };
            profile.dns_overrides = if lcg.next().is_multiple_of(2) {
                Vec::new()
            } else {
                vec![DnsOverride {
                    id: 1,
                    enabled: true,
                    domain_suffix: lcg.text(8),
                    server: lcg.text(8),
                }]
            };

            let profile_bytes = serde_json::to_vec(&profile).unwrap_or_else(|error| {
                panic!("seed={SEED:#x} iteration={iteration}: profile 序列化失败: {error}")
            });
            let restored: AppProfile =
                serde_json::from_slice(&profile_bytes).unwrap_or_else(|error| {
                    panic!("seed={SEED:#x} iteration={iteration}: profile 反序列化失败: {error}")
                });
            assert_eq!(
                restored, profile,
                "seed={SEED:#x} iteration={iteration}: AppProfile serde 往返必须一致"
            );

            let mut snapshot = SyncSnapshot::from_profile(&profile, lcg.next());
            if lcg.next().is_multiple_of(2) {
                snapshot.dns_overrides = None;
                snapshot.config_script = None;
            }
            let snapshot_bytes = serde_json::to_vec(&snapshot).unwrap_or_else(|error| {
                panic!("seed={SEED:#x} iteration={iteration}: snapshot 序列化失败: {error}")
            });
            let restored_snapshot: SyncSnapshot = serde_json::from_slice(&snapshot_bytes)
                .unwrap_or_else(|error| {
                    panic!("seed={SEED:#x} iteration={iteration}: snapshot 反序列化失败: {error}")
                });
            assert_eq!(
                restored_snapshot, snapshot,
                "seed={SEED:#x} iteration={iteration}: SyncSnapshot serde 往返必须一致"
            );
        }
    }

    #[test]
    fn generated_legacy_snapshots_with_missing_fields_decode() {
        const SEED: u64 = 0x4c45_4741_4359_534e; // "LEGACYSN"
        let mut lcg = Lcg::new(SEED);
        for iteration in 0..100 {
            // 模拟 v1 快照：subscription 缺 nodes，顶层缺 dns_overrides /
            // config_script，再随机省略 updated_at / subscriptions / custom_rules。
            let mut doc = serde_json::json!({
                "format": SYNC_SNAPSHOT_FORMAT,
                "version": lcg.next() % 3 + 1,
                "updated_at": lcg.next(),
                "subscriptions": [{
                    "id": 1,
                    "name": lcg.text(8),
                    "source": "https://example.com/sub",
                    "proxy_server_nameservers": [],
                    "rejected_count": 0
                }],
                "custom_rules": []
            });
            let object = doc
                .as_object_mut()
                .expect("legacy snapshot must be an object");
            for key in ["updated_at", "subscriptions", "custom_rules"] {
                if lcg.next().is_multiple_of(2) {
                    object.remove(key);
                }
            }
            let snapshot: SyncSnapshot = serde_json::from_value(doc).unwrap_or_else(|error| {
                panic!("seed={SEED:#x} iteration={iteration}: 旧版快照应可解码: {error}")
            });
            // 缺省字段全部由 #[serde(default)] 兜底。
            assert!(snapshot.dns_overrides.is_none());
            assert!(snapshot.config_script.is_none());
            assert!(snapshot
                .subscriptions
                .iter()
                .all(|subscription| subscription.nodes.is_empty()));
        }
    }

    #[test]
    fn custom_rules_validation_survives_serde_round_trip() {
        const SEED: u64 = 0x4355_5354_4f4d_5255; // "CUSTOMRU"
        let mut lcg = Lcg::new(SEED);
        for iteration in 0..100 {
            // 每组随机选一种非法规则形态，往返后校验必须仍然拒绝。
            let variant = iteration % 4;
            let mut rules = vec![CustomRule {
                id: 1,
                enabled: true,
                match_type: CustomRuleMatch::DomainKeyword,
                value: "example".to_string(),
                action: CustomRuleAction::Proxy,
            }];
            match variant {
                0 => rules[0].id = 0,        // InvalidId
                1 => rules[0].value.clear(), // Empty
                2 => {
                    let duplicate = rules[0].clone();
                    rules.push(duplicate); // InvalidId（重复）
                }
                _ => rules[0].value = lcg.text(MAX_CUSTOM_RULE_VALUE_BYTES + 64), // TooLong
            }
            let profile = AppProfile {
                custom_rules: rules,
                ..AppProfile::default()
            };
            let bytes = serde_json::to_vec(&profile).unwrap_or_else(|error| {
                panic!("seed={SEED:#x} iteration={iteration}: 序列化失败: {error}")
            });
            let restored: AppProfile = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                panic!("seed={SEED:#x} iteration={iteration}: 反序列化失败: {error}")
            });
            assert!(
                validate_custom_rules(&restored.custom_rules).is_err(),
                "seed={SEED:#x} iteration={iteration} variant={variant}: 往返后 custom_rules 校验仍应拒绝非法规则"
            );
            // 对照组：合法规则往返后必须通过校验。
            let valid = AppProfile {
                custom_rules: vec![CustomRule {
                    id: 9,
                    enabled: true,
                    match_type: CustomRuleMatch::DomainSuffix,
                    value: "example.com".to_string(),
                    action: CustomRuleAction::Direct,
                }],
                ..AppProfile::default()
            };
            let valid_bytes = serde_json::to_vec(&valid).expect("serialize");
            let restored_valid: AppProfile =
                serde_json::from_slice(&valid_bytes).expect("deserialize");
            assert!(validate_custom_rules(&restored_valid.custom_rules).is_ok());
        }
    }
}
