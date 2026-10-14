//! 仅原生桌面调用，不提供 HTTP 接口或配置同步。
//!
//! Linux 的注册走 crates.io 的 auto-launch 0.6。Windows 的关闭和查询在该库里会访问
//! HKLM，普通用户无法使用，所以 Windows 只操作当前用户的 Run 项。macOS 用 plist
//! 写当前用户的 LaunchAgent，避免库拼接 XML 时不转义路径。
#[cfg(target_os = "linux")]
use auto_launch::{AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode};
use dioxus::prelude::ServerFnError;
use std::path::{Path, PathBuf};

const ID: &str = "com.kitty-pro.autostart";

pub async fn status() -> Result<bool, ServerFnError> {
    super::run_native_blocking("读取登录自启动失败", || {
        read_enabled(false).map_err(ServerFnError::new)
    })
    .await
}

pub async fn set_enabled(enabled: bool) -> Result<bool, ServerFnError> {
    super::run_native_blocking("设置登录自启动失败", move || {
        if enabled {
            enable().map_err(ServerFnError::new)?;
        } else {
            disable().map_err(ServerFnError::new)?;
        }
        let actual = read_enabled(false).map_err(ServerFnError::new)?;
        if actual != enabled {
            return Err(ServerFnError::new(
                "系统未应用登录自启动设置，请检查当前用户的启动项权限",
            ));
        }
        Ok(actual)
    })
    .await
}

fn enable() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let appimage = if cfg!(target_os = "linux") {
        std::env::var_os("APPIMAGE").map(PathBuf::from)
    } else {
        None
    };
    let path = launch_path(&exe, appimage.as_deref(), cfg!(target_os = "macos"))?;
    if !path.is_file() {
        return Err("启动程序不存在，请安装到固定位置后重试".into());
    }
    #[cfg(not(target_os = "windows"))]
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let path = launch_path(&path, None, cfg!(target_os = "macos"))?;
    let text = path.to_str().ok_or("启动程序路径不是有效的 UTF-8")?;
    #[cfg(target_os = "linux")]
    {
        linux_launcher(desktop_exec_path(text))?
            .enable()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        write_launch_agent(&path)?;
        launchctl("enable")?;
    }
    #[cfg(target_os = "windows")]
    windows_enable(&windows_exec_path(text)?)?;
    Ok(())
}

fn disable() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    linux_launcher("unused")?
        .disable()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    {
        match std::fs::remove_file(launch_agent_path()?) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    #[cfg(target_os = "windows")]
    windows_disable()?;
    Ok(())
}

fn read_enabled(_unused: bool) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        return linux_launcher("unused")?
            .is_enabled()
            .map_err(|e| e.to_string());
    }
    #[cfg(target_os = "macos")]
    {
        if !launch_agent_path()?.is_file() {
            return Ok(false);
        }
        return Ok(!launchctl_disabled(&launchctl("print-disabled")?)?);
    }
    #[cfg(target_os = "windows")]
    return windows_enabled();
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Ok(false)
}

#[cfg(target_os = "linux")]
fn linux_launcher(path: &str) -> Result<AutoLaunch, String> {
    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(ID)
        .set_app_path(path)
        .set_args(&[] as &[&str])
        .set_linux_launch_mode(LinuxLaunchMode::XdgAutostart);
    builder.build().map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn launch_agent_path() -> Result<PathBuf, String> {
    let home = directories::BaseDirs::new()
        .ok_or("无法确定当前用户目录")?
        .home_dir()
        .to_path_buf();
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{ID}.plist")))
}

#[cfg(target_os = "macos")]
fn write_launch_agent(path: &Path) -> Result<(), String> {
    let entry = launch_agent_path()?;
    let parent = entry.parent().ok_or("登录启动项目录无效")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut data = plist::Dictionary::new();
    data.insert("Label".into(), ID.into());
    data.insert(
        "ProgramArguments".into(),
        plist::Value::Array(vec![path.to_str().expect("路径已验证").into()]),
    );
    data.insert("RunAtLoad".into(), true.into());
    plist::Value::Dictionary(data)
        .to_file_xml(&entry)
        .map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o644))
        .map_err(|e| e.to_string())
}

fn launch_path(exe: &Path, appimage: Option<&Path>, macos: bool) -> Result<PathBuf, String> {
    let path = appimage.unwrap_or(exe);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("启动程序必须使用固定的绝对路径".into());
    }
    let text = path.to_str().ok_or("启动程序路径不是有效的 UTF-8")?;
    if text.chars().any(char::is_control) {
        return Err("启动程序路径不能包含控制字符".into());
    }
    #[cfg(target_os = "linux")]
    if text.contains('=') {
        return Err("Linux 启动程序路径不能包含 '='".into());
    }
    if macos
        && (path
            .components()
            .any(|c| c.as_os_str() == "AppTranslocation")
            || [
                "/Volumes",
                "/tmp",
                "/private/tmp",
                "/var/folders",
                "/private/var/folders",
            ]
            .iter()
            .any(|base| path.starts_with(base)))
    {
        return Err("请将 Kitty Pro 拷贝到“应用程序”目录，退出磁盘映像并从安装位置重新打开，再启用登录自启动".into());
    }
    if !macos && appimage.is_none() && text.contains("/.mount_") {
        return Err("无法确定 AppImage 的真实文件，请从固定位置的 AppImage 启动后重试".into());
    }
    Ok(path.to_path_buf())
}

#[cfg(any(target_os = "linux", test))]
fn desktop_exec_path(text: &str) -> String {
    // 上游直接插入 Exec；先引用参数，再做 Desktop Entry 字符串层转义。
    let mut quoted = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\")
}

#[cfg(any(target_os = "windows", test))]
fn windows_exec_path(text: &str) -> Result<String, String> {
    if text.contains('"') || text.chars().any(char::is_control) {
        return Err("Windows 启动程序路径包含非法字符".into());
    }
    let command = format!("\"{text}\"");
    // Run 键 REG_SZ 值实际上限 255 个字符（含引号与上游追加的空格），
    // 比路径的 MAX_PATH 260 更严；超限时开机启动阶段会跳过该条目，
    // 表现为界面显示已开启但实际不自启。
    if command.encode_utf16().count() > 255 {
        return Err("Windows 登录启动命令不能超过 255 个字符，请安装到更短的路径".into());
    }
    Ok(command)
}

#[cfg(target_os = "windows")]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

#[cfg(target_os = "windows")]
fn windows_enable(command: &str) -> Result<(), String> {
    use windows_registry::CURRENT_USER;
    CURRENT_USER
        .create(RUN_KEY)
        .and_then(|key| key.set_string(ID, command))
        .map_err(|e| e.to_string())?;
    if let Ok(key) = CURRENT_USER.create(APPROVED_KEY) {
        let _ = key.set_bytes(
            ID,
            windows_registry::Type::Bytes,
            &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_disable() -> Result<(), String> {
    use windows_registry::CURRENT_USER;
    const NOT_FOUND: windows_result::HRESULT = windows_result::HRESULT::from_win32(0x8007_0002);
    for key_path in [RUN_KEY, APPROVED_KEY] {
        match CURRENT_USER
            .options()
            .write()
            .open(key_path)
            .and_then(|key| key.remove_value(ID))
        {
            Ok(()) => {}
            Err(error) if error.code() == NOT_FOUND => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_enabled() -> Result<bool, String> {
    use windows_registry::CURRENT_USER;
    let registered = match CURRENT_USER
        .open(RUN_KEY)
        .and_then(|key| key.get_string(ID))
    {
        Ok(value) => !value.trim().is_empty(),
        Err(_) => false,
    };
    if !registered {
        return Ok(false);
    }
    match CURRENT_USER
        .open(APPROVED_KEY)
        .and_then(|key| key.get_value(ID))
    {
        Ok(windows_registry::Data::Binary(bytes)) if bytes.len() == 12 => {
            Ok(matches!(bytes[0], 2 | 6))
        }
        Ok(_) => Ok(false),
        Err(_) => Ok(true),
    }
}

#[cfg(any(target_os = "macos", test))]
fn launchctl_args(action: &str, uid: &str) -> Result<Vec<String>, String> {
    if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
        return Err("当前用户 UID 无效".into());
    }
    let domain = format!("gui/{uid}");
    match action {
        "enable" => Ok(vec![action.into(), format!("{domain}/{ID}")]),
        "print-disabled" => Ok(vec![action.into(), domain]),
        _ => Err("不支持的 launchctl 操作".into()),
    }
}

#[cfg(any(target_os = "macos", test))]
fn launchctl_disabled(text: &str) -> Result<bool, String> {
    for line in text.lines() {
        if let Some((label, state)) = line.trim().split_once("=>") {
            if label.trim() == format!("\"{ID}\"") {
                // print-disabled 真实词法是 enabled/disabled；true/false 仅为旧版兼容。
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
fn launchctl(action: &str) -> Result<String, String> {
    let uid = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|e| e.to_string())?;
    if !uid.status.success() {
        return Err("无法读取当前用户 UID".into());
    }
    let uid = String::from_utf8(uid.stdout).map_err(|e| e.to_string())?;
    let output = std::process::Command::new("/bin/launchctl")
        .args(launchctl_args(action, uid.trim())?)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "launchctl {action} 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_paths_and_appimage() {
        for path in [
            "kitty",
            "/opt/../kitty",
            "/opt/kitty\npro",
            "/tmp/.mount_kitty/kitty",
        ] {
            assert!(launch_path(Path::new(path), None, false).is_err());
        }
        // '=' 只在 Linux Exec 上属于非法字符。
        #[cfg(target_os = "linux")]
        assert!(launch_path(Path::new("/opt/kitty=pro"), None, false).is_err());
        let image = Path::new("/home/user/My Apps/Kitty.AppImage");
        assert_eq!(
            launch_path(Path::new("/tmp/.mount_kitty/kitty"), Some(image), false).unwrap(),
            image
        );
        assert!(launch_path(
            Path::new("/opt/kitty"),
            Some(Path::new("relative.AppImage")),
            false
        )
        .is_err());
    }

    #[test]
    fn macos_rejects_temporary_locations() {
        for path in [
            "/Volumes/Kitty/kitty",
            "/private/var/folders/a/AppTranslocation/b/kitty",
            "/tmp/kitty",
        ] {
            assert!(launch_path(Path::new(path), None, true).is_err());
        }
        assert!(launch_path(
            Path::new("/Applications/Kitty & <Pro>.app/Contents/MacOS/kitty"),
            None,
            true
        )
        .is_ok());
    }

    #[test]
    fn quoting_and_windows_length() {
        assert_eq!(
            desktop_exec_path("/My Apps/kitty%$\"\\`"),
            "\"/My Apps/kitty%%\\\\$\\\\\"\\\\\\\\\\\\`\""
        );
        assert_eq!(
            windows_exec_path(r"C:\Program Files\Kitty\kitty.exe").unwrap(),
            r#""C:\Program Files\Kitty\kitty.exe""#
        );
        assert!(windows_exec_path(&"a".repeat(253)).is_ok());
        assert!(windows_exec_path(&"a".repeat(254)).is_err());
        assert!(windows_exec_path("bad\"path").is_err());
    }

    #[test]
    fn macos_disabled_override() {
        assert_eq!(
            launchctl_args("enable", "501").unwrap()[1],
            format!("gui/501/{ID}")
        );
        assert!(launchctl_args("enable", "501/other").is_err());
        // 真实输出：tab 缩进、引号 label、无分号。
        assert!(launchctl_disabled(&format!(
            "disabled services = {{\n\t\t\"{ID}\" => disabled\n}}"
        ))
        .unwrap());
        assert!(!launchctl_disabled(&format!(
            "disabled services = {{\n\t\t\"{ID}\" => enabled\n}}"
        ))
        .unwrap());
        // 旧版 true/false 词法仍兼容。
        assert!(
            launchctl_disabled(&format!("disabled services = {{\n\"{ID}\" => true\n}}")).unwrap()
        );
        assert!(
            !launchctl_disabled(&format!("disabled services = {{\n\"{ID}\" => false\n}}")).unwrap()
        );
        // 多行混合：只认本应用的 label。
        assert!(!launchctl_disabled(&format!(
            "disabled services = {{\n\
                 \t\t\"com.apple.Siri.agent\" => disabled\n\
                 \t\t\"{ID}\" => enabled\n\
                 \t\t\"homebrew.mxcl.redis\" => disabled\n}}"
        ))
        .unwrap());
        assert!(launchctl_disabled(&format!(
            "disabled services = {{\n\
                 \t\t\"{ID}\" => disabled\n\
                 \t\t\"com.apple.Siri.agent\" => enabled\n}}"
        ))
        .unwrap());
        // 空列表与只有其它 label 的列表都视为未禁用。
        assert!(!launchctl_disabled("disabled services = {\n}").unwrap());
        assert!(
            !launchctl_disabled("disabled services = {\n\t\t\"other\" => disabled\n}").unwrap()
        );
        // 未知词法仍报错。
        assert!(
            launchctl_disabled(&format!("disabled services = {{\n\"{ID}\" => unknown\n}}"))
                .is_err()
        );
        assert!(launchctl_disabled("unexpected output").is_err());
    }
}
