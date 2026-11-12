//! The root layout. A persistent sidebar carries navigation on a desktop; a phone gets a
//! bottom bar, since a 264pt rail would eat most of its width.

use crate::i18n;
use crate::pages;
use crate::store::{store, AppStore, Screen};
use crate::ui::{controls, theme, Icon};
use nsh_base::theme as base_theme;
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
use nsh_core::element::desktop::main_window;
use nsh_core::prelude::*;

/// How tall the window's top band is once the system's title bar is gone.
///
/// The user asked for a borderless window: the native bar across the top was the ugliest thing in
/// it. `TitleBarStyle::Hidden` is the right call rather than a truly frameless window, because it
/// keeps the traffic lights — a window with no close button can only be killed from the Dock or
/// `Cmd+Q`, and the app is meant to be quittable by press — and lets our own drawing reach the
/// top edge, which is what removes the bar.
///
/// What it does not do is move the lights: they float over whatever is at the top-left, so the
/// rail has to keep this much of its own height empty or the brand sits under them. 44 is the
/// tallest AppKit still treats as compact chrome, and the same number is handed to
/// the window's `title_bar` height so the controls are centred in the band we actually reserved.
pub const TITLE_BAR_HEIGHT: f32 = 44.0;

/// The rail's empty top band: where the traffic lights live, and the window's drag handle.
fn title_bar_band() -> impl IntoElement {
    view()
        .h(px(TITLE_BAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .window_drag(true)
}

/// The root's whole surface: theme, background, and the chrome for the idiom.
pub fn root() -> impl IntoElement {
    let store = store();
    // Bootstrap belongs to the root's owner, so both reads are cancelled with the app.
    // Nothing else asks the core what it is running, and without this the shell sits in
    // "正在检测" for ever with an empty node list.
    effect(move || {
        // The window has no native title bar. Asked for in the boot effect rather than in the
        // body so it is sent once: the root re-renders on navigation, and a command per render
        // would be a command per press for a thing that never changes.
        if is_desktop() {
            // 0.5.0 replaced the `set_title_bar` command with a window attribute, so the
            // hidden bar is now one field of a partial `WindowAttributes`: every other
            // attribute keeps its value.
            // Both types are `#[non_exhaustive]`, which forbids a struct literal from
            // another crate, so they are filled field by field from their defaults.
            let mut bar = TitleBarOptions::default();
            bar.style = TitleBarStyle::Hidden;
            bar.height = Some(px(TITLE_BAR_HEIGHT));
            let mut attributes = WindowAttributes::default();
            attributes.title_bar = Some(bar);
            main_window().set(attributes);
        }
        store.load_profile();
        store.refresh_status();
        // The OS proxy is the thing that makes a running core carry traffic, and the app
        // does not own it: a previous session, or another program, may have set it. Read it
        // before offering a switch about it.
        store.refresh_system_proxy();
        // The same argument applies to the login item, which is a file an installer or a
        // cleanup tool can delete while the app is closed. Both reads are here for the same
        // reason: a switch that shows the last value this app wrote is a claim about the
        // operating system, and the operating system was not asked.
        store.refresh_autostart();
        // The sync card is the one place the app can overwrite its own settings with a blank,
        // so it waits for this read rather than offering a form: until it lands every control is
        // parked, and a failed read stays parked with a retry.
        store.refresh_sync_config();
        // The update check runs here rather than from the card so it happens on launch whether or
        // not the settings page is ever opened; the card is a view over whatever it last learned.
        store.refresh_update_state();
    });
    // The core streams logs only while collection is on, and the log page is the only
    // reader, so the shell decides here rather than letting a poll run behind every screen.
    // The effect re-runs on the screen and the core state, which is exactly when the answer
    // can change.
    effect(move || {
        let _ = (store.screen.get(), store.core_state());
        store.sync_log_collection();
    });
    // Reading the language here is what makes a switch reach every label in the app.
    //
    // Most labels are built as `text("节点")` — the lookup happens when the element is built, and
    // NSH gives an app no way to rebuild a tree from inside, so such a label would keep the
    // language the page was first opened in. A component re-renders when a signal it reads
    // changes, and a re-render runs its body again, which builds the pages afresh in the new
    // language. So this one read turns "every label must be written as a closure" from a rule
    // ~450 call sites have to obey into one the root obeys for them.
    //
    // Labels that already read `tr` inside a binding are unaffected — they were correct either
    // way — and a rebuild costs one frame per press of a control the user pressed deliberately.
    view()
        .flex_1()
        .flex_row()
        .bg(move || base_theme().background)
        .text_color(move || base_theme().foreground)
        .child(when(is_desktop, move || sidebar(store)))
        .child(content_column(store))
        .child(store.feedback().stack())
}

/// The rail: identity, grouped navigation, appearance at the foot.
///
/// Its top padding is the title-bar band rather than `p_5`'s: with the system bar hidden the
/// traffic lights float over the top-left corner, and the brand is the one thing in the rail that
/// must not be under them.
fn sidebar(store: AppStore) -> impl IntoElement {
    view()
        .h_full()
        .w(px(264.0))
        .flex_shrink_0()
        .flex_col()
        .gap_6()
        .px_5()
        .pb_5()
        .border_r_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(
            view()
                .flex_col()
                .gap_2()
                .child(title_bar_band())
                .child(brand(store)),
        )
        .child(nav_groups(store))
}

/// The identity block. The kicker is the core build actually running, so the
/// sidebar never claims a version the engine has not reported.
fn brand(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            view()
                .h(px(36.0))
                .w(px(36.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(theme::radius_tile())
                .bg(move || base_theme().primary)
                .child(
                    controls::icon(Icon::Globe, 20.0)
                        .text_color(move || base_theme().primary_foreground),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    text("Kitty Pro")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_family(theme::font_display())
                        .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || brand_kicker(store.core_version()))
                        .text_xs()
                        .text_color(theme::text_faint()),
                ),
        )
}

/// Navigation is grouped by what the user is doing rather than listed flat: six
/// sibling items give no hint about which change the core.
fn nav_groups(store: AppStore) -> impl IntoElement {
    let group = |label: &'static str, screens: &'static [Screen]| {
        view()
            .flex_col()
            .gap_1()
            .child(
                text(move || i18n::tr(label))
                    .text_xs()
                    .text_color(theme::text_faint()),
            )
            .children(screens.iter().map(move |screen| nav_item(store, *screen)))
    };
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .child(group("连接", &[Screen::Overview, Screen::Nodes]))
        .child(group("配置", &[Screen::Subscriptions, Screen::Rules]))
        .child(group("运维", &[Screen::Logs, Screen::Settings]))
}

/// The count a rail row carries, matching the original's trailing chip.
///
/// Four of the six rows count something the page then lists, which is what makes the number worth
/// its space: it says how much is waiting before the row is opened. 总览 has nothing to count and
/// 设置 carries the update dot instead — the two never share a slot.
fn nav_count(store: AppStore, screen: Screen) -> Option<usize> {
    match screen {
        Screen::Overview => None,
        Screen::Nodes => Some(store.nodes().len()),
        Screen::Subscriptions => Some(store.subscription_count()),
        // Rules and its DNS overrides are one list to the user's eye: the page holds both, so the
        // row counts both.
        Screen::Rules => Some(store.custom_rule_count() + store.dns_overrides().len()),
        Screen::Logs => Some(store.logs().len()),
        Screen::Settings => None,
    }
}

/// What the row says about itself, read at draw time so an update discovered after launch shows up
/// without the page being rebuilt.
fn nav_mark(store: AppStore, screen: Screen) -> controls::NavMark {
    match nav_count(store, screen) {
        Some(count) => controls::NavMark::Count(count),
        None if screen == Screen::Settings && store.update_is_actionable() => {
            controls::NavMark::Update
        }
        None => controls::NavMark::None,
    }
}

fn nav_item(store: AppStore, screen: Screen) -> impl IntoElement {
    let active = computed(move || store.screen.get() == screen);
    controls::nav_row(
        screen.label(),
        screen.icon(),
        active,
        move || nav_mark(store, screen),
        move || store.set_screen(screen),
    )
}

/// The column that is not the rail: header, page, and on a phone the tab bar under them.
///
/// The bar belongs inside this column rather than beside it. In a row a full-width bar
/// leaves the page no width at all, which is how the overview vanished on a phone.
fn content_column(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .flex_col()
        .child(page_header(store))
        .child(
            // The shell bounds the page but does not scroll it. Each screen owns its scroll
            // region: wrapping everything here leaves a virtual list with unbounded height,
            // which lays out no rows at all, and gives a list page two bars on one axis.
            //
            // The bottom inset only. The top one belongs to `page_header`, which is what sits
            // under the status bar; asking for it here too spent the same inset twice — once
            // as a gap below the header and once as the header's own clearance, which was then
            // missing and left the badge under the clock.
            view()
                .flex_1()
                .min_h(px(0.0))
                .pb_safe_area()
                .child(screen_body(store)),
        )
        .child(when(move || !is_desktop(), move || tab_bar(store)))
}

/// The frame a text-and-cards page scrolls in: bounded height, and content held to a
/// readable measure on a wide window.
pub fn page_scroll(content: impl IntoElement) -> impl IntoElement {
    scroll().flex_1().min_h(px(0.0)).child(
        view()
            .w_full()
            .max_w(px(1080.0))
            .self_center()
            .flex_col()
            .gap_4()
            .px_5()
            .pt_5()
            // Every page floats an action over its own bottom edge, so the frame that scrolls
            // them all leaves the room once, rather than each page guessing at it.
            .pb(controls::action_clearance())
            .child(content),
    )
}

pub fn connect_label(store: AppStore) -> String {
    // Translated here rather than at the two pages that show it: the words are the shell's, and
    // the disc on 总览 and any later caller must say the same thing in the same language.
    let key = match store.busy() {
        crate::store::CoreBusy::Toggling if store.connected() => "正在断开",
        crate::store::CoreBusy::Toggling => "正在启动",
        crate::store::CoreBusy::Restarting => "正在重启",
        crate::store::CoreBusy::Tun => "正在申请网络权限",
        crate::store::CoreBusy::Idle if store.connected() => "断开",
        crate::store::CoreBusy::Idle => "启动代理",
    };
    i18n::tr(key)
}

/// The page's own bar: its name and state on the left, the two controls that describe the
/// window on the right.
///
/// It drags the window, because hiding the title bar leaves the rail's 44pt band as the only
/// handle and a 264pt strip is a poor thing to grab. The press handlers inside it stay
/// interactive — NSH keeps buttons working inside a drag region — so this is a title bar in
/// behaviour as well as in position.
///
/// Its top padding is the safe area's inset *plus* the 16pt the desktop uses, not the 16pt
/// alone. On a phone NSH's `AppActivity` draws edge to edge, so the window's top edge is under
/// the status bar and the inset is the only thing that keeps 总览 and 内核已就绪 out of the
/// clock; `py_4` by itself put the header's text at 16pt from the top of the screen and let the
/// badge sit under the status bar icons. The body below applies the same inset as its *own* top
/// padding (`content_column`), which is why the gap existed and simply was in the wrong place:
/// the header overlapped the status bar and the first card began a status bar's height below
/// where it should.
fn page_header(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .px_5()
        .py_4()
        .pt_safe_area_plus(px(16.0))
        .window_drag(is_desktop)
        .border_b_1()
        .border_color(move || base_theme().border)
        .child(
            view()
                .flex_col()
                .gap_0p5()
                .child(
                    // The screen's own name is a key; the builder that draws the rail translates
                    // the same key, so the header and the row that selected it cannot disagree.
                    text(move || i18n::tr(store.screen.get().label()))
                        .text_size(px(18.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_family(theme::font_display())
                        .text_color(move || base_theme().foreground),
                )
                .child(
                    text(move || subtitle(store))
                        .text_xs()
                        .text_color(theme::text_faint()),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(theme_toggle(store))
                .child(controls::connection_chip(store)),
        )
}

/// The appearance toggle in the header, where the original keeps it: one press, and its
/// name says which appearance the press will bring. The full labelled switch lives on
/// Settings, so the same choice is never offered twice in one view.
fn theme_toggle(store: AppStore) -> impl IntoElement {
    let dark = store.dark_mode;
    controls::icon_button(
        move || if dark.get() { Icon::Sun } else { Icon::Moon },
        move || SharedString::from(theme_label(dark.get())),
        move || {
            let next = !dark.get_untracked();
            store.set_dark_mode(next);
        },
    )
}

/// Name the action, not the state: a control labelled "深色模式" reads as if dark were
/// already on. The Settings switch keeps that label because there it names a setting.
pub fn theme_label(dark: bool) -> String {
    i18n::tr(if dark {
        "切换浅色外观"
    } else {
        "切换深色外观"
    })
}

/// The one-line summary under a page title: what the screen is about to do.
fn subtitle(store: AppStore) -> String {
    use crate::domain::{core_state_label, mode_label};
    if store.core_action_busy() {
        return i18n::tr("正在应用设置");
    }
    format!(
        "{} · {}",
        i18n::tr(core_state_label(&store.core_state())),
        i18n::tr(mode_label(store.tunnel_mode()))
    )
}

fn screen_body(store: AppStore) -> impl IntoElement {
    let screen = store.screen;
    // Only the current screen is built. Keeping the others mounted behind `hidden()`
    // would hold their timers and polls alive and would let a text assertion pass for a
    // screen nobody is looking at.
    //
    // `i18n::lang()` is read *inside* the closure, next to `screen.get()`, and that is what makes
    // a language change reach a page. A `dynamic` region re-runs its closure when a signal read in
    // the closure changes, and re-running it builds the page again — so every label the page builds
    // as a plain `text("节点")` is looked up afresh, in the new language, without any of the ~450
    // call sites needing to know a table exists. Reading it in `root()` instead would not do: the
    // region's closure is not the root's body, and it only follows signals it reads itself.
    dynamic(move || {
        let _lang = i18n::lang();
        match screen.get() {
            Screen::Overview => pages::overview::overview(store).into_element(),
            Screen::Settings => pages::settings::settings(store).into_element(),
            Screen::Nodes => pages::nodes::nodes(store).into_element(),
            Screen::Subscriptions => pages::subscriptions::subscriptions(store).into_element(),
            Screen::Rules => pages::rules::rules(store).into_element(),
            Screen::Logs => pages::logs::logs(store).into_element(),
        }
    })
}

/// The phone's navigation: all six destinations within thumb reach, no groups.
fn tab_bar(store: AppStore) -> impl IntoElement {
    view()
        .w_full()
        .flex_shrink_0()
        .flex_row()
        .pb_safe_area()
        .border_t_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .children(
            Screen::ALL
                .iter()
                .map(move |screen| tab_item(store, *screen)),
        )
}

/// The phone's mark: the update dot only, on 设置.
///
/// The original's bottom bar carries no counts at all — five names in a row have no room for a
/// chip beside each — while the dot still has to reach a phone user, since that is the only way
/// they learn an update is waiting without opening a page.
fn tab_mark(store: AppStore, screen: Screen) -> controls::NavMark {
    if screen == Screen::Settings && store.update_is_actionable() {
        controls::NavMark::Update
    } else {
        controls::NavMark::None
    }
}

fn tab_item(store: AppStore, screen: Screen) -> impl IntoElement {
    let active = computed(move || store.screen.get() == screen);
    let glyph = screen.icon();
    controls::nav_tab(
        screen.label(),
        glyph,
        active,
        move || tab_mark(store, screen),
        move || store.set_screen(screen),
    )
}

/// The core build as the sidebar says it: `kitty-pro-embedded/1.13.14` reads
/// `EMBEDDED · 1.13.14`.
pub fn brand_kicker(core_version: Option<String>) -> String {
    match core_version {
        Some(version) => version
            .strip_prefix("kitty-pro-")
            .unwrap_or(&version)
            .replace('/', " · ")
            .to_uppercase(),
        None => "SING-BOX CLIENT".to_string(),
    }
}
