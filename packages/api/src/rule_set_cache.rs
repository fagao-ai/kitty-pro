//! Rule-set cache download, install transaction and crash recovery.
//!
//! Split out of `lib.rs`; the crate root re-exports everything through
//! `pub(crate) use ...::*`, so sibling modules keep using `use super::*`.

use super::*;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rule_set_update_lock() -> &'static tokio::sync::Mutex<()> {
    use std::sync::OnceLock;

    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn prepare_rule_set_cache_for_request(
    request: Option<&ConnectionRequest>,
) -> Result<Option<RuleSetCachePaths>, ServerFnError> {
    if request.map(|request| request.mode) != Some(TunnelMode::Rule) {
        return Ok(None);
    }
    let paths = native_rule_set_cache_paths()?;
    let check_paths = paths.clone();
    if run_native_blocking("检查规则缓存失败", move || {
        native_rule_set_cache_ready(&check_paths)
    })
    .await?
    {
        return Ok(Some(paths));
    }

    let _guard = rule_set_update_lock().lock().await;
    let check_paths = paths.clone();
    if !run_native_blocking("检查规则缓存失败", move || {
        native_rule_set_cache_ready(&check_paths)
    })
    .await?
    {
        download_and_install_rule_sets(paths.clone()).await?;
    }
    Ok(Some(paths))
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub(crate) async fn prepare_rule_set_cache_for_request(
    _request: Option<&ConnectionRequest>,
) -> Result<Option<RuleSetCachePaths>, ServerFnError> {
    Ok(None)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn native_update_rule_sets(
    force: bool,
) -> Result<RuleSetUpdateResult, ServerFnError> {
    let _guard = rule_set_update_lock().lock().await;
    let paths = native_rule_set_cache_paths()?;
    if !force {
        let check_paths = paths.clone();
        let current = run_native_blocking("检查规则缓存失败", move || {
            native_rule_set_cache_current(&check_paths)
        })
        .await?;
        if current {
            return Ok(RuleSetUpdateResult { updated: false });
        }
    }
    download_and_install_rule_sets(paths).await?;
    Ok(RuleSetUpdateResult { updated: true })
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub(crate) async fn native_update_rule_sets(
    _force: bool,
) -> Result<RuleSetUpdateResult, ServerFnError> {
    Err(ServerFnError::new("浏览器端不能直接更新分流规则"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn download_and_install_rule_sets(
    paths: RuleSetCachePaths,
) -> Result<(), ServerFnError> {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("kitty-pro/0.1")
        .build()
        .map_err(|error| {
            api_err(
                ApiErrorCode::Network,
                format!("创建规则下载客户端失败: {error}"),
            )
        })?;
    let geosite_download =
        download_rule_set(client.clone(), CHINA_GEOSITE_RULE_SET_URL, "域名分流规则");
    let geoip_download = download_rule_set(client, CHINA_GEOIP_RULE_SET_URL, "IP 分流规则");
    let (geosite, geoip) = tokio::try_join!(geosite_download, geoip_download)?;
    run_native_blocking("保存规则缓存失败", move || {
        install_rule_set_cache(&paths, &geosite, &geoip)
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn download_rule_set(
    client: reqwest::Client,
    url: &'static str,
    label: &'static str,
) -> Result<Vec<u8>, ServerFnError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| api_err(ApiErrorCode::Network, format!("下载{label}失败: {error}")))?
        .error_for_status()
        .map_err(|error| {
            api_err(
                ApiErrorCode::Network,
                format!("{label}服务返回错误: {error}"),
            )
        })?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RULE_SET_BYTES as u64)
    {
        return Err(api_err(
            ApiErrorCode::Network,
            format!("{label}超过 8 MiB 限制"),
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| api_err(ApiErrorCode::Network, format!("读取{label}失败: {error}")))?;
    if bytes.is_empty() || bytes.len() > MAX_RULE_SET_BYTES {
        return Err(api_err(ApiErrorCode::Network, format!("{label}大小无效")));
    }
    Ok(bytes.to_vec())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_rule_set_cache_paths() -> Result<RuleSetCachePaths, ServerFnError> {
    let profile = profile_path()?;
    let directory = profile
        .parent()
        .ok_or_else(|| ServerFnError::new("无法确定规则缓存目录"))?
        .join("rules");
    let geosite = directory.join("geosite-cn.srs");
    let geoip = directory.join("geoip-cn.srs");
    Ok(RuleSetCachePaths {
        geosite: geosite
            .to_str()
            .ok_or_else(|| ServerFnError::new("域名规则缓存路径不是有效 UTF-8"))?
            .to_string(),
        geoip: geoip
            .to_str()
            .ok_or_else(|| ServerFnError::new("IP 规则缓存路径不是有效 UTF-8"))?
            .to_string(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn prepare_native_cache_file() -> Result<String, ServerFnError> {
    let profile = profile_path()?;
    let directory = profile
        .parent()
        .ok_or_else(|| ServerFnError::new("无法确定 sing-box 缓存目录"))?
        .join("cache");
    std::fs::create_dir_all(&directory)
        .map_err(|error| ServerFnError::new(format!("创建 sing-box 缓存目录失败: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| ServerFnError::new(format!("限制 sing-box 缓存目录权限失败: {error}")),
        )?;
    }

    let path = directory.join("sing-box.db");
    prepare_private_cache_file(&path)?;
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| ServerFnError::new("sing-box 缓存路径不是有效 UTF-8"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn prepare_private_cache_file(path: &std::path::Path) -> Result<(), ServerFnError> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(ServerFnError::new("sing-box 缓存文件不能是符号链接"));
        }
        if !metadata.file_type().is_file() {
            return Err(ServerFnError::new("sing-box 缓存路径不是普通文件"));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| ServerFnError::new(format!("准备 sing-box 缓存文件失败: {error}")))?;
    if !file
        .metadata()
        .map_err(|error| ServerFnError::new(format!("检查 sing-box 缓存文件失败: {error}")))?
        .is_file()
    {
        return Err(ServerFnError::new("sing-box 缓存路径不是普通文件"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                ServerFnError::new(format!("限制 FakeIP 缓存文件权限失败: {error}"))
            })?;
    }
    drop(file);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_rule_set_cache_ready(
    paths: &RuleSetCachePaths,
) -> Result<bool, ServerFnError> {
    let geosite = std::path::Path::new(&paths.geosite);
    let geoip = std::path::Path::new(&paths.geoip);
    recover_rule_set_transaction(geosite, geoip)?;
    if !geosite.is_file() || !geoip.is_file() {
        return Ok(false);
    }
    Ok(singbox::validate_rule_set_file(geosite).is_ok()
        && singbox::validate_rule_set_file(geoip).is_ok())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_rule_set_cache_current(
    paths: &RuleSetCachePaths,
) -> Result<bool, ServerFnError> {
    if !native_rule_set_cache_ready(paths)? {
        return Ok(false);
    }
    for path in [&paths.geosite, &paths.geoip] {
        let age = std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
            .map_err(|error| ServerFnError::new(format!("读取规则缓存时间失败: {error}")))?;
        if age > RULE_SET_CACHE_TTL {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_rule_set_cache(
    paths: &RuleSetCachePaths,
    geosite_content: &[u8],
    geoip_content: &[u8],
) -> Result<(), ServerFnError> {
    use std::io::Write;

    let geosite = std::path::Path::new(&paths.geosite);
    let geoip = std::path::Path::new(&paths.geoip);
    let directory = geosite
        .parent()
        .ok_or_else(|| ServerFnError::new("域名规则缓存目录无效"))?;
    if geoip.parent() != Some(directory) {
        return Err(ServerFnError::new("规则缓存文件不在同一目录"));
    }
    std::fs::create_dir_all(directory)
        .map_err(|error| ServerFnError::new(format!("创建规则缓存目录失败: {error}")))?;
    recover_rule_set_transaction(geosite, geoip)?;

    let geosite_temp = directory.join(".geosite-cn.pending.tmp");
    let geoip_temp = directory.join(".geoip-cn.pending.tmp");
    let write_result = (|| -> std::io::Result<()> {
        for (path, content) in [
            (geosite_temp.as_path(), geosite_content),
            (geoip_temp.as_path(), geoip_content),
        ] {
            let mut file = create_private_file(path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        return Err(api_err(
            ApiErrorCode::Storage,
            format!("保存规则临时文件失败: {error}"),
        ));
    }
    if let Err(error) = singbox::validate_rule_set_file(&geosite_temp)
        .and_then(|_| singbox::validate_rule_set_file(&geoip_temp))
    {
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        return Err(ServerFnError::new(error.to_string()));
    }

    let geosite_backup = rule_set_backup_path(geosite);
    let geoip_backup = rule_set_backup_path(geoip);
    let geosite_existed = geosite.exists();
    let geoip_existed = geoip.exists();
    let transaction = rule_set_transaction_path(directory);
    let committed = rule_set_commit_path(directory);
    let state = RuleSetInstallTransaction {
        geosite_existed,
        geoip_existed,
    };
    if let Err(error) = write_rule_set_transaction(&transaction, &state) {
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        return Err(api_err(
            ApiErrorCode::Storage,
            format!("创建规则更新事务失败: {error}"),
        ));
    }

    if let Err(error) = stage_rule_set_backup(geosite, &geosite_backup)
        .and_then(|_| stage_rule_set_backup(geoip, &geoip_backup))
        .and_then(|_| std::fs::rename(&geosite_temp, geosite))
        .and_then(|_| std::fs::rename(&geoip_temp, geoip))
        .and_then(|_| write_rule_set_commit(&committed))
    {
        let rollback_result = rollback_rule_set_pair(geosite, geoip, &state);
        let _ = std::fs::remove_file(&geosite_temp);
        let _ = std::fs::remove_file(&geoip_temp);
        if rollback_result.is_ok() {
            let _ = std::fs::remove_file(&transaction);
            let _ = std::fs::remove_file(&committed);
        }
        if let Err(rollback_error) = rollback_result {
            return Err(api_err(
                ApiErrorCode::Storage,
                format!("替换规则缓存失败: {error}；回滚失败: {rollback_error}"),
            ));
        }
        return Err(api_err(
            ApiErrorCode::Storage,
            format!("替换规则缓存失败: {error}"),
        ));
    }
    finish_rule_set_transaction(geosite, geoip)
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RuleSetInstallTransaction {
    // 事务恢复测试构造该结构体；拆分后需跨模块可见，仍限定在 crate 内。
    pub(crate) geosite_existed: bool,
    pub(crate) geoip_existed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rule_set_backup_path(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension("srs.backup")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rule_set_transaction_path(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join(".rule-set-install.json")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rule_set_commit_path(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join(".rule-set-install.committed")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_rule_set_transaction(
    path: &std::path::Path,
    state: &RuleSetInstallTransaction,
) -> std::io::Result<()> {
    use std::io::Write;

    let content = serde_json::to_vec(state).map_err(std::io::Error::other)?;
    let mut file = create_private_file(path)?;
    file.write_all(&content)?;
    file.sync_all()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_rule_set_commit(path: &std::path::Path) -> std::io::Result<()> {
    // 固定标记内容；B3：统一走 atomic_write_bytes（临时文件 + sync_all + 安全替换）。
    atomic_write_bytes(path, b"committed")
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn stage_rule_set_backup(
    path: &std::path::Path,
    backup: &std::path::Path,
) -> std::io::Result<()> {
    if path.exists() {
        std::fs::rename(path, backup)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn restore_rule_set_backup(
    path: &std::path::Path,
    backup: &std::path::Path,
) -> std::io::Result<()> {
    if backup.exists() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        std::fs::rename(backup, path)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rollback_rule_set_install(
    path: &std::path::Path,
    backup: &std::path::Path,
    existed: bool,
) -> std::io::Result<()> {
    if backup.exists() {
        return restore_rule_set_backup(path, backup);
    }
    if !existed && path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rollback_rule_set_pair(
    geosite: &std::path::Path,
    geoip: &std::path::Path,
    state: &RuleSetInstallTransaction,
) -> std::io::Result<()> {
    let geosite_result = rollback_rule_set_install(
        geosite,
        &rule_set_backup_path(geosite),
        state.geosite_existed,
    );
    let geoip_result =
        rollback_rule_set_install(geoip, &rule_set_backup_path(geoip), state.geoip_existed);
    geosite_result.and(geoip_result)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn finish_rule_set_transaction(
    geosite: &std::path::Path,
    geoip: &std::path::Path,
) -> Result<(), ServerFnError> {
    let directory = geosite
        .parent()
        .ok_or_else(|| ServerFnError::new("域名规则缓存目录无效"))?;
    for path in [rule_set_backup_path(geosite), rule_set_backup_path(geoip)] {
        if path.exists() {
            std::fs::remove_file(path).map_err(|error| {
                api_err(
                    ApiErrorCode::Storage,
                    format!("清理旧规则缓存备份失败: {error}"),
                )
            })?;
        }
    }
    let transaction = rule_set_transaction_path(directory);
    if transaction.exists() {
        std::fs::remove_file(&transaction).map_err(|error| {
            api_err(
                ApiErrorCode::Storage,
                format!("清理规则更新事务失败: {error}"),
            )
        })?;
    }
    let committed = rule_set_commit_path(directory);
    if committed.exists() {
        std::fs::remove_file(committed).map_err(|error| {
            api_err(
                ApiErrorCode::Storage,
                format!("完成规则更新事务失败: {error}"),
            )
        })?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn recover_rule_set_transaction(
    geosite: &std::path::Path,
    geoip: &std::path::Path,
) -> Result<(), ServerFnError> {
    let directory = geosite
        .parent()
        .ok_or_else(|| ServerFnError::new("域名规则缓存目录无效"))?;
    if geoip.parent() != Some(directory) {
        return Err(ServerFnError::new("规则缓存文件不在同一目录"));
    }
    let transaction = rule_set_transaction_path(directory);
    let committed = rule_set_commit_path(directory);
    if transaction.exists() {
        if committed.exists() {
            return finish_rule_set_transaction(geosite, geoip);
        }
        let content = std::fs::read(&transaction).map_err(|error| {
            api_err(
                ApiErrorCode::Storage,
                format!("读取规则更新事务失败: {error}"),
            )
        })?;
        let state: RuleSetInstallTransaction = match serde_json::from_slice(&content) {
            Ok(state) => state,
            Err(error)
                if !rule_set_backup_path(geosite).exists()
                    && !rule_set_backup_path(geoip).exists() =>
            {
                std::fs::remove_file(&transaction).map_err(|remove_error| {
                    ServerFnError::new(format!(
                        "规则更新事务损坏: {error}；清理失败: {remove_error}"
                    ))
                })?;
                return Ok(());
            }
            Err(error) => {
                return Err(api_err(
                    ApiErrorCode::Storage,
                    format!("规则更新事务损坏: {error}"),
                ));
            }
        };
        rollback_rule_set_pair(geosite, geoip, &state).map_err(|error| {
            api_err(ApiErrorCode::Storage, format!("恢复规则缓存失败: {error}"))
        })?;
        let _ = std::fs::remove_file(directory.join(".geosite-cn.pending.tmp"));
        let _ = std::fs::remove_file(directory.join(".geoip-cn.pending.tmp"));
        std::fs::remove_file(&transaction).map_err(|error| {
            api_err(
                ApiErrorCode::Storage,
                format!("清理规则更新事务失败: {error}"),
            )
        })?;
    } else {
        // Compatibility with caches written before pair transactions were introduced.
        recover_rule_set_backup(geosite)?;
        recover_rule_set_backup(geoip)?;
    }
    if committed.exists() {
        std::fs::remove_file(committed)
            .map_err(|error| ServerFnError::new(format!("清理规则提交标记失败: {error}")))?;
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn recover_rule_set_backup(path: &std::path::Path) -> Result<(), ServerFnError> {
    let backup = rule_set_backup_path(path);
    if !backup.exists() {
        return Ok(());
    }
    if path.exists() && singbox::validate_rule_set_file(path).is_ok() {
        std::fs::remove_file(backup)
            .map_err(|error| ServerFnError::new(format!("清理旧规则备份失败: {error}")))?;
        return Ok(());
    }
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| ServerFnError::new(format!("移除损坏规则缓存失败: {error}")))?;
    }
    std::fs::rename(backup, path)
        .map_err(|error| api_err(ApiErrorCode::Storage, format!("恢复规则缓存失败: {error}")))
}
