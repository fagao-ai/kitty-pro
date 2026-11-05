//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// U21: monotone generation for update checks. `UpdateMonitor` is built with a
/// struct literal in `ProxyApp` (a file outside this fix's scope), so a new
/// signal field cannot be added there. A process-wide counter shared by every
/// `refresh` call gives the same guarantee: only the newest request may write
/// its answer back. There is exactly one monitor per app run.
static UPDATE_REQUEST_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum UpdateAction {
    Check,
    Dismiss,
    Toggle,
}

/// The app's one owner of update state, provided as context.
///
/// `ProxyApp` drives it so a check happens on launch whether or not the user ever
/// opens the settings page - the shape Magpie's `UpdateService` has, with the
/// service living beside the window instead of inside an about page. The settings
/// card is a view over these signals, which is what lets the sidebar dot, the
/// card, and the launch toast agree without three reads of one endpoint.
///
/// Every field is a signal, including the two strings, so the monitor is `Copy`
/// and any event handler can take it by value.
#[derive(Clone, Copy)]
pub(crate) struct UpdateMonitor {
    pub(crate) status: Signal<Option<api::update::AppUpdateStatus>>,
    pub(crate) saved: Signal<api::update::UpdateSettings>,
    pub(crate) busy: Signal<Option<UpdateAction>>,
    pub(crate) ready: Signal<bool>,
    /// Version the launch toast already announced, so a re-check that finds the
    /// same release cannot stack a second toast.
    pub(crate) announced: Signal<Option<String>>,
    /// Why the last check never reached the answer stage, if it did not.
    ///
    /// Kept apart from `status`, which only ever holds a real answer: the
    /// transport error has to survive long enough for the card to show it, and a
    /// toast is the wrong place for a failure the user did not ask about.
    pub(crate) error: Signal<Option<String>>,
    pub(crate) platform: Signal<String>,
    pub(crate) version: Signal<String>,
}

impl UpdateMonitor {
    pub(crate) fn view(self) -> Option<api::update::AppUpdateStatus> {
        (self.status)().clone()
    }

    pub(crate) fn state(self) -> api::update::AppUpdateState {
        self.view()
            .map(|value| value.state)
            .unwrap_or(api::update::AppUpdateState::Idle)
    }

    pub(crate) fn latest(self) -> Option<api::update::AppRelease> {
        self.view().and_then(|value| value.latest.clone())
    }

    /// A newer release the user has not dismissed.
    pub(crate) fn has_actionable_update(self) -> bool {
        self.state() == api::update::AppUpdateState::Available
            && !self.view().is_some_and(|value| value.snoozed)
    }

    pub(crate) fn is_checking(self) -> bool {
        (self.busy)() == Some(UpdateAction::Check)
    }

    /// Reason the last check failed to produce an answer, if it did.
    pub(crate) fn failure(self) -> Option<String> {
        (self.error)().clone()
    }

    /// What the badge reports. See [`resolve_display_state`].
    pub(crate) fn display_state(self) -> api::update::AppUpdateState {
        resolve_display_state(self.view().as_ref(), (self.error)().as_deref())
    }

    pub(crate) fn is_idle(self) -> bool {
        (self.busy)().is_none()
    }

    /// Whether the remembered settings have been read at least once. Until then
    /// every action is disabled, because "no answer yet" and "no update" would
    /// otherwise render identically.
    pub(crate) fn has_loaded(self) -> bool {
        (self.ready)()
    }

    pub(crate) fn platform_text(self) -> String {
        (self.platform)()
    }

    pub(crate) fn version_text(self) -> String {
        (self.version)()
    }

    pub(crate) fn settings(self) -> api::update::UpdateSettings {
        (self.saved)()
    }

    /// Ask the server for an answer. `force` ignores the daily throttle.
    ///
    /// U21: every call takes a fresh generation id. A slow launch check (A)
    /// that returns after a faster manual check (B) started is stale and must
    /// not overwrite B's `status`/`error`, fire its toast, or clear `busy`
    /// while B is still running. Only the newest generation may write back.
    pub(crate) async fn refresh(mut self, force: bool, toast: ToastManager, announce: bool) {
        self.busy.set(Some(UpdateAction::Check));
        let request_id = UPDATE_REQUEST_ID.fetch_add(1, Ordering::SeqCst) + 1;
        let request = api::update::AppUpdateRequest {
            platform: self.platform_text(),
            current_version: self.version_text(),
            force,
        };
        let answer = api::update::check_app_update(request).await;
        // Superseded: discard the whole result (answer, error, toast and busy
        // clearing) so the newer request owns the final state.
        if request_id != UPDATE_REQUEST_ID.load(Ordering::SeqCst) {
            return;
        }
        match answer {
            Ok(answer) => {
                let release = answer.latest.clone();
                let available =
                    answer.state == api::update::AppUpdateState::Available && !answer.snoozed;
                // The answer replaces any transport error from an earlier attempt.
                self.error.set(None);
                self.status.set(Some(answer));
                if announce {
                    self.announce(available, release, toast);
                }
            }
            Err(error) => {
                // Kept for the card to render as "检查失败" plus the reason, which
                // is where a failure belongs whether or not anybody is watching.
                self.error.set(Some(error.to_string()));
                // Only a check the user started gets interrupted by a toast.
                //
                // This branch is the transport, not the network: an unreachable
                // GitHub comes back as `Ok` with `state: Failed` and a reason the
                // card already prints. What lands here instead is a state file
                // that cannot be read or written, or a server that cannot be
                // reached at all - which a launch would surface as a red banner
                // over an app the user was busy opening, on every launch, for a
                // fault the card already reports in its own row.
                if force {
                    toast.error(format!("检查更新失败: {error}"));
                }
            }
        }
        self.busy.set(None);
    }

    /// Report a check to the user, at most once per release.
    ///
    /// A launch check stays quiet unless there is something to act on: the card
    /// saying "already latest" is enough, and a toast for it would be noise on
    /// every start.
    pub(crate) fn announce(
        mut self,
        available: bool,
        release: Option<api::update::AppRelease>,
        toast: ToastManager,
    ) {
        let Some(release) = release.filter(|_| available) else {
            return;
        };
        if (self.announced)().as_deref() == Some(release.version.as_str()) {
            return;
        }
        self.announced.set(Some(release.version.clone()));
        toast.info(format!("发现新版本 v{}", release.version));
    }
}

/// Update card: the GitHub release check, and the only place in the app that
/// offers to leave it.
///
/// Everything it renders comes from the [`UpdateMonitor`] in context, which
/// `ProxyApp` mounted at startup. Nothing here downloads or installs: the release
/// page owns the installers and their checksums, and every package this project
/// ships needs privileges the app does not have (a signed `.dmg`, an MSI,
/// Android's own installer), so the last step is a link the user can read first.
#[component]
pub(crate) fn UpdateSettings() -> Element {
    let toast = use_context::<ToastManager>();
    let mut monitor = use_context::<UpdateMonitor>();
    let mut confirm_dismiss = use_signal(|| false);
    let mut notes_open = use_signal(|| false);

    let view = monitor.view();
    let state = monitor.display_state();
    let latest = monitor.latest();
    let update_ready = monitor.has_actionable_update();
    let settings = monitor.settings();
    // A check can fail two ways: it reached GitHub and has an answer to keep
    // beside the reason, or it never got that far and only the monitor remembers.
    // The card shows either, which is what makes not toasting a background
    // failure an honest trade rather than a swallowed one.
    let error = view
        .as_ref()
        .and_then(|value| value.error.clone())
        .or_else(|| monitor.failure());
    let platform = monitor.platform_text();
    let app_version = monitor.version_text();
    // Clock the age labels are measured against, read together with the stamp it
    // is compared to so the age cannot drift between two reads. A browser client
    // has no clock of its own and must not read one: that traps in `wasm32`.
    let server_time = view.as_ref().map_or(0, |value| value.server_time);
    // A release with nothing published for this machine still gets a card: the
    // version and the reason are the useful part, and the download link would be
    // a dead end.
    let show_asset_note = latest.is_some()
        && latest
            .as_ref()
            .is_some_and(|release| release.asset.is_none());

    rsx! {
        section { class: "settings-section update-settings glass-surface",
            div { class: "section-heading",
                div {
                    p { class: "eyebrow", "UPDATES" }
                    h2 { "检查更新" }
                }
                span { class: update_badge_class(state, update_ready),
                    {update_badge_label(
                        state,
                        monitor.is_checking(),
                        monitor.has_loaded(),
                        view.as_ref().is_some_and(|value| value.snoozed),
                    )}
                }
            }

            div { class: "setting-row",
                span { class: "setting-icon", Icon { icon: LdRocket, width: 19, height: 19 } }
                div {
                    strong { {format!("当前版本 v{app_version}")} }
                    small {
                        if let Some(answer) = view.as_ref() {
                            {format!("检查目标 {}", answer.target)}
                        } else {
                            {platform.clone()}
                        }
                    }
                }
                if let Some(release) = latest.as_ref() {
                    span { class: "setting-value", {format!("最新 v{}", release.version)} }
                }
            }

            label { class: "setting-row toggle-row",
                span { class: "setting-icon",
                    if settings.auto_check {
                        Icon { icon: LdBellRing, width: 19, height: 19 }
                    } else {
                        Icon { icon: LdBellOff, width: 19, height: 19 }
                    }
                }
                div {
                    strong { "自动检查更新" }
                    small {
                        {auto_check_label(
                            settings.auto_check,
                            view.as_ref().map_or(0, |value| value.last_check_at),
                            server_time,
                        )}
                    }
                }
                Switch {
                    checked: settings.auto_check,
                    label: "自动检查更新".to_string(),
                    disabled: !monitor.is_idle(),
                    onchange: move |enabled| {
                        spawn(async move {
                            monitor.busy.set(Some(UpdateAction::Toggle));
                            let mut next = monitor.settings();
                            next.auto_check = enabled;
                            match api::update::save_update_settings(next).await {
                                Ok(saved) => {
                                    monitor.saved.set(saved);
                                    if enabled {
                                        // Turning the switch back on should say
                                        // whether anything is waiting, right now.
                                        monitor.refresh(false, toast, true).await;
                                    } else {
                                        toast.success("已关闭自动检查，仍可手动检查");
                                    }
                                }
                                Err(error) => {
                                    toast.error(format!("更新设置保存失败: {error}"));
                                }
                            }
                            monitor.busy.set(None);
                        });
                    },
                }
            }

            if state == api::update::AppUpdateState::Unsupported {
                div { class: "inline-note",
                    Icon { icon: LdInfo, width: 16, height: 16 }
                    span { "此客户端不通过 GitHub 发布安装包，请从分发它的渠道更新" }
                }
            }

            if let Some(release) = latest.as_ref() {
                div { class: "update-release",
                    div { class: "update-release-head",
                        div {
                            strong { {release.title.clone()} }
                            small {
                                {release_subtitle(
                                    &release.title,
                                    &release.version,
                                    release_age_label(
                                        release.published_at.as_deref(),
                                        server_time,
                                    ),
                                )}
                                if release.prerelease { " · 预览版" }
                            }
                        }
                        if update_ready {
                            span { class: "status-badge pending", "可更新" }
                        } else if view.as_ref().is_some_and(|value| value.snoozed) {
                            span { class: "status-badge", "已忽略" }
                        }
                    }
                    if let Some(asset) = release.asset.as_ref() {
                        p { class: "update-asset",
                            Icon { icon: LdPackage, width: 14, height: 14 }
                            span { "{asset}" }
                            if release.asset_size > 0 {
                                small { {format_data_amount(release.asset_size)} }
                            }
                        }
                    } else {
                        // Newer, but nothing published for this OS/CPU pair: an
                        // exact answer beats a download button that cannot work.
                        p { class: "update-asset",
                            Icon { icon: LdCircleAlert, width: 14, height: 14 }
                            span { "此发布未包含适配本机的安装包" }
                        }
                    }
                    if release.notes.is_some() {
                        button {
                            class: "text-button",
                            aria_expanded: "{notes_open()}",
                            onclick: move |_| notes_open.toggle(),
                            {if notes_open() { "收起更新说明" } else { "查看更新说明" }}
                        }
                        if let Some(notes) = release.notes.as_ref() {
                            if notes_open() {
                                pre { class: "update-notes", "{notes}" }
                            }
                        }
                    }
                }
            }

            if let Some(message) = error.clone() {
                div { class: "inline-note", role: "alert",
                    Icon { icon: LdCircleAlert, width: 16, height: 16 }
                    span { "{message}" }
                }
            }

            div { class: "update-actions",
                button {
                    class: "secondary-button compact",
                    disabled: !monitor.is_idle() || !monitor.has_loaded(),
                    aria_busy: monitor.is_checking(),
                    onclick: move |_| {
                        spawn(async move {
                            // A manual check answers whatever it finds, including
                            // "nothing newer".
                            monitor.refresh(true, toast, true).await;
                        });
                    },
                    if monitor.is_checking() { span { class: "spinner" } }
                    else { Icon { icon: LdRefreshCw, width: 16, height: 16 } }
                    "检查更新"
                }
                if let Some(url) = latest.as_ref().map(|release| release.url.clone()) {
                    // A real anchor rather than a button that evals an opener.
                    //
                    // In a browser this is a plain link, and `target="_blank"` is
                    // what sends the release page to a background tab instead of
                    // navigating away from the console.
                    //
                    // On desktop and Android the same markup reaches the system
                    // browser by a different route: the native interpreter
                    // (`dioxus-interpreter-js`'s `handleClickNavigate`) calls
                    // `preventDefault` on any click that lands on an `<a>` and
                    // sends the href over IPC as `browser_open`, which
                    // `dioxus-desktop` hands to `webbrowser::open` - an Android
                    // `ACTION_VIEW` intent there. So `target` is inert on those
                    // shells. The one thing that must not be added here is an
                    // `onclick` that returns `prevent_default`: that suppression
                    // is what skips the interception, and the navigation is
                    // already cancelled - so the click would simply do nothing.
                    a {
                        class: "primary-button compact",
                        href: "{url}",
                        target: "_blank",
                        rel: "noreferrer",
                        Icon { icon: LdArrowUpRight, width: 16, height: 16 }
                        {if update_ready { "前往下载" } else { "发布页" }}
                    }
                }
                if update_ready {
                    if confirm_dismiss() {
                        button {
                            class: "secondary-button compact danger-text-button",
                            disabled: !monitor.is_idle(),
                            onclick: move |_| {
                                let Some(release) = latest.clone() else { return };
                                spawn(async move {
                                    monitor.busy.set(Some(UpdateAction::Dismiss));
                                    let result =
                                        api::update::dismiss_app_update(release.version.clone()).await;
                                    match result {
                                        Ok(_) => {
                                            confirm_dismiss.set(false);
                                            if let Some(mut answer) = (monitor.status)() {
                                                answer.snoozed = true;
                                                monitor.status.set(Some(answer));
                                            }
                                            toast.success(format!("已忽略 v{}", release.version));
                                        }
                                        Err(error) => toast.error(format!("忽略失败: {error}")),
                                    }
                                    monitor.busy.set(None);
                                });
                            },
                            Icon { icon: LdX, width: 16, height: 16 }
                            "确认忽略此版本"
                        }
                        button {
                            class: "secondary-button compact",
                            disabled: !monitor.is_idle(),
                            onclick: move |_| confirm_dismiss.set(false),
                            "取消"
                        }
                    } else {
                        button {
                            class: "secondary-button compact",
                            title: "直到出现更新的版本前不再提示",
                            disabled: !monitor.is_idle(),
                            onclick: move |_| confirm_dismiss.set(true),
                            Icon { icon: LdBellOff, width: 16, height: 16 }
                            "忽略此版本"
                        }
                    }
                }
            }

            if show_asset_note {
                div { class: "inline-note sync-security-note",
                    Icon { icon: LdInfo, width: 16, height: 16 }
                    span {
                        {format!("已按 {platform} 的操作系统与 CPU 匹配安装包；发布页仍可查看其他平台的包")}
                    }
                }
            }
        }
    }
}

/// The state the badge shows, given whatever the monitor holds.
///
/// A check that never produced an answer - a state file that could not be read, a
/// server that could not be reached - has no `AppUpdateStatus` to read a state
/// out of. That must not render as "尚未检查", which is what it said a moment
/// before: since a background failure no longer raises a toast, the badge is the
/// only place it can appear, and this is the mapping that keeps it visible.
pub(crate) fn resolve_display_state(
    answer: Option<&api::update::AppUpdateStatus>,
    failure: Option<&str>,
) -> api::update::AppUpdateState {
    match answer {
        Some(answer) => answer.state,
        None if failure.is_some() => api::update::AppUpdateState::Failed,
        None => api::update::AppUpdateState::Idle,
    }
}

pub(crate) fn update_badge_class(
    state: api::update::AppUpdateState,
    update_ready: bool,
) -> &'static str {
    if update_ready {
        "status-badge pending"
    } else {
        match state {
            api::update::AppUpdateState::UpToDate => "status-badge ready",
            api::update::AppUpdateState::Failed => "status-badge offline",
            _ => "status-badge",
        }
    }
}

pub(crate) fn update_badge_label(
    state: api::update::AppUpdateState,
    checking: bool,
    loaded: bool,
    snoozed: bool,
) -> &'static str {
    if checking {
        "检查中"
    } else if !loaded {
        "读取中"
    } else if snoozed {
        // The header would otherwise read "有更新" over a card that says "已忽略".
        "已忽略"
    } else {
        match state {
            api::update::AppUpdateState::Available => "有更新",
            api::update::AppUpdateState::UpToDate => "已是最新",
            api::update::AppUpdateState::Unsupported => "不支持",
            api::update::AppUpdateState::Failed => "检查失败",
            api::update::AppUpdateState::Idle => "尚未检查",
        }
    }
}

/// Release line of the card: the title, then the version and age unless the
/// title already is the version.
///
/// GitHub lets a release be named anything, and this project titles them exactly
/// like their tags, so printing both lines would read "v1.0.9 / v1.0.9 · 13 天前".
/// The version only earns its own place when it adds something.
pub(crate) fn release_subtitle(title: &str, version: &str, age: String) -> String {
    let names_version = {
        let trimmed = title.trim().trim_start_matches(['v', 'V']);
        trimmed.eq_ignore_ascii_case(version.trim())
    };
    if names_version {
        age
    } else {
        format!("v{version} · {age}")
    }
}

/// Elapsed time in the largest unit that still reads as "recent".
pub(crate) fn relative_age(seconds_ago: u64) -> String {
    if seconds_ago == 0 {
        return "刚刚".to_string();
    }
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if seconds_ago < MINUTE {
        format!("{seconds_ago} 秒前")
    } else if seconds_ago < HOUR {
        format!("{} 分钟前", seconds_ago / MINUTE)
    } else if seconds_ago < DAY {
        format!("{} 小时前", seconds_ago / HOUR)
    } else {
        format!("{} 天前", seconds_ago / DAY)
    }
}

/// The auto-check row's subtitle: how often it runs, and when it last ran.
///
/// The age is omitted while no check has answered yet, because the only usable
/// "now" is the one that came back with an answer.
pub(crate) fn auto_check_label(auto_check: bool, last_check_at: u64, server_time: u64) -> String {
    if !auto_check {
        return "仅手动检查，启动时不再联网".to_string();
    }
    match age_label(last_check_at, server_time) {
        Some(age) => format!("每天一次 · {age}"),
        None => "每天一次 · 启动时检查".to_string(),
    }
}

/// Age of an event stamped `then`, measured against `now`.
///
/// `None` when the answering clock is unknown. Both numbers arrive from the
/// server: a browser client compiled to `wasm32` has no platform clock, and
/// reading one there traps (see the uptime note in `ProxyApp`), so every age the
/// card prints is measured by the machine that checked GitHub. Zero for `then`
/// means "never", not "the epoch".
pub(crate) fn age_label(then: u64, now: u64) -> Option<String> {
    if now == 0 {
        return None;
    }
    Some(if then == 0 {
        "从未检查".to_string()
    } else {
        relative_age(now.saturating_sub(then))
    })
}

/// Age of a release from the RFC 3339 stamp GitHub sends, measured against `now`.
/// An unparseable stamp or an unknown clock says "发布时间未知" rather than a
/// wrong date.
pub(crate) fn release_age_label(published_at: Option<&str>, now: u64) -> String {
    match (now > 0)
        .then(|| published_at.and_then(proxy_core::unix_from_rfc3339))
        .flatten()
    {
        Some(unix) => relative_age(now.saturating_sub(unix)),
        None => "发布时间未知".to_string(),
    }
}
