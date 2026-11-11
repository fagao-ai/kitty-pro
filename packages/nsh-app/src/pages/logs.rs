//! Logs: what the core did with each connection, and what it said about itself.
//!
//! The console reads newest-first, so "following" means being parked at the top and a new row
//! never moves the viewport. Scroll away and the rows arrive above the fold, which is what the
//! count chip is for: it names how many were missed and jumps back.
//!
//! Collection runs only while this page is open over a running core, which the shell keeps in
//! sync. The indicator reports the answer from the backend rather than the wish, so a refusal
//! to collect parks the page on "已暂停" instead of claiming a live feed.

use crate::domain::{self, LogFilter};
use crate::i18n;
use crate::store::{AppStore, LOG_BUFFER};
use crate::ui::{controls, theme, Icon};
use api::CoreLogEntry;
use nsh_base::button;
use nsh_base::theme as base_theme;
use nsh_core::prelude::*;

/// Points from the top that still count as being at the newest row.
const FOLLOW_THRESHOLD: f32 = 4.0;

pub fn logs(store: AppStore) -> impl IntoElement {
    // Reading the newest row starts collection; the count only drives the rows themselves.
    let unseen = signal(0usize);
    let newest_seen = signal(None::<u64>);
    let following = signal(true);
    let list = element_ref();

    effect(move || {
        let entries = store.logs();
        let Some(newest) = entries.last().map(|entry| entry.sequence) else {
            // The buffer is empty: cleared, or the core restarted. Nothing is above the fold
            // any more, so a reader who had scrolled away is back in step.
            if unseen.get_untracked() != 0 {
                unseen.set(0);
            }
            if !following.get_untracked() {
                following.set(true);
            }
            return;
        };
        let previous = newest_seen.get_untracked();
        newest_seen.set(Some(newest));
        let Some(previous) = previous else {
            // The first batch since the page opened has nothing missed.
            return;
        };
        if newest <= previous || following.get_untracked() {
            if unseen.get_untracked() != 0 {
                unseen.set(0);
            }
            return;
        }
        let added = domain::log_entries_newer_than(&entries, previous);
        unseen.update(|count| *count = count.saturating_add(added));
    });

    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .max_w(px(1080.0))
        .self_center()
        .flex_col()
        .gap_3()
        .px_5()
        .pt_5()
        // Room for the page's own floating action, which sits over this column's bottom-right
        // corner; the log list is bounded by the column, so this lifts its last row clear.
        .pb(controls::action_clearance())
        .child(controls::card(
            view()
                .flex_col()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(each(
                            move || LogFilter::ALL.to_vec(),
                            |filter| *filter as u8,
                            move |item, _| {
                                let filter = move || item.get();
                                let selected = computed(move || store.log_filter() == filter());
                                controls::filter_pill(
                                    filter().label(),
                                    move || tally(store, filter()),
                                    selected,
                                    filter_tint(filter()),
                                    move || store.log_filter.set(filter()),
                                )
                            },
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .h(px(34.0))
                                .rounded(theme::radius_tile())
                                .border_1()
                                .border_color(move || base_theme().input)
                                .focus_within(|s| s.border_color(move || base_theme().ring))
                                .child(
                                    controls::icon(Icon::Search, 15.0)
                                        .text_color(move || base_theme().muted_foreground),
                                )
                                .child(
                                    nsh_base::input()
                                        .value(move || store.log_query.get())
                                        .on_change(move |text: &str| {
                                            store.log_query.set(text.to_owned());
                                        })
                                        .placeholder("搜索域名、IP、节点")
                                        .placeholder_color(|| color(base_theme().muted_foreground))
                                        .caret_color(|| color(base_theme().primary))
                                        .label("搜索域名、IP、节点")
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_sm()
                                        .text_color(move || base_theme().card_foreground),
                                ),
                        )
                        .child(collection_chip(store))
                        .child(controls::secondary_button_state(
                            "清空日志",
                            move || store.logs().is_empty(),
                            move || store.clear_logs(),
                        )),
                )
                .child(
                    text(move || i18n::tr(&(buffer_note(store))))
                        .text_xs()
                        .text_color(theme::text_faint()),
                ),
        ))
        .child(log_list(store, list, unseen, following))
        // 日志's own action: pause and resume the feed. The core streams logs only while asked,
        // so this is a real switch rather than a scroll lock, and the chip beside the search
        // says which of the three states the page is in.
        .child(controls::floating_layer(controls::floating_action_state(
            move || {
                if store.log_paused() {
                    Icon::Play
                } else {
                    Icon::Pause
                }
            },
            move || {
                if !store.connected() {
                    // Pausing is the store's doing when the core stops, so naming the press
                    // "resume" would promise a feed there is no core to produce one.
                    "内核未运行，无法采集日志".into()
                } else if store.log_paused() {
                    "继续采集日志".into()
                } else {
                    "暂停采集日志".into()
                }
            },
            // Lit while the feed is actually flowing: paused, or stopped along with the core,
            // is not, and the disc should not claim a live stream that is not one.
            move || store.connected() && !store.log_paused(),
            // With no core running there is nothing to collect, and the page already says 已停止.
            move || !store.connected(),
            move || store.set_log_collection(store.log_paused()),
        )))
}

/// Collecting, paused, or stopped because there is no core. Three real states rather than a
/// pause button that would do nothing when nothing runs.
fn collection_chip(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .h(px(32.0))
        .flex_shrink_0()
        .rounded(theme::radius_tile())
        .bg(move || collection_tint(store).wash())
        .child(controls::status_dot_tinted(move || collection_tint(store)))
        .child(
            text(move || i18n::tr(collection_label(store)))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || collection_tint(store).foreground()),
        )
}

fn collection_tint(store: AppStore) -> theme::Tint {
    if !store.connected() {
        theme::Tint::Danger
    } else if store.log_paused() {
        theme::Tint::Warning
    } else {
        theme::Tint::Success
    }
}

fn collection_label(store: AppStore) -> &'static str {
    if !store.connected() {
        "已停止"
    } else if store.log_paused() {
        "已暂停"
    } else {
        "采集中"
    }
}

/// How many lines are held and how many are showing.
///
/// Built from translated *templates* rather than translated after formatting: `fill` looks the
/// sentence up with its holes still in it, so English can say `0 of 0 lines shown` where the
/// Chinese puts the counts before the nouns. A `format!` first would hand the table a string no
/// table can hold.
fn buffer_note(store: AppStore) -> String {
    let stored = store.logs().len();
    let shown = visible(store).len();
    let mut note = i18n::fill("缓冲区 {} 条记录 · 当前显示 {} 条", &[&stored, &shown]);
    if stored >= LOG_BUFFER {
        note.push_str(&i18n::in_parens(&i18n::fill(
            "最多保留 {} 条",
            &[&LOG_BUFFER],
        )));
    }
    if let Some(reason) = store.log_note() {
        note.push_str(&i18n::fill(" · 采集失败：{}", &[&reason]));
    }
    note
}

fn log_list(
    store: AppStore,
    list: ElementRef,
    unseen: Signal<usize>,
    following: Signal<bool>,
) -> impl IntoElement {
    let rows = computed(move || visible(store));
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(header_row())
        .child(
            virtual_list(
                move || rows.with(Vec::len),
                move |at| {
                    let entry = computed(move || rows.with(|rows| rows.get(at.index()).cloned()));
                    log_row(entry)
                },
            )
            .keys(move || {
                rows.with(|rows| {
                    rows.iter()
                        .map(|entry| entry.sequence.to_string())
                        .collect::<Vec<_>>()
                })
            })
            .estimated_item_size(px(44.0))
            .separator(|| {
                view()
                    .h(px(1.0))
                    .mx_5()
                    .bg(move || base_theme().border)
                    .into_element()
            })
            .empty(when(
                move || store.connected(),
                move || {
                    controls::empty_state(
                        "暂无匹配日志",
                        "换个筛选或关键词；启动代理后每条连接都会写在这里。",
                    )
                },
            ))
            .flex_1()
            .min_h(px(0.0))
            .element_ref(&list)
            .on_scroll(move |event: &ScrollEvent| {
                // Parked at the top means following: the newest row is already in view, so
                // a row arriving above it costs the reader nothing.
                let at_latest = event.offset.y.abs() <= px(FOLLOW_THRESHOLD);
                if following.get_untracked() != at_latest {
                    following.set(at_latest);
                }
                if at_latest && unseen.get_untracked() != 0 {
                    unseen.set(0);
                }
            }),
        )
        .child(when(
            move || unseen.get() > 0,
            move || jump_to_latest(list, unseen, following),
        ))
}

/// The row of column names. The rows themselves are a plain stack, so without this a reader
/// has to guess which monospaced string is the exit and which is the host.
fn header_row() -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_5()
        .py_2()
        .border_b_1()
        .border_color(move || base_theme().border)
        .child(column_label("时间", 92.0))
        .child(column_label("动作", 52.0))
        .child(column_label("目标", 220.0))
        .child(column_label("命中链路", 160.0))
        .child(column_label("出口节点", 160.0))
        .child(
            text(move || i18n::tr("消息"))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .flex_1()
                .min_w(px(0.0))
                .text_color(theme::text_faint()),
        )
}

fn column_label(label: &'static str, width: f32) -> impl IntoElement {
    // Translated here: one line covers all six column headers, and the words are ours.
    text(move || i18n::tr(label))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .w(px(width))
        .flex_shrink_0()
        .text_color(theme::text_faint())
}

/// The chip that says how many rows arrived above the fold, and takes the reader back.
fn jump_to_latest(
    list: ElementRef,
    unseen: Signal<usize>,
    following: Signal<bool>,
) -> impl IntoElement {
    button()
        .on_press(move |_: &()| {
            following.set(true);
            unseen.set(0);
            list.scroll_to(Point::new(px(0.0), px(0.0)), false);
        })
        .self_center()
        .my_2()
        .px_3()
        .h(px(28.0))
        .flex_row()
        .items_center()
        .gap_1p5()
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().popover)
        .hover(|s| s.bg(move || base_theme().accent))
        .focus(|s| s.outline(px(2.0)).outline_color(move || base_theme().ring))
        .child(
            controls::icon(Icon::ArrowUp, 13.0).text_color(move || base_theme().popover_foreground),
        )
        .child(
            text(move || format!("{} 条新日志", unseen.get()))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || base_theme().popover_foreground),
        )
}

fn log_row(entry: Memo<Option<CoreLogEntry>>) -> impl IntoElement {
    let route = move || entry.with(|entry| entry.as_ref().and_then(|entry| entry.route.clone()));
    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_5()
        .py_2()
        .child(
            text(move || {
                entry
                    .with(|entry| entry.as_ref().map(|entry| entry.timestamp.clone()))
                    .unwrap_or_default()
            })
            .text_xs()
            .font_family(theme::font_mono())
            .w(px(92.0))
            .flex_shrink_0()
            .text_color(theme::text_faint()),
        )
        .child(when(
            move || route().is_some(),
            move || {
                view().w(px(52.0)).flex_shrink_0().child(controls::badge(
                    move || {
                        route()
                            .map(|route| domain::route_decision_label(route.decision).to_owned())
                            .unwrap_or_default()
                    },
                    move || {
                        route().map_or(theme::Tint::Neutral, |route| {
                            domain::route_decision_tint(route.decision)
                        })
                    },
                ))
            },
        ))
        .child(when(
            move || route().is_none(),
            move || {
                view().w(px(52.0)).flex_shrink_0().child(
                    text(move || {
                        entry
                            .with(|entry| entry.as_ref().map(|entry| entry.level.clone()))
                            .unwrap_or_default()
                            .to_uppercase()
                    })
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .font_family(theme::font_mono())
                    .text_color(move || base_theme().muted_foreground),
                )
            },
        ))
        .child(
            view()
                .w(px(220.0))
                .flex_shrink_0()
                .flex_col()
                .child(
                    text(move || {
                        route()
                            .map(|route| route.host)
                            .unwrap_or_else(|| "—".to_owned())
                    })
                    .text_xs()
                    .font_family(theme::font_mono())
                    .number_of_lines(1)
                    .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || target_note(route()))
                        .text_size(px(10.0))
                        .text_color(theme::text_faint()),
                ),
        )
        .child(
            view()
                .w(px(160.0))
                .flex_shrink_0()
                .flex_row()
                .gap_1()
                .child(
                    text(move || {
                        route().map_or_else(
                            || "-".to_owned(),
                            |route| domain::route_chain_hops(&route).join(" → "),
                        )
                    })
                    .text_size(px(10.0))
                    .font_family(theme::font_mono())
                    .number_of_lines(1)
                    .text_color(theme::text_soft()),
                ),
        )
        .child(
            view()
                .w(px(160.0))
                .flex_shrink_0()
                .flex_col()
                .child(
                    text(move || {
                        route()
                            .map(|route| route.outbound_tag)
                            .unwrap_or_else(|| "-".to_owned())
                    })
                    .text_xs()
                    .font_family(theme::font_mono())
                    .number_of_lines(1)
                    .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || route().map(|route| route.outbound_type).unwrap_or_default())
                        .text_size(px(10.0))
                        .text_color(theme::text_faint()),
                ),
        )
        .child(
            text(move || {
                entry
                    .with(|entry| entry.as_ref().map(|entry| entry.message.clone()))
                    .unwrap_or_default()
            })
            .text_xs()
            .font_family(theme::font_mono())
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_color(theme::text_soft()),
        )
}

/// What kind of target it was and which port, on the line under the host: the host alone does
/// not say whether 443 was reached by name or by address.
fn target_note(route: Option<api::RouteLogDetail>) -> String {
    let Some(route) = route else {
        return String::new();
    };
    match route.port {
        Some(port) => format!(
            "{} · {port}",
            domain::route_target_kind_label(route.target_kind)
        ),
        None => domain::route_target_kind_label(route.target_kind).to_owned(),
    }
}

fn filter_tint(filter: LogFilter) -> theme::Tint {
    match filter {
        LogFilter::Routes => theme::Tint::Neutral,
        LogFilter::Proxy => theme::Tint::Success,
        LogFilter::Direct => theme::Tint::Info,
        LogFilter::Block => theme::Tint::Danger,
        LogFilter::Raw => theme::Tint::Dns,
    }
}

/// One tally strip's number. Counts come from the whole buffer rather than the filtered view,
/// so the chip that says "拦截 3" is still correct after a search narrows the rows.
fn tally(store: AppStore, filter: LogFilter) -> usize {
    store
        .logs()
        .iter()
        .filter(|entry| domain::log_matches_filter(entry, filter))
        .count()
}

fn visible(store: AppStore) -> Vec<CoreLogEntry> {
    let query = store.log_query.get().trim().to_ascii_lowercase();
    domain::visible_log_entries(&store.logs(), store.log_filter(), &query)
}
