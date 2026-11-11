//! Nodes: everything a subscription imported, searchable, with a latency figure and one
//! chosen node that the core sends traffic through.
//!
//! The list is grouped the way the core groups it. The page asks the engine what the config it
//! would start with actually contains rather than assuming a subscription's node list is the
//! whole story, so a provider whose nodes arrive under named groups is browsable as those
//! groups, and a group's choice can be moved without a restart.
//!
//! One virtualized list holds the headers and the members. A bounded viewport can only recycle a
//! single flat sequence — a list per group would give every group after the first no height and
//! no rows — and a subscription can carry thousands of nodes, so no row is built until it is
//! reached. A cell therefore reads what it displays through a memo over its index and never
//! keeps state of its own.

use crate::domain;
use crate::i18n;
use crate::store::AppStore;
use crate::ui::{controls, theme, Icon};
use nsh_base::{button, input, theme as base_theme};
use nsh_core::prelude::*;
use proxy_core::ProxyGroupKind;
use std::rc::Rc;

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
        .pt_5()
        // The page's action floats over this column's bottom-right corner; the list below is
        // bounded by the column, so leaving room here lifts its last row clear of the disc.
        .pb(controls::action_clearance())
        .child(toolbar(store))
        .child(node_list(store))
        // One press measures every node, the next stops it: a probe that cannot be stopped makes
        // a slow subscription feel like a frozen app. The glyph carries that, since the word
        // that used to say it is gone.
        .child(controls::floating_layer(controls::floating_action_state(
            move || {
                if store.latency_busy() {
                    Icon::Pause
                } else {
                    Icon::Gauge
                }
            },
            move || {
                if store.latency_busy() {
                    "停止测速".into()
                } else {
                    "刷新延迟".into()
                }
            },
            // Lit while a sweep is running, so the disc says "work in progress" rather than
            // merely "this page is about nodes".
            move || store.latency_busy(),
            // Nothing to measure is the one case where the press would do nothing at all; the
            // page already says 还没有节点，先在订阅页导入 in its own summary.
            move || store.nodes().is_empty(),
            move || {
                if store.latency_busy() {
                    store.cancel_latency();
                } else {
                    store.measure_latency();
                }
            },
        )))
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
            view().flex_row().items_center().gap_3().child(
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
            ),
        )
        .child(
            text(move || i18n::tr(&(toolbar_summary(store))))
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

/// The node count, which is what the search acts on, and the state of the measurement.
///
/// It counts nodes rather than rows so a header does not inflate it and a node listed under two
/// groups is not counted twice — the number a user checks against the provider's own count is
/// the number of exits, not of lines.
fn toolbar_summary(store: AppStore) -> String {
    let query = store.node_query.get();
    let shown = visible_nodes(store, &query).len();
    let total = store.nodes().len();
    let tested = store
        .nodes()
        .iter()
        .filter(|node| store.latency_for(&node.tag).is_some())
        .count();
    if total == 0 {
        "还没有节点，先在订阅页导入".to_owned()
    } else if tested == 0 {
        i18n::fill("{} / {} 个节点 · 尚未测试延迟", &[&shown, &total])
    } else {
        i18n::fill("{} / {} 个节点 · 已测速 {} 个", &[&shown, &total, &tested])
    }
}

/// One row of the list, of whatever kind. Headers and members share one sequence because the
/// list is virtualized.
#[derive(Clone, PartialEq)]
enum ListRow {
    /// The engine has not answered what its config contains yet.
    Pending,
    /// The engine refused to generate the config, with its reason.
    Failed(String),
    Header {
        tag: String,
        kind: ProxyGroupKind,
        /// The current member resolved to a name where the node list is at hand, so drawing a
        /// header never has to look a tag up again.
        selected_label: String,
        members: usize,
    },
    Node {
        /// The group the member is being chosen within, or `None` for the flat list. It is what
        /// the live switch is told, so it has to be the group the row belongs to rather than a
        /// guess.
        group: Option<String>,
        /// Shared rather than copied: a row is rebuilt for every cell that shows it, and a
        /// subscription's nodes barely change between two frames. The same reasoning behind the
        /// `Rc<Vec<ProxyNode>>` tables in the Dioxus shell.
        node: Rc<proxy_core::ProxyNode>,
        current: bool,
        selectable: bool,
    },
    /// A member that is not one of the subscription's nodes: `direct`, `block`, or another
    /// group. It has no address and no measurement, only a name and a kind.
    Outbound {
        group: String,
        tag: String,
        kind_label: &'static str,
        current: bool,
        selectable: bool,
    },
}

impl ListRow {
    /// What identifies this row across a rebuild. A member is qualified by its group because
    /// two groups can hold the same node, and the same exit listed twice must not be
    /// reconciled into one row.
    ///
    /// The current selection is deliberately not part of it: a key that changed when a node was
    /// picked would throw away and rebuild every row in the group on each press, which is the
    /// one moment the list must stay put.
    fn key(&self) -> String {
        match self {
            ListRow::Pending => "pending".to_owned(),
            ListRow::Failed(_) => "failed".to_owned(),
            ListRow::Header { tag, .. } => format!("header:{tag}"),
            ListRow::Node { group, node, .. } => {
                format!("node:{}:{}", group.as_deref().unwrap_or("-"), node.tag)
            }
            ListRow::Outbound { group, tag, .. } => format!("outbound:{group}:{tag}"),
        }
    }

    fn is_current(&self) -> bool {
        match self {
            ListRow::Node { current, .. } | ListRow::Outbound { current, .. } => *current,
            _ => false,
        }
    }

    fn is_selectable(&self) -> bool {
        match self {
            ListRow::Node { selectable, .. } | ListRow::Outbound { selectable, .. } => *selectable,
            _ => false,
        }
    }

    /// The group and outbound a press should ask for, when a press can do anything.
    fn press_target(&self) -> Option<(String, String)> {
        match self {
            ListRow::Node {
                group,
                node,
                selectable: true,
                ..
            } => Some((
                group.clone().unwrap_or_else(|| "proxy".to_owned()),
                node.tag.clone(),
            )),
            ListRow::Outbound {
                group,
                tag,
                selectable: true,
                ..
            } => Some((group.clone(), tag.clone())),
            _ => None,
        }
    }
}

fn node_list(store: AppStore) -> impl IntoElement {
    let query = store.node_query;
    let rows = computed(move || list_rows(store, &query.get()));
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
            move || rows.with(Vec::is_empty),
            move || controls::empty_state("没有匹配的节点", "换个关键词，或先刷新订阅。"),
        ))
        .child(
            virtual_list(
                move || rows.with(|list| list.len()),
                move |at| {
                    // A cell shows other rows as it is reused, so what a row displays comes
                    // from a memo over the index rather than being captured at build time.
                    let row = computed(move || rows.with(|list| list.get(at.index()).cloned()));
                    list_row(store, row)
                },
            )
            .keys(move || rows.with(|list| list.iter().map(ListRow::key).collect::<Vec<_>>()))
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

/// The rows to show: the engine's groups with their members, or the node list on its own.
///
/// The flat list is not a shortcut. A preview that has not answered yet, or one that failed,
/// still has to leave the nodes reachable — this page is how a user picks an exit, and making
/// that wait on a derivation, or lose it to one, would be a regression in the one flow that
/// matters.
fn list_rows(store: AppStore, query: &str) -> Vec<ListRow> {
    let needle = query.trim().to_lowercase();
    let groups = store.groups();
    // Error before value, and before the loading check. A first load that fails leaves no value
    // at all, so a version of this that asked for the value first fell straight through to the
    // flat list and never showed the reason — the one case where the page owes the user an
    // explanation was the one it swallowed.
    if let Some(error) = groups.error() {
        // The nodes stay listed under the reason: the derivation failed, the subscription did
        // not, and a page that shows only the error takes the exit picker away over a detail.
        let mut rows = vec![ListRow::Failed(i18n::fill("代理组生成失败：{}", &[&error]))];
        rows.extend(flat_rows(store, &needle));
        return rows;
    }
    let Some(groups) = groups.get() else {
        // Loading with nothing to show yet gets the one placeholder row; a load that is
        // refreshing over a list it already produced keeps that list on screen rather than
        // blinking it out.
        return if groups.loading() && store.nodes().is_empty() {
            vec![ListRow::Pending]
        } else {
            flat_rows(store, &needle)
        };
    };
    if groups.is_empty() {
        return flat_rows(store, &needle);
    }
    let visible = domain::visible_proxy_groups(
        &groups,
        &store.nodes(),
        &store.group_selections(),
        &store.latency_results(),
        &needle,
    );
    visible
        .into_iter()
        .flat_map(|(group, members, selected)| {
            let selectable = domain::group_is_selectable(group.kind);
            let header = ListRow::Header {
                selected_label: domain::outbound_display_name(&selected, &store.nodes()),
                tag: group.tag.clone(),
                kind: group.kind,
                members: members.len(),
            };
            let rows = members.into_iter().map(move |member| match member.node {
                Some(node) => ListRow::Node {
                    group: Some(group.tag.clone()),
                    current: selected == member.tag,
                    selectable,
                    node: Rc::new(node),
                },
                None => ListRow::Outbound {
                    group: group.tag.clone(),
                    tag: member.tag.clone(),
                    kind_label: member_kind_label(&member),
                    current: selected == member.tag,
                    selectable,
                },
            });
            std::iter::once(header).chain(rows)
        })
        .collect()
}

/// The nodes as a plain list, which is what the page showed before it asked the engine anything.
fn flat_rows(store: AppStore, needle: &str) -> Vec<ListRow> {
    let selected = store.selected_tag_text();
    visible_nodes(store, needle)
        .into_iter()
        .map(|node| ListRow::Node {
            group: None,
            current: node.tag == selected,
            selectable: true,
            node: Rc::new(node),
        })
        .collect()
}

/// What a member that is not a node is called. The engine's own two outbounds get named, since
/// a row reading `direct` in a list of proxy nodes looks like a node with a broken name.
fn member_kind_label(member: &domain::ProxyGroupMember) -> &'static str {
    match member.nested_kind {
        Some(kind) => domain::proxy_group_kind_label(kind),
        None => match member.tag.as_str() {
            "direct" => "直连",
            "block" => "拦截",
            _ => "内置出口",
        },
    }
}

/// One cell. The kind of row is read through the memo inside `dynamic`, not matched once at
/// build time: a recycled cell can go from a member to a header, and a shape decided at build
/// time would keep drawing the first one it was given.
fn list_row(store: AppStore, row: Memo<Option<ListRow>>) -> impl IntoElement {
    dynamic(move || match row.with(|row| row.clone()) {
        Some(ListRow::Header {
            tag,
            kind,
            selected_label,
            members,
        }) => group_header(tag, kind, selected_label, members).into_element(),
        Some(ListRow::Pending) => pending_row().into_element(),
        Some(ListRow::Failed(error)) => controls::inline_note(
            theme::Tint::Danger,
            Icon::CircleAlert,
            SharedString::from(error),
        )
        .into_element(),
        Some(ListRow::Outbound { .. }) => outbound_row(store, row).into_element(),
        Some(ListRow::Node { .. }) => node_row(store, row).into_element(),
        None => view().into_element(),
    })
}

fn pending_row() -> AnyElement {
    view()
        .w_full()
        .py_8()
        .items_center()
        .justify_center()
        .child(
            text(move || i18n::tr("正在生成代理组"))
                .text_xs()
                .text_color(theme::text_faint()),
        )
        .into_element()
}

/// The line above a group's members: what it is called, how it picks, how many it holds and
/// which one it is on. The current member belongs here rather than only among the rows below,
/// because on a phone the members scroll and the group's answer is the one fact worth keeping.
fn group_header(
    tag: String,
    kind: ProxyGroupKind,
    selected_label: String,
    members: usize,
) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_5()
        .py_3()
        .bg(move || base_theme().muted)
        .child(
            controls::icon(
                if kind == ProxyGroupKind::UrlTest {
                    Icon::Gauge
                } else {
                    Icon::Layers
                },
                16.0,
            )
            .text_color(move || base_theme().muted_foreground),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(SharedString::from(tag))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(SharedString::from(i18n::fill(
                        "{} · {} 个成员 · 当前 {}",
                        &[
                            // Both come from `domain` already in words, so they are looked up
                            // here. A node's own name has no row and keeps its own characters;
                            // the engine's `direct` and `block` do have rows, and are the only
                            // values this slot holds that are the app's vocabulary.
                            &i18n::tr(domain::proxy_group_kind_label(kind)),
                            &members,
                            &i18n::tr(&selected_label),
                        ],
                    )))
                    .text_xs()
                    .number_of_lines(1)
                    .text_color(theme::text_faint()),
                ),
        )
}

/// A subscription node inside a group. Everything the row shows and does is read through the
/// memo, so the cell a recycled row holds is the row being drawn rather than the one built.
fn node_row(store: AppStore, row: Memo<Option<ListRow>>) -> impl IntoElement {
    // Its own memo over the node, so the row's three bindings share one cached lookup and the
    // closure handed to the mark stays `Copy`.
    let node = computed(move || {
        row.with(|row| match row {
            Some(ListRow::Node { node, .. }) => Some(node.clone()),
            _ => None,
        })
    });
    button()
        .selected(move || row.with(|row| row.as_ref().is_some_and(ListRow::is_current)))
        // An automatic group picks its own member by measuring it, so its rows are listed for
        // reading and parked for pressing: a selection the engine overrules within its check
        // interval is a claim the app cannot keep.
        .disabled(move || {
            !row.with(|row| row.as_ref().is_some_and(ListRow::is_selectable))
                || store.core_action_busy()
        })
        .on_press(move |_: &()| {
            if let Some((group, tag)) = row.with(|row| row.as_ref().and_then(ListRow::press_target))
            {
                store.select_group_outbound(group, tag);
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
        .disabled_style(|s| s.opacity(0.6))
        .child(protocol_mark(node))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(move || {
                        SharedString::from(
                            node.get().map(|node| node.name.clone()).unwrap_or_default(),
                        )
                    })
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .number_of_lines(1)
                    .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || {
                        SharedString::from(
                            node.get()
                                .map(|node| format!("{}:{}", node.server, node.port))
                                .unwrap_or_default(),
                        )
                    })
                    .text_xs()
                    .font_family(theme::font_mono())
                    .number_of_lines(1)
                    .text_color(theme::text_faint()),
                ),
        )
        .child(node_latency(store, node))
        .child(when(
            move || row.with(|row| row.as_ref().is_some_and(ListRow::is_current)),
            || controls::icon(Icon::Check, 16.0).text_color(move || base_theme().accent_foreground),
        ))
}

/// A member that is not a node: `direct`, `block`, or another group. Named, tappable where the
/// group allows a choice, and honest about having no measurement to show.
fn outbound_row(store: AppStore, row: Memo<Option<ListRow>>) -> impl IntoElement {
    let label = move || {
        row.with(|row| match row {
            Some(ListRow::Outbound {
                tag, kind_label, ..
            }) => Some((tag.clone(), *kind_label)),
            _ => None,
        })
    };
    button()
        .selected(move || row.with(|row| row.as_ref().is_some_and(ListRow::is_current)))
        .disabled(move || {
            !row.with(|row| row.as_ref().is_some_and(ListRow::is_selectable))
                || store.core_action_busy()
        })
        .on_press(move |_: &()| {
            if let Some((group, tag)) = row.with(|row| row.as_ref().and_then(ListRow::press_target))
            {
                store.select_group_outbound(group, tag);
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
        .disabled_style(|s| s.opacity(0.6))
        .child(outbound_mark(label))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    // The tag is the engine's name for the outbound and is not the app's
                    // vocabulary, so it is drawn as it is; the kind beside it is.
                    text(move || {
                        SharedString::from(label().map(|(tag, _)| tag).unwrap_or_default())
                    })
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .number_of_lines(1)
                    .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || {
                        SharedString::from(
                            label().map(|(_, kind)| i18n::tr(kind)).unwrap_or_default(),
                        )
                    })
                    .text_xs()
                    .number_of_lines(1)
                    .text_color(theme::text_faint()),
                ),
        )
        .child(when(
            move || row.with(|row| row.as_ref().is_some_and(ListRow::is_current)),
            || controls::icon(Icon::Check, 16.0).text_color(move || base_theme().accent_foreground),
        ))
}

/// The mark for a member with no protocol: the same tile a node gets, with the glyph that says
/// what the outbound is, so the two kinds of row line up in one column.
fn outbound_mark(label: impl Fn() -> Option<(String, &'static str)> + 'static) -> impl IntoElement {
    view()
        .h(px(32.0))
        .w(px(32.0))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(theme::radius_tile())
        .bg(move || base_theme().muted)
        .child(
            controls::icon(
                match label().map(|(_, kind)| kind).unwrap_or("内置出口") {
                    "直连" => Icon::Wifi,
                    "拦截" => Icon::ShieldCheck,
                    "手动选择" | "自动测速" => Icon::Layers,
                    _ => Icon::Route,
                },
                16.0,
            )
            .text_color(move || base_theme().muted_foreground),
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

/// The protocol as a two-letter mark, colored by family so a list of mixed protocols
/// groups itself without the user reading a single label.
fn protocol_mark(node: Memo<Option<Rc<proxy_core::ProxyNode>>>) -> impl IntoElement {
    view()
        .h(px(32.0))
        .w(px(32.0))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(theme::radius_tile())
        .bg(move || {
            node.get()
                .map(|node| domain::protocol_tint(node.protocol).wash())
                .unwrap_or_else(|| base_theme().muted)
        })
        .child(
            text(move || {
                node.get()
                    .map(|node| domain::protocol_abbreviation(node.protocol).to_owned())
                    .unwrap_or_else(|| "--".to_owned())
            })
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .font_family(theme::font_mono())
            .text_color(move || {
                node.get()
                    .map(|node| domain::protocol_tint(node.protocol).foreground())
                    .unwrap_or_else(|| base_theme().muted_foreground)
            }),
        )
}

fn node_latency(
    store: AppStore,
    node: Memo<Option<Rc<proxy_core::ProxyNode>>>,
) -> impl IntoElement {
    text(move || {
        node.get()
            .and_then(|node| store.latency_for(&node.tag))
            .map(|result| domain::format_latency(result.latency_ms, result.error.as_deref()).0)
            .unwrap_or_else(|| "—".to_owned())
    })
    .text_xs()
    .font_family(theme::font_mono())
    .flex_shrink_0()
    .text_color(theme::text_soft())
}
