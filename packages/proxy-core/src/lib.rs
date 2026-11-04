//! Shared proxy models, subscription parsing, and sing-box config generation.

mod model;
mod parser;
#[cfg(not(target_arch = "wasm32"))]
mod script;
mod singbox_config;
mod update;

pub use model::*;
pub use parser::{is_http_proxy_share_link, parse_share_link, parse_subscription};
#[cfg(not(target_arch = "wasm32"))]
pub use script::{apply_config_script, ConfigScriptError};
pub use singbox_config::{
    apply_proxy_group_selections, build_singbox_config, extract_proxy_groups, RuleSetCachePaths,
    SingBoxOptions, CHINA_GEOIP_RULE_SET_URL, CHINA_GEOSITE_RULE_SET_URL,
};
// `asset_matches_target` and `arch_group` stay crate-internal: callers ask for the
// asset a target should take, never for one predicate at a time.
pub use update::{
    is_newer_version, select_asset, unix_from_rfc3339, GitHubRelease, GitHubReleaseAsset,
    ReleaseKind, ReleaseTarget, ReleaseVersion, UPDATE_CHECK_INTERVAL_SECS,
};
