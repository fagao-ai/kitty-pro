//! System proxy, process list and native runtime shutdown.
//!
//! Split out of `lib.rs`; the crate root re-exports everything through
//! `pub(crate) use ...::*`, so sibling modules keep using `use super::*`.

use super::*;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) const SYSTEM_PROXY_HOST: &str = "127.0.0.1";
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) const SYSTEM_PROXY_PORT: u16 = 7890;

#[cfg(target_os = "windows")]
pub(crate) fn proxy_environment_variables() -> String {
    format!(
        "$env:http_proxy=\"http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}\"; \
         $env:https_proxy=\"http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}\"; \
         $env:all_proxy=\"socks5://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}\""
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn proxy_environment_variables() -> String {
    format!(
        "export http_proxy=http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT} && \
         export https_proxy=http://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT} && \
         export all_proxy=socks5://{SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}"
    )
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn pipe_text_to_command(program: &str, args: &[&str], text: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("启动 {program} 失败: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("{program} 未提供标准输入"))?
        .write_all(text.as_bytes())
        .map_err(|error| format!("写入 {program} 失败: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("等待 {program} 失败: {error}"))?;
    if !status.success() {
        return Err(format!("{program} 返回失败状态 {status}"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    pipe_text_to_command("/usr/bin/pbcopy", &[], &proxy_environment_variables())
        .map_err(ServerFnError::new)
}

#[cfg(target_os = "windows")]
pub(crate) fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    pipe_text_to_command("clip.exe", &[], &proxy_environment_variables())
        .map_err(ServerFnError::new)
}

#[cfg(target_os = "linux")]
pub(crate) fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    let text = proxy_environment_variables();
    let commands: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    let mut errors = Vec::new();
    for (program, args) in commands {
        match pipe_text_to_command(program, args, &text) {
            Ok(()) => return Ok(()),
            Err(error) => errors.push(error),
        }
    }
    Err(ServerFnError::new(format!(
        "未找到可用的系统剪贴板工具: {}",
        errors.join("；")
    )))
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
pub(crate) fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    Err(ServerFnError::new("当前平台尚未实现系统剪贴板适配"))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn native_copy_proxy_environment_variables() -> Result<(), ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接写入系统剪贴板"))
}

#[cfg(target_os = "windows")]
pub(crate) const SYSTEM_PROXY_BYPASS: &str = "localhost;127.*;192.168.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;<local>";
#[cfg(target_os = "linux")]
pub(crate) const SYSTEM_PROXY_BYPASS: &str =
    "localhost,127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,::1";
#[cfg(target_os = "macos")]
pub(crate) const SYSTEM_PROXY_BYPASS: &str =
    "127.0.0.1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,localhost,*.local,<local>";

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn system_proxy_operation_lock(
) -> Result<std::sync::MutexGuard<'static, ()>, ServerFnError> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| ServerFnError::new("系统代理操作锁已损坏"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn read_native_system_proxy() -> Result<sysproxy::Sysproxy, ServerFnError> {
    sysproxy::Sysproxy::get_system_proxy()
        .map_err(|error| ServerFnError::new(format!("读取系统代理失败: {error}")))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn apply_native_system_proxy(proxy: &sysproxy::Sysproxy) -> Result<(), ServerFnError> {
    proxy
        .set_system_proxy()
        .map_err(|error| ServerFnError::new(format!("设置系统代理失败: {error}")))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn apply_native_auto_proxy(proxy: &sysproxy::Autoproxy) -> Result<(), ServerFnError> {
    proxy
        .set_auto_proxy()
        .map_err(|error| ServerFnError::new(format!("设置自动代理失败: {error}")))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn is_kitty_system_proxy(proxy: &sysproxy::Sysproxy) -> bool {
    proxy.enable
        && proxy.host.eq_ignore_ascii_case(SYSTEM_PROXY_HOST)
        && proxy.port == SYSTEM_PROXY_PORT
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn native_system_proxy_status_unlocked() -> Result<SystemProxyStatus, ServerFnError> {
    let proxy = read_native_system_proxy()?;
    let enabled = is_kitty_system_proxy(&proxy);
    let detail = if enabled {
        format!(
            "已设置 {} 系统代理 {SYSTEM_PROXY_HOST}:{SYSTEM_PROXY_PORT}",
            system_proxy_platform_name()
        )
    } else if proxy.enable {
        "系统当前使用其他代理，Kitty Pro 未接管".to_string()
    } else {
        "未启用系统代理".to_string()
    };
    Ok(SystemProxyStatus {
        supported: true,
        enabled,
        detail,
    })
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    let _operation = system_proxy_operation_lock()?;
    native_system_proxy_status_unlocked()
}

#[cfg(target_os = "macos")]
pub(crate) const fn system_proxy_platform_name() -> &'static str {
    "macOS"
}

#[cfg(target_os = "windows")]
pub(crate) const fn system_proxy_platform_name() -> &'static str {
    "Windows"
}

#[cfg(target_os = "linux")]
pub(crate) const fn system_proxy_platform_name() -> &'static str {
    "Linux"
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
pub(crate) fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    Ok(SystemProxyStatus {
        supported: false,
        enabled: false,
        detail: "当前平台尚未实现系统代理适配".to_string(),
    })
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub(crate) fn native_system_proxy_status() -> Result<SystemProxyStatus, ServerFnError> {
    Ok(SystemProxyStatus {
        supported: false,
        enabled: false,
        detail: "浏览器目标不能直接修改系统代理".to_string(),
    })
}

/// Sentinel written while Kitty Pro manages the system proxy. If the process
/// dies (kill -9, crash, power loss) before it can restore the settings, the
/// next startup notices the leftover and clears the now-dangling proxy.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn system_proxy_sentinel_path() -> Result<std::path::PathBuf, ServerFnError> {
    // Follow the profile directory override (see core.rs::profile_path): the
    // overview page renders the sentinel's "system proxy enabled" state, so
    // an isolated test server must not observe the developer machine's live
    // system-proxy flag (it skewed the layout fold budget by 35px once).
    if let Ok(dir) = std::env::var("KITTY_PROFILE_DIR") {
        return Ok(std::path::PathBuf::from(dir).join("system-proxy.pending"));
    }
    let directories = directories::ProjectDirs::from("com", "kitty", "kitty-pro")
        .ok_or_else(|| ServerFnError::new("无法确定本地配置目录"))?;
    Ok(directories.data_local_dir().join("system-proxy.pending"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn write_system_proxy_sentinel() {
    if let Ok(path) = system_proxy_sentinel_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, std::process::id().to_string());
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn remove_system_proxy_sentinel() {
    if let Ok(path) = system_proxy_sentinel_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Clear a system proxy left behind by a previous Kitty Pro run that exited
/// without cleanup. Only touches proxies pointing at our own managed mixed
/// listener while nothing is listening on it; anything owned by another
/// application is left alone. Failures are silent best-effort cleanup.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn cleanup_stale_system_proxy_sentinel() {
    let Ok(path) = system_proxy_sentinel_path() else {
        return;
    };
    if !path.exists() {
        return;
    }
    let Ok(proxy) = read_native_system_proxy() else {
        return;
    };
    if is_kitty_system_proxy(&proxy) {
        let endpoint = std::net::SocketAddr::from(([127, 0, 0, 1], SYSTEM_PROXY_PORT));
        let listener_alive =
            std::net::TcpStream::connect_timeout(&endpoint, std::time::Duration::from_millis(200))
                .is_ok();
        if listener_alive {
            // Another core still serves the endpoint; keep managing it.
            return;
        }
        let proxy = sysproxy::Sysproxy {
            host: SYSTEM_PROXY_HOST.to_string(),
            bypass: SYSTEM_PROXY_BYPASS.to_string(),
            port: SYSTEM_PROXY_PORT,
            enable: false,
        };
        if apply_native_system_proxy(&proxy).is_err() {
            return;
        }
        let _ = apply_native_auto_proxy(&sysproxy::Autoproxy::default());
        eprintln!("[kitty-pro] 已清理上次异常退出残留的系统代理设置");
    }
    let _ = std::fs::remove_file(path);
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn set_native_system_proxy(enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    let _operation = system_proxy_operation_lock()?;

    if enabled {
        ensure_core_is_running()?;
        let proxy = sysproxy::Sysproxy {
            host: SYSTEM_PROXY_HOST.to_string(),
            bypass: SYSTEM_PROXY_BYPASS.to_string(),
            port: SYSTEM_PROXY_PORT,
            enable: true,
        };
        apply_native_auto_proxy(&sysproxy::Autoproxy::default())?;
        apply_native_system_proxy(&proxy)?;
        write_system_proxy_sentinel();
    } else {
        // sysproxy always rewrites the bypass list on macOS, and its default
        // (empty) bypass makes `networksetup -setproxybypassdomains` fail with
        // an argument count error. Reuse the managed bypass list so disabling
        // the proxy stays idempotent instead of erroring out.
        let proxy = sysproxy::Sysproxy {
            host: SYSTEM_PROXY_HOST.to_string(),
            bypass: SYSTEM_PROXY_BYPASS.to_string(),
            port: SYSTEM_PROXY_PORT,
            enable: false,
        };
        apply_native_system_proxy(&proxy)?;
        apply_native_auto_proxy(&sysproxy::Autoproxy::default())?;
        remove_system_proxy_sentinel();
    }

    native_system_proxy_status_unlocked()
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
pub(crate) fn set_native_system_proxy(_enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    Err(ServerFnError::new("当前平台尚未实现系统代理适配"))
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub(crate) fn set_native_system_proxy(_enabled: bool) -> Result<SystemProxyStatus, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接修改系统代理"))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn native_list_running_processes() -> Result<Vec<RunningProcess>, ServerFnError> {
    use std::collections::BTreeMap;

    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .without_tasks(),
    );

    let current_pid = std::process::id();
    let mut by_identity = BTreeMap::new();
    for process in system.processes().values() {
        let pid = process.pid().as_u32();
        if pid == current_pid {
            continue;
        }
        let Some(exe_path) = process.exe() else {
            continue;
        };
        insert_running_process(&mut by_identity, pid, exe_path);
    }

    Ok(by_identity.into_values().collect())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn insert_running_process(
    by_identity: &mut std::collections::BTreeMap<(String, String), RunningProcess>,
    pid: u32,
    exe_path: &std::path::Path,
) {
    let Some(name) = exe_path.file_name() else {
        return;
    };
    let name = name.to_string_lossy().into_owned();
    let exe_path = exe_path.to_string_lossy().into_owned();
    if name.is_empty() || exe_path.is_empty() {
        return;
    }

    let process = RunningProcess {
        pid,
        name: name.clone(),
        exe_path: Some(exe_path.clone()),
    };
    by_identity
        .entry((name, exe_path))
        .and_modify(|existing| {
            if pid < existing.pid {
                existing.pid = pid;
            }
        })
        .or_insert(process);
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
pub(crate) fn native_list_running_processes() -> Result<Vec<RunningProcess>, ServerFnError> {
    Ok(Vec::new())
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub(crate) fn native_list_running_processes() -> Result<Vec<RunningProcess>, ServerFnError> {
    Ok(Vec::new())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub(crate) fn ensure_core_is_running() -> Result<(), ServerFnError> {
    use singbox::CoreState;

    // B12：只读状态检查不排队到长事务后面，锁忙时直接给中文提示。
    let Some(guard) = try_lock_core_slot()? else {
        return Err(ServerFnError::new("内核正在切换，请稍后重试"));
    };
    let Some(core) = guard.as_ref() else {
        return Err(ServerFnError::new("请先建立连接，再启用系统代理"));
    };
    let status = core.engine.status();
    if status.state != CoreState::Running {
        return Err(ServerFnError::new("请先建立连接，再启用系统代理"));
    }
    Ok(())
}

/// Disable any active Kitty Pro system proxy, then stop the embedded core.
/// Desktop launchers call this during normal event-loop teardown so users are
/// not left with a dead loopback proxy after exit.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub fn shutdown_native_runtime() -> Result<(), String> {
    let mut errors = Vec::new();

    if let Err(error) = disable_managed_system_proxy_before_core_outage() {
        errors.push(format!("关闭系统代理失败: {error}"));
    }

    match core_slot().lock() {
        Ok(mut guard) => {
            if let Some(core) = guard.as_mut() {
                match core.engine.is_running() {
                    Ok(true) => match core.engine.stop() {
                        Ok(()) => {
                            core.engine.discard_prepared_config();
                            core.config = None;
                        }
                        Err(error) => {
                            errors.push(format!("停止 sing-box 内核失败: {error}"));
                        }
                    },
                    Ok(false) => {
                        core.engine.discard_prepared_config();
                        core.config = None;
                    }
                    Err(error) => errors.push(format!("读取 sing-box 状态失败: {error}")),
                }
            }
        }
        Err(_) => errors.push("sing-box 状态锁已损坏".to_string()),
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn core_slot() -> &'static std::sync::Mutex<Option<NativeCore>> {
    use std::sync::{Mutex, OnceLock};

    static CORE: OnceLock<Mutex<Option<NativeCore>>> = OnceLock::new();
    CORE.get_or_init(|| Mutex::new(None))
}

/// Lock the core slot unless a start/restart transaction holds it for a long
/// running operation. Returns `None` when the slot is busy.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn try_lock_core_slot(
) -> Result<Option<std::sync::MutexGuard<'static, Option<NativeCore>>>, ServerFnError> {
    match core_slot().try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(std::sync::TryLockError::WouldBlock) => Ok(None),
        Err(std::sync::TryLockError::Poisoned(_)) => {
            Err(ServerFnError::new("sing-box 状态锁已损坏"))
        }
    }
}

/// Last successfully observed core status, refreshed whenever the slot lock
/// is held by polling itself. Used to answer status polls while a long
/// start/restart transaction keeps the slot lock busy.
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn core_status_snapshot() -> &'static std::sync::RwLock<Option<ApiCoreStatus>> {
    use std::sync::{OnceLock, RwLock};

    static SNAPSHOT: OnceLock<RwLock<Option<ApiCoreStatus>>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| RwLock::new(None))
}

/// Refresh the status snapshot just before entering a long transaction that
/// holds the slot lock, so concurrent polls keep reporting the state the UI
/// showed right before the operation began.
#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) fn refresh_core_status_snapshot(core: &NativeCore) {
    let status = api_core_status(core.engine.status());
    if let Ok(mut snapshot) = core_status_snapshot().write() {
        *snapshot = Some(status);
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[derive(Debug)]
pub(crate) struct NativeCore {
    // `core.rs` 驱动内核生命周期，拆分后两个模块需要共享这两个字段；
    // 仍限定在 crate 内，不扩大对外 API 面。
    pub(crate) engine: singbox::SingBox,
    pub(crate) config: Option<serde_json::Value>,
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
impl NativeCore {
    pub(crate) fn new(engine: singbox::SingBox) -> Self {
        Self {
            engine,
            config: None,
        }
    }
}
