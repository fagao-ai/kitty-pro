//! Nodes: everything a subscription imported, searchable, with a latency figure and one
//! chosen node that the core sends traffic through.
//!
//! The list is virtualized. A subscription can carry thousands of nodes and a plain stack
//! would build every row at once; cells here are recycled, so a row reads its data through a
//! memo and never keeps state of its own.

use crate::domain;
use crate::store::AppStore;
use crate::ui::{controls, theme, Icon};
use nsh_base::{button, input, theme as base_theme};
use nsh_core::prelude::*;

pub fn nodes(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .max_w(px(1080.0))
        .self_center()
        .flex_col()
        .gap_4()
        .px_5()
        .py_5()
        .child(toolbar(store))
        .child(node_list(store))
}

/// Search, and the probe. The count is in the toolbar rather than a card of its own,
/// because it describes the list directly beneath it.
fn toolbar(store: AppStore) -> impl IntoElement {
    let query = store.node_query;
    view()
        .flex_col()
        .gap_4()
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .h(px(36.0))
                        .rounded(theme::radius_tile())
                        .border_1()
                        .border_color(move || base_theme().input)
                        .focus_within(|s| s.border_color(move || base_theme().ring))
                        .child(
                            controls::icon(Icon::Search, 15.0)
                                .text_color(move || base_theme().muted_foreground),
                        )
                        .child(field(query)),
                )
                .child(probe_button(store)),
        )
        .child(
            text(move || toolbar_summary(store))
                .text_xs()
                .text_color(theme::text_faint()),
        )
}

fn field(query: Signal<String>) -> nsh_base::Input {
    input()
        .value(move || query.get())
        .on_change(move |text: &str| query.set(text.to_owned()))
        .placeholder("搜索节点、地址或协议")
        .placeholder_color(|| color(base_theme().muted_foreground))
        .caret_color(|| color(base_theme().primary))
        .label("搜索节点、地址或协议")
        .flex_1()
        .min_w(px(0.0))
        .text_sm()
        .text_color(move || base_theme().card_foreground)
}

/// One button for the probe, and it becomes the way to stop it. A probe that cannot be
/// stopped makes a slow subscription feel like a frozen app.
fn probe_button(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(controls::secondary_button(
            move || {
                if store.latency_busy() {
                    "停止测速"
                } else {
                    "刷新延迟"
                }
            },
            move || {
                if store.latency_busy() {
                    store.cancel_latency();
                } else {
                    store.measure_latency();
                }
            },
        ))
}

fn toolbar_summary(store: AppStore) -> String {
    let shown = visible_nodes(store, &store.node_query.get()).len();
    let total = store.nodes().len();
    let tested = store
        .nodes()
        .iter()
        .filter(|node| store.latency_for(&node.tag).is_some())
        .count();
    if total == 0 {
        "还没有节点，先在订阅页导入".to_owned()
    } else if tested == 0 {
        format!("{shown} / {total} 个节点 · 尚未测试延迟")
    } else {
        format!("{shown} / {total} 个节点 · 已测速 {tested} 个")
    }
}

fn node_list(store: AppStore) -> impl IntoElement {
    let query = store.node_query;
    let visible = computed(move || visible_nodes(store, &query.get()));
    view()
        // The card takes the height the toolbar leaves, so the list inside it has a
        // bounded viewport. Without this the flexed list measures zero and draws no rows.
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .min_h(px(0.0))
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(when(
            move || visible.with(Vec::is_empty),
            move || controls::empty_state("没有匹配的节点", "换个关键词，或先刷新订阅。"),
        ))
        .child(
            virtual_list(
                move || visible.with(|list| list.len()),
                move |at| {
                    // A cell shows other rows as it is reused, so the row's node comes from
                    // a memo over the index rather than being captured at build time.
                    let node = computed(move || visible.with(|list| list.get(at.index()).cloned()));
                    node_row(store, node)
                },
            )
            .keys(move || {
                visible.with(|list| list.iter().map(|node| node.tag.clone()).collect::<Vec<_>>())
            })
            .estimated_item_size(px(64.0))
            .separator(|| {
                view()
                    .h(px(1.0))
                    .mx_5()
                    .bg(move || base_theme().border)
                    .into_element()
            })
            .flex_1()
            .min_h(px(0.0)),
        )
}

/// Filter by name, address or protocol, case-insensitively. Matching the protocol text is
/// what lets "ss" find Shadowsocks without the user knowing the display name.
fn visible_nodes(store: AppStore, query: &str) -> Vec<proxy_core::ProxyNode> {
    let needle = query.trim().to_lowercase();
    let nodes = store.nodes();
    if needle.is_empty() {
        return nodes;
    }
    nodes
        .into_iter()
        .filter(|node| {
            node.name.to_lowercase().contains(&needle)
                || node.server.to_lowercase().contains(&needle)
                || protocol_search_text(node.protocol).contains(&needle)
        })
        .collect()
}

fn protocol_search_text(protocol: proxy_core::ProxyProtocol) -> String {
    format!("{protocol:?}").to_lowercase()
}

fn node_row(store: AppStore, node: Memo<Option<proxy_core::ProxyNode>>) -> impl IntoElement {
    let tag = move || node.with(|node| node.as_ref().map(|node| node.tag.clone()));
    let chosen = move || tag().is_some_and(|tag| tag == store.selected_tag_text());
    button()
        .selected(chosen)
        .on_press(move |_: &()| {
            if let Some(tag) = tag() {
                store.select_node(&tag);
            }
        })
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_5()
        .py_4()
        .selected_style(|s| s.bg(move || base_theme().accent))
        .hover(|s| s.bg(move || base_theme().muted))
        .active(|s| s.opacity(0.85))
        .focus(|s| s.outline(px(2.0)).outline_color(move || base_theme().ring))
        .child(protocol_mark(node))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(move || {
                        node.with(|node| {
                            node.as_ref()
                                .map(|node| node.name.clone())
                                .unwrap_or_default()
                        })
                    })
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .number_of_lines(1)
                    .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || {
                        node.with(|node| {
                            node.as_ref()
                                .map(|node| format!("{}:{}", node.server, node.port))
                                .unwrap_or_default()
                        })
                    })
                    .text_xs()
                    .font_family(theme::font_mono())
                    .number_of_lines(1)
                    .text_color(theme::text_faint()),
                ),
        )
        .child(latency_label(store, node))
        .child(when(chosen, || {
            controls::icon(Icon::Check, 16.0).text_color(move || base_theme().accent_foreground)
        }))
}

/// The protocol as a two-letter mark, colored by family so a list of mixed protocols
/// groups itself without the user reading a single label.
fn protocol_mark(node: Memo<Option<proxy_core::ProxyNode>>) -> impl IntoElement {
    view()
        .h(px(32.0))
        .w(px(32.0))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(theme::radius_tile())
        .bg(move || {
            node.with(|node| {
                node.as_ref()
                    .map(|node| domain::protocol_tint(node.protocol).wash())
                    .unwrap_or_else(|| base_theme().muted)
            })
        })
        .child(
            text(move || {
                node.with(|node| {
                    node.as_ref()
                        .map(|node| domain::protocol_abbreviation(node.protocol))
                        .unwrap_or("--")
                        .to_owned()
                })
            })
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .font_family(theme::font_mono())
            .text_color(move || {
                node.with(|node| {
                    node.as_ref()
                        .map(|node| domain::protocol_tint(node.protocol).foreground())
                        .unwrap_or_else(|| base_theme().muted_foreground)
                })
            }),
        )
}

fn latency_label(store: AppStore, node: Memo<Option<proxy_core::ProxyNode>>) -> impl IntoElement {
    text(move || {
        let tag = node.with(|node| node.as_ref().map(|node| node.tag.clone()));
        match tag.and_then(|tag| store.latency_for(&tag)) {
            Some(result) => domain::format_latency(result.latency_ms, result.error.as_deref()).0,
            None => "—".to_owned(),
        }
    })
    .text_xs()
    .font_family(theme::font_mono())
    .flex_shrink_0()
    .text_color(theme::text_soft())
}
