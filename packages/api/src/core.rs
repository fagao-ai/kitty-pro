//! Core lifecycle, native config build, profile persistence and logs.
//!
//! Split out of `lib.rs`; the crate root re-exports everything through
//! `pub(crate) use ...::*`, so sibling modules keep using `use super::*`.

use super::*;

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn native_core_status() -> Result<ApiCoreStatus, ApiError> {
    use singbox::{unavailable_status, SingBox};

    // Start/restart transactions keep the slot lock for as long as the bind
    // retry window (up to 45s) or the TUN helper authorization wait (up to
    // 120s). Status polling must never queue behind them, so when the lock is
    // busy serve the last recorded snapshot instead of blocking.
    let Some(mut guard) = try_lock_core_slot()? else {
        if let Some(snapshot) = core_status_snapshot()
            .read()
            .ok()
            .and_then(|snapshot| snapshot.clone())
        {
            return Ok(snapshot);
        }
        return Ok(api_core_status(unavailable_status()));
    };
    let status = if let Some(core) = guard.as_mut() {
        core.engine.status()
    } else {
        match SingBox::discover().map(NativeCore::new) {
            Ok(core) => {
                let status = core.engine.status();
                *guard = Some(core);
                status
            }
            Err(_) => unavailable_status(),
        }
    };
    drop(guard);
    let status = api_core_status(status);
    if let Ok(mut snapshot) = core_status_snapshot().write() {
        *snapshot = Some(status.clone());
    }
    Ok(status)
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) fn native_core_status() -> Result<ApiCoreStatus, ApiError> {
    singbox::android::status()
        .map(api_core_status)
        .map_err(|error| ApiError::new(error.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn api_core_status(status: singbox::CoreStatus) -> ApiCoreStatus {
    use singbox::CoreState;

    let state = match status.state {
        CoreState::Unavailable => "unavailable",
        CoreState::Stopped => "stopped",
        CoreState::Running => "running",
    };
    ApiCoreStatus {
        state: state.to_string(),
        version: status.version,
        note: status.platform_note,
    }
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn load_native_profile() -> Result<AppProfile, ApiError> {
    use std::fs;

    // B16：统一崩溃恢复入口，放在读取配置之前。已确认不会递归：本函数只被
    // load_profile 调用，而 recover_interrupted_writes 仅用 profile_path() 定位
    // 目录、用 read_dir 清理残留，不会间接再调用 load_native_profile。
    // 恢复属于尽力而为：即便残留清理失败，也不能阻断用户读取配置。
    if let Err(error) = recover_interrupted_writes() {
        eprintln!("[kitty-pro] 崩溃恢复未完成（不中断配置读取）: {error}");
    }

    let path = profile_path()?;
    match fs::read(&path) {
        Ok(contents) => serde_json::from_slice(&contents).map_err(|error| {
            // Keep the unreadable file around. The UI starts from an empty
            // profile after a parse failure and saves over the path, so
            // without this copy one bad write would cost every subscription.
            let backup = path.with_extension("corrupt.json");
            if !backup.exists() {
                let _ = fs::copy(&path, &backup);
            }
            ApiError::new(format!("本地配置损坏: {error}"))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppProfile::default()),
        Err(error) => Err(ApiError::new(format!("读取本地配置失败: {error}"))),
    }
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn load_native_profile() -> Result<AppProfile, ApiError> {
    Ok(AppProfile::default())
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_native_profile(profile: &AppProfile) -> Result<(), ApiError> {
    let path = profile_path()?;
    save_native_profile_to_path(profile, &path)
}

/// Number of pre-pull profile backups to retain.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAX_PROFILE_BACKUPS: usize = 10;

/// Copy the current local profile to `profile-backups/` before a sync pull
/// overwrites it. Returns an optional user-visible note describing the backup
/// (or the failure). Failures never block the pull itself.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn backup_local_profile_before_sync_pull() -> Result<Option<String>, ApiError> {
    use std::fs;

    let path = profile_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let directory = path
        .parent()
        .ok_or_else(|| ApiError::new("本地配置目录无效"))?
        .join("profile-backups");
    fs::create_dir_all(&directory)
        .map_err(|error| ApiError::new(format!("创建配置备份目录失败: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
    }

    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let backup = directory.join(format!("profile-{seconds}.json"));
    fs::copy(&path, &backup)
        .map_err(|error| ApiError::new(format!("备份本地配置失败: {error}")))?;

    let mut entries = fs::read_dir(&directory)
        .map(|reader| {
            reader
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|entry| {
                    entry
                        .file_name()
                        .map(|name| {
                            let name = name.to_string_lossy();
                            name.starts_with("profile-") && name.ends_with(".json")
                        })
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // Timestamped names sort chronologically; keep the newest ones.
    entries.sort();
    for stale in entries
        .iter()
        .take(entries.len().saturating_sub(MAX_PROFILE_BACKUPS))
    {
        let _ = fs::remove_file(stale);
    }

    Ok(Some(format!("覆盖前已备份本地配置到 {}", backup.display())))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn save_native_profile_to_path(
    profile: &AppProfile,
    path: &std::path::Path,
) -> Result<(), ApiError> {
    let _guard = profile_write_lock()
        .lock()
        .map_err(|_| ApiError::new("本地配置写入锁已损坏"))?;
    // B3/B15：统一走 atomic_write_json（临时文件 + `sync_all` + 安全替换），
    // 保留 pretty JSON 输出；彻底消除 Windows 「先删旧文件再 rename」的数据丢失窗口。
    atomic_write_json(path, profile)
        .map_err(|error| ApiError::new(format!("保存本地配置失败: {error}")))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn profile_write_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[allow(dead_code)] // B3：临时文件创建已统一到 atomic_write.rs，保留以兼容旧结构。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_unique_profile_temporary_file(
    path: &std::path::Path,
) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    loop {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), id));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;

            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn save_native_profile(_profile: &AppProfile) -> Result<(), ApiError> {
    Err(ApiError::new("浏览器端不能直接写入本地配置"))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn profile_path() -> Result<std::path::PathBuf, ApiError> {
    // E2E isolation: the Playwright suite runs the app against a locally
    // served release build, and the app's debounced autosave POSTs whatever
    // profile the browser holds. Without this override those stubbed writes
    // landed in the same shared data directory the desktop app uses and
    // clobbered the developer's real profile (2026-11-05). Run the test
    // server with KITTY_PROFILE_DIR pointing at a scratch directory and no
    // suite write can touch user-owned state.
    if let Ok(dir) = std::env::var("KITTY_PROFILE_DIR") {
        return Ok(std::path::PathBuf::from(dir).join("profile.json"));
    }
    let directories = directories::ProjectDirs::from("com", "kitty", "kitty-pro")
        .ok_or_else(|| ApiError::new("无法确定本地配置目录"))?;
    Ok(directories.data_local_dir().join("profile.json"))
}

#[cfg(target_os = "android")]
pub(crate) fn profile_path() -> Result<std::path::PathBuf, ApiError> {
    singbox::android::files_dir()
        .map(|directory| directory.join("profile.json"))
        .map_err(|error| ApiError::new(error.to_string()))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    options.open(path)
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn validate_native_config_script(request: ConnectionRequest) -> Result<(), ApiError> {
    if request.config_script.is_none() {
        return Err(ApiError::new("脚本内容为空"));
    }
    build_native_config(&request, &SingBoxOptions::default()).map(|_| ())
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn validate_native_config_script(_request: ConnectionRequest) -> Result<(), ApiError> {
    Err(ApiError::new("浏览器端不能直接执行配置脚本"))
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn preview_native_proxy_groups(
    request: ConnectionRequest,
) -> Result<Vec<ProxyGroup>, ApiError> {
    let config = build_native_config(&request, &SingBoxOptions::default())?;
    Ok(proxy_core::extract_proxy_groups(&config))
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn select_native_proxy_group(group: String, outbound: String) -> Result<(), ApiError> {
    if group.trim().is_empty() || outbound.trim().is_empty() {
        return Err(ApiError::new("代理组和节点不能为空"));
    }
    // B12：短操作不排队到长事务后面，锁忙时直接给中文提示。
    let Some(guard) = try_lock_core_slot()? else {
        return Err(ApiError::new("内核正在切换，请稍后重试"));
    };
    let core = guard
        .as_ref()
        .ok_or_else(|| ApiError::new("sing-box 尚未启动"))?;
    core.engine
        .select_outbound(&group, &outbound)
        .map_err(|error| ApiError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) fn select_native_proxy_group(group: String, outbound: String) -> Result<(), ApiError> {
    if group.trim().is_empty() || outbound.trim().is_empty() {
        return Err(ApiError::new("代理组和节点不能为空"));
    }
    singbox::android::select_outbound(&group, &outbound)
        .map_err(|error| ApiError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn select_native_proxy_group(_group: String, _outbound: String) -> Result<(), ApiError> {
    Err(ApiError::new("浏览器端不能直接切换代理组"))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn preview_native_proxy_groups(
    _request: ConnectionRequest,
) -> Result<Vec<ProxyGroup>, ApiError> {
    Err(ApiError::new("浏览器端不能直接生成代理组"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn build_native_config(
    request: &ConnectionRequest,
    options: &SingBoxOptions,
) -> Result<serde_json::Value, ApiError> {
    let mut config = proxy_core::build_singbox_config(request, options);
    if let Some(script) = request.config_script.as_deref() {
        let node_names = request
            .nodes
            .iter()
            .map(|node| {
                (
                    node.tag.clone(),
                    serde_json::Value::String(node.name.clone()),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        config["__kitty_context"] = serde_json::json!({ "node_names": node_names });
        config = proxy_core::apply_config_script(script, config)
            .map_err(|error| ApiError::new(error.to_string()))?;
        if let Some(config) = config.as_object_mut() {
            config.remove("__kitty_context");
        }
    }
    apply_remote_rule_set_download_detour(&mut config);
    apply_owned_cache_config(&mut config, request, options);
    proxy_core::apply_proxy_group_selections(&mut config, &request.group_selections);
    #[cfg(target_os = "macos")]
    macos_route::pin_non_default_outbound_sources(&mut config)
        .map_err(|error| ApiError::new(format!("TUN 路由预检失败: {error}")))?;
    Ok(config)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn apply_remote_rule_set_download_detour(config: &mut serde_json::Value) {
    const DOWNLOAD_OUTBOUND: &str = "__kitty-rule-set-download";

    let Some(rule_sets) = config["route"]["rule_set"].as_array_mut() else {
        return;
    };
    let needs_download_outbound = rule_sets
        .iter()
        .any(|rule_set| rule_set["type"] == "remote" && rule_set["download_detour"] == "direct");
    if !needs_download_outbound {
        return;
    }
    for rule_set in rule_sets {
        if rule_set["type"] == "remote" && rule_set["download_detour"] == "direct" {
            rule_set["download_detour"] = serde_json::Value::String(DOWNLOAD_OUTBOUND.to_string());
        }
    }
    let Some(outbounds) = config["outbounds"].as_array_mut() else {
        return;
    };
    if !outbounds
        .iter()
        .any(|outbound| outbound["tag"] == DOWNLOAD_OUTBOUND)
    {
        outbounds.push(serde_json::json!({
            "type": "direct",
            "tag": DOWNLOAD_OUTBOUND,
            "domain_resolver": "dns-local",
        }));
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn apply_owned_cache_config(
    config: &mut serde_json::Value,
    request: &ConnectionRequest,
    options: &SingBoxOptions,
) {
    let Some(path) = options.cache_file.as_deref() else {
        return;
    };
    let Some(config) = config.as_object_mut() else {
        return;
    };
    let experimental = config
        .entry("experimental")
        .or_insert_with(|| serde_json::json!({}));
    if !experimental.is_object() {
        *experimental = serde_json::json!({});
    }
    experimental["cache_file"] = serde_json::json!({
        "enabled": true,
        "path": path,
        "store_fakeip": request.tun && request.mode != TunnelMode::Direct,
    });
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn native_core_status() -> Result<ApiCoreStatus, ApiError> {
    Ok(ApiCoreStatus {
        state: "unavailable".to_string(),
        version: None,
        note: Some("浏览器目标不能直接运行嵌入式 sing-box 内核".to_string()),
    })
}

#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn mixed_listen_address(allow_lan: bool) -> &'static str {
    if allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    }
}

// B12（保守修复）：状态/流量/日志轮询（native_core_status / native_core_traffic /
// native_core_logs）此前已改用 try_lock_core_slot + 快照降级；本轮再把
// select_native_proxy_group / native_set_core_log_collection / ensure_core_is_running
// 三处阻塞 lock() 收敛为 try_lock_core_slot，锁忙时返回「内核正在切换，请稍后重试」。
// 已知限制：toggle_native_core / restart_native_core / prepare_native_tun_mode /
// start_config_with_bind_retry 等写路径仍持锁跨最长 45s~120s 的 bind 重试 / TUN
// 授权等网络与特权调用，那套事务回滚测试很密，本轮不重构（B12 未完全解决）。
#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn toggle_native_core(
    enabled: bool,
    request: Option<ConnectionRequest>,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ApiError> {
    use proxy_core::SingBoxOptions;
    use singbox::SingBox;

    if !enabled {
        // Never leave the OS pointing at a loopback listener that is about to
        // disappear. If disabling the proxy fails, keep the core alive.
        disable_managed_system_proxy_before_core_outage().map_err(ApiError::new)?;
        let mut guard = core_slot()
            .lock()
            .map_err(|_| ApiError::new("sing-box 状态锁已损坏"))?;
        if let Some(core) = guard.as_mut() {
            if core
                .engine
                .is_running()
                .map_err(|error| ApiError::new(error.to_string()))?
            {
                if let Err(stop_error) = core.engine.stop() {
                    let state = match core.engine.is_running() {
                        Ok(false) => "复查确认内核已停止".to_string(),
                        Ok(true) => "复查发现内核仍在运行".to_string(),
                        Err(status_error) => {
                            format!("无法确认内核状态，复查失败: {status_error}")
                        }
                    };
                    core.engine.force_shutdown();
                    core.config = None;
                    return Err(ApiError::new(format!(
                        "停止 sing-box 内核返回错误，{state}；已强制清理内核: {stop_error}"
                    )));
                }
            }
            core.engine.discard_prepared_config();
            core.config = None;
        }
        drop(guard);
        return native_core_status();
    }

    let request = request.ok_or_else(|| ApiError::new("缺少连接配置"))?;
    if request.nodes.is_empty() {
        return Err(ApiError::new("请先导入并选择一个节点"));
    }
    let options = SingBoxOptions {
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
        ..SingBoxOptions::default()
    };
    let config = build_native_config(&request, &options)?;
    singbox::check_config(&config)
        .map_err(|error| ApiError::new(redact_secrets(&error.to_string())))?;

    let mut guard = core_slot()
        .lock()
        .map_err(|_| ApiError::new("sing-box 状态锁已损坏"))?;
    if guard.is_none() {
        *guard = Some(
            SingBox::discover()
                .map(NativeCore::new)
                .map_err(|error| ApiError::new(error.to_string()))?,
        );
    }
    let core = guard.as_mut().expect("sing-box core was just initialized");
    if core
        .engine
        .is_running()
        .map_err(|error| ApiError::new(error.to_string()))?
    {
        drop(guard);
        return native_core_status();
    }
    core.config = None;
    // The bind retry below can hold the slot lock for up to 45s; publish the
    // current status snapshot first so concurrent polls keep reporting it.
    refresh_core_status_snapshot(core);
    if let Err(error) = start_config_with_bind_retry(&mut core.engine, &config) {
        return Err(ApiError::new(error));
    }
    core.config = Some(config);
    drop(guard);
    native_core_status()
}

/// Start the core, retrying while the mixed-in port is still held by
/// TIME_WAIT sockets from the previous core.
///
/// On macOS, connections accepted by the Go runtime do not carry
/// SO_REUSEADDR, and their TIME_WAIT state keeps blocking a cross-process
/// rebind of the same port for 30 seconds (2 * MSL). With the system proxy
/// enabled this happens on every restart: apps hold active connections to
/// 127.0.0.1:7890, the stopped core closes them (server-side close => TIME_WAIT),
/// and the replacement core immediately fails to bind. Retrying over the
/// TIME_WAIT window makes toggling TUN / switching configs succeed instead of
/// erroring with "bind: address already in use".
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn start_config_with_bind_retry(
    engine: &mut singbox::SingBox,
    config: &serde_json::Value,
) -> Result<(), String> {
    // 45 attempts * 1s covers the macOS 30s TIME_WAIT window with margin.
    const MAX_ATTEMPTS: u32 = 45;
    for attempt in 0..MAX_ATTEMPTS {
        match engine.start_config(config) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let message = error.to_string();
                if !message.contains("address already in use") || attempt + 1 >= MAX_ATTEMPTS {
                    return Err(message);
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
    unreachable!("retry loop always returns")
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) trait RestartCoreEngine {
    fn restart_is_running(&self) -> Result<bool, String>;
    fn restart_stop(&mut self) -> Result<(), String>;
    fn restart_start(&mut self, config: &serde_json::Value) -> Result<(), String>;
    fn restart_force_shutdown(&mut self);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
impl RestartCoreEngine for singbox::SingBox {
    fn restart_is_running(&self) -> Result<bool, String> {
        self.is_running().map_err(|error| error.to_string())
    }

    fn restart_stop(&mut self) -> Result<(), String> {
        self.stop().map_err(|error| error.to_string())
    }

    fn restart_start(&mut self, config: &serde_json::Value) -> Result<(), String> {
        start_config_with_bind_retry(self, config)
    }

    fn restart_force_shutdown(&mut self) {
        self.force_shutdown();
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CoreRestartOutcome {
    CandidateRunning,
    PreviousPreserved { error: String },
    PreviousRestored { error: String },
    CoreOffline { error: String },
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) enum CoreStartObservation {
    Running,
    Stopped { error: String },
    Unknown { error: String },
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn start_core_and_observe<E: RestartCoreEngine>(
    engine: &mut E,
    config: &serde_json::Value,
) -> CoreStartObservation {
    match engine.restart_start(config) {
        Ok(()) => CoreStartObservation::Running,
        Err(start_error) => match engine.restart_is_running() {
            Ok(true) => CoreStartObservation::Running,
            Ok(false) => CoreStartObservation::Stopped { error: start_error },
            Err(status_error) => CoreStartObservation::Unknown {
                error: format!("{start_error}；复查运行状态失败: {status_error}"),
            },
        },
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn restore_previous_core<E: RestartCoreEngine>(
    engine: &mut E,
    previous: Option<&serde_json::Value>,
    cause: String,
) -> CoreRestartOutcome {
    let Some(previous) = previous else {
        return CoreRestartOutcome::CoreOffline {
            error: format!("{cause}；没有可回滚的旧配置"),
        };
    };
    match start_core_and_observe(engine, previous) {
        CoreStartObservation::Running => CoreRestartOutcome::PreviousRestored {
            error: format!("{cause}；已恢复旧配置"),
        },
        CoreStartObservation::Stopped {
            error: rollback_error,
        } => CoreRestartOutcome::CoreOffline {
            error: format!("{cause}；恢复旧配置失败: {rollback_error}"),
        },
        CoreStartObservation::Unknown {
            error: rollback_error,
        } => {
            engine.restart_force_shutdown();
            CoreRestartOutcome::CoreOffline {
                error: format!(
                    "{cause}；恢复旧配置后无法确认内核状态，已强制关闭内核: {rollback_error}"
                ),
            }
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn restart_core_transaction<E, C>(
    engine: &mut E,
    candidate: &serde_json::Value,
    previous: Option<&serde_json::Value>,
    check_candidate: C,
) -> CoreRestartOutcome
where
    E: RestartCoreEngine,
    C: FnOnce(&serde_json::Value) -> Result<(), String>,
{
    if let Err(error) = check_candidate(candidate) {
        return CoreRestartOutcome::PreviousPreserved {
            error: format!("新配置静态校验失败，内核未重启: {error}"),
        };
    }

    let was_running = match engine.restart_is_running() {
        Ok(running) => running,
        Err(error) => {
            engine.restart_force_shutdown();
            return CoreRestartOutcome::CoreOffline {
                error: format!("读取 sing-box 运行状态失败，已强制关闭无法确认状态的内核: {error}"),
            };
        }
    };
    if was_running {
        if let Err(stop_error) = engine.restart_stop() {
            // Some backends report a shutdown error after they have already
            // released their runtime handle. Re-check before claiming that
            // the previous core was preserved.
            match engine.restart_is_running() {
                Ok(true) => {
                    return CoreRestartOutcome::PreviousPreserved {
                        error: format!(
                            "停止旧 sing-box 内核失败，但旧内核仍在运行，配置未切换: {stop_error}"
                        ),
                    };
                }
                Ok(false) => {
                    // The stop reported an error but the core is confirmed
                    // stopped, so the shutdown actually completed. Apply the
                    // candidate instead of resurrecting the previous config:
                    // turning TUN off must not bounce back to the TUN config
                    // just because the old core reported a noisy stop.
                    return match start_core_and_observe(engine, candidate) {
                        CoreStartObservation::Running => CoreRestartOutcome::CandidateRunning,
                        CoreStartObservation::Stopped {
                            error: candidate_error,
                        } => restore_previous_core(
                            engine,
                            previous,
                            format!(
                                "停止旧 sing-box 内核返回错误且内核已停止: {stop_error}；新配置启动失败: {candidate_error}"
                            ),
                        ),
                        CoreStartObservation::Unknown {
                            error: candidate_error,
                        } => {
                            engine.restart_force_shutdown();
                            CoreRestartOutcome::CoreOffline {
                                error: format!(
                                    "停止旧 sing-box 内核返回错误且内核已停止: {stop_error}；新配置启动后无法确认内核状态，已强制关闭内核: {candidate_error}"
                                ),
                            }
                        }
                    };
                }
                Err(status_error) => {
                    engine.restart_force_shutdown();
                    return CoreRestartOutcome::CoreOffline {
                        error: format!(
                            "停止旧 sing-box 内核失败且无法确认状态，已强制关闭内核: {stop_error}；复查状态失败: {status_error}"
                        ),
                    };
                }
            }
        }
    }

    match start_core_and_observe(engine, candidate) {
        CoreStartObservation::Running => CoreRestartOutcome::CandidateRunning,
        CoreStartObservation::Stopped {
            error: candidate_error,
        } => restore_previous_core(
            engine,
            previous,
            format!("新配置启动失败: {candidate_error}"),
        ),
        CoreStartObservation::Unknown {
            error: candidate_error,
        } => {
            engine.restart_force_shutdown();
            CoreRestartOutcome::CoreOffline {
                error: format!("新配置启动后无法确认内核状态，已强制关闭内核: {candidate_error}"),
            }
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn core_config_after_restart(
    outcome: &CoreRestartOutcome,
    candidate: serde_json::Value,
    previous: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match outcome {
        CoreRestartOutcome::CandidateRunning => Some(candidate),
        CoreRestartOutcome::PreviousPreserved { .. }
        | CoreRestartOutcome::PreviousRestored { .. } => previous,
        CoreRestartOutcome::CoreOffline { .. } => None,
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn finish_core_restart<F>(
    outcome: CoreRestartOutcome,
    disable_managed_system_proxy: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    match outcome {
        CoreRestartOutcome::CandidateRunning => Ok(()),
        CoreRestartOutcome::PreviousPreserved { error }
        | CoreRestartOutcome::PreviousRestored { error } => Err(error),
        CoreRestartOutcome::CoreOffline { error } => match disable_managed_system_proxy() {
            Ok(()) => Err(error),
            Err(disable_error) => Err(format!(
                "{error}；关闭 Kitty Pro 管理的系统代理失败: {disable_error}"
            )),
        },
    }
}

/// Disable the system proxy when this app manages one, ahead of a core that is about to
/// stop. Takes only the proxy operation lock, never the core slot.
///
/// Public because the desktop launchers' teardown is not its only caller: a packaged app's
/// quit listener calls it too (`process::register_exit_cleanup` in the app crate, which
/// this one cannot name), and it must not do the rest of [`shutdown_native_runtime`]. That
/// takes `core_slot`, which the restart transaction holds across the bind retry window and
/// the helper authorization wait; a quit listener that blocked on it would block the engine
/// thread, and NSH joins that thread with no deadline once its 200ms quit budget is spent,
/// so quitting mid-restart would make the app wait out a transaction the cleanup never
/// needed. Stopping the embedded core is what process exit does by itself anyway, so the
/// proxy is the only part that has to be cleared before it.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn disable_managed_system_proxy_before_core_outage() -> Result<(), String> {
    let status = match native_system_proxy_status() {
        Ok(status) => status,
        // A transient read failure must not block shutting the core down.
        // The next core start re-applies the managed proxy anyway, so the
        // only cost of skipping cleanup here is a briefly dangling setting.
        Err(_) => return Ok(()),
    };
    if !status.enabled {
        return Ok(());
    }
    set_native_system_proxy(false)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(target_os = "android"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
pub(crate) fn disable_managed_system_proxy_before_core_outage() -> Result<(), String> {
    Ok(())
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn restart_native_core(
    request: ConnectionRequest,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ApiError> {
    use proxy_core::SingBoxOptions;
    use singbox::SingBox;

    let options = SingBoxOptions {
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
        ..SingBoxOptions::default()
    };
    let candidate = build_native_config(&request, &options)?;

    let mut guard = core_slot()
        .lock()
        .map_err(|_| ApiError::new("sing-box 状态锁已损坏"))?;
    if guard.is_none() {
        *guard = Some(
            SingBox::discover()
                .map(NativeCore::new)
                .map_err(|error| ApiError::new(error.to_string()))?,
        );
    }
    let core = guard.as_mut().expect("sing-box core was just initialized");
    let previous = core.config.clone();
    // The restart transaction can keep the slot lock through the bind retry
    // window and helper authorization wait; publish a fresh snapshot first.
    refresh_core_status_snapshot(core);
    core.engine
        .prepare_config(&candidate)
        .map_err(|error| ApiError::new(format!("新配置预检失败，内核未重启: {error}")))?;
    let restart_result =
        restart_core_transaction(&mut core.engine, &candidate, previous.as_ref(), |_| Ok(()));
    core.engine.discard_prepared_config();
    core.config = core_config_after_restart(&restart_result, candidate, previous);
    drop(guard);

    finish_core_restart(
        restart_result,
        disable_managed_system_proxy_before_core_outage,
    )
    .map_err(ApiError::new)?;
    native_core_status()
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) fn restart_native_core(
    request: ConnectionRequest,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ApiError> {
    // Starting the active VpnService again makes it stop the previous core,
    // establish a fresh VPN interface, and apply the replacement config.
    toggle_native_core(true, Some(request), rule_set_cache)
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn restart_native_core(
    _request: ConnectionRequest,
    _rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ApiError> {
    Err(ApiError::new("浏览器目标不能直接运行嵌入式 sing-box 内核"))
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn prepare_native_tun_mode(
    request: ConnectionRequest,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<(), ApiError> {
    use proxy_core::SingBoxOptions;
    use singbox::SingBox;

    let options = SingBoxOptions {
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
        ..SingBoxOptions::default()
    };
    let candidate = build_native_config(&request, &options)?;
    let mut guard = core_slot()
        .lock()
        .map_err(|_| ApiError::new("sing-box 状态锁已损坏"))?;
    if guard.is_none() {
        *guard = Some(
            SingBox::discover()
                .map(NativeCore::new)
                .map_err(|error| ApiError::new(error.to_string()))?,
        );
    }
    let core = guard.as_mut().expect("sing-box core was just initialized");
    if core
        .engine
        .is_running()
        .map_err(|error| ApiError::new(error.to_string()))?
    {
        return Err(ApiError::new(
            "sing-box 已在运行，请通过核心重启应用 TUN 设置",
        ));
    }
    // prepare_config may wait for TUN helper authorization (up to 120s) while
    // holding the slot lock; publish a snapshot so polls keep answering.
    refresh_core_status_snapshot(core);
    core.engine
        .prepare_config(&candidate)
        .map_err(|error| ApiError::new(format!("TUN 配置或权限预检失败: {error}")))
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn release_native_tun_mode() -> Result<(), ApiError> {
    let mut guard = core_slot()
        .lock()
        .map_err(|_| ApiError::new("sing-box 状态锁已损坏"))?;
    if let Some(core) = guard.as_mut() {
        if core
            .engine
            .is_running()
            .map_err(|error| ApiError::new(error.to_string()))?
        {
            return Err(ApiError::new(
                "sing-box 正在运行，不能单独释放当前 TUN helper",
            ));
        }
        core.engine.discard_prepared_config();
    }
    Ok(())
}

#[allow(dead_code)]
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub(crate) fn prepare_native_tun_mode(
    _request: ConnectionRequest,
    _rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<(), ApiError> {
    Err(ApiError::new("当前平台不使用桌面 TUN 权限预检"))
}

#[allow(dead_code)]
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub(crate) fn release_native_tun_mode() -> Result<(), ApiError> {
    Ok(())
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) fn toggle_native_core(
    enabled: bool,
    request: Option<ConnectionRequest>,
    rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ApiError> {
    if !enabled {
        singbox::android::stop().map_err(|error| ApiError::new(error.to_string()))?;
        return native_core_status();
    }

    let mut request = request.ok_or_else(|| ApiError::new("缺少连接配置"))?;
    if request.nodes.is_empty() {
        return Err(ApiError::new("请先导入并选择一个节点"));
    }
    // Android always owns the TUN through VpnService. A loopback mixed proxy
    // alone would not route device traffic.
    request.tun = true;
    proxy_core::validate_custom_rules(&request.custom_rules)
        .map_err(|error| ApiError::new(format!("自定义规则无效: {error}")))?;
    let options = SingBoxOptions {
        listen: mixed_listen_address(request.allow_lan).to_string(),
        traffic_api_port: Some(allocate_loopback_port()?),
        traffic_api_secret: Some(generate_traffic_api_secret()?),
        rule_set_cache,
        cache_file: Some(prepare_native_cache_file()?),
        ..SingBoxOptions::default()
    };
    let mut config = build_native_config(&request, &options)?;
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    let config = serde_json::to_string(&config)
        .map_err(|error| ApiError::new(format!("序列化 Android VPN 配置失败: {error}")))?;
    singbox::android::start(&config).map_err(|error| ApiError::new(error.to_string()))?;
    native_core_status()
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn native_core_traffic() -> Result<CoreTraffic, ApiError> {
    // Frequent polling: never queue behind a long start/restart transaction.
    let Some(guard) = try_lock_core_slot()? else {
        return Ok(CoreTraffic::default());
    };
    let Some(core) = guard.as_ref() else {
        return Ok(CoreTraffic::default());
    };
    if !core
        .engine
        .is_running()
        .map_err(|error| ApiError::new(error.to_string()))?
    {
        return Ok(CoreTraffic::default());
    }
    let traffic = core
        .engine
        .traffic()
        .map_err(|error| ApiError::new(error.to_string()))?;
    Ok(CoreTraffic {
        upload_total: traffic.upload_total,
        download_total: traffic.download_total,
        active_connections: traffic.active_connections,
    })
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) fn native_core_traffic() -> Result<CoreTraffic, ApiError> {
    let traffic = singbox::android::traffic().map_err(|error| ApiError::new(error.to_string()))?;
    Ok(CoreTraffic {
        upload_total: traffic.upload_total,
        download_total: traffic.download_total,
        active_connections: traffic.active_connections,
    })
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn native_core_traffic() -> Result<CoreTraffic, ApiError> {
    Ok(CoreTraffic::default())
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn native_core_logs(cursor: u64) -> Result<CoreLogBatch, ApiError> {
    // Frequent polling: never queue behind a long start/restart transaction.
    let Some(guard) = try_lock_core_slot()? else {
        return Ok(CoreLogBatch {
            next_cursor: cursor,
            entries: Vec::new(),
        });
    };
    let Some(core) = guard.as_ref() else {
        return Ok(CoreLogBatch {
            next_cursor: cursor,
            entries: Vec::new(),
        });
    };
    if !core
        .engine
        .is_running()
        .map_err(|error| ApiError::new(error.to_string()))?
    {
        return Ok(CoreLogBatch {
            next_cursor: cursor,
            entries: Vec::new(),
        });
    }
    core.engine
        .logs(cursor)
        .map(normalize_log_batch)
        .map_err(|error| ApiError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) fn native_core_logs(cursor: u64) -> Result<CoreLogBatch, ApiError> {
    singbox::android::logs(cursor)
        .map(normalize_log_batch)
        .map_err(|error| ApiError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn native_core_logs(cursor: u64) -> Result<CoreLogBatch, ApiError> {
    Ok(CoreLogBatch {
        next_cursor: cursor,
        entries: Vec::new(),
    })
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn native_set_core_log_collection(enabled: bool) -> Result<(), ApiError> {
    // B12：短操作不排队到长事务后面，锁忙时直接给中文提示。
    let Some(guard) = try_lock_core_slot()? else {
        return Err(ApiError::new("内核正在切换，请稍后重试"));
    };
    let Some(core) = guard.as_ref() else {
        return Ok(());
    };
    if !core
        .engine
        .is_running()
        .map_err(|error| ApiError::new(error.to_string()))?
    {
        return Ok(());
    }
    core.engine
        .set_log_enabled(enabled)
        .map_err(|error| ApiError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) fn native_set_core_log_collection(enabled: bool) -> Result<(), ApiError> {
    singbox::android::set_log_enabled(enabled).map_err(|error| ApiError::new(error.to_string()))
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn native_set_core_log_collection(_enabled: bool) -> Result<(), ApiError> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn normalize_log_batch(batch: singbox::LogBatch) -> CoreLogBatch {
    CoreLogBatch {
        next_cursor: batch.next_cursor,
        entries: batch
            .entries
            .into_iter()
            .map(|entry| CoreLogEntry {
                sequence: entry.sequence,
                timestamp: entry.timestamp,
                level: entry.level,
                route: parse_route_log(&entry.message, entry.outbound_chain, entry.source_ip),
                message: entry.message,
            })
            .collect(),
    }
}

pub(crate) fn parse_route_log(
    message: &str,
    outbound_chain: Vec<String>,
    source_ip: Option<String>,
) -> Option<RouteLogDetail> {
    let component = message.split_once("outbound/")?.1;
    let type_end = component.find('[')?;
    let outbound_type = component[..type_end].trim();
    let tag_start = type_end + 1;
    let tag_end = component[tag_start..].find(']')? + tag_start;
    let outbound_tag = component[tag_start..tag_end].trim();
    let detail = component[tag_end + 1..].trim_start_matches(':').trim();
    let target_start = detail.find("connection to ")? + "connection to ".len();
    let target = detail[target_start..].trim();
    if target.is_empty() {
        return None;
    }

    let (host, port) = split_route_target(target);
    let target_kind = if host.parse::<std::net::IpAddr>().is_ok() {
        RouteTargetKind::Ip
    } else {
        RouteTargetKind::Domain
    };
    Some(RouteLogDetail {
        decision: if outbound_type == "direct" || outbound_tag == "direct" {
            RouteDecision::Direct
        } else if outbound_type == "block" || outbound_tag == "block" {
            RouteDecision::Block
        } else {
            RouteDecision::Proxy
        },
        target: target.to_string(),
        host,
        port,
        target_kind,
        outbound_type: outbound_type.to_string(),
        outbound_tag: outbound_tag.to_string(),
        outbound_chain,
        source_ip,
    })
}

pub(crate) fn split_route_target(target: &str) -> (String, Option<u16>) {
    if let Some(ipv6) = target.strip_prefix('[') {
        if let Some((host, port)) = ipv6.split_once("]:") {
            return (host.to_string(), port.parse().ok());
        }
    }
    if let Some((host, port)) = target.rsplit_once(':') {
        if !host.is_empty() {
            if let Ok(port) = port.parse() {
                return (host.to_string(), Some(port));
            }
        }
    }
    (target.to_string(), None)
}

// Q5 预留端口 TOCTOU（本轮仅评估，不重构）：本函数 bind(127.0.0.1:0) 取到
// 空闲端口后，listener 在函数返回时即被 drop（RAII 释放端口），端口要等到之后
// 内核 / 特权进程真正 bind 才被占用——这中间确实存在「先分配→释放→再绑定」的
// 窗口。同型问题也出现在 singbox/src/desktop_helper.rs 的 reserve_loopback_port
// （helper RPC 端口）：launch() 先取 rpc_port 并 drop，随后提权 helper 才 bind。
//
// 窗口内本机进程能否抢占并骗取 token：能。loopback 端口释放后任何本机进程都可
// bind；helper 路径还要经过 pkexec/polkit 授权等待，窗口可达秒级，且 rpc_port
// 会以命令行参数出现、可被本机进程从 ps 或 /proc 命令行读到。抢占成功后：
//   (a) 真 helper bind 失败 → launch 失败，是可致拒绝服务；
//   (b) 更窄的竞态下，真 helper 已先连上父进程「一直持有」的租约端口 L 并写入
//       token（租约因此建立），随后才 bind P 失败退出；抢占者可应答 is_ready()
//       的 Ping，进而收到父进程 request() 发来的 HelperRequest（含 token）以及
//       后续 Start{config}（含节点凭据）。
//
// 现有缓解及为何不足以覆盖该窗口：token 文件 0600 + metadata.uid()==owner_id +
// owner_id 参数校验，保护的是「token 静态存储」与「真 helper 读文件」；租约端口
// L 由父进程持有、accept 时校验 peer 为 loopback 且 token 匹配；每个 RPC 请求用
// constant_time_eq 校验 token。这些能保证攻击者读不到 token 文件、也无法冒充真
// helper 通过租约/请求校验，但 RPC 连接本身没有 SO_PEERCRED / pid/uid 之类的对端
// 身份校验——父进程只 TcpStream::connect(rpc_port) 就把 token 与 config 发过去，
// 无法确认监听者就是提权后的 helper。故上述缓解不足以完全覆盖端口抢占向量。
//
// 彻底修复（不在本轮做，涉及特权进程启动时序、风险高）：把已绑定的 listener
// 一直持有到 RPC 建立——父进程 bind 后不 drop，把 listener/fd 交给 helper，或改
// 用 Unix domain socket 并配合 SO_PEERCRED 校验对端 uid/pid，消除「释放→重绑」
// 窗口。本轮仅评估、记录结论，不改动实现。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn allocate_loopback_port() -> Result<u16, ApiError> {
    use std::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| ApiError::new(format!("分配流量统计端口失败: {error}")))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| ApiError::new(format!("读取流量统计端口失败: {error}")))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn generate_traffic_api_secret() -> Result<String, ApiError> {
    use std::fmt::Write;

    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes)
        .map_err(|error| ApiError::new(format!("生成流量统计令牌失败: {error}")))?;
    let mut secret = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(secret, "{byte:02x}");
    }
    Ok(secret)
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) fn toggle_native_core(
    _enabled: bool,
    _request: Option<ConnectionRequest>,
    _rule_set_cache: Option<RuleSetCachePaths>,
) -> Result<ApiCoreStatus, ApiError> {
    Err(ApiError::new("浏览器目标不能直接运行嵌入式 sing-box 内核"))
}
