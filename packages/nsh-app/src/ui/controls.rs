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

/// A page's one floating action: a circular button at the bottom-right of its own screen.
///
/// One per page, and the page decides which action earns the slot — 总览 starts the proxy, 节点
/// measures latency, 规则 adds a rule, 日志 toggles collection. A control that floats over every
/// screen instead buries the page's own action under a global one, and the user asked for the
/// opposite: the button should belong to what is on screen.
///
/// Icon-only, because the words are what made the old version ugly: a pill wide enough for
/// 正在申请网络权限 is a banner, not a button. The name is still required — it is what a screen
/// reader says, and the only handle a headless test has (`role:button[启动代理]`).
///
/// `lit` carries the page's state in the control's own colour, which is how `packages/ui` draws
/// its power button: a neutral inset disc with no glow while nothing runs, and the accent fill,
/// glow and 6pt halo once it does. An always-orange disc shouts the same thing whether the proxy
/// is up or not, and spends the one colour that could say something.
pub fn floating_action_state(
    glyph: impl Fn() -> Icon + 'static,
    label: impl Fn() -> SharedString + 'static,
    lit: impl Fn() -> bool + Clone + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let skin = move || crate::domain::DiscSkin::for_state(lit());
    // Each binding below takes its own copy of the reader.
    let skin_border = skin.clone();
    let skin_shadow = skin.clone();
    let skin_glyph = skin.clone();
    button()
        .accessibility_label(move || Some(label()))
        .disabled(disabled)
        .on_press(move |_: &()| on_press())
        .h(px(ACTION_DISC))
        .w(px(ACTION_DISC))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_full()
        // Always 1pt, so switching skins moves no pixels — the rule `app.css` states for its
        // own power button.
        .border_1()
        .border_color(move || theme::disc_border(skin_border()))
        .bg(move || theme::disc_fill(skin()))
        .style_with(move |s| s.shadow(theme::action_shadows(skin_shadow())))
        .hover(|s| s.opacity(0.94))
        .active(|s| s.scale(0.94).opacity(0.88))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.55))
        // A finger aims at this one by feel; the slop is invisible and costs no layout.
        .hit_slop(EdgeInsets::all(px(if is_desktop() { 0.0 } else { 6.0 })))
        .child(live_icon(glyph, 24.0).text_color(move || theme::disc_glyph(skin_glyph())))
}

/// The disc's side, and the room a page must leave at its bottom edge.
///
/// A floating control that overlaps the last row of a list hides the thing the reader scrolled
/// for, which is worse than the bar it replaced. Pages end in `action_clearance()` rather than
/// their own guess at the number so the two cannot drift apart.
pub const ACTION_DISC: f32 = 56.0;

/// The gap the disc keeps from the window's bottom and right edges.
///
/// It is a margin on the control rather than padding on the layer, because `pb_safe_area()`
/// replaces the layer's own bottom padding with the device's safe-area inset — 0 on a desktop —
/// and a disc sitting flush against the window border reads as clipped rather than placed.
pub const ACTION_EDGE: f32 = 24.0;

/// Bottom padding that keeps a page's last row clear of its floating action.
pub fn action_clearance() -> Points {
    px(ACTION_DISC + ACTION_EDGE * 2.0)
}

/// The layer a page's floating action sits in: full size, transparent to touches everywhere
/// except the control itself.
///
/// `pointer_events` is what makes this safe; the driver cannot check it, because it resolves a
/// press through the node tree rather than hit-testing pixels, so an occluding layer looks
/// harmless in the suite and the bug appears only in a real window. Verified by eye.
pub fn floating_layer(action: impl IntoElement) -> impl IntoElement {
    overlay()
        .absolute()
        .inset_0()
        .flex_col()
        .items_end()
        .justify_end()
        .p_5()
        // Plain `pb_safe_area()` would replace `p_5`'s own bottom padding with the device's
        // safe-area inset, which is 0 on a desktop — the reason the disc sat flush against the
        // window border. `_plus` keeps both: the inset when there is one, and this gap always.
        .pb_safe_area_plus(px(ACTION_EDGE))
        .pointer_events(PointerEvents::BoxNone)
        .child(
            view()
                // A phone keeps its tab bar along this same bottom edge; lift clear of that
                // too, not just of the border.
                .mb(if is_desktop() { px(0.0) } else { px(64.0) })
                .pointer_events(PointerEvents::Auto)
                .child(action),
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
    switch_row_state(
        label,
        note,
        move || checked.get(),
        move || disabled.get(),
        on_change,
    )
}

/// The same row with the state read through closures, for a value that lives in the store
/// rather than in a signal of its own.
pub fn switch_row_state(
    label: impl Into<SharedString> + 'static,
    note: impl IntoReactive<SharedString> + 'static,
    checked: impl Fn() -> bool + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let control = switch_state(
        {
            let label = label.clone();
            move || Some(label.clone())
        },
        checked,
        disabled,
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
///
/// The parts carry the look, and every part is sized by hand. This control has no look of its
/// own — the switch box, the track and the thumb are three views, and the state styles attach
/// at different levels: `checked_style` paints the *box* and the track's own `checked:` paints
/// the *track*. An unsized track fills the box, which is what turned every checked switch into
/// an orange rectangle with a pale pill floating in the middle of it. See
/// `domain::SwitchGeometry` for why the numbers are not written out here.
pub fn switch_state(
    label: impl Fn() -> Option<SharedString> + 'static,
    checked: impl Fn() -> bool + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    let geo = crate::domain::SwitchGeometry::default();
    debug_assert!(geo.thumb_fits(), "the switch's parts no longer fit");
    switch()
        .accessibility_label(label)
        .checked(checked)
        .disabled(disabled)
        .on_change(move |next: &bool| on_change(*next))
        // The box is only a press target: no radius, no tint, so a checked switch cannot
        // paint a rectangle around the track.
        .h(px(geo.box_height))
        .w(px(geo.box_width))
        .flex_shrink_0()
        .items_center()
        .flex_row()
        // The whole control dims together: `disabled:` on the switch carries its parts, which
        // is what `nsh_base::Switch` documents, and the parts' own style builders have no
        // disabled layer to write.
        .disabled_style(|s| s.opacity(0.45))
        .track_style(|s| {
            s.h(px(geo.track_height))
                .w(px(geo.track_width))
                .flex_shrink_0()
                // A row, so the thumb sits at the near end and the inset is horizontal. As a
                // column — the default — `items_center` would center along x instead, and the
                // thumb would start in the middle of the track with nowhere to slide to.
                .flex_row()
                .items_center()
                .px(px(geo.inset))
                .rounded(px(geo.track_radius))
                .border_1()
                .border_color(move || theme().input)
                .bg(move || theme().muted)
                .checked(|s| {
                    s.bg(move || theme().primary)
                        .border_color(move || theme().primary)
                        // `app.css` gives the rule toggle's checked track a 2pt --accent-soft
                        // ring; here every switch gets it, because a primary track and a cream
                        // thumb need one more edge between them to read as two shapes.
                        .inset_ring(px(2.0))
                        .inset_ring_color(move || theme().accent)
                })
        })
        .thumb_style(|s| {
            s.h(px(geo.thumb))
                .w(px(geo.thumb))
                .flex_shrink_0()
                .rounded_full()
                // The thumb is opaque on purpose: see `theme::switch_thumb`.
                .bg(theme::switch_thumb)
                // …and lifted off the track by the shadow, which in the light appearance is
                // the only thing separating a white thumb from an 82% white track.
                .shadow([theme::switch_thumb_shadow()])
                // The thumb moves by a checked translate, after layout, so the track keeps
                // its width and nothing around the switch shifts as it is flipped.
                .checked(|s| s.translate_x(px(geo.travel())))
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

/// A row that opens and closes a block under it, naming what is inside while closed.
///
/// The app's form-folds use this: a card whose fields all show at once reads as a wall, and the
/// user asked for the sync target to be put away until it is wanted. What makes a fold honest is
/// the summary — closed must still answer "what is in there", so the caller passes words for both
/// states and only the caret changes.
///
/// The whole row is the press target, not just the caret: a 14pt chevron is not a touch target,
/// and on a phone the row is the only thing anyone can hit reliably.
pub fn disclosure_row(
    label: impl Into<SharedString> + 'static,
    summary: impl IntoReactive<SharedString> + 'static,
    open: impl Fn() -> bool + 'static,
    disabled: impl Fn() -> bool + 'static,
    on_toggle: impl Fn() + 'static,
) -> impl IntoElement {
    let name: SharedString = label.into();
    let label_text = name.clone();
    let summary_text = summary.into_reactive();
    button()
        // Named by what it opens, not by what is inside: the summary is on-screen text the
        // accessibility tree already reads out, and folding it into the name would make the
        // control's name change every time an address is typed.
        .accessibility_label(name)
        .disabled(disabled)
        .on_press(move |_: &()| on_toggle())
        .w_full()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap_3()
        .py_2()
        .px_3()
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().muted)
        .hover(|s| s.border_color(move || theme().ring))
        .active(|s| s.opacity(0.85))
        .focus(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label_text)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .flex_shrink_0()
                .text_color(move || theme().card_foreground),
        )
        .child(
            text(move || summary_text.get())
                .flex_1()
                .min_w(px(0.0))
                .text_xs()
                .number_of_lines(1)
                .text_right()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            live_icon(
                move || {
                    if open() {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    }
                },
                14.0,
            )
            .flex_shrink_0()
            .text_color(move || theme().muted_foreground),
        )
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

/// What a nav item says about itself beyond its name: nothing, a count, or an
/// update the user has not looked at.
///
/// One type rather than two optional arguments because the two never appear together — the
/// original keeps them in one slot — and a row carrying both would leave the user to work out
/// which one they were being asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavMark {
    None,
    Count(usize),
    Update,
}

/// The name a marked row gives a screen reader.
///
/// A dot is a coloured circle and a count is a number in a chip; neither says what it means to
/// someone who cannot see it, and a label on a bare span is not reliably announced. The button's
/// own name is the one place that is, so an unmarked row keeps the name its visible text already
/// gives it and an update-marked row says what the dot stands for. A count needs no such
/// translation: the digits are the message, and they are on screen as text.
pub fn nav_name(label: &str, mark: &NavMark) -> Option<SharedString> {
    match mark {
        NavMark::Update => Some(format!("{label}（有新版本可更新）").into()),
        NavMark::None | NavMark::Count(_) => None,
    }
}

/// The mark, in the row's trailing slot.
///
/// Takes the reader rather than the value so the mark is a binding: an update discovered after
/// launch appears in the rail without the page being rebuilt, and the row whose count changed
/// repaints on its own.
fn nav_mark(mark: impl Fn() -> NavMark + 'static) -> impl IntoElement {
    dynamic(move || match mark() {
        NavMark::None => view().w(px(0.0)).flex_shrink_0().into_element(),
        NavMark::Count(count) => view()
            .flex_shrink_0()
            .px_2()
            .py_0p5()
            .rounded(theme::radius_tile())
            .bg(move || theme().muted)
            .child(
                text(move || count.to_string())
                    .text_xs()
                    .font_family(theme::font_mono())
                    .text_color(move || theme().muted_foreground),
            )
            .into_element(),
        // Small, and in the family that already means "worth your attention, nothing is wrong",
        // so an unaddressed update waits rather than shouts.
        NavMark::Update => view()
            .h(px(7.0))
            .w(px(7.0))
            .flex_shrink_0()
            .rounded_full()
            .bg(move || theme::Tint::Warning.foreground())
            .into_element(),
    })
}

/// A sidebar destination. `active` is a memo the caller owns, so every binding
/// here reads one cached answer; `button` supplies role, focus and the selected
/// state, which is what makes the row reachable by keyboard and screen reader.
pub fn nav_row(
    label: &'static str,
    glyph: Icon,
    active: Memo<bool>,
    mark: impl Fn() -> NavMark + Clone + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .selected(move || active.get())
        .accessibility_label({
            let mark = mark.clone();
            move || nav_name(label, &mark())
        })
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
        .child(nav_mark(mark))
}

/// A bottom-bar destination: glyph above a short label, tall enough for a thumb.
///
/// The mark sits beside the glyph rather than in a trailing slot, because a tab has no trailing
/// slot — the label is centred, and anything appended after it would push the glyph off-centre on
/// every tab but the marked one, which reads as a layout bug rather than as a message.
pub fn nav_tab(
    label: &'static str,
    glyph: Icon,
    active: Memo<bool>,
    mark: impl Fn() -> NavMark + Clone + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .selected(move || active.get())
        .accessibility_label({
            let mark = mark.clone();
            move || nav_name(label, &mark())
        })
        .on_press(move |_: &()| on_press())
        .h(px(52.0))
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .selected_style(|s| s.bg(move || theme().accent))
        .active(|s| s.opacity(0.8))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(icon(glyph, 20.0).text_color(move || {
                    if active.get() {
                        theme().accent_foreground
                    } else {
                        theme().muted_foreground
                    }
                }))
                .child(nav_mark(mark)),
        )
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

/// A labeled single-line field whose value lives inside a larger struct rather than in a signal
/// of its own.
///
/// `text_field` is the common case — a signal per field. The sync form edits nine fields of one
/// saved config, and giving each a signal of its own would mean nine copies that can disagree
/// with the config they came from. This variant reads and writes through closures, so the config
/// stays the single owner. `masked` hides what is typed: it exists for the two secret fields,
/// where showing a token on screen is its own leak.
pub fn text_field_state(
    label: &'static str,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(String) + 'static,
    placeholder: impl Fn() -> &'static str + 'static,
    masked: bool,
) -> impl IntoElement {
    view()
        .w_full()
        .flex_col()
        .gap_1p5()
        .child(field_label(label.into()))
        .child(field_frame(
            nsh_base::input()
                .value(value)
                .on_change(move |text: &str| on_change(text.to_owned()))
                .masked(move || masked)
                .placeholder(placeholder)
                .placeholder_color(|| color(theme().muted_foreground))
                .caret_color(|| color(theme().primary))
                .label(label)
                .w_full()
                .min_w(px(0.0))
                .text_sm()
                .text_color(move || theme().card_foreground),
        ))
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

/// The same field with a handle, so another control can put the caret in it.
///
/// This exists for 规则's floating "+": a new-rule button that only clears the form leaves the
/// reader looking at an empty box they still have to tap. The handle is the only way in — NSH
/// has no autofocus and no way to focus a node by selector.
pub fn text_field_focus(
    label: &'static str,
    value: Signal<String>,
    placeholder: impl Fn() -> &'static str + 'static,
    focus: ElementRef,
) -> impl IntoElement {
    view()
        .w_full()
        .flex_col()
        .gap_1p5()
        .child(field_label(label.into()))
        .child(field_frame(
            nsh_base::input()
                .value(move || value.get())
                .on_change(move |text: &str| value.set(text.to_owned()))
                .placeholder(placeholder)
                .placeholder_color(|| color(theme().muted_foreground))
                .caret_color(|| color(theme().primary))
                .label(label)
                .element_ref(&focus)
                .w_full()
                .min_w(px(0.0))
                .text_sm()
                .text_color(move || theme().card_foreground),
        ))
}

/// A labeled single-line field. The label is above rather than a placeholder standing in
/// for it: a placeholder disappears the moment the user types, which is exactly when the
/// field's purpose is least obvious.
pub fn text_field(
    label: &'static str,
    value: Signal<String>,
    placeholder: impl Fn() -> &'static str + 'static,
) -> impl IntoElement {
    text_field_focus(label, value, placeholder, element_ref())
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

/// A row that states something and cannot be changed: a label and note on the left, the value
/// on the right.
///
/// The counterpart to [`switch_row`] for a fact rather than a choice. It matters that the two
/// look alike and only one has a control on it — a row with a value where a switch would be
/// reads as a setting someone forgot to make interactive.
pub fn value_row(
    label: impl Into<SharedString> + 'static,
    note: impl Into<SharedString> + 'static,
    value: impl IntoReactive<SharedString> + 'static,
) -> impl IntoElement {
    let label = label.into();
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
                .child(text(note.into()).text_xs().text_color(theme::text_faint())),
        )
        .child(
            text(value)
                .text_xs()
                .flex_shrink_0()
                .number_of_lines(1)
                .text_color(move || theme().muted_foreground),
        )
}

/// One fact of an about card: the word that names it above, its value below.
///
/// Four of these sit in a wrapped row, which is why the label is above rather than beside the
/// value: a column of label-value pairs reads at a glance, a column of pairs sharing one line
/// does not survive a narrow window.
pub fn about_fact(
    label: impl Into<SharedString> + 'static,
    value: impl IntoReactive<SharedString> + 'static,
) -> impl IntoElement {
    view()
        .flex_col()
        .gap_0p5()
        .child(text(label.into()).text_xs().text_color(theme::text_faint()))
        .child(
            text(value)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().card_foreground),
        )
}

/// A field whose only job is to narrow a list, marked by a leading glyph instead of a label.
///
/// A search box is the one field whose purpose is obvious from what is being typed into, so
/// the label would repeat the placeholder; it is still named for a screen reader, and the
/// glyph is what distinguishes it from a form field at a glance.
pub fn search_field(
    name: &'static str,
    placeholder: &'static str,
    value: Signal<String>,
) -> impl IntoElement {
    view()
        .w_full()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .h(px(36.0))
        .rounded(theme::radius_tile())
        .border_1()
        .border_color(move || theme().input)
        .bg(move || theme().card)
        .focus_within(|s| s.border_color(move || theme().ring))
        .child(icon(Icon::Search, 15.0).text_color(move || theme().muted_foreground))
        .child(
            nsh_base::input()
                .value(move || value.get())
                .on_change(move |text: &str| value.set(text.to_owned()))
                .placeholder(placeholder)
                .placeholder_color(|| color(theme().muted_foreground))
                .caret_color(|| color(theme().primary))
                // The same words as the placeholder: a field with no visible label still has
                // to say what it searches.
                .label(name)
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .text_color(move || theme().card_foreground),
        )
}
