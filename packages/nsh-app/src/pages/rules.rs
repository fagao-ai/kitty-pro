//! Rules: what the core does with a connection before it decides where it goes, and the
//! per-domain DNS that resolves it first.
//!
//! Two subjects share a page because a user reaches for them together: a rule that sends a
//! domain through the proxy only works if that domain resolved in the first place. They stay
//! on separate tabs because the fields differ, and one scroll of mixed rows would show
//! neither cleanly.
//!
//! Nothing here restarts the core on its own. Rules reach the engine only through a fresh
//! config, so each edit says that plainly rather than implying the rule is live.

use crate::domain;
use crate::i18n;
use crate::store::AppStore;
use crate::ui::{controls, theme, Icon};
use nsh_base::{button, dialog, theme as base_theme, SelectItem};
use nsh_core::prelude::*;
use proxy_core::{CustomRule, CustomRuleAction, CustomRuleMatch, DnsOverride};
use std::time::Duration;

/// How long a delete stays armed before it needs pressing again. Long enough to aim twice,
/// short enough that a row left saying "确认删除" is not read as an existing state.
const CONFIRM_SECONDS: u64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Routing,
    Dns,
}

pub fn rules(store: AppStore) -> impl IntoElement {
    let tab = signal(Tab::Routing);
    // Both drafts live here rather than in their tab's function because the page has one
    // floating action, and it has to open whichever form the visible tab owns.
    let draft = rule_draft();
    let dns_draft = dns_draft();
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(
            // The measure column keeps `flex_1`/`min_h` from the page: the rule list below it is
            // virtualized, and a scroll view given unbounded or zero height lays out no rows.
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
                // Room for the page's own floating "+", which sits over this column's
                // bottom-right corner; the rule list is bounded by the column, so this lifts
                // its last row clear of the disc.
                .pb(controls::action_clearance())
                .child(tab_bar(tab))
                .child(
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(when(
                            move || tab.get() == Tab::Routing,
                            move || routing(store, draft),
                        ))
                        .child(when(
                            move || tab.get() == Tab::Dns,
                            move || dns(store, dns_draft),
                        )),
                ),
        )
        .child(controls::floating_layer(add_rule_action(
            store, tab, draft, dns_draft,
        )))
}

/// 规则's one action: open the current tab's form, or put it away again.
///
/// Both tabs used to carry a permanently visible form, which made each read as a form with
/// some rows under it; the user asked for it to appear when the + is pressed. Pressing again
/// closes it and clears what was typed — half-finished text is discarded, but the control that
/// discarded it is the same one that opened it, and its glyph changed to say so. `编辑` on a
/// rule row opens that form too, filled.
///
/// One disc for two tabs means its name follows the tab: a button called 添加规则 while the DNS
/// list is showing would be a lie, and the tests press by that name.
fn add_rule_action(
    store: AppStore,
    tab: Signal<Tab>,
    draft: RuleDraft,
    dns_draft: DnsDraft,
) -> impl IntoElement {
    let on_routing = move || tab.get() == Tab::Routing;
    let open = move || {
        if on_routing() {
            draft.open.get()
        } else {
            dns_draft.open.get()
        }
    };
    // The closure captures only `Copy` handles, so every binding below gets its own copy.
    let open_press = open;
    controls::floating_action_state(
        move || {
            if open() {
                Icon::X
            } else {
                Icon::Plus
            }
        },
        move || match (on_routing(), open()) {
            (true, false) => "添加规则".into(),
            (true, true) => "收起规则表单".into(),
            (false, false) => "添加专用 DNS".into(),
            (false, true) => "收起专用 DNS 表单".into(),
        },
        // A form action: nothing about it is a running state, so it stays neutral and lets the
        // one disc that means "live" mean it.
        || false,
        // The override replaces the whole rule set, so custom rules cannot be edited at all.
        move || on_routing() && store.config_script_active(),
        move || {
            if on_routing() {
                // Read before `reset`, which closes the form along with clearing it.
                let was_open = open_press();
                reset(draft, store);
                draft.open.set(!was_open);
            } else {
                let was_open = open_press();
                reset_dns(dns_draft);
                dns_draft.open.set(!was_open);
            }
        },
    )
}

/// Two subjects, so a segmented pair rather than a list of pills: the choice is exclusive
/// and both names are always visible.
fn tab_bar(tab: Signal<Tab>) -> impl IntoElement {
    let segment = |label: &'static str, value: Tab| {
        let selected = move || tab.get() == value;
        nsh_base::tab()
            .selected(selected)
            .on_press(move || tab.set(value))
            .h(px(30.0))
            .px_3()
            .items_center()
            .justify_center()
            .rounded(theme::radius_tile())
            .selected_style(|s| s.bg(move || base_theme().muted))
            .hover(|s| s.bg(move || base_theme().accent))
            .focus(|s| s.outline(px(2.0)).outline_color(move || base_theme().ring))
            .child(
                text(move || i18n::tr(label))
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if selected() {
                            base_theme().foreground
                        } else {
                            base_theme().muted_foreground
                        }
                    }),
            )
    };
    nsh_base::tabs()
        .flex_row()
        .gap_1()
        .child(segment("分流规则", Tab::Routing))
        .child(segment("专用 DNS", Tab::Dns))
}

/// Routing rules, their editor, and the two conditions that make them inert.
///
/// The draft comes from the page, which shares it with the floating "+".
fn routing(store: AppStore, draft: RuleDraft) -> impl IntoElement {
    let armed = signal(None::<u64>);
    let arm_generation = signal(0u64);
    let picker_open = signal(false);
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .gap_3()
        .child(when(picker_open, move || process_picker(store, draft, picker_open)))
        .child(summary_row(store, Tab::Routing))
        .child(when(
            move || store.config_script_active(),
            move || {
                controls::inline_note(
                    theme::Tint::Warning,
                    Icon::CircleAlert,
                    "JavaScript 配置覆写已启用，自定义分流规则暂不可用。",
                )
            },
        ))
        .child(when(
            move || hidden_process_rules(store) > 0,
            move || {
                controls::inline_note(
                    theme::Tint::Info,
                    Icon::Info,
                    move || i18n::fill("另有 {} 条进程规则在当前平台不可见，但仍计入规则总数并随配置同步。", &[&(hidden_process_rules(store))]),
                )
            },
        ))
        .child(when(
            // The note's whole premise is the desktop's choice between TUN and the system
            // proxy. A phone has no such choice — its tunnel is TUN and always is — so the
            // sentence would tell a reader to turn on a switch that no longer exists, about a
            // mode that cannot be reached. The row above already says the true thing on a
            // phone: that process rules are held but not evaluated here.
            move || {
                !domain::is_phone()
                    && store.custom_rules().iter().any(process_rule)
                    && !store.tun_enabled()
            },
            move || {
                controls::inline_note(
                    theme::Tint::Info,
                    Icon::Info,
                    "进程分流需开启 TUN 才能覆盖所有进程；仅系统代理模式下，进程规则只作用于走代理的进程。",
                )
            },
        ))
        // The form is hidden until the floating + opens it, so the page reads as a list with
        // one action rather than as a form with some rows under it.
        .child(when(draft.open, move || {
            editor(store, draft, picker_open)
        }))
        .child(rule_list(store, draft, armed, arm_generation))
}

/// The counts that describe the tab, with the one action that refreshes external rule-sets.
fn summary_row(store: AppStore, tab: Tab) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            // `tr` outside the match, not inside it: the closure returns a *key* either way, and
            // a closure that computes a key without looking it up is the version of the frozen-
            // label bug that still re-runs on every poll.
            text(move || {
                let key = match tab {
                    Tab::Routing => routing_summary(store),
                    Tab::Dns => "命中域名用专用 DNS 解析并直连，任何分流模式都生效".to_owned(),
                };
                i18n::tr(&key)
            })
            .text_xs()
            .text_color(theme::text_faint()),
        )
        .child(when(
            move || tab == Tab::Routing,
            move || update_rules_button(store),
        ))
}

fn routing_summary(store: AppStore) -> String {
    let rules = store.custom_rules();
    let enabled = rules.iter().filter(|rule| rule.enabled).count();
    i18n::fill("{} 条启用 · {} 条规则", &[&enabled, &rules.len()])
}

/// The rule-set refresh. Its running flag lives in the store rather than in a signal here,
/// because the work outlives the screen: a refresh started and then navigated away from has to
/// finish and report, and a button that vanished with its own progress flag would leave the
/// second press starting a second download.
fn update_rules_button(store: AppStore) -> impl IntoElement {
    controls::secondary_button(
        move || {
            if store.rule_sets_busy() {
                "正在更新"
            } else {
                "更新规则"
            }
        },
        move || store.update_rule_sets(),
    )
}

/// What the editor holds. Kept as signals owned by the page rather than the store: a draft
/// is not state a relaunch should restore, and only this screen reads it.
///
/// `Copy` because it is only handles: the list's rows and the editor both need it, and a row
/// built once per cell captures it the way it captures a signal.
#[derive(Clone, Copy)]
struct RuleDraft {
    editing: Signal<Option<u64>>,
    /// Whether the form is on screen at all. It used to be permanently open, which put three
    /// fields and an 添加 button above every list and made the page read as a form with some
    /// rows under it; the user asked for the form to appear when the floating + is pressed.
    /// `编辑` on a row opens it too, and `reset` closes it, so nothing can leave it showing an
    /// empty draft behind a list.
    open: Signal<bool>,
    match_type: Signal<SharedString>,
    action: Signal<SharedString>,
    value: Signal<String>,
    error: Signal<Option<String>>,
}

fn rule_draft() -> RuleDraft {
    RuleDraft {
        editing: signal(None),
        open: signal(false),
        match_type: signal(match_value(CustomRuleMatch::DomainSuffix)),
        action: signal(action_value(CustomRuleAction::Proxy)),
        value: signal(String::new()),
        error: signal(None),
    }
}

/// The DNS tab's half of the same thing: its two fields, its error, and whether the form is on
/// screen. Owned by the page for the same reason as `RuleDraft` — the floating disc is one
/// control shared by both tabs, so it cannot reach into a tab's own locals.
#[derive(Clone, Copy)]
struct DnsDraft {
    open: Signal<bool>,
    /// The kind as a `SelectItem` value, so the picker is the same control the rule form uses.
    /// Read through `parse_match` when the form is submitted.
    kind: Signal<SharedString>,
    domain: Signal<String>,
    server: Signal<String>,
    error: Signal<Option<String>>,
}

fn dns_draft() -> DnsDraft {
    DnsDraft {
        open: signal(false),
        kind: signal(match_value(CustomRuleMatch::DomainSuffix)),
        domain: signal(String::new()),
        server: signal(String::new()),
        error: signal(None),
    }
}

/// Clear the DNS form and close it, the way `reset` does for a rule.
///
/// The kind goes back to a suffix as well. Leaving it on `domain_keyword` after an add would mean
/// the next rule opened silently narrower or wider than the last, which is the one thing a picker
/// that defaults should not do.
fn reset_dns(draft: DnsDraft) {
    draft.open.set(false);
    draft.kind.set(match_value(CustomRuleMatch::DomainSuffix));
    draft.domain.set(String::new());
    draft.server.set(String::new());
    draft.error.set(None);
}

fn editor(store: AppStore, draft: RuleDraft, picker_open: Signal<bool>) -> impl IntoElement {
    let (match_type, action, value, error) =
        (draft.match_type, draft.action, draft.value, draft.error);
    let editing = draft.editing;
    controls::card(
        view()
            .flex_col()
            .gap_3()
            .child(
                text(move || {
                    i18n::tr(if editing.get().is_some() {
                        "编辑规则"
                    } else {
                        "添加规则"
                    })
                })
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || base_theme().card_foreground),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_end()
                    .gap_3()
                    .child(
                        view()
                            .w(px(150.0))
                            .flex_shrink_0()
                            .child(controls::choice_field(
                                "匹配类型",
                                match_type,
                                match_options,
                                move |next| {
                                    match_type.set(next);
                                    error.set(None);
                                },
                            )),
                    )
                    .child(value_field(store, draft, picker_open))
                    .child(
                        view()
                            .w(px(120.0))
                            .flex_shrink_0()
                            .child(controls::choice_field(
                                "动作",
                                action,
                                action_options,
                                move |next| {
                                    action.set(next);
                                    error.set(None);
                                },
                            )),
                    ),
            )
            .child(controls::field_error(error))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        text(move || i18n::tr("此处指单条规则的匹配动作，与全局分流模式不同"))
                            .text_xs()
                            .text_color(theme::text_faint()),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(when(
                                move || editing.get().is_some(),
                                move || {
                                    controls::secondary_button("取消", move || {
                                        reset(draft, store)
                                    })
                                },
                            ))
                            .child(controls::primary_button_state(
                                move || {
                                    if editing.get().is_some() {
                                        "保存"
                                    } else {
                                        "添加"
                                    }
                                },
                                // Nothing to match on yet. The select beside it always holds
                                // a value, so an empty content field is the one thing that
                                // makes this press do nothing.
                                move || value.get().trim().is_empty(),
                                move || commit(store, draft),
                            )),
                    ),
            ),
    )
}

fn commit(store: AppStore, draft: RuleDraft) {
    let result = store.save_custom_rule(
        draft.editing.get_untracked(),
        parse_match(draft.match_type.get_untracked()),
        parse_action(draft.action.get_untracked()),
        &draft.value.get_untracked(),
    );
    match result {
        Ok(()) => reset(draft, store),
        Err(error) => draft.error.set(Some(error)),
    }
}

/// Clear the form, and put back the values of the rule that was being edited so cancelling
/// an edit cannot leave the editor showing half of a draft.
fn reset(draft: RuleDraft, store: AppStore) {
    let editing = draft.editing.get_untracked();
    let restored =
        editing.and_then(|id| store.custom_rules().into_iter().find(|rule| rule.id == id));
    draft.editing.set(None);
    // Closing is part of clearing: a form left open with nothing in it is the state the page
    // used to start in, which is what the user objected to.
    draft.open.set(false);
    draft.value.set(String::new());
    draft.error.set(None);
    draft.match_type.set(match_value(
        restored
            .as_ref()
            .map_or(CustomRuleMatch::DomainSuffix, |rule| rule.match_type),
    ));
    draft.action.set(action_value(
        restored
            .as_ref()
            .map_or(CustomRuleAction::Proxy, |rule| rule.action),
    ));
}

/// Fill the draft from a stored rule so pressing 编辑 on a row visibly moves the form.
fn fill(draft: RuleDraft, rule: CustomRule) {
    draft.editing.set(Some(rule.id));
    // Editing opens the form the + owns: a row that says 编辑 and reveals nothing would be the
    // same bug as the always-open form's opposite.
    draft.open.set(true);
    draft.match_type.set(match_value(rule.match_type));
    draft.action.set(action_value(rule.action));
    draft.value.set(rule.value);
    draft.error.set(None);
}

fn rule_list(
    store: AppStore,
    draft: RuleDraft,
    armed: Signal<Option<u64>>,
    arm_generation: Signal<u64>,
) -> impl IntoElement {
    let supported = domain::process_rules_supported();
    // The empty state is one of the list's own cells: drawn only while its count is zero,
    // and inside the same frame as the rows it replaces.
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(
            virtual_list(
                move || visible_rule_ids(store, supported).len(),
                move |at| {
                    let rule = computed(move || {
                        let id = visible_rule_ids(store, supported)
                            .get(at.index())
                            .copied()?;
                        store.custom_rules().into_iter().find(|item| item.id == id)
                    });
                    let position = computed(move || {
                        let id = rule.with(|rule| rule.as_ref().map(|rule| rule.id));
                        visible_rule_ids(store, supported)
                            .iter()
                            .position(|item| Some(*item) == id)
                            .unwrap_or(0)
                    });
                    rule_row(store, draft, armed, arm_generation, rule, position)
                },
            )
            .keys(move || {
                visible_rule_ids(store, supported)
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
            })
            .estimated_item_size(px(76.0))
            .separator(|| {
                view()
                    .h(px(1.0))
                    .mx_5()
                    .bg(move || base_theme().border)
                    .into_element()
            })
            .empty(controls::empty_state(
                "暂无自定义规则",
                "添加一条规则，为指定域名或网段固定走向。",
            ))
            .flex_1()
            .min_h(px(0.0)),
        )
}

/// The rules in the order the user sees them, process rules dropped where the OS cannot
/// report them. Ids rather than rules, so the row's memo can find the current value after an
/// edit instead of holding a stale copy.
fn visible_rule_ids(store: AppStore, supported: bool) -> Vec<u64> {
    store
        .custom_rules()
        .into_iter()
        .filter(|rule| domain::custom_rule_available(rule.match_type, supported))
        .map(|rule| rule.id)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn rule_row(
    store: AppStore,
    draft: RuleDraft,
    armed: Signal<Option<u64>>,
    arm_generation: Signal<u64>,
    rule: Memo<Option<CustomRule>>,
    position: Memo<usize>,
) -> impl IntoElement {
    let id = move || rule.with(|rule| rule.as_ref().map(|rule| rule.id));
    let armed_here = move || id().is_some() && armed.get() == id();
    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_5()
        .py_3()
        .child(
            text(move || (position.get() + 1).to_string())
                .text_xs()
                .font_family(theme::font_mono())
                .w(px(24.0))
                .flex_shrink_0()
                .text_color(theme::text_faint()),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_0p5()
                .child(
                    text(move || {
                        rule.with(|rule| {
                            rule.as_ref()
                                .map(|rule| rule.value.clone())
                                .unwrap_or_default()
                        })
                    })
                    .text_sm()
                    .font_family(theme::font_mono())
                    .number_of_lines(1)
                    .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || {
                        rule.with(|rule| {
                            rule.as_ref()
                                .map(|rule| {
                                    i18n::tr(domain::custom_rule_match_label(rule.match_type))
                                })
                                .unwrap_or_default()
                        })
                    })
                    .text_xs()
                    .text_color(theme::text_faint()),
                ),
        )
        .child(controls::switch_state(
            move || {
                let value = rule.with(|rule| {
                    rule.as_ref()
                        .map(|rule| rule.value.clone())
                        .unwrap_or_default()
                });
                Some(i18n::fill("启用规则 {}", &[&value]).into())
            },
            move || rule.with(|rule| rule.as_ref().is_some_and(|rule| rule.enabled)),
            || false,
            move |_: bool| {
                if let Some(id) = id() {
                    store.toggle_custom_rule(id);
                }
            },
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_shrink_0()
                .child(controls::badge(
                    move || {
                        rule.with(|rule| {
                            domain::custom_rule_action_label(
                                rule.as_ref()
                                    .map_or(CustomRuleAction::Proxy, |rule| rule.action),
                            )
                            .to_owned()
                        })
                    },
                    move || {
                        rule.with(|rule| {
                            domain::custom_rule_action_tint(
                                rule.as_ref()
                                    .map_or(CustomRuleAction::Proxy, |rule| rule.action),
                            )
                        })
                    },
                ))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(controls::icon_button(
                            || Icon::ArrowUpRight,
                            move || "上移".into(),
                            move || {
                                if let Some(id) = id() {
                                    store.move_custom_rule(id, -1);
                                }
                            },
                        ))
                        .child(controls::icon_button(
                            || Icon::ChevronDown,
                            move || "下移".into(),
                            move || {
                                if let Some(id) = id() {
                                    store.move_custom_rule(id, 1);
                                }
                            },
                        )),
                )
                .child(controls::icon_button(
                    move || Icon::Pencil,
                    move || "编辑这条规则".into(),
                    move || {
                        if let Some(rule) = rule.get_untracked() {
                            fill(draft, rule);
                        }
                    },
                ))
                .child(controls::icon_button(
                    move || Icon::Trash2,
                    move || {
                        if armed_here() {
                            "再次点击确认删除这条规则".into()
                        } else {
                            "删除这条规则".into()
                        }
                    },
                    move || {
                        let Some(id) = id() else { return };
                        if armed.get_untracked() == Some(id) {
                            arm_generation.update(|n| *n = n.wrapping_add(1));
                            armed.set(None);
                            store.delete_custom_rule(id);
                            return;
                        }
                        // Any other press moves the arm here, so a second row's trash can
                        // never fire on the first row's arming.
                        let generation = arm_generation.get_untracked().wrapping_add(1);
                        arm_generation.set(generation);
                        armed.set(Some(id));
                        spawn(async move {
                            timer(Duration::from_secs(CONFIRM_SECONDS)).await;
                            if arm_generation.get_untracked() == generation {
                                armed.set(None);
                            }
                        });
                    },
                )),
        )
}

/// DNS overrides: which names, resolved by which server, and nothing else, so the rows stay one
/// line.
///
/// The kind picker is the routing rule form's own control with three of its six options. The user
/// asked for exactly that list and excluded the two process kinds; IP CIDR is excluded for a
/// reason that is not a preference — see [`dns_kind_options`].
fn dns(store: AppStore, draft: DnsDraft) -> impl IntoElement {
    let (kind_field, domain_field, server_field, error) =
        (draft.kind, draft.domain, draft.server, draft.error);
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .gap_3()
        .child(summary_row(store, Tab::Dns))
        // Hidden until the page's disc opens it, exactly as the rule form is: two fields and an
        // 添加 button above the list made this tab read as a form with some rows under it.
        .child(when(draft.open, move || {
            controls::card(
                view()
                    .flex_col()
                    .gap_3()
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_end()
                            .gap_3()
                            .child(view().w(px(150.0)).flex_shrink_0().child(
                                controls::choice_field(
                                    "匹配类型",
                                    kind_field,
                                    dns_kind_options,
                                    move |next| {
                                        kind_field.set(next);
                                        error.set(None);
                                    },
                                ),
                            ))
                            .child(view().flex_1().min_w(px(200.0)).child(controls::text_field(
                                // "Match" rather than the chosen kind's own name: `text_field`
                                // takes its label as a `&'static str` read once at build time, so
                                // naming the kind here would leave the field saying 域名后缀 after
                                // the picker moved to 域名关键字 — the exact frozen-label mistake
                                // the whole app has to avoid. The routing form above makes the
                                // same choice, and the placeholder *is* reactive, so the example
                                // under the cursor does follow the kind.
                                "匹配内容",
                                domain_field,
                                move || placeholder_for(kind_field.get()),
                            )))
                            .child(view().flex_1().min_w(px(200.0)).child(controls::text_field(
                                "DNS 服务器",
                                server_field,
                                || "223.5.5.5",
                            )))
                            .child(controls::primary_button_state(
                                "添加",
                                move || {
                                    domain_field.get().trim().is_empty()
                                        || server_field.get().trim().is_empty()
                                },
                                {
                                    let (store, draft) = (store, draft);
                                    move || {
                                        let result = store.add_dns_override(
                                            parse_match(draft.kind.get_untracked()),
                                            &draft.domain.get_untracked(),
                                            &draft.server.get_untracked(),
                                        );
                                        match result {
                                            // Saved: the form has done its job and steps aside,
                                            // the same way the rule editor does.
                                            Ok(()) => reset_dns(draft),
                                            // Refused: the text stays, because the reader has a
                                            // mistake in front of them to correct.
                                            Err(message) => draft.error.set(Some(message)),
                                        }
                                    }
                                },
                            )),
                    )
                    .child(controls::field_error(error)),
            )
        }))
        .child(dns_list(store))
}

fn dns_list(store: AppStore) -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
        .child(
            virtual_list(
                move || store.dns_overrides().len(),
                move |at| {
                    let item = computed(move || store.dns_overrides().get(at.index()).cloned());
                    // The row number is the index in the list, read at draw time exactly as the
                    // routing list does it: a virtualized cell is built once and shown for many
                    // rows, so the position cannot be baked in when the row was made.
                    let position = computed(move || at.index());
                    dns_row(store, item, position)
                },
            )
            .keys(move || {
                store
                    .dns_overrides()
                    .into_iter()
                    .map(|item| item.id.to_string())
                    .collect::<Vec<_>>()
            })
            .estimated_item_size(px(64.0))
            .separator(|| {
                view()
                    .h(px(1.0))
                    .mx_5()
                    .bg(move || base_theme().border)
                    .into_element()
            })
            .empty(controls::empty_state(
                "尚未配置专用 DNS",
                "为指定域名固定 DNS，避免公共 DNS 解析到错误地址。",
            ))
            .flex_1()
            .min_h(px(0.0)),
        )
}

/// One DNS override, drawn with the routing list's anatomy: number, value over its kind, the
/// switch, a badge stating what the row does, then delete.
///
/// The user asked for the two lists to read as one thing, and they are one thing — a rule that
/// matches and an effect — so the same column order, sizes and type styles are used rather than a
/// DNS-flavoured layout. What each slot carries follows the routing row: line 1 is the value,
/// line 2 is the kind (the routing row's `IP CIDR` / `域名后缀`), and the badge is the effect —
/// `Direct` there, the resolver here. The kind is always shown now, not only when it is unusual,
/// because a list where one row has a second line and the next does not reads as a row that is
/// missing something.
fn dns_row(
    store: AppStore,
    item: Memo<Option<DnsOverride>>,
    position: Memo<usize>,
) -> impl IntoElement {
    let id = move || item.with(|item| item.as_ref().map(|item| item.id));
    let value = move || {
        item.with(|item| {
            item.as_ref()
                .map(|item| item.domain_suffix.clone())
                .unwrap_or_default()
        })
    };
    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_5()
        .py_3()
        .child(
            text(move || (position.get() + 1).to_string())
                .text_xs()
                .font_family(theme::font_mono())
                .w(px(24.0))
                .flex_shrink_0()
                .text_color(theme::text_faint()),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_0p5()
                .child(
                    text(value)
                        .text_sm()
                        .font_family(theme::font_mono())
                        .number_of_lines(1)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || {
                        item.with(|item| {
                            item.as_ref()
                                .map(|item| {
                                    i18n::tr(domain::custom_rule_match_label(item.match_type))
                                })
                                .unwrap_or_default()
                        })
                    })
                    .text_xs()
                    .text_color(theme::text_faint()),
                ),
        )
        .child(controls::switch_state(
            move || Some(i18n::fill("启用 {} 的专用 DNS", &[&(value())]).into()),
            move || item.with(|item| item.as_ref().is_some_and(|item| item.enabled)),
            || false,
            move |_: bool| {
                if let Some(id) = id() {
                    store.toggle_dns_override(id);
                }
            },
        ))
        .child(
            // Wrapped exactly as the routing row wraps its action badge, and the wrapper is the
            // whole reason the pill sits level here. `controls::badge` carries `.self_start()`,
            // which is load-bearing where a badge is a child of a *column* — the log row gives one
            // a fixed 52pt width and the pill must not stretch to fill it — and reads as a bug
            // anywhere else: inside a row it overrides that row's `items_center` and pins the pill
            // to the top edge, so a two-line row shows it floating. NSH has no `items_start` to
            // undo it from the parent, so a caller that wants a centred badge supplies a wrapper the
            // height of one line, which is what the routing row has always done.
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_shrink_0()
                .child(controls::badge(
                    // The resolver, in the slot the routing row spends on its action: both answer
                    // "and then what?". Not translated — an address is data, not the app's
                    // vocabulary.
                    move || {
                        item.with(|item| {
                            item.as_ref()
                                .map(|item| item.server.clone())
                                .unwrap_or_default()
                        })
                    },
                    || theme::Tint::Dns,
                )),
        )
        .child(controls::icon_button(
            move || Icon::Trash2,
            // One press, as this row always was. The routing row arms because a rule sits in an
            // ordered list and deleting one silently reshuffles the rest; a DNS override is an
            // entry in a set, and the user asked for this list to match in *style*, not to gain a
            // confirmation step it never had.
            move || i18n::fill("删除 {} 的专用 DNS", &[&(value())]).into(),
            move || {
                if let Some(id) = id() {
                    store.delete_dns_override(id);
                }
            },
        ))
}

/// Whether this row is a process rule, for the note that only applies when one exists.
fn process_rule(rule: &CustomRule) -> bool {
    matches!(
        rule.match_type,
        CustomRuleMatch::ProcessName | CustomRuleMatch::ProcessPath
    )
}

fn hidden_process_rules(store: AppStore) -> usize {
    if domain::process_rules_supported() {
        return 0;
    }
    store
        .custom_rules()
        .iter()
        .filter(|rule| process_rule(rule))
        .count()
}

/// The rule's value: a typed field, plus the picker for the two kinds that name a process.
///
/// A process name is the one value on this form a user cannot invent reliably — it is spelled
/// whatever the binary is spelled, and a rule written as `Telegram.app` silently matches
/// nothing. Where the OS can say what is running, the field gets the button that reads the
/// answer off the machine; where it cannot, the field stays as it was and the option is not
/// offered at all.
fn value_field(store: AppStore, draft: RuleDraft, picker: Signal<bool>) -> impl IntoElement {
    let match_type = draft.match_type;
    view()
        .flex_1()
        .min_w(px(220.0))
        .flex_col()
        .gap_2()
        .child(controls::text_field(
            "匹配内容",
            draft.value,
            move || placeholder_for(match_type.get()),
        ))
        .child(when(
            move || process_pick_offered(parse_match(match_type.get())),
            move || {
                controls::secondary_button(
                    move || {
                        if store.processes_busy() {
                            "正在读取"
                        } else {
                            "选择进程"
                        }
                    },
                    move || {
                        if store.processes_busy() {
                            return;
                        }
                        store.refresh_processes();
                        picker.set(true);
                    },
                )
            },
        ))
}

/// Whether this match kind names something the operating system can enumerate.
fn process_pick_offered(match_type: CustomRuleMatch) -> bool {
    domain::process_rules_supported()
        && matches!(
            match_type,
            CustomRuleMatch::ProcessName | CustomRuleMatch::ProcessPath
        )
}

/// The process picker: a searchable list of what is running, where one press fills the field.
///
/// A dialog rather than an inline dropdown because the list is long — a working machine runs a
/// couple of hundred processes — and because it is a tool for finishing one field rather than a
/// subject of its own. It mounts as a sibling of the list rather than inside the editor card,
/// which is the same reason the original keeps it out of the card: a card that scrolls takes a
/// centred overlay out of reach with it.
fn process_picker(store: AppStore, draft: RuleDraft, picker: Signal<bool>) -> impl IntoElement {
    let query = signal(String::new());
    // The close handler captures only a `Copy` signal, so the same one goes to the sheet's
    // cancel, its list rows and its button without any cloning.
    let close = move || picker.set(false);
    dialog()
        .presentation(ModalPresentation::PageSheet)
        .on_cancel(close)
        .flex_1()
        .items_center()
        .justify_center()
        .bg(move || base_theme().background.opacity(0.6))
        .child(
            view()
                .w_full()
                .max_w(px(560.0))
                .flex_col()
                .gap_3()
                .m_5()
                .p_5()
                .rounded(theme::radius_card())
                .border_1()
                .border_color(move || base_theme().border)
                .bg(move || base_theme().card)
                .shadow_lg()
                .child(
                    text(move || i18n::tr("选择进程"))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(controls::search_field(
                    "搜索进程名或路径",
                    "搜索进程名或路径…",
                    query,
                ))
                .child(picker_body(store, draft, query, close))
                .child(controls::secondary_button("取消", close)),
        )
}

/// The picker's middle: loading, failed, empty, or the list. Exactly one of four, because an
/// empty list alone cannot tell a quiet machine from a failed read.
fn picker_body(
    store: AppStore,
    draft: RuleDraft,
    query: Signal<String>,
    close: impl Fn() + Clone + 'static,
) -> impl IntoElement {
    view().w_full().flex_col().child(dynamic(move || {
        if let Some(error) = store.processes_error() {
            return controls::inline_note(theme::Tint::Danger, Icon::CircleAlert, error)
                .into_element();
        }
        if !store.processes_loaded() {
            return text(move || i18n::tr("正在读取进程列表…"))
                .text_xs()
                .text_color(theme::text_faint())
                .into_element();
        }
        let keyword = domain::search_keyword(&query.get());
        let hits = process_hits(store, &keyword);
        if hits.is_empty() {
            return text(move || i18n::tr("没有匹配的进程"))
                .text_xs()
                .text_color(theme::text_faint())
                .into_element();
        }
        picker_rows(hits, draft, close.clone()).into_element()
    }))
}

/// The list, in a scroll region of its own: a couple of hundred rows would otherwise push the
/// dialog's buttons past the bottom of the window.
fn picker_rows(
    hits: Vec<api::RunningProcess>,
    draft: RuleDraft,
    close: impl Fn() + Clone + 'static,
) -> impl IntoElement {
    scroll()
        .w_full()
        .h(px(280.0))
        .flex_col()
        .gap_1()
        .test_id("process-list")
        .child(each(
            move || hits.clone(),
            |process| process.pid,
            move |item, _| picker_row(item, draft, close.clone()),
        ))
}

fn picker_row(
    item: ReadSignal<api::RunningProcess>,
    draft: RuleDraft,
    close: impl Fn() + 'static,
) -> impl IntoElement {
    let process = move || item.get();
    button()
        .accessibility_label(move || Some(process().name.clone().into()))
        .on_press(move |_: &()| {
            // Which half of the process is copied depends on what the rule matches on: a path
            // rule wants the path, a name rule the name. Reading the draft at press time rather
            // than when the row was built is what keeps a kind changed while the dialog was
            // open from filling the field with the other half.
            let kind = parse_match(draft.match_type.get_untracked());
            let chosen = process();
            draft.value.set(domain::process_rule_value(&chosen, kind));
            // The field now holds something the machine reported, so any earlier complaint
            // about it is stale.
            draft.error.set(None);
            close();
        })
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .px_3()
        .py_2()
        .rounded(theme::radius_tile())
        .hover(|s| s.bg(move || base_theme().accent))
        .active(|s| s.opacity(0.8))
        .focus(|s| s.outline(px(2.0)).outline_color(move || base_theme().ring))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(move || process().name)
                        .text_sm()
                        .number_of_lines(1)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(when(
                    move || {
                        process()
                            .exe_path
                            .as_deref()
                            .is_some_and(|path| !path.is_empty())
                    },
                    move || {
                        text(move || process().exe_path.clone().unwrap_or_default())
                            .text_xs()
                            .font_family(theme::font_mono())
                            .number_of_lines(1)
                            .text_color(theme::text_faint())
                    },
                )),
        )
        .child(
            text(move || format!("PID {}", process().pid))
                .text_xs()
                .font_family(theme::font_mono())
                .flex_shrink_0()
                .text_color(theme::text_faint()),
        )
}

/// The processes the query leaves, in the order the backend reported them (by name).
fn process_hits(store: AppStore, keyword: &str) -> Vec<api::RunningProcess> {
    store
        .processes()
        .into_iter()
        .filter(|process| domain::process_matches(process, keyword))
        .collect()
}

/// The kinds a dedicated-DNS rule can be written with.
///
/// The user asked for the routing form's list without the two process kinds, and that is what this
/// is — minus one more, for a reason that is the core's rather than a preference: a DNS rule whose
/// only condition is an IP CIDR does not scope resolution to that range, it sends *every* forward
/// lookup to that server. A resolver is chosen before an address exists, so sing-box keeps the CIDR
/// for afterwards, as a filter on the answer (`dns/router.go` skips such a rule only for non-address
/// queries, then `DefaultDNSRule::Match` sets `IgnoreDestinationIPCIDRMatch`, which leaves the rule
/// matching everything). The app already makes the same judgement one file over:
/// `custom_direct_dns_rule_to_value` returns `None` for `IpCidr` when turning routing rules into DNS
/// rules. Offering it here would mean a rule that silently breaks DNS, so the picker does not
/// contain it and `numbered_dns_overrides` refuses to write one if a profile says so anyway.
fn dns_kind_options() -> Vec<SelectItem> {
    proxy_core::DNS_OVERRIDE_MATCHES
        .into_iter()
        .map(|kind| {
            SelectItem::new(
                match_value(kind),
                i18n::tr(domain::custom_rule_match_label(kind)),
            )
        })
        .collect()
}

/// The rule types the platform allows. Process rules are left out where the OS cannot report
/// the owning process, which is the only honest way to offer them.
fn match_options() -> Vec<SelectItem> {
    let supported = domain::process_rules_supported();
    [
        CustomRuleMatch::Domain,
        CustomRuleMatch::DomainSuffix,
        CustomRuleMatch::DomainKeyword,
        CustomRuleMatch::IpCidr,
        CustomRuleMatch::ProcessName,
        CustomRuleMatch::ProcessPath,
    ]
    .into_iter()
    .filter(|kind| domain::custom_rule_available(*kind, supported))
    .map(|kind| {
        SelectItem::new(
            match_value(kind),
            i18n::tr(domain::custom_rule_match_label(kind)),
        )
    })
    .collect()
}

fn action_options() -> Vec<SelectItem> {
    [
        CustomRuleAction::Direct,
        CustomRuleAction::Proxy,
        CustomRuleAction::Block,
    ]
    .into_iter()
    .map(|action| {
        SelectItem::new(
            action_value(action),
            i18n::tr(domain::custom_rule_action_label(action)),
        )
    })
    .collect()
}

/// The values the profile file already uses, so editing a rule does not rewrite its stored
/// form.
fn match_value(match_type: CustomRuleMatch) -> SharedString {
    SharedString::from(match_type.singbox_field())
}

fn parse_match(value: SharedString) -> CustomRuleMatch {
    match value.as_str() {
        "domain" => CustomRuleMatch::Domain,
        "domain_keyword" => CustomRuleMatch::DomainKeyword,
        "ip_cidr" => CustomRuleMatch::IpCidr,
        "process_name" => CustomRuleMatch::ProcessName,
        "process_path" => CustomRuleMatch::ProcessPath,
        _ => CustomRuleMatch::DomainSuffix,
    }
}

fn action_value(action: CustomRuleAction) -> SharedString {
    SharedString::from(action.outbound())
}

fn parse_action(value: SharedString) -> CustomRuleAction {
    match value.as_str() {
        "direct" => CustomRuleAction::Direct,
        "block" => CustomRuleAction::Block,
        _ => CustomRuleAction::Proxy,
    }
}

fn placeholder_for(value: SharedString) -> &'static str {
    domain::custom_rule_placeholder(parse_match(value))
}
