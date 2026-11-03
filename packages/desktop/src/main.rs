#![cfg_attr(
    all(feature = "desktop", target_os = "windows"),
    windows_subsystem = "windows"
)]

#[cfg(feature = "desktop")]
use dioxus::desktop::{
    icon_from_memory,
    trayicon::{
        init_tray_icon,
        menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    },
    use_muda_event_handler, use_window, Config, DesktopContext, WindowBuilder,
    WindowCloseBehaviour,
};
#[cfg(all(feature = "desktop", target_os = "macos"))]
use dioxus::desktop::{
    muda::{
        accelerator::{Accelerator, Code, Modifiers},
        AboutMetadata,
    },
    tao::event::Event,
    use_wry_event_handler,
};
use dioxus::prelude::*;
#[cfg(all(feature = "desktop", target_os = "macos"))]
use std::cell::RefCell;
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
/// Menu-bar-only item. macOS hands `Cmd+Q` to `terminate:`, which would kill
/// the embedded core mid-flight, so this item takes the key instead.
#[cfg(feature = "desktop")]
const MENU_MINIMIZE_ID: &str = "kitty-minimize";
#[cfg(feature = "desktop")]
const TRAY_SUBSCRIPTION_PREFIX: &str = "kitty-subscription:";
#[cfg(feature = "desktop")]
const TRAY_NODE_PREFIX: &str = "kitty-node:";
#[cfg(feature = "desktop")]
const TRAY_SYSTEM_PROXY_ID: &str = "kitty-system-proxy";
#[cfg(feature = "desktop")]
const TRAY_COPY_ENV_ID: &str = "kitty-copy-environment";

#[cfg(feature = "desktop")]
#[derive(Clone, Debug, PartialEq, Eq)]
enum TrayCommand {
    Show,
    /// Put the window away without stopping the core. This is macOS `Cmd+Q`.
    Minimize,
    App(DesktopTrayCommand),
    Quit,
}

#[cfg(feature = "desktop")]
fn tray_command(id: &str) -> Option<TrayCommand> {
    match id {
        TRAY_SHOW_ID => Some(TrayCommand::Show),
        MENU_MINIMIZE_ID => Some(TrayCommand::Minimize),
        TRAY_QUIT_ID => Some(TrayCommand::Quit),
        TRAY_SYSTEM_PROXY_ID => Some(TrayCommand::App(DesktopTrayCommand::ToggleSystemProxy)),
        TRAY_COPY_ENV_ID => Some(TrayCommand::App(
            DesktopTrayCommand::CopyEnvironmentVariables,
        )),
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
    minimize_window: impl FnOnce(),
    dispatch_app_command: impl FnOnce(DesktopTrayCommand),
    shutdown_runtime: impl FnOnce(),
    exit_process: impl FnOnce(),
) {
    match command {
        TrayCommand::Show => show_window(),
        TrayCommand::Minimize => minimize_window(),
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

/// `Cmd+Q`. Closing the window only hides it (tray behaviour), so the key can
/// never mean "destroy the only window": a visible window goes to the Dock, a
/// window already in the Dock comes back, and a window hidden in the tray is
/// revealed - every press does something instead of nothing.
///
/// The proxy is untouched either way. Quitting stays on `Opt+Cmd+Q` and the
/// tray's 退出, both of which shut the core down first.
#[cfg(feature = "desktop")]
fn minimize_desktop_window(window: &DesktopContext) {
    if window.is_minimized() {
        window.set_minimized(false);
        window.set_focus();
        return;
    }
    if window.is_visible() {
        window.set_minimized(true);
        return;
    }
    show_desktop_window(window);
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

    let proxy_controls_separator = PredefinedMenuItem::separator();
    let system_proxy = CheckMenuItem::with_id(
        TRAY_SYSTEM_PROXY_ID,
        "系统代理",
        state.ready
            && state.system_proxy_ready
            && state.system_proxy_supported
            && !state.busy
            && (state.connected || state.system_proxy_enabled),
        state.system_proxy_enabled,
        None,
    );
    let copy_environment = MenuItem::with_id(
        TRAY_COPY_ENV_ID,
        "复制环境变量",
        state.ready && !state.busy,
        None,
    );
    let window_actions_separator = PredefinedMenuItem::separator();
    let show = MenuItem::with_id(TRAY_SHOW_ID, "显示 Kitty Pro", true, None);
    let quit = MenuItem::with_id(TRAY_QUIT_ID, "退出", true, None);
    menu.append_items(&[
        &status,
        &subscriptions_menu,
        &nodes_menu,
        &proxy_controls_separator,
        &system_proxy,
        &copy_environment,
        &window_actions_separator,
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

/// Dioxus injects a blanket `contextmenu` suppressor into every release build so
/// its dev-tool menu cannot be opened. That also takes away the webview's own
/// editing menu, so right-click in an input offered no Cut/Copy/Paste and
/// right-click on a selection in the log view offered no Copy at all.
///
/// Stopping propagation for editable targets and live selections hands those
/// right-clicks back to the webview. Capture phase is what makes this work and
/// it is not optional: the suppressor listens on `document` in the bubble phase,
/// so a capture listener on the same node runs first and can cut the event off
/// before `preventDefault()` is ever reached. Registering in the bubble phase
/// would lose, since wry runs its own init script at document start - before any
/// page script, this one included.
///
/// Everywhere else the event still propagates, so the rest of the page keeps the
/// dev-tool-free behaviour the suppressor exists for. Needed on every native
/// platform, not only macOS.
#[cfg(feature = "desktop")]
const NATIVE_EDITING_MENU_JS: &str = r#"
(function () {
    var editable = 'input, textarea, [contenteditable=""], [contenteditable="true"]';
    document.addEventListener('contextmenu', function (event) {
        var target = event.target;
        var selected = window.getSelection && String(window.getSelection());
        if (selected || (target && target.closest && target.closest(editable))) {
            event.stopPropagation();
        }
    }, true);
})();
"#;

/// Head extra for native shells. `custom_head` is the hook Dioxus gives for
/// script that must be present in the very first document.
#[cfg(feature = "desktop")]
fn extra_head_html() -> String {
    format!("<script>{NATIVE_EDITING_MENU_JS}</script>")
}

/// The quit-related key bindings, as data: the menu bar installs these, and a
/// unit test pins them down without building muda items off the main thread.
#[cfg(all(feature = "desktop", target_os = "macos"))]
fn quit_key_bindings() -> Vec<(&'static str, Accelerator)> {
    vec![
        // `Cmd+Q` puts the window away; the core keeps running.
        (
            MENU_MINIMIZE_ID,
            Accelerator::new(Some(Modifiers::SUPER), Code::KeyQ),
        ),
        // The binding that really stops everything, and it goes through the same
        // cleanup as the tray's "退出".
        (
            TRAY_QUIT_ID,
            Accelerator::new(Some(Modifiers::SUPER | Modifiers::ALT), Code::KeyQ),
        ),
    ]
}

/// Edit-menu labels in item order. muda's own defaults are English (`Cu&t`,
/// `&Paste`, ...), and an explicit label does not disturb the native selector or
/// key equivalent it installs, so the clipboard keys keep working. Keeping the
/// list in one place lets a test pin the order the menu is built in.
#[cfg(all(feature = "desktop", target_os = "macos"))]
fn edit_menu_labels() -> [&'static str; 6] {
    ["撤销", "重做", "剪切", "拷贝", "粘贴", "全选"]
}

/// The macOS menu bar. It is not decoration: WKWebView only performs
/// copy/paste/select-all when a menu item owns the key equivalent and routes it
/// to the first responder, so without a bar the installed app had no working
/// `Cmd+C`/`Cmd+V` at all, and `Cmd+Q` went nowhere because nothing bound it to
/// `terminate:`. The Edit menu is what brings the clipboard back.
///
/// The application menu claims `Cmd+Q` for [`TrayCommand::Minimize`] - the
/// window only ever hides to the tray, and letting `terminate:` fire would kill
/// a running core mid-flight - so real quit moves to `Opt+Cmd+Q` and shares the
/// tray's quit id, keeping one cleanup path.
#[cfg(all(feature = "desktop", target_os = "macos"))]
fn app_menu_bar() -> AppMenuBar {
    let bindings = quit_key_bindings();
    let (minimize_id, minimize_accel) = bindings
        .first()
        .expect("the minimize key binding must be present");
    let (quit_id, quit_accel) = bindings
        .get(1)
        .expect("the quit key binding must be present");
    let [undo, redo, cut, copy, paste, select_all] = edit_menu_labels();

    let app_menu = Submenu::new("Kitty Pro", true);
    let about = PredefinedMenuItem::about(
        Some("关于 Kitty Pro"),
        Some(AboutMetadata {
            name: Some("Kitty Pro".to_string()),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            ..Default::default()
        }),
    );
    let minimize = MenuItem::with_id(*minimize_id, "最小化", true, Some(*minimize_accel));
    // The quit id is the tray item's id, so quitting from either place runs the
    // same cleanup before the process exits.
    let quit = MenuItem::with_id(*quit_id, "退出 Kitty Pro", true, Some(*quit_accel));
    app_menu
        .append_items(&[
            &about,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some("隐藏 Kitty Pro")),
            &PredefinedMenuItem::hide_others(Some("隐藏其他")),
            &PredefinedMenuItem::show_all(Some("全部显示")),
            &PredefinedMenuItem::separator(),
            &minimize,
            &quit,
        ])
        .expect("application menu must be valid");

    let edit_menu = Submenu::new("编辑", true);
    edit_menu
        .append_items(&[
            &PredefinedMenuItem::undo(Some(undo)),
            &PredefinedMenuItem::redo(Some(redo)),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(Some(cut)),
            &PredefinedMenuItem::copy(Some(copy)),
            &PredefinedMenuItem::paste(Some(paste)),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::select_all(Some(select_all)),
        ])
        .expect("edit menu must be valid");

    let window_menu = Submenu::new("窗口", true);
    let show = MenuItem::with_id(TRAY_SHOW_ID, "显示 Kitty Pro", true, None);
    window_menu
        .append_items(&[
            &PredefinedMenuItem::minimize(Some("最小化")),
            &PredefinedMenuItem::maximize(Some("缩放")),
            &PredefinedMenuItem::fullscreen(Some("进入全屏幕")),
            &PredefinedMenuItem::separator(),
            &show,
            &PredefinedMenuItem::close_window(Some("关闭")),
        ])
        .expect("window menu must be valid");

    let menu = Menu::new();
    menu.append_items(&[&app_menu, &edit_menu, &window_menu])
        .expect("menu bar must be valid");

    AppMenuBar { menu, window_menu }
}

/// The menu bar plus the part macOS can only adopt after it is installed.
#[cfg(all(feature = "desktop", target_os = "macos"))]
struct AppMenuBar {
    menu: Menu,
    window_menu: Submenu,
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
thread_local! {
    static WINDOW_MENU: RefCell<Option<Submenu>> = const { RefCell::new(None) };
}

/// Hand the menu bar to Dioxus. macOS turns a submenu into the real windows
/// menu only once `NSApp.mainMenu` exists, which Dioxus does while building the
/// window - after this `Config` is assembled - so the submenu is parked here and
/// adopted by [`adopt_window_menu`] once the first frame is up.
#[cfg(all(feature = "desktop", target_os = "macos"))]
fn with_platform_menu(config: Config) -> Config {
    let bar = app_menu_bar();
    WINDOW_MENU.with(|slot| *slot.borrow_mut() = Some(bar.window_menu.clone()));
    config.with_menu(bar.menu)
}

/// Windows and Linux keep the previous behaviour: WebView2 and WebKitGTK handle
/// the clipboard keys in the page themselves, and both already show their own
/// accelerator labels, so no bar is needed.
#[cfg(all(feature = "desktop", not(target_os = "macos")))]
fn with_platform_menu(config: Config) -> Config {
    config.with_menu(None)
}

/// Adopt the windows menu. Runs on the main thread that muda requires, and
/// after Dioxus installed the menu bar.
#[cfg(all(feature = "desktop", target_os = "macos"))]
fn adopt_window_menu() {
    WINDOW_MENU.with(|slot| {
        if let Some(window_menu) = slot.borrow().as_ref() {
            window_menu.set_as_windows_menu_for_nsapp();
        }
    });
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
    // Clear a system proxy that a previous Kitty Pro run left pointing at a
    // dead core after it crashed or was killed before cleanup.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    api::cleanup_stale_system_proxy_sentinel();
    let config = Config::new()
        .with_window(WindowBuilder::new().with_title("Kitty Pro"))
        .with_icon(window_icon)
        .with_custom_head(format!(
            "<style>{MAIN_CSS}\n{APP_CSS}</style>{}",
            extra_head_html()
        ))
        .with_close_behaviour(WindowCloseBehaviour::WindowHides)
        .with_custom_event_handler(|event, _| {
            if matches!(event, dioxus::desktop::tao::event::Event::LoopDestroyed) {
                let _ = api::shutdown_native_runtime();
            }
        });
    // `with_menu` is a no-op unless the window still has decorations, so it has
    // to run after `with_window`.
    let config = with_platform_menu(config);

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
    #[cfg(all(feature = "desktop", target_os = "macos"))]
    use_effect(adopt_window_menu);

    rsx! {
        document::Style { "{MAIN_CSS}" }
        ProxyApp {
            platform: "Desktop".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            desktop_tray,
        }
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
    // Dioxus registers the shared muda handler before its tray-specific handler,
    // so tray menu clicks are delivered as regular muda events on Dioxus 0.7. The
    // macOS menu bar shares these ids, so `Cmd+Q` and the menu items land here too.
    let _tray_menu_handler = use_muda_event_handler(move |event| {
        let Some(command) = tray_command(event.id().as_ref()) else {
            return;
        };

        execute_tray_command(
            command,
            || show_desktop_window(&window),
            || minimize_desktop_window(&window),
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
        assert_eq!(tray_command(MENU_MINIMIZE_ID), Some(TrayCommand::Minimize));
        assert_eq!(
            tray_command(TRAY_SYSTEM_PROXY_ID),
            Some(TrayCommand::App(DesktopTrayCommand::ToggleSystemProxy))
        );
        assert_eq!(
            tray_command(TRAY_COPY_ENV_ID),
            Some(TrayCommand::App(
                DesktopTrayCommand::CopyEnvironmentVariables
            ))
        );
        assert_eq!(tray_command("unknown"), None);
    }

    #[test]
    fn quit_command_cleans_up_before_exiting() {
        let sequence = Cell::new(0);

        execute_tray_command(
            TrayCommand::Quit,
            || panic!("quit must not show the window"),
            || panic!("quit must not minimize the window"),
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
            || panic!("show must not minimize the window"),
            |_| panic!("show must not dispatch an app command"),
            || panic!("show must not shut down the runtime"),
            || panic!("show must not exit the process"),
        );

        assert!(shown.get());
    }

    /// `Cmd+Q` is the key everyone presses first, so it must never be the key
    /// that stops a running core: it puts the window away and leaves the proxy
    /// alive, which is what the tray-based close behaviour already promises.
    #[test]
    fn minimize_command_keeps_the_runtime_alive() {
        let minimized = Cell::new(false);

        execute_tray_command(
            TrayCommand::Minimize,
            || panic!("minimize must not show the window"),
            || minimized.set(true),
            |_| panic!("minimize must not dispatch an app command"),
            || panic!("minimize must not shut down the runtime"),
            || panic!("minimize must not exit the process"),
        );

        assert!(minimized.get());
    }

    /// The command reaches `use_muda_event_handler` through an id, so a menu bar
    /// that names the item differently from the dispatcher would silently do
    /// nothing on `Cmd+Q`.
    #[test]
    #[cfg(target_os = "macos")]
    fn menu_bar_binds_quit_keys_to_the_shared_ids() {
        let bindings = quit_key_bindings();
        let ids: Vec<&str> = bindings.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![MENU_MINIMIZE_ID, TRAY_QUIT_ID]);
        // Every bound id must map back to a command, or the key would be a no-op.
        for (id, _) in &bindings {
            assert!(tray_command(id).is_some(), "{id} has no command");
        }
        // `Cmd+Q` may never reach the item that stops the core.
        let (minimize_id, minimize) = &bindings[0];
        assert_eq!(tray_command(minimize_id), Some(TrayCommand::Minimize));
        assert_eq!(minimize.modifiers(), Modifiers::SUPER);
        assert_eq!(minimize.key(), Code::KeyQ);
        let (quit_id, quit) = &bindings[1];
        assert_eq!(tray_command(quit_id), Some(TrayCommand::Quit));
        assert_eq!(quit.modifiers(), Modifiers::SUPER | Modifiers::ALT);
        assert_eq!(quit.key(), Code::KeyQ);
    }

    /// WebKit needs a menu item per clipboard key; a plain `Edit` submenu of
    /// predefined items is the only thing that makes `Cmd+C`/`Cmd+V` work in a
    /// release build, so the item list is pinned here.
    #[test]
    #[cfg(target_os = "macos")]
    fn edit_menu_covers_every_clipboard_key() {
        let items = edit_menu_labels();
        for expected in ["撤销", "重做", "剪切", "拷贝", "粘贴", "全选"] {
            assert!(items.contains(&expected), "{expected} is missing");
        }
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
    #[cfg_attr(
        target_os = "macos",
        ignore = "muda menus must be created on the macOS main thread"
    )]
    fn tray_menu_marks_the_active_subscription_and_node() {
        let state = DesktopTrayState {
            ready: true,
            busy: false,
            connected: true,
            system_proxy_ready: true,
            system_proxy_supported: true,
            system_proxy_enabled: true,
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
        let system_proxy = root_items[4]
            .as_check_menuitem()
            .expect("system proxy should be a check item");
        assert!(system_proxy.is_enabled());
        assert!(system_proxy.is_checked());
        assert_eq!(
            root_items[5]
                .as_menuitem()
                .expect("environment copy should be selectable")
                .text(),
            "复制环境变量"
        );
    }
}
