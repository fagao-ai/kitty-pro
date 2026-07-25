use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyProtocol {
    Hysteria2,
    Vmess,
    Vless,
    Trojan,
    Shadowsocks,
}

impl ProxyProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hysteria2 => "Hysteria2",
            Self::Vmess => "VMess",
            Self::Vless => "VLESS",
            Self::Trojan => "Trojan",
            Self::Shadowsocks => "Shadowsocks",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRequest {
    pub nodes: Vec<ProxyNode>,
    pub selected_tag: String,
    #[serde(default)]
    pub mode: TunnelMode,
    #[serde(default)]
    pub tun: bool,
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
    pub selected_tag: String,
    #[serde(default)]
    pub tunnel_mode: TunnelMode,
    #[serde(default)]
    pub tun_enabled: bool,
    #[serde(default)]
    pub dark_mode: bool,
}

impl Default for AppProfile {
    fn default() -> Self {
        Self {
            version: default_profile_version(),
            subscriptions: Vec::new(),
            selected_tag: String::new(),
            tunnel_mode: TunnelMode::Rule,
            tun_enabled: false,
            dark_mode: false,
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
            dark_mode: true,
            ..AppProfile::default()
        };

        let restored: AppProfile = serde_json::from_slice(
            &serde_json::to_vec(&profile).expect("profile should serialize"),
        )
        .expect("profile should deserialize");

        assert_eq!(restored, profile);
        assert_eq!(restored.subscriptions[0].node_count(), 1);
    }
}
