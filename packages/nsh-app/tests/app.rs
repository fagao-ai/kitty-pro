//! The app under NSH's real engine, headless.
//!
//! `Screen::texts` reports only what a user can actually see: a text whose view measures
//! nothing, is hidden, or is clipped outside the window is left out (verified against the
//! engine, 2026-11-09). So `wait_for` is a layout check as much as a content one, which is
//! how a bottom bar that squeezed the page to zero width, and a virtual list whose card had
//! no height, both showed up here as missing text rather than passing quietly.
//!
//! What these cannot show is whether the result looks right: rhythm, alignment, whether a
//! part that draws nothing is styled. Run the app for that.

use nsh_core::geometry::{px, Size};
use nsh_engine::{ColorScheme, EngineConfig, Environment, Idiom, Os};
use nsh_testing::Headless;

/// The lock that keeps one test's profile to itself.
///
/// Screens read and write the profile through `api`, which honours `KITTY_PROFILE_DIR`
/// (see `packages/api/src/core.rs`) for exactly this reason: the Playwright suite once
/// clobbered a developer's real profile. The variable is process-wide and cargo runs tests
/// on threads, so every test takes this lock and holds it for its whole body.
///
/// A test must lock once and start its engines inside that guard. An engine helper that
/// locks again deadlocks on the same thread, which is how this suite hung once; that is why
/// `desktop` and `seeded` below take a `&Profile` instead of acquiring it themselves.
///
/// Panicking poisons the lock. A poisoned lock still guards the directory, so the rest of
/// the suite carries on rather than cascading one failure into the others.
static PROFILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A test's exclusive scratch profile.
struct Profile {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Profile {
    /// Claim the lock and point `KITTY_PROFILE_DIR` at an empty directory.
    fn claim(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kitty-nsh-headless-{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch profile directory");
        let _guard = PROFILE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::set_var("KITTY_PROFILE_DIR", &dir);
        Self { _guard }
    }

    /// Start over from a stored profile, for the cases where what was saved is the point.
    fn seed(&self, profile_json: &str) {
        let dir = std::path::PathBuf::from(
            std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
        );
        std::fs::write(dir.join("profile.json"), profile_json).expect("a seedable profile");
    }
}

fn environment() -> Environment {
    Environment {
        window: Size::new(px(1280.0), px(840.0)),
        os: Os::MacOs,
        idiom: Idiom::Desktop,
        color_scheme: ColorScheme::Light,
        ..Environment::default()
    }
}

/// A desktop surface. The sidebar only appears at a desktop idiom; a phone gets a bottom
/// bar instead. Call it twice on one `Profile` to relaunch over the same files.
fn desktop(profile: &Profile) -> Headless {
    let _ = profile;
    Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(840.0)),
        environment: environment(),
        ..EngineConfig::new(nsh_app_kitty::app_root())
    })
    .expect("the app starts")
}

/// A phone surface, to prove the layout survives a narrow width.
fn phone(profile: &Profile) -> Headless {
    let _ = profile;
    Headless::start(EngineConfig::new(nsh_app_kitty::app_root())).expect("the app starts")
}

/// Claim a profile and start a desktop engine on it. The guard travels with the engine as
/// `_profile`, so a test that does not care about relaunching stays one line long.
fn desktop_test(test: &str) -> (Headless, Profile) {
    let profile = Profile::claim(test);
    let app = desktop(&profile);
    (app, profile)
}

/// Claim a profile and start a phone engine on it.
fn phone_test(test: &str) -> (Headless, Profile) {
    let profile = Profile::claim(test);
    let app = phone(&profile);
    (app, profile)
}

/// The same, from a seeded profile.
fn seeded_test(test: &str, profile_json: &str) -> (Headless, Profile) {
    let profile = Profile::claim(test);
    profile.seed(profile_json);
    let app = desktop(&profile);
    (app, profile)
}

#[test]
fn theme_json_parses() {
    nsh_app_kitty::tokens().expect("theme.json must parse as shadcn cssVars");
}

#[test]
fn the_shell_opens_on_the_overview() {
    let (mut app, _profile) = desktop_test("the_shell_opens_on_the_overview");
    app.wait_for("Kitty Pro").unwrap();
    app.wait_for("总览").unwrap();
    app.wait_for("近一分钟流量").unwrap();
    // Boot is not decoration: the root asks the core what it is running, so the shell
    // settles out of "正在检测" on its own. A lost effect leaves it stuck there.
    app.wait_for("内核已就绪").unwrap();
    // Nothing has started a core, so the one large control offers to. Named by role so
    // it is not read as the "连接" navigation group, which shares the word.
    app.wait_for("role:button[启动代理]").unwrap();
}

#[test]
fn the_rail_groups_navigation_by_task() {
    let (mut app, _profile) = desktop_test("the_rail_groups_navigation_by_task");
    app.wait_for("Kitty Pro").unwrap();
    // "连接" names this group and also the power button, in the original app's wording,
    // so a test that means the button says `role:button[...]`.
    for group in ["连接", "配置", "运维"] {
        assert!(app.shows(group), "{group} missing from the rail");
    }
    for label in ["节点", "订阅", "规则", "日志", "设置"] {
        assert!(app.shows(label), "{label} missing from the rail");
    }
}

#[test]
fn a_press_moves_to_another_screen() {
    let (mut app, _profile) = desktop_test("a_press_moves_to_another_screen");
    app.wait_for("近一分钟流量").unwrap();
    app.press("设置").unwrap();
    app.wait_for("流量走向").unwrap();
    // Only one screen is built at a time, so the overview's text really goes away.
    assert!(!app.shows("近一分钟流量"));
}

/// The header toggle names the appearance the press will bring, so its own name flips
/// with the state. This is the control the original app puts in the top bar.
#[test]
fn the_header_toggle_switches_appearance() {
    let (mut app, _profile) = desktop_test("the_header_toggle_switches_appearance");
    app.wait_for("切换深色外观").unwrap();
    app.press("切换深色外观").unwrap();
    app.wait_for("切换浅色外观").unwrap();
}

/// Settings holds the labelled switch. The rail used to carry a second copy of it, which
/// is how one setting ends up with two names; this asserts there is only one.
#[test]
fn settings_offers_the_appearance_switch_once() {
    let (mut app, _profile) = desktop_test("settings_offers_the_appearance_switch_once");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("深色模式").unwrap();
    assert_eq!(
        app.screen().count("text:深色模式").unwrap(),
        1,
        "the appearance switch is offered twice"
    );
}

#[test]
fn the_phone_replaces_the_rail_with_a_tab_bar() {
    let (mut app, _profile) = phone_test("the_phone_replaces_the_rail_with_a_tab_bar");
    // The 264pt rail would eat most of a 390pt screen, so it is not drawn at all.
    app.wait_for("总览").unwrap();
    app.wait_for("设置").unwrap();
    assert!(!app.shows("运维"), "the desktop rail leaked onto a phone");
    // The page still gets the full width: the bar sits under it, not beside it.
    app.wait_for("近一分钟流量").unwrap();
}

/// The log page is open over a core that is not running, so nothing is being collected and
/// the page has to say which of those it is: an empty list and a stopped feed look the same
/// until something names them.
#[test]
fn the_log_page_reports_a_stopped_core_rather_than_an_empty_one() {
    let (mut app, _profile) =
        desktop_test("the_log_page_reports_a_stopped_core_rather_than_an_empty_one");
    app.wait_for("Kitty Pro").unwrap();
    app.press("日志").unwrap();
    for tally in ["全部路由", "代理", "直连", "拦截", "内核日志"] {
        assert!(app.shows(tally), "the tally strip is missing {tally}");
    }
    app.wait_for("已停止").unwrap();
    app.wait_for("缓冲区 0 条记录 · 当前显示 0 条").unwrap();
    // The column headings are what make a row of monospaced strings readable at all.
    for column in ["时间", "动作", "目标", "命中链路", "出口节点", "消息"] {
        assert!(
            app.shows(column),
            "the log table is missing the {column} column"
        );
    }
    // Nothing to clear, so the action says so instead of offering a no-op.
    assert!(
        !app.screen().enabled("清空日志").unwrap(),
        "清空日志 is offered while the buffer is empty"
    );
}

#[test]
fn the_rules_page_lists_nothing_and_offers_the_editor() {
    let (mut app, _profile) = desktop_test("the_rules_page_lists_nothing_and_offers_the_editor");
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("分流规则").unwrap();
    app.wait_for("专用 DNS").unwrap();
    app.wait_for("添加规则").unwrap();
    app.wait_for("暂无自定义规则").unwrap();
    app.wait_for("0 条启用 · 0 条规则").unwrap();
    // The editor's own labels, which is also the proof the selects drew their triggers: an
    // unstyled select has nothing to press and would leave these two names unsaid.
    for field in ["匹配类型", "匹配内容", "动作"] {
        assert!(
            app.shows(field),
            "the rule editor is missing its {field} field"
        );
    }
    // A draft rule cannot be added, so the page does not pretend otherwise.
    assert!(!app.screen().enabled("添加").unwrap());
}

/// Switching tabs swaps the subject and its fields, and the other tab's own text has to
/// actually go away: a page that kept both mounted would pass a `shows` check for content
/// nobody is looking at.
#[test]
fn the_rules_tabs_swap_their_subjects() {
    let (mut app, _profile) = desktop_test("the_rules_tabs_swap_their_subjects");
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("暂无自定义规则").unwrap();

    app.press("专用 DNS").unwrap();
    app.wait_for("尚未配置专用 DNS").unwrap();
    app.wait_for("域名后缀").unwrap();
    app.wait_for("DNS 服务器").unwrap();
    assert!(
        !app.shows("暂无自定义规则"),
        "the routing tab stayed drawn under the DNS tab"
    );

    app.press("分流规则").unwrap();
    app.wait_for("暂无自定义规则").unwrap();
    assert!(!app.shows("尚未配置专用 DNS"));
}

#[test]
fn the_subscriptions_page_asks_for_the_first_one() {
    let (mut app, _profile) = desktop_test("the_subscriptions_page_asks_for_the_first_one");
    app.wait_for("Kitty Pro").unwrap();
    app.press("订阅").unwrap();
    app.wait_for("暂无订阅").unwrap();
    app.wait_for("还没有订阅").unwrap();
    // The empty state carries the action itself: a reader who arrived by keyboard from the
    // header should not have to hunt the toolbar for the only way forward.
    assert_eq!(
        app.screen().count("role:button[添加订阅]").unwrap(),
        2,
        "the empty state should repeat the add action"
    );
    // With nothing to refresh, the header's refresh control is not offered at all.
    assert_eq!(app.screen().count("role:button[刷新全部订阅]").unwrap(), 0);
}

/// Importing is the page's one destructive-by-default act (it replaces the active node set),
/// so the form it opens in has to be reachable and dismissible without a network round trip.
#[test]
fn the_import_dialog_opens_and_closes_without_touching_the_network() {
    let (mut app, _profile) =
        desktop_test("the_import_dialog_opens_and_closes_without_touching_the_network");
    app.wait_for("Kitty Pro").unwrap();
    app.press("订阅").unwrap();
    app.press("role:button[添加订阅] >> nth=0").unwrap();
    app.wait_for("订阅地址或节点内容").unwrap();
    app.wait_for("名称（可选）").unwrap();
    app.press("取消").unwrap();
    app.wait_for_gone("名称（可选）", std::time::Duration::from_secs(2))
        .unwrap();
    // Closed means gone, not hidden: a dialog left in the tree keeps its inputs focusable.
    assert_eq!(app.screen().count("text:名称（可选）").unwrap(), 0);
}

/// A seeded profile's subscription has to reach the page as a row that says which one the
/// core is using. This is also the proof that only the active subscription's nodes are in
/// play: the overview names 东京 01 and no other row competes with it.
#[test]
fn a_seeded_subscription_shows_as_the_active_row() {
    let (mut app, _profile) = seeded_test(
        "a_seeded_subscription_shows_as_the_active_row",
        SEEDED_PROFILE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("订阅").unwrap();
    app.wait_for("实验室").unwrap();
    // A leading `https:` reads as a selector prefix to the driver, so this one goes
    // through the plain text check.
    assert!(
        app.shows("https://example.test"),
        "the row omits its source"
    );
    app.wait_for("2 个节点").unwrap();
    app.wait_for("当前使用").unwrap();
    app.wait_for("1 个订阅 · 共 2 个节点").unwrap();
    // Switching to the subscription already in use cannot do anything, so it is parked;
    // refreshing it can, so it is not.
    assert!(!app.screen().enabled("使用").unwrap());
    assert!(app.screen().enabled("role:button[刷新 实验室]").unwrap());
    assert!(app.screen().enabled("role:button[刷新全部订阅]").unwrap());
}

/// The theme file and the bundled fonts have to survive all the way into a resolved span.
/// A misspelled token falls back quietly and a woff2 that fails to register falls back to
/// the system font, so `shows` would pass either way. Reading the computed style is what
/// proves the design system is actually in force.
#[test]
fn the_theme_and_the_fonts_reach_the_render() {
    let (mut app, _profile) = desktop_test("the_theme_and_the_fonts_reach_the_render");
    app.wait_for("Kitty Pro").unwrap();

    // `--foreground` light is #171528; shadcn's neutral default is #0c0a09, so this fails
    // if theme.json was never applied.
    let brand = app.screen().span_style("text:Kitty Pro").unwrap();
    assert_eq!(
        hex(brand.color),
        "171528",
        "the brand is not drawing --foreground"
    );
    assert_eq!(brand.font_weight, 600, "the brand should be semibold");
    assert!(
        brand
            .font_family
            .is_some_and(|family| family.name().contains("Space Grotesk")),
        "the display face did not register: {:?}",
        brand.font_family.map(|family| family.name().to_string())
    );

    let kicker = app.screen().span_style("text:SING-BOX CLIENT").unwrap();
    assert_eq!(
        hex(kicker.color),
        "5f5b79",
        "the kicker is not drawing --text-faint"
    );
    assert_eq!(kicker.font_size.0, 12.0, "a caption should be 12pt");

    // A large figure carries the display face and its own size, not the row's.
    let metric = app.screen().span_style("text:--").unwrap();
    assert_eq!(metric.font_size.0, 22.0);
}

/// The resolved color as `RRGGBB`, whatever alpha it carried.
fn hex(color: nsh_core::Color) -> String {
    let (r, g, b, _a) = color.to_f64();
    format!(
        "{:02x}{:02x}{:02x}",
        (r * 255.0) as u32,
        (g * 255.0) as u32,
        (b * 255.0) as u32
    )
}

/// Small text in a status color is the easiest thing in the app to get wrong, because a
/// wash is picked to look quiet while the text on it still has to be readable. The pairs
/// are read from `theme.json`, so adding a role puts it under this check automatically.
/// The original app lost AA on one faint dark-mode label and only an axe run caught it.
#[test]
fn every_status_pair_clears_aa_contrast() {
    let theme: serde_json::Value =
        serde_json::from_str(nsh_app_kitty::THEME_JSON).expect("theme.json is JSON");
    let vars = theme.get("cssVars").expect("cssVars");

    for mode in ["light", "dark"] {
        let set = vars.get(mode).expect("a mode");
        let card = surface(set, mode);
        for role in ["success", "info", "warning", "danger", "dns"] {
            let foreground = parse(set.get(role).and_then(|v| v.as_str()), mode, role);
            let ratio = contrast(foreground, card);
            assert!(
                ratio >= 4.5,
                "{mode}: --{role} on a card reads at {ratio:.2}:1, below WCAG AA for small text"
            );
        }
        for role in ["text-faint", "text-soft"] {
            let foreground = parse(set.get(role).and_then(|v| v.as_str()), mode, role);
            let ratio = contrast(foreground, card);
            assert!(
                ratio >= 4.5,
                "{mode}: --{role} on a card reads at {ratio:.2}:1, below AA"
            );
        }
        // The primary button: white-on-accent in light, dark-on-amber in dark.
        let on_primary = parse(
            set.get("primary-foreground").and_then(|v| v.as_str()),
            mode,
            "primary-foreground",
        );
        let primary = parse(set.get("primary").and_then(|v| v.as_str()), mode, "primary");
        let ratio = contrast(on_primary, primary);
        assert!(
            ratio >= 4.5,
            "{mode}: a label on the primary button reads at {ratio:.2}:1, below AA"
        );
    }
}

/// Where a card actually lands: `--card` is translucent, so it composites over the page.
fn surface(set: &serde_json::Value, mode: &str) -> (f64, f64, f64) {
    let background = parse(
        set.get("background").and_then(|v| v.as_str()),
        mode,
        "background",
    );
    let card = set.get("card").and_then(|v| v.as_str()).expect("--card");
    let (r, g, b, a) = components(card, mode, "card");
    (
        r * a + background.0 * (1.0 - a),
        g * a + background.1 * (1.0 - a),
        b * a + background.2 * (1.0 - a),
    )
}

fn parse(value: Option<&str>, mode: &str, name: &str) -> (f64, f64, f64) {
    let (r, g, b, a) = components(
        value.unwrap_or_else(|| panic!("{mode} is missing --{name}")),
        mode,
        name,
    );
    (r * a, g * a, b * a)
}

/// `#rrggbb` or `rgba(r, g, b, a)` as channels in `0.0..=1.0`.
fn components(value: &str, _mode: &str, name: &str) -> (f64, f64, f64, f64) {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let byte = |range: std::ops::Range<usize>| {
            f64::from(u32::from_str_radix(&hex[range], 16).unwrap()) / 255.0
        };
        return (byte(0..2), byte(2..4), byte(4..6), 1.0);
    }
    let inner = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or_else(|| panic!("--{name}: unsupported color `{value}`"));
    let mut parts = inner
        .split(',')
        .map(|part| part.trim().parse::<f64>().unwrap());
    (
        parts.next().unwrap() / 255.0,
        parts.next().unwrap() / 255.0,
        parts.next().unwrap() / 255.0,
        parts.next().unwrap_or(1.0),
    )
}

/// WCAG 2.1 contrast ratio between two opaque colors.
fn contrast(foreground: (f64, f64, f64), background: (f64, f64, f64)) -> f64 {
    fn luminance(color: (f64, f64, f64)) -> f64 {
        let channel = |s: f64| {
            if s <= 0.039_28 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
    }
    let (light, dark) = (luminance(foreground), luminance(background));
    let (hi, lo) = if light > dark {
        (light, dark)
    } else {
        (dark, light)
    };
    (hi + 0.05) / (lo + 0.05)
}

/// A saved appearance has to reach the engine, not just the switch next to it. Setting
/// the signal alone would relabel the control while the window stayed in the appearance
/// the system asked for, which is easy to miss because every text assertion still passes.
#[test]
fn a_saved_dark_appearance_comes_back_after_a_relaunch() {
    let (mut app, _profile) = seeded_test(
        "a_saved_dark_appearance_comes_back_after_a_relaunch",
        r#"{ "dark_mode": true }"#,
    );
    app.wait_for("切换浅色外观").unwrap();
    // The system asked for light (see `desktop`), so only the stored choice can produce
    // the dark foreground: theme.json's `--foreground` dark is #f0eef5.
    let brand = app.screen().span_style("text:Kitty Pro").unwrap();
    assert_eq!(
        hex(brand.color),
        "f0eef5",
        "the window stayed in the system appearance despite a stored choice"
    );
}

/// A profile in the format the Dioxus app writes, generated from `AppProfile` itself so
/// the port is shown to read real files rather than a shape invented for the test. Two
/// nodes, one chosen. It caught a wrong guess the first time: protocols serialize in
/// lowercase and `auth` is internally tagged with `kind`.
const SEEDED_PROFILE: &str = include_str!("fixtures/profile-two-nodes.json");

/// The same profile with one saved routing rule, for the cases where a row has to exist
/// before it can be edited, reordered or deleted.
const PROFILE_WITH_RULE: &str = include_str!("fixtures/profile-one-rule.json");

#[test]
fn a_seeded_profile_fills_the_shell() {
    let (mut app, _profile) = seeded_test("a_seeded_profile_fills_the_shell", SEEDED_PROFILE);
    // The kicker reads the core build, and the node count reads the subscription, so both
    // figures come from the file on disk rather than a stub.
    app.wait_for("EMBEDDED · 1.13.14").unwrap();
    // The seeded choice is named on the overview, which only holds once the file parses.
    app.wait_for("东京 01").unwrap();

    app.press("节点").unwrap();
    app.wait_for("东京 01").unwrap();
    app.wait_for("大阪 02").unwrap();
    app.wait_for("2 / 2 个节点 · 尚未测试延迟").unwrap();
}

/// The list is virtualized, so the filter has to narrow the data the list indexes into,
/// not merely hide rows: a search that leaves the count alone reads as broken.
#[test]
fn a_search_narrows_the_node_list() {
    let (mut app, _profile) = seeded_test("a_search_narrows_the_node_list", SEEDED_PROFILE);
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    app.wait_for("大阪 02").unwrap();

    app.type_text("搜索节点、地址或协议", "大阪").unwrap();
    app.wait_for("1 / 2 个节点 · 尚未测试延迟").unwrap();
    assert!(!app.shows("东京 01"), "the filtered row is still listed");

    app.clear("搜索节点、地址或协议").unwrap();
    app.type_text("搜索节点、地址或协议", "shadowsocks")
        .unwrap();
    app.wait_for("1 / 2 个节点 · 尚未测试延迟").unwrap();
    assert!(
        app.shows("大阪 02"),
        "matching on the protocol should find Shadowsocks"
    );

    app.clear("搜索节点、地址或协议").unwrap();
    app.type_text("搜索节点、地址或协议", "osaka").unwrap();
    app.wait_for("1 / 2 个节点 · 尚未测试延迟").unwrap();
    assert!(
        app.shows("大阪 02"),
        "matching on the address should find the host"
    );

    app.clear("搜索节点、地址或协议").unwrap();
    app.type_text("搜索节点、地址或协议", "没有这种东西")
        .unwrap();
    app.wait_for("没有匹配的节点").unwrap();
}

/// Choosing a node is the one decision this screen exists for, so the overview has to
/// show the new one. Selecting through the real list also exercises the virtualized
/// cells, whose rows are reused and read their node through a memo.
#[test]
fn picking_a_node_shows_it_on_the_overview() {
    let (mut app, _profile) =
        seeded_test("picking_a_node_shows_it_on_the_overview", SEEDED_PROFILE);
    app.wait_for("Kitty Pro").unwrap();
    // The seeded selection is node "a".
    app.wait_for("东京 01").unwrap();

    app.press("节点").unwrap();
    app.wait_for("大阪 02").unwrap();
    app.press("大阪 02").unwrap();

    app.press("总览").unwrap();
    app.wait_for("大阪 02").unwrap();
    assert!(
        !app.shows("东京 01"),
        "the overview still names the node that was replaced"
    );
}

/// Adding a rule, and being refused when it duplicates one already there.
///
/// The editor's fields are addressed by their labels, which is also the assertion that each
/// control is named: an input whose only name was a placeholder could not be found this way,
/// and a select with no trigger would have nothing to press.
#[test]
fn a_rule_is_added_and_a_duplicate_is_refused() {
    let (mut app, _profile) = desktop_test("a_rule_is_added_and_a_duplicate_is_refused");
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("添加规则").unwrap();

    app.type_text("label:匹配内容", "ad.example.com").unwrap();
    app.press("添加").unwrap();

    app.wait_for("1 条启用 · 1 条规则").unwrap();
    app.wait_for("ad.example.com").unwrap();
    app.wait_for("域名后缀").unwrap();
    app.wait_for("规则已保存").unwrap();
    // The list moved and the form cleared with it: the value is gone from the field, so the
    // next press cannot resubmit the rule that was just added.
    assert_eq!(app.screen().count("text:ad.example.com").unwrap(), 1);

    // The same match again: the store re-validates the whole list, which is what catches a
    // collision with another rule rather than only a bad value.
    app.type_text("label:匹配内容", "ad.example.com").unwrap();
    app.press("添加").unwrap();
    app.wait_for("已存在相同的匹配规则").unwrap();
    assert!(
        app.shows("1 条启用 · 1 条规则"),
        "a refused rule still reached the list"
    );
}

/// An empty value is not a rule, so the button declines rather than reporting a failure
/// after a press that could do nothing.
#[test]
fn an_empty_rule_cannot_be_submitted() {
    let (mut app, _profile) = desktop_test("an_empty_rule_cannot_be_submitted");
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("匹配内容").unwrap();
    assert!(
        !app.screen().enabled("添加").unwrap(),
        "添加 is offered with nothing to match on"
    );
    let error = app
        .press("添加")
        .expect_err("a refused press should report why");
    assert!(error.contains("disabled"), "unexpected refusal: {error}");
}

/// An armed delete that stays armed is a later single press turning into a silent deletion,
/// so the arming expires. `idle` pumps the engine's clock, which is what lets the timer run
/// out without the test sleeping.
#[test]
fn an_armed_delete_forgets_itself_when_left_alone() {
    let (mut app, _profile) = seeded_test(
        "an_armed_delete_forgets_itself_when_left_alone",
        PROFILE_WITH_RULE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("ad.example.com").unwrap();

    app.press("role:button[删除这条规则]").unwrap();
    app.wait_for("再次点击确认删除这条规则").unwrap();
    // The window is four seconds; give it room without making the test slow on a loaded CI.
    for _ in 0..12 {
        app.idle(std::time::Duration::from_millis(500)).unwrap();
        if app.screen().count("role:button[删除这条规则]").unwrap_or(0) == 1 {
            return;
        }
    }
    panic!("the armed delete never expired: {:?}", app.screen().texts());
}

/// A per-domain resolver is added through the same shape as a rule and shares the rule cap,
/// so the second tab gets its own check that the write and the row agree.
#[test]
fn a_dns_override_is_added_and_persisted() {
    let profile = Profile::claim("a_dns_override_is_added_and_persisted");
    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.press("专用 DNS").unwrap();
    app.wait_for("尚未配置专用 DNS").unwrap();

    app.type_text("label:域名后缀", "example.com").unwrap();
    app.type_text("label:DNS 服务器", "223.5.5.5").unwrap();
    app.press("添加").unwrap();

    app.wait_for("example.com").unwrap();
    app.wait_for("223.5.5.5").unwrap();
    app.wait_for("规则已保存").unwrap();
    // The row's own switch carries `enabled`, which is the flag the core reads. A row that
    // could not show it would be storing state the user cannot see.
    let toggle = "role:switch[启用 example.com 的专用 DNS]";
    assert_eq!(
        app.screen().accessibility_state(toggle).unwrap().checked,
        Some(nsh_core::Checked::True),
        "a new override should be on, and the row should say so"
    );
    wait_until_profile(|stored| stored.contains("223.5.5.5"), &mut app);
    drop(app);

    let mut again = desktop(&profile);
    again.wait_for("Kitty Pro").unwrap();
    again.press("规则").unwrap();
    again.press("专用 DNS").unwrap();
    again.wait_for("223.5.5.5").unwrap();
}

/// A malformed resolver is refused where it was typed, because a notice ages out while the
/// mistake stays in the field.
#[test]
fn a_bad_dns_server_is_refused_in_the_field() {
    let (mut app, _profile) = desktop_test("a_bad_dns_server_is_refused_in_the_field");
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.press("专用 DNS").unwrap();
    app.type_text("label:域名后缀", "example.com").unwrap();
    // Spaces survive trim and rule out both an address and a hostname; "not-a-resolver"
    // would not, because that is a legal host name.
    app.type_text("label:DNS 服务器", "not a resolver").unwrap();
    app.press("添加").unwrap();
    app.wait_for("DNS 服务器必须是 IP 或域名").unwrap();
    assert!(!app.shows("规则已保存"));
}

/// Deleting needs a second press, and the first one only arms the row. The window is what
/// makes the pattern safe: an armed row that stayed armed would turn a later single press
/// into a silent deletion.
#[test]
fn deleting_a_rule_asks_twice_and_only_the_second_press_removes_it() {
    let (mut app, _profile) = seeded_test(
        "deleting_a_rule_asks_twice_and_only_the_second_press_removes_it",
        PROFILE_WITH_RULE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("ad.example.com").unwrap();
    let trash = "role:button[删除这条规则]";
    app.press(trash).unwrap();
    // The button's own name changes, which is how the arming reaches someone who cannot see
    // the row, and how this test knows it is pressing the armed control.
    app.wait_for("再次点击确认删除这条规则").unwrap();
    assert!(
        app.shows("ad.example.com"),
        "the first press already removed the rule"
    );

    app.press("role:button[再次点击确认删除这条规则]").unwrap();
    app.wait_for("暂无自定义规则").unwrap();
    app.wait_for("0 条启用 · 0 条规则").unwrap();
}

/// A subscription's nodes go with it and cannot be recovered without the provider, so the
/// deletion asks in a native alert naming the one being removed.
#[test]
fn deleting_a_subscription_asks_first_and_names_it() {
    let (mut app, _profile) = seeded_test(
        "deleting_a_subscription_asks_first_and_names_it",
        SEEDED_PROFILE,
    );
    app.wait_for("Kitty Pro").unwrap();
    // Left to itself the driver answers a dialog and picks cancel, so the app would never
    // see the press this test means to make.
    app.answer_dialogs_myself();
    app.press("订阅").unwrap();
    app.wait_for("实验室").unwrap();
    app.press("role:button[删除 实验室]").unwrap();

    let dialog = app
        .pending_dialog()
        .expect("the delete should ask before it acts");
    assert_eq!(
        dialog.title.as_deref(),
        Some("删除「实验室」？"),
        "the alert should name what it is about to remove"
    );
    app.answer_dialog(nsh_testing::DialogAnswer::Press("删除".to_owned()))
        .unwrap();

    app.wait_for("暂无订阅").unwrap();
    app.wait_for("已删除 实验室").unwrap();
}

/// Answering "取消" has to leave the subscription where it was: the alert is the only thing
/// standing between a press and an irreversible loss.
#[test]
fn cancelling_a_delete_keeps_the_subscription() {
    let (mut app, _profile) =
        seeded_test("cancelling_a_delete_keeps_the_subscription", SEEDED_PROFILE);
    app.wait_for("Kitty Pro").unwrap();
    app.answer_dialogs_myself();
    app.press("订阅").unwrap();
    app.press("role:button[删除 实验室]").unwrap();
    app.answer_dialog(nsh_testing::DialogAnswer::Press("取消".to_owned()))
        .unwrap();
    app.wait_for("实验室").unwrap();
    assert!(!app.shows("已删除 实验室"));
}

/// Every new page on a phone. The bottom bar sits inside the content column, and a layout
/// that moved it beside the page would squeeze these to nothing, which a text check sees
/// because a clipped view reports no text.
#[test]
fn the_new_pages_hold_their_layout_on_a_phone() {
    let (mut app, _profile) =
        seeded_test("the_new_pages_hold_their_layout_on_a_phone", SEEDED_PROFILE);
    app.wait_for("Kitty Pro").unwrap();
    for (tab, witness) in [
        ("订阅", "当前使用"),
        ("规则", "添加规则"),
        ("日志", "缓冲区 0 条记录 · 当前显示 0 条"),
    ] {
        app.press(tab).unwrap();
        app.wait_for(witness)
            .unwrap_or_else(|_| panic!("{tab} lost its content at a phone width"));
    }
}

/// The config script lives on Settings because enabling it takes the whole rule set over.
/// Its editor opens over the saved text, and a blank script cannot be switched on.
#[test]
fn the_script_editor_opens_over_the_saved_text() {
    let (mut app, _profile) = seeded_test(
        "the_script_editor_opens_over_the_saved_text",
        r#"{
          "config_script": "function main(config) { return config; }",
          "config_script_enabled": false
        }"#,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("JavaScript 配置覆写").unwrap();
    app.wait_for("main(config) · QuickJS · 40 字符").unwrap();
    app.press("编辑").unwrap();
    app.wait_for("收起").unwrap();
    // Only Settings carries a switch, so this names the script's own. It is off because the
    // file says so, and it stays pressable because the script is not empty.
    let toggle = "role:switch[启用 JavaScript 配置覆写]";
    app.wait_for(toggle).unwrap();
    assert!(app.screen().enabled(toggle).unwrap());
    assert_eq!(
        app.screen().accessibility_state(toggle).unwrap().checked,
        Some(nsh_core::Checked::False),
        "loading a stored script switched it on"
    );
}

/// The profile is written back on the way in, so a relaunch keeps the choice. This is
/// the round trip that a store-only unit test cannot show.
#[test]
fn a_choice_survives_a_relaunch() {
    let profile = Profile::claim("a_choice_survives_a_relaunch");
    profile.seed(SEEDED_PROFILE);
    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    app.wait_for("大阪 02").unwrap();

    app.press("大阪 02").unwrap();
    // The write is debounced, so wait for the selection to land in the file rather than
    // guessing at a delay. Reading it back is the assertion; a fixed sleep would pass on a
    // fast machine and fail on a loaded one.
    wait_until_profile(
        |stored| stored.contains("大阪") || stored.contains("\"b\""),
        &mut app,
    );
    drop(app);

    // A second engine over the same directory, still inside this test's claim.
    let mut again = desktop(&profile);
    again.wait_for("大阪 02").unwrap();
}

/// Pump the app until its profile file says `wanted`, and fail if it never does. The engine
/// runs on the system clock, so `idle` is what lets the debounced write happen; a fixed
/// sleep would pass on a fast machine and fail on a loaded one.
fn wait_until_profile(wanted: impl Fn(&str) -> bool, app: &mut Headless) {
    let path = std::path::PathBuf::from(
        std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
    )
    .join("profile.json");
    for _ in 0..40 {
        if std::fs::read_to_string(&path).is_ok_and(|text| wanted(&text)) {
            return;
        }
        app.idle(std::time::Duration::from_millis(100)).unwrap();
    }
    panic!("the profile at {path:?} never reached the expected state");
}

/// Connecting with nothing to route to is a guaranteed failure, so the control declines
/// the press rather than reporting an error after it. Disconnecting stays available even
/// when the core stops answering, which is exactly when a user needs it.
#[test]
fn connect_is_off_until_there_is_something_to_connect_to() {
    let profile = Profile::claim("connect_is_off_until_there_is_something_to_connect_to");
    let button = "role:button[启动代理]";

    let mut empty = desktop(&profile);
    empty.wait_for("内核已就绪").unwrap();
    empty.wait_for(button).unwrap();
    assert!(
        !empty.screen().enabled(button).unwrap(),
        "an app with no nodes should not offer to connect"
    );
    drop(empty);

    profile.seed(SEEDED_PROFILE);
    let mut seeded = desktop(&profile);
    seeded.wait_for("内核已就绪").unwrap();
    seeded.wait_for(button).unwrap();
    assert!(
        seeded.screen().enabled(button).unwrap(),
        "a profile with nodes should offer to connect"
    );
}

/// A binding recomputes when the store moves; a value read while building does not. This
/// checks the header's live text through the mode switch, which is the same class of bug
/// as the connect button's label and, unlike that press, has no side effects: with no core
/// running, changing the mode only writes the scratch profile.
#[test]
fn switching_the_mode_updates_the_live_text_around_it() {
    let (mut app, _profile) = seeded_test(
        "switching_the_mode_updates_the_live_text_around_it",
        SEEDED_PROFILE,
    );
    app.wait_for("内核已就绪").unwrap();
    app.wait_for("· 规则模式").unwrap();
    app.press("设置").unwrap();
    app.wait_for("全部流量走代理").unwrap();
    // The row's own pill, not the sidebar or the header: three rows share the label.
    // The three rows share the "启用" label, so the press names its row by position:
    // rule, global, direct.
    app.press("role:button[启用] >> nth=1").unwrap();

    // The header's subtitle is one text node, and it moved with the store.
    app.wait_for("内核已就绪 · 全局模式").unwrap();
    assert!(
        !app.shows("内核已就绪 · 规则模式"),
        "the header kept the mode that was replaced"
    );
}

/// A press the control has refused must not reach the core. The connect path is otherwise
/// deliberately untested: it would start the sing-box running on the machine, so the
/// assertions here stop at the guard rather than crossing it.
#[test]
fn a_refused_press_starts_nothing() {
    let (mut app, _profile) = desktop_test("a_refused_press_starts_nothing");
    app.wait_for("内核已就绪").unwrap();
    let button = "role:button[启动代理]";
    assert!(!app.screen().enabled(button).unwrap());

    // The press is refused outright, which is what keeps this test from ever reaching
    // the core.
    let error = app
        .press(button)
        .expect_err("a refused press should report why");
    assert!(error.contains("disabled"), "unexpected refusal: {error}");
    assert!(!app.shows("正在启动"));
}
