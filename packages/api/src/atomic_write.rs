//! 统一的原子写与崩溃恢复（B3 / B15 / B16）。
//!
//! 拆分后由 crate 根 `pub(crate) use atomic_write::*;` 重新导出，因此沿用
//! `use super::*;` 直接引用其它模块的函数。所有文件操作只在桌面 / 服务端目标
//! 编译：wasm 侧有自己的空实现，这里把整个模块内容用 cfg 门控掉。

#[cfg(not(target_arch = "wasm32"))]
use super::*;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write as _;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// 本模块生成的临时/备份文件都带这个标记，便于崩溃恢复精确识别，
/// 不会误删规则集的 `.srs.backup`、`corrupt.json` 或历史备份。
#[cfg(not(target_arch = "wasm32"))]
const ATOMIC_MARKER: &str = "kitty-atomic";
/// 文件名中「原名」与唯一戳之间的分隔符，用于从备份名反推目标文件。
#[cfg(not(target_arch = "wasm32"))]
const ATOMIC_SEPARATOR: &str = ".kitty-atomic-";

/// 进程 id + 时间戳 + 原子计数器，配合 `create_new` 避免任何碰撞。
#[cfg(not(target_arch = "wasm32"))]
fn unique_stamp() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!(
        "{}-{}-{}-{}",
        ATOMIC_MARKER,
        std::process::id(),
        timestamp,
        id
    )
}

/// 在目标文件同一目录生成一个唯一的「.tmp」兄弟文件名。
#[cfg(not(target_arch = "wasm32"))]
fn temporary_sibling(path: &Path) -> PathBuf {
    match path.file_name() {
        Some(name) => {
            let mut name = name.to_os_string();
            name.push(format!(".{}.tmp", unique_stamp()));
            path.with_file_name(name)
        }
        None => path.with_extension(format!("{}.tmp", unique_stamp())),
    }
}

/// 在目标文件同一目录生成一个唯一的「.backup」兄弟文件名（Windows 用）。
#[cfg(not(target_arch = "wasm32"))]
fn backup_sibling(path: &Path) -> PathBuf {
    match path.file_name() {
        Some(name) => {
            let mut name = name.to_os_string();
            name.push(format!(".{}.backup", unique_stamp()));
            path.with_file_name(name)
        }
        None => path.with_extension(format!("{}.backup", unique_stamp())),
    }
}

/// 用 `create_new` 创建唯一临时文件，权限等价于 `create_private_file` 的 0600
/// （这里不便直接复用后者：`create_private_file` 用 `create+truncate`，无法
/// 保证唯一性与不跟随已存在文件）。
#[cfg(not(target_arch = "wasm32"))]
fn create_unique_temporary(path: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    loop {
        let temporary = temporary_sibling(path);
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

/// 把已经写好并 `sync_all` 的临时文件安全地替换到目标路径。
///
/// Windows 无法用 `rename` 原子覆盖已存在文件，必须先把旧文件「移动」到唯一
/// 备份名，再 rename 新文件；任一步失败都把备份移回原位并清理临时文件，
/// 绝不出现「旧文件已删、新文件未就位」的窗口。
///
/// 非 Windows 上 `rename` 本身即原子覆盖；仅当它罕见地失败时才回退到
/// `replace_with_backup`，这样 Windows 分支的备份/回滚逻辑在 macOS / Linux
/// 上也能被单测直接覆盖。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn atomic_replace(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    let result = replace_with_backup(temporary, destination);
    #[cfg(not(windows))]
    let result = match std::fs::rename(temporary, destination) {
        Ok(()) => Ok(()),
        Err(_) => replace_with_backup(temporary, destination),
    };
    if result.is_err() {
        // 任何失败路径都清理临时文件，不留垃圾。
        let _ = std::fs::remove_file(temporary);
    }
    result
}

/// Windows 安全替换逻辑：备份旧文件 → 安装新文件 → 失败回滚。
///
/// 故意不加 `#[cfg(windows)]`，以便在非 Windows 上也能被单测直接调用，
/// 验证备份/回滚分支。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn replace_with_backup(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    // 目标若已存在但不是普通文件（目录/符号链接），绝不移动或覆盖它。
    if std::fs::symlink_metadata(destination)
        .map(|metadata| metadata.file_type().is_dir())
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(temporary);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "替换目标是目录，不能用文件覆盖",
        ));
    }

    let backup = backup_sibling(destination);
    // 先移动旧文件到唯一备份名（绝不删除），再安装新文件。
    let had_backup = match std::fs::rename(destination, &backup) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            let _ = std::fs::remove_file(temporary);
            return Err(error);
        }
    };

    if let Err(error) = std::fs::rename(temporary, destination) {
        // 任一步失败：把备份移回原位，删除临时文件并返回错误。
        if had_backup {
            if let Err(restore_error) = std::fs::rename(&backup, destination) {
                let _ = std::fs::remove_file(temporary);
                return Err(std::io::Error::new(
                    error.kind(),
                    format!(
                        "{error}；回滚备份失败: {restore_error}（备份保留在 {}）",
                        backup.display()
                    ),
                ));
            }
        }
        let _ = std::fs::remove_file(temporary);
        return Err(error);
    }

    // 成功：旧文件已不再需要。
    if had_backup {
        let _ = std::fs::remove_file(&backup);
    }
    Ok(())
}

/// 原子写入字节：创建父目录 → 同目录唯一临时文件 → 写入 + `sync_all` →
/// 关闭 → 安全替换。任何失败路径都会清理临时文件。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "写入路径缺少父目录")
    })?;
    std::fs::create_dir_all(parent)?;

    let (temporary, mut file) = create_unique_temporary(path)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    let result = result.and_then(|_| atomic_replace(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// 把可序列化值以 pretty JSON 原子写入文件。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn atomic_write_json<T: serde::Serialize + ?Sized>(
    path: &Path,
    value: &T,
) -> Result<(), ServerFnError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| api_err(ApiErrorCode::Storage, format!("序列化配置失败: {error}")))?;
    atomic_write_bytes(path, &bytes)
        .map_err(|error| api_err(ApiErrorCode::Storage, format!("写入配置文件失败: {error}")))
}

/// 从备份残留名反推目标文件名（去掉唯一戳与 `.backup` 后缀）。
#[cfg(not(target_arch = "wasm32"))]
fn destination_from_backup(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".backup")?;
    let (original, _) = stem.split_once(ATOMIC_SEPARATOR)?;
    if original.is_empty() {
        None
    } else {
        Some(original)
    }
}

/// 清理单个目录里本模块产生的 `.tmp` / `.backup` 残留。
///
/// 只处理顶层普通文件，绝不递归子目录，因此 `profile-backups/` 下
/// `MAX_PROFILE_BACKUPS` 保留的历史备份与子目录原样保留。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn cleanup_atomic_residue_in(directory: &Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.contains(ATOMIC_MARKER) {
            continue;
        }
        if name.ends_with(".tmp") {
            // 未提交的新数据，直接清理。
            let _ = std::fs::remove_file(&path);
        } else if name.ends_with(".backup") {
            // 目标仍在则删除（已被后续写入取代）；目标缺失则移回原位
            // （上次替换在两步之间崩溃），避免丢数据。
            match destination_from_backup(name) {
                Some(original) => {
                    let destination = directory.join(original);
                    if destination.exists() {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        let _ = std::fs::rename(&path, &destination);
                    }
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    Ok(())
}

/// 统一崩溃恢复入口：规则集事务恢复 + 本模块写入残留清理。
///
/// 恢复本质是启动期一次性动作；`load_native_profile` 每次读取都会调用本函数，
/// 用 OnceLock 保证只真正执行一次，避免重复触发规则集事务恢复、降低与
/// install_rule_set_cache 的竞争。单测通过 `recover_interrupted_writes_in`
/// 注入临时目录，不经过这里，也不会碰用户真实配置目录。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn recover_interrupted_writes() -> Result<(), ServerFnError> {
    use std::sync::OnceLock;

    static RECOVERED: OnceLock<()> = OnceLock::new();
    if RECOVERED.get().is_some() {
        return Ok(());
    }
    let result = recover_interrupted_writes_once();
    // 尽力而为：无论成功与否都标记已运行，避免后续读取反复触发或反复报错。
    let _ = RECOVERED.set(());
    result
}

#[cfg(not(target_arch = "wasm32"))]
fn recover_interrupted_writes_once() -> Result<(), ServerFnError> {
    let paths = native_rule_set_cache_paths()?;
    let config_directory = profile_path()?
        .parent()
        .ok_or_else(|| api_err(ApiErrorCode::Storage, "本地配置目录无效"))?
        .to_path_buf();
    let rules_directory = Path::new(&paths.geosite)
        .parent()
        .ok_or_else(|| api_err(ApiErrorCode::Storage, "规则缓存目录无效"))?
        .to_path_buf();
    recover_interrupted_writes_in(&config_directory, &rules_directory)
}

/// 可注入目录的恢复逻辑（供单测在临时目录验证，也供生产入口复用）。
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn recover_interrupted_writes_in(
    config_directory: &Path,
    rules_directory: &Path,
) -> Result<(), ServerFnError> {
    // 规则集事务沿用 rule_set_cache.rs 的崩溃恢复（按其现有签名传参）。
    let geosite = rules_directory.join("geosite-cn.srs");
    let geoip = rules_directory.join("geoip-cn.srs");
    recover_rule_set_transaction(&geosite, &geoip)?;
    // 清理本模块产生的 `.tmp` / `.backup` 残留；
    // `MAX_PROFILE_BACKUPS` 的历史备份在 profile-backups/ 子目录且为 .json，不在范围内。
    cleanup_atomic_residue_in(config_directory)
        .map_err(|error| api_err(ApiErrorCode::Storage, format!("清理写入残留失败: {error}")))?;
    cleanup_atomic_residue_in(rules_directory)
        .map_err(|error| api_err(ApiErrorCode::Storage, format!("清理写入残留失败: {error}")))?;
    Ok(())
}
