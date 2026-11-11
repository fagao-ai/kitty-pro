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
//! ## How a change reaches a label
//!
//! There is no way for an app to rebuild itself: `Root::remount` and `Engine::remount` belong to
//! the host, not to app code. But a `dynamic` region re-runs its closure when a signal read
//! *inside that closure* changes, and re-running it builds its subtree again — so
//! `shell::screen_body` reads `lang()` next to the current screen, and every page is rebuilt in
//! the new language when it changes. That is why `text("节点")` is enough on a page: ~450 call
//! sites did not have to become closures.
//!
//! Two places are outside that region and do need the closure form, `text(move || tr("总览"))`:
//! the rail and header, which are built by `root()` and survive a language change. A label there
//! written without a closure translates correctly at startup and then sits in the first language
//! forever — which happened twice while this was being written, and `tests/app.rs` names both by
//! walking every screen after a switch.

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

/// A sentence with holes: translate the *template*, then fill it.
///
/// `format!("刷新 {}", name)` cannot be a table key — the string reaching `tr` would be
/// `刷新 实验室`, which no table can hold. Translating the whole formatted result would need a key
/// per value; translating only the verb would bake the Chinese word order into every language,
/// since `{} 个节点` puts the count before the noun and English puts it after. So the template is
/// the key and the holes are filled after: `fill("刷新 {}", &[&name])` and
/// `fill("{} 个节点", &[&count])` become `Refresh …` and `… nodes`.
///
/// Holes are positional. A template written with named captures (`"共 {nodes} 个节点"`) has them
/// flattened at the call site, because the names are a convenience for the reader of the Chinese
/// and the English sentence has no reason to keep the same order or the same number of holes.
pub fn fill(key: &str, holes: &[&dyn std::fmt::Display]) -> String {
    fill_at(lang(), key, holes)
}

/// [`fill`] with the language named, so the hole arithmetic is testable without a reactive
/// runtime (`global` needs the engine's thread, which a unit test is not on).
pub fn fill_at(lang: Lang, key: &str, holes: &[&dyn std::fmt::Display]) -> String {
    let template = translate(lang, key);
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template.as_str();
    let mut holes = holes.iter();
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        // A template with more holes than the caller passed keeps the hole rather than dropping
        // it: a visible `{}` is a bug someone can see and report, a silently deleted word is one
        // nobody notices.
        match holes.next() {
            Some(value) => out.push_str(&value.to_string()),
            None => out.push_str("{}"),
        }
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
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

/// Put already-translated words in the brackets this language uses.
///
/// [`parenthesised`] takes a *key*; this takes text that has been through the table already — a
/// filled template, a computed note — which is the common case once a sentence is built from
/// parts. Both exist because a full-width bracket reads correctly between Chinese glyphs and
/// wrong around Latin ones.
pub fn in_parens(words: &str) -> String {
    in_parens_at(lang(), words)
}

/// [`in_parens`] with the language named, for `domain`'s pure sentence-builders.
pub fn in_parens_at(lang: Lang, words: &str) -> String {
    if lang == Lang::Chinese {
        format!("（{words}）")
    } else {
        format!(" ({words})")
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
    ("正在检测", "Detecting"),
    ("已启用", "On"),
    ("已停用", "Off"),
    ("未启用", "Not enabled"),
    ("读取中", "Loading"),
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
    // Overview.
    ("未连接", "Not connected"),
    ("空闲", "Idle"),
    ("内核未启动", "Core stopped"),
    ("可用节点", "Nodes available"),
    ("没有可用节点", "No nodes available"),
    ("活动连接", "Active connections"),
    ("近一分钟流量", "Traffic, last minute"),
    ("运行时长", "Uptime"),
    ("节点延迟", "Node latency"),
    ("系统代理", "System proxy"),
    ("测试连通", "Test connectivity"),
    ("先启动代理，才能探测出口", "Connect first to test the exit"),
    ("等待导入订阅", "Waiting for a subscription"),
    ("下载", "Download"),
    ("上传", "Upload"),
    // Nodes.
    ("没有匹配的节点", "No nodes match"),
    (
        "换个关键词，或先刷新订阅。",
        "Try another word, or refresh a subscription first.",
    ),
    (
        "还没有节点，先在订阅页导入",
        "No nodes yet — import one on the Subscriptions page",
    ),
    // Subscriptions.
    ("暂无订阅", "No subscriptions yet"),
    ("还没有订阅", "No subscriptions"),
    (
        "订阅用于导入代理节点；添加后即可在节点页挑选线路、回到概览连接。",
        "Subscriptions import proxy nodes. Add one, pick a line on the Nodes page, then connect \
         from the Overview.",
    ),
    // Rules.
    ("分流规则", "Routing rules"),
    ("专用 DNS", "Dedicated DNS"),
    ("暂无自定义规则", "No custom rules yet"),
    ("更新规则", "Update rule sets"),
    // Logs.
    ("全部路由", "All routes"),
    ("代理", "Proxied"),
    ("直连", "Direct"),
    ("拦截", "Blocked"),
    ("内核日志", "Core log"),
    ("时间", "Time"),
    ("动作", "Action"),
    ("目标", "Target"),
    ("出口节点", "Exit node"),
    ("命中链路", "Matched chain"),
    ("消息", "Message"),
    ("清空日志", "Clear log"),
    ("已停止", "Stopped"),
    // Settings: takeover and mode rows.
    ("TUN 模式", "TUN mode"),
    (
        "仅接管走代理端口的流量",
        "Takes over only traffic on the proxy port",
    ),
    ("允许局域网连接", "Allow devices on the network"),
    ("只监听本机", "Listens on this device only"),
    ("国内直连，其余走代理", "Domestic direct, the rest proxied"),
    ("全部流量走代理", "All traffic through the proxy"),
    ("全部直连，仅本机监听", "All direct, listening locally"),
    ("启用", "Enable"),
    // Settings: sync card.
    ("WebDAV 地址", "WebDAV address"),
    ("S3 兼容", "S3 compatible"),
    ("远程路径", "Remote path"),
    ("用户名", "Username"),
    ("密码 / 应用密码", "Password / app password"),
    ("尚未填写", "Not filled in"),
    ("保存同步设置", "Save sync settings"),
    ("下载覆盖本机", "Download and replace this device"),
    ("覆盖远端", "Overwrite the remote"),
    (
        "远端内容包含订阅来源（含令牌）、自定义规则、专用 DNS 与配置脚本。",
        "The remote holds subscription sources (including tokens), custom rules, dedicated DNS \
         and the config script.",
    ),
    // Settings: updates, script, about.
    ("检查更新", "Check for updates"),
    ("每天自动检查", "Check daily"),
    ("已是最新", "Up to date"),
    ("JavaScript 配置覆写", "JavaScript config override"),
    ("main(config) · 尚未配置", "main(config) · not configured"),
    ("手动打开后才工作", "Starts when you open it"),
    ("跟随系统", "Follow the system"),
    // A few labels are built from data and never appear alone; these do.
    ("自定义规则", "Custom rules"),
    ("节点 / 订阅", "Nodes / subscriptions"),
    // Sentence *templates*, keyed with their holes in place. `i18n::fill` looks these up before
    // substituting, which is the only way English can move a number to where English puts it.
    (
        "缓冲区 {} 条记录 · 当前显示 {} 条",
        "{} lines buffered, {} shown",
    ),
    ("最多保留 {} 条", "holds at most {}"),
    (" · 采集失败：{}", " · collection failed: {}"),
    ("{} 条启用 · {} 条规则", "{} enabled · {} rules"),
    ("内容写入同级目录 {}/", "written alongside {}/"),
    ("当前 v{} · 上次检查 {}", "Now v{} · last checked {}"),
    (
        "已设置 macOS 系统代理 127.0.0.1:7890",
        "macOS system proxy set to 127.0.0.1:7890",
    ),
    ("每天一次 · 启动时检查", "Daily · checked at launch"),
    ("每天一次", "Daily"),
    ("未启动", "Not started"),
    // Time, in the largest unit that still reads as recent. `刚刚` and `从未检查` are the two
    // answers that are not a count at all.
    ("刚刚", "just now"),
    ("从未检查", "never checked"),
    ("{} 秒前", "{}s ago"),
    ("{} 分钟前", "{} min ago"),
    ("{} 小时前", "{} hours ago"),
    ("{} 天前", "{} days ago"),
    ("每天一次 · {}", "Daily · {}"),
    (
        "仅手动检查，启动时不再联网",
        "Manual only — no check at launch",
    ),
    ("尚未测量", "Not measured"),
    ("探测中", "Probing"),
    ("未测试", "Untested"),
    ("最近一次探测", "Last probe"),
    ("正在逐个测量", "Measuring nodes"),
    ("订阅内节点", "Nodes in subscriptions"),
    ("正在转发", "Forwarding"),
    ("已连接", "Connected"),
    ("处理中", "Working"),
    ("系统代理设置中", "Setting the system proxy"),
    // Fields' placeholders and labels — invisible to a screen sweep, since a placeholder is not
    // a text node and an accessibility name is not drawn at all.
    ("搜索设置项", "Search settings"),
    ("搜索节点、地址或协议", "Search nodes, addresses or protocols"),
    ("搜索域名、IP、节点", "Search domains, IPs or nodes"),
    ("搜索进程名或路径", "Search a process name or path"),
    ("搜索进程名或路径…", "Search a process name or path…"),
    ("https:// 订阅地址，或 ss://、vless:// 等节点链接", "https:// subscription URL, or ss://, vless:// links"),
    ("订阅地址或节点内容", "Subscription URL or node links"),
    ("名称（可选）", "Name (optional)"),
    ("名称留空时用订阅地址的主机命名。", "Left empty, the subscription's host is used as the name."),
    ("域名后缀", "Domain suffix"),
    ("DNS 服务器", "DNS server"),
    ("尚未配置专用 DNS", "No dedicated DNS yet"),
    ("匹配内容", "Match"),
    ("匹配类型", "Match type"),
    ("此处指单条规则的匹配动作，与全局分流模式不同", "The action for this rule, not the global mode"),
    ("命中域名用专用 DNS 解析并直连，任何分流模式都生效", "Matching domains resolve here and go direct, in every mode"),
    ("为指定域名固定 DNS，避免公共 DNS 解析到错误地址。", "Pin DNS for a domain when public resolvers answer wrongly."),
    // The words `proxy_core` answers with when a DNS field is refused. They are listed here rather
    // than in the crate that writes them because that crate is shared with the Dioxus shell, which
    // draws them as they are and knows nothing of this table.
    ("域名无效", "that domain is not valid"),
    ("域名关键字无效", "that keyword is not valid"),
    ("域名后缀无效", "that domain suffix is not valid"),
    ("DNS 服务器必须是 IP 或域名", "the DNS server must be an IP or a domain"),
    ("专用 DNS 只支持域名类匹配", "dedicated DNS matches on a domain only"),
    // The routing form's refusals, same reason and same sink. Two of this enum's variants embed a
    // number in their message and so cannot be keys at all; they are listed by name in
    // `every_rule_validation_error_translates`, which is what keeps this list honest.
    ("规则内容不能为空", "the rule needs something to match"),
    ("域名格式无效", "that is not a valid domain"),
    ("域名关键字不能包含空白、路径或 URL scheme", "a keyword cannot contain spaces, a path or a scheme"),
    (
        "CIDR 格式无效，请输入类似 192.168.0.0/16 或 2001:db8::/32",
        "not a valid CIDR; try 192.168.0.0/16 or 2001:db8::/32",
    ),
    (
        "进程名不能为空或包含控制字符，请直接填进程名（如 Telegram）",
        "a process name cannot be empty or hold control characters; give the name alone (Telegram)",
    ),
    (
        "进程路径必须是绝对路径，如 /Applications/Telegram.app/Contents/MacOS/Telegram 或 C:\\Program Files\\Telegram\\Telegram.exe",
        "a process path must be absolute, e.g. /Applications/Telegram.app/Contents/MacOS/Telegram \
         or C:\\Program Files\\Telegram\\Telegram.exe",
    ),
    ("规则 ID 必须唯一且不能为 0", "a rule id must be unique and not 0"),
    ("已存在相同的匹配规则", "a rule with that match already exists"),
    // The two that name a limit, so they are templates and their holes are filled after the
    // lookup. They live in `ENGLISH` like any other row and are reached through
    // `domain::rule_error_message`, which is the only place that knows the number.
    ("规则内容不能超过 {} 字节", "a rule cannot be longer than {} bytes"),
    ("自定义规则不能超过 {} 条", "no more than {} custom rules"),
    ("添加一条规则，为指定域名或网段固定走向。", "Add a rule to pin a domain or range to one route."),
    ("JavaScript 配置覆写已启用，自定义分流规则暂不可用。", "The JavaScript override is on, so custom routing rules are inactive."),
    ("启用 JavaScript 配置覆写", "Enable the JavaScript config override"),
    ("启用后自定义分流规则将被脚本接管。", "When on, a script takes over custom routing rules."),
    ("还没有填写脚本", "No script yet"),
    ("配置脚本校验", "Validate the config script"),
    ("脚本", "Script"),
    ("选择", "Select"),
    // Buttons and their states.
    ("添加订阅", "Add subscription"),
    ("添加规则", "Add rule"),
    ("添加专用 DNS", "Add dedicated DNS"),
    ("收起", "Collapse"),
    ("收起规则表单", "Close the rule form"),
    ("收起专用 DNS 表单", "Close the DNS form"),
    ("编辑规则", "Edit rule"),
    ("编辑这条规则", "Edit this rule"),
    ("删除这条规则", "Delete this rule"),
    ("再次点击确认删除这条规则", "Press again to delete this rule"),
    ("上移", "Move up"),
    ("下移", "Move down"),
    ("导入", "Import"),
    ("使用", "Use"),
    ("刷新中", "Refreshing"),
    ("刷新全部订阅", "Refresh all"),
    ("刷新延迟", "Check latency"),
    ("停止测速", "Stop measuring"),
    ("自动测速", "Auto-measure"),
    ("手动选择", "Choose manually"),
    ("内置出口", "Built-in exit"),
    ("重新连接", "Reconnect"),
    ("重新检测内核", "Re-check the core"),
    ("重试", "Retry"),
    ("更新分流规则", "Update rule sets"),
    ("忽略这个版本", "Ignore this release"),
    ("正在保存", "Saving"),
    ("正在校验", "Validating"),
    ("正在检查", "Checking"),
    ("正在测量", "Measuring"),
    ("正在更新", "Updating"),
    ("正在刷新", "Refreshing"),
    ("正在读取", "Reading"),
    ("正在解析", "Parsing"),
    ("正在覆盖", "Overwriting"),
    ("正在下载", "Downloading"),
    ("正在上传", "Uploading"),
    ("正在探测出口", "Probing the exit"),
    ("正在生成代理组", "Building proxy groups"),
    ("正在通过代理拨测外部地址", "Dialling an external address through the proxy"),
    ("从代理拨一次外部地址，确认流量真的出得去", "Dial out through the proxy to confirm traffic leaves"),
    ("运行中", "Running"),
    ("等待代理流量", "Waiting for proxied traffic"),
    ("本地配置已改动，运行中的内核还是旧的。", "Local settings changed; the running core still has the old config."),
    ("内核不可用，请重新检测；若仍失败，请检查服务是否运行、端口是否被占用。", "The core is unavailable — re-check; if it stays unavailable, see whether it runs and the port is free."),
    ("内核未能直接切换，正在重新连接以应用", "The core could not switch in place; reconnecting to apply"),
    ("内核正在处理其他请求", "The core is busy with another request"),
    ("局域网内其他设备可使用 7890", "Other devices on the network can use 7890"),
    ("接管全部流量，首次需要管理员授权", "Takes over all traffic; needs administrator rights the first time"),
    // Notices, which arrive late and so are the strings a screen sweep never sees.
    ("分流规则已更新", "Routing rules updated"),
    ("规则已保存", "Rule saved"),
    ("规则已保存，重新连接后生效", "Rule saved; applies after reconnecting"),
    ("要编辑的规则不存在", "The rule to edit no longer exists"),
    ("无法生成规则 ID", "Could not allocate a rule id"),
    ("最多添加 {} 条规则", "At most {} rules"),
    ("没有匹配的进程", "No processes match"),
    ("没有找到可用节点", "No usable node found"),
    ("正在读取进程列表…", "Reading the process list…"),
    ("进程分流需开启 TUN 才能覆盖所有进程；仅系统代理模式下，进程规则只作用于走代理的进程。", "Process routing needs TUN to cover every process; in system-proxy mode a process rule only covers proxied traffic."),
    ("TUN 模式已关闭", "TUN mode is off"),
    ("已切换订阅，重新连接后生效", "Subscription switched; applies after reconnecting"),
    ("订阅已刷新，可重新连接或启动", "Subscription refreshed; reconnect or connect now"),
    ("订阅已更新但节点尚未解析，连接前请先刷新全部订阅。", "The subscription changed but its nodes are not parsed — refresh all before connecting."),
    ("请先填写订阅地址或内容", "Enter a subscription URL or links first"),
    ("我的订阅", "My subscription"),
    ("该订阅的节点会一并移除，正在使用的节点会改用其他订阅。", "Its nodes go too; the one in use moves to another subscription."),
    ("当前使用", "In use"),
    ("已暂停", "Paused"),
    ("采集中", "Collecting"),
    ("暂停采集日志", "Pause log collection"),
    ("继续采集日志", "Resume log collection"),
    ("暂无匹配日志", "No log lines match"),
    ("换个筛选或关键词；启动代理后每条连接都会写在这里。", "Try another filter or word; every connection is written here once the proxy runs."),
    ("内核未运行，无法采集日志", "The core is not running, so there is nothing to collect"),
    ("同步配置已保存", "Sync settings saved"),
    ("同步配置保存失败", "Could not save the sync settings"),
    ("正在读取同步设置", "Reading the sync settings"),
    ("正在读取已保存的同步设置", "Reading the saved sync settings"),
    ("远端上传失败", "Upload to the remote failed"),
    ("远端下载失败", "Download from the remote failed"),
    ("覆盖远端失败", "Overwriting the remote failed"),
    ("确认覆盖本机", "Confirm replacing this device"),
    ("确认覆盖远端", "Confirm overwriting the remote"),
    ("正在读取登录项", "Reading the login item"),
    ("正在写入当前用户的登录项", "Writing the login item for this user"),
    ("已加入登录启动项", "Added to login items"),
    ("已移出登录启动项", "Removed from login items"),
    ("登录后自动启动，不自动连接", "Opens at login, connects when you do"),
    ("当前平台没有登录启动项", "This platform has no login item"),
    ("系统代理已启用", "System proxy enabled"),
    ("系统代理已关闭", "System proxy disabled"),
    ("配置脚本已保存", "Config script saved"),
    ("配置脚本已保存，重新连接后生效", "Config script saved; applies after reconnecting"),
    ("配置脚本校验通过", "Config script validated"),
    ("已开启每日自动检查", "Daily automatic checks are on"),
    ("已改为仅手动检查", "Set to manual checks only"),
    ("已忽略这个版本，下次有更新再提醒", "This release is ignored; the next one will ask"),
    ("尚未检查过更新", "Not checked yet"),
    ("有新版本可更新", "a newer version is available"),
    ("代理环境变量", "proxy environment variables"),
    ("失败", "failed"),
    ("超时", "timed out"),
    // Sentences built from parts. Every one of these is a *template*: `i18n::fill` looks the row
    // up with its holes in place and substitutes afterwards. That is what makes the English able
    // to move a number or a name to where English puts it, and it is the only way a notice like
    // `已删除 实验室` can be translated at all — by the time that string exists, no table can hold
    // it, because it has a user's subscription name inside it.
    ("{} 切换失败，仍是原来的出口", "{} could not be switched; the old exit is still in use"),
    ("{} 已切换，无需重新连接", "{} switched; no reconnect needed"),
    ("{}（未连接，启动后生效）", "{} (not connected; applies once you connect)"),
    ("{}：订阅已不在列表中", "{}: that subscription is no longer listed"),
    ("关闭 TUN 失败：{}", "Could not turn TUN off: {}"),
    ("准备 TUN 权限失败，内核未启动：{}", "Could not get TUN permission; the core did not start: {}"),
    ("发现新版本 v{}", "A new version is available: v{}"),
    ("已切换到 {}", "Switched to {}"),
    ("已删除 {}", "Deleted {}"),
    ("已删除 {}，重新连接后生效", "Deleted {}; applies after reconnecting"),
    ("，重新连接后生效", "; applies after reconnecting"),
    ("已刷新 {} 个订阅，{} 个失败：{}", "Refreshed {} subscriptions, {} failed: {}"),
    ("已存在 {} 的专用 DNS", "Dedicated DNS for {} already exists"),
    ("已导入并切换到 {}，共 {} 个节点", "Imported and switched to {}, {} nodes"),
    ("，{} 个无法解析", ", {} could not be parsed"),
    ("忽略失败：{}", "Could not ignore the release: {}"),
    ("更新设置保存失败：{}", "Could not save the update settings: {}"),
    ("更新设置读取失败：{}", "Could not read the update settings: {}"),
    ("登录启动项设置失败：{}", "Could not set the login item: {}"),
    ("系统代理设置失败：{}", "Could not set the system proxy: {}"),
    ("订阅解析失败：{}", "Could not parse the subscription: {}"),
    ("远端数据已下载，但保存到本机失败：{}", "Downloaded from the remote, but saving locally failed: {}"),
    ("配置保存失败：{}", "Could not save the config: {}"),
    ("main(config) · QuickJS · {} 字符", "main(config) · QuickJS · {} characters"),
    ("上次检查 {}", "last checked {}"),
    ("同步配置读取失败：{}。已停用所有同步操作，以免用空配置覆盖已保存的设置。",
     "Could not read the sync settings: {}. Every sync action is parked so an empty config cannot overwrite the saved one."),
    ("后端原文：{}", "Backend said: {}"),
    ("正在保存同步设置", "Saving sync settings"),
    ("此构建无发布通道（{}）", "This build has no release channel ({})"),
    ("此构建没有发布通道（{}），请从发布页手动下载。",
     "This build has no release channel ({}); download it from the releases page."),
    ("{} 个节点", "{} nodes"),
    ("{} 个订阅 · 共 {} 个节点", "{} subscriptions · {} nodes"),
    ("删除 {}", "Delete {}"),
    ("删除「{}」？", "Delete “{}”?"),
    ("刷新 {}", "Refresh {}"),
    ("忽略 {}", "Ignore {}"),
    ("删除 {} 的专用 DNS", "Delete the dedicated DNS for {}"),
    ("启用 {} 的专用 DNS", "Enable the dedicated DNS for {}"),
    ("启用规则 {}", "Enable rule {}"),
    (
        "另有 {} 条进程规则在当前平台不可见，但仍计入规则总数并随配置同步。",
        "{} process rules are hidden on this platform but still counted and synced.",
    ),
    ("{} / {} 个节点 · 尚未测试延迟", "{} of {} nodes · latency not measured"),
    ("{} / {} 个节点 · 已测速 {} 个", "{} of {} nodes · {} measured"),
    ("{} · {} 个成员 · 当前 {}", "{} · {} members · now {}"),
    ("代理组生成失败：{}", "Could not build the proxy groups: {}"),
    ("{} 个活动连接", "{} active connections"),
    ("出口不可用：{}", "Exit unavailable: {}"),
    ("连通正常：{}", "Reachable through the proxy: {}"),
    ("{} 条专用 DNS", "{} dedicated DNS entries"),
    ("已同步 {} 个订阅、{} 条分流规则、{}", "Synced {} subscriptions, {} routing rules, {}"),
    ("远端未含专用 DNS（保留本机）", "the remote had no DNS section (local kept)"),
    (
        "将用远端 {} 个订阅 / {} 条规则替换本机 {} 个订阅 / {} 条规则",
        "This replaces {} local subscriptions / {} rules with {} / {} from the remote.",
    ),
    (
        "将用远端配置替换本机 {} 个订阅 / {} 条规则（远端数量需下载后确认）",
        "This replaces {} local subscriptions / {} rules with the remote's contents (its counts \
         are only known after a download).",
    ),
    ("缺 {}", "missing {}"),
    ("局域网代理 {}", "LAN proxy {}"),
    ("本地代理 {}", "Local proxy {}"),
    ("{} 个成员", "{} members"),
    ("{} 条新日志", "{} new lines"),
    ("选择进程", "Choose a process"),
    ("校验", "Validate"),
    ("{}失败：{}", "{}: {}"),
    // The shape every error notice takes: what was attempted, then the reason. `feedback` fills
    // it once per failure and both sides go in already translated.
    ("{}：{}", "{}: {}"),
    // Words the domain layer builds and the pages draw. These have rows for the same reason the
    // literals above do — a page hands the string to a sink that asks for it in English — and they
    // are the ones a screen sweep is least likely to see, because they name a state a machine has
    // to be in: a sync conflict, a platform without a system proxy, a group on `direct`.
    ("订阅已不存在", "that subscription is gone"),
    ("本地内容", "pasted content"),
    ("精确域名", "exact domain"),
    ("域名关键字", "domain keyword"),
    ("进程名", "process name"),
    ("进程路径", "process path"),
    ("域名", "domain"),
    ("正在应用到系统网络服务", "applying to the system's network services"),
    ("正在读取系统代理状态", "reading the system proxy state"),
    ("当前平台不支持系统代理", "this platform has no system proxy"),
    ("先启动代理，才能开启系统代理", "start the proxy before turning the system proxy on"),
    (
        "Android 由 VPN 接管，无需系统代理",
        "Android routes everything through the VPN, so no system proxy is needed",
    ),
    ("未检测到版本", "no version detected"),
    ("开启", "on"),
    ("待同步", "not synced"),
    ("已同步", "in sync"),
    (
        "远端配置已被其它设备更新，请先下载覆盖本机或明确覆盖远端",
        "another device updated the remote; download it first, or overwrite it deliberately",
    ),
    (
        "远端存储不支持安全上传，请改用「覆盖远端」",
        "that storage has no safe upload; use Overwrite Remote instead",
    ),
    (
        "本机尚无该远端目标的同步基线，请先下载覆盖本机或覆盖远端",
        "this machine has no baseline for that target; download it first, or overwrite it",
    ),
    (
        "远端版本校验未通过，请下载覆盖本机或覆盖远端",
        "the remote's revision check failed; download it first, or overwrite it",
    ),
    ("网盘的应用密码比主密码更安全", "an app password is safer than your account password"),
    ("检查中", "checking"),
    ("已忽略", "ignored"),
    ("有更新", "update available"),
    ("不支持", "unsupported"),
    ("检查失败", "check failed"),
    ("尚未检查", "not checked"),
    // One word inside a line assembled elsewhere: `about_build_label` puts it after `sing-box`,
    // where both languages want it, so only this word is looked up.
    ("未就绪", "not ready"),
    // Two strings the earlier screen sweep and my first scan both excused for having a space in
    // them: a field's label and a field's hint, both drawn verbatim. A space is not a reason to
    // skip a string, and the scan no longer treats it as one.
    ("用户名 / 密码", "username / password"),
    (
        "留空使用 AWS，或填写 https://s3.example.com",
        "leave empty for AWS, or enter https://s3.example.com",
    ),
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

    /// A sentence is translated as a template and filled afterwards, which is the only way English
    /// can put the count *after* the noun where Chinese puts it before.
    #[test]
    fn a_template_is_translated_then_filled() {
        assert_eq!(
            translate(Lang::Chinese, "刷新 {}"),
            "刷新 {}",
            "a template is stored in the language it was written in"
        );
        // Filling is done on the translated text, so an English template with a different hole
        // order works unchanged.
        let holes: Vec<&dyn std::fmt::Display> = vec![&"实验室"];
        assert_eq!(fill_at(Lang::Chinese, "刷新 {}", &holes), "刷新 实验室");
        let english = translate(Lang::English, "刷新 {}");
        assert!(
            english.contains("{}") || english == "刷新 {}",
            "the template must keep its hole to be fillable: {english}"
        );
    }

    /// More holes than arguments is a programming error, and the worst version of it is a silent
    /// one: a dropped word reads as prose and nobody reports it.
    #[test]
    fn a_missing_argument_leaves_the_hole_visible() {
        let none: Vec<&dyn std::fmt::Display> = vec![];
        assert_eq!(fill_at(Lang::Chinese, "{} 个节点", &none), "{} 个节点");
        let one: Vec<&dyn std::fmt::Display> = vec![&"3"];
        assert_eq!(fill_at(Lang::Chinese, "共 {} 和 {}", &one), "共 3 和 {}");
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

    /// Every template's English has exactly as many holes as its Chinese.
    ///
    /// `fill` substitutes positionally and leaves a hole it cannot fill visible, so an English
    /// side written with three `{}` against a Chinese `{}` × 2 puts a literal `{}` on the screen —
    /// the one failure mode of this design that no rendering test would describe usefully. The
    /// count is checked instead.
    #[test]
    fn a_template_keeps_its_holes() {
        for (from, to) in ENGLISH {
            let (want, got) = (from.matches("{}").count(), to.matches("{}").count());
            assert_eq!(want, got, "`{from}` has {want} holes but `{to}` has {got}");
        }
    }

    /// No key may appear twice.
    ///
    /// A table of ~270 rows edited by hand is exactly the shape in which a row gets pasted twice
    /// with different English, and `english` returns the first match, so the second one is dead
    /// code that reads as if it applied. (This caught two while the table was being written.)
    ///
    /// Two *different* keys answering with the same English is fine and is not checked: 正在刷新
    /// and 刷新中 are both "Refreshing", because English has no separate word for the in-progress
    /// aspect Chinese marks with 正在/中. Forbidding that would only push the table into invented
    /// English to satisfy a test.
    #[test]
    fn no_key_is_listed_twice() {
        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for (from, to) in ENGLISH {
            if let Some(before) = seen.insert(*from, *to) {
                panic!("`{from}` is listed twice, as `{before}` and `{to}`");
            }
        }
    }

    /// Every Chinese literal the app can draw has an English row.
    ///
    /// This is the check a screen sweep cannot be. The sweep sees what is drawn, so it missed 55
    /// templates and every word behind a state the test machine was not in: a toast that fires
    /// monthly, a conflict panel needing a second device, a label needing a running core. Here the
    /// question is what is *written* — every string literal with a Han character in the production
    /// sources must appear in `ENGLISH` as a key, or the table is incomplete no matter how clean
    /// the sweep looked.
    ///
    /// Written as a source scan rather than a call graph because the two failure modes differ:
    /// this catches a missing row, `english_leaves_no_chinese_on_any_screen` catches a row that
    /// exists but is never asked for.
    ///
    /// `ENGLISH` is walked rather than `tr` called, because reading the table needs no reactive
    /// runtime: `global` lives on the engine thread, and this has to run in a plain unit test.
    #[test]
    fn every_drawn_string_has_an_english_row() {
        let keys: std::collections::HashSet<&str> = ENGLISH.iter().map(|(from, _)| *from).collect();
        let mut missing = Vec::new();
        let mut drawn = 0usize;
        for path in crate_sources() {
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            for lit in string_literals(&production_code(&src)) {
                if !lit.chars().any(is_han) {
                    continue;
                }
                drawn += 1;
                // Compared after unescaping, because the scan reads source text and the table holds
                // compiled values. `C:\\Program Files` in a file is `C:\Program Files` in a binary,
                // so comparing the two raw reports a key that exists as a key that does not. This
                // is not a hypothetical: the process-path validation message names a Windows path
                // and was the first string to trip over it.
                let Some(lit) = unescape(lit) else {
                    continue;
                };
                if keys.contains(lit.as_str())
                    || searched_not_drawn(&lit)
                    || support_text_only(&lit)
                {
                    continue;
                }
                missing.push(format!(
                    "{lit}  <-  {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
        missing.sort();
        missing.dedup();
        // A scan that read nothing would pass the assertion below by finding nothing to complain
        // about, which is the failure this whole test exists to prevent. The floor is loose on
        // purpose — it asks only that sources were opened and test modules were not mistaken for
        // them.
        assert!(
            drawn > 400,
            "the scan saw only {drawn} Chinese literals across {:?}, so it is not reading the \
             crate's sources",
            crate_sources().len()
        );
        assert!(
            missing.is_empty(),
            "{} Chinese literal(s) reach the app with no English row:\n{}",
            missing.len(),
            missing.join("\n")
        );
    }

    /// Chinese kept because it is *searched*, not drawn: the words a settings card answers to.
    /// They are matched against what the reader types, so they must hold the Chinese terms even
    /// on a page drawn in English — translating them would take the search box's own vocabulary
    /// away from the person using it in Chinese.
    fn searched_not_drawn(lit: &str) -> bool {
        [
            "外观 深色 浅色 主题 语言 配色 appearance 简体中文",
            "流量走向 分流 模式 规则模式 全局模式 直连模式 routing",
            "接管方式 tun 局域网 允许局域网 系统代理 网络 权限 scope",
            "启动与终端 自动启动 开机 启动 登录 登录项 终端 autostart",
            "同步 云同步 备份 备份位置 远端 上传 下载 覆盖 webdav s3 bucket 拉取 推送 快照 配置 sync 存储 冲突",
            "更新 升级 版本 检查 下载 发布 发行 忽略 有新版本 update 自动检查 github",
            "配置脚本 内核 sing-box javascript 配置覆写 脚本 quickjs core",
            "关于 诊断 内核状态 内核版本 节点 订阅 规则 dns 复制 about 版本",
        ]
        .contains(&lit)
    }

    /// Chinese on purpose, because a reader other than the person using the app sees it.
    ///
    /// - the diagnostic report goes into a bug report, where the wording has to match what the
    ///   issue tracker's other lines and the backend's own logs say;
    /// - the `contains` queries in [`crate::domain::classify_sync_error`] read the *backend's*
    ///   sentence, which this app writes in Chinese whatever the interface language is. Matching a
    ///   translated query would simply stop the match.
    fn support_text_only(lit: &str) -> bool {
        [
            "Kitty Pro 诊断信息",
            "已被其他设备修改",
            "已被其它设备修改",
            "基线",
        ]
        .contains(&lit)
            || DIAGNOSTIC_LINES.iter().any(|line| lit.contains(line))
    }

    /// The report's lines, each of which reaches the screen as part of one multi-line literal
    /// rather than on its own — which is why they are matched as fragments.
    const DIAGNOSTIC_LINES: [&str; 7] = [
        "界面平台: {platform}",
        "内核状态: {state_label} ({core_state})",
        "内核版本: {core_version}",
        "分流模式: {mode}",
        "TUN: {tun} · 局域网: {lan} ({endpoint})",
        "节点: {node_count} · 订阅: {subscription_count}",
        "自定义规则: {rule_count} · 专用 DNS: {dns_count}",
    ];

    /// Every `.rs` in this crate's `src`, so a new page is scanned the moment it exists.
    ///
    /// Anchored on `CARGO_MANIFEST_DIR` rather than `file!()`: the latter is written relative to
    /// the workspace root and the test's working directory is the crate's own directory, so the
    /// obvious spelling resolves to a path that does not exist — which a scan that reports "no
    /// gaps" would never notice. `i18n.rs` is included: its keys are literals that each have a row
    /// by construction, so reading it costs nothing and catches a table entry mistyped into a
    /// string the lookup can never produce.
    fn crate_sources() -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        let mut queue = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = queue.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                // A packaged build has no sources beside the binary. A development check that
                // cannot see its inputs says nothing, so it says nothing rather than failing —
                // and `drawn > 400` below is what stops that from being mistaken for a pass.
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    queue.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    found.push(path);
                }
            }
        }
        found.sort();
        found
    }

    /// The file with every `#[cfg(test)]` item removed, doc comments stripped, and — because the
    /// offsets are kept — every remaining literal still on its real line.
    ///
    /// Cutting the test code out rather than truncating at the first `#[cfg(test)]` matters: in a
    /// file this size a test module starts a third of the way in and holds most of what the module
    /// does, so truncating reports a clean file for the lines nobody looked at.
    fn production_code(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let mut rest = src;
        while let Some(at) = rest.find("#[cfg(test)]") {
            out.push_str(&strip_comments(&rest[..at]));
            let after = &rest[at + "#[cfg(test)]".len()..];
            match item_kind(after) {
                Some(Item::Braced(start)) => {
                    let end = matching_brace(after, start).unwrap_or(after.len());
                    out.extend(
                        after[..end]
                            .chars()
                            .map(|c| if c == '\n' { c } else { ' ' }),
                    );
                    rest = &after[end..];
                }
                Some(Item::ToSemicolon(start)) => {
                    let end = after[start..]
                        .find(';')
                        .map_or(after.len(), |found| start + found + 1);
                    out.extend(
                        after[..end]
                            .chars()
                            .map(|c| if c == '\n' { c } else { ' ' }),
                    );
                    rest = &after[end..];
                }
                None => {
                    out.push_str("#[cfg(test)]");
                    rest = after;
                }
            }
        }
        out.push_str(&strip_comments(rest));
        out
    }

    enum Item {
        /// A `mod`, `fn` or `impl`: ends where its braces balance.
        Braced(usize),
        /// A `use`, `const` or `static`: ends at the next semicolon.
        ToSemicolon(usize),
    }

    fn item_kind(after: &str) -> Option<Item> {
        let bytes = after.as_bytes();
        let mut i = 0;
        // Past whitespace and any other attributes stacked on the same item, in either form.
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let open = match bytes.get(i) {
                Some(b'[') => i,
                Some(b'#')
                    if bytes.get(i + 1) == Some(&b'!') && bytes.get(i + 2) == Some(&b'[') =>
                {
                    i + 2
                }
                Some(b'#') if bytes.get(i + 1) == Some(&b'[') => i + 1,
                _ => break,
            };
            i = matching_pair(after, open, b'[', b']')?;
        }
        let rest = &after[i..];
        // A visibility may precede the keyword: `pub fn`, `pub(crate) mod`, `pub(super) const`.
        let rest = rest.strip_prefix("pub").unwrap_or(rest).trim_start();
        let rest = match rest.strip_prefix('(') {
            Some(inner) => matching_pair(after, after.len() - inner.len(), b'(', b')')
                .map_or(inner, |end| &after[end..])
                .trim_start(),
            None => rest,
        };
        // What is left must name one of the kinds `#[cfg(test)]` can sit on.
        for kind in ["mod ", "fn ", "impl ", "struct ", "enum ", "trait "] {
            if rest.starts_with(kind) {
                return Some(Item::Braced(after.len() - rest.len()));
            }
        }
        for kind in ["const ", "static ", "use "] {
            if rest.starts_with(kind) {
                return Some(Item::ToSemicolon(after.len() - rest.len()));
            }
        }
        None
    }

    /// Index of the brace closing the `{` at `open`.
    fn matching_brace(src: &str, open: usize) -> Option<usize> {
        matching_pair(src, open, b'{', b'}')
    }

    /// A literal's text as the compiler would hold it.
    ///
    /// The scan reads a file and the table holds values, and the two differ wherever a string
    /// escapes something: `C:\\Program Files` in a file is `C:\Program Files` in a binary, and a
    /// backslash at the end of a line swallows the newline and the next line's indent. Unescaping
    /// only the table's side would leave every escaped key looking absent, so the comparison has to
    /// resolve the file's side the same way. An escape this does not know is left as written: that
    /// makes the comparison *fail*, which reports the string for a human to look at, rather than
    /// silently skipping it — the failure mode this test exists to prevent.
    fn unescape(lit: &str) -> Option<String> {
        if !lit.contains('\\') {
            return Some(lit.to_owned());
        }
        let mut out = String::with_capacity(lit.len());
        let mut rest = lit;
        while let Some(at) = rest.find('\\') {
            out.push_str(&rest[..at]);
            let mut chars = rest[at + 1..].chars();
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('0') => out.push('\0'),
                Some('\'') => out.push('\''),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                // A line continuation: the newline and the next line's indent both disappear.
                Some('\n') => {
                    let tail = chars.as_str();
                    let kept = tail.trim_start_matches([' ', '\t', '\n', '\r']);
                    rest = kept;
                    continue;
                }
                // Unknown, or a `\u{…}` this does not attempt: leave it as written so the
                // comparison cannot pass by accident.
                other => {
                    out.push('\\');
                    if let Some(c) = other {
                        out.push(c);
                        rest = chars.as_str();
                        continue;
                    }
                    rest = "";
                    continue;
                }
            }
            rest = chars.as_str();
        }
        out.push_str(rest);
        Some(out)
    }

    /// Index of the closing bracket for the opener at `open`, ignoring brackets inside a string.
    ///
    /// `char::from` and a test's own `"{"` both appear in this crate, so a depth count that reads
    /// a brace inside a string drifts and swallows the production code that follows it.
    fn matching_pair(src: &str, open: usize, left: u8, right: u8) -> Option<usize> {
        let bytes = src.as_bytes();
        let mut depth = 0usize;
        let mut index = open;
        let mut quoted = false;
        let mut escaped = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
            } else if byte == left {
                depth += 1;
            } else if byte == right {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            index += 1;
        }
        None
    }

    /// Doc and line comments gone, newlines kept.
    fn strip_comments(src: &str) -> String {
        src.lines()
            .map(|line| {
                if line.trim_start().starts_with("//") {
                    " ".repeat(line.len())
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every ordinary `"..."` literal in the source, with raw strings and char literals stepped
    /// over rather than parsed as text.
    ///
    /// A linear walk is the point: a regex over the whole file reads the interior of a raw string
    /// — where this crate keeps JSON fixtures with Chinese in them — as production text, and reads
    /// a `'{'` as the start of one.
    fn string_literals(src: &str) -> Vec<&str> {
        let bytes = src.as_bytes();
        let mut found = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'r' if bytes
                    .get(index + 1)
                    .is_some_and(|b| *b == b'"' || *b == b'#') =>
                {
                    let mut hashes = 0;
                    let mut open = index + 1;
                    while bytes.get(open) == Some(&b'#') {
                        hashes += 1;
                        open += 1;
                    }
                    if bytes.get(open) != Some(&b'"') {
                        index += 1;
                        continue;
                    }
                    let close = format!("\"{}", "#".repeat(hashes));
                    index = src[open + 1..]
                        .find(&close)
                        .map_or(bytes.len(), |at| open + 1 + at + close.len());
                }
                b'\'' => index = skip_char(bytes, index),
                b'"' => {
                    let end = match string_end(bytes, index + 1) {
                        Some(end) => end,
                        None => break,
                    };
                    found.push(&src[index + 1..end]);
                    index = end + 1;
                }
                _ => index += 1,
            }
        }
        found
    }

    /// One past the closing `"` of the literal opened just after `open`.
    fn string_end(bytes: &[u8], open: usize) -> Option<usize> {
        let mut index = open;
        while index < bytes.len() {
            match bytes[index] {
                b'\\' => index += 2,
                b'"' => return Some(index),
                _ => index += 1,
            }
        }
        None
    }

    /// One past a char literal, including the awkward ones: a char *is* a quote or a brace, and a
    /// lifetime `'a` is not a literal at all.
    fn skip_char(bytes: &[u8], open: usize) -> usize {
        let mut index = open + 1;
        if bytes.get(index) == Some(&b'\\') {
            index += 2;
        } else {
            index += 1;
        }
        if bytes.get(index) == Some(&b'\'') {
            return index + 1;
        }
        // Not a char literal: a lifetime. Leave it alone and resume just after the apostrophe, so
        // `'static str` and `&'a mut` cost nothing.
        open + 1
    }

    fn is_han(c: char) -> bool {
        matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
    }
}
