//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

pub(crate) const BRAND_ICON_SVG: &str = include_str!("../../assets/kitty-pro.svg");

/// Restart the embedded core. The design promotes this action from a bare
/// sidebar icon to a labelled topbar button, so phone and desktop share it.
#[component]
pub(crate) fn RestartCoreButton(
    status: CoreStatusSignals,
    settings: RuntimeSettings,
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    subscriptions: Signal<Vec<Subscription>>,
    active_subscription_id: Signal<Option<u64>>,
    group_selections: Signal<HashMap<String, String>>,
    core_busy: Signal<bool>,
    runtime_action_busy: Signal<bool>,
    system_proxy_busy: Signal<bool>,
) -> Element {
    let toast = use_context::<ToastManager>();
    let restarting = status.core_restarting;
    let is_restarting = restarting();
    let is_connected = (status.connected)();
    let busy = core_busy() || runtime_action_busy() || system_proxy_busy();
    let disabled = !is_connected || busy || is_restarting;
    rsx! {
        button {
            class: "secondary-button restart-core-button",
            title: "重启内核",
            aria_label: "重启内核",
            aria_busy: is_restarting,
            disabled,
            onclick: move |_| {
                let status = status;
                let settings = settings;
                let toast = toast;
                let request = build_connection_request(
                    nodes(),
                    selected_tag(),
                    active_subscription_proxy_server_nameservers(
                        &subscriptions(),
                        active_subscription_id(),
                    ),
                    settings,
                    group_selections(),
                );
                async move {
                    if core_busy() || runtime_action_busy() || system_proxy_busy() {
                        return;
                    }
                    restart_core_from_ui(request, status, toast, "sing-box 内核已重启").await;
                }
            },
            if is_restarting {
                span { class: "spinner" }
            } else {
                Icon { icon: LdRefreshCw, width: 15, height: 15 }
            }
            span { "重启内核" }
        }
    }
}

/// JS source shared by enter/teardown. A stack is required because dialogs
/// nest here (the process picker opens on top of the rule editor): only the
/// topmost panel may own the Tab ring, and each layer restores focus to the
/// layer beneath it rather than to the page.
pub(crate) const DIALOG_STACK_JS: &str = r#"
const state = (window.__kittyDialogs ||= { stack: [] });
if (!state.trap) {
    const focusable = (root) => Array.from(
        root.querySelectorAll('a[href],button:not([disabled]),textarea:not([disabled]),input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])')
    ).filter((el) => el.offsetWidth || el.offsetHeight || el.getClientRects().length);
    state.focusable = focusable;
    // Capture phase: a listener on the page cannot break the ring by calling
    // stopPropagation first, and the ring still sees keys while focus sits on
    // the panel itself.
    state.trap = (event) => {
        const top = state.stack[state.stack.length - 1];
        if (!top || event.key !== 'Tab') return;
        const items = focusable(top.panel);
        if (items.length === 0) {
            event.preventDefault();
            top.panel.focus({ preventScroll: true });
            return;
        }
        const first = items[0];
        const last = items[items.length - 1];
        const active = document.activeElement;
        if (!top.panel.contains(active)) { event.preventDefault(); first.focus(); return; }
        if (event.shiftKey && active === first) {
            event.preventDefault();
            last.focus();
        } else if (!event.shiftKey && active === last) {
            event.preventDefault();
            first.focus();
        }
    };
    document.addEventListener('keydown', state.trap, true);
}
"#;

/// Script that arms one dialog instance: remembers what had focus, moves focus
/// into the panel, and pushes it onto the Tab-ring stack.
pub(crate) fn dialog_enter_script(panel_id: &str) -> String {
    format!(
        r#"
{DIALOG_STACK_JS}
const panel = document.getElementById({panel_id:?});
if (panel) {{
    state.stack.push({{ panel, returnFocus: document.activeElement }});
    const target = panel.querySelector('[data-autofocus]') || state.focusable(panel)[0];
    if (target) {{
        target.focus({{ preventScroll: true }});
    }} else {{
        panel.tabIndex = -1;
        panel.focus({{ preventScroll: true }});
    }}
}}
"#
    )
}

/// Teardown of `dialog_enter_script`: remove *this* layer (identified by the
/// panel still referenced by the stack entry) and hand focus back to whatever
/// was focused before it opened, so the keyboard never lands on
/// `document.body` after Escape. Popping by identity rather than unconditionally
/// matters when two dialogs close in a quick sequence: an async `eval` reorder
/// would otherwise pop the wrong entry and leave the closed panel on the stack.
pub(crate) fn dialog_leave_script(panel_id: &str) -> String {
    format!(
        r#"
const state = window.__kittyDialogs;
if (state) {{
    const panel = document.getElementById({panel_id:?});
    let index = state.stack.length - 1;
    while (index >= 0 && state.stack[index].panel !== panel) {{ index -= 1; }}
    // A panel that re-rendered away (id gone from the DOM) still matches its
    // stack entry by insertion order, so fall back to the top when the node
    // detached before this teardown ran.
    if (index < 0 && state.stack.length > 0 && !panel) index = state.stack.length - 1;
    if (index >= 0) {{
        const closed = state.stack.splice(index, 1)[0];
        if (state.stack.length === 0) {{
            document.removeEventListener('keydown', state.trap, true);
            state.trap = null;
        }}
        // Each layer records what had focus when it opened, which for a nested
        // dialog is the trigger inside the outer one. A trigger that no longer
        // exists is skipped by the contains() guard rather than throwing.
        if (closed.returnFocus && document.contains(closed.returnFocus)) {{
            closed.returnFocus.focus({{ preventScroll: true }});
        }}
    }}
}}
"#
    )
}

#[component]
/// Backdrop + panel used by every dialog in the app.
///
/// Owning the shell is what makes the keyboard behaviour uniform: Escape
/// dismisses (when the dialog is idle), Tab cannot escape the panel, and focus
/// returns to the trigger on close. It also keeps the panel a genuine viewport
/// overlay - the rule card's `backdrop-filter` makes it a containing block for
/// `position: fixed` descendants, which used to fling the process picker off
/// screen once the rule list was long enough to scroll.
pub(crate) fn DialogShell(
    /// DOM id for the panel; the focus scripts find the panel through it.
    panel_id: String,
    /// Panel classes, so each dialog keeps its own width and padding.
    class: String,
    /// Accessible name of the dialog.
    label: String,
    /// False while a submit is in flight: Escape must not throw work away.
    dismissible: bool,
    on_close: EventHandler<()>,
    children: Element,
) -> Element {
    let dismiss_on_key = move |event: Event<KeyboardData>| {
        if event.key() != Key::Escape || !dismissible {
            return;
        }
        event.prevent_default();
        event.stop_propagation();
        on_close.call(());
    };
    let dismiss_on_click = move |_| {
        if dismissible {
            on_close.call(());
        }
    };
    let enter_id = panel_id.clone();
    let leave_id = panel_id.clone();
    use_effect(move || {
        document::eval(&dialog_enter_script(&enter_id));
    });
    use_drop(move || {
        document::eval(&dialog_leave_script(&leave_id));
    });

    rsx! {
        div {
            class: "modal-backdrop",
            role: "presentation",
            onclick: dismiss_on_click,
            div {
                class,
                id: panel_id,
                role: "dialog",
                aria_modal: "true",
                aria_label: label,
                onclick: move |event| event.stop_propagation(),
                onkeydown: dismiss_on_key,
                {children}
            }
        }
    }
}

/// Sidebar identity. The kicker is the real build (`EMBEDDED · 1.13.14`), not a
/// fixed tag, so the shell the user is driving is readable at a glance.
#[component]
pub(crate) fn Brand(kicker: String) -> Element {
    rsx! {
        div { class: "brand",
            div {
                class: "brand-mark",
                aria_hidden: "true",
                dangerous_inner_html: BRAND_ICON_SVG,
            }
            div {
                strong { "Kitty Pro" }
                small { "{kicker}" }
            }
        }
    }
}

/// Render the reported core build as the sidebar kicker: a core that reports
/// `kitty-pro-embedded/1.13.14` reads as `EMBEDDED · 1.13.14`.
pub(crate) fn brand_kicker(core_version: Option<String>) -> String {
    match core_version {
        Some(version) => version
            .strip_prefix("kitty-pro-")
            .unwrap_or(&version)
            .replace('/', " · ")
            .to_uppercase(),
        None => "SING-BOX CLIENT".to_string(),
    }
}

#[component]
pub(crate) fn NavItem(
    view: AppView,
    mut active_view: Signal<AppView>,
    /// Trailing count chip (nodes, subscriptions, rules, log entries).
    count: Option<String>,
    /// Small attention dot. Used by Settings while an update is waiting; the
    /// count chip is absent there, so the two never compete for the same slot.
    #[props(default)]
    dot: bool,
) -> Element {
    let active = active_view() == view;
    rsx! {
        button {
            class: if active { "nav-item active" } else { "nav-item" },
            aria_current: if active { "page" },
            // The dot itself is a 7px circle: colour carries the whole message,
            // which no screen reader reads. Naming the button is the one place
            // that is guaranteed to be announced, and `aria_label` on a bare
            // `span` is not - so the dot stays decorative and the label says what
            // the dot means. Only while the dot is up, so every other item keeps
            // the name its visible text already gives it.
            aria_label: if dot {
                format!("{}（有新版本可更新）", view.title())
            },
            onclick: move |_| active_view.set(view),
            match view {
                AppView::Overview => rsx! { Icon { icon: LdLayoutDashboard, width: 20, height: 20 } },
                AppView::Nodes => rsx! { Icon { icon: LdGitFork, width: 20, height: 20 } },
                AppView::Subscriptions => rsx! { Icon { icon: LdRss, width: 20, height: 20 } },
                AppView::Rules => rsx! { Icon { icon: LdSplit, width: 20, height: 20 } },
                AppView::Logs => rsx! { Icon { icon: LdScrollText, width: 20, height: 20 } },
                AppView::Settings => rsx! { Icon { icon: LdSlidersHorizontal, width: 20, height: 20 } },
            }
            span { "{view.title()}" }
            if let Some(count) = count {
                span { class: "nav-count", "{count}" }
            } else if dot {
                // Tooltip for a pointer; assistive tech reads the button's name.
                span { class: "nav-dot", title: "有新版本可更新", aria_hidden: "true" }
            }
        }
    }
}
