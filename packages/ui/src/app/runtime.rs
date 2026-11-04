//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

pub(crate) const ANDROID_VPN_WAITING_NOTICE: &str = "正在等待 Android VPN 授权或启动服务";

/// Signals describing the current tunnel/routing configuration. Grouped so
/// connection-request builders avoid a long positional argument list.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct RuntimeSettings {
    pub(crate) tunnel_mode: Signal<TunnelMode>,
    pub(crate) tun_enabled: Signal<bool>,
    pub(crate) allow_lan: Signal<bool>,
    pub(crate) custom_rules: Signal<Vec<CustomRule>>,
    pub(crate) dns_overrides: Signal<Vec<DnsOverride>>,
    pub(crate) config_script_enabled: Signal<bool>,
    pub(crate) config_script: Signal<String>,
}

pub(crate) fn build_connection_request(
    nodes: Vec<ProxyNode>,
    selected_tag: String,
    proxy_server_nameservers: Vec<String>,
    settings: RuntimeSettings,
    group_selections: HashMap<String, String>,
) -> ConnectionRequest {
    let RuntimeSettings {
        tunnel_mode,
        tun_enabled,
        allow_lan,
        custom_rules,
        dns_overrides,
        config_script_enabled,
        config_script,
    } = settings;
    let script_enabled = config_script_enabled();
    let script = config_script();
    let custom_rules = connection_custom_rules(script_enabled, &script, custom_rules());
    let config_script = if script_enabled && !script.trim().is_empty() {
        Some(script)
    } else {
        None
    };

    ConnectionRequest {
        nodes,
        selected_tag,
        proxy_server_nameservers,
        mode: tunnel_mode(),
        tun: tun_enabled(),
        allow_lan: allow_lan(),
        custom_rules,
        dns_overrides: dns_overrides(),
        config_script,
        group_selections,
    }
}

/// Signals mirroring the sing-box core status back into the UI after a
/// (re)start, grouped to keep `restart_core_from_ui`'s signature small.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct CoreStatusSignals {
    pub(crate) connected: Signal<bool>,
    pub(crate) core_restarting: Signal<bool>,
    pub(crate) core_state: Signal<String>,
    pub(crate) core_version: Signal<Option<String>>,
    pub(crate) core_note: Signal<Option<String>>,
    /// The OS proxy can change underneath the UI whenever the core goes down:
    /// a failed restart transaction disables it server-side before reporting
    /// `CoreOffline`. Every failure path re-reads it through
    /// `refresh_runtime_state`, so this lives with the other status signals
    /// rather than at the handful of call sites that used to remember it.
    pub(crate) system_proxy: Signal<SystemProxyLoadState>,
    /// Set while the running core lags behind local edits (refreshed
    /// subscriptions, imported sources, edited rules). Cleared by the
    /// restart/start that applies the current config - every path that
    /// accepts a freshly-built `ConnectionRequest` runs through
    /// `apply_core_status` or `restart_core_from_ui`, so clearing lives here
    /// once instead of at each call site.
    pub(crate) config_pending: Signal<bool>,
}

/// Re-read the core (and, on request, the OS proxy) after a failed operation
/// so the UI converges on what the backend actually did. Without the proxy
/// re-read the switch kept showing "已启用" after a failed restart had already
/// turned the system proxy off, and the next click then toggled from a lie.
pub(crate) async fn refresh_runtime_state(status: CoreStatusSignals, include_system_proxy: bool) {
    let CoreStatusSignals {
        mut connected,
        core_restarting: _,
        mut core_state,
        mut core_version,
        mut core_note,
        mut system_proxy,
        // A failed-status refresh does not apply any pending config, so it
        // must leave the "待生效" flag standing.
        config_pending: _,
    } = status;
    match api::core_status().await {
        Ok(core) => {
            connected.set(core.state == "running");
            core_state.set(core.state);
            core_version.set(core.version);
            core_note.set(core.note);
        }
        Err(status_error) => {
            connected.set(false);
            core_state.set("unavailable".to_string());
            core_note.set(Some(status_error.to_string()));
        }
    }
    if include_system_proxy {
        match api::system_proxy_status().await {
            Ok(proxy_status) => system_proxy.set(SystemProxyLoadState::Ready(proxy_status)),
            Err(error) => system_proxy.set(SystemProxyLoadState::Failed(error.to_string())),
        }
    }
}

pub(crate) async fn restart_core_from_ui(
    request: ConnectionRequest,
    status: CoreStatusSignals,
    toast: ToastManager,
    success_message: impl Into<String>,
) -> bool {
    let CoreStatusSignals {
        mut connected,
        mut core_restarting,
        mut core_state,
        mut core_version,
        mut core_note,
        mut config_pending,
        ..
    } = status;
    let success_message = success_message.into();
    // Re-entry guard for every caller, including buttons whose busy check runs
    // inside the event future (after a render tick, too late for a double
    // click). Callers no longer have to remember this signal in their guards.
    if *core_restarting.peek() {
        return false;
    }
    core_restarting.set(true);
    let success = match api::restart_core(request).await {
        Ok(status) => {
            let is_running = status.state == "running";
            connected.set(is_running);
            core_state.set(status.state);
            core_version.set(status.version);
            core_note.set(status.note);
            config_pending.set(false);
            if is_running {
                toast.success(success_message);
            } else {
                // Accepted but not up yet (Android VPN consent pending). The
                // backend DID apply the new config and is starting against it,
                // so this stays a success: rolling the caller's optimistic
                // switch back here would show the old node while the core comes
                // up on the new one. `connected` is truthfully false meanwhile,
                // and the waiting notice explains the gap.
                toast.info(ANDROID_VPN_WAITING_NOTICE);
            }
            true
        }
        Err(error) => {
            toast.error(format!("内核重启失败: {error}"));
            refresh_runtime_state(status, true).await;
            false
        }
    };
    core_restarting.set(false);
    success
}

/// Busy/status signals shared by every runtime knob that restarts the core
/// (TUN, LAN listening, system proxy). Grouping them keeps the helpers below
/// small enough for both the settings page and the phone quick toggles.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct RuntimeSignals {
    pub(crate) status: CoreStatusSignals,
    pub(crate) core_busy: Signal<bool>,
    pub(crate) system_proxy_busy: Signal<bool>,
    pub(crate) runtime_action_busy: Signal<bool>,
}

impl RuntimeSignals {
    pub(crate) fn busy(self) -> bool {
        (self.core_busy)()
            || (self.status.core_restarting)()
            || (self.system_proxy_busy)()
            || (self.runtime_action_busy)()
    }
}

/// Toggle TUN mode. With the core stopped the privileged helper is prepared or
/// released; while running, the core restarts with the new config.
/// Mirror a core status response back into the shared status signals.
pub(crate) fn apply_core_status(status: api::ApiCoreStatus, signals: CoreStatusSignals) -> bool {
    let CoreStatusSignals {
        mut connected,
        core_restarting: _,
        mut core_state,
        mut core_version,
        mut core_note,
        mut config_pending,
        ..
    } = signals;
    let is_running = status.state == "running";
    connected.set(is_running);
    core_state.set(status.state);
    core_version.set(status.version);
    core_note.set(status.note);
    // This helper only ever runs on an accepted (re)start built from the
    // current local config, so the core now matches the UI.
    if is_running {
        config_pending.set(false);
    }
    is_running
}

/// The saved request with the TUN switch moved to `enabled`.
///
/// Requests are built from the `tun_enabled`/`allow_lan` signals, and those only
/// change once the backend has accepted the toggle, so the request a click builds
/// still carries the position being switched away from. Stamping the new value
/// here - the one place both switches route through - means no call site has to
/// remember to do it.
///
/// Skipping it is not merely a wrong config: with the core stopped,
/// `prepare_tun_mode` rejects a request whose `tun` is false, and while running
/// the core restarts on the old value yet the UI still reports the new one.
pub(crate) fn request_with_tun(mut request: ConnectionRequest, enabled: bool) -> ConnectionRequest {
    request.tun = enabled;
    request
}

/// `request_with_tun` for the LAN switch. Same trap, quieter failure: the core
/// restarts on the old bind address and the toast still claims it changed.
pub(crate) fn request_with_allow_lan(
    mut request: ConnectionRequest,
    enabled: bool,
) -> ConnectionRequest {
    request.allow_lan = enabled;
    request
}

pub(crate) fn spawn_tun_toggle(
    enabled: bool,
    request: ConnectionRequest,
    mut tun_enabled: Signal<bool>,
    mut tun_busy: Signal<bool>,
    runtime: RuntimeSignals,
    toast: ToastManager,
) {
    if runtime.busy() {
        return;
    }
    let request = request_with_tun(request, enabled);
    let CoreStatusSignals {
        connected,
        mut core_restarting,
        ..
    } = runtime.status;
    let mut runtime_action_busy = runtime.runtime_action_busy;
    let previous_enabled = tun_enabled();
    tun_busy.set(true);
    runtime_action_busy.set(true);
    // `core_restarting` drives the "正在重启 sing-box 内核" banner and locks the
    // power button. Setting it for the permission-only path told the user the
    // core was restarting while the OS was actually waiting on an elevation
    // prompt, so it is armed just before the real restart below instead.
    spawn(async move {
        if !connected() {
            let result = if enabled {
                api::prepare_tun_mode(request.clone()).await
            } else {
                api::release_tun_mode().await
            };
            match result {
                Ok(()) => {
                    tun_enabled.set(enabled);
                    toast.success(if enabled {
                        "TUN 权限已准备，启动内核时直接生效"
                    } else {
                        "TUN 模式已关闭"
                    });
                }
                Err(error) => {
                    tun_enabled.set(previous_enabled);
                    if !enabled {
                        // The local `connected` state can be stale (page reload,
                        // failed status poll), so the core may actually still be
                        // running and refuse a bare helper release. Refresh the
                        // real state and fall back to restarting without TUN.
                        match api::core_status().await {
                            Ok(status) if status.state == "running" => {
                                core_restarting.set(true);
                                match api::restart_core(request).await {
                                    Ok(status) if status.state == "running" => {
                                        apply_core_status(status, runtime.status);
                                        tun_enabled.set(false);
                                        toast.success("TUN 模式已关闭");
                                    }
                                    Ok(status) => {
                                        let state = status.state.clone();
                                        apply_core_status(status, runtime.status);
                                        toast.error(format!(
                                            "关闭 TUN 失败: 内核未以新配置运行（当前状态 {state}），请重试"
                                        ));
                                    }
                                    Err(restart_error) => {
                                        toast.error(format!("关闭 TUN 失败: {restart_error}"));
                                        refresh_runtime_state(runtime.status, true).await;
                                    }
                                }
                                core_restarting.set(false);
                            }
                            Ok(_) => {
                                toast.error(format!("关闭 TUN 失败: {error}"));
                            }
                            Err(status_error) => {
                                toast.error(format!(
                                    "关闭 TUN 失败: {error}；刷新内核状态失败: {status_error}"
                                ));
                            }
                        }
                    } else {
                        toast.error(format!("TUN 权限准备失败: {error}"));
                    }
                }
            }
            tun_busy.set(false);
            runtime_action_busy.set(false);
            return;
        }

        core_restarting.set(true);
        match api::restart_core(request).await {
            Ok(status) if status.state == "running" => {
                apply_core_status(status, runtime.status);
                tun_enabled.set(enabled);
                toast.success(if enabled {
                    "TUN 模式已启用"
                } else {
                    "TUN 模式已关闭"
                });
            }
            Ok(status) => {
                // Ok, but not up yet (Android VPN consent pending). The backend
                // accepted and applied the new config, so the switch keeps the
                // clicked position; `apply_core_status` records that the core
                // itself is still starting.
                apply_core_status(status, runtime.status);
                tun_enabled.set(enabled);
                toast.info(ANDROID_VPN_WAITING_NOTICE);
            }
            Err(error) => {
                tun_enabled.set(previous_enabled);
                refresh_runtime_state(runtime.status, true).await;
                toast.error(format!("应用 TUN 设置失败: {error}"));
            }
        }
        tun_busy.set(false);
        core_restarting.set(false);
        runtime_action_busy.set(false);
    });
}

/// Toggle LAN listening. While the core runs it restarts on the new bind
/// address; while stopped the preference applies to the next start.
pub(crate) fn spawn_allow_lan_toggle(
    enabled: bool,
    request: ConnectionRequest,
    mut allow_lan: Signal<bool>,
    mut lan_busy: Signal<bool>,
    runtime: RuntimeSignals,
    toast: ToastManager,
) {
    if runtime.busy() {
        return;
    }
    let request = request_with_allow_lan(request, enabled);
    let CoreStatusSignals {
        connected,
        mut core_restarting,
        ..
    } = runtime.status;
    let mut runtime_action_busy = runtime.runtime_action_busy;
    let previous_enabled = allow_lan();
    // Both busy signals are armed synchronously, before the spawn: arming
    // `core_restarting` inside the task left a window where a second knob
    // (the power button) passed its guard and fired a concurrent restart.
    lan_busy.set(true);
    runtime_action_busy.set(true);
    core_restarting.set(true);
    spawn(async move {
        if !connected() {
            allow_lan.set(enabled);
            toast.info(if enabled {
                "局域网连接将在下次启动内核时生效"
            } else {
                "仅本机访问将在下次启动内核时生效"
            });
            core_restarting.set(false);
            lan_busy.set(false);
            runtime_action_busy.set(false);
            return;
        }
        match api::restart_core(request).await {
            Ok(status) if status.state == "running" => {
                apply_core_status(status, runtime.status);
                allow_lan.set(enabled);
                toast.success(if enabled {
                    "已允许局域网连接，sing-box 内核已重启"
                } else {
                    "已恢复仅本机访问，sing-box 内核已重启"
                });
            }
            Ok(status) => {
                apply_core_status(status, runtime.status);
                allow_lan.set(enabled);
                toast.info(ANDROID_VPN_WAITING_NOTICE);
            }
            Err(error) => {
                allow_lan.set(previous_enabled);
                refresh_runtime_state(runtime.status, true).await;
                toast.error(format!("应用局域网监听设置失败: {error}"));
            }
        }
        core_restarting.set(false);
        lan_busy.set(false);
        runtime_action_busy.set(false);
    });
}

/// Start or stop the core from the UI. The overview power button and the log
/// page's "启动代理开始采集" affordance share this so both keep the TUN
/// pre-flight, the status refresh on failure, and the system-proxy re-read.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct CoreToggleSignals {
    pub(crate) status: CoreStatusSignals,
    pub(crate) core_busy: Signal<bool>,
    pub(crate) system_proxy_busy: Signal<bool>,
    pub(crate) runtime_action_busy: Signal<bool>,
}

pub(crate) fn spawn_core_toggle(
    request: Option<ConnectionRequest>,
    signals: CoreToggleSignals,
    toast: ToastManager,
) {
    let CoreToggleSignals {
        mut status,
        mut core_busy,
        system_proxy_busy,
        runtime_action_busy,
    } = signals;
    // Same guard set as every other runtime knob (`RuntimeSignals::busy`): the
    // power button used to ignore in-flight TUN/LAN restarts and system-proxy
    // writes, so starting while either was pending fired a concurrent call.
    if core_busy() || (status.core_restarting)() || (system_proxy_busy)() || (runtime_action_busy)()
    {
        return;
    }
    core_busy.set(true);
    let target = !(status.connected)();
    spawn(async move {
        // With TUN toggled on but the core stopped, the privileged helper is
        // gone and starting would fail with "TUN 权限尚未准备". Re-acquire the
        // permission (prompting for administrator access only when no
        // authorized helper exists) before starting, so the TUN switch stays a
        // persistent preference instead of a one-shot setup.
        // Only when TUN is actually enabled: the server rejects a pre-flight for
        // a non-TUN config, which would otherwise block a plain proxy start.
        if let Some(tun_request) = request.clone().filter(|req| target && req.tun) {
            if let Err(error) = api::prepare_tun_mode(tun_request).await {
                // The core may already be running behind a stale local state;
                // refresh before giving up so toggling again stays idempotent.
                match api::core_status().await {
                    Ok(status) if status.state == "running" => {}
                    Ok(_) => {
                        core_busy.set(false);
                        toast.error(format!("准备 TUN 权限失败，内核未启动: {error}"));
                        return;
                    }
                    Err(status_error) => {
                        core_busy.set(false);
                        toast.error(format!(
                            "准备 TUN 权限失败: {error}；刷新内核状态失败: {status_error}"
                        ));
                        return;
                    }
                }
            }
        }
        match api::set_core_enabled(target, request).await {
            Ok(core_status) => {
                let is_running = apply_core_status(core_status, status);
                if !target {
                    toast.success("连接已断开");
                } else if is_running {
                    toast.success("sing-box 已启动");
                } else {
                    toast.info(ANDROID_VPN_WAITING_NOTICE);
                }
            }
            Err(error) => {
                toast.error(error.to_string());
                refresh_runtime_state(status, false).await;
            }
        }
        // Stopping the core (successfully or not) can leave the OS proxy
        // pointing at a dead port; a failed start may also have torn it down.
        // Re-read after any stop and after any failed start.
        if !target || !(status.connected)() {
            match api::system_proxy_status().await {
                Ok(proxy_status) => {
                    (status.system_proxy).set(SystemProxyLoadState::Ready(proxy_status));
                }
                Err(error) => {
                    (status.system_proxy).set(SystemProxyLoadState::Failed(error.to_string()));
                }
            }
        }
        core_busy.set(false);
    });
}

/// Apply a system proxy change and always re-read the real status on failure.
pub(crate) fn spawn_system_proxy_toggle(
    enabled: bool,
    mut system_proxy: Signal<SystemProxyLoadState>,
    mut system_proxy_busy: Signal<bool>,
    core_action_busy: bool,
    toast: ToastManager,
) {
    if core_action_busy || system_proxy_busy() {
        return;
    }
    system_proxy_busy.set(true);
    spawn(async move {
        match api::set_system_proxy(enabled).await {
            Ok(status) => {
                system_proxy.set(SystemProxyLoadState::Ready(status));
                if enabled {
                    toast.success("系统代理已启用");
                } else {
                    toast.success("系统代理已关闭");
                }
            }
            Err(error) => {
                toast.error(format!("系统代理设置失败: {error}"));
                match api::system_proxy_status().await {
                    Ok(status) => {
                        system_proxy.set(SystemProxyLoadState::Ready(status));
                    }
                    Err(refresh_error) => {
                        system_proxy.set(SystemProxyLoadState::Failed(refresh_error.to_string()));
                    }
                }
            }
        }
        system_proxy_busy.set(false);
    });
}
