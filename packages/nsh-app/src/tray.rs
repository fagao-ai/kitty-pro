//! The system tray: the app in the notification area when its window is not
//! in the way.
//!
//! Two halves, so the decision-making is testable without a desktop:
//!
//! * The pure half builds the whole tray — tooltip and every menu entry,
//!   labels included — from plain state, as plain specs. It is a function of
//!   nothing but its arguments and the language, so its tests run on any
//!   thread.
//! * [`mount`] is the reactive bridge: an effect that reads the store and the
//!   language and hands the specs to `nsh_core::element::desktop::set_tray`,
//!   plus the close-to-hide wiring the tray implies. Off desktop, `set_tray`
//!   is a no-op and the bridge compiles to effects that decide nothing.
//!
//! The menu mirrors the one the Dioxus front end carried (deleted with it in
//! `fb384af`): a status line, the subscriptions and their nodes, the system
//! proxy switch, the copied environment, and Show over Quit. The same
//! truncation limits and the same `●`/`○` markers came across too — they were
//! tuned once and nothing about the tray's width has changed since.

use crate::i18n::{self, Lang};
use crate::store::{self, AppStore, CoreBusy};
use nsh_core::desktop::{MenuRole, WindowAttributes};
use nsh_core::element::desktop::{main_window, set_tray, tray, MenuItem};
use nsh_core::prelude::*;

/// The tray icon, from the crate's `assets/` directory beside the window
/// icon `nsh.json` names. The black SVG is a *template* on macOS: the menu
/// bar tints it to its own text color, dark or light. Windows has no such
/// mechanism and its taskbar is usually dark, so there the white SVG keeps
/// its own color and stays visible on both themes — the black one on a dark
/// taskbar is a blob.
#[cfg(target_os = "macos")]
macro_rules! tray_icon {
    () => {
        asset!("kitty-pro-tray.svg")
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! tray_icon {
    () => {
        asset!("kitty-pro-tray-white.svg")
    };
}

/// What one menu row does when it is chosen. `Quit` is carried by its
/// `MenuRole` and listed here only so the spec stays exhaustive.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayAction {
    Show,
    Quit,
    Subscription(u64),
    Node(String),
    SystemProxy,
    CopyEnv,
}

/// One menu row, decided before any NSH type is touched.
#[derive(Debug, PartialEq, Eq)]
struct TrayItemSpec {
    label: String,
    /// `Some` makes the row a checkable item, checked or not.
    checked: Option<bool>,
    disabled: bool,
    action: Option<TrayAction>,
}

/// One slot in the tray's menu: a row, a folded list, or the line between.
#[derive(Debug, PartialEq, Eq)]
enum TrayMenuEntry {
    Item(TrayItemSpec),
    Submenu {
        label: String,
        disabled: bool,
        items: Vec<TrayItemSpec>,
    },
    Separator,
}

/// Everything the menu and the tooltip are a function of, gathered from the
/// store. Plain data with borrowed lists, so the builders stay pure and the
/// bridge can build it inline from locals.
struct TrayState<'a> {
    profile_loaded: bool,
    busy: CoreBusy,
    connected: bool,
    /// Each subscription as `(id, name, node count)`.
    subscriptions: &'a [(u64, String, usize)],
    active_subscription_id: Option<u64>,
    /// The active subscription's nodes, as `(tag, name)`.
    nodes: &'a [(String, String)],
    selected_tag: &'a str,
    system_proxy_enabled: bool,
    system_proxy_ready: bool,
    system_proxy_supported: bool,
}

impl TrayState<'_> {
    fn busy(&self) -> bool {
        self.busy != CoreBusy::Idle
    }

    /// The system proxy row is enabled on exactly the terms the in-app
    /// switch is: the old tray's expression, carried over verbatim. An
    /// enabled proxy stays reachable while the core is down, so it can
    /// always be switched back off.
    fn system_proxy_row_enabled(&self) -> bool {
        self.profile_loaded
            && self.system_proxy_ready
            && self.system_proxy_supported
            && !self.busy()
            && (self.connected || self.system_proxy_enabled)
    }
}

/// The tooltip: what the pointer resting on the icon should say. The node's
/// name when it is connected, the work in flight when one is, the bare truth
/// otherwise.
fn tray_tooltip(
    lang: Lang,
    profile_loaded: bool,
    busy: CoreBusy,
    connected: bool,
    selected_node: Option<&str>,
) -> String {
    let state = if !profile_loaded {
        i18n::translate(lang, "正在加载配置")
    } else if busy != CoreBusy::Idle {
        i18n::translate(lang, "正在应用更改")
    } else if connected {
        i18n::translate(lang, "已连接")
    } else {
        i18n::translate(lang, "已断开")
    };
    // The node's name only while it is the one traffic is settled on: a
    // busy app is between nodes, so naming either would be a guess.
    let settled = connected && busy == CoreBusy::Idle;
    match (settled, selected_node) {
        (true, Some(node)) => format!("Kitty Pro · {state} · {}", menu_text(node, 48)),
        _ => format!("Kitty Pro · {state}"),
    }
}

/// A label fit for a menu row: mnemonics escaped, then cut on characters —
/// not bytes, which would split a CJK name mid-glyph — with an ellipsis.
/// The limits themselves live at the call sites, the old tray's: 28 for a
/// submenu's own title, 42 for a subscription row, 48 for a node row.
fn menu_text(value: &str, max_chars: usize) -> String {
    let mut text = value.replace('&', "&&");
    if text.chars().count() <= max_chars {
        return text;
    }
    text = text.chars().take(max_chars.saturating_sub(3)).collect();
    text.push_str("...");
    text
}

/// The whole menu, as plain specs.
///
/// The status line sits on top where the old tray put it, inert; the quit
/// row is the one thing that is never disabled, because a hidden window
/// leaves it as the only exit.
fn build_menu_specs(lang: Lang, state: &TrayState) -> Vec<TrayMenuEntry> {
    let busy = state.busy();
    let status = if !state.profile_loaded {
        "状态：正在加载配置"
    } else if busy {
        "状态：正在应用更改"
    } else if state.connected {
        "状态：已连接"
    } else {
        "状态：未连接"
    };

    let mut menu = vec![TrayMenuEntry::Item(TrayItemSpec {
        label: i18n::translate(lang, status),
        checked: None,
        disabled: true,
        action: None,
    })];

    // Subscriptions. The title carries the active one, so the folded list
    // answers "which am I on" without being opened.
    let active_subscription = state
        .subscriptions
        .iter()
        .find(|(id, ..)| Some(*id) == state.active_subscription_id)
        .map(|(_, name, _)| menu_text(name, 28))
        .unwrap_or_else(|| i18n::translate(lang, "未选择"));
    let subscription_rows = if state.subscriptions.is_empty() {
        vec![TrayItemSpec {
            label: i18n::translate(lang, "暂无订阅"),
            checked: None,
            disabled: true,
            action: None,
        }]
    } else {
        state
            .subscriptions
            .iter()
            .map(|(id, name, count)| TrayItemSpec {
                label: format!(
                    "{} {} ({})",
                    if Some(*id) == state.active_subscription_id {
                        "●"
                    } else {
                        "○"
                    },
                    menu_text(name, 42),
                    i18n::fill_at(lang, "{} 个节点", &[count]),
                ),
                checked: None,
                disabled: busy,
                action: Some(TrayAction::Subscription(*id)),
            })
            .collect()
    };
    menu.push(TrayMenuEntry::Submenu {
        label: format!(
            "{} · {}",
            i18n::translate(lang, "订阅"),
            active_subscription
        ),
        disabled: !state.profile_loaded || busy || state.subscriptions.is_empty(),
        items: subscription_rows,
    });

    // Nodes of the active subscription only, as the old tray drew them.
    let active_node = state
        .nodes
        .iter()
        .find(|(tag, _)| *tag == state.selected_tag)
        .map(|(_, name)| menu_text(name, 28))
        .unwrap_or_else(|| i18n::translate(lang, "未选择"));
    let node_rows = if state.nodes.is_empty() {
        vec![TrayItemSpec {
            label: i18n::translate(lang, "当前订阅没有可用节点"),
            checked: None,
            disabled: true,
            action: None,
        }]
    } else {
        state
            .nodes
            .iter()
            .map(|(tag, name)| TrayItemSpec {
                label: format!(
                    "{} {}",
                    if *tag == state.selected_tag {
                        "●"
                    } else {
                        "○"
                    },
                    menu_text(name, 48),
                ),
                checked: None,
                disabled: busy,
                action: Some(TrayAction::Node(tag.clone())),
            })
            .collect()
    };
    menu.push(TrayMenuEntry::Submenu {
        label: format!("{} · {}", i18n::translate(lang, "节点"), active_node),
        disabled: !state.profile_loaded || busy || state.nodes.is_empty(),
        items: node_rows,
    });

    menu.push(TrayMenuEntry::Separator);
    menu.push(TrayMenuEntry::Item(TrayItemSpec {
        label: i18n::translate(lang, "系统代理"),
        checked: Some(state.system_proxy_enabled),
        disabled: !state.system_proxy_row_enabled(),
        action: Some(TrayAction::SystemProxy),
    }));
    menu.push(TrayMenuEntry::Item(TrayItemSpec {
        label: i18n::translate(lang, "复制环境变量"),
        checked: None,
        disabled: !state.profile_loaded || busy,
        action: Some(TrayAction::CopyEnv),
    }));

    menu.push(TrayMenuEntry::Separator);
    menu.push(TrayMenuEntry::Item(TrayItemSpec {
        label: i18n::translate(lang, "显示 Kitty Pro"),
        checked: None,
        disabled: false,
        action: Some(TrayAction::Show),
    }));
    menu.push(TrayMenuEntry::Item(TrayItemSpec {
        label: i18n::translate(lang, "退出"),
        checked: None,
        disabled: false,
        action: Some(TrayAction::Quit),
    }));
    menu
}

/// Run what a chosen row asks for. Every store action gates itself, so a row
/// that slipped past a stale disabled state still cannot do harm.
fn perform(action: TrayAction, store: AppStore) {
    match action {
        TrayAction::Show => {
            main_window().show();
            main_window().focus();
        }
        // The platform ends the app for a Quit role, and the engine runs the
        // quit hook (`process::register_exit_cleanup`) on the way out.
        TrayAction::Quit => {}
        TrayAction::Subscription(id) => store.select_subscription(id),
        TrayAction::Node(tag) => store.select_group_outbound("proxy".to_owned(), tag),
        TrayAction::SystemProxy => {
            let enable = !store.system_proxy().is_enabled();
            store.set_system_proxy(enable);
        }
        TrayAction::CopyEnv => store.copy_proxy_environment(),
    }
}

/// One spec, as an NSH menu item with its handler attached.
fn menu_item(spec: TrayItemSpec, store: AppStore) -> MenuItem {
    let mut item = match spec.action {
        // A role item carries the platform's quit behaviour; the label is
        // ours so the menu reads in the app's language, like every row.
        Some(TrayAction::Quit) => MenuItem::role(MenuRole::Quit).label(spec.label),
        _ => MenuItem::new(spec.label),
    };
    if let Some(checked) = spec.checked {
        item = item.checked(checked);
    }
    if spec.disabled {
        item = item.disabled(true);
    }
    if let Some(action) = spec.action {
        item = item.on_select(move || perform(action.clone(), store));
    }
    item
}

fn menu_entry(entry: TrayMenuEntry, store: AppStore) -> MenuItem {
    match entry {
        TrayMenuEntry::Item(spec) => menu_item(spec, store),
        TrayMenuEntry::Submenu {
            label,
            disabled,
            items,
        } => MenuItem::submenu(
            label,
            items
                .into_iter()
                .map(|spec| menu_item(spec, store))
                .collect::<Vec<_>>(),
        )
        .disabled(disabled),
        TrayMenuEntry::Separator => MenuItem::separator(),
    }
}

/// Wire the tray to the store. Called once from the root component; from
/// there the effect rebuilds icon, tooltip and menu whenever anything it
/// reads changes, for as long as the app lives.
pub(crate) fn mount() {
    // Closing the window hides it rather than ending the app: the tray is
    // the exit now. Sent once, like the hidden title bar in `shell::root`,
    // since the attribute never changes; `hide` does not fire the quit hook,
    // and the real quits (the menu's Quit role, the system) still run
    // `register_exit_cleanup` on their way out.
    effect(move || {
        if is_desktop() {
            let mut attributes = WindowAttributes::default();
            attributes.close_on_press = Some(false);
            main_window().set(attributes);
            main_window().on_close_press(move |_reason| {
                main_window().hide();
            });
        }
    });

    effect(move || {
        let store = store::store();
        let lang = i18n::lang();
        let subscriptions = store
            .subscriptions()
            .into_iter()
            .map(|subscription| (subscription.id, subscription.name, subscription.nodes.len()))
            .collect::<Vec<_>>();
        let nodes = store
            .nodes()
            .into_iter()
            .map(|node| (node.tag, node.name))
            .collect::<Vec<_>>();
        let selected_tag = store.selected_tag();
        let state = TrayState {
            profile_loaded: store.profile_loaded(),
            busy: store.busy(),
            connected: store.connected(),
            subscriptions: &subscriptions,
            active_subscription_id: store.active_subscription_id(),
            nodes: &nodes,
            selected_tag: &selected_tag,
            system_proxy_enabled: store.system_proxy().is_enabled(),
            system_proxy_ready: store.system_proxy().is_ready(),
            system_proxy_supported: store.system_proxy().is_supported(),
        };
        let tooltip = tray_tooltip(
            lang,
            state.profile_loaded,
            state.busy,
            state.connected,
            state
                .nodes
                .iter()
                .find(|(tag, _)| *tag == state.selected_tag)
                .map(|(_, name)| name.as_str()),
        );
        let items = build_menu_specs(lang, &state)
            .into_iter()
            .map(|entry| menu_entry(entry, store))
            .collect::<Vec<_>>();
        set_tray(
            tray()
                .icon(tray_icon!())
                .tooltip(tooltip)
                .menu(items)
                .on_click(move |_| {
                    main_window().show();
                    main_window().focus();
                }),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The common shape: a loaded profile, a ready and supported proxy
    /// setting, and the proxy itself off. The proxy-matrix test builds its
    /// `TrayState`s by hand, since the proxy flags are what it varies.
    fn fixture<'a>(
        busy: CoreBusy,
        connected: bool,
        subscriptions: &'a [(u64, String, usize)],
        active: Option<u64>,
        nodes: &'a [(String, String)],
        selected_tag: &'a str,
    ) -> TrayState<'a> {
        TrayState {
            profile_loaded: true,
            busy,
            connected,
            subscriptions,
            active_subscription_id: active,
            nodes,
            selected_tag,
            system_proxy_enabled: false,
            system_proxy_ready: true,
            system_proxy_supported: true,
        }
    }

    #[test]
    fn menu_text_escapes_mnemonics_and_truncates_on_characters() {
        assert_eq!(menu_text("A&B", 10), "A&&B");
        assert_eq!(menu_text("香港节点一号", 5), "香港...");
        assert_eq!(menu_text("短", 5), "短");
    }

    #[test]
    fn tooltip_names_the_node_when_connected() {
        let zh = Lang::Chinese;
        assert_eq!(
            tray_tooltip(zh, true, CoreBusy::Idle, true, Some("香港 01")),
            "Kitty Pro · 已连接 · 香港 01"
        );
        assert_eq!(
            tray_tooltip(zh, true, CoreBusy::Idle, false, None),
            "Kitty Pro · 已断开"
        );
        assert_eq!(
            tray_tooltip(zh, false, CoreBusy::Idle, false, None),
            "Kitty Pro · 正在加载配置"
        );
        assert_eq!(
            tray_tooltip(zh, true, CoreBusy::Restarting, true, Some("x")),
            "Kitty Pro · 正在应用更改"
        );
    }

    #[test]
    fn the_status_row_sits_on_top_and_is_never_clickable() {
        let subscriptions = [(1, "机场".to_owned(), 2)];
        let nodes = [("a".to_owned(), "香港 01".to_owned())];
        let state = fixture(CoreBusy::Idle, false, &subscriptions, Some(1), &nodes, "a");
        let menu = build_menu_specs(Lang::Chinese, &state);
        let TrayMenuEntry::Item(spec) = &menu[0] else {
            panic!("the first entry must be the status row");
        };
        assert!(spec.disabled);
        assert_eq!(spec.label, "状态：未连接");
    }

    #[test]
    fn subscriptions_and_nodes_mark_the_active_rows() {
        let subscriptions = [(1, "机场甲".to_owned(), 2), (2, "机场乙".to_owned(), 3)];
        let nodes = [
            ("a".to_owned(), "香港 01".to_owned()),
            ("b".to_owned(), "东京 01".to_owned()),
        ];
        let state = fixture(CoreBusy::Idle, true, &subscriptions, Some(2), &nodes, "b");
        let menu = build_menu_specs(Lang::Chinese, &state);
        let TrayMenuEntry::Submenu {
            label,
            disabled,
            items,
        } = &menu[1]
        else {
            panic!("the second entry must be the subscriptions submenu");
        };
        assert_eq!(label, "订阅 · 机场乙");
        assert!(!disabled);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "○ 机场甲 (2 个节点)");
        assert_eq!(items[1].label, "● 机场乙 (3 个节点)");
        assert!(!items[0].disabled);

        let TrayMenuEntry::Submenu { label, items, .. } = &menu[2] else {
            panic!("the third entry must be the nodes submenu");
        };
        assert_eq!(label, "节点 · 东京 01");
        assert_eq!(items[0].label, "○ 香港 01");
        assert_eq!(items[1].label, "● 东京 01");
    }

    #[test]
    fn empty_lists_fold_into_placeholders() {
        let state = fixture(CoreBusy::Idle, false, &[], None, &[], "");
        let menu = build_menu_specs(Lang::Chinese, &state);
        let TrayMenuEntry::Submenu {
            disabled, items, ..
        } = &menu[1]
        else {
            panic!("the second entry must be the subscriptions submenu");
        };
        assert!(disabled);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "暂无订阅");
        assert!(items[0].disabled);
        let TrayMenuEntry::Submenu { items, .. } = &menu[2] else {
            panic!("the third entry must be the nodes submenu");
        };
        assert_eq!(items[0].label, "当前订阅没有可用节点");
    }

    #[test]
    fn busy_folds_the_lists_and_disables_the_rows_but_not_quit() {
        let subscriptions = [(1, "机场".to_owned(), 2)];
        let nodes = [("a".to_owned(), "香港 01".to_owned())];
        let state = fixture(
            CoreBusy::Toggling,
            false,
            &subscriptions,
            Some(1),
            &nodes,
            "a",
        );
        let menu = build_menu_specs(Lang::Chinese, &state);
        let TrayMenuEntry::Submenu {
            disabled, items, ..
        } = &menu[1]
        else {
            panic!("the second entry must be the subscriptions submenu");
        };
        assert!(disabled);
        assert!(items.iter().all(|item| item.disabled));
        let TrayMenuEntry::Item(spec) = menu.last().unwrap() else {
            panic!("the last entry must be the quit row");
        };
        assert!(!spec.disabled);
    }

    /// The proxy row is the one thing that varies all three proxy flags, so
    /// this test builds its `TrayState`s by hand rather than through the
    /// fixture's ready-and-supported defaults.
    fn proxy_state<'a>(
        connected: bool,
        enabled: bool,
        ready: bool,
        supported: bool,
        subscriptions: &'a [(u64, String, usize)],
        nodes: &'a [(String, String)],
    ) -> TrayState<'a> {
        TrayState {
            profile_loaded: true,
            busy: CoreBusy::Idle,
            connected,
            subscriptions,
            active_subscription_id: Some(1),
            nodes,
            selected_tag: "a",
            system_proxy_enabled: enabled,
            system_proxy_ready: ready,
            system_proxy_supported: supported,
        }
    }

    /// The system proxy row's `(checked, disabled)` for a hand-built state.
    fn proxy_row(state: TrayState) -> (Option<bool>, bool) {
        match &build_menu_specs(Lang::Chinese, &state)[4] {
            TrayMenuEntry::Item(spec) => (spec.checked, spec.disabled),
            _ => panic!("the fifth entry must be the system proxy row"),
        }
    }

    #[test]
    fn the_system_proxy_row_mirrors_the_in_app_switch() {
        let subscriptions = [(1, "机场".to_owned(), 1)];
        let nodes = [("a".to_owned(), "香港 01".to_owned())];
        // Not connected, proxy off: nothing for the toggle to apply to.
        let (checked, disabled) = proxy_row(proxy_state(
            false,
            false,
            true,
            true,
            &subscriptions,
            &nodes,
        ));
        assert_eq!(checked, Some(false));
        assert!(disabled);
        // Connected turns it on, still unchecked.
        let (_, disabled) = proxy_row(proxy_state(true, false, true, true, &subscriptions, &nodes));
        assert!(!disabled);
        // Enabled keeps it reachable even once the core is down, so it can be
        // switched back off — the reason for the `(connected || enabled)`
        // half of the old expression.
        let (checked, disabled) =
            proxy_row(proxy_state(false, true, true, true, &subscriptions, &nodes));
        assert_eq!(checked, Some(true));
        assert!(!disabled);
        // A platform without the setting never offers it.
        let (_, disabled) = proxy_row(proxy_state(
            true,
            false,
            true,
            false,
            &subscriptions,
            &nodes,
        ));
        assert!(disabled);
    }

    #[test]
    fn english_reaches_the_rows_through_the_table() {
        let subscriptions = [(1, "provider".to_owned(), 1)];
        let nodes = [("a".to_owned(), "HK 01".to_owned())];
        let state = fixture(CoreBusy::Idle, false, &subscriptions, Some(1), &nodes, "a");
        let menu = build_menu_specs(Lang::English, &state);
        let TrayMenuEntry::Submenu { label, .. } = &menu[1] else {
            panic!("the second entry must be the subscriptions submenu");
        };
        assert_eq!(label, "Subscriptions · provider");
        let TrayMenuEntry::Item(spec) = menu.last().unwrap() else {
            panic!("the last entry must be the quit row");
        };
        assert_eq!(spec.label, "Quit");
    }
}
