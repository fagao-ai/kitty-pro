//! Settings: appearance, and the mode that decides where traffic goes.
//!
//! Every row writes through the store, which owns persistence, so this screen never touches
//! the disk itself.

use crate::domain;
use crate::store::{AppStore, Screen, SyncAction};
use crate::ui::{controls, theme, Icon};
use nsh_base::{button, theme as base_theme};
use nsh_core::prelude::*;
use proxy_core::TunnelMode;
use std::time::Duration;

/// How long an armed destructive sync press stays armed. Shorter than the rule delete's window:
/// these two replace data rather than remove one row, and the summary under the button is meant
/// to be read before the second click, not after the arm has lapsed.
const SYNC_CONFIRM_SECONDS: u64 = 3;

/// The one name for the appearance switch. The rail carries the same control, so the
/// label and note live here and are shared rather than written twice.
pub const APPEARANCE_LABEL: &str = "深色模式";

/// The switch's note: what the current value means, not a restatement of the label.
pub fn appearance_note(dark: bool) -> SharedString {
    SharedString::from(if dark { "已启用" } else { "跟随系统" })
}

pub fn settings(store: AppStore) -> impl IntoElement {
    let query = signal(String::new());
    crate::shell::page_scroll(
        view()
            .flex_col()
            .gap_4()
            .child(controls::search_field("搜索设置", "搜索设置项", query))
            .child(when(move || no_match(query), move || unmatched(query)))
            .child(when(move || shows(query, Section::Appearance), {
                move || appearance_card(store)
            }))
            .child(when(move || shows(query, Section::Mode), {
                move || mode_card(store)
            }))
            .child(when(move || shows(query, Section::Scope), {
                move || scope_card(store)
            }))
            .child(when(move || shows(query, Section::Session), {
                move || session_card(store)
            }))
            .child(when(move || shows(query, Section::Sync), {
                move || sync_card(store)
            }))
            .child(when(move || shows(query, Section::Updates), {
                move || updates_card(store)
            }))
            .child(when(move || shows(query, Section::Script), {
                move || script_card(store)
            }))
            .child(when(move || shows(query, Section::Links), {
                move || config_links_card(store)
            }))
            .child(when(move || shows(query, Section::About), {
                move || about_card(store)
            })),
    )
}

/// The seven cards, under the name the search filters them by.
///
/// An alias so the page reads as its own vocabulary rather than reaching into the domain
/// module for a type it only passes through.
type Section = domain::SettingsSection;

/// Whether a card survives the query. The signal is read here so one binding decides it for
/// the card, and the card is either built whole or not at all.
fn shows(query: Signal<String>, section: Section) -> bool {
    section.matches(&domain::search_keyword(&query.get()))
}

/// Whether the query found nothing at all. Said once rather than per card, because seven
/// empty cards is not an answer and a page that quietly shows nothing reads as a crash.
fn no_match(query: Signal<String>) -> bool {
    let raw = query.get();
    !domain::search_keyword(&raw).is_empty() && domain::visible_settings_sections(&raw).is_empty()
}

/// The one exit from a search that found nothing: clear the box, see the page again.
fn unmatched(query: Signal<String>) -> impl IntoElement {
    view()
        .w_full()
        .flex_col()
        .items_center()
        .gap_3()
        .py_10()
        .child(controls::icon(Icon::Search, 28.0).text_color(move || base_theme().muted_foreground))
        .child(
            text("没有匹配的设置项")
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || base_theme().card_foreground),
        )
        .child(controls::secondary_button("清除", move || {
            query.set(String::new())
        }))
}

/// The two things a user does to a running proxy from a terminal or the next boot, rather than
/// to the configuration.
///
/// Grouped as one card because they share a subject: the machine, not the profile. Neither is
/// written to disk, and both are worth keeping off the connection card, where the everyday press
/// lives.
fn session_card(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_4()
        .child(heading("启动与终端"))
        .child(when(
            move || store.autostart().is_supported(),
            move || autostart_row(store),
        ))
        .child(controls::secondary_button_state(
            move || {
                if store.env_copy_busy() {
                    "正在复制"
                } else {
                    "复制代理环境变量"
                }
            },
            // The variables name a port, so they are worth copying whenever there is one to
            // name. They do not need a running core: a shell that exports them before the core
            // is up simply finds nothing until it is.
            move || store.env_copy_busy(),
            move || store.copy_proxy_environment(),
        ))
        .child(
            text(move || env_copy_note(store))
                .text_xs()
                .text_color(theme::text_faint()),
        )
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
}

/// What the copied lines are and where they fit, said in the shape the shell wants.
///
/// The backend copies a ready line rather than three bare names — `export http_proxy=… && …` on
/// macOS and Linux, `$env:http_proxy = …` on Windows — so the note names the variable in the case
/// the clipboard actually uses, and never promises an uppercase name the line does not contain.
fn env_copy_note(store: AppStore) -> String {
    let port = if store.allow_lan() {
        "局域网可达的 "
    } else {
        "本机的 "
    };
    format!(
        "一整行可直接粘贴到终端，指向{port}{} 端口，给不读系统代理的命令行工具用。",
        domain::PROXY_PORT
    )
}

/// Start at login, as the operating system reports it.
///
/// The state is read from the OS on boot and reread after a refused write, never kept from the
/// last value this app wrote: a login item is a file that an installer, a cleanup tool or the
/// user can delete while the app is running, and a switch left on would then be promising a
/// launch that will not happen.
fn autostart_row(store: AppStore) -> impl IntoElement {
    controls::switch_row_state(
        "登录时启动",
        move || store.autostart().note(store.autostart_busy()),
        move || store.autostart().enabled(),
        move || !store.autostart().is_ready() || store.autostart_busy(),
        move |next| store.set_autostart(next),
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
            ))
            // The language is stated, not offered: neither front end can switch it, and a row
            // with a menu of one would promise a feature the app does not have.
            .child(controls::value_row(
                "语言",
                "界面语言",
                domain::LANGUAGE_LABEL.to_owned(),
            )),
    )
}

/// How the machine hands traffic over. TUN takes it at the network layer and needs
/// administrator rights the first time; sharing the listener lets other devices on the
/// network use the same core.
///
/// Here rather than beside the connect control because neither is part of connecting: they
/// change how a connection is made, and the everyday case never opens this.
fn scope_card(store: AppStore) -> impl IntoElement {
    controls::card(
        view()
            .flex_col()
            .gap_4()
            .child(heading("接管方式"))
            .child(controls::switch_row_state(
                "TUN 模式",
                move || tun_note(store.tun_enabled()),
                move || store.tun_enabled(),
                move || store.core_action_busy(),
                move |next| store.set_tun_enabled(next),
            ))
            .child(controls::switch_row_state(
                "允许局域网连接",
                move || lan_note(store.allow_lan()),
                move || store.allow_lan(),
                move || store.core_action_busy(),
                move |next| store.set_allow_lan(next),
            )),
    )
}

/// What each value means standing on its own. Whether a change reached the running core is
/// the press's business: the store says 重新连接后生效 when it has not.
fn tun_note(enabled: bool) -> SharedString {
    SharedString::from(if enabled {
        "接管全部流量，首次需要管理员授权"
    } else {
        "仅接管走代理端口的流量"
    })
}

fn lan_note(enabled: bool) -> SharedString {
    SharedString::from(if enabled {
        "局域网内其他设备可使用 7890"
    } else {
        "只监听本机"
    })
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

/// What this build is and what it currently holds, with the one action that sends it away.
///
/// The figures are the counts a support conversation asks for — how many nodes, how many
/// rules — and every one of them is the same number the screens themselves show, so the card
/// cannot report a state the app has not displayed. The copy button exists because those
/// figures are useless one at a time: the point is a block of text to paste, not a page to
/// read off a screen.
fn about_card(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_4()
        .child(heading("诊断与关于"))
        .child(about_identity(store))
        .child(about_facts(store))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(controls::secondary_button(
                    "复制诊断信息",
                    move || store.copy_diagnostic_report(),
                )),
        )
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || base_theme().border)
        .bg(move || base_theme().card)
}

/// The build line: app version, core build, platform. Same words the sidebar's kicker uses,
/// at the length a bug report needs.
fn about_identity(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            view()
                .h(px(36.0))
                .w(px(36.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(theme::radius_tile())
                .bg(move || base_theme().primary)
                .child(
                    controls::icon(Icon::Package, 20.0)
                        .text_color(move || base_theme().primary_foreground),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text("Kitty Pro")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .font_family(theme::font_display())
                        .text_color(move || base_theme().card_foreground),
                )
                .child(
                    text(move || about_line(store))
                        .text_xs()
                        .font_family(theme::font_mono())
                        .text_color(theme::text_faint()),
                ),
        )
}

fn about_line(store: AppStore) -> String {
    let facts = store.diagnostic_facts();
    domain::about_build_label(
        &facts.app_version,
        facts.core_version.as_deref(),
        &facts.platform,
    )
}

/// The four figures, wrapped so two sit side by side on a desktop and they stack on a phone.
fn about_facts(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_4()
        .child(controls::about_fact("内核状态", move || {
            domain::core_state_label(&store.core_state())
        }))
        .child(controls::about_fact("节点 / 订阅", move || {
            node_subscription_counts(store)
        }))
        .child(controls::about_fact("自定义规则", move || {
            store.custom_rule_count().to_string()
        }))
        .child(controls::about_fact("专用 DNS", move || {
            store.dns_overrides().len().to_string()
        }))
}

fn node_subscription_counts(store: AppStore) -> String {
    format!("{} / {}", store.nodes().len(), store.subscription_count())
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

// --------------------------------------------------------------------- sync

/// The remote-sync card: where the snapshot lives, and the four things that can be done with it.
///
/// This is the one card whose presses reach outside the machine, which shapes everything below:
/// nothing is enabled until the saved config has actually been read (saving a default over a real
/// `sync.json` is invisible data loss), the two actions that replace data ask twice, and a refusal
/// the remote answers in ETag jargon is turned into the one move that resolves it.
fn sync_card(store: AppStore) -> impl IntoElement {
    let armed = signal(None::<SyncAction>);
    let arm_generation = signal(0u64);
    controls::card(
        view()
            .flex_col()
            .gap_4()
            .child(controls::section_header(
                "规则与订阅同步",
                Some(sync_badge(store)),
            ))
            .child(when(
                move || store.sync_load_error().is_some(),
                move || sync_load_error(store),
            ))
            .child(provider_row(store))
            .child(controls::switch_row_state(
                "远端同步",
                move || sync_enable_note(store),
                move || store.sync_config().is_some_and(|config| config.enabled),
                move || store.sync_busy().is_some() || !store.sync_ready(),
                move |enabled| store.set_sync_enabled(enabled),
            ))
            .child(controls::inline_note(
                theme::Tint::Info,
                Icon::Info,
                "远端内容包含订阅来源（含令牌）、自定义规则、专用 DNS 与配置脚本。",
            ))
            .child(sync_form(store))
            .child(sync_actions(store, armed, arm_generation))
            .child(when(
                move || armed.get() == Some(SyncAction::Pull),
                move || pull_summary(store),
            ))
            .child(when(
                move || store.sync_conflict().is_some(),
                move || conflict_panel(store),
            )),
    )
}

/// The badge is the card's answer to "is my other device current?", which is the question that
/// sent the user here. 待同步 and 已同步 are different facts and must not share a colour.
fn sync_badge(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(controls::badge(
            move || store.sync_status().label().to_owned(),
            move || store.sync_status().tint(),
        ))
        .child(when(
            move || !sync_age(store).is_empty(),
            move || {
                text(move || sync_age(store))
                    .text_xs()
                    .font_family(theme::font_mono())
                    .text_color(theme::text_faint())
            },
        ))
}

/// How long ago the last exchange was. Empty when there has not been one, so the badge never
/// says 刚刚 about something that has never happened.
fn sync_age(store: AppStore) -> String {
    let status = store.sync_status();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    status.since_label(now).unwrap_or_default()
}

fn sync_enable_note(store: AppStore) -> String {
    if !store.sync_ready() {
        return "正在读取已保存的同步设置".to_owned();
    }
    match store.sync_config() {
        Some(config) if config.enabled => "已启用".to_owned(),
        _ => "已停用".to_owned(),
    }
}

/// A failed read of the settings parks every control and says why, with the retry that lets the
/// user un-park it. Without the reason the card looks merely disabled, and a disabled card with
/// no explanation reads as a broken feature.
fn sync_load_error(store: AppStore) -> impl IntoElement {
    controls::inline_note_with(
        theme::Tint::Danger,
        Icon::CircleAlert,
        // The backend's own words first, then what the app did about it: the reason is what the
        // user acts on, and the consequence is what explains the parked buttons. Joined with a
        // full stop because a backend error has no ending punctuation of its own.
        move || {
            format!(
                "同步配置读取失败：{}。已停用所有同步操作，以免用空配置覆盖已保存的设置。",
                store.sync_load_error().unwrap_or_default()
            )
        },
        Some(controls::secondary_button("重试", move || {
            store.refresh_sync_config();
        })),
    )
}

/// Which storage the snapshot goes to. Two segments, and switching them swaps the whole field
/// set rather than reinterpreting what is in it.
fn provider_row(store: AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text("存储类型")
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || base_theme().card_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_1()
                .child(provider_pill(store, false))
                .child(provider_pill(store, true)),
        )
}

fn provider_pill(store: AppStore, s3: bool) -> impl IntoElement {
    let chosen = move || {
        store
            .sync_config()
            .is_some_and(|config| (config.provider == api::SyncProviderKind::S3) == s3)
    };
    button()
        .selected(chosen)
        .disabled(move || store.sync_busy().is_some() || !store.sync_ready())
        .on_press(move |_: &()| store.set_sync_provider(s3))
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
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(
                domain::SyncProviderForm::from_kind(if s3 {
                    api::SyncProviderKind::S3
                } else {
                    api::SyncProviderKind::WebDav
                })
                .label(),
            )
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
                if chosen() {
                    base_theme().primary_foreground
                } else {
                    base_theme().card_foreground
                }
            }),
        )
}

/// The address, the object path, and the provider's own credentials.
///
/// Fields are drawn from `SyncProviderForm::fields` rather than written out per provider: that
/// list is also what says which stored value each input edits, so a label and its field cannot
/// drift into saving a password into the username.
///
/// The provider is read through a memo rather than inline. The two branches must rebuild when it
/// changes, and the whole form must not rebuild on every keystroke — a recreated input loses the
/// cursor mid-address, which is the one thing that makes a field unusable.
fn sync_form(store: AppStore) -> impl IntoElement {
    let is_s3 = computed(move || provider_form(store) == domain::SyncProviderForm::S3);
    view()
        .flex_col()
        .gap_3()
        .child(
            when(
                move || is_s3.get(),
                move || endpoint_field(store, domain::SyncField::S3Endpoint).into_element(),
            )
            .otherwise(move || {
                endpoint_field(store, domain::SyncField::WebDavEndpoint).into_element()
            }),
        )
        .child(path_field(store))
        .child(
            when(
                move || is_s3.get(),
                move || provider_fields(store, domain::SyncProviderForm::S3).into_element(),
            )
            .otherwise(move || {
                provider_fields(store, domain::SyncProviderForm::WebDav).into_element()
            }),
        )
}

fn provider_form(store: AppStore) -> domain::SyncProviderForm {
    domain::SyncProviderForm::from_kind(
        store
            .sync_config()
            .map_or(api::SyncProviderKind::WebDav, |config| config.provider),
    )
}

/// The provider's credential fields, drawn from its own list.
fn provider_fields(store: AppStore, form: domain::SyncProviderForm) -> impl IntoElement {
    view().flex_col().gap_3().child(each(
        move || form.fields(),
        |spec| spec.field,
        move |item, _| {
            let spec = item.get();
            provider_field(store, spec.field, spec.label, spec.placeholder, spec.secret)
                .into_element()
        },
    ))
}

/// One field of the form. `masked` is what distinguishes a secret from an address: a token left
/// plain on screen is shoulder-surfaced, and the field can hide what it holds without costing the
/// user anything, since the value is saved as typed.
fn provider_field(
    store: AppStore,
    field: domain::SyncField,
    label: &'static str,
    placeholder: &'static str,
    masked: bool,
) -> impl IntoElement {
    controls::text_field_state(
        label,
        move || {
            field
                .read(&store.sync_config().unwrap_or_default())
                .to_owned()
        },
        move |value| store.set_sync_field(field, value),
        move || placeholder,
        masked,
    )
}

fn endpoint_field(store: AppStore, field: domain::SyncField) -> impl IntoElement {
    let label = if field == domain::SyncField::S3Endpoint {
        domain::SyncProviderForm::S3.endpoint_label()
    } else {
        domain::SyncProviderForm::WebDav.endpoint_label()
    };
    let placeholder = if field == domain::SyncField::S3Endpoint {
        domain::SyncProviderForm::S3.endpoint_placeholder()
    } else {
        domain::SyncProviderForm::WebDav.endpoint_placeholder()
    };
    controls::text_field_state(
        label,
        move || {
            field
                .read(&store.sync_config().unwrap_or_default())
                .to_owned()
        },
        move |value| store.set_sync_field(field, value),
        move || placeholder,
        false,
    )
}

/// The path, with the directory it implies. The snapshot travels as a bundle beside the named
/// object, so the hint says where the rest of it lands rather than leaving a guess.
fn path_field(store: AppStore) -> impl IntoElement {
    view()
        .flex_col()
        .gap_1p5()
        .child(controls::text_field_state(
            "远程路径",
            move || {
                domain::SyncField::Path
                    .read(&store.sync_config().unwrap_or_default())
                    .to_owned()
            },
            move |value| store.set_sync_field(domain::SyncField::Path, value),
            || "kitty-pro-sync.json",
            false,
        ))
        .child(
            text(move || format!("内容写入同级目录 /{}/", bundle_directory(store)))
                .text_xs()
                .text_color(theme::text_faint()),
        )
}

fn bundle_directory(store: AppStore) -> String {
    proxy_core::sync_bundle_directory(
        domain::SyncField::Path.read(&store.sync_config().unwrap_or_default()),
    )
}

/// The four actions. 保存 is local; the other three go out, and two of those replace data.
fn sync_actions(
    store: AppStore,
    armed: Signal<Option<SyncAction>>,
    arm_generation: Signal<u64>,
) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(controls::secondary_button_state(
            // Named rather than left as 保存: this page has more than one thing that can be
            // saved, and the script editor's own 保存 shares it whenever the editor is open. A
            // button that says what it writes is the difference between saving a settings file
            // and wondering afterwards which one you just pressed.
            move || {
                if store.sync_busy() == Some(SyncAction::Save) {
                    "正在保存同步设置"
                } else {
                    "保存同步设置"
                }
            },
            move || !store.sync_ready() || store.sync_busy().is_some(),
            move || {
                disarm(armed);
                store.save_sync_config();
            },
        ))
        .child(confirm_action(
            store,
            armed,
            arm_generation,
            SyncAction::Pull,
            "下载覆盖本机",
            "确认覆盖本机",
        ))
        .child(controls::primary_button_state(
            move || {
                if store.sync_busy() == Some(SyncAction::Push) {
                    "正在上传"
                } else {
                    "上传"
                }
            },
            // Safe upload only: it refuses when the remote moved, and the panel under it is what
            // offers the two ways out.
            move || !can_reach_remote(store) || store.sync_busy().is_some(),
            move || {
                disarm(armed);
                store.sync_push();
            },
        ))
        .child(confirm_action(
            store,
            armed,
            arm_generation,
            SyncAction::ForcePush,
            "覆盖远端",
            "确认覆盖远端",
        ))
}

/// Whether a remote action may start: enabled, read, and complete enough to dial. Naming the
/// missing field is the point of `sync_target_incomplete` — a press that goes out and fails at
/// the far end costs a round trip to say what was visible beforehand.
fn can_reach_remote(store: AppStore) -> bool {
    let Some(config) = store.sync_config() else {
        return false;
    };
    if !config.enabled {
        return false;
    }
    domain::sync_target_incomplete(provider_form(store), &config).is_none()
}

/// One action, with the second click its destructive variants need.
///
/// The arm expires: a button left reading 确认覆盖本机 while the user goes to check something is
/// a button that will do the wrong thing when they get back. The generation token is what lets a
/// fresh arm survive the older arm's own timer.
fn confirm_action(
    store: AppStore,
    armed: Signal<Option<SyncAction>>,
    arm_generation: Signal<u64>,
    action: SyncAction,
    label: &'static str,
    confirm_label: &'static str,
) -> impl IntoElement {
    let is_armed = move || armed.get() == Some(action);
    let destructive = matches!(action, SyncAction::Pull | SyncAction::ForcePush);
    let disabled = move || match action {
        SyncAction::Pull | SyncAction::ForcePush => {
            !can_reach_remote(store) || store.sync_busy().is_some()
        }
        _ => !store.sync_ready() || store.sync_busy().is_some(),
    };
    let on_press = move || {
        if !destructive {
            run_action(store, action);
            return;
        }
        if is_armed() {
            disarm(armed);
            run_action(store, action);
            return;
        }
        let generation = arm_generation.get_untracked().wrapping_add(1);
        arm_generation.set(generation);
        armed.set(Some(action));
        spawn(async move {
            timer(Duration::from_secs(SYNC_CONFIRM_SECONDS)).await;
            if arm_generation.get_untracked() == generation {
                armed.set(None);
            }
        });
    };
    if matches!(action, SyncAction::ForcePush) {
        return controls::destructive_button(
            move || {
                if is_armed() {
                    confirm_label.to_owned()
                } else {
                    label.to_owned()
                }
            },
            disabled,
            on_press,
        )
        .into_element();
    }
    controls::secondary_button_state(
        move || {
            if is_armed() {
                confirm_label.to_owned()
            } else if store.sync_busy() == Some(action) {
                sync_running_words(action).to_owned()
            } else {
                label.to_owned()
            }
        },
        disabled,
        on_press,
    )
    .into_element()
}

fn run_action(store: AppStore, action: SyncAction) {
    match action {
        SyncAction::Save => store.save_sync_config(),
        SyncAction::Push => store.sync_push(),
        SyncAction::ForcePush => store.sync_force_push(),
        SyncAction::Pull => store.sync_pull(),
    }
}

fn disarm(armed: Signal<Option<SyncAction>>) {
    armed.set(None);
}

fn sync_running_words(action: SyncAction) -> &'static str {
    match action {
        SyncAction::Save => "正在保存",
        SyncAction::Push => "正在上传",
        SyncAction::ForcePush => "正在覆盖",
        SyncAction::Pull => "正在下载",
    }
}

/// What a confirmed download is about to replace, on both sides.
fn pull_summary(store: AppStore) -> impl IntoElement {
    controls::inline_note(theme::Tint::Warning, Icon::CircleAlert, move || {
        let counts = store.sync_remote_counts();
        domain::pull_confirm_summary(
            counts,
            store.subscriptions().len(),
            store.custom_rule_count(),
        )
    })
}

/// The refusal the remote sent back, with the two moves that resolve it.
///
/// The friendly line is the headline and the backend's own wording is kept under it: the first is
/// for whoever pressed the button, the second for whoever has to fix the server.
fn conflict_panel(store: AppStore) -> impl IntoElement {
    view()
        .w_full()
        .flex_col()
        .gap_2()
        .child(controls::inline_note(
            theme::Tint::Danger,
            Icon::CircleAlert,
            move || {
                store
                    .sync_conflict()
                    .map(|(friendly, _)| friendly)
                    .unwrap_or_default()
            },
        ))
        .child(
            text(move || {
                store
                    .sync_conflict()
                    .map(|(_, raw)| format!("后端原文：{raw}"))
                    .unwrap_or_default()
            })
            .text_xs()
            .font_family(theme::font_mono())
            .text_color(theme::text_faint()),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(controls::secondary_button_state(
                    "下载覆盖本机",
                    move || store.sync_busy().is_some(),
                    move || store.sync_pull(),
                ))
                .child(controls::secondary_button_state(
                    "覆盖远端",
                    move || store.sync_busy().is_some(),
                    move || store.sync_force_push(),
                )),
        )
}

// ------------------------------------------------------------------- updates

/// The update card: what the release feed said, and the two things the user can do about it.
///
/// The card is mostly a report rather than a control, because the download itself belongs to the
/// release page — the notes and the checksums are one click from the asset, and an app that
/// fetched its own replacement could not verify what it got. The one state that offers a press
/// is a release the user has not yet declined.
fn updates_card(store: AppStore) -> impl IntoElement {
    controls::card(
        view()
            .flex_col()
            .gap_4()
            .child(controls::section_header(
                "应用更新",
                Some(controls::badge(
                    move || update_badge(store),
                    move || update_tint(store),
                )),
            ))
            .child(controls::switch_row_state(
                "每天自动检查",
                move || auto_check_note(store),
                move || {
                    store
                        .update_settings()
                        .is_some_and(|settings| settings.auto_check)
                },
                move || !store.update_ready() || store.update_busy(),
                move |enabled| store.set_update_auto_check(enabled),
            ))
            .child(when(
                move || store.update_error().is_some(),
                move || {
                    controls::inline_note(theme::Tint::Danger, Icon::CircleAlert, move || {
                        store.update_error().unwrap_or_default()
                    })
                },
            ))
            .child(when(
                move || release_line(store).is_some(),
                move || {
                    text(move || release_line(store).unwrap_or_default())
                        .text_xs()
                        .text_color(theme::text_soft())
                },
            ))
            .child(when(
                // Driven by the backend's answer rather than by a guess at the window shape: an
                // NSH phone build is Android, which is published, so a front-end rule here would
                // park a control the server would happily have answered.
                move || store.update_display_state() == api::update::AppUpdateState::Unsupported,
                move || {
                    controls::inline_note(theme::Tint::Info, Icon::Info, move || {
                        format!(
                            "此构建没有发布通道（{}），请从发布页手动下载。",
                            store
                                .update_status()
                                .map_or_else(String::new, |status| status.target)
                        )
                    })
                },
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(controls::secondary_button_state(
                        move || {
                            if store.update_busy() {
                                "正在检查"
                            } else {
                                "检查更新"
                            }
                        },
                        move || store.update_busy() || !store.update_ready(),
                        move || store.check_for_update(),
                    ))
                    .child(when(
                        move || store.update_is_actionable(),
                        move || {
                            view().flex_row().gap_2().child(controls::secondary_button(
                                "忽略这个版本",
                                move || store.dismiss_update(),
                            ))
                        },
                    )),
            )
            .child(
                text(move || check_note(store))
                    .text_xs()
                    .text_color(theme::text_faint()),
            ),
    )
}

fn update_badge(store: AppStore) -> &'static str {
    domain::update_badge_label(
        store.update_display_state(),
        store.update_busy(),
        store.update_ready(),
        store.update_status().is_some_and(|status| status.snoozed),
    )
}

fn update_tint(store: AppStore) -> theme::Tint {
    if store.update_is_actionable() {
        return theme::Tint::Warning;
    }
    match store.update_display_state() {
        api::update::AppUpdateState::UpToDate => theme::Tint::Success,
        api::update::AppUpdateState::Failed => theme::Tint::Danger,
        _ => theme::Tint::Neutral,
    }
}

/// The auto-check row's note.
///
/// The timestamp is whichever source has the newer one. The settings are read once at boot and the
/// check answer arrives later, so reading only the settings would keep the row quoting the launch
/// check long after the user pressed 检查更新 themselves — a control that reports a stale result
/// reads as a control that did nothing.
fn auto_check_note(store: AppStore) -> String {
    let settings = store.update_settings();
    let auto_check = settings.as_ref().is_none_or(|held| held.auto_check);
    let status = store.update_status();
    let last_check_at = settings
        .as_ref()
        .map_or(0, |held| held.last_check_at)
        .max(status.as_ref().map_or(0, |held| held.last_check_at));
    let server_time = status.map_or(0, |held| held.server_time);
    domain::auto_check_label(auto_check, last_check_at, server_time)
}

/// The newest release, in one line: its name, and how old the app's knowledge of it is.
fn release_line(store: AppStore) -> Option<String> {
    let status = store.update_status()?;
    let release = status.latest?;
    let age = domain::age_label(
        status.last_check_at,
        // Ages are measured by the machine that contacted the feed: it wrote both stamps, and a
        // client with no usable clock would otherwise render 从未检查 for ever.
        status.server_time,
    )
    .unwrap_or_else(|| "刚刚".to_owned());
    Some(domain::release_subtitle(
        &release.title,
        &release.version,
        age,
    ))
}

/// What the last exchange established, said so that "no update" and "could not tell" never sound
/// alike.
fn check_note(store: AppStore) -> String {
    let status = store.update_status();
    let server_time = status.as_ref().map_or(0, |held| held.server_time);
    let age = status
        .as_ref()
        .and_then(|held| domain::age_label(held.last_check_at, server_time));
    match (status, age) {
        (Some(held), Some(age)) if held.state == api::update::AppUpdateState::UpToDate => {
            format!("当前 v{} · 上次检查 {age}", held.current_version)
        }
        (Some(held), _) if held.state == api::update::AppUpdateState::Unsupported => {
            format!("此构建无发布通道（{}）", held.target)
        }
        (_, Some(age)) => format!("上次检查 {age}"),
        (_, None) => "尚未检查过更新".to_owned(),
    }
}
