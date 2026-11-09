//! The root layout. A persistent sidebar carries navigation on a desktop; a phone gets a
//! bottom bar, since a 264pt rail would eat most of its width.

use crate::pages;
use crate::store::{store, AppStore, Screen};
use crate::ui::{controls, theme, Icon};
use nsh_base::theme as base_theme;
use nsh_core::prelude::*;

/// The root's whole surface: theme, background, and the chrome for the idiom.
pub fn root() -> impl IntoElement {
    let store = store();
    // Bootstrap belongs to the root's owner, so both reads are cancelled with the app.
    // Nothing else asks the core what it is running, and without this the shell sits in
    // "正在检测" for ever with an empty node list.
    effect(move || {
        store.load_profile();
        store.refresh_status();
        // The OS proxy is the thing that makes a running core carry traffic, and the app
        // does not own it: a previous session, or another program, may have set it. Read it
        // before offering a switch about it.
        store.refresh_system_proxy();
    });
    // The core streams logs only while collection is on, and the log page is the only
    // reader, so the shell decides here rather than letting a poll run behind every screen.
    // The effect re-runs on the screen and the core state, which is exactly when the answer
    // can change.
    effect(move || {
        let _ = (store.screen.get(), store.core_state());
        store.sync_log_collection();
    });
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
fn sidebar(store: AppStore) -> impl IntoElement {
    view()
        .h_full()
        .w(px(264.0))
        .flex_shrink_0()
        .flex_col()
        .gap_6()
        .p_5()
        .border_r_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(brand(store))
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
            .child(text(label).text_xs().text_color(theme::text_faint()))
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

fn nav_item(store: AppStore, screen: Screen) -> impl IntoElement {
    let active = computed(move || store.screen.get() == screen);
    controls::nav_row(screen.label(), screen.icon(), active, move || {
        store.set_screen(screen)
    })
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
            view()
                .flex_1()
                .min_h(px(0.0))
                .py_safe_area()
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
            .py_5()
            .child(content),
    )
}

fn page_header(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .px_5()
        .py_4()
        .border_b_1()
        .border_color(move || base_theme().border)
        .child(
            view()
                .flex_col()
                .gap_0p5()
                .child(
                    text(move || store.screen.get().label())
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
    if dark {
        "切换浅色外观".to_owned()
    } else {
        "切换深色外观".to_owned()
    }
}

/// The one-line summary under a page title: what the screen is about to do.
fn subtitle(store: AppStore) -> String {
    use crate::domain::{core_state_label, mode_label};
    if store.core_action_busy() {
        "正在应用设置".to_owned()
    } else {
        format!(
            "{} · {}",
            core_state_label(&store.core_state()),
            mode_label(store.tunnel_mode())
        )
    }
}

fn screen_body(store: AppStore) -> impl IntoElement {
    let screen = store.screen;
    // Only the current screen is built. Keeping the others mounted behind `hidden()`
    // would hold their timers and polls alive and would let a text assertion pass for a
    // screen nobody is looking at.
    dynamic(move || match screen.get() {
        Screen::Overview => pages::overview::overview(store).into_element(),
        Screen::Settings => pages::settings::settings(store).into_element(),
        Screen::Nodes => pages::nodes::nodes(store).into_element(),
        Screen::Subscriptions => pages::subscriptions::subscriptions(store).into_element(),
        Screen::Rules => pages::rules::rules(store).into_element(),
        Screen::Logs => pages::logs::logs(store).into_element(),
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

fn tab_item(store: AppStore, screen: Screen) -> impl IntoElement {
    let active = computed(move || store.screen.get() == screen);
    let glyph = screen.icon();
    controls::nav_tab(screen.label(), glyph, active, move || {
        store.set_screen(screen)
    })
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
