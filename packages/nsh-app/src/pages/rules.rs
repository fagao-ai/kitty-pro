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
use crate::store::AppStore;
use crate::ui::{controls, theme, Icon};
use nsh_base::{theme as base_theme, SelectItem};
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
        .pb_2()
        .child(tab_bar(tab))
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .child(when(
                    move || tab.get() == Tab::Routing,
                    move || routing(store),
                ))
                .child(when(move || tab.get() == Tab::Dns, move || dns(store))),
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
                text(label)
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
fn routing(store: AppStore) -> impl IntoElement {
    let draft = rule_draft();
    let armed = signal(None::<u64>);
    let arm_generation = signal(0u64);
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .gap_3()
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
                    move || format!("另有 {} 条进程规则在当前平台不可见，但仍计入规则总数并随配置同步。", hidden_process_rules(store)),
                )
            },
        ))
        .child(when(
            move || store.custom_rules().iter().any(process_rule) && !store.tun_enabled(),
            move || {
                controls::inline_note(
                    theme::Tint::Info,
                    Icon::Info,
                    "进程分流需开启 TUN 才能覆盖所有进程；仅系统代理模式下，进程规则只作用于走代理的进程。",
                )
            },
        ))
        .child(editor(store, draft))
        .child(rule_list(store, draft, armed, arm_generation))
}

/// The counts that describe the tab, with the one action that refreshes external rule-sets.
fn summary_row(store: AppStore, tab: Tab) -> impl IntoElement {
    let busy = signal(false);
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(move || match tab {
                Tab::Routing => routing_summary(store),
                Tab::Dns => "命中域名后缀用专用 DNS 解析并直连，任何分流模式都生效".to_owned(),
            })
            .text_xs()
            .text_color(theme::text_faint()),
        )
        .child(when(
            move || tab == Tab::Routing,
            move || update_rules_button(busy),
        ))
}

fn routing_summary(store: AppStore) -> String {
    let rules = store.custom_rules();
    let enabled = rules.iter().filter(|rule| rule.enabled).count();
    format!("{enabled} 条启用 · {} 条规则", rules.len())
}

/// Refresh the downloaded rule-sets. Distinct from the app's own rules, which need no
/// download, so the label says 规则 in the sense the geodata provider uses.
fn update_rules_button(busy: Signal<bool>) -> impl IntoElement {
    controls::secondary_button(
        move || {
            if busy.get() {
                "正在更新"
            } else {
                "更新规则"
            }
        },
        {
            move || {
                if busy.get_untracked() {
                    return;
                }
                busy.set(true);
                spawn(async move {
                    let feedback = crate::store::store().feedback();
                    let result = api::update_rule_sets(true).await.map(|_| "分流规则已更新");
                    feedback.report("更新分流规则", result.map_err(|error| error.to_string()));
                    busy.set(false);
                });
            }
        },
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
    match_type: Signal<SharedString>,
    action: Signal<SharedString>,
    value: Signal<String>,
    error: Signal<Option<String>>,
}

fn rule_draft() -> RuleDraft {
    RuleDraft {
        editing: signal(None),
        match_type: signal(match_value(CustomRuleMatch::DomainSuffix)),
        action: signal(action_value(CustomRuleAction::Proxy)),
        value: signal(String::new()),
        error: signal(None),
    }
}

fn editor(store: AppStore, draft: RuleDraft) -> impl IntoElement {
    let (match_type, action, value, error) =
        (draft.match_type, draft.action, draft.value, draft.error);
    let editing = draft.editing;
    controls::card(
        view()
            .flex_col()
            .gap_3()
            .child(
                text(move || {
                    if editing.get().is_some() {
                        "编辑规则"
                    } else {
                        "添加规则"
                    }
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
                    .child(view().flex_1().min_w(px(220.0)).child(controls::text_field(
                        "匹配内容",
                        value,
                        move || placeholder_for(match_type.get()),
                    )))
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
                        text("此处指单条规则的匹配动作，与全局分流模式不同")
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
                                    domain::custom_rule_match_label(rule.match_type).to_owned()
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
                Some(format!("启用规则 {value}").into())
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

/// DNS overrides: a suffix, a resolver, and nothing else, so the rows stay one line.
fn dns(store: AppStore) -> impl IntoElement {
    let domain_field = signal(String::new());
    let server_field = signal(String::new());
    let error = signal(None::<String>);
    view()
        .flex_1()
        .min_h(px(0.0))
        .flex_col()
        .gap_3()
        .child(summary_row(store, Tab::Dns))
        .child(controls::card(
            view()
                .flex_col()
                .gap_3()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_end()
                        .gap_3()
                        .child(view().flex_1().min_w(px(200.0)).child(controls::text_field(
                            "域名后缀",
                            domain_field,
                            || "example.com",
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
                                let (domain_field, server_field, error) =
                                    (domain_field, server_field, error);
                                move || {
                                    let result = store.add_dns_override(
                                        &domain_field.get_untracked(),
                                        &server_field.get_untracked(),
                                    );
                                    match result {
                                        Ok(()) => {
                                            domain_field.set(String::new());
                                            server_field.set(String::new());
                                            error.set(None);
                                        }
                                        Err(message) => error.set(Some(message)),
                                    }
                                }
                            },
                        )),
                )
                .child(controls::field_error(error)),
        ))
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
                    dns_row(store, item)
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
                "为指定域名后缀固定 DNS，避免公共 DNS 解析到错误地址。",
            ))
            .flex_1()
            .min_h(px(0.0)),
        )
}

fn dns_row(store: AppStore, item: Memo<Option<DnsOverride>>) -> impl IntoElement {
    let id = move || item.with(|item| item.as_ref().map(|item| item.id));
    let suffix = move || {
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
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_col()
                .gap_0p5()
                .child(
                    text(suffix)
                        .text_sm()
                        .font_family(theme::font_mono())
                        .number_of_lines(1)
                        .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || {
                        item.with(|item| {
                            item.as_ref()
                                .map(|item| item.server.clone())
                                .unwrap_or_default()
                        })
                    })
                    .text_xs()
                    .font_family(theme::font_mono())
                    .number_of_lines(1)
                    .text_color(theme::text_faint()),
                ),
        )
        .child(controls::switch_state(
            move || Some(format!("启用 {} 的专用 DNS", suffix()).into()),
            move || item.with(|item| item.as_ref().is_some_and(|item| item.enabled)),
            || false,
            move |_: bool| {
                if let Some(id) = id() {
                    store.toggle_dns_override(id);
                }
            },
        ))
        .child(controls::icon_button(
            move || Icon::Trash2,
            move || format!("删除 {} 的专用 DNS", suffix()).into(),
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
    .map(|kind| SelectItem::new(match_value(kind), domain::custom_rule_match_label(kind)))
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
            domain::custom_rule_action_label(action),
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
