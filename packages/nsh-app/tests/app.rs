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
use nsh_core::MountItem;
use nsh_engine::{ColorScheme, EngineConfig, Environment, Idiom, Os};
use nsh_testing::Headless;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
        // The update card checks the release feed on launch, and that check speaks HTTP through
        // reqwest rather than NSH's `fetch`, so the suite's `StubNetwork` cannot intercept it.
        // A state file stamped a minute ago holds the daily throttle, which keeps every test in
        // this file offline without changing what the card renders. A test that wants the other
        // state writes its own.
        std::fs::write(
            dir.join("update.json"),
            format!(
                r#"{{"settings":{{"auto_check":true,"last_check_at":{}}},"latest":null,"snoozed_version":""}}"#,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_secs())
                    - 60
            ),
        )
        .expect("a seeded update state");
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

/// A desktop window tall enough to show the whole settings page at once. The port's label and
/// the latency row sit below a laptop-height fold, which is normal for a scrolling page; a test
/// that asserts their text should not have to fight the scroll position to read it.
fn tall(profile: &Profile) -> Headless {
    windowed(profile, 1600.0)
}

/// [`tall`], extended to the length the settings page has now grown to: it ends in the about
/// card, and a test reading that card's figures needs them measured.
fn full_height(profile: &Profile) -> Headless {
    windowed(profile, 2400.0)
}

fn windowed(profile: &Profile, height: f32) -> Headless {
    let _ = profile;
    Headless::start(EngineConfig {
        viewport: Size::new(px(1280.0), px(height)),
        environment: Environment {
            window: Size::new(px(1280.0), px(height)),
            ..environment()
        },
        ..EngineConfig::new(nsh_app_kitty::app_root())
    })
    .expect("the app starts")
}

/// A phone surface, to prove the layout survives a narrow width.
fn phone(profile: &Profile) -> Headless {
    let _ = profile;
    Headless::start(EngineConfig::new(nsh_app_kitty::app_root())).expect("the app starts")
}

/// Whether any view on screen matches a selector.
///
/// Separate from `Headless::shows` because the two take different things: `shows` compares a
/// string against whole text nodes after trimming, so it cannot see a word inside a longer
/// sentence. Asserting that some phrase is *absent* therefore needs a selector — a `shows` call
/// on a phrase that only ever appears inside another node is true no matter what is on screen.
/// `text:/…/` is the regex form.
fn shows_matching(app: &Headless, selector: &str) -> bool {
    app.screen()
        .matches(selector)
        .unwrap_or_else(|error| panic!("`{selector}`: {error}"))
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

/// Open the rule form through the page's floating disc, as a user does.
///
/// The form is hidden until it is pressed, so every test that edits a rule has to ask for the
/// form first — and waiting on the field's own label is what proves the press opened it rather
/// than that the previous screen happened to show the word.
fn open_rule_form(app: &mut Headless) {
    app.press("role:button[添加规则]").unwrap();
    app.wait_for("匹配内容").unwrap();
}

/// Arrive on 规则 with its list built, then open the form.
///
/// The wait is on the empty-state or the tab's own heading rather than on the form: the disc
/// carries an icon and no visible text, so `添加规则` is a name the driver can press by and
/// nothing a `wait_for` on screen text can see.
fn rules_page_with_form(app: &mut Headless) {
    app.press("规则").unwrap();
    app.wait_for("分流规则").unwrap();
    open_rule_form(app);
}

#[test]
fn the_rules_page_lists_nothing_and_keeps_its_form_closed() {
    let (mut app, _profile) =
        desktop_test("the_rules_page_lists_nothing_and_keeps_its_form_closed");
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("分流规则").unwrap();
    app.wait_for("专用 DNS").unwrap();
    app.wait_for("暂无自定义规则").unwrap();
    app.wait_for("0 条启用 · 0 条规则").unwrap();

    // The form used to sit above the list permanently, which made the page read as a form with
    // some rows under it. It is now opened by the disc, so on arrival the fields are absent —
    // asserted through the inputs themselves, since a text node that never existed satisfies
    // nothing.
    assert_eq!(
        app.screen().count("label:匹配内容").unwrap(),
        0,
        "the rule editor is open before anything asked for it"
    );

    // And one press brings it, with both selects drawing their triggers: an unstyled select has
    // nothing to press and would leave these names unsaid.
    open_rule_form(&mut app);
    for field in ["匹配类型", "匹配内容", "动作"] {
        assert!(
            app.shows(field),
            "the opened editor is missing its {field} field"
        );
    }
    // A draft rule cannot be added, so the page does not pretend otherwise.
    assert!(!app.screen().enabled("添加").unwrap());

    // The same disc closes it again, and its glyph says which the next press does.
    app.press("role:button[收起规则表单]").unwrap();
    app.wait_for_gone("匹配内容", std::time::Duration::from_secs(2))
        .expect("the disc closed the form and its own label");
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
    // The tab's form is hidden until its disc opens it, so the proof this tab is drawn is its
    // own empty state and heading — the fields come only on request, on both tabs.
    assert_eq!(
        app.screen().count("label:域名后缀").unwrap(),
        0,
        "the DNS tab shows its form before anything asked for it"
    );
    app.press("role:button[添加专用 DNS]").unwrap();
    app.wait_for("域名后缀").unwrap();
    app.wait_for("DNS 服务器").unwrap();
    assert!(
        !app.shows("暂无自定义规则"),
        "the routing tab stayed drawn under the DNS tab"
    );

    app.press("分流规则").unwrap();
    app.wait_for("暂无自定义规则").unwrap();
    assert!(!app.shows("尚未配置专用 DNS"));
    // Back on routing, its own form is closed too: each tab keeps its own draft, and neither is
    // open until asked. The positive check below is what makes the negative one mean something
    // — a selector that matched no view at all would satisfy `== 0` forever.
    app.press("role:button[添加规则]").unwrap();
    app.wait_for("匹配内容").unwrap();
    assert!(
        app.screen().count("label:匹配内容").unwrap() > 0,
        "label:匹配内容 matches nothing even with the form open, so the check above proves \
         nothing"
    );
    app.press("role:button[收起规则表单]").unwrap();
    app.wait_for_gone("匹配内容", std::time::Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        app.screen().count("label:匹配内容").unwrap(),
        0,
        "switching tabs left the rule form open"
    );
    assert!(
        !shows_matching(&app, "role:button[收起专用 DNS 表单]"),
        "the disc still names the other tab's form"
    );
}

/// Arrive on the DNS tab with its form open, as a user does: the tab's disc, then the field.
fn dns_tab_with_form(app: &mut Headless) {
    app.press("专用 DNS").unwrap();
    app.press("role:button[添加专用 DNS]").unwrap();
    app.wait_for("域名后缀").unwrap();
}

#[test]
fn the_subscriptions_page_asks_for_the_first_one() {
    let (mut app, _profile) = desktop_test("the_subscriptions_page_asks_for_the_first_one");
    app.wait_for("Kitty Pro").unwrap();
    app.press("订阅").unwrap();
    app.wait_for("暂无订阅").unwrap();
    app.wait_for("还没有订阅").unwrap();
    // One way in, from the disc: the empty state and the toolbar used to carry their own
    // 添加订阅 buttons, which made three controls for one dialog once the page grew a floating
    // action. The empty state now explains what a subscription is for and leaves the pressing
    // to the disc.
    assert_eq!(
        app.screen().count("role:button[添加订阅]").unwrap(),
        1,
        "the add action is offered more than once on 订阅"
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

/// What the engine handed the platform to draw: the view tree, each view's frame and its
/// visual style.
///
/// This exists because a `Screen` answers questions about *content* — text, selectors, a11y
/// state — and nothing about appearance. Every switch in the app was visibly broken while the
/// suite stayed green: the thumb never slid, and the checked switch painted an orange rectangle
/// around a floating pill. Both are visible in the item stream a real backend receives, so the
/// stream is recorded here and read back.
///
/// The tree is only as complete as it needs to be: `Insert` order is kept by appending, which
/// is the tree order for a control whose parts are created once and never reordered.
#[derive(Default)]
struct Drawn {
    parent: HashMap<u32, u32>,
    children: HashMap<u32, Vec<u32>>,
    frames: HashMap<u32, nsh_core::Bounds<nsh_core::Points>>,
    styles: HashMap<u32, nsh_core::ViewStyle>,
    /// What each text view was told to draw. Position claims about a *specific* string — the
    /// toast, say — need the node that holds it, and the driver's own queries stop at
    /// `Screen::texts`, which reports what is on screen but not where.
    texts: HashMap<u32, String>,
}

impl Drawn {
    /// The node drawing `needle`, if one is in the record.
    fn node_with_text(&self, needle: &str) -> Option<u32> {
        self.texts
            .iter()
            .find(|(_, text)| text.contains(needle))
            .map(|(id, _)| *id)
    }

    /// The parts inside `id`, in tree order.
    fn children(&self, id: u32) -> &[u32] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The style a view was given, or the engine's default for one that never got one.
    fn style(&self, id: u32) -> nsh_core::ViewStyle {
        self.styles
            .get(&id)
            .cloned()
            .unwrap_or(nsh_core::ViewStyle::default())
    }

    fn frame(&self, id: u32) -> nsh_core::Bounds<nsh_core::Points> {
        self.frames[&id]
    }

    /// Where the view sits in window space. `MountItem::Frame` is relative to the superview,
    /// so frames from different levels cannot be compared until they are walked up to the root.
    fn origin_in_root(&self, id: u32) -> (f32, f32) {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut current = id;
        // The root's own frame is its only ancestor, and the chain is short: the deepest
        // control here is a few views wide. The guard is against a cycle in the record, which
        // would otherwise hang the suite rather than fail it.
        for _ in 0..64 {
            let Some(frame) = self.frames.get(&current) else {
                break;
            };
            x += frame.origin.x.0;
            y += frame.origin.y.0;
            match self.parent.get(&current) {
                Some(parent) => current = *parent,
                None => break,
            }
        }
        (x, y)
    }

    /// The view's centre in window space.
    fn center_in_root(&self, id: u32) -> (f32, f32) {
        let (x, y) = self.origin_in_root(id);
        let size = self.frame(id).size;
        (x + size.width.0 / 2.0, y + size.height.0 / 2.0)
    }

    /// How far the view is slid after layout, along x.
    fn translated_x(&self, id: u32) -> f32 {
        self.style(id)
            .transform
            .ops()
            .iter()
            .find(|op| op.kind == nsh_core::TransformKind::TranslateX)
            .map(|op| op.value)
            .unwrap_or(0.0)
    }
}

/// Attach [`Drawn`] to an engine. Must be called before the batches are applied, so a test
/// uses it in place of the plain `desktop`/`tall` helper.
fn observing(mut app: Headless) -> (Headless, Arc<Mutex<Drawn>>) {
    let drawn = Arc::new(Mutex::new(Drawn::default()));
    let sink = drawn.clone();
    // The observer has to be `Send`, so the record lives behind a mutex rather than in a
    // `RefCell`; nothing contends for it, since the driver applies batches on this thread.
    app.observe(move |_, batch| {
        let Ok(mut drawn) = sink.lock() else { return };
        for item in &batch.items {
            match item {
                MountItem::Insert { parent, child, .. } => {
                    drawn.parent.insert(*child, *parent);
                    let kids = drawn.children.entry(*parent).or_default();
                    if !kids.contains(child) {
                        kids.push(*child);
                    }
                }
                MountItem::Remove { id } => {
                    if let Some(parent) = drawn.parent.get(id).copied() {
                        if let Some(kids) = drawn.children.get_mut(&parent) {
                            kids.retain(|kid| kid != id);
                        }
                    }
                }
                MountItem::Frame { id, frame } => {
                    drawn.frames.insert(*id, *frame);
                }
                MountItem::ViewStyle { id, style } => {
                    drawn.styles.insert(*id, (**style).clone());
                }
                MountItem::Text { id, text, .. } => {
                    drawn.texts.insert(*id, text.text.clone());
                }
                _ => {}
            }
        }
    });
    (app, drawn)
}

/// The track and the thumb of the switch a selector names, as the platform received them.
fn switch_parts(drawn: &Drawn, switch: u32) -> (u32, u32) {
    let &track = drawn
        .children(switch)
        .first()
        .expect("a switch always has a track");
    let &thumb = drawn
        .children(track)
        .first()
        .expect("a track always has a thumb");
    (track, thumb)
}

/// The parts of a switch, as the platform received them.
struct SwitchDrawn {
    track: nsh_core::Bounds<nsh_core::Points>,
    thumb: nsh_core::Bounds<nsh_core::Points>,
    /// How far the thumb is slid along x.
    slide: f32,
    track_fill: nsh_core::Color,
    thumb_fill: nsh_core::Color,
    box_fill: nsh_core::Color,
    thumb_shadowed: bool,
}

impl SwitchDrawn {
    fn read(drawn: &Drawn, switch: u32) -> Self {
        let (track, thumb) = switch_parts(drawn, switch);
        let track_style = drawn.style(track);
        let thumb_style = drawn.style(thumb);
        Self {
            track: drawn.frame(track),
            thumb: drawn.frame(thumb),
            slide: drawn.translated_x(thumb),
            track_fill: track_style.background_color,
            thumb_fill: thumb_style.background_color,
            box_fill: drawn.style(switch).background_color,
            thumb_shadowed: thumb_style
                .effects
                .as_ref()
                .is_some_and(|fx| fx.box_shadows.iter().any(|layer| !layer.inset)),
        }
    }
}

/// The bug this pins: every switch in the app was broken in the same way and every test passed,
/// because a `Switch` has no look of its own — the parts are styled by the app, and a part that
/// is styled at the wrong level simply draws somewhere else. `checked_style` paints the switch's
/// own box, so using it for the checked fill drew an orange rectangle *around* the track; an
/// unsized track filled that box, which centred the thumb and left it with no room to slide.
///
/// These are assertions about the item stream, which is what a real platform draws from, so they
/// are the closest this suite can get to looking at the screen.
#[test]
fn a_switch_draws_a_track_and_a_thumb_that_moves() {
    let profile = Profile::claim("a_switch_draws_a_track_and_a_thumb_that_moves");
    let (mut app, sink) = observing(windowed(&profile, 2400.0));
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("接管方式").unwrap();

    let toggle = "role:switch[允许局域网连接]";
    let switch = app
        .screen()
        .press_target(toggle)
        .unwrap()
        .expect("the switch is pressable");

    let off = SwitchDrawn::read(&sink.lock().unwrap(), switch);
    // The track is the whole switch: a part that fills its parent is an unsized part.
    assert_eq!(
        (off.track.size.width.0, off.track.size.height.0),
        (40.0, 23.0)
    );
    // The thumb is the original's 16pt circle, and starts at the near end.
    assert_eq!(
        (off.thumb.size.width.0, off.thumb.size.height.0),
        (16.0, 16.0)
    );
    assert_eq!(off.slide, 0.0, "an off switch must not be slid over");
    // The switch's own box paints nothing, so a checked switch cannot surround the track with
    // a rectangle. The engine sends a transparent fill as black with no alpha, so alpha is the
    // only honest reading of "draws nothing".
    assert_eq!(
        off.box_fill.to_f64().3,
        0.0,
        "the switch's own box is painted: it draws behind the track"
    );
    // The thumb is lifted off the track by a shadow. In the light appearance the thumb is
    // #ffffff over a track that is an 82% white, so without this layer an off switch is a
    // blank pill: the state is invisible even though every style here is "correct".
    assert!(
        off.thumb_shadowed,
        "the thumb has no drop shadow, so it cannot be told apart from the track"
    );
    // And the thumb carries its own opaque role rather than the card color: --card is a 68%
    // white over the track's 82%, which composites to within a few percent of the track and
    // reads as no thumb at all. This is the second half of the same visible bug.
    assert_eq!(
        hex(off.thumb_fill),
        "ffffff",
        "the thumb is not drawing --switch-thumb"
    );
    assert_eq!(
        off.thumb_fill.to_f64().3,
        1.0,
        "a translucent thumb shows the track through it"
    );

    app.press(toggle).unwrap();
    app.wait_for("局域网内其他设备可使用 7890").unwrap();
    let on = SwitchDrawn::read(&sink.lock().unwrap(), switch);

    // The track takes the accent in the checked state, and the box still paints nothing.
    assert_eq!(
        hex(on.track_fill),
        "a85208",
        "the checked track is not --primary"
    );
    assert_eq!(
        on.box_fill.to_f64().3,
        0.0,
        "checking the switch painted its box rather than its track"
    );
    // The thumb is where the slide puts it: 16pt along, `inset` from the far end and no
    // further, so the switch reads as on without the thumb escaping the track.
    assert_eq!(
        on.slide, 16.0,
        "the thumb never moved: a checked switch looks exactly like an off one"
    );
    assert!(
        on.thumb.origin.x.0 + on.slide + on.thumb.size.width.0
            <= on.track.size.width.0 - 2.0 + 1e-3,
        "the thumb leaves the track when checked: x {} + slide {} + {} in a track of {}",
        on.thumb.origin.x.0,
        on.slide,
        on.thumb.size.width.0,
        on.track.size.width.0
    );
    // And the track's size is unchanged by the state, so nothing around the switch shifts.
    assert_eq!(off.track, on.track, "the track changed size between states");
    // The a11y value moved with the drawing, so the two cannot disagree.
    assert_eq!(
        app.screen().accessibility_state(toggle).unwrap().checked,
        Some(nsh_core::Checked::True)
    );
}

/// Every control in a mutually exclusive group, as the platform received it.
struct Painted {
    id: u32,
    fill: nsh_core::Color,
}

/// The view a selector names, plus what it is filled with.
///
/// `press_target` is used rather than a raw resolve because it takes the same selectors the
/// rest of the suite presses, and it refuses a match that cannot be pressed.
fn painted(app: &Headless, drawn: &Arc<Mutex<Drawn>>, selector: &str) -> Painted {
    let id = app
        .screen()
        .press_target(selector)
        .unwrap_or_else(|error| panic!("`{selector}`: {error}"))
        .unwrap_or_else(|| panic!("`{selector}` names nothing pressable"));
    Painted {
        id,
        fill: drawn.lock().unwrap().style(id).background_color,
    }
}

/// The general form of the bug that broke every switch: a control carries a style for its
/// selected state, and the style reaches no view that draws anything, so the group looks the
/// same whichever member is chosen — and every text assertion still passes because the text is
/// fine. NSH's own docs warn of this shape for a `Switch` in TSX too ("draws nothing, and its
/// test still passes"), so it is a property of the engine, not of the language the UI is
/// written in.
///
/// The invariant: in a group where exactly one member is chosen, exactly one member is painted,
/// and it is the chosen one. A pill whose `selected_style` never lands is unpainted in both
/// states; one whose style lands on the wrong view paints a neighbour.
fn assert_exactly_one_painted(app: &Headless, drawn: &Arc<Mutex<Drawn>>, group: &[&str]) {
    let seen: Vec<(&str, Painted)> = group
        .iter()
        .map(|selector| (*selector, painted(app, drawn, selector)))
        .collect();
    // Each name has to reach its own view. If two of them resolved to the same node the check
    // below would compare a view with itself and pass on any styling at all.
    let distinct: std::collections::HashSet<u32> = seen.iter().map(|(_, p)| p.id).collect();
    assert_eq!(
        distinct.len(),
        group.len(),
        "`{group:?}` does not name that many different views"
    );
    let lit: Vec<&str> = seen
        .iter()
        .filter(|(_, p)| p.fill.to_f64().3 > 0.0)
        .map(|(selector, _)| *selector)
        .collect();
    let chosen: Vec<&str> = group
        .iter()
        .filter(|selector| {
            app.screen()
                .selected(selector)
                .unwrap_or_else(|error| panic!("`{selector}`: {error}"))
        })
        .copied()
        .collect();
    assert_eq!(
        chosen.len(),
        1,
        "a one-of-n group should have exactly one selected member, found {chosen:?}"
    );
    assert_eq!(
        lit, chosen,
        "the drawn state does not follow the real one: selected is {chosen:?} but painted is {lit:?}"
    );
}

/// The tally strip on the log page is five pills whose only difference when chosen is a
/// background, so it is the most direct test of the invariant above. It also has to move: a
/// group that is painted in the initial state only would pass a single read.
#[test]
fn a_chosen_tally_pill_is_the_one_that_is_painted() {
    let profile = Profile::claim("a_chosen_tally_pill_is_the_one_that_is_painted");
    let (mut app, sink) = observing(desktop(&profile));
    app.wait_for("Kitty Pro").unwrap();
    app.press("日志").unwrap();
    app.wait_for("已停止").unwrap();

    let all = "role:button[全部路由 0]";
    let blocked = "role:button[拦截 0]";
    let strip = [all, "role:button[代理 0]", "role:button[直连 0]", blocked];
    assert_exactly_one_painted(&app, &sink, &strip);

    app.press(blocked).unwrap();
    app.wait_for("缓冲区 0 条记录 · 当前显示 0 条").unwrap();
    assert!(
        app.screen().selected(blocked).unwrap(),
        "the press did not move the choice at all"
    );
    assert_exactly_one_painted(&app, &sink, &strip);
}

/// Each page carries its own single floating action, and no page carries another's.
///
/// The user asked for exactly this list after a global bar was tried and rejected: 总览 starts
/// the proxy, 节点 measures latency, 规则 adds a rule, 日志 toggles collection, 设置 has none.
/// Asserted per screen with the page's own text as the wait — counting a control before its
/// screen is built reads the previous one and passes for the wrong reason — and asserted by
/// absence as well as presence, because a bar that leaked 启动代理 onto every page would satisfy
/// a presence check on each of them.
#[test]
fn each_page_carries_its_own_floating_action_and_nobody_elses() {
    let profile = Profile::claim("each_page_carries_its_own_floating_action_and_nobody_elses");
    profile.seed(SEEDED_PROFILE);
    let (mut app, _sink) = observing(windowed(&profile, 2400.0));
    app.wait_for("东京 01").unwrap();

    // (page, marker that proves it is built, the one action it owns, actions it must not show)
    let pages: [(&str, &str, &str, &[&str]); 6] = [
        (
            "总览",
            "近一分钟流量",
            "启动代理",
            &["刷新延迟", "添加规则", "暂停采集日志", "添加订阅"],
        ),
        (
            "节点",
            "搜索节点、地址或协议",
            "刷新延迟",
            &["启动代理", "添加规则", "添加订阅"],
        ),
        (
            "订阅",
            "1 个订阅 · 共 2 个节点",
            "添加订阅",
            &["启动代理", "刷新延迟", "添加规则"],
        ),
        (
            "规则",
            "分流规则",
            "添加规则",
            &["启动代理", "刷新延迟", "添加订阅"],
        ),
        (
            "日志",
            "已停止",
            "内核未运行，无法采集日志",
            &["启动代理", "刷新延迟", "添加规则"],
        ),
        (
            "设置",
            "接管方式",
            "",
            &[
                "启动代理",
                "刷新延迟",
                "添加规则",
                "添加订阅",
                "暂停采集日志",
            ],
        ),
    ];

    for (screen, marker, own, forbidden) in pages {
        app.press(screen).unwrap();
        app.wait_for(marker).unwrap();
        if !own.is_empty() {
            assert_eq!(
                app.screen().count(&format!("role:button[{own}]")).unwrap(),
                1,
                "`{screen}` should own exactly one {own}; buttons: {} role:button total, label-hit {}",
                app.screen().count("role:button").unwrap(),
                app.screen().count(&format!("label:{own}")).unwrap_or(999)
            );
        }
        for action in forbidden {
            assert_eq!(
                app.screen()
                    .count(&format!("role:button[{action}]"))
                    .unwrap(),
                0,
                "`{screen}` shows {action}, which belongs to another page"
            );
        }
    }
}

/// The floating action is a disc, not a bar, and sits bottom-right of its own page.
///
/// It used to be a 56pt slab the width of the overview card — 934pt inside 976pt — which reads
/// as a progress track; the icon-only version is 56pt square. Checked through the drawn frames
/// rather than a class name, because the failure mode is invisible to a text test: every label
/// present, every press correct, and the control spanning the window or parked in a corner of
/// the rail. Frames are relative to each superview, so the chain is walked to the root before
/// comparing against the viewport.
#[test]
fn the_floating_action_is_a_disc_at_the_bottom_right() {
    let profile = Profile::claim("the_floating_action_is_a_disc_at_the_bottom_right");
    profile.seed(SEEDED_PROFILE);
    let (mut app, sink) = observing(windowed(&profile, 900.0));
    app.wait_for("东京 01").unwrap();

    let button = app
        .screen()
        .press_target("role:button[启动代理]")
        .unwrap()
        .expect("the seeded profile can connect");
    let drawn = sink.lock().unwrap();
    let size = drawn.frame(button).size;

    // A circle: equal sides, the width of the glyph rather than of a sentence.
    let disc = nsh_app_kitty::ui::controls::ACTION_DISC;
    assert_eq!(
        (size.width.0, size.height.0),
        (disc, disc),
        "the floating action is not the square disc it should be"
    );
    // Bottom-right of the window, and clear of the rail on the left.
    let (center_x, center_y) = drawn.center_in_root(button);
    assert!(
        center_x > 1280.0 * 0.7,
        "x {center_x} of a 1280pt window: not the trailing side"
    );
    // And off the edge, not against it. "Somewhere near the bottom" was the old check and it
    // passed while the disc was flush with the border, which is what the user reported; the
    // distance is what matters, and `p_5`'s own padding was being overwritten by the safe-area
    // inset (0 on a desktop), so the gap had to be asserted to be defended.
    let edge = nsh_app_kitty::ui::controls::ACTION_EDGE;
    let bottom_gap = 900.0 - (center_y + size.height.0 / 2.0);
    assert!(
        bottom_gap >= edge - 1.0,
        "the disc is {bottom_gap:.1}pt from the bottom border: it should keep {edge}pt"
    );
    let right_gap = 1280.0 - (center_x + size.width.0 / 2.0);
    assert!(
        right_gap >= 20.0 - 1.0,
        "the disc is {right_gap:.1}pt from the right border"
    );
}

/// The disc owns the form's life: one press opens it, the next closes it, and closing discards
/// what was typed — a draft left behind would be submitted against the next rule by accident.
#[test]
fn the_floating_action_opens_and_closes_the_rule_form() {
    let profile = Profile::claim("the_floating_action_opens_and_closes_the_rule_form");
    profile.seed(PROFILE_WITH_RULE);
    let (mut app, _sink) = observing(windowed(&profile, 2400.0));
    app.wait_for("Kitty Pro").unwrap();
    app.press("规则").unwrap();
    app.wait_for("分流规则").unwrap();

    open_rule_form(&mut app);
    app.type_text("匹配内容", "manual.example.com").unwrap();
    assert_eq!(
        app.screen().input_text("匹配内容").unwrap(),
        Some("manual.example.com"),
        "nothing was typed into the opened form"
    );

    // The glyph flipped with the state, so the second press is the one that closes.
    app.press("role:button[收起规则表单]").unwrap();
    app.wait_for_gone("匹配内容", std::time::Duration::from_secs(2))
        .expect("the form stayed open after the disc closed it");

    open_rule_form(&mut app);
    assert_eq!(
        app.screen().input_text("匹配内容").unwrap(),
        Some(""),
        "closing the form left the abandoned draft in it"
    );
}

/// 日志's action is a real switch: pausing stops the drain, and the indicator follows. The core
/// streams logs only while asked, so this is not a scroll lock dressed up as a button.
#[test]
fn pausing_collection_from_the_floating_action_stops_the_feed() {
    let profile = Profile::claim("pausing_collection_from_the_floating_action_stops_the_feed");
    let (mut app, _sink) = observing(desktop(&profile));
    app.wait_for("Kitty Pro").unwrap();
    app.press("日志").unwrap();
    app.wait_for("已停止").unwrap();

    // No core running, so the action is parked rather than offering a press that changes
    // nothing — and the chip says 已停止, which is the truth about the core, not about the
    // reader's pause.
    assert!(
        !app.screen()
            .enabled("role:button[内核未运行，无法采集日志]")
            .unwrap(),
        "the log action is offered with no core to collect from"
    );
}

/// The disc is neutral while nothing runs and lights up only when the proxy does.
///
/// The first cut was always orange, which shouted the same thing whether the core was up or not
/// and spent the one colour that could carry a state. `packages/ui` draws its power button on an
/// inset surface with no glow, and switches it to the accent with a glow and a halo when active;
/// that is what this checks, through the styles the platform is handed rather than by eye.
#[test]
fn the_disc_is_neutral_until_the_core_is_running() {
    let profile = Profile::claim("the_disc_is_neutral_until_the_core_is_running");
    profile.seed(SEEDED_PROFILE);
    let (mut app, sink) = observing(windowed(&profile, 900.0));
    app.wait_for("东京 01").unwrap();

    let button = app
        .screen()
        .press_target("role:button[启动代理]")
        .unwrap()
        .expect("the seeded profile can connect");

    let read = |sink: &Arc<Mutex<Drawn>>| {
        let drawn = sink.lock().unwrap();
        let style = drawn.style(button);
        let shadows = style.effects.as_ref().map_or(0, |fx| fx.box_shadows.len());
        (hex(style.background_color), shadows)
    };

    // No core running: the inset surface, and one plain shadow to sit it on the page.
    let (idle_fill, idle_shadows) = read(&sink);
    assert_ne!(
        idle_fill, "a85208",
        "an idle disc is painted the accent: it claims the proxy is running"
    );
    assert_eq!(
        idle_shadows, 1,
        "an idle disc carries the lit treatment ({} shadows)",
        idle_shadows
    );

    // The lit skin is checked in `domain`'s `disc_skin_tests`: reaching it end to end needs a
    // running core, which the suite never starts. What is checked here is the half only the
    // view can get wrong — reading the wrong role, or not binding the state at all.
    assert_ne!(
        idle_fill, "a85208",
        "an idle disc is painted --primary: it claims the proxy is running"
    );
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
    rules_page_with_form(&mut app);

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
    // collision with another rule rather than only a bad value. Saving closed the form, so it
    // has to be opened again exactly as a second rule would be by hand.
    open_rule_form(&mut app);
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
    app.wait_for("暂无自定义规则").unwrap();
    open_rule_form(&mut app);
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
    dns_tab_with_form(&mut app);

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
    dns_tab_with_form(&mut app);
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

/// The subscription page's one path that reaches a network at all is a URL, and CI has no
/// provider to point at. Pasted node links go through the same store call with the parse done
/// locally, so this drives import end to end: the dialog, the parse, the new row becoming the
/// active one, the nodes reaching 节点, and the file on disk.
#[test]
fn pasted_links_import_become_active_and_persist() {
    let profile = Profile::claim("pasted_links_import_become_active_and_persist");
    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("订阅").unwrap();
    app.wait_for("暂无订阅").unwrap();

    app.press("role:button[添加订阅] >> nth=0").unwrap();
    app.wait_for("订阅地址或节点内容").unwrap();
    app.type_text(
        "label:订阅地址或节点内容",
        "vless://11111111-1111-1111-1111-111111111111@edge.example.test:443?security=tls#边缘节点",
    )
    .unwrap();
    app.press("导入").unwrap();

    app.wait_for("已导入并切换到 本地内容，共 1 个节点")
        .unwrap();
    app.wait_for("1 个订阅 · 共 1 个节点").unwrap();
    app.wait_for("当前使用").unwrap();
    // The dialog closes on success, and the store owns the fields, so reopening is clean
    // rather than showing the link that was just imported.
    app.wait_for_gone("名称（可选）", std::time::Duration::from_secs(2))
        .unwrap();
    wait_until_profile(|stored| stored.contains("边缘节点"), &mut app);
    drop(app);

    let mut again = desktop(&profile);
    again.wait_for("Kitty Pro").unwrap();
    again.press("节点").unwrap();
    again.wait_for("边缘节点").unwrap();
}

/// The config script's two outcomes, both announced. The validator is the core's own QuickJS
/// pass over a built config, so this needs no network and exercises the class of bug where a
/// button does the work and never says anything: the notice is the only answer the user gets
/// for a press whose result is invisible.
#[test]
fn the_script_validator_reports_both_ways() {
    let (mut app, _profile) = seeded_test("the_script_validator_reports_both_ways", SEEDED_PROFILE);
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("JavaScript 配置覆写").unwrap();
    app.press("编辑").unwrap();
    app.wait_for("校验").unwrap();

    app.type_text("label:脚本", "function main(config) {\n  return config;\n}")
        .unwrap();
    app.press("校验").unwrap();
    app.wait_for("配置脚本校验通过").unwrap();

    // Now something the runtime rejects: the draft keeps growing, so the second script is a
    // valid function followed by a syntax error. The notice has to name the failure and say
    // what the validator said, which is why this matches the prefix rather than a whole node.
    app.type_text("label:脚本", "this is not javascript")
        .unwrap();
    app.press("校验").unwrap();
    app.wait_for("text:/^配置脚本校验失败/").unwrap();
    // The earlier success may still be on its way out; what matters is that the failure
    // arrived with its own words. 校验 never writes, which is the point of the button.
    assert!(!app.shows("配置脚本已保存"));
}

/// Saving a script is the one edit here that changes what the core would run, so it persists
/// and comes back. An empty script cannot be left switched on.
#[test]
fn a_saved_script_comes_back_after_a_relaunch() {
    let profile = Profile::claim("a_saved_script_comes_back_after_a_relaunch");
    profile.seed(SEEDED_PROFILE);
    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.press("编辑").unwrap();
    app.type_text("label:脚本", "function main(config) { return config; }")
        .unwrap();
    app.press("保存").unwrap();
    app.wait_for("配置脚本已保存").unwrap();
    app.wait_for("main(config) · QuickJS · 40 字符").unwrap();
    // The write has to outlive the press that started it: the same press closes the editor,
    // and a task owned by that region would be cancelled before the file is touched.
    wait_until_profile(|stored| stored.contains("function main(config)"), &mut app);
    // Saving keeps the switch where it was, and this profile had it off, so the script is
    // stored inert rather than silently taking the rule set over.
    assert!(
        !app.shows("启用后自定义分流规则将被脚本接管。"),
        "saving a script switched it on"
    );
    drop(app);

    let mut again = desktop(&profile);
    again.wait_for("Kitty Pro").unwrap();
    again.press("设置").unwrap();
    again.wait_for("main(config) · QuickJS · 40 字符").unwrap();
}

/// The core listens on a port; the operating system's proxy setting is what makes anything
/// use it. A client that can connect but not point the machine at the port shows "已连接"
/// while traffic goes nowhere, so the switch has to be there and parked with a reason when
/// there is no core behind it yet.
/// The switch that decides whether anything uses the port. Whether the machine is already
/// proxied is not the app's to decide, so this asserts only what holds on any machine: the
/// control is there, it is named, and it reports a read answer rather than a default. The
/// wording of every branch is covered by `a_parked_proxy_switch_says_why` in `domain`.
#[test]
fn the_system_proxy_switch_reads_the_machine() {
    let (mut app, _profile) = desktop_test("the_system_proxy_switch_reads_the_machine");
    app.wait_for("系统代理").unwrap();
    let toggle = "role:switch[系统代理]";
    // The read has to land on something. Which of these a given machine says is not the
    // app's business: a runner with no desktop session fails the read, and a developer's
    // laptop may already be proxied by something else. What the test insists on is that the
    // row stops saying "正在读取", which is what a lost boot read looks like from outside.
    //
    // `先启动代理` belongs in the list. It is what a machine with no core running and no proxy
    // set answers with — the read succeeded, and the row is explaining why the switch is parked
    // — so leaving it out made this test pass only on a machine that was already proxied, which
    // is exactly the machine that cannot show the parked state it exists to check.
    let settled = |app: &Headless| {
        app.screen().texts().iter().any(|text| {
            [
                "已设置",
                "系统当前使用其他代理",
                "未启用系统代理",
                "当前平台不支持系统代理",
                "读取系统代理失败",
                "当前平台尚未实现",
                "先启动代理",
            ]
            .iter()
            .any(|prefix| text.starts_with(prefix))
        })
    };
    for _ in 0..30 {
        if settled(&app) {
            break;
        }
        app.idle(std::time::Duration::from_millis(300)).unwrap();
    }
    assert!(settled(&app), "the system-proxy read never settled");
    // With no core running the switch is pressable only when the machine is already
    // proxied, which is the one case where turning it off is a real action. Offering it
    // off-and-pressable would point the machine at a closed port.
    let enabled =
        app.screen().accessibility_state(toggle).unwrap().checked == Some(nsh_core::Checked::True);
    assert_eq!(
        app.screen().enabled(toggle).unwrap(),
        enabled,
        "the switch is pressable with nothing behind the port, or inert while the machine \
         is already proxied"
    );
}

/// TUN and LAN are the two ways the machine hands traffic over, and both are settings a
/// relaunch has to keep. Toggling with no core running writes the file and starts nothing.
#[test]
fn the_takeover_switches_persist_across_a_relaunch() {
    let profile = Profile::claim("the_takeover_switches_persist_across_a_relaunch");
    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("接管方式").unwrap();

    let tun = "role:switch[TUN 模式]";
    let lan = "role:switch[允许局域网连接]";
    app.press(tun).unwrap();
    app.press(lan).unwrap();
    // Each note says what its own value means, so the row reads without a legend.
    app.wait_for("接管全部流量，首次需要管理员授权").unwrap();
    app.wait_for("局域网内其他设备可使用 7890").unwrap();
    wait_until_profile(|stored| stored.contains(r#""tun_enabled": true"#), &mut app);
    drop(app);

    let mut again = desktop(&profile);
    again.wait_for("Kitty Pro").unwrap();
    again.press("设置").unwrap();
    again.wait_for("接管全部流量，首次需要管理员授权").unwrap();
    assert_eq!(
        again.screen().accessibility_state(lan).unwrap().checked,
        Some(nsh_core::Checked::True),
        "the shared-listener choice did not come back"
    );
}

/// The port's label on the overview is the one place the LAN choice shows up without opening
/// Settings, so it has to move with the switch.
/// The port's label on the overview is the one place the LAN choice shows without opening
/// Settings. The connection card puts it below a 840pt fold, so this opens a taller window
/// rather than pretending otherwise: the assertion is about the binding, not the scroll
/// position.
#[test]
fn the_overview_port_label_follows_the_lan_switch() {
    let profile = Profile::claim("the_overview_port_label_follows_the_lan_switch");
    profile.seed(SEEDED_PROFILE);
    let mut app = tall(&profile);
    // The row names an endpoint only once there is something that could listen on it.
    app.wait_for("本地代理 7890").unwrap();
    app.press("设置").unwrap();
    app.press("role:switch[允许局域网连接]").unwrap();
    app.press("总览").unwrap();
    app.wait_for("局域网代理 7890").unwrap();
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
    // Weaker than it looks, on purpose. Nothing on the overview distinguishes "the profile
    // loaded and holds no nodes" from "the profile has not loaded yet" — both leave the node list
    // empty — so this half can settle early. It stays sound because the button is off in both
    // states; it is the seeded half below that can fail early, and that one now waits properly.
    assert!(
        !empty.screen().enabled(button).unwrap(),
        "an app with no nodes should not offer to connect"
    );
    drop(empty);

    profile.seed(SEEDED_PROFILE);
    let mut seeded = desktop(&profile);
    seeded.wait_for("内核已就绪").unwrap();
    // The button's enabled state comes from the node list, and the node list comes from the
    // profile — a second boot read, unrelated to the status one. Waiting for "内核已就绪" only
    // proves the core answered, so under a loaded machine this assertion used to run before the
    // profile had landed and found the button still off. Wait for the loaded profile's own
    // evidence instead: the overview names the node the file selects.
    seeded.wait_for("东京 01").unwrap();
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

/// The nodes page lists the groups the engine's own config contains rather than a grouping the
/// page invented. `build_singbox_config` turns any node list into one `proxy` selector holding
/// those nodes, so the seeded two-node subscription has to arrive as a named group saying how it
/// picks and which member it is on.
#[test]
fn the_nodes_page_lists_the_groups_the_engine_builds() {
    let (mut app, _profile) = seeded_test(
        "the_nodes_page_lists_the_groups_the_engine_builds",
        SEEDED_PROFILE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    app.wait_for("proxy").unwrap();
    // The member the selection resolves to is named in the header and not only ticked below it:
    // on a phone the members scroll, and the group's answer is the fact worth keeping.
    app.wait_for("手动选择 · 2 个成员 · 当前 东京 01").unwrap();
    app.wait_for("东京 01").unwrap();
    app.wait_for("大阪 02").unwrap();
    // The engine's own words stay in the engine, which takes a selector rather than `shows`: the
    // header is one text node, so `app.shows("selector")` could only match a node that is that
    // word alone and would pass on any screen. The positive line beside it is what proves the
    // selector is doing work — a negative assertion on its own passes just as happily when
    // nothing matches at all.
    assert!(shows_matching(&app, "text:/手动选择/"));
    assert!(
        !shows_matching(&app, "text:/selector/i"),
        "the internal group type reached the screen"
    );
}

/// A group whose members are all filtered out is dropped, header included. The count line keeps
/// reporting against the node list — the number a user checks against the provider's own is the
/// number of exits — while the list below it shows what the filter leaves, so the two say
/// different things on purpose.
#[test]
fn a_search_that_leaves_a_group_empty_removes_its_header() {
    let (mut app, _profile) = seeded_test(
        "a_search_that_leaves_a_group_empty_removes_its_header",
        SEEDED_PROFILE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    app.wait_for("proxy").unwrap();

    app.type_text("搜索节点、地址或协议", "东京").unwrap();
    app.wait_for("手动选择 · 1 个成员 · 当前 东京 01").unwrap();
    assert!(
        !app.shows("大阪 02"),
        "the filtered member is still drawn under the header"
    );

    // Nothing matches: the group is gone rather than a heading over an empty list, and the page
    // says what to do about it.
    app.clear("搜索节点、地址或协议").unwrap();
    app.type_text("搜索节点、地址或协议", "不存在").unwrap();
    app.wait_for("没有匹配的节点").unwrap();
    assert!(
        !app.shows("proxy"),
        "a group with no visible members still kept its header"
    );
    // The count describes the whole subscription, so it never claimed the filter had deleted a
    // node: 0 of 2 shown, 2 still there.
    app.wait_for("0 / 2 个节点 · 尚未测试延迟").unwrap();

    app.clear("搜索节点、地址或协议").unwrap();
    app.wait_for("proxy").unwrap();
    app.wait_for("手动选择 · 2 个成员 · 当前 东京 01").unwrap();
}

/// After a filter goes and comes back, the list is the list again and a pick inside it still
/// repaints the header.
///
/// What this is really guarding is the memo a cell reads through: rows come and go with the
/// filter, so a cell that resolved its content once at build time would keep showing what it was
/// first given. Note the limit of the assertion — a stale current-selection tick is an icon
/// rather than a text node, so nothing here would notice one; the header line is the part that
/// can be read as text, and it is what the check is worth.
#[test]
fn a_filter_round_trip_leaves_the_group_consistent() {
    let (mut app, _profile) = seeded_test(
        "a_filter_round_trip_leaves_the_group_consistent",
        SEEDED_PROFILE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    app.wait_for("proxy").unwrap();

    // Header, 东京 01, 大阪 02.
    app.type_text("搜索节点、地址或协议", "东京").unwrap();
    app.wait_for("手动选择 · 1 个成员 · 当前 东京 01").unwrap();
    app.clear("搜索节点、地址或协议").unwrap();
    app.wait_for("手动选择 · 2 个成员 · 当前 东京 01").unwrap();
    app.wait_for("大阪 02").unwrap();

    app.press("大阪 02").unwrap();
    app.wait_for("手动选择 · 2 个成员 · 当前 大阪 02").unwrap();
    assert_eq!(app.screen().count("text:大阪 02").unwrap(), 1);
    assert!(shows_matching(&app, "text:/^SS$/"));
}

/// message has to say which. Both halves matter: a toast claiming a switch describes a connection
/// that does not exist, and silence leaves the user re-reading the row to find out whether the
/// press did anything.
#[test]
fn a_choice_with_no_core_running_is_saved_and_says_so() {
    let (mut app, _profile) = seeded_test(
        "a_choice_with_no_core_running_is_saved_and_says_so",
        SEEDED_PROFILE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    app.wait_for("proxy").unwrap();
    app.press("大阪 02").unwrap();
    app.wait_for("proxy → 大阪 02（未连接，启动后生效）")
        .unwrap();
    app.wait_for("手动选择 · 2 个成员 · 当前 大阪 02").unwrap();

    // The two fields the engine reads are both on the chosen node. A stale `proxy` entry is the
    // one thing that makes a saved choice lie, because the engine applies that entry over the
    // selection when it builds the config.
    let saved = std::fs::read_to_string(
        std::path::PathBuf::from(
            std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
        )
        .join("profile.json"),
    )
    .expect("the profile a press wrote");
    let written: serde_json::Value = serde_json::from_str(&saved).expect("a parseable profile");
    assert_eq!(written["selected_tag"], "b");
    assert_eq!(
        written["group_selections"]["proxy"], "b",
        "the selection moved and the group entry did not, which is the shape the engine obeys \
         over the selection"
    );
}

/// The other direction: a profile that arrives with the two fields disagreeing. This is the only
/// route to that shape that is not this app's fault — the Dioxus shell and a sync pull write the
/// same file — and the field the engine obeys is not the one the list shows, so loading has to
/// reconcile them rather than draw a choice the core will not honour.
#[test]
fn a_choice_written_by_another_shell_is_reconciled_on_load() {
    let profile = Profile::claim("a_choice_written_by_another_shell_is_reconciled_on_load");
    profile.seed(
        &SEEDED_PROFILE
            .replace("\"selected_tag\": \"a\"", "\"selected_tag\": \"b\"")
            .replace(
                "\"group_selections\": {}",
                "\"group_selections\": {\"proxy\": \"a\"}",
            ),
    );

    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    app.wait_for("手动选择 · 2 个成员 · 当前 大阪 02").unwrap();
    assert!(shows_matching(&app, "text:/当前 大阪 02/"));
    assert!(
        !shows_matching(&app, "text:/当前 东京 01/"),
        "the stale remembered entry won, so the page names an exit the core would not use"
    );
    drop(app);

    // A relaunch reads the same file and answers the same way: the reconciliation is not a one-time
    // correction that a second load undoes.
    let mut again = desktop(&profile);
    again.wait_for("Kitty Pro").unwrap();
    again.press("节点").unwrap();
    again
        .wait_for("手动选择 · 2 个成员 · 当前 大阪 02")
        .unwrap();
}

/// "Connected" is a claim about a process and a listening port. Whether traffic gets out is a
/// separate claim, answerable only by dialing through the proxy. The control belongs on the
/// screen where the user has just connected, and it stays parked with its reason before then: a
/// press against a closed port would report a broken proxy when the truth is that none is running.
#[test]
fn the_exit_probe_is_offered_and_parked_until_a_core_runs() {
    let (mut app, _profile) = seeded_test(
        "the_exit_probe_is_offered_and_parked_until_a_core_runs",
        SEEDED_PROFILE,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.wait_for("测试连通").unwrap();
    app.wait_for("先启动代理，才能探测出口").unwrap();
    let button = "role:button[测试连通]";
    assert!(
        !app.screen().enabled(button).unwrap(),
        "the probe offered to dial a port with nothing behind it"
    );
    // Deliberately never pressed. Over a running core this would be a real outbound request, and
    // the suite starts no core; what makes not testing the request safe is that the control
    // cannot be reached in that state.
}

/// The login item and the clipboard describe the machine rather than the profile, so both belong
/// on the settings page and neither is pressed here: one would write a real launch entry into the
/// developer's account, the other would overwrite their clipboard. What is checked is that the
/// controls are named and that the read settles — the failure mode of a boot read that never
/// lands is a row stuck on "正在读取".
#[test]
fn the_machine_owned_switches_are_present_and_read_settles() {
    let (mut app, _profile) =
        desktop_test("the_machine_owned_switches_are_present_and_read_settles");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("启动与终端").unwrap();

    let login = "role:switch[登录时启动]";
    app.wait_for(login).unwrap();
    let settled = |app: &Headless| {
        app.screen().texts().iter().any(|text| {
            [
                "登录后自动启动，不自动连接",
                "手动打开后才工作",
                "正在写入当前用户的登录项",
            ]
            .iter()
            .any(|prefix| text.starts_with(prefix))
                || text.contains("启动项失败")
        })
    };
    for _ in 0..30 {
        if settled(&app) {
            break;
        }
        app.idle(std::time::Duration::from_millis(300)).unwrap();
    }
    assert!(settled(&app), "the login-item read never settled");

    app.wait_for("复制代理环境变量").unwrap();
}

/// The copied line names the port the core listens on, and the note is the only place a user
/// learns that sharing with the network changes who can reach it.
#[test]
fn the_copied_line_names_the_port_the_core_listens_on() {
    let (mut app, _profile) = desktop_test("the_copied_line_names_the_port_the_core_listens_on");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("text:/指向本机的 7890 端口/").unwrap();

    app.press("role:switch[允许局域网连接]").unwrap();
    app.wait_for("text:/指向局域网可达的 7890 端口/").unwrap();
}

/// The preview is a config build, and a broken config script is the realistic way it fails. That
/// must not take the exit picker away: the derivation failed, the subscription did not.
#[test]
fn a_script_that_breaks_the_config_leaves_the_nodes_reachable() {
    let profile = Profile::claim("a_script_that_breaks_the_config_leaves_the_nodes_reachable");
    profile.seed(
        &SEEDED_PROFILE
            .replace(
                "\"config_script_enabled\": false",
                "\"config_script_enabled\": true",
            )
            .replace(
                "\"config_script\": \"\"",
                "\"config_script\": \"function main(config) {Syntax\"",
            ),
    );

    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("节点").unwrap();
    // Both facts on the screen at once: what went wrong, and the nodes to act on.
    app.wait_for("text:/^代理组生成失败/").unwrap();
    app.wait_for("东京 01").unwrap();
    app.wait_for("大阪 02").unwrap();
    app.press("大阪 02").unwrap();
    app.wait_for("proxy → 大阪 02（未连接，启动后生效）")
        .unwrap();
}

/// The about card's figures are the reason it exists, so they have to come from the profile on
/// disk rather than from a default: two nodes and one subscription is what that file says.
#[test]
fn the_about_card_reports_the_build_and_the_counts() {
    let profile = Profile::claim("the_about_card_reports_the_build_and_the_counts");
    profile.seed(SEEDED_PROFILE);
    let mut app = full_height(&profile);

    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("诊断与关于").unwrap();
    // The version of the core is whatever this machine's engine reports, so only the shape is
    // asserted: app version, then a core, then the platform. Whether the build is known is the
    // sidebar kicker's test, not this one's.
    app.wait_for("text:/^v\\d+\\.\\d+\\.\\d+ · sing-box/")
        .unwrap();
    app.wait_for("text:/· Desktop$/").unwrap();
    app.wait_for("节点 / 订阅").unwrap();
    // Two nodes and one subscription is what the seeded file says, so the figures came from disk
    // rather than from a default.
    app.wait_for("2 / 1").unwrap();
    app.wait_for("专用 DNS").unwrap();
    app.wait_for("自定义规则").unwrap();
}

/// Pressing 复制诊断信息 is the one place the app writes the clipboard, and the engine's
/// clipboard is the test's own — so unlike the environment-variable copy, which shells out to
/// `pbcopy` and would overwrite the developer's clipboard, this press is safe to make and its
/// result is worth reading back: the text is the deliverable.
#[test]
fn copying_the_report_puts_the_whole_text_on_the_clipboard() {
    let profile = Profile::claim("copying_the_report_puts_the_whole_text_on_the_clipboard");
    profile.seed(SEEDED_PROFILE);
    let mut app = desktop(&profile);
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("复制诊断信息").unwrap();
    app.press("复制诊断信息").unwrap();

    let copied = |app: &Headless| app.screen().clipboard().unwrap_or_default().to_owned();
    for _ in 0..30 {
        if copied(&app).contains("Kitty Pro 诊断信息") {
            break;
        }
        app.idle(std::time::Duration::from_millis(100)).unwrap();
    }
    let text = copied(&app);
    assert!(text.contains("界面平台: Desktop"), "{text}");
    assert!(text.contains("节点: 2 · 订阅: 1"), "{text}");
    assert!(text.contains("本地代理 7890"), "{text}");
    assert!(
        !text.contains("None"),
        "the report leaked an Option: {text}"
    );
    // The notice is what tells the user it worked; the clipboard itself is invisible.
    app.wait_for("诊断信息已复制到剪贴板").unwrap();
}

/// Notices appear at the top of the window and centred across it, which is where the user asked
/// for them.
///
/// This is a position claim, so it goes through the drawn frames: the toast's text is found by
/// what it says, and that node's place in the window is measured against the viewport. A text
/// assertion alone could never fail on placement — the words are the same in any corner.
#[test]
fn a_notice_lands_top_centre() {
    let profile = Profile::claim("a_notice_lands_top_centre");
    profile.seed(SEEDED_PROFILE);
    let (mut app, sink) = observing(windowed(&profile, 900.0));
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("复制诊断信息").unwrap();
    app.press("复制诊断信息").unwrap();
    app.wait_for("诊断信息已复制到剪贴板").unwrap();

    let drawn = sink.lock().unwrap();
    let toast = drawn
        .node_with_text("诊断信息已复制到剪贴板")
        .expect("the notice drew nothing to locate");
    let (center_x, center_y) = drawn.center_in_root(toast);

    // Top of the window, not the bottom where the stack used to hang.
    assert!(
        center_y < 900.0 * 0.35,
        "the notice is at y {center_y} of a 900pt window: not the top"
    );
    // Centred across the whole window, which includes the rail — so a notice hung off the
    // content column alone would sit right of the middle.
    assert!(
        (center_x - 1280.0 / 2.0).abs() < 80.0,
        "the notice is centred at x {center_x} of a 1280pt window: not across the middle"
    );
}

/// The search filters the page in place. What matters is that a filtered card is *gone*, not
/// merely reordered — a page that kept every card mounted behind a filter would still answer a
/// `shows` check for content nobody asked for.
#[test]
fn the_settings_search_keeps_only_the_cards_that_answer() {
    let (mut app, _profile) = desktop_test("the_settings_search_keeps_only_the_cards_that_answer");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("启动与终端").unwrap();

    app.type_text("搜索设置", "诊断").unwrap();
    app.wait_for("诊断与关于").unwrap();
    for gone in ["启动与终端", "接管方式", "流量走向"] {
        assert!(
            !shows_matching(&app, &format!("text:{gone}")),
            "`{gone}` survived a search for 诊断"
        );
    }

    // The same box answers English, which is what a user types when the term is forgotten.
    app.clear("搜索设置").unwrap();
    app.type_text("搜索设置", "tun").unwrap();
    app.wait_for("接管方式").unwrap();
    assert!(
        !shows_matching(&app, "text:诊断与关于"),
        "a search for tun should not surface the about card"
    );
}

/// A search that finds nothing empties the page, which the app has to say out loud and undo in
/// one press: a blank settings page reads as a crash.
#[test]
fn a_search_that_finds_nothing_says_so_and_clears_itself() {
    let (mut app, _profile) = desktop_test("a_search_that_finds_nothing_says_so_and_clears_itself");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();

    app.type_text("搜索设置", "打印机").unwrap();
    app.wait_for("没有匹配的设置项").unwrap();
    assert!(
        !shows_matching(&app, "text:外观"),
        "the page kept a card a search for 打印机 cannot mean"
    );

    app.press("清除").unwrap();
    app.wait_for("外观").unwrap();
    app.wait_for("启动与终端").unwrap();
}

/// The language is stated rather than offered: neither front end can switch it, and a row with
/// a menu of one would promise a feature the app does not have. It also has to be findable,
/// since it is the row users look for first.
#[test]
fn the_language_is_stated_and_found_by_the_search() {
    let (mut app, _profile) = desktop_test("the_language_is_stated_and_found_by_the_search");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("简体中文（暂不支持切换）").unwrap();
    // Stated, not chosen: nothing on the row takes a press.
    assert!(
        !shows_matching(&app, "role:combobox[语言]"),
        "the language row offers a choice the app cannot honour"
    );

    app.type_text("搜索设置", "语言").unwrap();
    app.wait_for("简体中文（暂不支持切换）").unwrap();
    assert!(
        !shows_matching(&app, "text:接管方式"),
        "the search left the whole page mounted behind it"
    );
}

/// A process rule is the one kind whose value cannot be invented reliably, so the editor offers
/// the machine's own list — and offers it only where the OS can report processes at all.
#[test]
fn the_picker_is_offered_for_a_process_rule_and_not_for_a_domain_rule() {
    let (mut app, _profile) =
        desktop_test("the_picker_is_offered_for_a_process_rule_and_not_for_a_domain_rule");
    app.wait_for("Kitty Pro").unwrap();
    rules_page_with_form(&mut app);

    // The default match kind is a domain suffix, which has no list to choose from.
    assert!(
        !shows_matching(&app, "text:选择进程"),
        "a domain rule was offered a process picker"
    );

    app.press("role:combobox[匹配类型]").unwrap();
    app.press("role:option[进程名]").unwrap();
    app.wait_for("选择进程").unwrap();
    // The field's own example changes with the kind, which is what says the two are wired.
    app.wait_for("text:Telegram").unwrap();
}

/// The picker's whole job is to put something into the field. The list is the machine's, so the
/// test does not name a process — it takes the first row and checks the field now holds what it
/// says, which is the only claim that holds on every machine.
#[test]
fn picking_a_process_fills_the_rule_field() {
    let (mut app, _profile) = desktop_test("picking_a_process_fills_the_rule_field");
    app.wait_for("Kitty Pro").unwrap();
    rules_page_with_form(&mut app);
    app.press("role:combobox[匹配类型]").unwrap();
    app.press("role:option[进程路径]").unwrap();
    app.wait_for("选择进程").unwrap();

    app.press("选择进程").unwrap();
    // The read is a real one against the OS, so the dialog may land on either a list or a
    // refusal; both are answers. What must never happen is the dialog sitting on 正在读取 for
    // ever, which is why the wait ends on an answer rather than on a timeout.
    let settled = |app: &Headless| {
        app.screen().matches("id:process-list").unwrap_or(false)
            || app.screen().matches("text:没有匹配的进程").unwrap_or(false)
            || app
                .screen()
                .matches("text:/进程列表读取任务失败|不支持/")
                .unwrap_or(false)
    };
    for _ in 0..40 {
        if settled(&app) {
            break;
        }
        app.idle(std::time::Duration::from_millis(200)).unwrap();
    }
    assert!(
        settled(&app),
        "the picker never answered: {:?}",
        app.screen().texts()
    );

    if !app.screen().matches("id:process-list").unwrap_or(false) {
        // Nothing readable to pick (a container with no process table the app may enumerate) is
        // a platform fact, not a wiring failure. The dialog answered, which is the contract; the
        // rows themselves are exercised on every machine that has them.
        app.press("取消").unwrap();
        app.wait_for("匹配内容").unwrap();
        return;
    }

    let first = "id:process-list >> role:button >> nth=0";
    // The editor cannot be submitted with nothing in it, which is what makes the next
    // assertion mean something: only a value that really landed re-enables the button.
    assert!(
        !app.screen().enabled("添加").unwrap(),
        "an empty rule was submittable before anything was picked"
    );
    app.press(first).unwrap();
    // The dialog's own list is what goes away; the button that opened it stays, since the user
    // may well pick again.
    app.wait_for_gone("id:process-list", std::time::Duration::from_secs(2))
        .unwrap();
    let value = app
        .screen()
        .input_text("label:匹配内容")
        .unwrap()
        .unwrap_or_default()
        .to_owned();
    assert!(
        !value.trim().is_empty(),
        "picking a process left the rule's value empty"
    );
    assert!(
        app.screen().enabled("添加").unwrap(),
        "the picked process reached the field but not the form: {value:?}"
    );
}

/// The picker's search narrows the machine's list rather than re-reading it: the rows come and
/// go with what is typed, and clearing the box brings them all back.
#[test]
fn the_process_list_filters_by_name_and_path() {
    let (mut app, _profile) = desktop_test("the_process_list_filters_by_name_and_path");
    app.wait_for("Kitty Pro").unwrap();
    rules_page_with_form(&mut app);
    app.press("role:combobox[匹配类型]").unwrap();
    app.press("role:option[进程名]").unwrap();
    app.wait_for("选择进程").unwrap();
    app.press("选择进程").unwrap();

    let loaded = |app: &Headless| app.screen().matches("id:process-list").unwrap_or(false);
    let answered = |app: &Headless| {
        loaded(app) || app.screen().matches("text:没有匹配的进程").unwrap_or(false)
    };
    for _ in 0..40 {
        if answered(&app) {
            break;
        }
        app.idle(std::time::Duration::from_millis(200)).unwrap();
    }
    assert!(
        answered(&app),
        "the picker never answered: {:?}",
        app.screen().texts()
    );
    if !loaded(&app) {
        // No list to filter on this machine; the dialog answered, which is all this test can
        // ask of a process table it does not own.
        app.press("取消").unwrap();
        return;
    }

    app.type_text("搜索进程名或路径", "kitty-no-such-process")
        .unwrap();
    app.wait_for("没有匹配的进程").unwrap();
    // Clearing the box brings the list back: the filter is a view over the read, not a new one.
    app.clear("搜索进程名或路径").unwrap();
    app.wait_for("id:process-list").unwrap();
}

/// The about card's figures wrap rather than sit in fixed columns, which is the kind of row that
/// collapses to nothing at a phone width. The values are asserted rather than the labels: a
/// squeezed row still prints its heading while its figures have left the window.
#[test]
fn the_about_card_holds_its_figures_on_a_phone() {
    let profile = Profile::claim("the_about_card_holds_its_figures_on_a_phone");
    profile.seed(SEEDED_PROFILE);
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(390.0), px(2400.0)),
        environment: Environment {
            window: Size::new(px(390.0), px(2400.0)),
            ..Environment::default()
        },
        ..EngineConfig::new(nsh_app_kitty::app_root())
    })
    .expect("the app starts");

    // A phone has no rail, so there is no wordmark to wait for; the seeded node's name is the
    // profile's own evidence, which is what the about card's figures read.
    app.wait_for("东京 01").unwrap();
    app.press("设置").unwrap();
    app.wait_for("诊断与关于").unwrap();
    app.wait_for("2 / 1").unwrap();
    app.wait_for("复制诊断信息").unwrap();
    // The build line is the card's longest text and the first thing a narrow window clips. A
    // phone idiom reports itself as Mobile, which is the platform label doing its job.
    app.wait_for("text:/^v\\d+\\.\\d+\\.\\d+ · sing-box.* · Mobile$/")
        .unwrap();
}

/// A WebDAV target with everything filled in, so the remote actions are reachable — and never
/// pressed. `sync::push` and `sync::pull` speak HTTP through reqwest rather than NSH's `fetch`,
/// so the suite's `StubNetwork` cannot intercept them: pressing one would dial the address on
/// screen, which is the same reason 启动代理 stays unpressed here. What is asserted instead is
/// every guard that decides whether the press would be allowed at all.
const COMPLETE_SYNC_JSON: &str = r#"{
  "enabled": true,
  "provider": "web_dav",
  "webdav_endpoint": "https://dav.example.test/dav",
  "s3_endpoint": "",
  "path": "kitty-pro-sync.json",
  "username": "hezz",
  "password": "app-pass",
  "bucket": "",
  "region": "us-east-1",
  "access_key": "",
  "secret_key": "",
  "last_sync_at": 0,
  "last_remote_revision": ""
}"#;

fn sync_test(name: &str, sync_json: &str) -> (Headless, Profile) {
    let profile = Profile::claim(name);
    write_sync_json(sync_json);
    let app = full_height(&profile);
    (app, profile)
}

/// The same, over a profile with nodes and a subscription in it, for the cases where the card's
/// counts and the pull summary have to have something to count.
fn seeded_sync_test(name: &str, sync_json: &str) -> (Headless, Profile) {
    let profile = Profile::claim(name);
    profile.seed(SEEDED_PROFILE);
    write_sync_json(sync_json);
    let app = full_height(&profile);
    (app, profile)
}

fn write_sync_json(sync_json: &str) {
    let dir = std::path::PathBuf::from(
        std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
    );
    std::fs::write(dir.join("sync.json"), sync_json).expect("a seedable sync config");
}

/// The card reads the settings file on disk and says which of three states it found. Off,
/// never-run and already-synced are different answers to "is my other device current", and a
/// card that called a disabled sync 已同步 would send the user away believing too much.
#[test]
fn the_sync_card_reports_the_state_on_disk() {
    let (mut app, _profile) = sync_test(
        "the_sync_card_reports_the_state_on_disk",
        COMPLETE_SYNC_JSON,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("规则与订阅同步").unwrap();
    // Enabled, complete, and nothing has travelled yet.
    app.wait_for("待同步").unwrap();
    assert!(
        !shows_matching(&app, "text:已同步"),
        "a sync that never ran reported itself as done"
    );
    // The stored address is what the field shows, not a placeholder. Read from the input rather
    // than from the screen's text: an input's value is not a text node, and a leading `https:`
    // reads as a selector prefix to the driver anyway.
    assert_eq!(
        app.screen()
            .input_text("label:WebDAV 地址")
            .unwrap()
            .unwrap_or_default(),
        "https://dav.example.test/dav"
    );
    // Each provider owns an address field; the other's must not be on screen.
    assert!(
        !shows_matching(&app, "text:S3 Endpoint"),
        "the WebDAV form drew S3's address field"
    );
}

#[test]
fn a_disabled_sync_says_off_and_offers_nothing() {
    let (mut app, _profile) = sync_test(
        "a_disabled_sync_says_off_and_offers_nothing",
        &COMPLETE_SYNC_JSON.replace("\"enabled\": true", "\"enabled\": false"),
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("规则与订阅同步").unwrap();
    app.wait_for("未启用").unwrap();
    for action in ["上传", "下载覆盖本机", "覆盖远端"] {
        assert!(
            !app.screen().enabled(action).unwrap(),
            "`{action}` was offered while sync is off"
        );
    }
    // Saving is local, so it stays available: a user who wants to record an address before
    // switching sync on has to be able to.
    assert!(
        app.screen().enabled("保存同步设置").unwrap(),
        "the local save was parked along with the remote actions"
    );
}

/// An incomplete target parks the remote buttons and names what is missing, because a press that
/// fails at the far end costs a round trip to report something visible beforehand.
#[test]
fn an_incomplete_target_parks_the_remote_actions() {
    let (mut app, _profile) = sync_test(
        "an_incomplete_target_parks_the_remote_actions",
        &COMPLETE_SYNC_JSON.replace("https://dav.example.test/dav", ""),
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("规则与订阅同步").unwrap();
    for action in ["上传", "下载覆盖本机", "覆盖远端"] {
        assert!(
            !app.screen().enabled(action).unwrap(),
            "`{action}` was enabled with no address to dial"
        );
    }

    // Typing the address brings them up, which is the proof the field and the guard are wired.
    app.type_text("label:WebDAV 地址", "https://dav.example.test/dav")
        .unwrap();
    app.wait_for("上传").unwrap();
    assert!(
        app.screen().enabled("上传").unwrap(),
        "a complete target still would not upload: {:?}",
        app.screen().texts()
    );
}

/// Switching the segment swaps the whole field set. The two providers keep separate stored
/// addresses on purpose, so a typed WebDAV URL can never be saved as an S3 endpoint.
#[test]
fn switching_provider_swaps_the_fields_it_edits() {
    let (mut app, _profile) = sync_test(
        "switching_provider_swaps_the_fields_it_edits",
        COMPLETE_SYNC_JSON,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("WebDAV 地址").unwrap();

    app.press("S3 兼容").unwrap();
    app.wait_for("S3 Endpoint").unwrap();
    app.wait_for("Bucket").unwrap();
    // The WebDAV address is not merely hidden — its field is gone, which is what stops a hidden
    // value being saved under the other provider's name.
    assert!(
        !shows_matching(&app, "text:WebDAV 地址"),
        "the S3 form kept WebDAV's field"
    );
    // Nothing was typed for S3, so its own target is incomplete and the actions park again.
    assert!(
        !app.screen().enabled("上传").unwrap(),
        "switching to an empty S3 target left upload enabled"
    );

    app.press("WebDAV").unwrap();
    app.wait_for("WebDAV 地址").unwrap();
    // Switching back has to bring the stored address with it: the two providers keep separate
    // fields precisely so a round trip loses nothing.
    assert_eq!(
        app.screen()
            .input_text("label:WebDAV 地址")
            .unwrap()
            .unwrap_or_default(),
        "https://dav.example.test/dav"
    );
}

/// The stored secret reaches the app as a mask, never as the password.
///
/// This is the leak check, not a rendering check: whether the field draws bullets is something
/// only a real platform can show, and the headless driver reports the text either way. What the
/// tests can pin is that `load_sync_config` hands back `••••••••` rather than the token from disk
/// — because a client that received the real value would also have put it in a text buffer, a
/// screenshot, and an accessibility tree.
#[test]
fn a_stored_secret_never_reaches_the_app_as_plaintext() {
    let (mut app, _profile) = sync_test(
        "a_stored_secret_never_reaches_the_app_as_plaintext",
        COMPLETE_SYNC_JSON,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("密码 / 应用密码").unwrap();

    let shown = app
        .screen()
        .input_text("label:密码 / 应用密码")
        .unwrap()
        .unwrap_or_default()
        .to_owned();
    assert!(!shown.is_empty(), "the password field arrived empty");
    assert!(
        !shown.contains("app-pass"),
        "the real password reached the client: {shown}"
    );
    // The address, which is not a secret, arrives whole — otherwise this mask would just be
    // evidence that nothing was read at all.
    assert_eq!(
        app.screen()
            .input_text("label:WebDAV 地址")
            .unwrap()
            .unwrap_or_default(),
        "https://dav.example.test/dav"
    );
}

/// 保存 writes only the settings file. That is the one sync press with no side effect beyond the
/// scratch directory, so it is the one worth driving end to end.
#[test]
fn saving_the_sync_settings_writes_only_the_settings_file() {
    let (mut app, profile) = sync_test(
        "saving_the_sync_settings_writes_only_the_settings_file",
        COMPLETE_SYNC_JSON,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("规则与订阅同步").unwrap();

    app.type_text("label:远程路径", "-v2").unwrap();
    app.press("保存同步设置").unwrap();
    app.wait_for("同步配置已保存").unwrap();

    let dir = std::path::PathBuf::from(
        std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
    );
    let saved = std::fs::read_to_string(dir.join("sync.json")).expect("the saved sync config");
    assert!(
        saved.contains("kitty-pro-sync.json-v2"),
        "the new path never reached sync.json: {saved}"
    );
    // Saving settings is not saving the profile: the two files have different owners and a
    // press on one must not rewrite the other.
    let on_disk = std::fs::read_to_string(dir.join("profile.json")).unwrap_or_default();
    assert!(
        !on_disk.contains("kitty-pro-sync.json-v2"),
        "saving the sync config wrote into the profile"
    );
    let _ = profile;
}

/// The two buttons that replace data ask twice, and the second press says what is about to be
/// replaced on both sides — before anything is written.
#[test]
fn arming_a_replacement_shows_what_would_be_lost() {
    let (mut app, _profile) = seeded_sync_test(
        "arming_a_replacement_shows_what_would_be_lost",
        COMPLETE_SYNC_JSON,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("下载覆盖本机").unwrap();

    app.press("下载覆盖本机").unwrap();
    app.wait_for("确认覆盖本机").unwrap();
    // The counts are the decision. A remote that has never been read reports itself as unknown
    // rather than as empty, seconds before the user would overwrite their own work.
    app.wait_for("text:/需下载后确认/").unwrap();
    // And the press did not go out: the button only armed.
    assert!(
        shows_matching(&app, "text:确认覆盖本机"),
        "the first press seems to have run the pull"
    );
}

/// An armed replacement expires. A button left reading 确认覆盖本机 while the user goes to check
/// something is a button that does the wrong thing when they come back.
#[test]
fn an_armed_replacement_forgets_itself_when_left_alone() {
    let (mut app, _profile) = seeded_sync_test(
        "an_armed_replacement_forgets_itself_when_left_alone",
        COMPLETE_SYNC_JSON,
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.press("覆盖远端").unwrap();
    app.wait_for("确认覆盖远端").unwrap();

    // The window is three seconds; give it room without making the test slow on a loaded CI.
    for _ in 0..12 {
        app.idle(std::time::Duration::from_millis(500)).unwrap();
        if app.screen().count("role:button[覆盖远端]").unwrap_or(0) == 1 {
            return;
        }
    }
    panic!(
        "the armed overwrite never expired: {:?}",
        app.screen().texts()
    );
}

/// The sync card answers to the word a user types for it, and to nothing else on the page.
#[test]
fn the_sync_card_is_found_by_a_search_for_sync() {
    let (mut app, _profile) = desktop_test("the_sync_card_is_found_by_a_search_for_sync");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.type_text("搜索设置", "云同步").unwrap();
    app.wait_for("规则与订阅同步").unwrap();
    assert!(
        !shows_matching(&app, "text:接管方式"),
        "a search for 云同步 left the whole page mounted"
    );
}

/// An update state with a newer release already remembered, checked a minute ago.
///
/// The recent stamp is what keeps the launch check off the network: `check_app_update` speaks
/// HTTP through reqwest rather than NSH's `fetch`, so the suite's `StubNetwork` cannot intercept
/// it and a due check would dial GitHub from a test. That is also why 检查更新 is asserted as
/// reachable below and never pressed.
fn update_state(latest: Option<(&str, &str)>, auto_check: bool) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let release = latest.map_or_else(
        || "null".to_owned(),
        |(version, title)| {
            format!(
                r#"{{"version":"{version}","title":"{title}","url":"https://github.example.test/releases/{version}","notes":null,"published_at":null,"prerelease":false,"asset":null,"asset_size":0}}"#
            )
        },
    );
    format!(
        r#"{{"settings":{{"auto_check":{auto_check},"last_check_at":{}}},"latest":{release},"snoozed_version":""}}"#,
        now - 60
    )
}

/// Claim a profile, write an update state over the harness default, and open it full height.
fn update_test(name: &str, update_json: &str) -> (Headless, Profile) {
    let profile = Profile::claim(name);
    let dir = std::path::PathBuf::from(
        std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
    );
    std::fs::write(dir.join("update.json"), update_json).expect("a seedable update state");
    profile.seed(SEEDED_PROFILE);
    let app = full_height(&profile);
    (app, profile)
}

/// Read one field out of the scratch directory's `update.json`.
///
/// Parsed rather than matched as text: the backend writes the file pretty-printed, so a
/// `"key":"value"` substring never appears and an assertion written that way would fail on
/// format rather than on behaviour.
fn update_state_field(field: &str) -> String {
    let dir = std::path::PathBuf::from(
        std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
    );
    let text = std::fs::read_to_string(dir.join("update.json")).expect("the update state");
    let value: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("update.json: {error}\n{text}"));
    let mut node = &value;
    for part in field.split('.') {
        node = node
            .get(part)
            .unwrap_or_else(|| panic!("`{field}` missing from {text}"));
    }
    match node {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[test]
fn the_update_card_is_found_by_a_search_for_updates() {
    let (mut app, _profile) = desktop_test("the_update_card_is_found_by_a_search_for_updates");
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.type_text("搜索设置", "更新").unwrap();
    app.wait_for("应用更新").unwrap();
    assert!(
        !shows_matching(&app, "text:规则与订阅同步"),
        "a search for 更新 left the sync card mounted"
    );
}

/// A remembered newer release is presented as something to act on, and ignoring it is the one
/// press this card can finish offline: dismissing writes a local file, while the download itself
/// belongs to the release page.
/// A waiting update reaches the user before they open anything: once as a notice, and after that
/// as a mark on 设置 in the rail.
///
/// The release is remembered rather than fetched — the launch check reads the same state file the
/// card does, so the whole path runs offline. The notice is what says it the first time; the dot
/// is what is still true the fifth time, and it has to be said in the button's name as well as
/// drawn, because a coloured circle carries nothing for someone who cannot see colour.
#[test]
fn a_waiting_update_is_announced_and_then_marked() {
    let (mut app, _profile) = update_test(
        "a_waiting_update_is_announced_and_then_marked",
        &update_state(Some(("9.9.9", "v9.9.9")), true),
    );
    app.wait_for("发现新版本 v9.9.9").unwrap();
    // The rail row says what the dot means, not just its name.
    app.wait_for("role:button[设置（有新版本可更新）]").unwrap();

    // The count chips are the rail's other mark, and they never share a slot with the dot.
    // The count is part of the row, so it is part of the row's name: two nodes and one
    // subscription is what the seeded profile holds.
    app.wait_for("role:button[节点 2]").unwrap();
    app.wait_for("role:button[订阅 1]").unwrap();
}

/// The dot is the notice's successor, not its duplicate: once the release is declined, the rail
/// stops asking.
#[test]
fn declining_a_release_takes_the_mark_off_the_rail() {
    let (mut app, _profile) = update_test(
        "declining_a_release_takes_the_mark_off_the_rail",
        &update_state(Some(("9.9.9", "v9.9.9")), true),
    );
    app.wait_for("role:button[设置（有新版本可更新）]").unwrap();
    app.press("设置").unwrap();
    app.wait_for("忽略这个版本").unwrap();
    app.press("忽略这个版本").unwrap();
    app.wait_for("已忽略这个版本，下次有更新再提醒").unwrap();

    app.wait_for("role:button[设置]").unwrap();
    assert!(
        !shows_matching(&app, "text:有新版本可更新"),
        "the rail kept asking after the release was declined: {:?}",
        app.screen().texts()
    );
}

#[test]
fn a_newer_release_can_be_ignored_and_stays_ignored() {
    let (mut app, profile) = update_test(
        "a_newer_release_can_be_ignored_and_stays_ignored",
        &update_state(Some(("9.9.9", "v9.9.9")), true),
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("应用更新").unwrap();
    // The launch check read the state file and reported what it held, without going out.
    app.wait_for("有更新").unwrap();
    app.wait_for("忽略这个版本").unwrap();
    assert!(
        app.screen().enabled("检查更新").unwrap(),
        "a desktop build with a remembered release had no way to ask again"
    );

    app.press("忽略这个版本").unwrap();
    app.wait_for("已忽略这个版本，下次有更新再提醒").unwrap();
    // The header follows the card: 有更新 over a release the user has declined is a contradiction.
    app.wait_for("已忽略").unwrap();
    assert!(
        !shows_matching(&app, "text:忽略这个版本"),
        "the dismiss button survived the dismissal"
    );

    let saved = update_state_field("snoozed_version");
    assert_eq!(saved, "9.9.9", "the dismissal never reached the state file");
    drop(app);

    // And it holds across a relaunch, which is the point of writing it at all.
    let mut again = desktop(&profile);
    again.wait_for("Kitty Pro").unwrap();
    again.press("设置").unwrap();
    again.wait_for("已忽略").unwrap();
    assert!(
        !shows_matching(&again, "text:忽略这个版本"),
        "a dismissed release asked again after a relaunch"
    );
}

/// Turning the daily check off has to survive a restart or the switch is decoration: its whole
/// promise is that the app stops contacting the feed on its own.
#[test]
fn turning_off_the_daily_check_is_remembered() {
    let (mut app, profile) = update_test(
        "turning_off_the_daily_check_is_remembered",
        &update_state(None, true),
    );
    app.wait_for("Kitty Pro").unwrap();
    app.press("设置").unwrap();
    app.wait_for("应用更新").unwrap();
    // A check ran and remembered no newer release: the installed build is the newest one.
    app.wait_for("已是最新").unwrap();

    let toggle = "role:switch[每天自动检查]";
    app.wait_for(toggle).unwrap();
    app.press(toggle).unwrap();
    app.wait_for("已改为仅手动检查").unwrap();

    assert_eq!(
        update_state_field("settings.auto_check"),
        "false",
        "the switch never reached the state file"
    );
    // The throttle is not the caller's to move: a settings write that reset it would let a client
    // reach the feed as often as it liked.
    assert_ne!(
        update_state_field("settings.last_check_at"),
        "0",
        "saving the switch reset the daily throttle"
    );
    drop(app);

    let mut again = desktop(&profile);
    again.wait_for("Kitty Pro").unwrap();
    again.press("设置").unwrap();
    again.wait_for(toggle).unwrap();
    assert!(
        !again.screen().selected(toggle).unwrap_or(true),
        "the daily check came back on after a relaunch"
    );
}

/// A phone-width window still asks the release feed on the same terms as a desktop one.
///
/// This pins a decision rather than a coincidence. The card could reasonably have derived
/// "updatable" from the window it is drawn in, and does not: an NSH phone build is Android, which
/// is a published channel, so a shape-based rule would park a control the server would happily
/// have answered. The platform string sent to the backend is the only input that decides.
#[test]
fn a_phone_window_asks_the_release_feed_like_any_other() {
    // Held for the body: it owns the lock on the scratch profile directory.
    let _profile = Profile::claim("a_phone_window_asks_the_release_feed_like_any_other");
    let dir = std::path::PathBuf::from(
        std::env::var("KITTY_PROFILE_DIR").expect("the scratch directory"),
    );
    std::fs::write(
        dir.join("update.json"),
        update_state(Some(("9.9.9", "v9.9.9")), true),
    )
    .expect("a seedable update state");
    // `Environment::default()` is the phone idiom, which is what the platform label comes from;
    // the height only keeps the card measured rather than scrolled past.
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(390.0), px(2400.0)),
        environment: Environment {
            window: Size::new(px(390.0), px(2400.0)),
            ..Environment::default()
        },
        ..EngineConfig::new(nsh_app_kitty::app_root())
    })
    .expect("the app starts");

    app.wait_for("设置").unwrap();
    app.press("设置").unwrap();
    app.wait_for("应用更新").unwrap();
    app.wait_for("有更新").unwrap();
    assert!(
        app.screen().enabled("检查更新").unwrap(),
        "a narrow window decided for itself that this build has no channel"
    );
    // The about card on the same screen names the same platform, so the two cannot disagree about
    // which build this is.
    app.wait_for("text:/· Mobile$/").unwrap();
}
