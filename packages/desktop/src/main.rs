use dioxus::prelude::*;
use ui::ProxyApp;

const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        ProxyApp { platform: "Desktop".to_string() }
    }
}
