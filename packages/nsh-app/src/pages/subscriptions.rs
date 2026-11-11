//! Subscriptions: where the nodes come from.
//!
//! One subscription is active at a time and only its nodes reach the core, so the page's
//! central act is choosing which one that is. Import, refresh and delete all report through
//! the store, which owns persistence and the notice stack.

use crate::i18n;
use crate::store::AppStore;
use crate::ui::{controls, theme, Icon};
use nsh_base::{alert_dialog, dialog, theme as base_theme};
use nsh_core::prelude::*;
use proxy_core::Subscription;

pub fn subscriptions(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .max_w(px(1080.0))
        .self_center()
        .flex_col()
        .child(when(
            move || store.import_open(),
            move || import_dialog(store),
        ))
        .child(
            scroll().flex_1().min_h(px(0.0)).child(
                view()
                    .w_full()
                    .max_w(px(1080.0))
                    .self_center()
                    .flex_col()
                    .gap_4()
                    .px_5()
                    .pt_5()
                    .pb(controls::action_clearance())
                    .child(toolbar(store))
                    .child(when(move || store.subscriptions().is_empty(), empty_state))
                    .child(when(
                        move || !store.subscriptions().is_empty() && store.nodes().is_empty(),
                        move || {
                            controls::inline_note_with(
                                theme::Tint::Info,
                                Icon::Info,
                                "订阅已更新但节点尚未解析，连接前请先刷新全部订阅。",
                                Some(controls::secondary_button(
                                    "刷新全部订阅",
                                    move || {
                                        store.refresh_all_subscriptions();
                                    },
                                )),
                            )
                        },
                    ))
                    .child(when(
                        move || !store.subscriptions().is_empty(),
                        move || list(store),
                    )),
            ),
        )
        // 订阅's own action, like every other page's. The two inline 添加订阅 buttons are gone:
        // with a disc in the corner, the toolbar's copy was a third way to the same dialog.
        .child(controls::floating_layer(controls::floating_action_state(
            || Icon::Plus,
            || "添加订阅".into(),
            // A form action: the disc's accent is reserved for pages with a running state.
            || false,
            || false,
            move || store.open_import(),
        )))
}

/// The count, and the two actions that change it. Refresh sits beside add because both are
/// about the set of sources rather than about any one of them.
fn toolbar(store: AppStore) -> impl IntoElement {
    controls::card(
        view()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex_col()
                    .gap_0p5()
                    .child(
                        text(move || i18n::tr("订阅"))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || base_theme().card_foreground),
                    )
                    .child(
                        text(move || i18n::tr(&(count_summary(store))))
                            .text_xs()
                            .text_color(theme::text_faint()),
                    ),
            )
            .child(when(
                move || !store.subscriptions().is_empty(),
                move || refresh_all_button(store),
            )),
    )
}

fn count_summary(store: AppStore) -> String {
    let subscriptions = store.subscriptions();
    let nodes: usize = subscriptions.iter().map(Subscription::node_count).sum();
    if subscriptions.is_empty() {
        "还没有订阅".to_owned()
    } else {
        i18n::fill(
            "{} 个订阅 · 共 {} 个节点",
            &[&(subscriptions.len()), &nodes],
        )
    }
}

fn refresh_all_button(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(controls::icon_button(
            move || Icon::RefreshCw,
            move || "刷新全部订阅".into(),
            move || store.refresh_all_subscriptions(),
        ))
        .child(when(
            move || store.refresh_busy().is_some(),
            move || {
                text(move || i18n::tr("正在刷新"))
                    .text_xs()
                    .text_color(move || base_theme().muted_foreground)
            },
        ))
}

/// The first-run state explains what a subscription is for and stops there: the action belongs
/// to the page's floating disc, which is in the tab ring like any other button.
fn empty_state() -> impl IntoElement {
    controls::card(
        view()
            .flex_col()
            .items_center()
            .gap_3()
            .py_10()
            .child(
                controls::icon(Icon::RadioTower, 28.0)
                    .text_color(move || base_theme().muted_foreground),
            )
            .child(
                text(move || i18n::tr("暂无订阅"))
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || base_theme().foreground),
            )
            .child(
                text(move || {
                    i18n::tr("订阅用于导入代理节点；添加后即可在节点页挑选线路、回到概览连接。")
                })
                .text_xs()
                .text_color(theme::text_faint()),
            ),
    )
}

fn list(store: AppStore) -> impl IntoElement {
    view().flex_col().gap_3().child(each(
        move || store.subscriptions(),
        |subscription| subscription.id,
        move |item, _| row(store, item),
    ))
}

fn row(store: AppStore, item: ReadSignal<Subscription>) -> impl IntoElement {
    let subscription = move || item.get();
    let id = move || subscription().id;
    let active = move || store.active_subscription_id() == Some(id());
    let refreshing = move || store.refresh_busy().is_some_and(|busy| busy.covers(id()));
    // Any refresh in flight parks every row's buttons: switching while a fetch is running
    // would point the core at a list that is about to be replaced.
    let busy = move || store.refresh_busy().is_some() || store.core_action_busy();

    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .p_4()
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || {
            if active() {
                base_theme().primary
            } else {
                base_theme().border
            }
        })
        .bg(move || base_theme().card)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_0p5()
                .child(
                    text(move || subscription().name)
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .number_of_lines(1)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(when(
                    // Pasted content has no host to name it by, so its source label is the
                    // same words as its name; a row repeating itself is noise.
                    move || {
                        crate::domain::source_label(&subscription().source) != subscription().name
                    },
                    move || {
                        // Either a host, which is data and has no row, or the app's own
                        // 本地内容, which does. One lookup covers both because a miss keeps the
                        // string it was given.
                        text(move || i18n::tr(&crate::domain::source_label(&subscription().source)))
                            .text_xs()
                            .font_family(theme::font_mono())
                            .number_of_lines(1)
                            .text_color(theme::text_faint())
                    },
                ))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(move || {
                                i18n::fill("{} 个节点", &[&(subscription().node_count())])
                            })
                            .text_xs()
                            .text_color(theme::text_soft()),
                        )
                        .child(when(
                            move || subscription().rejected_count > 0,
                            move || {
                                controls::badge(
                                    move || {
                                        i18n::fill("忽略 {}", &[&(subscription().rejected_count)])
                                    },
                                    theme::Tint::Warning,
                                )
                            },
                        ))
                        .child(when(active, move || {
                            controls::badge("当前使用", theme::Tint::Success)
                        })),
                ),
        )
        .child(controls::secondary_button_state(
            move || if refreshing() { "刷新中" } else { "使用" },
            move || busy() || active(),
            move || store.select_subscription(id()),
        ))
        .child(controls::icon_button(
            move || Icon::RefreshCw,
            move || i18n::fill("刷新 {}", &[&(subscription().name)]).into(),
            move || {
                if busy() {
                    return;
                }
                store.refresh_subscription(id());
            },
        ))
        .child(controls::icon_button(
            move || Icon::Trash2,
            move || i18n::fill("删除 {}", &[&(subscription().name)]).into(),
            move || confirm_delete(store, id(), subscription().name),
        ))
}

/// Deleting a subscription discards its nodes, which cannot be recovered without the
/// provider, so it asks. The name is in the question because a list of similar providers is
/// exactly where an id alone means nothing.
fn confirm_delete(store: AppStore, id: u64, name: String) {
    let asking = alert_dialog()
        .title(i18n::fill("删除「{}」？", &[&name]))
        .description("该订阅的节点会一并移除，正在使用的节点会改用其他订阅。")
        .cancel("cancel", "取消")
        .destructive("delete", "删除")
        .show();
    spawn(async move {
        if asking
            .await
            .is_ok_and(|answer| answer.action.as_deref() == Some("delete"))
        {
            store.delete_subscription(id);
        }
    });
}

/// The import form. A dialog rather than a native alert because it collects two fields,
/// and it is dismissed by leaving the tree, which is what `when` does with the store's flag.
fn import_dialog(store: AppStore) -> impl IntoElement {
    let (name, source) = store.import_fields();
    let busy = move || store.import_busy();
    dialog()
        .presentation(ModalPresentation::PageSheet)
        .on_cancel(move || store.close_import())
        .flex_1()
        .items_center()
        .justify_center()
        .bg(move || base_theme().background.opacity(0.6))
        .child(
            view()
                .w_full()
                .max_w(px(480.0))
                .flex_col()
                .gap_4()
                .m_5()
                .p_5()
                .rounded(theme::radius_card())
                .border_1()
                .border_color(move || base_theme().border)
                .bg(move || base_theme().card)
                .shadow_lg()
                .child(
                    text(move || i18n::tr("添加订阅"))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(controls::text_field("名称（可选）", name, || {
                    "我的订阅"
                }))
                .child(controls::text_editor(
                    "订阅地址或节点内容",
                    source,
                    "https:// 订阅地址，或 ss://、vless:// 等节点链接",
                    8.0,
                ))
                .child(controls::field_error(store.import_error()))
                .child(
                    text(move || i18n::tr("名称留空时用订阅地址的主机命名。"))
                        .text_xs()
                        .text_color(theme::text_faint()),
                )
                .child(
                    view()
                        .flex_row()
                        .justify_end()
                        .gap_2()
                        .child(controls::secondary_button("取消", move || {
                            store.close_import();
                        }))
                        .child(controls::primary_button(
                            move || if busy() { "正在解析" } else { "导入" },
                            move || {
                                if busy() {
                                    return;
                                }
                                store.import_subscription(
                                    name.get_untracked(),
                                    source.get_untracked(),
                                );
                            },
                        )),
                ),
        )
}
