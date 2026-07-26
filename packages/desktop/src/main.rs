#![cfg_attr(
    all(feature = "desktop", target_os = "windows"),
    windows_subsystem = "windows"
)]

#[cfg(feature = "desktop")]
use dioxus::desktop::{
    icon_from_memory,
    trayicon::{
        init_tray_icon,
        menu::{Menu, MenuItem, PredefinedMenuItem},
    },
    use_tray_menu_event_handler, use_window, Config, WindowBuilder, WindowCloseBehaviour,
};
use dioxus::prelude::*;
use ui::ProxyApp;

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
fn main() {
    let window_icon = icon_from_memory(APP_ICON).expect("embedded application icon must be valid");
    let config = Config::new()
        .with_window(WindowBuilder::new().with_title("Kitty Pro"))
        .with_icon(window_icon)
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
    use_desktop_tray();

    rsx! {
        document::Style { "{MAIN_CSS}" }
        ProxyApp { platform: "Desktop".to_string() }
    }
}

#[cfg(feature = "desktop")]
fn use_desktop_tray() {
    let window = use_window();
    let _tray = use_hook(|| {
        let menu = Menu::new();
        let show = MenuItem::with_id(TRAY_SHOW_ID, "显示 Kitty Pro", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::with_id(TRAY_QUIT_ID, "退出", true, None);
        menu.append_items(&[&show, &separator, &quit])
            .expect("tray menu must be valid");

        let icon = icon_from_memory(TRAY_ICON).expect("embedded tray icon must be valid");
        let tray = init_tray_icon(menu, Some(icon));
        let _ = tray.set_tooltip(Some("Kitty Pro"));
        #[cfg(target_os = "macos")]
        tray.set_icon_as_template(true);
        tray
    });

    let _tray_menu_handler = use_tray_menu_event_handler(move |event| match event.id.as_ref() {
        TRAY_SHOW_ID => {
            window.set_visible(true);
            window.set_focus();
        }
        TRAY_QUIT_ID => {
            let _ = api::shutdown_native_runtime();
            window.set_close_behavior(WindowCloseBehaviour::WindowCloses);
            window.close();
        }
        _ => {}
    });
}
