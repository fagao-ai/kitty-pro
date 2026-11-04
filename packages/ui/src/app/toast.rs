//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

pub(crate) const TOAST_DISMISS_MILLIS: u64 = 4_500;

pub(crate) const TOAST_SWEEP_MILLIS: u64 = 250;

pub(crate) const TOAST_DISMISS_TICKS: u32 = (TOAST_DISMISS_MILLIS / TOAST_SWEEP_MILLIS) as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToastMessage {
    pub(crate) id: u64,
    pub(crate) kind: ToastKind,
    pub(crate) message: String,
    pub(crate) born_tick: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct ToastManager {
    pub(crate) messages: Signal<Vec<ToastMessage>>,
    pub(crate) next_id: Signal<u64>,
    pub(crate) tick: Signal<u64>,
}

impl ToastManager {
    pub(crate) fn info(self, message: impl Into<String>) {
        self.push(ToastKind::Info, message.into());
    }

    pub(crate) fn success(self, message: impl Into<String>) {
        self.push(ToastKind::Success, message.into());
    }

    pub(crate) fn error(self, message: impl Into<String>) {
        self.push(ToastKind::Error, message.into());
    }

    pub(crate) fn push(self, kind: ToastKind, message: String) {
        let mut messages = self.messages;
        {
            let mut stored = messages.write();
            // Same text arriving again (Android start taps, repeated poll
            // failures) refreshes the existing toast's lifetime instead of
            // stacking duplicates of one notice.
            if let Some(existing) = stored
                .iter_mut()
                .find(|toast| toast.kind == kind && toast.message == message)
            {
                existing.born_tick = *self.tick.peek();
                return;
            }
            let id = {
                let mut next_id = self.next_id;
                let mut value = next_id.write();
                *value = value.wrapping_add(1);
                *value
            };
            if stored.len() >= 4 {
                stored.remove(0);
            }
            stored.push(ToastMessage {
                id,
                kind,
                message,
                born_tick: *self.tick.peek(),
            });
        }
    }

    /// 由根组件（ProxyApp）调用一次：在根 scope 中周期性清理过期 toast。
    /// 不能在 push 的调用方 scope 里 spawn 定时任务——组件卸载（切页、弹窗
    /// 关闭、列表行删除）会取消任务，导致 toast 永远残留。
    pub(crate) fn start_sweeper(self) {
        let mut messages = self.messages;
        let mut tick = self.tick;
        spawn(async move {
            loop {
                wait_for_toast_sweep().await;
                if messages.peek().is_empty() {
                    continue;
                }
                let now = {
                    let mut value = tick.write();
                    *value = value.wrapping_add(1);
                    *value
                };
                // 所有 toast 展示时长相同，按 push 顺序过期；只在真的有
                // toast 到期时才写 messages，避免每 tick 无谓重渲染视口。
                let has_expired = messages
                    .peek()
                    .iter()
                    .any(|toast| now.wrapping_sub(toast.born_tick) >= TOAST_DISMISS_TICKS as u64);
                if has_expired {
                    messages.write().retain(|toast| {
                        now.wrapping_sub(toast.born_tick) < TOAST_DISMISS_TICKS as u64
                    });
                }
            }
        });
    }

    pub(crate) fn dismiss(self, id: u64) {
        let mut messages = self.messages;
        messages.write().retain(|toast| toast.id != id);
    }

    #[cfg(target_os = "android")]
    pub(crate) fn contains(self, message: &str) -> bool {
        self.messages
            .peek()
            .iter()
            .any(|toast| toast.message == message)
    }

    #[cfg(target_os = "android")]
    pub(crate) fn dismiss_message(self, message: &str) {
        let mut messages = self.messages;
        messages.write().retain(|toast| toast.message != message);
    }
}

#[component]
pub(crate) fn ToastViewport() -> Element {
    let toast = use_context::<ToastManager>();

    rsx! {
        div {
            class: "toast-viewport",
            aria_live: "polite",
            aria_relevant: "additions",
            for item in (toast.messages)() {
                div {
                    // Sweepers remove middle entries; keyed rows keep their
                    // dismiss buttons bound to the right message.
                    key: "{item.id}",
                    class: match item.kind {
                        ToastKind::Info => "toast toast-info",
                        ToastKind::Success => "toast toast-success",
                        ToastKind::Error => "toast toast-error",
                    },
                    role: if item.kind == ToastKind::Error { "alert" } else { "status" },
                    span { class: "toast-icon",
                        match item.kind {
                            ToastKind::Info => rsx! { Icon { icon: LdInfo, width: 18, height: 18 } },
                            ToastKind::Success => rsx! { Icon { icon: LdCircleCheck, width: 18, height: 18 } },
                            ToastKind::Error => rsx! { Icon { icon: LdCircleAlert, width: 18, height: 18 } },
                        }
                    }
                    span { class: "toast-message", "{item.message}" }
                    button {
                        class: "toast-close",
                        title: "关闭提示",
                        aria_label: "关闭提示",
                        onclick: move |_| toast.dismiss(item.id),
                        Icon { icon: LdX, width: 16, height: 16 }
                    }
                }
            }
        }
    }
}
