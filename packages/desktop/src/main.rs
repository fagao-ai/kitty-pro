#![cfg_attr(
    all(feature = "desktop", target_os = "windows"),
    windows_subsystem = "windows"
)]

#[cfg(feature = "desktop")]
use dioxus::desktop::{
    icon_from_memory,
    trayicon::{
        init_tray_icon,
        menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    },
    use_tray_menu_event_handler, use_window, Config, DesktopContext, WindowBuilder,
    WindowCloseBehaviour,
};
#[cfg(all(feature = "desktop", target_os = "macos"))]
use dioxus::desktop::{tao::event::Event, use_wry_event_handler};
use dioxus::prelude::*;
use ui::ProxyApp;
#[cfg(feature = "desktop")]
use ui::{DesktopTrayBridge, DesktopTrayCommand, DesktopTrayState, APP_CSS};

const MAIN_CSS: &str = include_str!("../assets/main.css");
#[cfg(feature = "desktop")]
const APP_ICON: &[u8] = include_bytes!("../../../assets/icon/kitty-pro-256.png");
#[cfg(all(feature = "desktop", target_os = "macos"))]
const TRAY_ICON: &[u8] = include_bytes!("../../../assets/icon/kitty-pro-tray.png");
#[cfg(all(feature = "desktop", not(target_os = "macos")))]
const TRAY_ICON: &[u8] = include_bytes!("../../../assets/icon/kitty-pro-64.png");
#[cfg(feature = "desktop")]
const TRAY_SHOW_ID: &str = "kitty-show";
#[cfg(feature = "desktop")]
const TRAY_QUIT_ID: &str = "kitty-quit";
#[cfg(feature = "desktop")]
const TRAY_SUBSCRIPTION_PREFIX: &str = "kitty-subscription:";
#[cfg(feature = "desktop")]
const TRAY_NODE_PREFIX: &str = "kitty-node:";

#[cfg(feature = "desktop")]
#[derive(Clone, Debug, PartialEq, Eq)]
enum TrayCommand {
    Show,
    App(DesktopTrayCommand),
    Quit,
}

#[cfg(feature = "desktop")]
fn tray_command(id: &str) -> Option<TrayCommand> {
    match id {
        TRAY_SHOW_ID => Some(TrayCommand::Show),
        TRAY_QUIT_ID => Some(TrayCommand::Quit),
        _ => id
            .strip_prefix(TRAY_SUBSCRIPTION_PREFIX)
            .and_then(|id| id.parse::<u64>().ok())
            .map(DesktopTrayCommand::SelectSubscription)
            .or_else(|| {
                id.strip_prefix(TRAY_NODE_PREFIX)
                    .filter(|tag| !tag.is_empty())
                    .map(|tag| DesktopTrayCommand::SelectNode(tag.to_string()))
            })
            .map(TrayCommand::App),
    }
}

#[cfg(feature = "desktop")]
fn execute_tray_command(
    command: TrayCommand,
    show_window: impl FnOnce(),
    dispatch_app_command: impl FnOnce(DesktopTrayCommand),
    shutdown_runtime: impl FnOnce(),
    exit_process: impl FnOnce(),
) {
    match command {
        TrayCommand::Show => show_window(),
        TrayCommand::App(command) => dispatch_app_command(command),
        TrayCommand::Quit => {
            shutdown_runtime();
            exit_process();
        }
    }
}

#[cfg(feature = "desktop")]
fn show_desktop_window(window: &DesktopContext) {
    window.set_visible(true);
    window.set_focus();
}

#[cfg(feature = "desktop")]
fn tray_menu_text(value: &str, max_chars: usize) -> String {
    let mut text = value.replace('&', "&&");
    if text.chars().count() <= max_chars {
        return text;
    }
    text = text.chars().take(max_chars.saturating_sub(3)).collect();
    text.push_str("...");
    text
}

#[cfg(feature = "desktop")]
fn active_tray_nodes(state: &DesktopTrayState) -> &[ui::DesktopTrayNode] {
    state
        .active_subscription_id
        .and_then(|id| state.subscriptions.iter().find(|item| item.id == id))
        .map(|subscription| subscription.nodes.as_slice())
        .unwrap_or_default()
}

#[cfg(feature = "desktop")]
fn build_tray_menu(state: &DesktopTrayState) -> Menu {
    let menu = Menu::new();
    let status = if !state.ready {
        "状态：正在加载配置".to_string()
    } else if state.busy {
        "状态：正在应用更改".to_string()
    } else if state.connected {
        "状态：已连接".to_string()
    } else {
        "状态：未连接".to_string()
    };
    let status = MenuItem::new(status, false, None);

    let active_subscription = state
        .active_subscription_id
        .and_then(|id| state.subscriptions.iter().find(|item| item.id == id))
        .map(|item| tray_menu_text(&item.name, 28))
        .unwrap_or_else(|| "未选择".to_string());
    let subscriptions_menu = Submenu::new(
        format!("订阅 · {active_subscription}"),
        state.ready && !state.busy && !state.subscriptions.is_empty(),
    );
    if state.subscriptions.is_empty() {
        let empty = MenuItem::new("暂无订阅", false, None);
        subscriptions_menu
            .append(&empty)
            .expect("empty subscription menu item must be valid");
    } else {
        for subscription in &state.subscriptions {
            let selected = state.active_subscription_id == Some(subscription.id);
            let label = format!(
                "{} {} ({} 个节点)",
                if selected { "●" } else { "○" },
                tray_menu_text(&subscription.name, 42),
                subscription.nodes.len()
            );
            let item = MenuItem::with_id(
                format!("{TRAY_SUBSCRIPTION_PREFIX}{}", subscription.id),
                label,
                !state.busy,
                None,
            );
            subscriptions_menu
                .append(&item)
                .expect("subscription menu item must be valid");
        }
    }

    let active_nodes = active_tray_nodes(state);
    let active_node = active_nodes
        .iter()
        .find(|item| item.tag == state.selected_tag)
        .map(|item| tray_menu_text(&item.name, 28))
        .unwrap_or_else(|| "未选择".to_string());
    let nodes_menu = Submenu::new(
        format!("节点 · {active_node}"),
        state.ready && !state.busy && !active_nodes.is_empty(),
    );
    if active_nodes.is_empty() {
        let empty = MenuItem::new("当前订阅没有可用节点", false, None);
        nodes_menu
            .append(&empty)
            .expect("empty node menu item must be valid");
    } else {
        for node in active_nodes {
            let selected = node.tag == state.selected_tag;
            let item = MenuItem::with_id(
                format!("{TRAY_NODE_PREFIX}{}", node.tag),
                format!(
                    "{} {}",
                    if selected { "●" } else { "○" },
                    tray_menu_text(&node.name, 48)
                ),
                !state.busy,
                None,
            );
            nodes_menu
                .append(&item)
                .expect("node menu item must be valid");
        }
    }

    let show = MenuItem::with_id(TRAY_SHOW_ID, "显示 Kitty Pro", true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id(TRAY_QUIT_ID, "退出", true, None);
    menu.append_items(&[
        &status,
        &subscriptions_menu,
        &nodes_menu,
        &separator,
        &show,
        &quit,
    ])
    .expect("tray menu must be valid");
    menu
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
fn should_show_window_on_reopen<T: 'static>(event: &Event<'_, T>) -> bool {
    matches!(event, Event::Reopen { .. })
}

#[cfg(feature = "desktop")]
fn main() {
    #[cfg(target_os = "macos")]
    if let Some(exit_code) = singbox::macos::run_helper_from_args() {
        // Only the separately authorized helper process reaches this branch;
        // the original GUI process continues with the user's privileges.
        std::process::exit(exit_code);
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    if let Some(exit_code) = singbox::desktop_helper::run_helper_from_args() {
        // The elevated helper must exit before Dioxus initializes a second
        // desktop runtime in the child process.
        std::process::exit(exit_code);
    }

    let window_icon = icon_from_memory(APP_ICON).expect("embedded application icon must be valid");
    let config = Config::new()
        .with_window(WindowBuilder::new().with_title("Kitty Pro"))
        .with_menu(None)
        .with_icon(window_icon)
        .with_custom_head(format!("<style>{MAIN_CSS}\n{APP_CSS}</style>"))
        .with_close_behaviour(WindowCloseBehaviour::WindowHides)
        .with_custom_event_handler(|event, _| {
            if matches!(event, dioxus::desktop::tao::event::Event::LoopDestroyed) {
                let _ = api::shutdown_native_runtime();
            }
        });

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}

#[cfg(not(feature = "desktop"))]
fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    #[cfg(feature = "desktop")]
    let desktop_tray = {
        let bridge = DesktopTrayBridge {
            state: use_signal(DesktopTrayState::default),
            command: use_signal(|| None),
        };
        use_desktop_tray(bridge);
        Some(bridge)
    };
    #[cfg(not(feature = "desktop"))]
    let desktop_tray = None;

    rsx! {
        document::Style { "{MAIN_CSS}" }
        ProxyApp { platform: "Desktop".to_string(), desktop_tray }
    }
}

#[cfg(feature = "desktop")]
fn use_desktop_tray(bridge: DesktopTrayBridge) {
    let window = use_window();
    let _tray = use_hook(|| {
        let menu = build_tray_menu(&DesktopTrayState::default());
        let icon = icon_from_memory(TRAY_ICON).expect("embedded tray icon must be valid");
        let tray = init_tray_icon(menu, Some(icon));
        let _ = tray.set_tooltip(Some("Kitty Pro"));
        #[cfg(target_os = "macos")]
        tray.set_icon_as_template(true);
        tray
    });

    let tray_state = bridge.state;
    use_effect(move || {
        let tray_state = tray_state();
        _tray.set_menu(Some(Box::new(build_tray_menu(&tray_state))));
        let tooltip = if tray_state.connected {
            active_tray_nodes(&tray_state)
                .iter()
                .find(|node| node.tag == tray_state.selected_tag)
                .map(|node| format!("Kitty Pro - {}", node.name))
                .unwrap_or_else(|| "Kitty Pro - 已连接".to_string())
        } else {
            "Kitty Pro - 未连接".to_string()
        };
        let _ = _tray.set_tooltip(Some(tooltip));
    });

    #[cfg(target_os = "macos")]
    let _reopen_event_handler = {
        let window = window.clone();
        use_wry_event_handler(move |event, _| {
            if should_show_window_on_reopen(event) {
                show_desktop_window(&window);
            }
        })
    };

    let mut app_command = bridge.command;
    let _tray_menu_handler = use_tray_menu_event_handler(move |event| {
        let Some(command) = tray_command(event.id.as_ref()) else {
            return;
        };

        execute_tray_command(
            command,
            || show_desktop_window(&window),
            |command| app_command.set(Some(command)),
            || {
                if let Err(error) = api::shutdown_native_runtime() {
                    eprintln!("failed to clean up the native runtime before exit: {error}");
                }
            },
            || std::process::exit(0),
        );
    });
}

#[cfg(all(test, feature = "desktop"))]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn tray_ids_map_to_expected_commands() {
        assert_eq!(tray_command(TRAY_SHOW_ID), Some(TrayCommand::Show));
        assert_eq!(tray_command(TRAY_QUIT_ID), Some(TrayCommand::Quit));
        assert_eq!(tray_command("unknown"), None);
    }

    #[test]
    fn quit_command_cleans_up_before_exiting() {
        let sequence = Cell::new(0);

        execute_tray_command(
            TrayCommand::Quit,
            || panic!("quit must not show the window"),
            |_| panic!("quit must not dispatch an app command"),
            || {
                assert_eq!(sequence.get(), 0);
                sequence.set(1);
            },
            || {
                assert_eq!(sequence.get(), 1);
                sequence.set(2);
            },
        );

        assert_eq!(sequence.get(), 2);
    }

    #[test]
    fn show_command_does_not_shutdown_or_exit() {
        let shown = Cell::new(false);

        execute_tray_command(
            TrayCommand::Show,
            || shown.set(true),
            |_| panic!("show must not dispatch an app command"),
            || panic!("show must not shut down the runtime"),
            || panic!("show must not exit the process"),
        );

        assert!(shown.get());
    }

    #[test]
    fn selection_ids_map_to_app_commands() {
        assert_eq!(
            tray_command("kitty-subscription:42"),
            Some(TrayCommand::App(DesktopTrayCommand::SelectSubscription(42)))
        );
        assert_eq!(
            tray_command("kitty-node:subscription-1:node-a"),
            Some(TrayCommand::App(DesktopTrayCommand::SelectNode(
                "subscription-1:node-a".to_string()
            )))
        );
        assert_eq!(tray_command("kitty-subscription:not-a-number"), None);
        assert_eq!(tray_command("kitty-node:"), None);
    }

    #[test]
    fn tray_menu_text_escapes_mnemonics_and_truncates_on_characters() {
        assert_eq!(tray_menu_text("A&B", 10), "A&&B");
        assert_eq!(tray_menu_text("香港节点一号", 5), "香港...");
    }

    #[test]
    fn tray_menu_marks_the_active_subscription_and_node() {
        let state = DesktopTrayState {
            ready: true,
            busy: false,
            connected: true,
            active_subscription_id: Some(2),
            selected_tag: "subscription-2:node-b".to_string(),
            subscriptions: vec![
                ui::DesktopTraySubscription {
                    id: 1,
                    name: "订阅一".to_string(),
                    nodes: vec![ui::DesktopTrayNode {
                        tag: "subscription-1:node-c".to_string(),
                        name: "节点 C".to_string(),
                    }],
                },
                ui::DesktopTraySubscription {
                    id: 2,
                    name: "订阅二".to_string(),
                    nodes: vec![
                        ui::DesktopTrayNode {
                            tag: "subscription-2:node-a".to_string(),
                            name: "节点 A".to_string(),
                        },
                        ui::DesktopTrayNode {
                            tag: "subscription-2:node-b".to_string(),
                            name: "节点 B".to_string(),
                        },
                    ],
                },
            ],
        };

        let menu = build_tray_menu(&state);
        let root_items = menu.items();
        let subscription_items = root_items[1]
            .as_submenu()
            .expect("second item should be the subscription submenu")
            .items();
        let node_items = root_items[2]
            .as_submenu()
            .expect("third item should be the node submenu")
            .items();

        assert!(subscription_items[0]
            .as_menuitem()
            .expect("subscription should be selectable")
            .text()
            .starts_with('○'));
        assert!(subscription_items[1]
            .as_menuitem()
            .expect("subscription should be selectable")
            .text()
            .starts_with('●'));
        assert!(node_items[0]
            .as_menuitem()
            .expect("node should be selectable")
            .text()
            .starts_with('○'));
        assert!(node_items[1]
            .as_menuitem()
            .expect("node should be selectable")
            .text()
            .starts_with('●'));
    }
}
