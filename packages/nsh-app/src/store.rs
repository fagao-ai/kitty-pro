//! The app's state: signals with private writers, read by the screens.
//!
//! One owner per piece of state. The screens call these methods and never touch a signal
//! directly, so what a press starts stays in one place and cancels the way the code says.

pub use crate::domain::SystemProxy;
use crate::{backend, i18n};
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

/// What the last exit self-check said. Kept as the app's own pair rather than the backend's
/// `ConnectivityProbe` so a screen never has to unwrap an `Option` inside a binding just to
/// pick a color.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeOutcome {
    pub ok: bool,
    pub detail: String,
}

/// Which remote-sync action is in flight.
///
/// Four actions rather than one busy flag: 保存 touches only `sync.json`, 上传 and 覆盖远端
/// touch the remote, and 下载覆盖本机 touches the remote *and* rewrites the local profile. A
/// single flag would let a press during any of them start another, and the spinner would name
/// the wrong work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Save,
    Push,
    ForcePush,
    Pull,
}

/// What the operating system says about starting at login.
///
/// Read from the OS rather than remembered in the profile: the launch agent or Run key is a
/// file the user or an installer can remove at any time, and a switch holding the last value
/// the app wrote would claim a login item that no longer exists.
#[derive(Debug, Clone, PartialEq)]
pub enum Autostart {
    /// Not read yet.
    Loading,
    /// Whether it is on, as the OS answered.
    Ready(bool),
    /// The read itself failed, with the reason.
    Failed(String),
    /// This build cannot offer it: a mobile shell has no login item to write.
    Unsupported,
}

impl Autostart {
    pub fn enabled(&self) -> bool {
        matches!(self, Autostart::Ready(true))
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Autostart::Ready(_))
    }

    /// Whether the row should be drawn at all. A control that cannot be acted on is worse
    /// than no control, so a platform with no login item gets neither the switch nor an
    /// explanation of it.
    pub fn is_supported(&self) -> bool {
        !matches!(self, Autostart::Unsupported)
    }

    pub fn note(&self, busy: bool) -> String {
        if busy {
            return "正在写入当前用户的登录项".to_owned();
        }
        match self {
            Autostart::Loading => "正在读取登录项".to_owned(),
            Autostart::Failed(error) => error.clone(),
            Autostart::Unsupported => String::new(),
            Autostart::Ready(enabled) => {
                if *enabled {
                    // Launching the app is all a login item can do. It does not connect, does not
                    // open the listener and does not raise TUN — nothing writes those on launch,
                    // and a note claiming otherwise would leave the machine unproxied after a
                    // reboot while the setting read as if it were.
                    "登录后自动启动，不自动连接".to_owned()
                } else {
                    "手动打开后才工作".to_owned()
                }
            }
        }
    }
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
    /// The exit self-check: whether one is running, and what the last one said. The result
    /// stays on screen until the next press because the answer is worth rereading, and a
    /// notice ages out while the user is still deciding whether the proxy is usable.
    probe_busy: Signal<bool>,
    probe_result: Signal<Option<ProbeOutcome>>,
    /// Whether the app is registered to start at login, as the OS last answered. Read rather
    /// than remembered: the entry can be deleted with the app running.
    autostart: Signal<Autostart>,
    autostart_busy: Signal<bool>,
    /// Whether a proxy-environment copy is in flight. Its own flag because it is a different
    /// action from setting the system proxy, and a spinner borrowed from one would report the
    /// wrong work on the wrong control.
    env_copy_busy: Signal<bool>,
    /// The rule editor's process picker: the list, and whether it has arrived. Owned by the
    /// store rather than the dialog because the read outlives the press that asked for it, and
    /// a picker closed and reopened while a read runs must not start a second one.
    processes: Signal<Vec<api::RunningProcess>>,
    processes_loaded: Signal<bool>,
    processes_busy: Signal<bool>,
    /// Why the last process read failed, kept so a reopened picker can say what went wrong
    /// rather than showing an empty list that looks like an idle machine.
    processes_error: Signal<Option<String>>,
    /// The remote-sync settings, as `sync.json` last answered.
    ///
    /// `None` means *unread*, which is different from an empty config: the form's buttons stay
    /// parked while it is `None`, because saving an untouched `SyncConfig::default()` would
    /// overwrite the real settings with blanks. That is the one failure this field exists to
    /// prevent rather than merely record.
    sync_config: Signal<Option<api::SyncConfig>>,
    /// Why the sync settings could not be read, alongside the retry the card offers.
    sync_load_error: Signal<Option<String>>,
    /// Which sync action is in flight, so one press cannot start two and the right button
    /// carries the spinner. Save, push, force-push and pull are four different pieces of work
    /// against two different places, and a spinner on the wrong one is a lie about progress.
    sync_busy: Signal<Option<SyncAction>>,
    /// The last refusal that only the user can resolve, with the raw backend text behind it.
    sync_conflict: Signal<Option<(String, String)>>,
    /// What the remote held the last time it was read, for the pull confirm summary.
    sync_remote_counts: Signal<Option<crate::domain::SyncCounts>>,
    /// The last answer from the release feed, or `None` while nothing has answered.
    ///
    /// Kept apart from `update_error`: a status is an answer about releases and an error is the
    /// absence of one. Folding them together makes a failed check read as "no update", which is
    /// the one confusion that leaves a user on an old build.
    update_status: Signal<Option<api::update::AppUpdateStatus>>,
    /// The remembered update settings, `None` until read.
    update_settings: Signal<Option<api::update::UpdateSettings>>,
    update_busy: Signal<bool>,
    update_error: Signal<Option<String>>,
    /// The version the launch notice already gave, so a re-check that finds the same release
    /// cannot stack a second one.
    update_announced: Signal<Option<String>>,
    /// The core's proxy groups, generated from the config the current state would start with.
    ///
    /// A resource rather than a signal the page fills: it re-loads exactly when the config it
    /// describes changes, and NSH drops a superseded load, so a fast sequence of edits cannot
    /// have an older preview land on top of a newer one. It is `Copy` like a signal, so it
    /// travels with the store.
    groups: Resource<Vec<proxy_core::ProxyGroup>, String>,
    feedback: crate::feedback::Feedback,
    /// The poll generation: a restart or disconnect invalidates in-flight reads.
    poll_generation: Signal<u64>,
    /// The owner every task this store starts belongs to.
    owner: Owner,
}

impl AppStore {
    pub fn new() -> Self {
        // The config a proxy-group preview is generated from, held as locals so the store's
        // fields and the watcher's source name the *same* signals. Listing them is also the
        // clearest statement of what the core's group set depends on.
        let nodes = signal(Vec::new());
        let selected_tag = signal(String::new());
        let subscriptions = signal(Vec::new());
        let active_subscription_id = signal(None);
        let tunnel_mode = signal(TunnelMode::Rule);
        let tun_enabled = signal(false);
        let allow_lan = signal(false);
        let custom_rules = signal(Vec::new());
        let dns_overrides = signal(Vec::new());
        let config_script = signal(String::new());
        let config_script_enabled = signal(false);
        let group_selections = signal(HashMap::new());
        Self {
            screen: signal(Screen::Overview),
            dark_mode: signal(false),
            core_state: signal("checking".to_owned()),
            core_version: signal(None),
            core_note: signal(None),
            connected: signal(false),
            core_busy: signal(CoreBusy::Idle),
            nodes,
            selected_tag,
            tunnel_mode,
            tun_enabled,
            allow_lan,
            group_selections,
            custom_rules,
            dns_overrides,
            config_script,
            config_script_enabled,
            subscriptions,
            active_subscription_id,
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
            probe_busy: signal(false),
            probe_result: signal(None),
            autostart: signal(Autostart::Loading),
            autostart_busy: signal(false),
            env_copy_busy: signal(false),
            processes: signal(Vec::new()),
            processes_loaded: signal(false),
            processes_busy: signal(false),
            processes_error: signal(None),
            sync_config: signal(None),
            sync_load_error: signal(None),
            sync_busy: signal(None),
            sync_conflict: signal(None),
            sync_remote_counts: signal(None),
            update_status: signal(None),
            update_settings: signal(None),
            update_busy: signal(false),
            update_error: signal(None),
            update_announced: signal(None),
            groups: watch_groups(WatchInputs {
                nodes,
                selected_tag,
                subscriptions,
                active_subscription_id,
                tunnel_mode,
                tun_enabled,
                allow_lan,
                custom_rules,
                dns_overrides,
                config_script,
                config_script_enabled,
                group_selections,
            }),
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
            match backend::load_profile().await {
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
        // The stored `proxy` choice is brought in line with the selection the file resolves to.
        // This is the one boundary where the two can disagree through no fault of this app: a
        // profile written by the Dioxus shell, pulled from another device, or edited by hand can
        // name a node in one field and a different one in the other, and the engine obeys the
        // group entry. Normalizing here rather than at every write is what lets the rest of the
        // store assume the pair agrees; other groups' entries are kept exactly as stored.
        let mut group_selections = profile.group_selections;
        crate::domain::sync_proxy_group_selection(&mut group_selections, &selected);
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
        self.group_selections.set(group_selections);
        // A stored choice outranks the system appearance. Pushing it to the engine is
        // the whole point: setting the signal alone would relabel the switch while the
        // window stayed in the appearance the system asked for.
        self.apply_appearance(profile.dark_mode);
        // Same rule as the appearance: a stored choice outranks everything else, and applying it
        // is a write to the global the labels read, not a rebuild.
        crate::i18n::set_lang(crate::i18n::Lang::from_stored(&profile.language));
    }

    /// The DNS servers the active subscription's nodes need to be resolved.
    ///
    /// A provider that ships nodes behind hostnames it resolves itself expects its own
    /// resolvers: without them the app asks the system, which can return nothing for a
    /// hostname only that provider's DNS knows, and the connection fails as an unexplained
    /// timeout. Only the active subscription's servers apply, matching the nodes in use.
    pub fn proxy_server_nameservers(&self) -> Vec<String> {
        proxy_server_nameservers_from(
            &self.subscriptions.get_untracked(),
            self.active_subscription_id.get_untracked(),
        )
    }

    /// The request the core starts with, built from current state.
    ///
    /// Reads untracked: callers are press handlers, where a tracked read would subscribe the
    /// widget to a value it only needs once. `watch_groups` is the one place that wants the
    /// same set tracked, and it goes through `build_request` so the two cannot drift.
    pub fn connection_request(&self) -> ConnectionRequest {
        build_request(RequestParts {
            nodes: self.nodes.get_untracked(),
            selected_tag: self.selected_tag.get_untracked(),
            nameservers: self.proxy_server_nameservers(),
            mode: self.tunnel_mode.get_untracked(),
            tun: self.tun_enabled.get_untracked(),
            allow_lan: self.allow_lan.get_untracked(),
            custom_rules: self.custom_rules.get_untracked(),
            dns_overrides: self.dns_overrides.get_untracked(),
            config_script_enabled: self.config_script_enabled.get_untracked(),
            config_script: self.config_script.get_untracked(),
            group_selections: self.group_selections.get_untracked(),
        })
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
                if let Err(error) = backend::prepare_tun_mode(tun_request).await {
                    self.core_busy.set(CoreBusy::Idle);
                    self.feedback
                        .error(i18n::fill("准备 TUN 权限失败，内核未启动：{}", &[&error]));
                    self.refresh_status();
                    return;
                }
            }
            match backend::set_core_enabled(enable, request).await {
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
            match backend::system_proxy_status().await {
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
            match backend::set_system_proxy(enable).await {
                Ok(status) => {
                    self.system_proxy.set(SystemProxy::Ready(status));
                    self.feedback.success(if enable {
                        "系统代理已启用"
                    } else {
                        "系统代理已关闭"
                    });
                }
                Err(error) => {
                    self.feedback
                        .error(i18n::fill("系统代理设置失败：{}", &[&error]));
                    match backend::system_proxy_status().await {
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
    ///
    /// The busy check is here rather than only inside `restart_now` so a second press in the
    /// same frame is refused before it can queue a second restart.
    pub fn restart_core(self) {
        if self.core_busy.get_untracked() != CoreBusy::Idle {
            return;
        }
        self.work(async move {
            let _ = self.restart_now().await;
        });
    }

    /// The same, answering whether the core came back.
    ///
    /// The caller that needs the answer is a node pick whose live switch was refused: it has
    /// already written the new choice into the list, so a restart that fails has to put the
    /// old one back rather than leave the screen naming an exit the core is not using.
    async fn restart_now(self) -> Result<(), String> {
        if self.core_busy.get_untracked() != CoreBusy::Idle {
            return Err("内核正在处理其他请求".to_owned());
        }
        self.core_busy.set(CoreBusy::Restarting);
        self.bump_generation();
        let request = self.connection_request();
        let result = backend::restart_core(request).await;
        self.core_busy.set(CoreBusy::Idle);
        match result {
            Ok(status) => {
                self.apply_core_status(status, true);
                self.config_revision.set(0);
                self.start_polling();
                Ok(())
            }
            Err(error) => {
                let text = error.to_string();
                self.core_note.set(Some(text.clone()));
                Err(text)
            }
        }
    }

    /// One status read, used on first paint and by the "重新检测" action.
    pub fn refresh_status(self) {
        self.work(async move {
            match backend::core_status().await {
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
                let Ok(traffic) = backend::core_traffic().await else {
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
            let Ok(mut profile) = backend::load_profile().await else {
                return;
            };
            profile.dark_mode = self.dark_mode.get_untracked();
            let _ = backend::save_profile(profile).await;
        });
    }

    /// Record the language choice, alone, for the same reason `persist_theme` writes alone: the
    /// language is the only field this path owns, so a concurrent edit to anything else in the
    /// profile is not overwritten from a copy taken before it.
    fn persist_language(self) {
        self.work(async move {
            let Ok(mut profile) = backend::load_profile().await else {
                return;
            };
            profile.language = crate::i18n::lang_untracked().as_str().to_owned();
            let _ = backend::save_profile(profile).await;
        });
    }

    /// Draw in `next` and remember it. Returns whether the choice moved, so a repeat press does
    /// not rewrite the file.
    ///
    /// Nothing rebuilds here: the labels read the language reactively, so setting it repaints
    /// what is on screen, which is the whole mechanism (see `crate::i18n`).
    pub fn set_language(self, next: crate::i18n::Lang) {
        if crate::i18n::set_lang(next) {
            self.persist_language();
        }
    }

    /// The language the interface is drawn in, as a signal read so the picker follows a change
    /// made anywhere — including by a sync pull.
    pub fn language(&self) -> crate::i18n::Lang {
        crate::i18n::lang()
    }

    /// Point the node list, the selection and the group choice at one tag. The three are one
    /// decision, and the group entry is the one the engine applies last.
    fn set_selected(self, tag: &str) {
        self.selected_tag.set(tag.to_owned());
        self.group_selections
            .update(|selections| crate::domain::sync_proxy_group_selection(selections, tag));
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
            if let Err(error) = backend::set_core_log_collection(enable).await {
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
                match backend::core_logs(cursor).await {
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
            let result = backend::preview_subscription(source.clone()).await;
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
                        self.feedback
                            .error(i18n::fill("配置保存失败：{}", &[&error]));
                    }
                    let mut message =
                        i18n::fill("已导入并切换到 {}，共 {} 个节点", &[&display, &count]);
                    if rejected > 0 {
                        message.push_str(&i18n::fill("，{} 个无法解析", &[&rejected]));
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
                        .set(Some(i18n::fill("订阅解析失败：{}", &[&reason])));
                }
                Err(error) => self
                    .import_error
                    .set(Some(i18n::fill("订阅解析失败：{}", &[&error]))),
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
                .success(i18n::fill("已切换到 {}", &[&(subscription.name)]));
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
        self.set_selected(&selected);
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
                match backend::preview_subscription(source).await {
                    Ok(report) => {
                        let mut outcome = None;
                        self.subscriptions.update(|stored| {
                            outcome =
                                Some(crate::domain::apply_subscription_report(stored, id, report));
                        });
                        match outcome {
                            Some(Ok(_)) => refreshed += 1,
                            Some(Err(reason)) => failures.push(format!("{name}：{reason}")),
                            None => failures.push(i18n::fill("{}：订阅已不在列表中", &[&name])),
                        }
                    }
                    Err(error) => failures.push(format!("{name}：{error}")),
                }
            }
            self.after_refresh();
            if failures.is_empty() {
                self.feedback.success("订阅已刷新，可重新连接或启动");
            } else {
                self.feedback.error(i18n::fill(
                    "已刷新 {} 个订阅，{} 个失败：{}",
                    &[&refreshed, &(failures.len()), &(failures.join("；"))],
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
        // Prune first, then write the pair. Pruning drops a choice whose node no longer exists;
        // a refresh can also leave an entry that is *not* stale and still not the selection — the
        // old tag survives under a different subscription — and that is the shape the engine
        // obeys over `selected_tag`, so `set_selected` has to run after it, not before.
        self.group_selections.update(|selections| {
            crate::domain::prune_stale_group_selections(selections, &live);
        });
        self.set_selected(&selected);
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
            self.feedback
                .info(i18n::fill("已删除 {}，重新连接后生效", &[&name]));
        } else {
            self.feedback.success(i18n::fill("已删除 {}", &[&name]));
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
            return Err(i18n::fill(
                "最多添加 {} 条规则",
                &[&(proxy_core::MAX_CUSTOM_RULES)],
            ));
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
        .map_err(|error| crate::domain::rule_error_message(&error, self.language()))?;

        if let Some(existing) = rules.iter_mut().find(|rule| rule.id == id) {
            *existing = candidate;
        } else if editing_id.is_some() {
            return Err(i18n::tr("要编辑的规则不存在"));
        } else {
            rules.push(candidate);
        }
        proxy_core::validate_custom_rules(&rules)
            .map_err(|error| crate::domain::rule_error_message(&error, self.language()))?;
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

    /// Validate and add one dedicated-DNS override, rejecting a duplicate.
    ///
    /// Duplicate means the same kind *and* the same value, which is narrower than the old
    /// value-alone test now that the kind is chosen: `example.com` as an exact host and as a suffix
    /// are two different promises about which names go to that resolver, and refusing the second
    /// because the first exists would make the kind picker useless.
    pub fn add_dns_override(
        self,
        match_type: proxy_core::CustomRuleMatch,
        value: &str,
        server: &str,
    ) -> Result<(), String> {
        let (match_type, value, server) =
            proxy_core::normalize_dns_override_match(match_type, value, server)?;
        let mut overrides = self.dns_overrides.get_untracked();
        if overrides
            .iter()
            .any(|item| item.match_type == match_type && item.domain_suffix == value)
        {
            return Err(i18n::fill("已存在 {} 的专用 DNS", &[&value]));
        }
        if overrides.len() >= proxy_core::MAX_CUSTOM_RULES {
            return Err(i18n::fill(
                "最多添加 {} 条规则",
                &[&(proxy_core::MAX_CUSTOM_RULES)],
            ));
        }
        let id = crate::domain::next_dns_override_id(&overrides);
        overrides.push(proxy_core::DnsOverride {
            id,
            enabled: true,
            match_type,
            domain_suffix: value,
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
            let result = backend::validate_config_script(request).await;
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
            let result = backend::update_rule_sets(true).await;
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
            let Ok(session) = backend::start_node_latency(nodes, Vec::new()).await else {
                self.latency_busy.set(false);
                return;
            };
            // A bounded poll: a core that stops answering must not leave the button
            // spinning for ever. 600 x 100ms, the budget this app has always used.
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
                match backend::poll_node_latency(session).await {
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
                match backend::release_tun_mode().await {
                    Ok(()) => self.feedback.success("TUN 模式已关闭"),
                    Err(error) => {
                        self.feedback
                            .error(i18n::fill("关闭 TUN 失败：{}", &[&error]));
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
        let mut profile = backend::load_profile()
            .await
            .map_err(|error| error.to_string())?;
        profile.subscriptions = self.subscriptions.get_untracked();
        profile.active_subscription_id = self.active_subscription_id.get_untracked();
        profile.selected_tag = self.selected_tag.get_untracked();
        profile.tunnel_mode = self.tunnel_mode.get_untracked();
        profile.tun_enabled = self.tun_enabled.get_untracked();
        profile.allow_lan = self.allow_lan.get_untracked();
        profile.dark_mode = self.dark_mode.get_untracked();
        profile.language = crate::i18n::lang_untracked().as_str().to_owned();
        profile.custom_rules = self.custom_rules.get_untracked();
        profile.dns_overrides = self.dns_overrides.get_untracked();
        profile.config_script_enabled = self.config_script_enabled.get_untracked();
        profile.config_script = self.config_script.get_untracked();
        profile.group_selections = self.group_selections.get_untracked();
        backend::save_profile(profile)
            .await
            .map_err(|error| error.to_string())
    }

    /// Fire-and-forget save for a screen that has already said what it did. Failures are
    /// still reported: a silent one means the edit vanishes on the next launch.
    fn persist_profile(self) {
        self.work(async move {
            if let Err(error) = self.save().await {
                self.feedback
                    .error(i18n::fill("配置保存失败：{}", &[&error]));
            }
        });
    }

    /// The core's proxy groups, as the engine generated them from the current config.
    ///
    /// Handed out as the resource rather than as a plain `Option` so a page can tell the three
    /// answers apart: no value and loading is "still generating", no value and an error is "the
    /// engine refused, here is why", and `Some(vec![])` is "this config has no groups", which is
    /// a fact about the subscription rather than a moment.
    pub fn groups(&self) -> Resource<Vec<proxy_core::ProxyGroup>, String> {
        self.groups
    }

    /// Every remembered group choice, for deriving a group's current member.
    pub fn group_selections(&self) -> HashMap<String, String> {
        self.group_selections.get()
    }

    /// The measured latencies, so a group can order its members by what answers.
    pub fn latency_results(&self) -> HashMap<String, api::NodeLatency> {
        self.latency.get()
    }

    /// Ask the running core to move one group to another outbound.
    ///
    /// The live API is tried first and a restart is the fallback, not the other way round:
    /// restarting drops the listener and, with TUN, the tunnel's routes and the session-scoped
    /// DNS for the length of a full start, which is long enough to break connections that had
    /// nothing to do with the change. The live call answers in microseconds and leaves
    /// everything already flowing alone.
    ///
    /// Both the group entry and, for the built-in `proxy` group, the node selection move
    /// together, and they move whether the live call worked or the restart had to do it: the
    /// engine reads `group_selections` after `selected_tag` when it builds a config, so an
    /// entry left on the old node would undo the pick on the next start. A choice inside another
    /// group touches only that group's entry, since nothing else derives from it.
    pub fn select_group_outbound(self, group: String, outbound: String) {
        if self.core_action_busy() {
            return;
        }
        let previous = self.group_selections.get_untracked();
        let previous_selected = self.selected_tag.get_untracked();
        if group == "proxy" {
            self.set_selected(&outbound);
        } else {
            self.group_selections.update(|selections| {
                selections.insert(group.clone(), outbound.clone());
            });
        }
        self.persist_profile();

        let label = format!(
            "{group} → {}",
            crate::domain::outbound_display_name(&outbound, &self.nodes.get_untracked())
        );
        if !self.connected.get_untracked() {
            // Nothing is running to apply it, so the choice is saved and will be used by the
            // next start. Saying 已切换 alone would overstate it, and marking the config pending
            // would be noise: nothing is running to disagree with it.
            self.feedback
                .success(i18n::fill("{}（未连接，启动后生效）", &[&label]));
            return;
        }
        let group_for_call = group.clone();
        let outbound_for_call = outbound.clone();
        self.work(async move {
            if backend::select_proxy_group(group_for_call, outbound_for_call)
                .await
                .is_ok()
            {
                self.feedback
                    .success(i18n::fill("{} 已切换，无需重新连接", &[&label]));
                return;
            }
            self.feedback.info("内核未能直接切换，正在重新连接以应用");
            if self.restart_now().await.is_ok() {
                return;
            }
            // Neither path applied it, and the core is still on the old outbound. Put the
            // screen back to what is actually running rather than let it name a choice the
            // engine refused; `restart_now` has already parked the reason in the core note.
            self.group_selections.set(previous);
            self.selected_tag.set(previous_selected);
            // Saved again, because the press saved the choice that was then refused. Leaving it
            // on disk would mean a restart at any later point — a reboot, a TUN change — picks
            // up an exit the user watched the app reject.
            self.persist_profile();
            self.feedback
                .error(i18n::fill("{} 切换失败，仍是原来的出口", &[&label]));
        });
    }

    /// The last exit self-check, if one has run.
    pub fn probe_result(&self) -> Option<ProbeOutcome> {
        self.probe_result.get()
    }

    pub fn probe_busy(&self) -> bool {
        self.probe_busy.get()
    }

    /// Dial out through the proxy the core is serving and report what came back.
    ///
    /// This is the check the connection state cannot give: a running core with a working
    /// listener is not the same claim as traffic that reaches the internet. The probe goes
    /// through `127.0.0.1:7890`, so it exercises the whole chain the user depends on — the
    /// listener, the selected exit, and whatever the provider does on the far side — and it is
    /// the only way to notice that a node answers a latency ping but cannot actually carry a
    /// connection.
    ///
    /// Off while the core is not connected: with nothing behind the port the answer would be a
    /// connection refusal, which reads as "your proxy is broken" when the truth is "you have
    /// not started it", and the button is parked for that reason rather than the press
    /// reporting a failure it caused itself.
    pub fn probe_connectivity(self) {
        if self.probe_busy.get_untracked() {
            return;
        }
        self.probe_busy.set(true);
        self.probe_result.set(None);
        self.work(async move {
            let outcome = match backend::probe_connectivity().await {
                Ok(probe) => ProbeOutcome {
                    ok: probe.ok,
                    detail: probe.detail,
                },
                Err(error) => ProbeOutcome {
                    ok: false,
                    detail: error.to_string(),
                },
            };
            self.probe_result.set(Some(outcome));
            self.probe_busy.set(false);
        });
    }

    /// What the operating system says about starting at login. Called on boot and after a
    /// refused write, since the login item is a file outside the app's control.
    pub fn refresh_autostart(self) {
        if !crate::autostart::SUPPORTED {
            self.autostart.set(Autostart::Unsupported);
            return;
        }
        self.work(async move {
            match crate::autostart::status().await {
                Ok(enabled) => self.autostart.set(Autostart::Ready(enabled)),
                Err(error) => self.autostart.set(Autostart::Failed(error)),
            }
        });
    }

    pub fn autostart(&self) -> Autostart {
        self.autostart.get()
    }

    pub fn autostart_busy(&self) -> bool {
        self.autostart_busy.get()
    }

    /// Register or unregister the app as a login item, then read the answer back.
    ///
    /// The write is followed by a read because the backend itself does this: a login item that
    /// the OS did not accept is a different fact from one it refused to remove, and a switch
    /// left on the pressed value would claim a login item that does not exist.
    pub fn set_autostart(self, enable: bool) {
        if self.autostart_busy.get_untracked() {
            return;
        }
        self.autostart_busy.set(true);
        self.work(async move {
            match crate::autostart::set_enabled(enable).await {
                Ok(enabled) => {
                    self.autostart.set(Autostart::Ready(enabled));
                    self.feedback.success(if enabled {
                        "已加入登录启动项"
                    } else {
                        "已移出登录启动项"
                    });
                }
                Err(error) => {
                    self.feedback
                        .error(i18n::fill("登录启动项设置失败：{}", &[&error]));
                    match crate::autostart::status().await {
                        Ok(enabled) => self.autostart.set(Autostart::Ready(enabled)),
                        Err(refresh) => self.autostart.set(Autostart::Failed(refresh)),
                    }
                }
            }
            self.autostart_busy.set(false);
        });
    }

    /// Copy the shell environment variables that point a command-line tool at the core, for a
    /// terminal whose program does not read the operating system's proxy setting.
    ///
    /// Goes through the system clipboard rather than NSH's own, because that is the part of this
    /// feature the operating system owns: the point is to paste the lines into a terminal outside
    /// the app, where a clipboard the app keeps to itself would deliver nothing. It shells out to
    /// `pbcopy` and its equivalents and can fail when none is installed, which is reported as the
    /// backend words it rather than softened.
    pub fn copy_proxy_environment(self) {
        if self.env_copy_busy.get_untracked() {
            return;
        }
        self.env_copy_busy.set(true);
        self.work(async move {
            let result = backend::copy_proxy_environment_variables()
                .await
                .map_err(|error| error.to_string());
            self.env_copy_busy.set(false);
            self.feedback
                .settled("代理环境变量", result.map(|_| "已复制代理环境变量"));
        });
    }

    pub fn env_copy_busy(&self) -> bool {
        self.env_copy_busy.get()
    }

    /// Which shell this is, in the words the update check classifies on.
    ///
    /// Which release asset this build should be offered.
    ///
    /// The decision itself is `domain::update_channel_label`, which is where the three
    /// answers are pinned against the vocabulary the update check reads; this only feeds
    /// it the two facts. `cfg!(target_arch = "wasm32")` is the web arm because a page
    /// compiled to WebAssembly is the only build that cannot load the Go core, and it
    /// has to be asked before NSH's `is_desktop()`, which is true of a browser on a
    /// desktop idiom. The same string is what the about card prints.
    pub fn platform_label(&self) -> &'static str {
        crate::domain::update_channel_label(cfg!(target_arch = "wasm32"), is_desktop())
    }

    /// The state the about card reports, read from the signals it is shown next to.
    ///
    /// Assembled here rather than at the call site: the report is only honest if every line
    /// comes from the same instant the card is drawn, and a screen that read nine signals in
    /// nine bindings could mix two moments.
    pub fn diagnostic_facts(&self) -> crate::domain::DiagnosticFacts {
        crate::domain::DiagnosticFacts {
            app_version: crate::domain::APP_VERSION.to_owned(),
            platform: self.platform_label().to_owned(),
            core_state: self.core_state.get(),
            core_version: self.core_version.get(),
            tunnel_mode: self.tunnel_mode.get(),
            tun_enabled: self.tun_enabled.get(),
            allow_lan: self.allow_lan.get(),
            node_count: self.nodes.get().len(),
            subscription_count: self.subscriptions.get().len(),
            rule_count: self.custom_rules.get().len(),
            dns_count: self.dns_overrides.get().len(),
        }
    }

    /// Put the diagnostic text on the system clipboard.
    ///
    /// NSH's clipboard, not a subprocess: the report is pasted into a bug tracker or a chat
    /// window, which is exactly what the system clipboard is for, and the app is a native
    /// process so there is nothing between the two. There is no busy flag for this one, unlike
    /// `copy_proxy_environment`: the engine's write is synchronous and gives no answer back, so
    /// reserving a spinner for a result that can never arrive would park the button for one
    /// frame and report nothing the user could act on.
    pub fn copy_diagnostic_report(self) {
        let text = crate::domain::diagnostic_report(&self.diagnostic_facts());
        set_clipboard(text);
        self.feedback.success("诊断信息已复制到剪贴板");
    }

    /// Read the machine's running processes for the rule editor's picker.
    ///
    /// Asked for when the picker opens rather than at boot: the list answers a question the
    /// user is asking now, and a snapshot taken at startup is wrong by the time it is used.
    /// The read is blocking work in the backend, so it is started here rather than in the
    /// dialog — closing the dialog must not cancel a read whose result the next opening wants.
    pub fn refresh_processes(self) {
        if self.processes_busy.get_untracked() {
            return;
        }
        self.processes_busy.set(true);
        self.processes_loaded.set(false);
        self.processes_error.set(None);
        self.work(async move {
            let result = backend::list_running_processes()
                .await
                .map_err(|error| error.to_string());
            self.processes_busy.set(false);
            match result {
                Ok(processes) => {
                    self.processes_loaded.set(true);
                    self.processes.set(processes);
                }
                Err(error) => self.processes_error.set(Some(error)),
            }
        });
    }

    /// The picker's list, before any filter.
    pub fn processes(&self) -> Vec<api::RunningProcess> {
        self.processes.get()
    }

    pub fn processes_loaded(&self) -> bool {
        self.processes_loaded.get()
    }

    pub fn processes_busy(&self) -> bool {
        self.processes_busy.get()
    }

    pub fn processes_error(&self) -> Option<String> {
        self.processes_error.get()
    }

    // ---------------------------------------------------------------- sync

    /// Read the sync settings. Runs once, from the root, beside the other boot reads.
    ///
    /// The result is kept as `Option` rather than defaulted for one reason: every sync control
    /// is parked until this lands, because saving a default config over a real one is a data
    /// loss the user cannot see coming and cannot undo.
    pub fn refresh_sync_config(self) {
        self.work(async move {
            match backend::load_sync_config().await {
                Ok(config) => {
                    self.sync_config.set(Some(config));
                    self.sync_load_error.set(None);
                }
                Err(error) => {
                    self.sync_config.set(None);
                    self.sync_load_error.set(Some(error.to_string()));
                }
            }
        });
    }

    /// The settings as read, or `None` while unread. Handed to the form field by field so the
    /// page never writes a signal it does not own.
    pub fn sync_config(&self) -> Option<api::SyncConfig> {
        self.sync_config.get()
    }

    pub fn sync_load_error(&self) -> Option<String> {
        self.sync_load_error.get()
    }

    pub fn sync_busy(&self) -> Option<SyncAction> {
        self.sync_busy.get()
    }

    pub fn sync_conflict(&self) -> Option<(String, String)> {
        self.sync_conflict.get()
    }

    pub fn sync_remote_counts(&self) -> Option<crate::domain::SyncCounts> {
        self.sync_remote_counts.get()
    }

    /// Whether the card can act. Unread and failed-both park the controls, which is the point of
    /// keeping the config as an `Option`.
    pub fn sync_ready(&self) -> bool {
        self.sync_config.get().is_some()
    }

    pub fn sync_status(&self) -> crate::domain::SyncStatus {
        match self.sync_config.get() {
            None => crate::domain::SyncStatus::Loading,
            Some(config) if !config.enabled => crate::domain::SyncStatus::Disabled,
            Some(config) if config.last_sync_at == 0 => crate::domain::SyncStatus::Pending,
            Some(config) => crate::domain::SyncStatus::Synced(config.last_sync_at),
        }
    }

    /// One field of the form, written back into the config signal.
    ///
    /// A setter rather than a public signal so no screen can hold a stale copy of the whole
    /// config across a read-modify-write, which is how one keystroke would undo another.
    pub fn set_sync_field(self, field: crate::domain::SyncField, value: String) {
        let Some(mut config) = self.sync_config.get_untracked() else {
            return;
        };
        field.write(&mut config, &value);
        self.sync_config.set(Some(config));
    }

    pub fn set_sync_enabled(self, enabled: bool) {
        let Some(mut config) = self.sync_config.get_untracked() else {
            return;
        };
        config.enabled = enabled;
        self.sync_config.set(Some(config));
    }

    pub fn set_sync_provider(self, s3: bool) {
        let Some(mut config) = self.sync_config.get_untracked() else {
            return;
        };
        config.provider = if s3 {
            api::SyncProviderKind::S3
        } else {
            api::SyncProviderKind::WebDav
        };
        self.sync_config.set(Some(config));
    }

    /// The action every sync press takes before touching the network: claim the slot, or refuse.
    ///
    /// Returns `None` when something is already running, which is also the guard against a
    /// double-press in one frame — the reads and writes here are not idempotent.
    fn claim_sync(self, action: SyncAction) -> Option<api::SyncConfig> {
        if self.sync_busy.get_untracked().is_some() {
            return None;
        }
        let config = self.sync_config.get_untracked()?;
        self.sync_busy.set(Some(action));
        Some(config)
    }

    /// Finish an action, saying what happened and whether only the user can resolve it.
    fn settle_sync(self, action: SyncAction, result: Result<String, String>) {
        self.sync_busy.set(None);
        match result {
            Ok(message) => {
                self.feedback.success(message);
                self.sync_conflict.set(None);
            }
            Err(error) => {
                // Redaction is the backend's job — every sync error it builds goes through its
                // own `redact` before it leaves `packages/api` — so the text is shown as it
                // arrived.
                let (friendly, is_conflict) = crate::domain::classify_sync_error(&error);
                // Both pieces are looked up before being joined. A toast holds finished text and
                // is translated as it is drawn, so a string assembled here would reach the table
                // with a sentence inside it and match nothing.
                self.feedback.error(i18n::fill(
                    "{}：{}",
                    &[&i18n::tr(sync_action_name(action)), &i18n::tr(&friendly)],
                ));
                if is_conflict {
                    self.sync_conflict.set(Some((friendly, error)));
                }
            }
        }
    }

    /// Save the settings without contacting the remote.
    ///
    /// Separate from uploading because a user who has just typed an address wants it kept, not
    /// used: the two are different promises and the buttons say which one is being made.
    pub fn save_sync_config(self) {
        let Some(config) = self.claim_sync(SyncAction::Save) else {
            return;
        };
        self.work(async move {
            let result = backend::save_sync_config(config)
                .await
                .map(|saved| {
                    self.sync_config.set(Some(saved));
                    "同步配置已保存".to_owned()
                })
                .map_err(|error| error.to_string());
            self.settle_sync(SyncAction::Save, result);
        });
    }

    /// Upload the local settings, refusing to overwrite a remote someone else moved.
    pub fn sync_push(self) {
        self.run_sync_push(false);
    }

    /// Upload and overwrite whatever is there, which is the way out of a conflict and the way in
    /// for a first upload to a target that already exists.
    pub fn sync_force_push(self) {
        self.run_sync_push(true);
    }

    fn run_sync_push(self, force: bool) {
        let action = if force {
            SyncAction::ForcePush
        } else {
            SyncAction::Push
        };
        let Some(config) = self.claim_sync(action) else {
            return;
        };
        let snapshot = self.build_snapshot();
        self.work(async move {
            let result = backend::sync_push(config, snapshot, force)
                .await
                .map(|result| {
                    self.remember_sync_checkpoint(&result);
                    crate::domain::sync_result_clause(
                        result.subscription_count,
                        result.rule_count,
                        result.dns_rule_count,
                        crate::i18n::lang(),
                    )
                })
                .map_err(|error| error.to_string());
            self.settle_sync(action, result);
        });
    }

    /// Download the remote snapshot and make it local.
    ///
    /// Two backend calls, and the gap between them is the whole reason this is careful: the
    /// download is not applied until the second call has backed up the local profile. A failure
    /// in between leaves the machine as it was, so it is reported as exactly that rather than as
    /// a failed download.
    pub fn sync_pull(self) {
        let Some(config) = self.claim_sync(SyncAction::Pull) else {
            return;
        };
        let current = self.snapshot_profile();
        self.work(async move {
            let downloaded = match backend::sync_pull(config.clone()).await {
                Ok(downloaded) => downloaded,
                Err(error) => {
                    self.settle_sync(SyncAction::Pull, Err(error.to_string()));
                    return;
                }
            };
            let snapshot = downloaded.snapshot;
            let counts = crate::domain::SyncCounts {
                remote_subscriptions: snapshot.subscriptions.len(),
                remote_rules: snapshot.custom_rules.len(),
            };
            let result = backend::commit_sync_pull(
                config,
                current,
                snapshot.clone(),
                downloaded.remote_revision,
            )
            .await
            .map(|result| {
                self.remember_sync_checkpoint(&result);
                self.sync_remote_counts.set(Some(counts));
                // The file on disk is now the remote's, so the store has to become it too. Going
                // through `apply_profile` rather than writing fields here is what keeps a pulled
                // profile subject to the same reconciliation as one read at boot.
                self.apply_profile_from_disk();
                crate::domain::sync_result_clause(
                    result.subscription_count,
                    result.rule_count,
                    result.dns_rule_count,
                    crate::i18n::lang(),
                )
            })
            .map_err(|error| i18n::fill("远端数据已下载，但保存到本机失败：{}", &[&error]));
            self.settle_sync(SyncAction::Pull, result);
        });
    }

    /// Re-read the profile from disk and put it in the store.
    ///
    /// Used after a pull, where the file has changed underneath the app. It deliberately does
    /// not go through `load_profile`: that one parks the core on a read failure, and here the
    /// read has already succeeded once — the pull wrote the file.
    fn apply_profile_from_disk(self) {
        self.work(async move {
            if let Ok(profile) = backend::load_profile().await {
                self.apply_profile(profile);
                if self.connected.get_untracked() {
                    self.mark_config_dirty();
                }
            }
        });
    }

    /// The part of local state that travels to another device.
    ///
    /// Built here rather than from `SyncSnapshot::from_profile` because the store holds the
    /// fields as signals and has no on-disk profile in hand mid-session; the two paths must stay
    /// in step, which is what the shared `SyncSubscription::from` conversion is for.
    fn build_snapshot(self) -> proxy_core::SyncSnapshot {
        proxy_core::SyncSnapshot {
            format: proxy_core::SYNC_SNAPSHOT_FORMAT.to_owned(),
            version: proxy_core::SYNC_SNAPSHOT_VERSION,
            updated_at: self
                .sync_config
                .get_untracked()
                .map_or(0, |config| config.last_sync_at),
            subscriptions: self
                .subscriptions
                .get_untracked()
                .iter()
                .map(proxy_core::SyncSubscription::from)
                .collect(),
            custom_rules: self.custom_rules.get_untracked(),
            dns_overrides: Some(self.dns_overrides.get_untracked()),
            config_script: Some(proxy_core::SyncConfigScript {
                enabled: self.config_script_enabled.get_untracked(),
                script: self.config_script.get_untracked(),
            }),
        }
    }

    /// The current state as a profile, for the pull's merge. The backend applies the snapshot
    /// onto this and writes the result, so device-local fields have to come from here rather
    /// than from a default — that is how a pull once erased local DNS rules.
    fn snapshot_profile(self) -> AppProfile {
        AppProfile {
            version: AppProfile::default().version,
            subscriptions: self.subscriptions.get_untracked(),
            active_subscription_id: self.active_subscription_id.get_untracked(),
            selected_tag: self.selected_tag.get_untracked(),
            tunnel_mode: self.tunnel_mode.get_untracked(),
            tun_enabled: self.tun_enabled.get_untracked(),
            allow_lan: self.allow_lan.get_untracked(),
            dark_mode: self.dark_mode.get_untracked(),
            // Travels with the appearance for the same reason: both are how the user wants the
            // app to look, and a pull that restored one while resetting the other would leave a
            // machine half-restored.
            language: crate::i18n::lang_untracked().as_str().to_owned(),
            custom_rules: self.custom_rules.get_untracked(),
            dns_overrides: self.dns_overrides.get_untracked(),
            config_script_enabled: self.config_script_enabled.get_untracked(),
            config_script: self.config_script.get_untracked(),
            group_selections: self.group_selections.get_untracked(),
        }
    }

    /// Keep the baseline the remote just agreed to, so the next push is a compare-and-swap
    /// against this revision instead of a blind write.
    fn remember_sync_checkpoint(self, result: &api::SyncResult) {
        if !result.checkpoint_saved {
            if let Some(warning) = &result.warning {
                self.feedback.info(warning.clone());
            }
            return;
        }
        let Some(mut config) = self.sync_config.get_untracked() else {
            return;
        };
        config.last_sync_at = result.updated_at;
        config.last_remote_revision = result.remote_revision.clone();
        self.sync_config.set(Some(config));
        if let Some(warning) = &result.warning {
            self.feedback.info(warning.clone());
        }
    }
}

/// What a sync action was called to do, for the sentence that reports its failure.
fn sync_action_name(action: SyncAction) -> &'static str {
    match action {
        SyncAction::Save => "同步配置保存失败",
        SyncAction::Push => "远端上传失败",
        SyncAction::ForcePush => "覆盖远端失败",
        SyncAction::Pull => "远端下载失败",
    }
}

// ------------------------------------------------------------------- updates

impl AppStore {
    /// Read the remembered update settings. Runs once, from the root.
    ///
    /// The settings and the last answer come from the same file on the backend, so this loads
    /// both; the card needs the throttle timestamp to say when it last ran, and needs the
    /// previous answer to avoid calling a failed check "已是最新".
    pub fn refresh_update_state(self) {
        self.work(async move {
            match backend::load_update_settings().await {
                Ok(settings) => self.update_settings.set(Some(settings)),
                Err(error) => self
                    .update_error
                    .set(Some(i18n::fill("更新设置读取失败：{}", &[&error]))),
            }
        });
        self.run_update_check(false);
    }

    /// Check for a newer release. `force` is what a press means: the daily throttle is the
    /// app's courtesy on a launch, not a reason to refuse someone who asked.
    pub fn check_for_update(self) {
        self.run_update_check(true);
    }

    fn run_update_check(self, force: bool) {
        if self.update_busy.get_untracked() {
            return;
        }
        self.update_busy.set(true);
        self.work(async move {
            let request = api::update::AppUpdateRequest {
                platform: self.platform_label().to_owned(),
                current_version: crate::domain::APP_VERSION.to_owned(),
                force,
            };
            match backend::check_app_update(request).await {
                Ok(status) => {
                    // An answer replaces any earlier failure; a failure inside an otherwise
                    // good answer (a state file that would not write) is carried by the status
                    // itself, so the previous answer stays visible beside it.
                    //
                    // Only the status is written here. The settings signal has three other
                    // writers (the boot read, the switch, the dismissal) and each owns its own
                    // copy of the truth; a check that also wrote it would race with the boot read
                    // and could overwrite the remembered `auto_check` with a default. The card
                    // reads the check's timestamp off the status, which carries it anyway.
                    self.update_error.set(None);
                    // Read before the status is replaced: the rule for whether this answer earns
                    // a notice belongs to `domain`, where a test can drive it without a network.
                    let announce = crate::domain::update_announcement(
                        self.update_announced.get_untracked().as_deref(),
                        Some(&status),
                    );
                    self.update_status.set(Some(status));
                    if let Some(version) = announce {
                        self.update_announced.set(Some(version.clone()));
                        self.feedback
                            .info(i18n::fill("发现新版本 v{}", &[&version]));
                    }
                }
                Err(error) => {
                    // Kept rather than shown once and dropped: the card is usually not on screen
                    // when a launch check fails, and a notice ages out before the user gets there.
                    self.update_error.set(Some(error.to_string()));
                }
            }
            self.update_busy.set(false);
        });
    }

    pub fn update_status(&self) -> Option<api::update::AppUpdateStatus> {
        self.update_status.get()
    }

    pub fn update_settings(&self) -> Option<api::update::UpdateSettings> {
        self.update_settings.get()
    }

    pub fn update_busy(&self) -> bool {
        self.update_busy.get()
    }

    pub fn update_error(&self) -> Option<String> {
        self.update_error.get()
    }

    /// Whether the settings have been read. Until they have, the card cannot tell "no update"
    /// from "nothing asked yet", and saying the wrong one is the whole failure mode here.
    pub fn update_ready(&self) -> bool {
        self.update_settings.get().is_some()
    }

    pub fn update_display_state(&self) -> api::update::AppUpdateState {
        crate::domain::update_display_state(
            self.update_status.get().as_ref(),
            self.update_error.get().as_deref(),
        )
    }

    pub fn update_is_actionable(&self) -> bool {
        crate::domain::update_is_actionable(self.update_status.get().as_ref())
    }

    /// Turn the daily check on or off.
    pub fn set_update_auto_check(self, auto_check: bool) {
        if self.update_busy.get_untracked() {
            return;
        }
        let Some(settings) = self.update_settings.get_untracked() else {
            return;
        };
        self.update_busy.set(true);
        let request = api::update::UpdateSettings {
            auto_check,
            ..settings
        };
        self.work(async move {
            let result = backend::save_update_settings(request)
                .await
                .map_err(|error| error.to_string());
            self.update_busy.set(false);
            match result {
                Ok(saved) => {
                    self.update_settings.set(Some(saved));
                    self.feedback.success(if auto_check {
                        "已开启每日自动检查"
                    } else {
                        "已改为仅手动检查"
                    });
                }
                Err(error) => self
                    .feedback
                    .error(i18n::fill("更新设置保存失败：{}", &[&error])),
            }
        });
    }

    /// Stop presenting the current newest release as an update.
    ///
    /// Takes the version rather than a flag because ignoring a release is a statement about that
    /// release: the next one has to ask again, which a permanent "don't show updates" would not.
    pub fn dismiss_update(self) {
        let Some(version) = self
            .update_status
            .get_untracked()
            .and_then(|status| status.latest.map(|release| release.version))
        else {
            return;
        };
        if self.update_busy.get_untracked() {
            return;
        }
        self.update_busy.set(true);
        self.work(async move {
            let result = backend::dismiss_app_update(version.clone())
                .await
                .map_err(|error| error.to_string());
            self.update_busy.set(false);
            match result {
                Ok(settings) => {
                    self.update_settings.set(Some(settings));
                    // Re-render from the same answer with the flag set, rather than re-asking the
                    // feed: the release has not changed, only the user's stance toward it has.
                    if let Some(mut status) = self.update_status.get_untracked() {
                        status.snoozed = true;
                        self.update_status.set(Some(status));
                    }
                    self.feedback.success("已忽略这个版本，下次有更新再提醒");
                }
                Err(error) => self.feedback.error(i18n::fill("忽略失败：{}", &[&error])),
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

/// The pieces a `ConnectionRequest` is assembled from, before the two reductions the engine's
/// request applies (which rules survive, whether a script counts as an override).
///
/// This exists so the group preview can watch exactly the inputs the core's config is built
/// from. Deriving them is the whole point: a preview that tracked a hand-picked subset would
/// quietly keep showing yesterday's groups after an edit outside the subset.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestParts {
    nodes: Vec<ProxyNode>,
    selected_tag: String,
    nameservers: Vec<String>,
    mode: TunnelMode,
    tun: bool,
    allow_lan: bool,
    custom_rules: Vec<proxy_core::CustomRule>,
    dns_overrides: Vec<proxy_core::DnsOverride>,
    config_script_enabled: bool,
    config_script: String,
    group_selections: HashMap<String, String>,
}

/// Assemble the request the core starts with. The one place the two reductions live, shared by
/// the real start path and the preview, so a preview cannot describe a config other than the one
/// the core would get.
fn build_request(parts: RequestParts) -> ConnectionRequest {
    let RequestParts {
        nodes,
        selected_tag,
        nameservers,
        mode,
        tun,
        allow_lan,
        custom_rules,
        dns_overrides,
        config_script_enabled,
        config_script,
        group_selections,
    } = parts;
    let script_applied = config_script_enabled && !config_script.trim().is_empty();
    ConnectionRequest {
        nodes,
        selected_tag,
        proxy_server_nameservers: nameservers,
        mode,
        tun,
        allow_lan,
        custom_rules: crate::domain::connection_custom_rules(
            config_script_enabled,
            &config_script,
            custom_rules,
            crate::domain::process_rules_supported(),
        ),
        dns_overrides,
        config_script: script_applied.then_some(config_script),
        group_selections,
    }
}

/// The active subscription's resolvers. Only its servers apply, matching the nodes in use.
fn proxy_server_nameservers_from(
    subscriptions: &[proxy_core::Subscription],
    active: Option<u64>,
) -> Vec<String> {
    subscriptions
        .iter()
        .find(|subscription| Some(subscription.id) == active)
        .map(|subscription| subscription.proxy_server_nameservers.clone())
        .unwrap_or_default()
}

/// The signals a group preview watches. One struct so the list is written once, next to the
/// preview that consumes it.
#[derive(Clone, Copy)]
struct WatchInputs {
    nodes: Signal<Vec<ProxyNode>>,
    selected_tag: Signal<String>,
    subscriptions: Signal<Vec<proxy_core::Subscription>>,
    active_subscription_id: Signal<Option<u64>>,
    tunnel_mode: Signal<TunnelMode>,
    tun_enabled: Signal<bool>,
    allow_lan: Signal<bool>,
    custom_rules: Signal<Vec<proxy_core::CustomRule>>,
    dns_overrides: Signal<Vec<proxy_core::DnsOverride>>,
    config_script: Signal<String>,
    config_script_enabled: Signal<bool>,
    group_selections: Signal<HashMap<String, String>>,
}

/// The core's proxy groups, generated from the config the current state would start with.
///
/// Asked of the engine rather than assembled here. `build_singbox_config` is what decides that
/// a node list becomes one `selector` group named `proxy` plus `direct` and `block`, and a
/// second implementation of that decision in the UI would be a second truth to keep in step.
/// The call is pure Rust and touches no core: no process starts and no port opens, which is
/// what lets the nodes page ask it on every edit.
///
/// `resource` rather than a signal a page fills: it reloads exactly when its source changes and
/// NSH drops a superseded load, so a fast run of edits cannot have an older group list land on
/// top of a newer one. With no nodes there is nothing to group, so the source returns `None` and
/// nothing is generated at all.
fn watch_groups(inputs: WatchInputs) -> Resource<Vec<proxy_core::ProxyGroup>, String> {
    resource(
        move || {
            let parts = RequestParts {
                nodes: inputs.nodes.get(),
                selected_tag: inputs.selected_tag.get(),
                nameservers: proxy_server_nameservers_from(
                    &inputs.subscriptions.get(),
                    inputs.active_subscription_id.get(),
                ),
                mode: inputs.tunnel_mode.get(),
                tun: inputs.tun_enabled.get(),
                allow_lan: inputs.allow_lan.get(),
                custom_rules: inputs.custom_rules.get(),
                dns_overrides: inputs.dns_overrides.get(),
                config_script_enabled: inputs.config_script_enabled.get(),
                config_script: inputs.config_script.get(),
                group_selections: inputs.group_selections.get(),
            };
            (!parts.nodes.is_empty()).then_some(parts)
        },
        |parts, _info| async move {
            backend::preview_proxy_groups(build_request(parts))
                .await
                .map_err(|error| error.to_string())
        },
    )
}
