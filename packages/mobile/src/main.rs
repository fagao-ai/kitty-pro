use dioxus::prelude::*;
use ui::ProxyApp;
#[cfg(feature = "mobile")]
use ui::APP_CSS;

const MAIN_CSS: &str = include_str!("../assets/main.css");

#[cfg(feature = "mobile")]
fn main() {
    let initial_head = format!("<style>{MAIN_CSS}\n{APP_CSS}</style>");
    let config = dioxus::mobile::Config::new().with_custom_head(initial_head);
    dioxus::LaunchBuilder::mobile().with_cfg(config).launch(App);
}

#[cfg(not(feature = "mobile"))]
fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Style { "{MAIN_CSS}" }
        ProxyApp { platform: "Mobile".to_string() }
    }
}
