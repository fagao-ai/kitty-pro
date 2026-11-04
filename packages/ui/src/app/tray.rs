//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopTrayCommand {
    SelectSubscription(u64),
    SelectNode(String),
    ToggleSystemProxy,
    CopyEnvironmentVariables,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopTraySubscription {
    pub id: u64,
    pub name: String,
    pub nodes: Vec<DesktopTrayNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopTrayNode {
    pub tag: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesktopTrayState {
    pub ready: bool,
    pub busy: bool,
    pub connected: bool,
    pub system_proxy_ready: bool,
    pub system_proxy_supported: bool,
    pub system_proxy_enabled: bool,
    pub active_subscription_id: Option<u64>,
    pub selected_tag: String,
    pub subscriptions: Vec<DesktopTraySubscription>,
}

#[derive(Clone, Copy, PartialEq)]
pub struct DesktopTrayBridge {
    pub state: Signal<DesktopTrayState>,
    pub command: Signal<Option<DesktopTrayCommand>>,
}
