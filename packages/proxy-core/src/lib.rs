//! Shared proxy models, subscription parsing, and sing-box config generation.

mod model;
mod parser;
mod singbox_config;

pub use model::*;
pub use parser::{is_http_proxy_share_link, parse_share_link, parse_subscription};
pub use singbox_config::{
    build_singbox_config, RuleSetCachePaths, SingBoxOptions, CHINA_GEOIP_RULE_SET_URL,
    CHINA_GEOSITE_RULE_SET_URL,
};
