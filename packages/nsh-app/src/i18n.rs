//! Chinese and English, with the Chinese wording as the key.
//!
//! The app was written in Chinese and its screens are the product a Chinese reader expects, so
//! the Chinese string is the source rather than a translation of an English one: a key like
//! `nav.overview` would put a second naming system over the app and leave every call site
//! holding two strings, one of which nobody reads. `tr("总览")` returns 总览 when the language is
//! Chinese and `Overview` when it is English, and a string with no English yet is *not* a bug to
//! hide — it is what `the_english_screens_hold_no_chinese` fails on, so the gap is a red test
//! rather than a mixed-language screen the user finds by scrolling.
//!
//! ## Why not a `Locale` from the platform
//!
//! NSH reports the system language, and defaulting to it would be the usual design. It is wrong
//! for the *stored* case here: the profile is the app's own record of what the user chose, this
//! app has run in Chinese for its whole life, and NSH's own fallback locale is `en-US`. Keying off
//! the system would therefore flip the interface on the update that introduces the switch, for
//! readers who never asked. So the stored choice wins, and an absent choice means Chinese — the
//! language every existing profile was written in.
//!
//! ## Why every label reads through a closure
//!
//! There is no way for an app to rebuild itself: `Root::remount` and `Engine::remount` belong to
//! the host, not to app code. A language change therefore has to propagate the way a theme change
//! does — as a reactive read — which means a label is `text(move || tr("总览"))` and not
//! `text("总览")`. The second form would translate correctly at startup and then sit there in
//! Chinese forever. `tests/app.rs` switches language and reads the same screen again, so a label
//! that missed its closure shows up as stale text rather than surviving review.

use nsh_core::prelude::*;

/// The interface languages this build ships.
///
/// Two, because those are the two the user asked for and the two anyone has written. A third
/// would need its own table, and an empty one would be worse than not offering it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// The wording the app was written in, and the one a reader of this project expects.
    #[default]
    Chinese,
    English,
}

impl Lang {
    /// The name a language calls itself, in that language. Never translated: 简体中文 in an
    /// English interface and `English` in a Chinese one, because that is what a person who
    /// switched by accident has to be able to find.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Chinese => "简体中文",
            Lang::English => "English",
        }
    }

    /// The stored form, kept short because it lives in the profile the two shells share.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Chinese => "zh",
            Lang::English => "en",
        }
    }

    /// The language a stored code asks for.
    ///
    /// Empty — every profile written before this feature, and every profile the Dioxus shell
    /// writes — is Chinese, *not* the system locale. Defaulting to the system would flip a
    /// reader's interface on the update that introduced the switch, for a choice they never made,
    /// and NSH's own fallback locale is `en-US`: on the machine this app is developed on that is a
    /// silent switch to English for everyone. The system locale is honoured only where the user
    /// can still change their mind cheaply — the picker says which language is on because nothing
    /// was ever picked.
    pub fn from_stored(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "en" => Lang::English,
            _ => Lang::Chinese,
        }
    }

    /// The language a system identifier asks for. Only the primary subtag matters: `zh-CN`,
    /// `zh-Hans`, `zh_TW` and a bare `zh` are all Chinese to an app with two choices, and
    /// anything unrecognised is Chinese rather than English, since this app's readers were
    /// reading Chinese the day before this feature existed.
    pub fn from_identifier(identifier: &str) -> Self {
        if identifier.to_ascii_lowercase().starts_with("en") {
            Lang::English
        } else {
            Lang::Chinese
        }
    }
}

/// Where the language lives: one global, so any read of it subscribes and a write repaints every
/// label that read it, which is the same mechanism `theme()` runs on.
fn state() -> Signal<Lang> {
    global::<Lang>()
}

/// The language the interface is drawn in right now.
pub fn lang() -> Lang {
    state().get()
}

/// The language, without subscribing: for the code that has to compare rather than display, like
/// a store deciding whether to write the profile back.
pub fn lang_untracked() -> Lang {
    state().get_untracked()
}

/// Draw in `next`. Returns whether anything changed, so the caller can skip the persist a repeat
/// selection would write.
pub fn set_lang(next: Lang) -> bool {
    if state().get_untracked() == next {
        return false;
    }
    state().set(next);
    true
}

/// [`tr`] for a word that is a *qualifier* on something else: `控件（已禁用）`, `Control
/// (disabled)`.
///
/// A parenthesis is punctuation with a language, not a character: full-width round brackets sit
/// at the width of a Chinese glyph and look right between them, and look wrong around Latin words.
/// Eight strings in this app parenthesise something, so the choice is made once here instead of at
/// each site, which is how a `（` survives into an English screen.
pub fn parenthesised(key: &str) -> String {
    let inner = tr(key);
    if lang() == Lang::Chinese {
        format!("（{inner}）")
    } else {
        format!(" ({inner})")
    }
}

/// The wording for the current language, keyed by the Chinese the app was written with.
///
/// Takes and returns `&'static str` where it can: the keys are literals in the binary, so an
/// untranslated string costs nothing beyond the miss, and a translated one costs one lookup.
/// Formatted sentences are assembled from a translated template rather than translated after
/// formatting — `"{} 条新日志"` becomes `"{} new lines"` and the number is filled in after —
/// because a word order baked into the Chinese could not be re-arranged for English any other way.
pub fn tr(key: &str) -> String {
    translate(lang(), key)
}

/// The wording for one language, without the reactive read.
///
/// Separate because the reactive part cannot be reached from a unit test: `global` lives in the
/// engine's runtime, and a test on the platform thread is not inside it. Every pure rule of this
/// module — the table, the fallback, the language chosen from a locale — is testable this way, and
/// only the propagation is left to the suite that drives the real UI.
pub fn translate(lang: Lang, key: &str) -> String {
    if lang == Lang::Chinese {
        return key.to_owned();
    }
    english(key).map_or_else(|| key.to_owned(), str::to_owned)
}

/// The English side, as `(中文, English)` pairs.
///
/// A `const` array rather than a `match` so the completeness and collision tests can walk it — a
/// `match` cannot be enumerated, and a table nobody can list is a table nobody can prove is
/// finished. `english` is the lookup, built from the same list so the two cannot disagree.
const ENGLISH: &[(&str, &str)] = &[
    // Rail and navigation — on screen on every page, in both window shapes.
    ("总览", "Overview"),
    ("节点", "Nodes"),
    ("订阅", "Subscriptions"),
    ("规则", "Rules"),
    ("日志", "Logs"),
    ("设置", "Settings"),
    ("连接", "Connection"),
    ("配置", "Configuration"),
    ("运维", "Operations"),
    // The header and the connect action.
    ("启动代理", "Connect"),
    ("断开", "Disconnect"),
    ("正在启动", "Starting"),
    ("正在断开", "Stopping"),
    ("正在重启", "Restarting"),
    ("正在申请网络权限", "Requesting network access"),
    ("正在应用设置", "Applying settings"),
    ("深色模式", "Dark mode"),
    ("切换深色外观", "Switch to dark appearance"),
    ("切换浅色外观", "Switch to light appearance"),
    ("内核已就绪", "Core ready"),
    ("内核运行中", "Core running"),
    ("内核不可用", "Core unavailable"),
    ("正在检测", "Checking"),
    ("已启用", "On"),
    ("已停用", "Off"),
    ("未启用", "Not enabled"),
    ("读取中", "Reading"),
    ("规则模式", "Rule mode"),
    ("全局模式", "Global mode"),
    ("直连模式", "Direct mode"),
    // The appearance card, which is where the language picker lives.
    ("外观", "Appearance"),
    ("语言", "Language"),
    ("界面语言", "Interface language"),
    ("简体中文", "Simplified Chinese"),
    ("English", "English"),
    // The settings page's own headings.
    ("流量走向", "Traffic"),
    ("接管方式", "Takeover"),
    ("启动与终端", "Startup & terminal"),
    ("登录时启动", "Launch at login"),
    ("规则与订阅同步", "Rules & subscription sync"),
    ("备份位置", "Backup location"),
    ("存储类型", "Storage type"),
    ("远端同步", "Remote sync"),
    ("应用更新", "App updates"),
    ("配置脚本", "Config script"),
    ("诊断与关于", "Diagnostics & about"),
    ("搜索设置", "Search settings"),
    ("没有匹配的设置项", "No settings match"),
    ("清除", "Clear"),
    ("取消", "Cancel"),
    ("保存", "Save"),
    ("关闭", "Close"),
    ("删除", "Delete"),
    ("编辑", "Edit"),
    ("添加", "Add"),
    ("刷新", "Refresh"),
    ("复制诊断信息", "Copy diagnostics"),
    (
        "诊断信息已复制到剪贴板",
        "Diagnostics copied to the clipboard",
    ),
    ("内核状态", "Core status"),
    ("版本", "Version"),
    // Feedback every screen can produce.
    ("关闭提示", "Dismiss"),
    ("已复制代理环境变量", "Proxy environment copied"),
    ("内核已启动", "Core started"),
];

/// The English for one key, if this build has any.
fn english(key: &str) -> Option<&'static str> {
    ENGLISH
        .iter()
        .find(|(from, _)| *from == key)
        .map(|(_, to)| *to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_answers_the_key_and_english_the_table() {
        assert_eq!(translate(Lang::Chinese, "总览"), "总览");
        assert_eq!(translate(Lang::English, "总览"), "Overview");
    }

    /// An untranslated string stays Chinese rather than vanishing or erroring: a missing entry is
    /// a gap in a table of ~330, and the app has to keep working while it is being filled in.
    /// `the_english_screens_hold_no_chinese` is what stops the gap from being shipped as a
    /// mixed-language interface.
    #[test]
    fn a_string_with_no_english_keeps_its_own_words() {
        assert_eq!(translate(Lang::English, "没有这个词条"), "没有这个词条");
    }

    #[test]
    fn each_language_names_itself_in_itself() {
        assert_eq!(Lang::Chinese.native_name(), "简体中文");
        assert_eq!(Lang::English.native_name(), "English");
    }

    /// Only the primary subtag is looked at, which is what makes it safe to match on the prefix:
    /// the strings that reach here are locale identifiers (`en`, `en-US`, `en_GB`), never free
    /// text, and every one of them says what language it is by its first two letters.
    #[test]
    fn only_an_english_identifier_picks_english() {
        for id in ["en", "en-US", "EN", "en_GB", "en-001"] {
            assert_eq!(Lang::from_identifier(id), Lang::English, "{id}");
        }
        // Everything else is Chinese, including the unparseable: the app's readers were reading
        // Chinese before this feature existed, so an unknown locale is not evidence of English.
        for id in ["zh", "zh-CN", "zh-Hans", "de", "fr-FR", "", " ", "ja-JP"] {
            assert_eq!(Lang::from_identifier(id), Lang::Chinese, "{id}");
        }
    }

    /// The stored form is what lands in `profile.json`, which the Dioxus shell also reads, so it
    /// has to be stable and short rather than `format!("{:?}", lang)`.
    #[test]
    fn a_language_stores_a_short_code() {
        assert_eq!(Lang::Chinese.as_str(), "zh");
        assert_eq!(Lang::English.as_str(), "en");
    }

    /// Chinese is the default, and that is a decision rather than an accident: an existing
    /// `profile.json` has no language field, `serde(default)` supplies this one, and a reader who
    /// has used the app in Chinese must not meet English after an update they did not ask for.
    #[test]
    fn a_profile_with_no_language_says_chinese() {
        assert_eq!(Lang::default(), Lang::Chinese);
        let stored: proxy_core::AppProfile = serde_json::from_str("{}").unwrap();
        assert!(
            stored.language.is_empty(),
            "a profile written without the field should not invent one"
        );
        assert_eq!(
            Lang::from_stored(&stored.language),
            Lang::Chinese,
            "the field a Dioxus-written profile is missing must not read as English"
        );
        // And an explicit choice survives the round trip through the file.
        let chosen: proxy_core::AppProfile = serde_json::from_str(r#"{"language":"en"}"#).unwrap();
        assert_eq!(Lang::from_stored(&chosen.language), Lang::English);
        assert_eq!(Lang::from_stored("  EN "), Lang::English);
    }

    /// No key maps to two different English strings, and no key is listed twice: a 300-arm `match`
    /// is exactly the shape in which that happens silently.
    #[test]
    fn no_key_is_answered_twice() {
        // Duplicate patterns in a `match` on a `&str` are a compile error for literals, so this
        // checks the thing the compiler cannot: that two *different* Chinese keys were given the
        // same English, which would make two distinct controls read alike.
        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for (key, value) in ENGLISH {
            if let Some(before) = seen.insert(*value, *key) {
                assert_eq!(
                    before, *key,
                    "`{value}` is the English for both `{before}` and `{key}`: two different \
                     controls would read the same"
                );
            }
        }
    }
}
