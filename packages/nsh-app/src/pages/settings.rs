//! Settings: appearance, and the mode that decides where traffic goes.
//!
//! Every row writes through the store, which owns persistence, so this screen never touches
//! the disk itself.

use crate::domain;
use crate::store::{AppStore, Screen};
use crate::ui::{controls, theme};
use nsh_base::{button, theme as base_theme};
use nsh_core::prelude::*;
use proxy_core::TunnelMode;

/// The one name for the appearance switch. The rail carries the same control, so the
/// label and note live here and are shared rather than written twice.
pub const APPEARANCE_LABEL: &str = "深色模式";

/// The switch's note: what the current value means, not a restatement of the label.
pub fn appearance_note(dark: bool) -> SharedString {
    SharedString::from(if dark { "已启用" } else { "跟随系统" })
}

pub fn settings(store: AppStore) -> impl IntoElement {
    crate::shell::page_scroll(
        view()
            .flex_col()
            .gap_4()
            .child(appearance_card(store))
            .child(mode_card(store))
            .child(script_card(store))
            .child(config_links_card(store)),
    )
}

/// The appearance choice is the app's, not the window's: it goes into the profile, so a
/// relaunch comes back the same way.
fn appearance_card(store: AppStore) -> impl IntoElement {
    let dark = store.dark_mode;
    controls::card(
        view()
            .flex_col()
            .gap_4()
            .child(heading("外观"))
            .child(controls::switch_row(
                APPEARANCE_LABEL,
                move || appearance_note(dark.get()),
                dark,
                signal(false),
                move |next| store.set_dark_mode(next),
            )),
    )
}

/// Where traffic goes. Rule is the everyday mode, so the other two read as deliberate
/// exceptions rather than three equal choices.
fn mode_card(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_4()
        .child(heading("流量走向"))
        .child(mode_row(store, TunnelMode::Rule, "国内直连，其余走代理"))
        .child(mode_row(store, TunnelMode::Global, "全部流量走代理"))
        .child(mode_row(store, TunnelMode::Direct, "全部直连，仅本机监听"))
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
}

fn mode_row(store: AppStore, mode: TunnelMode, note: &'static str) -> impl IntoElement {
    let selected = computed(move || store.tunnel_mode() == mode);
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(domain::mode_label(mode))
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(text(note).text_xs().text_color(theme::text_faint())),
        )
        .child(mode_pill(selected, move || store.set_tunnel_mode(mode)))
}

/// The live mode wears the accent so one glance says which is in effect. `on_select`
/// is a no-op while already chosen, which `set_tunnel_mode` guards as well.
fn mode_pill(selected: Memo<bool>, on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .selected(move || selected.get())
        .on_press(move |_: &()| on_press())
        .h(px(30.0))
        .px_3()
        .items_center()
        .justify_center()
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || base_theme().border)
        .selected_style(|s| {
            s.bg(move || base_theme().primary)
                .border_color(move || base_theme().primary)
        })
        .hover(|s| s.bg(move || base_theme().muted))
        .active(|s| s.opacity(0.8))
        .focus(|s| s.outline(px(2.0)).outline_color(move || base_theme().ring))
        .child(
            text("启用")
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if selected.get() {
                        base_theme().primary_foreground
                    } else {
                        base_theme().card_foreground
                    }
                }),
        )
}

/// The rest of the configuration belongs to its own screens; this page points there instead
/// of keeping a second copy of it.
fn config_links_card(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_4()
        .child(heading("配置"))
        .child(config_link(
            store,
            "订阅",
            "导入与更新节点来源",
            Screen::Subscriptions,
        ))
        .child(config_link(
            store,
            "自定义规则",
            "按域名与端口改写走向",
            Screen::Rules,
        ))
        .child(config_link(
            store,
            "日志",
            "查看转发与拦截记录",
            Screen::Logs,
        ))
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
}

fn config_link(
    store: AppStore,
    label: &'static str,
    note: &'static str,
    target: Screen,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(label)
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(text(note).text_xs().text_color(theme::text_faint())),
        )
        .child(controls::secondary_button("管理", move || {
            store.set_screen(target)
        }))
}

fn heading(label: &'static str) -> impl IntoElement {
    text(label)
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || base_theme().card_foreground)
}

/// The QuickJS override that can take the whole config in hand. It sits here rather than on
/// Rules because it does not add a rule: it replaces the set, and a user who finds rules
/// greyed out needs to see that a script is what took them over.
///
/// The editor writes straight into the store and saves on every 保存. Validation is separate,
/// because only the core's own validator can say whether a script runs, and it costs a
/// config build to find out.
fn script_card(store: AppStore) -> impl IntoElement {
    let draft = signal(String::new());
    let open = signal(false);
    view()
        .flex_col()
        .gap_4()
        .child(heading("配置脚本"))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_4()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text("JavaScript 配置覆写")
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || base_theme().card_foreground),
                        )
                        .child(
                            text(move || script_note(store))
                                .text_xs()
                                .text_color(theme::text_faint()),
                        ),
                )
                .child(controls::secondary_button(move || editor_label(open), {
                    let (draft, open) = (draft, open);
                    move || {
                        let next = !open.get_untracked();
                        if next {
                            draft.set(store.config_script());
                        }
                        open.set(next);
                    }
                }))
                .child(controls::switch_state(
                    || Some("启用 JavaScript 配置覆写".into()),
                    move || store.config_script_enabled(),
                    // Nothing to enable while the script is blank: the switch would claim an
                    // override that the core never applies.
                    move || store.config_script().trim().is_empty(),
                    move |enabled| {
                        store.set_config_script(enabled, store.config_script());
                    },
                )),
        )
        .child(when(open, move || script_editor(store, draft, open)))
        .child(when(
            move || store.config_script_active(),
            || {
                controls::inline_note(
                    theme::Tint::Info,
                    crate::ui::Icon::Info,
                    "启用后自定义分流规则将被脚本接管。",
                )
            },
        ))
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
}

fn script_note(store: AppStore) -> String {
    let script = store.config_script();
    if script.trim().is_empty() {
        "main(config) · 尚未配置".to_owned()
    } else {
        format!("main(config) · QuickJS · {} 字符", script.chars().count())
    }
}

fn editor_label(open: Signal<bool>) -> &'static str {
    if open.get() {
        "收起"
    } else {
        "编辑"
    }
}

fn script_editor(store: AppStore, draft: Signal<String>, open: Signal<bool>) -> impl IntoElement {
    view()
        .flex_col()
        .gap_3()
        .child(controls::text_editor(
            "脚本",
            draft,
            "function main(config) {\n  return config;\n}",
            12.0,
        ))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(controls::secondary_button(
                    move || {
                        if store.script_busy() {
                            "正在校验"
                        } else {
                            "校验"
                        }
                    },
                    move || {
                        if store.script_busy() {
                            return;
                        }
                        store.validate_config_script(draft.get_untracked());
                    },
                ))
                .child(controls::primary_button("保存", move || {
                    // Saving keeps the switch where it is. Only an emptied script forces it
                    // off, which the store does itself.
                    store.set_config_script(store.config_script_enabled(), draft.get_untracked());
                    open.set(false);
                })),
        )
}
