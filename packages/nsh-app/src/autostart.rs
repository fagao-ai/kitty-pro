//! Start-at-login, through NSH's own login item.
//!
//! NSH 0.6 gives a standalone desktop app `launch_at_login` and `set_launch_at_login`:
//! macOS's `SMAppService.mainApp`, the current user's `Run` entry on Windows and an XDG
//! autostart entry on Linux. The runner owns where that entry lives, so this app no longer
//! writes one itself, and the system's settings page and the app agree on the same item.
//!
//! Older builds of Kitty Pro registered the login item themselves under
//! `com.kitty-pro.autostart` (a macOS LaunchAgent, a Windows `Run` value, an XDG
//! `.desktop` file). Upgrading without removing those would start two copies of the app
//! at login, so `migrate_legacy` hands the old registration over: it turns the NSH entry
//! on first, and only removes the old one once the system reports the new one on. A
//! registration waiting for the user's approval on macOS keeps the old entry until they
//! approve, rather than leaving them with no autostart at all.

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", test))]
use nsh_core::desktop::LaunchAtLoginError;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", test))]
use nsh_core::element::desktop::{launch_at_login, set_launch_at_login};

/// Whether this build can offer a login item at all, so the row is absent rather than dead.
pub const SUPPORTED: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));

/// The label older builds wrote. Kept only to find and remove what they left behind.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", test))]
const LEGACY_ID: &str = "com.kitty-pro.autostart";

/// What to do about a login item an older build registered.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Migration {
    /// No old registration is left.
    Nothing,
    /// The old registration is on and NSH's is off: turn NSH's on, then remove the old one.
    TakeOver,
    /// NSH's registration already covers the app, or the old one was off: remove the old one.
    Remove,
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", test))]
fn plan(legacy: Option<bool>, current: bool) -> Migration {
    match legacy {
        None => Migration::Nothing,
        Some(true) if !current => Migration::TakeOver,
        Some(_) => Migration::Remove,
    }
}

/// Whether the app will open at login, counting an old registration that still serves while
/// a new one waits for approval.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", test))]
fn effective(current: bool, legacy: Option<bool>) -> bool {
    current || legacy == Some(true)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub async fn status() -> Result<bool, String> {
    migrate_legacy().await?;
    Ok(effective(launch_at_login().await, legacy_state()?))
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub async fn set_enabled(enabled: bool) -> Result<bool, String> {
    match set_launch_at_login(enabled).await {
        Ok(()) => {}
        // A development run has no bundle to register, but it can still remove an entry
        // an older build wrote. Turning the switch off should not be stuck behind that.
        Err(LaunchAtLoginError::Unavailable) if !enabled => {
            remove_legacy()?;
            return Ok(false);
        }
        Err(error) => return Err(explain(error)),
    }
    let actual = launch_at_login().await;
    if enabled && !actual {
        return Err(crate::i18n::tr(
            "系统已登记登录项，但仍需你在系统设置的“登录项”中允许后才会生效",
        ));
    }
    // The press is the user's decision, so the old entry goes as soon as the new one is
    // the one the system reports. On the approval path above it stays, so the app still
    // launches at login while the user decides.
    remove_legacy()?;
    Ok(actual)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
async fn migrate_legacy() -> Result<(), String> {
    let legacy = legacy_state()?;
    let current = launch_at_login().await;
    match plan(legacy, current) {
        Migration::Nothing => Ok(()),
        Migration::Remove => remove_legacy(),
        Migration::TakeOver => {
            match set_launch_at_login(true).await {
                Ok(()) => {}
                // `nsh run` and an embedded view have no login item to hand over. The old
                // registration is still the one launching the app, so it stays until a
                // packaged start can migrate it.
                Err(LaunchAtLoginError::Unavailable) => return Ok(()),
                Err(error) => {
                    return Err(crate::i18n::fill(
                        "迁移登录启动项失败：{}",
                        &[&explain(error)],
                    ))
                }
            }
            if launch_at_login().await {
                remove_legacy()
            } else {
                // The system holds the new registration until the user approves it in
                // System Settings. The old entry keeps launching the app meanwhile, and
                // the next start retries this handover.
                Ok(())
            }
        }
    }
}

/// The words the login-item API's errors reach the screen with.
///
/// `Unavailable` is the normal answer under `nsh run`, where the app has no bundle to
/// register, and the platform's own text for a refusal is English. Both are translated
/// here so the settings row does not turn into a mixed-language sentence.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux", test))]
fn explain(error: LaunchAtLoginError) -> String {
    match error {
        LaunchAtLoginError::Unavailable => {
            crate::i18n::tr("开发运行或嵌入环境不写入登录项，请使用安装后的 Kitty Pro")
        }
        LaunchAtLoginError::Denied => crate::i18n::tr(
            "登录启动项未获授权：请在 nsh.json 的 capabilities 中加入 “application”: true",
        ),
        LaunchAtLoginError::System(error) => {
            crate::i18n::fill("系统拒绝了登录项修改：{}", &[&error])
        }
        // `LaunchAtLoginError` is non-exhaustive: a future variant keeps its own words
        // rather than disappearing behind a catch-all translation.
        _ => error.to_string(),
    }
}

#[cfg(target_os = "macos")]
fn legacy_path() -> Option<std::path::PathBuf> {
    Some(
        std::path::PathBuf::from(std::env::var_os("HOME")?)
            .join("Library/LaunchAgents")
            .join(format!("{LEGACY_ID}.plist")),
    )
}

#[cfg(target_os = "macos")]
fn legacy_state() -> Result<Option<bool>, String> {
    let Some(path) = legacy_path() else {
        return Ok(None);
    };
    if !path.is_file() {
        return Ok(None);
    }
    // The old app treated a LaunchAgent disabled in System Settings as off even though the
    // file stayed. Reading the same override keeps an upgrade from turning autostart back
    // on for someone who had turned it off. If the override cannot be read, the file is
    // the registration the user still has, so it counts as on.
    Ok(Some(!read_launchctl_disabled().unwrap_or(false)))
}

#[cfg(target_os = "macos")]
fn remove_legacy() -> Result<(), String> {
    let Some(path) = legacy_path() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(crate::i18n::fill("清理旧版登录启动项失败：{}", &[&error])),
    }
    // The file is what launches the app at the next login, so removing it is enough. The
    // old agent may still be loaded for this session, and booting it out could terminate
    // the copy of the app that is doing the migration; it will not load again after the
    // file is gone.
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn launchctl_disabled(text: &str) -> Result<bool, String> {
    for line in text.lines() {
        if let Some((label, state)) = line.trim().split_once("=>") {
            if label.trim() == format!("\"{LEGACY_ID}\"") {
                return match state.trim().trim_end_matches(';') {
                    "true" | "disabled" => Ok(true),
                    "false" | "enabled" => Ok(false),
                    _ => Err("launchctl 禁用状态格式无效".into()),
                };
            }
        }
    }
    if text.trim().starts_with("disabled services = {") && text.trim().ends_with('}') {
        Ok(false)
    } else {
        Err("launchctl 输出格式无效".into())
    }
}

#[cfg(target_os = "macos")]
fn read_launchctl_disabled() -> Result<bool, String> {
    let uid = current_uid()?;
    let text = launchctl(&["print-disabled".to_owned(), format!("gui/{uid}")])?;
    launchctl_disabled(&text)
}

#[cfg(target_os = "macos")]
fn current_uid() -> Result<String, String> {
    let output = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("无法读取当前用户 UID".into());
    }
    let uid = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    let uid = uid.trim();
    if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("当前用户 UID 无效".into());
    }
    Ok(uid.to_owned())
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[String]) -> Result<String, String> {
    let output = std::process::Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "launchctl 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn legacy_path() -> Option<std::path::PathBuf> {
    // The old implementation used `$HOME` directly rather than `XDG_CONFIG_HOME`, so this
    // looks in the same place whichever desktop environment is running.
    Some(
        std::path::PathBuf::from(std::env::var_os("HOME")?)
            .join(".config/autostart")
            .join(format!("{LEGACY_ID}.desktop")),
    )
}

#[cfg(target_os = "linux")]
fn legacy_state() -> Result<Option<bool>, String> {
    Ok(legacy_path().filter(|path| path.is_file()).map(|_| true))
}

#[cfg(target_os = "linux")]
fn remove_legacy() -> Result<(), String> {
    let Some(path) = legacy_path() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(crate::i18n::fill("清理旧版登录启动项失败：{}", &[&error])),
    }
}

#[cfg(target_os = "windows")]
const LEGACY_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const LEGACY_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
#[cfg(target_os = "windows")]
const ERROR_FILE_NOT_FOUND: u32 = 0x8007_0002;

#[cfg(target_os = "windows")]
fn legacy_state() -> Result<Option<bool>, String> {
    use windows_registry::CURRENT_USER;
    let registered = match CURRENT_USER
        .open(LEGACY_RUN_KEY)
        .and_then(|key| key.get_string(LEGACY_ID))
    {
        Ok(value) => !value.trim().is_empty(),
        Err(error) if error.code().0 as u32 == ERROR_FILE_NOT_FOUND => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !registered {
        return Ok(Some(false));
    }
    // Task Manager's startup switch lives in StartupApproved: a `Run` value that is still
    // present can have been turned off there. Treating that as on would silently undo the
    // user's choice during the upgrade.
    match CURRENT_USER
        .open(LEGACY_APPROVED_KEY)
        .and_then(|key| key.get_value(LEGACY_ID))
    {
        Ok(value) if value.ty() == windows_registry::Type::Bytes && value.len() == 12 => {
            Ok(Some(matches!(value[0], 2 | 6)))
        }
        Ok(_) => Ok(Some(false)),
        Err(_) => Ok(Some(true)),
    }
}

#[cfg(target_os = "windows")]
fn remove_legacy() -> Result<(), String> {
    use windows_registry::CURRENT_USER;
    for key_path in [LEGACY_RUN_KEY, LEGACY_APPROVED_KEY] {
        match CURRENT_USER
            .options()
            .write()
            .open(key_path)
            .and_then(|key| key.remove_value(LEGACY_ID))
        {
            Ok(()) => {}
            Err(error) if error.code().0 as u32 == ERROR_FILE_NOT_FOUND => {}
            Err(error) => return Err(crate::i18n::fill("清理旧版登录启动项失败：{}", &[&error])),
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub async fn status() -> Result<bool, String> {
    Err("当前平台没有登录启动项".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub async fn set_enabled(_enabled: bool) -> Result<bool, String> {
    Err("当前平台没有登录启动项".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_registration_is_handed_over_before_it_is_removed() {
        assert_eq!(plan(None, false), Migration::Nothing);
        assert_eq!(plan(None, true), Migration::Nothing);
        assert_eq!(plan(Some(true), false), Migration::TakeOver);
        assert_eq!(plan(Some(true), true), Migration::Remove);
        assert_eq!(plan(Some(false), false), Migration::Remove);
        assert_eq!(plan(Some(false), true), Migration::Remove);
    }

    #[test]
    fn an_old_entry_still_counts_while_the_new_one_waits() {
        assert!(effective(true, None));
        assert!(effective(false, Some(true)));
        assert!(!effective(false, Some(false)));
        assert!(!effective(false, None));
    }

    #[test]
    fn macos_disabled_override() {
        // Real output: tab indentation, a quoted label, no semicolon.
        assert!(launchctl_disabled(&format!(
            "disabled services = {{\n\t\t\"{LEGACY_ID}\" => disabled\n}}"
        ))
        .unwrap());
        assert!(!launchctl_disabled(&format!(
            "disabled services = {{\n\t\t\"{LEGACY_ID}\" => enabled\n}}"
        ))
        .unwrap());
        // The older true/false wording is still understood.
        assert!(launchctl_disabled(&format!(
            "disabled services = {{\n\"{LEGACY_ID}\" => true\n}}"
        ))
        .unwrap());
        assert!(!launchctl_disabled(&format!(
            "disabled services = {{\n\"{LEGACY_ID}\" => false\n}}"
        ))
        .unwrap());
        // Other agents in the list do not decide this app's state.
        assert!(!launchctl_disabled(&format!(
            "disabled services = {{\n\
                 \t\t\"com.apple.Siri.agent\" => disabled\n\
                 \t\t\"{LEGACY_ID}\" => enabled\n\
                 \t\t\"homebrew.mxcl.redis\" => disabled\n}}"
        ))
        .unwrap());
        assert!(!launchctl_disabled("disabled services = {\n}").unwrap());
        assert!(launchctl_disabled("unexpected output").is_err());
    }
}
