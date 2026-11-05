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

/// 确定性生成式测试的共享工具（仅 `cfg(test)` 编译，不进入生产产物）。
/// 纯标准库 LCG，不引入 rand / proptest / cargo-fuzz；同一 seed 必得同一序列，
/// 断言失败信息带上 seed 与迭代序号即可精确复现。
#[cfg(test)]
pub(crate) mod deterministic_test_support {
    #![allow(dead_code)]

    /// 64 位线性同余伪随机源：state = state * A + C（wrapping）。
    pub(crate) struct Lcg(pub(crate) u64);

    impl Lcg {
        pub(crate) fn new(seed: u64) -> Self {
            Self(seed)
        }

        pub(crate) fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }

        pub(crate) fn bytes(&mut self, n: usize) -> Vec<u8> {
            (0..n).map(|_| (self.next() >> 24) as u8).collect()
        }

        /// 由可打印 ASCII、引号、YAML/JS 元字符、控制字符与随机 Unicode 混合的
        /// 字符集生成确定性文本，专门喂给解析器 / 脚本 / serde 做抗压测试。
        pub(crate) fn text(&mut self, n: usize) -> String {
            const ALPHABET: &[char] = &[
                'a', 'b', 'c', 'x', 'y', 'z', '0', '1', '9', ' ', ' ', ':', '-', '_', '.', '/',
                '\\', '\'', '"', ';', '&', '*', '!', '#', '$', '(', ')', '[', ']', '{', '}', '\0',
                '\r', '\n', '\t', '中', '文', 'é', '🙂', '💡',
            ];
            (0..n)
                .map(|_| ALPHABET[(self.next() % ALPHABET.len() as u64) as usize])
                .collect()
        }
    }
}
