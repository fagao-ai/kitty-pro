//! Transient feedback: what a press actually did.
//!
//! Every action on the subscription, rule and log screens touches disk or the core, and both
//! can fail. A silent failure reads as success, so each path reports one of these. Repeated
//! identical notices refresh a single toast rather than stacking, which matters for a poll
//! failure that would otherwise arrive once a second.

use crate::ui::icons::Icon;
use crate::ui::theme::{self as tokens, Tint};
use nsh_base::{
    theme, toast, toast_stack, Anchor, ToastManager, ToastMotion, ToastOptions, ToastStackState,
    ToastTransitionStatus,
};
use nsh_core::prelude::*;
use nsh_core::time::now;
use std::time::Duration;

/// How long a notice stays up: long enough to read a provider's error string.
const VISIBLE: Duration = Duration::from_secs(5);

/// The lifecycle tick. Half the motion's duration, so a toast enters and leaves without a
/// visible step.
const TICK: Duration = Duration::from_millis(50);

/// At most this many at once; older ones are pushed out of the stack.
const VISIBLE_LIMIT: usize = 3;

/// What a notice means, and the color that says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Success,
    Error,
}

impl Kind {
    fn tint(self) -> Tint {
        match self {
            Kind::Info => Tint::Info,
            Kind::Success => Tint::Success,
            Kind::Error => Tint::Danger,
        }
    }

    /// The icon that carries the meaning for someone who cannot see color.
    fn glyph(self) -> Icon {
        match self {
            Kind::Info => Icon::Info,
            Kind::Success => Icon::CircleCheck,
            Kind::Error => Icon::CircleAlert,
        }
    }
}

/// A notice: its meaning and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Notice {
    kind: Kind,
    text: String,
}

/// The hub every screen reports through.
///
/// `Copy`, like the store, so a handler captures it the way it captures a signal. The
/// manager is held in `stored`, since `nsh_base`'s `ToastManager` is owned mutable state
/// rather than a signal; `shown` is what the stack renders, so a new notice re-runs only
/// the stack's binding.
#[derive(Clone, Copy)]
pub struct Feedback {
    manager: Stored<ToastManager<u64, Notice>>,
    shown: Signal<Vec<(u64, Notice, ToastTransitionStatus)>>,
    next_id: Stored<u64>,
    stack: ToastStackState,
}

impl Feedback {
    /// Create the hub and start the clock that ages notices out. Call once, in the root.
    pub fn new() -> Self {
        let manager = stored(ToastManager::<u64, Notice>::new(ToastMotion::sonner()));
        let shown = signal(Vec::new());
        let next_id = stored(0u64);
        let stack = ToastStackState::new();

        let sync = {
            move || {
                let snapshot = manager.with(|manager| {
                    manager
                        .visible(VISIBLE_LIMIT)
                        .map(|(id, notice, status)| (*id, notice.clone(), status))
                        .collect::<Vec<_>>()
                });
                shown.set(snapshot);
            }
        };

        // The interval belongs to the root, which is the owner that made it, so it stops
        // when the app does. Pausing while expanded keeps a stack the pointer is resting
        // on from ageing away mid-read.
        set_interval(TICK, {
            move || {
                let advanced = manager.with(|m| m.advance(now(), stack.is_expanded()));
                if advanced.changed {
                    sync();
                }
            }
        });

        Self {
            manager,
            shown,
            next_id,
            stack,
        }
    }

    pub fn info(self, message: impl Into<String>) {
        self.push(Kind::Info, message.into());
    }

    pub fn success(self, message: impl Into<String>) {
        self.push(Kind::Success, message.into());
    }

    pub fn error(self, message: impl Into<String>) {
        self.push(Kind::Error, message.into());
    }

    /// Report both outcomes of a call whose result is only worth announcing. On success the
    /// `Ok` carries the words to say; on failure the action names what was attempted. For a
    /// call whose value the caller goes on to use, [`Self::report`] instead.
    ///
    /// Distinct from `report` on purpose: that one returns the value and stays quiet on
    /// success, so using it to announce a finished action drops the message the user pressed
    /// the button to get.
    pub fn settled<T: std::fmt::Display, E: std::fmt::Display>(
        self,
        action: &str,
        result: Result<T, E>,
    ) {
        match result {
            Ok(message) => self.success(message.to_string()),
            Err(error) => self.error(format!("{action}失败：{error}")),
        }
    }

    /// Report the outcome of a backend call, choosing the kind and prefixing the failure
    /// with what was attempted. Every screen ends up saying it in the same shape.
    ///
    /// Silent on success, because the caller keeps the value: a screen that wants the user to
    /// hear about the success wants [`Self::settled`].
    pub fn report<T, E: std::fmt::Display>(self, action: &str, result: Result<T, E>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.error(format!("{action}失败：{error}"));
                None
            }
        }
    }

    /// Add a notice. One already up with the same meaning and wording keeps its slot: the
    /// id is reused and `ToastManager::push` replaces the value and restarts the timer.
    /// Without that, a poll failing once a second would stack a new copy every second.
    fn push(self, kind: Kind, text: String) {
        let notice = Notice { kind, text };
        let existing = self.manager.with(|m| {
            m.iter()
                .find(|(_, held, _)| **held == notice)
                .map(|(id, _, _)| *id)
        });
        let id = match existing {
            Some(id) => id,
            None => self.next_id.with(|n| {
                *n = n.wrapping_add(1);
                *n
            }),
        };
        self.manager
            .with(|m| m.push(id, notice, ToastOptions::timeout(Some(VISIBLE)), now()));
        self.resync();
    }

    /// The stack, mounted once by the root over everything else on the surface.
    pub fn stack(self) -> impl IntoElement {
        overlay()
            .absolute()
            .inset_0()
            .flex_col()
            .items_end()
            .justify_end()
            .gap_2()
            .p_5()
            .pb_safe_area()
            .pointer_events(PointerEvents::BoxNone)
            .child(
                toast_stack()
                    .placement(Anchor::BottomCenter)
                    .state(self.stack)
                    // Full width up to a readable measure: a fixed 420pt would overflow a
                    // phone, and a fixed narrow column wastes a desktop.
                    .w_full()
                    .max_w(px(420.0))
                    .on_cancel(move || {
                        self.dismiss_newest();
                    })
                    .child(each(
                        self.shown,
                        |(id, _, _)| *id,
                        move |item, _| {
                            let id = move || item.get().0;
                            toast()
                                .transition_status(move || item.get().2)
                                .accessibility_label(move || Some(item.get().1.text.clone().into()))
                                .on_dismiss_request(move || self.dismiss(id()))
                                .pointer_events(PointerEvents::Auto)
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .px_4()
                                .py_3()
                                .rounded(tokens::radius_card())
                                .border_1()
                                .border_color(move || theme().border)
                                .bg(move || theme().popover)
                                .shadow_lg()
                                .child(
                                    crate::ui::controls::icon(item.get().1.kind.glyph(), 16.0)
                                        .text_color(move || item.get().1.kind.tint().foreground()),
                                )
                                .child(
                                    text(move || item.get().1.text.clone())
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_sm()
                                        .text_color(move || theme().popover_foreground),
                                )
                                .child(crate::ui::controls::icon_button(
                                    move || crate::ui::icons::Icon::X,
                                    move || "关闭提示".into(),
                                    move || self.dismiss(id()),
                                ))
                        },
                    )),
            )
    }

    fn dismiss(self, id: u64) {
        self.manager.with(|m| m.dismiss(&id, now()));
        self.resync();
    }

    /// Escape and Android back dismiss the newest notice, the way the platform dismisses
    /// the top of a stack.
    fn dismiss_newest(self) {
        let newest = self.shown.get_untracked().last().map(|(id, _, _)| *id);
        if let Some(id) = newest {
            self.dismiss(id);
        }
    }

    fn resync(self) {
        let snapshot = self.manager.with(|m| {
            m.visible(VISIBLE_LIMIT)
                .map(|(id, notice, status)| (*id, notice.clone(), status))
                .collect::<Vec<_>>()
        });
        self.shown.set(snapshot);
    }
}

impl Default for Feedback {
    fn default() -> Self {
        Self::new()
    }
}
