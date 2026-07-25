//! Shared proxy models, subscription parsing, and sing-box config generation.

mod model;
mod parser;
mod singbox_config;

pub use model::*;
pub use parser::{parse_share_link, parse_subscription};
pub use singbox_config::{build_singbox_config, SingBoxOptions};
