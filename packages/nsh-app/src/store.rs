//! The app's state: signals with private writers, read by the screens.
//!
//! One owner per piece of state. The screens call these methods and never touch a signal
//! directly, so what a press starts stays in one place and cancels the way the code says.

pub use crate::domain::SystemProxy;
use api::{ApiCoreStatus, CoreTraffic};
use nsh_core::prelude::*;
use proxy_core::{
    AppProfile, ConnectionRequest, CustomRuleAction, CustomRuleMatch, ProxyNode, TunnelMode,
};
use std::collections::HashMap;
use std::time::Duration;

/// The screen the shell shows. Desktop and phone share the set; only the chrome differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Nodes,
    Subscriptions,
    Rules,
    Logs,
    Settings,
}

impl Screen {
    pub fn label(self) -> &'static str {
        match self {
            Screen::Overview => "总览",
            Screen::Nodes => "节点",
            Screen::Subscriptions => "订阅",
            Screen::Rules => "规则",
            Screen::Logs => "日志",
            Screen::Settings => "设置",
        }
    }

    pub fn icon(self) -> crate::ui::icons::Icon {
        use crate::ui::icons::Icon;
        match self {
            Screen::Overview => Icon::LayoutDashboard,
            Screen::Nodes => Icon::GitFork,
            Screen::Subscriptions => Icon::Rss,
            Screen::Rules => Icon::Split,
            Screen::Logs => Icon::ScrollText,
            Screen::Settings => Icon::SlidersHorizontal,
        }
    }

    /// Order in the sidebar and the phone tab bar.
    pub const ALL: [Screen; 6] = [
        Screen::Overview,
        Screen::Nodes,
        Screen::Subscriptions,
        Screen::Rules,
        Screen::Logs,
        Screen::Settings,
    ];
}

/// Why the core control is inert, so one flag can disable several affordances at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreBusy {
    Idle,
    Toggling,
    Restarting,
    Tun,
}

/// Traffic over the last minute, one sample per poll, for the sparkline.
///
/// The core reports cumulative counters, so the history holds the *difference* between
/// consecutive reads: feeding the raw totals to a chart draws a ramp that only ever goes
/// up, which looks like busy traffic while the connection is idle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrafficWindow {
    /// Cumulative counters as the core reports them.
    pub upload: u64,
    pub download: u64,
    /// The newest sample, for a rate readout.
    pub upload_per_second: u64,
    pub download_per_second: u64,
    pub active_connections: u32,
    /// Bytes per second, oldest first, newest last.
    pub upload_history: Vec<u64>,
    /// Bytes per second, oldest first, newest last.
    pub download_history: Vec<u64>,
}

const HISTORY_LEN: usize = 60;

/// Log rows kept in memory. The core streams continuously, so an unbounded buffer would
/// grow for as long as the page stayed open; the oldest rows fall off the top.
pub const LOG_BUFFER: usize = 500;

/// Which refresh a subscription button is waiting on, so the right one shows a spinner
/// when both a single row and the header can start work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshTarget {
    One(u64),
    All,
}

impl RefreshTarget {
    /// Whether this target covers `id`, for a row deciding whether it is the one working.
    pub fn covers(self, id: u64) -> bool {
        match self {
            RefreshTarget::One(target) => target == id,
            RefreshTarget::All => true,
        }
    }
}

/// Whether a refresh was asked for by one row or by the header, which decides what the
/// completion notice says and which button parks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshScope {
    One,
    All,
}

/// The app's state. `Copy`, so a screen captures it the way it captures a signal.
#[derive(Clone, Copy)]
pub struct AppStore {
    pub screen: Signal<Screen>,
    pub dark_mode: Signal<bool>,
    core_state: Signal<String>,
    core_version: Signal<Option<String>>,
    core_note: Signal<Option<String>>,
    connected: Signal<bool>,
    core_busy: Signal<CoreBusy>,
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    group_selections: Signal<HashMap<String, String>>,
    custom_rules: Signal<Vec<proxy_core::CustomRule>>,
    dns_overrides: Signal<Vec<proxy_core::DnsOverride>>,
    config_script: Signal<String>,
    config_script_enabled: Signal<bool>,
    subscriptions: Signal<Vec<proxy_core::Subscription>>,
    /// Which subscription's nodes the core is offering. Only one subscription's nodes
    /// reach the core at a time, which is what makes switching subscriptions change the
    /// exit rather than merely reordering a list.
    active_subscription_id: Signal<Option<u64>>,
    traffic: Signal<TrafficWindow>,
    latency: Signal<HashMap<String, api::NodeLatency>>,
    latency_busy: Signal<bool>,
    /// The Nodes screen's filter. It belongs to the store rather than the row so the
    /// search survives a trip to another screen.
    pub node_query: Signal<String>,
    latency_cancel: Signal<bool>,
    /// Bumped when results go stale, so a probe that finishes after a restart cannot
    /// repopulate the list it was measuring.
    latency_generation: Signal<u64>,
    uptime_seconds: Signal<Option<u64>>,
    profile_loaded: Signal<bool>,
    /// The log page's buffer. The core only produces logs while collection is on, which
    /// the shell turns on when that page is visible and the core is running.
    logs: Signal<Vec<api::CoreLogEntry>>,
    log_cursor: Signal<u64>,
    log_generation: Signal<u64>,
    /// Read off the backend rather than assumed: a refusal to start collecting has to
    /// park the indicator on the truth, or it claims to be live while nothing arrives.
    log_paused: Signal<bool>,
    log_note: Signal<Option<String>>,
    /// The log page's filter, with its tally strip. Owned here so a trip to another screen
    /// does not silently reset what the reader was looking for.
    pub log_filter: Signal<crate::domain::LogFilter>,
    pub log_query: Signal<String>,
    /// Which generation's poll is live, so a second call cannot start a second loop and
    /// append every batch twice.
    log_poll: Signal<Option<u64>>,
    /// Increments on every edit the running core has not applied yet.
    config_revision: Signal<u64>,
    /// The import dialog: whether it is open, what was typed, and whether a parse is
    /// running. The fields live here rather than in the page so a trip to another screen
    /// does not discard a half-typed URL.
    import_open: Signal<bool>,
    import_name: Signal<String>,
    import_source: Signal<String>,
    import_busy: Signal<bool>,
    /// Why the last import failed, shown inside the dialog rather than only in a notice: a
    /// notice ages out while the mistyped URL stays in the field.
    import_error: Signal<Option<String>>,
    /// Bumped when an import is abandoned, so a parse that finishes after the dialog
    /// closed cannot add a subscription the user already walked away from.
    import_generation: Signal<u64>,
    /// The refresh whose result the buttons are waiting for, if any.
    refresh_busy: Signal<Option<RefreshTarget>>,
    /// Every refresh started and not yet finished. A list rather than one flag: pressing
    /// two rows in the same frame queues both instead of losing the second.
    refresh_in_flight: Signal<Vec<RefreshTarget>>,
    /// Where every action reports what actually happened. Made with the store, under the
    /// root's owner, so its lifecycle interval ends with the app.
    /// The operating system's proxy setting, as the backend last reported it. The core
    /// listens on a port; this is what makes anything actually use that port, so a client
    /// without it can connect and route nothing.
    system_proxy: Signal<SystemProxy>,
    system_proxy_busy: Signal<bool>,
    /// The config script editor: whether a validation is running.
    script_busy: Signal<bool>,
    /// Whether a downloaded rule-set refresh is running.
    rule_sets_busy: Signal<bool>,
    feedback: crate::feedback::Feedback,
    /// The poll generation: a restart or disconnect invalidates in-flight reads.
    poll_generation: Signal<u64>,
    /// The owner every task this store starts belongs to.
    owner: Owner,
}

impl AppStore {
    pub fn new() -> Self {
        Self {
            screen: signal(Screen::Overview),
            dark_mode: signal(false),
            core_state: signal("checking".to_owned()),
            core_version: signal(None),
            core_note: signal(None),
            connected: signal(false),
            core_busy: signal(CoreBusy::Idle),
            nodes: signal(Vec::new()),
            selected_tag: signal(String::new()),
            tunnel_mode: signal(TunnelMode::Rule),
            tun_enabled: signal(false),
            allow_lan: signal(false),
            group_selections: signal(HashMap::new()),
            custom_rules: signal(Vec::new()),
            dns_overrides: signal(Vec::new()),
            config_script: signal(String::new()),
            config_script_enabled: signal(false),
            subscriptions: signal(Vec::new()),
            active_subscription_id: signal(None),
            traffic: signal(TrafficWindow::default()),
            latency: signal(HashMap::new()),
            latency_busy: signal(false),
            node_query: signal(String::new()),
            latency_cancel: signal(false),
            latency_generation: signal(0),
            uptime_seconds: signal(None),
            profile_loaded: signal(false),
            logs: signal(Vec::new()),
            log_cursor: signal(0),
            log_generation: signal(0),
            log_paused: signal(false),
            log_note: signal(None),
            log_filter: signal(crate::domain::LogFilter::Routes),
            log_query: signal(String::new()),
            log_poll: signal(None),
            config_revision: signal(0),
            import_open: signal(false),
            import_name: signal(String::new()),
            import_source: signal(String::new()),
            import_busy: signal(false),
            import_error: signal(None),
            import_generation: signal(0),
            refresh_busy: signal(None),
            refresh_in_flight: signal(Vec::new()),
            script_busy: signal(false),
            rule_sets_busy: signal(false),
            system_proxy: signal(SystemProxy::Loading),
            system_proxy_busy: signal(false),
            feedback: crate::feedback::Feedback::new(),
            poll_generation: signal(0),
            owner: Owner::current(),
        }
    }

    /// Start a task that belongs to the app rather than to whatever control started it.
    ///
    /// `spawn` takes the current owner, and inside a press handler the current owner is the
    /// widget that was pressed. Saving is the case that shows: a dialog closed by the same
    /// press that submits it disposes its subtree before the write's first await, so the
    /// file is never touched and the notice already up has said it was saved. Deleting a row
    /// that starts a save does the same to the row's own owner. Everything here outlives the
    /// screen that asked for it, which is the point of asking through the store.
    fn work(self, future: impl std::future::Future<Output = ()> + 'static) {
        self.owner.run(|| {
            spawn(future);
        });
    }

    /// The notice hub, so the shell can mount the stack and a screen can report a
    /// validation problem it found before calling in.
    pub fn feedback(&self) -> crate::feedback::Feedback {
        self.feedback
    }

    /// The core state string the chrome shows: `running`, `stopped`, `checking`,
    /// or `unavailable`.
    pub fn core_state(&self) -> String {
        self.core_state.get()
    }

    pub fn core_version(&self) -> Option<String> {
        self.core_version.get()
    }

    pub fn core_note(&self) -> Option<String> {
        self.core_note.get()
    }

    pub fn connected(&self) -> bool {
        self.connected.get()
    }

    pub fn busy(&self) -> CoreBusy {
        self.core_busy.get()
    }

    /// Any core affordance is inert while work of any kind is in flight.
    pub fn core_action_busy(&self) -> bool {
        self.core_busy.get() != CoreBusy::Idle
    }

    pub fn nodes(&self) -> Vec<ProxyNode> {
        self.nodes.get()
    }

    /// The chosen node's tag, for a row deciding whether it is the current one.
    pub fn selected_tag_text(&self) -> String {
        self.selected_tag.get()
    }

    pub fn selected_tag(&self) -> String {
        self.selected_tag.get()
    }

    /// The chosen node, or `None` before a subscription has been imported.
    ///
    /// Tracked on purpose: screens call this from a binding, and an untracked read here
    /// would leave the overview naming no node after the profile finishes loading.
    pub fn selected_node(&self) -> Option<ProxyNode> {
        let tag = self.selected_tag.get();
        self.nodes.get().into_iter().find(|node| node.tag == tag)
    }

    pub fn subscription_count(&self) -> usize {
        self.subscriptions.get().len()
    }

    pub fn custom_rule_count(&self) -> usize {
        self.custom_rules.get().len()
    }

    pub fn tunnel_mode(&self) -> TunnelMode {
        self.tunnel_mode.get()
    }

    pub fn tun_enabled(&self) -> bool {
        self.tun_enabled.get()
    }

    pub fn allow_lan(&self) -> bool {
        self.allow_lan.get()
    }

    pub fn traffic(&self) -> TrafficWindow {
        self.traffic.get()
    }

    pub fn uptime_seconds(&self) -> Option<u64> {
        self.uptime_seconds.get()
    }

    /// `true` once local config has been read, so the shell can tell "no nodes"
    /// from "still loading".
    pub fn profile_loaded(&self) -> bool {
        self.profile_loaded.get()
    }

    /// Local edits the running core has not applied: the banner's trigger.
    ///
    /// Tracked: an edit while connected moves only the revision, so an untracked read
    /// would leave the banner hidden until something else happened to repaint.
    pub fn config_pending(&self) -> bool {
        self.connected.get() && self.config_revision.get() > 0
    }

    /// Read the profile and put it in the store. Runs once, from the root.
    pub fn load_profile(self) {
        self.work(async move {
            match api::load_profile().await {
                Ok(profile) => self.apply_profile(profile),
                Err(error) => {
                    self.core_state.set("unavailable".to_owned());
                    self.core_note.set(Some(error.to_string()));
                }
            }
            self.profile_loaded.set(true);
        });
    }

    fn apply_profile(self, profile: AppProfile) {
        // Resolve the active subscription against what is actually there rather than
        // trusting the stored id: a pulled or hand-edited file can name one that no longer
        // exists, which would leave an empty node list and no way to start.
        let active = crate::domain::resolve_active_subscription_id(
            &profile.subscriptions,
            profile.active_subscription_id,
            &profile.selected_tag,
        );
        let nodes = crate::domain::subscription_nodes(&profile.subscriptions, active);
        // The stored tag only survives if that node is still in the list; otherwise fall
        // back to the first, so the core is never asked to use a node that is gone.
        let selected = crate::domain::select_available_tag(&nodes, &profile.selected_tag);
        self.subscriptions.set(profile.subscriptions);
        self.active_subscription_id.set(active);
        self.nodes.set(nodes);
        self.selected_tag.set(selected);
        self.tunnel_mode.set(profile.tunnel_mode);
        self.tun_enabled.set(profile.tun_enabled);
        self.allow_lan.set(profile.allow_lan);
        self.custom_rules.set(profile.custom_rules.clone());
        self.dns_overrides.set(profile.dns_overrides.clone());
        self.config_script.set(profile.config_script.clone());
        self.config_script_enabled
            .set(profile.config_script_enabled);
        self.group_selections.set(profile.group_selections.clone());
        // A stored choice outranks the system appearance. Pushing it to the engine is
        // the whole point: setting the signal alone would relabel the switch while the
        // window stayed in the appearance the system asked for.
        self.apply_appearance(profile.dark_mode);
    }

    /// The DNS servers the active subscription's nodes need to be resolved.
    ///
    /// A provider that ships nodes behind hostnames it resolves itself expects its own
    /// resolvers: without them the app asks the system, which can return nothing for a
    /// hostname only that provider's DNS knows, and the connection fails as an unexplained
    /// timeout. Only the active subscription's servers apply, matching the nodes in use.
    pub fn proxy_server_nameservers(&self) -> Vec<String> {
        let active = self.active_subscription_id.get_untracked();
        self.subscriptions
            .get_untracked()
            .into_iter()
            .find(|subscription| Some(subscription.id) == active)
            .map(|subscription| subscription.proxy_server_nameservers)
            .unwrap_or_default()
    }

    /// The request the core starts with, built from current state.
    pub fn connection_request(&self) -> ConnectionRequest {
        let script_enabled = self.config_script_enabled.get_untracked();
        let script = self.config_script.get_untracked();
        let script_applied = script_enabled && !script.trim().is_empty();
        ConnectionRequest {
            nodes: self.nodes.get_untracked(),
            selected_tag: self.selected_tag.get_untracked(),
            proxy_server_nameservers: self.proxy_server_nameservers(),
            mode: self.tunnel_mode.get_untracked(),
            tun: self.tun_enabled.get_untracked(),
            allow_lan: self.allow_lan.get_untracked(),
            custom_rules: crate::domain::connection_custom_rules(
                script_enabled,
                &script,
                self.custom_rules.get_untracked(),
                crate::domain::process_rules_supported(),
            ),
            dns_overrides: self.dns_overrides.get_untracked(),
            config_script: script_applied.then_some(script),
            group_selections: self.group_selections.get_untracked(),
        }
    }

    /// Start or stop the core. A second press while busy does nothing.
    pub fn toggle_core(self, enable: bool) {
        if self.core_busy.get_untracked() != CoreBusy::Idle {
            return;
        }
        self.core_busy.set(CoreBusy::Toggling);
        self.bump_generation();
        self.work(async move {
            let request = enable.then(|| self.connection_request());
            // TUN needs the privileged helper, which does not exist while the core is
            // stopped. Starting without acquiring it first fails with a permission error the
            // user cannot act on, so the prompt happens here and the switch stays a
            // preference rather than a one-shot setup. Only when TUN is on: the backend
            // rejects a pre-flight for a plain proxy config.
            let tun_requested = request.as_ref().is_some_and(|request| request.tun);
            if let Some(tun_request) = request.clone().filter(|_| enable && tun_requested) {
                if let Err(error) = api::prepare_tun_mode(tun_request).await {
                    self.core_busy.set(CoreBusy::Idle);
                    self.feedback
                        .error(format!("准备 TUN 权限失败，内核未启动：{error}"));
                    self.refresh_status();
                    return;
                }
            }
            match api::set_core_enabled(enable, request).await {
                Ok(status) => {
                    self.apply_core_status(status, enable);
                    if enable {
                        self.config_revision.set(0);
                        self.start_polling();
                        self.feedback.success("内核已启动");
                    } else {
                        self.stop_side_effects();
                    }
                }
                Err(error) => {
                    self.core_note.set(Some(error.to_string()));
                    self.feedback.error(error.to_string());
                }
            }
            self.core_busy.set(CoreBusy::Idle);
            // The OS proxy points at a port that may no longer be there: a stop tears the
            // route down and a failed start may have too. Re-read rather than assume, since
            // a switch left on would claim traffic is proxied when it is not.
            if !enable || !self.connected.get_untracked() {
                self.refresh_system_proxy();
            }
        });
    }

    /// What has to be forgotten when the core is no longer running.
    fn stop_side_effects(self) {
        self.connected.set(false);
        self.traffic.set(TrafficWindow::default());
        self.uptime_seconds.set(None);
    }

    /// Read the operating system's proxy setting. Called on boot, after a stop, and whenever
    /// a write is refused, because the value is not the app's to remember.
    pub fn refresh_system_proxy(self) {
        self.work(async move {
            match api::system_proxy_status().await {
                Ok(status) => self.system_proxy.set(SystemProxy::Ready(status)),
                Err(error) => self
                    .system_proxy
                    .set(SystemProxy::Failed(error.to_string())),
            }
        });
    }

    pub fn system_proxy(&self) -> SystemProxy {
        self.system_proxy.get()
    }

    /// Point the operating system at the core, or stop it doing so. This is what makes a
    /// connected core actually carry anything.
    ///
    /// A refusal re-reads the status instead of leaving the switch where the press put it:
    /// the write may have half-applied, and the real setting is the only truth worth showing.
    pub fn set_system_proxy(self, enable: bool) {
        if self.core_action_busy() || self.system_proxy_busy() {
            return;
        }
        self.system_proxy_busy.set(true);
        self.work(async move {
            match api::set_system_proxy(enable).await {
                Ok(status) => {
                    self.system_proxy.set(SystemProxy::Ready(status));
                    self.feedback.success(if enable {
                        "系统代理已启用"
                    } else {
                        "系统代理已关闭"
                    });
                }
                Err(error) => {
                    self.feedback.error(format!("系统代理设置失败：{error}"));
                    match api::system_proxy_status().await {
                        Ok(status) => self.system_proxy.set(SystemProxy::Ready(status)),
                        Err(refresh) => self
                            .system_proxy
                            .set(SystemProxy::Failed(refresh.to_string())),
                    }
                }
            }
            self.system_proxy_busy.set(false);
        });
    }

    pub fn system_proxy_busy(&self) -> bool {
        self.system_proxy_busy.get()
    }

    /// Push the current config to a running core.
    pub fn restart_core(self) {
        if self.core_busy.get_untracked() != CoreBusy::Idle {
            return;
        }
        self.core_busy.set(CoreBusy::Restarting);
        self.bump_generation();
        let request = self.connection_request();
        self.work(async move {
            match api::restart_core(request).await {
                Ok(status) => {
                    self.apply_core_status(status, true);
                    self.config_revision.set(0);
                    self.start_polling();
                }
                Err(error) => self.core_note.set(Some(error.to_string())),
            }
            self.core_busy.set(CoreBusy::Idle);
        });
    }

    /// One status read, used on first paint and by the "重新检测" action.
    pub fn refresh_status(self) {
        self.work(async move {
            match api::core_status().await {
                Ok(status) => {
                    let running = status.state == "running";
                    self.apply_core_status(status, running);
                    if running {
                        self.start_polling();
                    }
                }
                Err(error) => {
                    self.core_state.set("unavailable".to_owned());
                    self.core_note.set(Some(error.to_string()));
                }
            }
        });
    }

    fn apply_core_status(self, status: ApiCoreStatus, connected: bool) {
        self.core_state.set(status.state);
        self.core_version.set(status.version);
        self.core_note.set(status.note);
        self.connected.set(connected);
    }

    /// A new generation makes every in-flight poll result stale, so a restart cannot
    /// have an older read overwrite the fresh one.
    fn bump_generation(self) {
        self.poll_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// Poll traffic once a second while the core runs, and count uptime off the same tick.
    /// Stops as soon as the generation changes or the core is no longer connected, so a
    /// restart cannot leave two loops writing the same signals.
    ///
    /// Uptime is counted here rather than read from the core: the control API reports byte
    /// counters and connection counts but no start time, so the app's own view of how long
    /// it has been connected is the honest one. It resets to zero on connect, which is what
    /// the user can verify against the moment they pressed the button.
    pub fn start_polling(self) {
        let generation = self.poll_generation.get_untracked();
        self.uptime_seconds.set(Some(0));
        self.work(async move {
            let mut seconds = 0_u64;
            loop {
                if self.poll_generation.get_untracked() != generation
                    || !self.connected.get_untracked()
                {
                    return;
                }
                let Ok(traffic) = api::core_traffic().await else {
                    return;
                };
                if self.poll_generation.get_untracked() != generation {
                    return;
                }
                self.push_traffic(traffic);
                seconds = seconds.saturating_add(1);
                self.uptime_seconds.set(Some(seconds));
                timer(Duration::from_secs(1)).await;
            }
        });
    }

    /// Turn the core's cumulative counters into one per-second sample.
    ///
    /// A read that goes backwards means the core restarted and the counters reset with it;
    /// differencing those would underflow to a meaningless spike, so the sample reads zero.
    fn push_traffic(self, traffic: CoreTraffic) {
        let previous = self.traffic.get_untracked();
        let restart =
            traffic.download_total < previous.download || traffic.upload_total < previous.upload;
        let (per_second_download, per_second_upload) = if restart {
            (0, 0)
        } else {
            (
                traffic.download_total.saturating_sub(previous.download),
                traffic.upload_total.saturating_sub(previous.upload),
            )
        };
        self.traffic.update(|window| {
            window.upload = traffic.upload_total;
            window.download = traffic.download_total;
            window.upload_per_second = per_second_upload;
            window.download_per_second = per_second_download;
            window.active_connections = traffic.active_connections;
            window.upload_history.push(per_second_upload);
            window.download_history.push(per_second_download);
            if window.upload_history.len() > HISTORY_LEN {
                window.upload_history.remove(0);
            }
            if window.download_history.len() > HISTORY_LEN {
                window.download_history.remove(0);
            }
        });
    }

    pub fn set_screen(self, screen: Screen) {
        self.screen.set(screen);
    }

    /// Switch appearance and remember the choice. The profile is the single source, so a
    /// relaunch comes back the same way.
    pub fn set_dark_mode(self, dark: bool) {
        self.apply_appearance(dark);
        self.persist_theme();
    }

    /// Draw in `dark` without writing anything. Restoring the profile uses this: a saved
    /// value is not a new choice, and persisting it would rewrite the file it came from.
    pub fn apply_appearance(self, dark: bool) {
        self.dark_mode.set(dark);
        set_color_scheme(if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        });
    }

    fn persist_theme(self) {
        self.work(async move {
            // Read-modify-write: the theme is the only field this path owns, so a
            // concurrent profile edit is not overwritten from a stale copy.
            let Ok(mut profile) = api::load_profile().await else {
                return;
            };
            profile.dark_mode = self.dark_mode.get_untracked();
            let _ = api::save_profile(profile).await;
        });
    }

    /// Choose the node the core sends traffic through. A running core restarts so the
    /// choice takes effect; the selection is saved either way.
    pub fn select_node(self, tag: &str) {
        if self.selected_tag.get_untracked() == tag {
            return;
        }
        self.selected_tag.set(tag.to_owned());
        self.mark_config_dirty();
        self.persist_profile();
        if self.connected.get_untracked() {
            self.restart_core();
        }
    }

    /// The buffered log entries, oldest first.
    pub fn logs(&self) -> Vec<api::CoreLogEntry> {
        self.logs.get()
    }

    /// Whether collection is off. Shown rather than hidden: an empty list that is not
    /// filling looks identical to a core that forwarded nothing, and only the first of
    /// those has a fix.
    pub fn log_paused(&self) -> bool {
        self.log_paused.get()
    }

    pub fn log_note(&self) -> Option<String> {
        self.log_note.get()
    }

    pub fn log_filter(&self) -> crate::domain::LogFilter {
        self.log_filter.get()
    }

    /// Start collecting if the log page is open over a running core, and stop otherwise.
    ///
    /// The core only streams logs while collection is on, and a buffer nobody is reading is
    /// memory spent for nothing, so this belongs to the page rather than the connection.
    /// Idempotent: the shell calls it from an effect that re-runs whenever the screen or the
    /// core state moves, and a repeated call must not start a second drain.
    pub fn sync_log_collection(self) {
        let active = self.core_state.get_untracked() == "running"
            && self.screen.get_untracked() == Screen::Logs;
        if !active {
            if !self.log_paused.get_untracked() {
                self.set_log_collection(false);
            }
            return;
        }
        if self.log_paused.get_untracked() {
            self.set_log_collection(true);
        }
        let generation = self.log_generation.get_untracked();
        if self.log_poll.get_untracked() == Some(generation) {
            return;
        }
        self.log_poll.set(Some(generation));
        self.poll_logs();
    }

    /// Ask the core to collect logs, or to stop. The indicator is parked on the answer:
    /// a refusal left reporting "collecting" would claim a live feed that is not one.
    pub fn set_log_collection(self, enable: bool) {
        self.log_paused.set(!enable);
        self.work(async move {
            if let Err(error) = api::set_core_log_collection(enable).await {
                if enable {
                    self.log_paused.set(true);
                    self.log_note.set(Some(error.to_string()));
                }
            }
        });
    }

    /// Drain what the core has collected, once a second, while the log page is open and
    /// the core is running. It returns on its own when any of that stops, so the shell
    /// starts it on arrival rather than keeping a poll alive behind every page.
    pub fn poll_logs(self) {
        let generation = self.log_generation.get_untracked();
        self.work(async move {
            loop {
                let open = self.screen.get_untracked() == Screen::Logs;
                let running = self.core_state.get_untracked() == "running";
                if self.log_generation.get_untracked() != generation
                    || !open
                    || !running
                    || self.log_paused.get_untracked()
                {
                    self.release_log_poll(generation);
                    return;
                }
                let cursor = self.log_cursor.get_untracked();
                match api::core_logs(cursor).await {
                    Ok(batch) => {
                        // The cursor advances even for an empty batch: not moving it
                        // would re-read the same window for ever.
                        self.log_cursor.set(batch.next_cursor);
                        if !batch.entries.is_empty() {
                            self.logs.update(|stored| {
                                stored.extend(batch.entries);
                                let excess = stored.len().saturating_sub(LOG_BUFFER);
                                if excess > 0 {
                                    stored.drain(..excess);
                                }
                            });
                        }
                    }
                    // Stop rather than spin against a core that stopped answering. The
                    // page reports itself paused, and leaving it starts collection again.
                    Err(_) => {
                        self.release_log_poll(generation);
                        return;
                    }
                }
                timer(Duration::from_secs(1)).await;
            }
        });
    }

    /// Give up the poll slot, but only if this loop still holds it: a loop that has been
    /// superseded must not clear the flag that marks its replacement live, which would let a
    /// third start alongside it.
    fn release_log_poll(self, generation: u64) {
        if self.log_poll.get_untracked() == Some(generation) {
            self.log_poll.set(None);
        }
    }

    /// Stop collecting and drop the buffer. Bumping the generation makes the running poll
    /// give up at once, so a late batch cannot repopulate a list the user just cleared.
    pub fn clear_logs(self) {
        self.log_generation.update(|n| *n = n.wrapping_add(1));
        self.logs.set(Vec::new());
        self.log_cursor.set(0);
        self.log_poll.set(None);
        // Emptying the buffer is not leaving the page: collecting carries straight on,
        // against the new cursor, so a reader who clears and keeps watching still sees rows.
        self.sync_log_collection();
    }

    /// Every subscription, in the order it was added.
    pub fn subscriptions(&self) -> Vec<proxy_core::Subscription> {
        self.subscriptions.get()
    }

    pub fn active_subscription_id(&self) -> Option<u64> {
        self.active_subscription_id.get()
    }

    /// Parse a subscription URL or pasted node list and, if it yields nodes, add it and
    /// switch to it.
    ///
    /// Switching on import is deliberate: the overview's reconnect banner only reflects the
    /// active subscription, so an import that left the old one active would appear to do
    /// nothing. A parse that yields no nodes is reported with the parser's own reason,
    /// which is usually the useful one ("unsupported protocol", "missing password").
    pub fn import_subscription(self, name: String, source: String) {
        let source = source.trim().to_owned();
        if source.is_empty() {
            self.feedback.error("请先填写订阅地址或内容");
            return;
        }
        let generation = self.import_generation.get_untracked();
        self.import_busy.set(true);
        self.import_error.set(None);
        self.work(async move {
            let result = api::preview_subscription(source.clone()).await;
            // The dialog may have been closed, or another import started, while this was
            // in flight. A late result must not reopen it or overwrite the newer state.
            if self.import_generation.get_untracked() != generation {
                return;
            }
            self.import_busy.set(false);
            match result {
                Ok(report) if !report.nodes.is_empty() => {
                    let id =
                        crate::domain::next_subscription_id(&self.subscriptions.get_untracked());
                    let count = report.nodes.len();
                    let rejected = report.rejected.len();
                    let display = if name.trim().is_empty() {
                        crate::domain::source_label(&source)
                    } else {
                        name.trim().to_owned()
                    };
                    let nodes = crate::domain::namespace_nodes(id, report.nodes);
                    let subscription = proxy_core::Subscription {
                        id,
                        name: display.clone(),
                        source,
                        nodes: nodes.clone(),
                        proxy_server_nameservers: report.proxy_server_nameservers,
                        rejected_count: rejected,
                    };
                    self.subscriptions
                        .update(|stored| stored.push(subscription));
                    self.activate_nodes(id, nodes);
                    if let Err(error) = self.save().await {
                        self.feedback.error(format!("配置保存失败：{error}"));
                    }
                    let mut message = format!("已导入并切换到 {display}，共 {count} 个节点");
                    if rejected > 0 {
                        message.push_str(&format!("，{rejected} 个无法解析"));
                    }
                    if self.connected.get_untracked() {
                        message.push_str("，重新连接后生效");
                        self.mark_config_dirty();
                    }
                    self.feedback.success(message);
                    // Closed and emptied, not left open holding the link that was just
                    // imported: an open form invites a second press, which would add the
                    // same provider twice under a different id.
                    self.import_name.set(String::new());
                    self.import_source.set(String::new());
                    self.import_error.set(None);
                    self.import_open.set(false);
                }
                // Kept open with the text intact and the reason under the field it came
                // from: the usual cause is one mistyped character, and the user has to be
                // able to see and fix it.
                Ok(report) => {
                    let reason = report
                        .rejected
                        .first()
                        .map(|issue| issue.reason.clone())
                        .unwrap_or_else(|| "没有找到可用节点".to_owned());
                    self.import_error
                        .set(Some(format!("订阅解析失败：{reason}")));
                }
                Err(error) => self
                    .import_error
                    .set(Some(format!("订阅解析失败：{error}"))),
            }
        });
    }

    /// Abort an in-flight import. Bumping the generation is what discards the result, so
    /// closing the dialog cannot be undone by a slow response.
    pub fn cancel_import(self) {
        self.import_generation.update(|n| *n = n.wrapping_add(1));
        self.import_busy.set(false);
        self.import_error.set(None);
        self.import_open.set(false);
    }

    pub fn import_busy(&self) -> bool {
        self.import_busy.get()
    }

    pub fn import_open(&self) -> bool {
        self.import_open.get()
    }

    /// Open the dialog with empty fields. A stale draft from an abandoned import would
    /// otherwise be submitted again under a name the user has forgotten typing.
    pub fn open_import(self) {
        self.import_name.set(String::new());
        self.import_source.set(String::new());
        self.import_error.set(None);
        self.import_open.set(true);
    }

    pub fn close_import(self) {
        if self.import_busy.get_untracked() {
            self.cancel_import();
            return;
        }
        self.import_open.set(false);
    }

    /// The dialog's two fields. Handed out as signals so the inputs write straight through
    /// and the text survives a trip to another screen.
    pub fn import_fields(&self) -> (Signal<String>, Signal<String>) {
        (self.import_name, self.import_source)
    }

    /// The dialog's failure line.
    pub fn import_error(&self) -> Signal<Option<String>> {
        self.import_error
    }

    /// Make one subscription the source of the node list, keeping the selected node when
    /// it still exists there and falling back to the first otherwise.
    pub fn select_subscription(self, id: u64) {
        if self.active_subscription_id.get_untracked() == Some(id) {
            return;
        }
        let Some(subscription) = self
            .subscriptions
            .get_untracked()
            .into_iter()
            .find(|subscription| subscription.id == id)
        else {
            return;
        };
        self.activate_nodes(id, subscription.nodes.clone());
        self.persist_profile();
        if self.connected.get_untracked() {
            self.mark_config_dirty();
            self.feedback.info("已切换订阅，重新连接后生效");
        } else {
            self.feedback
                .success(format!("已切换到 {}", subscription.name));
        }
    }

    /// Point the node list, the selection and the proxy-group choice at one
    /// subscription's nodes. The remembered `proxy` outbound has to move with them or the
    /// core keeps sending traffic through a node that is no longer listed.
    fn activate_nodes(self, id: u64, nodes: Vec<ProxyNode>) {
        let previous = self.selected_tag.get_untracked();
        let selected = crate::domain::select_available_tag(&nodes, &previous);
        self.active_subscription_id.set(Some(id));
        self.nodes.set(nodes.clone());
        self.selected_tag.set(selected.clone());
        self.group_selections.update(|selections| {
            if selected.is_empty() {
                selections.remove("proxy");
            } else {
                selections.insert("proxy".to_owned(), selected);
            }
        });
    }

    /// Re-fetch one subscription, replacing its nodes with what the provider returns now.
    pub fn refresh_subscription(self, id: u64) {
        self.refresh_subscriptions(vec![id], RefreshScope::One);
    }

    /// Re-fetch every subscription.
    pub fn refresh_all_subscriptions(self) {
        let ids = self
            .subscriptions
            .get_untracked()
            .into_iter()
            .map(|subscription| subscription.id)
            .collect();
        self.refresh_subscriptions(ids, RefreshScope::All);
    }

    /// Fetch the given subscriptions in sequence and apply each result.
    ///
    /// Sequential rather than concurrent: a provider that sees four simultaneous requests
    /// from one client is more likely to throttle it than serve it faster. Each failure is
    /// named, because "1 个失败" gives nobody with four subscriptions anything to act on.
    fn refresh_subscriptions(self, ids: Vec<u64>, scope: RefreshScope) {
        if ids.is_empty() {
            return;
        }
        let target = match scope {
            RefreshScope::One => RefreshTarget::One(ids[0]),
            RefreshScope::All => RefreshTarget::All,
        };
        // Claim synchronously: two clicks before the next frame must not both start work.
        if self.refresh_in_flight.get_untracked().contains(&target) {
            return;
        }
        self.refresh_in_flight
            .update(|running| running.push(target));
        self.refresh_busy.set(Some(target));
        let sources: Vec<(u64, String)> = self
            .subscriptions
            .get_untracked()
            .into_iter()
            .filter(|subscription| ids.contains(&subscription.id))
            .map(|subscription| (subscription.id, subscription.source))
            .collect();
        self.work(async move {
            let mut refreshed = 0usize;
            let mut failures: Vec<String> = Vec::new();
            for (id, source) in sources {
                let name = self
                    .subscriptions
                    .get_untracked()
                    .into_iter()
                    .find(|subscription| subscription.id == id)
                    .map(|subscription| subscription.name)
                    .unwrap_or_else(|| format!("#{id}"));
                match api::preview_subscription(source).await {
                    Ok(report) => {
                        let mut outcome = None;
                        self.subscriptions.update(|stored| {
                            outcome =
                                Some(crate::domain::apply_subscription_report(stored, id, report));
                        });
                        match outcome {
                            Some(Ok(_)) => refreshed += 1,
                            Some(Err(reason)) => failures.push(format!("{name}：{reason}")),
                            None => failures.push(format!("{name}：订阅已不在列表中")),
                        }
                    }
                    Err(error) => failures.push(format!("{name}：{error}")),
                }
            }
            self.after_refresh();
            if failures.is_empty() {
                self.feedback.success("订阅已刷新，可重新连接或启动");
            } else {
                self.feedback.error(format!(
                    "已刷新 {refreshed} 个订阅，{} 个失败：{}",
                    failures.len(),
                    failures.join("；")
                ));
            }
            self.release_refresh(target);
        });
    }

    /// Rebuild the node list after a refresh, then drop selections whose node is gone.
    ///
    /// A refresh can delete the selected node. Without resolving a live active
    /// subscription here, the app would be left with an empty list and a start button that
    /// can never be enabled again.
    fn after_refresh(self) {
        let stored = self.subscriptions.get_untracked();
        let active = crate::domain::resolve_active_subscription_id(
            &stored,
            self.active_subscription_id.get_untracked(),
            &self.selected_tag.get_untracked(),
        );
        let nodes = crate::domain::subscription_nodes(&stored, active);
        let selected =
            crate::domain::select_available_tag(&nodes, &self.selected_tag.get_untracked());
        let live = crate::domain::all_live_node_tags(&stored);
        self.active_subscription_id.set(active);
        self.nodes.set(nodes);
        self.selected_tag.set(selected);
        self.group_selections.update(|selections| {
            crate::domain::prune_stale_group_selections(selections, &live);
        });
        if self.connected.get_untracked() {
            self.mark_config_dirty();
        }
        self.persist_profile();
    }

    fn release_refresh(self, target: RefreshTarget) {
        self.refresh_in_flight
            .update(|running| running.retain(|item| *item != target));
        let next = self.refresh_in_flight.get_untracked().last().copied();
        self.refresh_busy.set(next);
    }

    /// Which refresh is running, for the button that starts it.
    pub fn refresh_busy(&self) -> Option<RefreshTarget> {
        self.refresh_busy.get()
    }

    /// Delete a subscription and its nodes. The node list is rebuilt from what remains,
    /// so deleting the active one does not leave the core pointing at nothing.
    pub fn delete_subscription(self, id: u64) {
        let name = self
            .subscriptions
            .get_untracked()
            .into_iter()
            .find(|subscription| subscription.id == id)
            .map(|subscription| subscription.name);
        let Some(name) = name else { return };
        self.subscriptions
            .update(|stored| stored.retain(|subscription| subscription.id != id));
        self.after_refresh();
        if self.connected.get_untracked() {
            self.feedback.info(format!("已删除 {name}，重新连接后生效"));
        } else {
            self.feedback.success(format!("已删除 {name}"));
        }
    }

    /// The custom routing rules, in the order they are applied.
    pub fn custom_rules(&self) -> Vec<proxy_core::CustomRule> {
        self.custom_rules.get()
    }

    /// The per-domain DNS overrides.
    pub fn dns_overrides(&self) -> Vec<proxy_core::DnsOverride> {
        self.dns_overrides.get()
    }

    /// Validate and add or replace one routing rule.
    ///
    /// The candidate is normalized before the list is touched, so a value the core would
    /// reject never reaches disk, and the whole prospective list is re-validated afterwards
    /// to catch a duplicate of another rule. On success the profile is written and, if the
    /// core is running, the edit is marked pending: rules only reach the core through a
    /// (re)start, and saying so is the difference between a saved rule and a live one.
    pub fn save_custom_rule(
        self,
        editing_id: Option<u64>,
        match_type: CustomRuleMatch,
        action: CustomRuleAction,
        value: &str,
    ) -> Result<(), String> {
        let mut rules = self.custom_rules.get_untracked();
        if editing_id.is_none() && rules.len() >= proxy_core::MAX_CUSTOM_RULES {
            return Err(format!("最多添加 {} 条规则", proxy_core::MAX_CUSTOM_RULES));
        }
        let id = editing_id
            .or_else(|| crate::domain::next_custom_rule_id(&rules))
            .ok_or_else(|| "无法生成规则 ID".to_owned())?;
        let enabled = rules
            .iter()
            .find(|rule| rule.id == id)
            .map(|rule| rule.enabled)
            .unwrap_or(true);
        let candidate = proxy_core::CustomRule {
            id,
            enabled,
            match_type,
            value: value.to_owned(),
            action,
        }
        .normalized()
        .map_err(|error| error.to_string())?;

        if let Some(existing) = rules.iter_mut().find(|rule| rule.id == id) {
            *existing = candidate;
        } else if editing_id.is_some() {
            return Err("要编辑的规则不存在".to_owned());
        } else {
            rules.push(candidate);
        }
        proxy_core::validate_custom_rules(&rules).map_err(|error| error.to_string())?;
        self.custom_rules.set(rules);
        self.rule_change();
        Ok(())
    }

    pub fn toggle_custom_rule(self, id: u64) {
        self.custom_rules.update(|rules| {
            if let Some(rule) = rules.iter_mut().find(|rule| rule.id == id) {
                rule.enabled = !rule.enabled;
            }
        });
        self.rule_change();
    }

    pub fn delete_custom_rule(self, id: u64) {
        self.custom_rules
            .update(|rules| rules.retain(|rule| rule.id != id));
        self.rule_change();
    }

    /// Move a rule past its neighbour in the order the user sees.
    ///
    /// The swap happens between *visible* rows: process rules are hidden where the OS cannot
    /// report them, and moving across a hidden row would look like a jump of two.
    pub fn move_custom_rule(self, id: u64, offset: isize) {
        let supported = crate::domain::process_rules_supported();
        self.custom_rules.update(|rules| {
            let visible: Vec<usize> = rules
                .iter()
                .enumerate()
                .filter_map(|(index, rule)| {
                    crate::domain::custom_rule_available(rule.match_type, supported)
                        .then_some(index)
                })
                .collect();
            let Some(position) = visible.iter().position(|index| rules[*index].id == id) else {
                return;
            };
            let Some(target) = position.checked_add_signed(offset) else {
                return;
            };
            if let Some(target_index) = visible.get(target) {
                rules.swap(visible[position], *target_index);
            }
        });
        self.rule_change();
    }

    /// Validate and add one per-domain DNS override, rejecting a duplicate suffix.
    pub fn add_dns_override(self, domain: &str, server: &str) -> Result<(), String> {
        let (domain_suffix, server) = proxy_core::normalize_dns_override(domain, server)?;
        let mut overrides = self.dns_overrides.get_untracked();
        if overrides
            .iter()
            .any(|item| item.domain_suffix == domain_suffix)
        {
            return Err(format!("已存在 {domain_suffix} 的专用 DNS"));
        }
        if overrides.len() >= proxy_core::MAX_CUSTOM_RULES {
            return Err(format!("最多添加 {} 条规则", proxy_core::MAX_CUSTOM_RULES));
        }
        let id = crate::domain::next_dns_override_id(&overrides);
        overrides.push(proxy_core::DnsOverride {
            id,
            enabled: true,
            domain_suffix,
            server,
        });
        self.dns_overrides.set(overrides);
        self.rule_change();
        Ok(())
    }

    pub fn toggle_dns_override(self, id: u64) {
        self.dns_overrides.update(|items| {
            if let Some(item) = items.iter_mut().find(|item| item.id == id) {
                item.enabled = !item.enabled;
            }
        });
        self.rule_change();
    }

    pub fn delete_dns_override(self, id: u64) {
        self.dns_overrides
            .update(|items| items.retain(|item| item.id != id));
        self.rule_change();
    }

    /// Report a rule or DNS edit, and note it against a running core.
    ///
    /// The wording distinguishes saved from applied because the two differ here: nothing
    /// the core forwards changes until it restarts, and a user who is told only "已保存"
    /// has no reason to expect the rule not to work yet.
    fn rule_change(self) {
        let connected = self.connected.get_untracked();
        if connected {
            self.mark_config_dirty();
        }
        self.persist_profile();
        self.feedback.success(if connected {
            "规则已保存，重新连接后生效"
        } else {
            "规则已保存"
        });
    }

    pub fn config_script(&self) -> String {
        self.config_script.get()
    }

    pub fn config_script_enabled(&self) -> bool {
        self.config_script_enabled.get()
    }

    /// Whether the script override is what the core would use: an enabled but empty script
    /// is inert, and the rules page says so rather than implying the list is switched off.
    pub fn config_script_active(&self) -> bool {
        self.config_script_enabled.get() && !self.config_script.get().trim().is_empty()
    }

    /// Save the override script, optionally switching it on. Clearing the text switches it
    /// off: a script that is enabled but empty would otherwise leave the rules page claiming an
    /// override that the core never applies.
    pub fn set_config_script(self, enabled: bool, script: String) {
        let enabled = enabled && !script.trim().is_empty();
        self.config_script_enabled.set(enabled);
        self.config_script.set(script);
        if self.connected.get_untracked() {
            self.mark_config_dirty();
            self.persist_profile();
            self.feedback.success("配置脚本已保存，重新连接后生效");
        } else {
            self.persist_profile();
            self.feedback.success("配置脚本已保存");
        }
    }

    /// Run `script` against the current config without saving it, and report the answer.
    ///
    /// The core's own validator is the only authority on whether a script works, so this
    /// reports what it says rather than guessing at the JavaScript. It takes the text rather
    /// than reading the store: a draft is checked before it is saved, which is the whole point
    /// of the button.
    pub fn validate_config_script(self, script: String) {
        if script.trim().is_empty() {
            self.feedback.error("还没有填写脚本");
            return;
        }
        self.script_busy.set(true);
        let request = ConnectionRequest {
            config_script: Some(script),
            ..self.connection_request()
        };
        self.work(async move {
            let result = api::validate_config_script(request).await;
            self.script_busy.set(false);
            self.feedback
                .settled("配置脚本校验", result.map(|_| "配置脚本校验通过"));
        });
    }

    pub fn script_busy(&self) -> bool {
        self.script_busy.get()
    }

    /// Refresh the rule-sets the core's geo rules download. Separate from the app's own
    /// rules, which need no download, so the label says 规则 in the sense the provider uses.
    ///
    /// The flag lives here rather than on the button because the work outlives the screen: a
    /// refresh started and then navigated away from has to finish and report, not stop
    /// halfway with the reason never said.
    pub fn update_rule_sets(self) {
        if self.rule_sets_busy.get_untracked() {
            return;
        }
        self.rule_sets_busy.set(true);
        self.work(async move {
            let result = api::update_rule_sets(true).await;
            self.rule_sets_busy.set(false);
            self.feedback
                .settled("更新分流规则", result.map(|_| "分流规则已更新"));
        });
    }

    pub fn rule_sets_busy(&self) -> bool {
        self.rule_sets_busy.get()
    }

    /// The measured latency for one node, if it has been probed.
    pub fn latency_for(&self, tag: &str) -> Option<api::NodeLatency> {
        self.latency.get().remove(tag)
    }

    /// A probe is running: the button must not start a second one.
    pub fn latency_busy(&self) -> bool {
        self.latency_busy.get()
    }

    /// Measure every node, keeping results as they arrive so the list fills in rather
    /// than waiting for the slowest node. A second press while a probe runs is ignored.
    pub fn measure_latency(self) {
        if self.latency_busy.get_untracked() || self.nodes.get_untracked().is_empty() {
            return;
        }
        let nodes = self.nodes.get_untracked();
        let generation = self.latency_generation.get_untracked();
        self.latency_busy.set(true);
        self.work(async move {
            let Ok(session) = api::start_node_latency(nodes, Vec::new()).await else {
                self.latency_busy.set(false);
                return;
            };
            // A bounded poll: a core that stops answering must not leave the button
            // spinning for ever. 600 x 100ms, as the Dioxus app caps it.
            for _ in 0..600 {
                if self.latency_generation.get_untracked() != generation {
                    self.latency_busy.set(false);
                    return;
                }
                // The cancel flag is checked each tick, so stopping keeps what has
                // already come back rather than discarding a partial run.
                if self.latency_cancel.get_untracked() {
                    self.latency_cancel.set(false);
                    break;
                }
                match api::poll_node_latency(session).await {
                    Ok(snapshot) => {
                        if !snapshot.results.is_empty()
                            && self.latency_generation.get_untracked() == generation
                        {
                            self.latency.update(|stored| {
                                for result in snapshot.results {
                                    stored.insert(result.tag.clone(), result);
                                }
                            });
                        }
                        if snapshot.done {
                            break;
                        }
                    }
                    // Keep what already arrived: dropping the batch threw measurements
                    // away on a late failure.
                    Err(_) => break,
                }
                timer(std::time::Duration::from_millis(100)).await;
            }
            self.latency_busy.set(false);
        });
    }

    /// Stop the current probe, keeping the results already returned.
    pub fn cancel_latency(self) {
        self.latency_cancel.set(true);
    }

    /// Note that config moved away from what the core is running.
    pub fn mark_config_dirty(self) {
        self.config_revision.update(|n| *n = n.wrapping_add(1));
    }

    /// Switch between rule, global and direct modes. A running core restarts so the
    /// change actually applies.
    /// Turn TUN on or off.
    ///
    /// A running core picks it up through a restart. A stopped one is different in one
    /// direction only: switching it off has to hand back the privileged helper, which stays
    /// installed otherwise. Switching it on does not need to acquire it here, because
    /// `toggle_core` asks for it before starting and reports a refusal where the press is.
    pub fn set_tun_enabled(self, enable: bool) {
        if self.tun_enabled.get_untracked() == enable {
            return;
        }
        self.tun_enabled.set(enable);
        self.mark_config_dirty();
        self.persist_profile();
        if self.connected.get_untracked() {
            self.restart_core();
            return;
        }
        if !enable {
            self.core_busy.set(CoreBusy::Tun);
            self.work(async move {
                match api::release_tun_mode().await {
                    Ok(()) => self.feedback.success("TUN 模式已关闭"),
                    Err(error) => {
                        self.feedback.error(format!("关闭 TUN 失败：{error}"));
                        self.refresh_status();
                    }
                }
                self.core_busy.set(CoreBusy::Idle);
            });
        }
    }

    /// Whether other machines on the network may use the proxy. Off means the listener is
    /// loopback only, which is what the port's label on the overview says.
    pub fn set_allow_lan(self, enable: bool) {
        if self.allow_lan.get_untracked() == enable {
            return;
        }
        self.allow_lan.set(enable);
        self.mark_config_dirty();
        self.persist_profile();
        if self.connected.get_untracked() {
            self.restart_core();
        }
    }

    pub fn set_tunnel_mode(self, mode: TunnelMode) {
        if self.tunnel_mode.get_untracked() == mode {
            return;
        }
        self.tunnel_mode.set(mode);
        self.mark_config_dirty();
        self.persist_profile();
        if self.connected.get_untracked() {
            self.restart_core();
        }
    }

    /// Write the store's state to disk, reporting the outcome. Returns the error text so a
    /// caller that is mid-flow (importing, say) can fold it into its own message instead of
    /// two notices arriving for one action.
    async fn save(self) -> Result<(), String> {
        let mut profile = api::load_profile()
            .await
            .map_err(|error| error.to_string())?;
        profile.subscriptions = self.subscriptions.get_untracked();
        profile.active_subscription_id = self.active_subscription_id.get_untracked();
        profile.selected_tag = self.selected_tag.get_untracked();
        profile.tunnel_mode = self.tunnel_mode.get_untracked();
        profile.tun_enabled = self.tun_enabled.get_untracked();
        profile.allow_lan = self.allow_lan.get_untracked();
        profile.dark_mode = self.dark_mode.get_untracked();
        profile.custom_rules = self.custom_rules.get_untracked();
        profile.dns_overrides = self.dns_overrides.get_untracked();
        profile.config_script_enabled = self.config_script_enabled.get_untracked();
        profile.config_script = self.config_script.get_untracked();
        profile.group_selections = self.group_selections.get_untracked();
        api::save_profile(profile)
            .await
            .map_err(|error| error.to_string())
    }

    /// Fire-and-forget save for a screen that has already said what it did. Failures are
    /// still reported: a silent one means the edit vanishes on the next launch.
    fn persist_profile(self) {
        self.work(async move {
            if let Err(error) = self.save().await {
                self.feedback.error(format!("配置保存失败：{error}"));
            }
        });
    }
}

impl Default for AppStore {
    fn default() -> Self {
        Self::new()
    }
}

/// The store every screen reads, provided once by the root.
pub fn store() -> AppStore {
    use_context::<AppStore>().expect("the root provides the store")
}
