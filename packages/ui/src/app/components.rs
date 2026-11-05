//! Part of the `app` module (split from the original monolithic `app.rs`).
//!
//! Shared control layer (U8). The switch, segmented control and two-step
//! confirm button used to be hand-written in six different pages with subtly
//! different aria/busy/arm behaviour. These three components own that surface
//! now. Every class used below already exists in `app.css`; no new styles.

use super::*;

/// The hidden checkbox + switch track, plus a `role="status"` region that
/// carries the hint / disabled reason for assistive tech.
///
/// `Switch` is deliberately a fragment (no wrapping `<label>`): each call site
/// keeps its own `label` row (`setting-row toggle-row`, `compact-toggle`,
/// `quick-toggle`, `overview-proxy-toggle`, ...), so the whole-row click target
/// and the `input:checked + .switch` CSS stay exactly as they were. The busy
/// spinner variant is rendered by the call site while its own action is in
/// flight, since only that scope knows the platform-specific busy wording.
#[component]
pub(crate) fn Switch(
    checked: bool,
    label: String,
    onchange: EventHandler<bool>,
    #[props(default)] disabled: bool,
    #[props(default)] aria_label: Option<String>,
    #[props(default)] hint: Option<String>,
) -> Element {
    // U8: the accessible name defaults to the visible label; `hint` (a disabled
    // reason or a short explanation) lives in an sr-only status region so it is
    // announced without disturbing the row layout.
    let name = aria_label.unwrap_or_else(|| label.clone());
    let note = hint.unwrap_or_default();
    rsx! {
        input {
            r#type: "checkbox",
            aria_label: "{name}",
            checked: checked,
            disabled: disabled,
            onchange: move |event| onchange.call(event.checked()),
        }
        span { class: "switch" }
        span { class: "sr-only", role: "status", "{note}" }
    }
}

/// A grouped set of mutually-exclusive buttons. `extra_class` lets callers keep
/// the existing `wide` / `rules-tab-control` / `sync-provider-control` variants
/// without adding any new CSS class.
///
/// 键盘交互约定（U5，WAI-ARIA `role="group"` + `aria_pressed` 开关按钮，
/// 手动激活模型）：
/// - roving tabindex：只有「当前选中项」是 `tabindex="0"`，其余都是 `-1`，
///   所以 Tab 只会在控件里停一次；`onfocus` 把真实焦点同步进 `focus_index`，
///   方向键据此计算下一项，Tab / 点击 / 脚本聚焦后基准都不会错位。
/// - → / ↓ 下一项、← / ↑ 上一项、Home 首项、End 末项，均循环（wrap），
///   只移动 DOM 焦点并 `prevent_default()`（避免页面滚动 / 误触发）；方向键
///   不改变选中、也不调用 `onchange`。
/// - Enter / Space 不在这里拦截，交给 `<button>` 原生 click → `onclick`
///   → `onchange` 才真正切换 `value` / `aria_pressed`，这正是「方向键不会
///   误触发」的含义。
/// - `disabled` 时所有按钮禁用，方向键直接返回，不做任何处理。
///
/// 方向键聚焦方案：用项目已跨端（wasm32 / 桌面 webview，见 proxy_app.rs 与
/// chrome.rs 的无条件调用）的 `document::eval` + 稳定元素 id 调 `.focus()`；
/// eval 为异步，故 Playwright 断言用 `expect.poll` 等待 `document.activeElement`
/// 落位。若目标根本没有 DOM（纯 native 且无 webview），eval 为空操作，但
/// roving tabindex 与 `onchange` 逻辑仍完全正确，仅残留「不移动 OS 焦点」一点。
#[component]
pub(crate) fn SegmentedControl(
    options: Vec<(String, String)>,
    value: String,
    onchange: EventHandler<String>,
    aria_label: String,
    #[props(default)] extra_class: String,
    #[props(default)] disabled: bool,
) -> Element {
    let control_class = if extra_class.is_empty() {
        "segmented-control".to_string()
    } else {
        format!("segmented-control {extra_class}")
    };

    let option_count = options.len();
    // 焦点索引初值取选中项（而非固定 0）：首次 Tab / 点击聚焦前若就收到方向键，
    // 计算基准也是正确的；聚焦后由每个按钮的 onfocus 继续维护。
    let selected_index = options
        .iter()
        .position(|(option_value, _)| *option_value == value)
        .unwrap_or(0);
    let mut focus_index = use_signal(|| selected_index);

    // 稳定 id：aria_label 去掉 ASCII 空白后拼 `-seg-{i}`。中文标签在 HTML id
    // 与 getElementById 里合法，{:?} 会把整串安全转义成 JS 字符串字面量；
    // 三个调用点的 aria_label 互不相同，因此 id 不会互相冲突。
    let id_base: String = aria_label
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect();
    let key_id_base = id_base.clone();

    let on_key = move |event: Event<KeyboardData>| {
        if disabled || option_count == 0 {
            return;
        }
        let current = focus_index();
        let next = match event.key() {
            Key::ArrowRight | Key::ArrowDown => (current + 1) % option_count,
            Key::ArrowLeft | Key::ArrowUp => (current + option_count - 1) % option_count,
            Key::Home => 0,
            Key::End => option_count - 1,
            // Enter / Space 及其它键一律不拦截，交给 button 原生 click，
            // 确保方向键不会误触发 onchange / 选中。
            _ => return,
        };
        event.prevent_default();
        focus_index.set(next);
        let id = format!("{key_id_base}-seg-{next}");
        document::eval(&format!("document.getElementById({:?}).focus()", id));
    };

    rsx! {
        div {
            class: "{control_class}",
            role: "group",
            aria_label: "{aria_label}",
            onkeydown: on_key,
            for (i, (option_value, option_label)) in options.into_iter().enumerate() {
                {
                    let selected = option_value == value;
                    let clicked_value = option_value.clone();
                    let button_id = format!("{id_base}-seg-{i}");
                    rsx! {
                        button {
                            key: "{option_value}",
                            id: "{button_id}",
                            class: if selected { "active" } else { "" },
                            // U5/a11y spec：保留 aria_pressed；Dioxus 把 bool
                            // 渲染成显式的 "true" / "false" 两个字符串。
                            aria_pressed: selected,
                            // roving tabindex：仅选中项可 Tab 进入。
                            tabindex: if selected { "0" } else { "-1" },
                            disabled: disabled,
                            onfocus: move |_| focus_index.set(i),
                            onclick: move |_| {
                                focus_index.set(i);
                                onchange.call(clicked_value.clone());
                            },
                            "{option_label}"
                        }
                    }
                }
            }
        }
    }
}

/// Two-step confirm button (U7). No timer is spawned: the arm is an
/// `Instant` checked on render and on each click, so wasm/native behave alike,
/// a stale arm expires after 3 s, and switching pages unmounts the component
/// (which drops the arm) for free.
///
/// `compact` is for icon-only rows (the subscription delete button): it renders
/// a trash icon instead of the visible text, while `aria_label` / `title` still
/// swap between `label` and `confirm_label`. `onarm` reports every arm/disarm so
/// a caller can keep an existing summary panel or arm generation in sync.
#[component]
pub(crate) fn ConfirmButton(
    label: String,
    confirm_label: String,
    onconfirm: EventHandler<()>,
    #[props(default)] disabled: bool,
    #[props(default)] title: Option<String>,
    #[props(default)] extra_class: String,
    #[props(default)] compact: bool,
    #[props(default)] onarm: EventHandler<bool>,
) -> Element {
    let armed = use_signal(|| None::<std::time::Instant>);
    // A subscribed read (not peek) so setting the arm re-renders the label;
    // the elapsed check makes any render after 3 s treat it as expired.
    let is_armed = match armed() {
        Some(at) => at.elapsed() <= std::time::Duration::from_secs(3),
        None => false,
    };
    let display = if is_armed {
        confirm_label.clone()
    } else {
        label.clone()
    };
    let tip = title.unwrap_or_else(|| label.clone());
    let mut button_class = if extra_class.is_empty() {
        "secondary-button compact".to_string()
    } else {
        extra_class.clone()
    };
    if is_armed {
        button_class.push_str(" armed");
    }
    rsx! {
        button {
            class: "{button_class}",
            aria_label: "{display}",
            title: "{tip}",
            disabled: disabled,
            onclick: move |_| {
                let mut arm = armed;
                let active = match *arm.peek() {
                    Some(at) => at.elapsed() <= std::time::Duration::from_secs(3),
                    None => false,
                };
                onarm.call(!active);
                if active {
                    arm.set(None);
                    onconfirm.call(());
                } else {
                    // A fresh arm; the elapsed check above makes any stale
                    // stamp expire before it can confirm a much later click.
                    arm.set(Some(std::time::Instant::now()));
                }
            },
            if compact {
                Icon { icon: LdTrash2, width: 17, height: 17 }
            } else {
                "{display}"
            }
        }
    }
}
