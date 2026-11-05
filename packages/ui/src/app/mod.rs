//! Kitty Pro UI. Split from the original monolithic `app.rs` into
//! per-concern modules; the shared surface is re-exported here so every
//! sibling module works against one scope through `use super::*`.

mod chrome;
mod components;
mod diagnostics;
mod domain;
mod latency;
mod logs;
mod nodes;
mod overview;
mod proxy_app;
mod rules;
mod runtime;
mod settings;
mod subscriptions;
mod sync;
#[cfg(test)]
mod tests;
mod toast;
mod toggles;
mod tray;
mod types;
mod updates;

pub(crate) use chrome::*;
pub(crate) use components::*;
pub(crate) use diagnostics::*;
pub(crate) use domain::*;
pub(crate) use latency::*;
pub(crate) use logs::*;
pub(crate) use nodes::*;
pub(crate) use overview::*;
pub use proxy_app::ProxyApp;
pub(crate) use rules::*;
pub(crate) use runtime::*;
pub(crate) use settings::*;
pub(crate) use subscriptions::*;
pub(crate) use sync::*;
pub(crate) use toast::*;
pub(crate) use toggles::*;
pub use tray::{
    DesktopTrayBridge, DesktopTrayCommand, DesktopTrayNode, DesktopTrayState,
    DesktopTraySubscription,
};
pub(crate) use types::*;
pub(crate) use updates::*;

pub(crate) use api::{
    CoreLogEntry, CoreTraffic, NodeLatency, RouteDecision, RouteLogDetail, RouteTargetKind,
    SyncConfig, SyncProviderKind, SystemProxyStatus,
};
pub(crate) use dioxus::html::geometry::PixelsVector2D;
pub(crate) use dioxus::prelude::*;
pub(crate) use dioxus_free_icons::icons::ld_icons::{
    LdArrowDown, LdArrowDownFromLine, LdArrowUp, LdArrowUpRight, LdBan, LdBellOff, LdBellRing,
    LdCheck, LdChevronDown, LdChevronRight, LdCircleAlert, LdCircleCheck, LdClipboardCopy, LdCloud,
    LdCloudUpload, LdGauge, LdGitFork, LdGlobe, LdInfo, LdLanguages, LdLayers, LdLayoutDashboard,
    LdListFilter, LdMoon, LdNetwork, LdPackage, LdPause, LdPencil, LdPlay, LdPlus, LdPower,
    LdRadioTower, LdRefreshCw, LdRocket, LdRoute, LdRss, LdSave, LdScrollText, LdSearch,
    LdShieldCheck, LdSlidersHorizontal, LdSplit, LdSun, LdTerminal, LdTimer, LdTrash2, LdWifi, LdX,
};
pub(crate) use dioxus_free_icons::Icon;
pub(crate) use proxy_core::{
    validate_custom_rules, AppProfile, ConnectionRequest, CustomRule, CustomRuleAction,
    CustomRuleMatch, DnsOverride, ParseReport, ProxyGroup, ProxyGroupKind, ProxyNode,
    ProxyProtocol, Subscription, SyncConfigScript, SyncSnapshot, SyncSubscription, TunnelMode,
    MAX_CUSTOM_RULES, SYNC_SNAPSHOT_VERSION,
};
pub(crate) use std::collections::HashMap;
pub(crate) use std::rc::Rc;

pub(crate) use crate::APP_CSS;
