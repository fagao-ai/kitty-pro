//! Styled controls: each wraps one `nsh_base` component with the app's look and states.
//!
//! `nsh_base` brings behavior and no appearance, so a screen calls these instead of styling a
//! base control itself. Every color is a token from `theme.json`.

use crate::ui::{
    theme::{self, Tint},
    Icon,
};
use nsh_base::{button, switch, theme, SelectItem};
use nsh_core::prelude::*;
use std::rc::Rc;

/// A glyph at the size the app actually uses it. The stroke is `currentColor`, so the
/// caller's `text_color` tints it.
pub fn icon(glyph: Icon, size: f32) -> Svg {
    svg(glyph.src()).w(px(size)).h(px(size)).flex_shrink_0()
}

/// A glyph that changes with app state, such as a theme toggle's sun and moon. The
/// closure is a binding: only the image swaps, and the row around it is not rebuilt.
pub fn live_icon(glyph: impl Fn() -> Icon + 'static, size: f32) -> Svg {
    svg(move || glyph().src())
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
}

/// The app's primary action: a solid accent fill.
pub fn primary_button(
    label: impl IntoReactive<SharedString> + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    primary_button_state(label, || false, on_press)
}

/// The same, with the reason it cannot be used yet. A form that submits nothing reads as a
/// broken button, so an incomplete one is parked instead, and the field that is missing says
/// so through its own placeholder.
pub fn primary_button_state(
    label: impl IntoReactive<SharedString> + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .disabled(disabled)
        .on_press(move |_: &()| on_press())
        .h(px(36.0))
        .px_4()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(theme::radius_tile())
        .bg(move || theme().primary)
        .active(|s| s.opacity(0.88))
        .hover(|s| s.opacity(0.94))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary_foreground),
        )
}

/// A card-sized primary action: connect and disconnect.
pub fn hero_button(
    label: impl IntoReactive<SharedString> + 'static,
    glyph: impl Fn() -> Icon + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .disabled(disabled)
        .on_press(move |_: &()| on_press())
        .h(px(56.0))
        .px_6()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_3()
        .rounded(theme::radius_card())
        .bg(move || theme().primary)
        .shadow_lg()
        .active(|s| s.opacity(0.88))
        .hover(|s| s.opacity(0.94))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(live_icon(glyph, 20.0).text_color(move || theme().primary_foreground))
        .child(
            text(label)
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary_foreground),
        )
}

/// A quiet action: a bordered control on the card.
pub fn secondary_button(
    label: impl IntoReactive<SharedString> + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    secondary_button_state(label, || false, on_press)
}

/// The same, parked while its result would be nothing: clearing an empty buffer, or
/// switching to the subscription already in use.
pub fn secondary_button_state(
    label: impl IntoReactive<SharedString> + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .disabled(disabled)
        .on_press(move |_: &()| on_press())
        .h(px(32.0))
        .px_3()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.8))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().card_foreground),
        )
}

/// An icon-only action. The label is required: it names the button to a screen
/// reader and gives headless tests a selector.
pub fn icon_button(
    glyph: impl Fn() -> Icon + 'static,
    label: impl Fn() -> SharedString + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(move || Some(label()))
        .on_press(move |_: &()| on_press())
        .h(px(32.0))
        .w(px(32.0))
        .items_center()
        .justify_center()
        .rounded(theme::radius_tile())
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.7))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        // 44pt on a touch idiom without growing the layout: the slop is invisible.
        .hit_slop(EdgeInsets::all(px(if is_desktop() { 0.0 } else { 6.0 })))
        .child(live_icon(glyph, 16.0).text_color(move || theme().muted_foreground))
}

/// A labeled switch whose track and thumb are both styled: an unstyled `switch`
/// renders nothing.
pub fn switch_row(
    label: impl Into<SharedString> + 'static,
    note: impl IntoReactive<SharedString> + 'static,
    checked: Signal<bool>,
    disabled: Signal<bool>,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let control = switch_state(
        {
            let label = label.clone();
            move || Some(label.clone())
        },
        move || checked.get(),
        move || disabled.get(),
        on_change,
    );

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
                        .text_color(move || theme().card_foreground),
                )
                .child(text(note).text_xs().text_color(theme::text_faint)),
        )
        .child(control)
}

/// The switch's parts, styled, with the state read through closures.
///
/// A row in a virtual list is built once and shown for many items, so its checked state has
/// to be read at draw time from whatever row it currently holds; a signal of its own would
/// belong to the cell rather than the data. `switch_row` uses this with signals.
///
/// The name is said the same way: a switch with no label of its own and no text inside it is
/// unnamed to a screen reader, which is why a row passes what it is switching on.
pub fn switch_state(
    label: impl Fn() -> Option<SharedString> + 'static,
    checked: impl Fn() -> bool + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    switch()
        .accessibility_label(label)
        .checked(checked)
        .disabled(disabled)
        .on_change(move |next: &bool| on_change(*next))
        .h(px(22.0))
        .w(px(38.0))
        .flex_shrink_0()
        .track_style(|s| {
            s.rounded_full()
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().muted)
        })
        .checked_style(|s| {
            s.bg(move || theme().primary)
                .border_color(move || theme().primary)
        })
        .thumb_style(|s| {
            s.h(px(16.0))
                .w(px(16.0))
                .rounded_full()
                .bg(move || theme().card)
        })
}

/// The card: the app's one raised surface. The caller lays out its own content, so a card
/// holding a row and a card holding a stack are the same call.
pub fn card(content: impl IntoElement) -> impl IntoElement {
    view()
        .p_5()
        .rounded(theme::radius_card())
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(content)
}

/// A card's heading with an optional trailing action.
pub fn section_header(
    title: impl Into<SharedString> + 'static,
    trailing: Option<impl IntoElement>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(title.into())
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().card_foreground),
        )
        .child(trailing)
}

/// A status pill: colored wash, matching foreground, one line of text.
pub fn badge(
    label: impl IntoReactive<SharedString> + 'static,
    tint: impl IntoReactive<Tint> + 'static,
) -> impl IntoElement {
    let tint = Rc::new(tint.into_reactive());
    let tint_bg = tint.clone();
    let tint_fg = tint;
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .self_start()
        .rounded(theme::radius_tile())
        .bg(move || tint_bg.get().wash())
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || tint_fg.get().foreground()),
        )
}

/// A dot that says whether the core is up, drawn even at 8pt.
pub fn status_dot(tint: Tint) -> impl IntoElement {
    view()
        .h(px(8.0))
        .w(px(8.0))
        .flex_shrink_0()
        .rounded_full()
        .bg(move || tint.foreground())
}

/// A labeled figure on the overview: the value carries the weight, the unit sits beside it.
pub fn metric(
    label: impl Into<SharedString> + 'static,
    value: String,
    unit: String,
) -> impl IntoElement {
    view()
        .flex_col()
        .gap_1()
        .child(text(label.into()).text_xs().text_color(theme::text_faint))
        .child(
            view()
                .flex_row()
                .items_end()
                .gap_1()
                .child(
                    text(value)
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().card_foreground),
                )
                .child(when(!unit.is_empty(), {
                    let unit = unit.clone();
                    move || text(unit.clone()).text_xs().text_color(theme::text_faint)
                })),
        )
}

/// A sidebar destination. `active` is a memo the caller owns, so every binding
/// here reads one cached answer; `button` supplies role, focus and the selected
/// state, which is what makes the row reachable by keyboard and screen reader.
pub fn nav_row(
    label: &'static str,
    glyph: Icon,
    active: Memo<bool>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .selected(move || active.get())
        .on_press(move |_: &()| on_press())
        .h(px(36.0))
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .rounded(theme::radius_tile())
        .selected_style(|s| s.bg(move || theme().accent))
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.8))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        // The label carries the selected weight: color alone is not enough on a
        // rail where several rows share one tint.
        .style_with(move |s| {
            s.font_weight(if active.get() {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
        })
        .child(icon(glyph, 18.0).text_color(move || {
            if active.get() {
                theme().accent_foreground
            } else {
                theme().muted_foreground
            }
        }))
        .child(
            text(label)
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_sm()
                .text_color(move || {
                    if active.get() {
                        theme().accent_foreground
                    } else {
                        theme().card_foreground
                    }
                }),
        )
}

/// A bottom-bar destination: glyph above a short label, tall enough for a thumb.
pub fn nav_tab(
    label: &'static str,
    glyph: Icon,
    active: Memo<bool>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .selected(move || active.get())
        .on_press(move |_: &()| on_press())
        .h(px(52.0))
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .selected_style(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.8))
        .child(icon(glyph, 20.0).text_color(move || {
            if active.get() {
                theme().accent_foreground
            } else {
                theme().muted_foreground
            }
        }))
        .child(text(label).text_size(px(11.0)).text_color(move || {
            if active.get() {
                theme().accent_foreground
            } else {
                theme().muted_foreground
            }
        }))
}

/// The core's state where the top bar needs it. Connected and idle share one
/// family and only a missing core escalates, so the chip does not shout.
pub fn connection_chip(store: crate::store::AppStore) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .flex_shrink_0()
        .rounded(theme::radius_tile())
        .bg(move || core_tint(store).wash())
        .child(status_dot_tinted(move || core_tint(store)))
        .child(
            text(move || crate::domain::core_state_label(&store.core_state()))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || core_tint(store).foreground()),
        )
}

fn core_tint(store: crate::store::AppStore) -> Tint {
    crate::domain::core_tint(store.connected(), &store.core_state())
}

/// A dot whose tint is decided at draw time, for a state that changes.
pub fn status_dot_tinted(tint: impl Fn() -> Tint + 'static) -> impl IntoElement {
    view()
        .h(px(8.0))
        .w(px(8.0))
        .flex_shrink_0()
        .rounded_full()
        .bg(move || tint().foreground())
}

/// A page's empty state: what is missing and the one action that fixes it.
pub fn empty_state(
    title: impl Into<SharedString> + 'static,
    note: impl Into<SharedString> + 'static,
) -> impl IntoElement {
    let title = title.into();
    let note = note.into();
    view()
        .w_full()
        .flex_col()
        .items_center()
        .gap_1()
        .py_10()
        .child(
            text(title)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(text(note).text_xs().text_color(theme::text_faint()))
}

/// The bordered box a text field is drawn in, shared by the single-line and the
/// multiline case so both answer focus the same way.
fn field_frame(content: impl IntoElement) -> impl IntoElement {
    view()
        .w_full()
        .flex_col()
        .gap_1p5()
        .child(content)
        .min_w(px(0.0))
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || theme().input)
        .px_3()
        .py_2()
        .focus_within(|s| s.border_color(move || theme().ring))
}

/// A labeled single-line field. The label is above rather than a placeholder standing in
/// for it: a placeholder disappears the moment the user types, which is exactly when the
/// field's purpose is least obvious.
pub fn text_field(
    label: impl Into<SharedString> + 'static,
    value: Signal<String>,
    placeholder: impl Fn() -> &'static str + 'static,
) -> impl IntoElement {
    let label = label.into();
    view()
        .w_full()
        .flex_col()
        .gap_1p5()
        .child(field_label(label.clone()))
        .child(field_frame(
            nsh_base::input()
                .value(move || value.get())
                .on_change(move |text: &str| value.set(text.to_owned()))
                .placeholder(placeholder)
                .placeholder_color(|| color(theme().muted_foreground))
                .caret_color(|| color(theme().primary))
                // The visible label is also its name: a placeholder stops being read once
                // the field holds text, which is when the name matters most.
                .label(label)
                .w_full()
                .min_w(px(0.0))
                .text_sm()
                .text_color(move || theme().card_foreground),
        ))
}

/// A labeled multiline field, for the config script. `lines` sets the visible height so
/// the editor does not collapse to one row of a script the user cannot read.
pub fn text_editor(
    label: impl Into<SharedString> + 'static,
    value: Signal<String>,
    placeholder: &'static str,
    lines: f32,
) -> impl IntoElement {
    let label = label.into();
    view()
        .w_full()
        .flex_col()
        .gap_1p5()
        .child(field_label(label.clone()))
        .child(field_frame(
            nsh_base::input()
                .multiline(true)
                .value(move || value.get())
                .on_change(move |text: &str| value.set(text.to_owned()))
                .placeholder(placeholder)
                .placeholder_color(|| color(theme().muted_foreground))
                .caret_color(|| color(theme().primary))
                .label(label)
                .w_full()
                .min_w(px(0.0))
                .h(px(lines * 18.0))
                .text_size(px(12.0))
                .font_family(theme::font_mono())
                .text_color(move || theme().card_foreground),
        ))
}

fn field_label(label: SharedString) -> impl IntoElement {
    text(label)
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || theme().muted_foreground)
}

/// A labeled single choice from a short list. `select()` brings the keyboard, the role and
/// the sheet-on-phone behaviour and no appearance, so the trigger and popup are drawn here:
/// the trigger looks like the neighbouring text fields, and the popup is the same surface
/// as a popover, which is what makes the two read as one system.
pub fn choice_field(
    label: &'static str,
    chosen: Signal<SharedString>,
    items: impl Fn() -> Vec<SelectItem> + 'static,
    on_change: impl Fn(SharedString) + 'static,
) -> impl IntoElement {
    let items = Rc::new(items);
    let title_items = items.clone();
    let title = move || {
        let value = chosen.get();
        title_items()
            .into_iter()
            .find(|item| item.value == value)
            .map(|item| item.title)
            .unwrap_or_else(|| "选择".into())
    };
    view()
        .w_full()
        .min_w(px(0.0))
        .flex_col()
        .gap_1p5()
        .child(field_label(label.into()))
        .child(
            nsh_base::select()
                .items(move || items())
                .value(move || Some(chosen.get()))
                .on_change(move |next: &Option<SharedString>| {
                    if let Some(next) = next.clone() {
                        on_change(next);
                    }
                })
                .accessibility_label(label)
                .offset(px(4.0))
                .popup_style(|s| {
                    s.p_1()
                        .rounded(theme::radius_tile())
                        .border_1()
                        .border_color(move || theme().border)
                        .bg(move || theme().popover)
                        .shadow_md()
                })
                .item_style(|s| {
                    s.px_3()
                        .py_2()
                        .rounded(theme::radius_tile())
                        .text_color(move || theme().popover_foreground)
                })
                .trigger(
                    view()
                        .w_full()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .h(px(36.0))
                        .px_3()
                        .rounded(theme::radius_tile())
                        .border_1()
                        .border_color(move || theme().input)
                        .focus_within(|s| s.border_color(move || theme().ring))
                        .child(
                            text(title)
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .number_of_lines(1)
                                .text_color(move || theme().card_foreground),
                        )
                        .child(
                            icon(Icon::ChevronDown, 14.0)
                                .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
}

/// The field's own error line: the message sits under the field it describes rather than
/// only in a notice, because a notice ages out while the mistake stays.
pub fn field_error(message: Signal<Option<String>>) -> impl IntoElement {
    let shown = move || message.get().is_some();
    when(shown, move || {
        text(move || message.get().unwrap_or_default())
            .text_xs()
            .text_color(move || theme::Tint::Danger.foreground())
    })
}

/// A destructive action. It wears the danger family so an irreversible press never looks
/// like an ordinary one; the copy still names the result.
pub fn destructive_button(
    label: impl IntoReactive<SharedString> + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .disabled(disabled)
        .on_press(move |_: &()| on_press())
        .h(px(32.0))
        .px_3()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || theme::Tint::Danger.wash())
        .bg(move || theme::Tint::Danger.wash())
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.8))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme::Tint::Danger.foreground()),
        )
}

/// A count that also filters: the tally strip on a log page, where reading the numbers is
/// the same act as choosing what to look at. Selected wears its own family so the tally and
/// the rows agree.
pub fn filter_pill(
    label: impl Into<SharedString> + 'static,
    count: impl Fn() -> usize + 'static,
    selected: Memo<bool>,
    tint: Tint,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let label = label.into();
    button()
        .selected(move || selected.get())
        .on_press(move |_: &()| on_press())
        .h(px(28.0))
        .px_3()
        .flex_row()
        .items_center()
        .gap_1p5()
        .flex_shrink_0()
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || theme().border)
        .selected_style(|s| s.bg(move || tint.wash()).border_color(move || tint.wash()))
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.8))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if selected.get() {
                        tint.foreground()
                    } else {
                        theme().card_foreground
                    }
                }),
        )
        .child(
            text(move || count().to_string())
                .text_xs()
                .font_family(theme::font_mono())
                .text_color(move || {
                    if selected.get() {
                        tint.foreground()
                    } else {
                        theme::text_faint()
                    }
                }),
        )
}

/// A note that explains the state of the region below it, with nothing to press. Warning,
/// danger and info share this shape so the color carries the severity and the layout stays
/// put.
pub fn inline_note(
    tint: Tint,
    glyph: Icon,
    message: impl IntoReactive<SharedString> + 'static,
) -> impl IntoElement {
    inline_note_with(tint, glyph, message, None::<nsh_core::element::View>)
}

/// The same note with the one action that recovers from the condition.
pub fn inline_note_with(
    tint: Tint,
    glyph: Icon,
    message: impl IntoReactive<SharedString> + 'static,
    action: Option<impl IntoElement>,
) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_2()
        .p_3()
        .rounded(theme::radius_tile())
        .bg(move || tint.wash())
        .child(icon(glyph, 15.0).text_color(move || tint.foreground()))
        .child(
            text(message)
                .flex_1()
                .min_w(px(0.0))
                .text_xs()
                .text_color(move || tint.foreground()),
        )
        .child(action)
}
