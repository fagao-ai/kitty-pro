use super::{profile_path, SyncConfig, SyncProviderKind, SyncResult};
use dioxus::prelude::ServerFnError;
use proxy_core::{validate_custom_rules, SyncSnapshot, SYNC_SNAPSHOT_FORMAT};
use reqwest::header::{ETAG, IF_MATCH, IF_NONE_MATCH};
use reqwest::{Method, Response, StatusCode};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_SYNC_BYTES: usize = 20 * 1024 * 1024;

pub(super) struct DownloadedSnapshot {
    pub snapshot: SyncSnapshot,
    pub remote_revision: String,
}

#[derive(Debug, Clone)]
enum UploadCondition {
    Unconditional,
    Match(String),
    Missing,
}

pub(super) fn load_native_sync_config() -> Result<SyncConfig, ServerFnError> {
    let path = sync_config_path()?;
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| ServerFnError::new(format!("同步配置损坏: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SyncConfig::default()),
        Err(error) => Err(ServerFnError::new(format!("读取同步配置失败: {error}"))),
    }
}

pub(super) fn save_native_sync_config(config: &SyncConfig) -> Result<SyncConfig, ServerFnError> {
    let existing = load_native_sync_config()?;
    let saved = merge_sync_metadata(&existing, config);
    save_native_sync_state(&saved)?;
    Ok(saved)
}

fn merge_sync_metadata(existing: &SyncConfig, config: &SyncConfig) -> SyncConfig {
    let mut saved = config.clone();
    if same_remote_target(existing, config) {
        saved.last_sync_at = existing.last_sync_at;
        saved.last_remote_revision = existing.last_remote_revision.clone();
    } else {
        saved.last_sync_at = 0;
        saved.last_remote_revision.clear();
    }
    saved
}

pub(super) fn save_native_sync_state(config: &SyncConfig) -> Result<(), ServerFnError> {
    if config.enabled {
        validate_config(config)?;
    }
    write_private_json(&sync_config_path()?, config)
}

pub(super) fn operation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub(super) async fn push(
    config: &SyncConfig,
    mut snapshot: SyncSnapshot,
    force: bool,
) -> Result<SyncResult, ServerFnError> {
    validate_config(config)?;
    validate_snapshot(&snapshot)?;

    let condition = if force {
        UploadCondition::Unconditional
    } else {
        match download_snapshot(config).await? {
            Some(remote) => {
                if config.last_sync_at == 0 && config.last_remote_revision.is_empty() {
                    return Err(ServerFnError::new(
                        "远程同步文件已存在，但本机没有该目标的同步基线，请先下载或明确选择覆盖上传",
                    ));
                }
                if remote.remote_revision.is_empty() {
                    return Err(ServerFnError::new(
                        "远端存储未提供 ETag，无法保证安全上传，请使用明确覆盖上传",
                    ));
                }
                if !config.last_remote_revision.is_empty()
                    && remote.remote_revision != config.last_remote_revision
                {
                    return Err(ServerFnError::new(
                        "远程同步文件已被其他设备修改，请先下载或明确选择覆盖上传",
                    ));
                }
                if remote.snapshot.updated_at > config.last_sync_at {
                    return Err(ServerFnError::new(format!(
                        "远程配置更新于本机上次同步之后（远程版本 {}），请先下载或明确选择覆盖上传",
                        remote.snapshot.updated_at
                    )));
                }
                UploadCondition::Match(remote.remote_revision)
            }
            None if config.last_sync_at > 0 || !config.last_remote_revision.is_empty() => {
                return Err(ServerFnError::new(
                    "远程同步文件已在本机上次同步后被删除，请明确选择覆盖上传以重新创建",
                ));
            }
            None => UploadCondition::Missing,
        }
    };

    let now = unix_timestamp()?;
    snapshot.updated_at = now.max(config.last_sync_at.saturating_add(1));
    let payload = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| ServerFnError::new(format!("序列化同步快照失败: {error}")))?;
    if payload.len() > MAX_SYNC_BYTES {
        return Err(ServerFnError::new("同步快照超过 20 MiB 限制"));
    }
    let mut remote_revision = upload(config, payload, condition).await?;
    if remote_revision.is_empty() {
        if let Ok(Some(downloaded)) = download_snapshot(config).await {
            if downloaded.snapshot == snapshot {
                remote_revision = downloaded.remote_revision;
            }
        }
    }
    let warning = remote_revision
        .is_empty()
        .then(|| "远端存储未提供 ETag；本次上传已完成，后续普通上传将要求明确覆盖".to_string());

    Ok(SyncResult {
        updated_at: snapshot.updated_at,
        subscription_count: snapshot.subscriptions.len(),
        rule_count: snapshot.custom_rules.len(),
        remote_revision,
        checkpoint_saved: false,
        warning,
    })
}

pub(super) async fn pull(config: &SyncConfig) -> Result<DownloadedSnapshot, ServerFnError> {
    validate_config(config)?;
    download_snapshot(config)
        .await?
        .ok_or_else(|| ServerFnError::new("远程同步文件不存在，请先从一台设备上传"))
}

fn same_remote_target(left: &SyncConfig, right: &SyncConfig) -> bool {
    if left.provider != right.provider
        || left.endpoint.trim() != right.endpoint.trim()
        || left.path.trim_matches('/') != right.path.trim_matches('/')
    {
        return false;
    }
    match left.provider {
        SyncProviderKind::WebDav => left.username.trim() == right.username.trim(),
        SyncProviderKind::S3 => {
            left.bucket.trim() == right.bucket.trim()
                && left.region.trim() == right.region.trim()
                && left.access_key.trim() == right.access_key.trim()
        }
    }
}

fn validate_config(config: &SyncConfig) -> Result<(), ServerFnError> {
    if !config.enabled {
        return Err(ServerFnError::new("请先启用并保存同步配置"));
    }
    validate_remote_path(&config.path)?;

    match config.provider {
        SyncProviderKind::WebDav => {
            parse_http_endpoint(&config.endpoint, false)?;
            if config.username.trim().is_empty() && !config.password.is_empty() {
                return Err(ServerFnError::new("WebDAV 密码已填写，但用户名为空"));
            }
        }
        SyncProviderKind::S3 => {
            if config.bucket.trim().is_empty() {
                return Err(ServerFnError::new("S3 Bucket 不能为空"));
            }
            if config.bucket.len() > 255
                || !config
                    .bucket
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
            {
                return Err(ServerFnError::new("S3 Bucket 格式无效"));
            }
            if config.region.trim().is_empty() {
                return Err(ServerFnError::new("S3 Region 不能为空"));
            }
            if config.region.len() > 64
                || !config
                    .region
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
            {
                return Err(ServerFnError::new("S3 Region 格式无效"));
            }
            if config.access_key.trim().is_empty() || config.secret_key.is_empty() {
                return Err(ServerFnError::new("S3 Access Key 和 Secret Key 不能为空"));
            }
            if !config.endpoint.trim().is_empty() {
                parse_http_endpoint(&config.endpoint, true)?;
            }
        }
    }
    Ok(())
}

fn parse_http_endpoint(raw: &str, allow_bare_host: bool) -> Result<reqwest::Url, ServerFnError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ServerFnError::new("同步地址不能为空"));
    }
    let normalized = if allow_bare_host && !trimmed.contains("://") {
        format!("https://{trimmed}")
    } else {
        trimmed.to_string()
    };
    let endpoint = reqwest::Url::parse(&normalized)
        .map_err(|error| ServerFnError::new(format!("同步地址无效: {error}")))?;
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err(ServerFnError::new("同步地址只支持 HTTP 或 HTTPS"));
    }
    if endpoint.host_str().is_none()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(ServerFnError::new(
            "同步地址必须包含主机，且不能包含凭据、查询参数或片段",
        ));
    }
    Ok(endpoint)
}

fn validate_remote_path(path: &str) -> Result<(), ServerFnError> {
    let path = path.trim_matches('/');
    if path.is_empty() {
        return Err(ServerFnError::new("远程文件路径不能为空"));
    }
    if path.len() > 1024
        || path.contains('\0')
        || path.contains('\\')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(ServerFnError::new("远程文件路径无效"));
    }
    Ok(())
}

pub(super) fn validate_snapshot(snapshot: &SyncSnapshot) -> Result<(), ServerFnError> {
    if snapshot.format != SYNC_SNAPSHOT_FORMAT {
        return Err(ServerFnError::new(format!(
            "不支持的同步快照格式: {}",
            snapshot.format
        )));
    }
    if snapshot.version != 1 {
        return Err(ServerFnError::new(format!(
            "不支持的同步快照版本: {}",
            snapshot.version
        )));
    }
    validate_custom_rules(&snapshot.custom_rules)
        .map_err(|error| ServerFnError::new(format!("远程自定义规则无效: {error}")))?;
    if snapshot.subscriptions.len() > 1024 {
        return Err(ServerFnError::new("远程订阅数量超过 1024 个限制"));
    }
    let mut subscription_ids = HashSet::with_capacity(snapshot.subscriptions.len());
    let mut node_tags = HashSet::new();
    for subscription in &snapshot.subscriptions {
        if subscription.id == 0 {
            return Err(ServerFnError::new("远程订阅 ID 不能为 0"));
        }
        if !subscription_ids.insert(subscription.id) {
            return Err(ServerFnError::new(format!(
                "远程订阅 ID 重复: {}",
                subscription.id
            )));
        }
        for node in &subscription.nodes {
            if node.tag.trim().is_empty() {
                return Err(ServerFnError::new("远程节点 tag 不能为空"));
            }
            if node.server.trim().is_empty() || node.port == 0 {
                return Err(ServerFnError::new(format!("远程节点不可用: {}", node.name)));
            }
            if !node_tags.insert(node.tag.as_str()) {
                return Err(ServerFnError::new(format!(
                    "远程节点 tag 重复: {}",
                    node.tag
                )));
            }
        }
    }
    if snapshot
        .active_subscription_id
        .is_some_and(|id| !subscription_ids.contains(&id))
    {
        return Err(ServerFnError::new("远程活动订阅 ID 不存在"));
    }
    Ok(())
}

pub(super) fn validate_remote_revision(revision: &str) -> Result<(), ServerFnError> {
    if revision.len() > 1024
        || revision
            .bytes()
            .any(|byte| byte.is_ascii_control() && byte != b'\t')
    {
        return Err(ServerFnError::new("远端同步 revision 无效"));
    }
    Ok(())
}

async fn download_snapshot(
    config: &SyncConfig,
) -> Result<Option<DownloadedSnapshot>, ServerFnError> {
    let response = match config.provider {
        SyncProviderKind::WebDav => send_webdav(config, Method::GET, None).await?,
        SyncProviderKind::S3 => {
            send_s3(
                config,
                Method::GET,
                Vec::new(),
                &UploadCondition::Unconditional,
            )
            .await?
        }
    };
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = require_success(response, "下载远程同步文件").await?;
    let remote_revision = response_revision(&response);
    if response
        .content_length()
        .is_some_and(|length| length > MAX_SYNC_BYTES as u64)
    {
        return Err(ServerFnError::new("远程同步文件超过 20 MiB 限制"));
    }
    let bytes = read_limited_body(response, MAX_SYNC_BYTES, "读取远程同步文件").await?;
    let snapshot = serde_json::from_slice(&bytes)
        .map_err(|error| ServerFnError::new(format!("远程同步文件无效: {error}")))?;
    validate_snapshot(&snapshot)?;
    Ok(Some(DownloadedSnapshot {
        snapshot,
        remote_revision,
    }))
}

async fn upload(
    config: &SyncConfig,
    payload: Vec<u8>,
    condition: UploadCondition,
) -> Result<String, ServerFnError> {
    let response = match config.provider {
        SyncProviderKind::WebDav => {
            ensure_webdav_collections(config).await?;
            send_webdav_with_condition(config, Method::PUT, Some(payload), &condition).await?
        }
        SyncProviderKind::S3 => send_s3(config, Method::PUT, payload, &condition).await?,
    };
    if response.status() == StatusCode::PRECONDITION_FAILED {
        return Err(ServerFnError::new(
            "远程同步文件在检查后发生变化，已取消上传，请先下载后重试",
        ));
    }
    let response = require_success(response, "上传远程同步文件").await?;
    Ok(response_revision(&response))
}

fn http_client() -> Result<reqwest::Client, ServerFnError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("kitty-pro-sync/1")
        .build()
        .map_err(|error| ServerFnError::new(format!("创建同步客户端失败: {error}")))
}

async fn send_webdav(
    config: &SyncConfig,
    method: Method,
    body: Option<Vec<u8>>,
) -> Result<Response, ServerFnError> {
    send_webdav_with_condition(config, method, body, &UploadCondition::Unconditional).await
}

async fn send_webdav_with_condition(
    config: &SyncConfig,
    method: Method,
    body: Option<Vec<u8>>,
    condition: &UploadCondition,
) -> Result<Response, ServerFnError> {
    let url = build_remote_url(&config.endpoint, None, &config.path)?;
    let mut request = http_client()?.request(method, url);
    if !config.username.trim().is_empty() {
        request = request.basic_auth(&config.username, Some(&config.password));
    }
    request = apply_upload_condition(request, condition);
    if let Some(body) = body {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
    }
    request
        .send()
        .await
        .map_err(|error| ServerFnError::new(format!("WebDAV 请求失败: {error}")))
}

async fn ensure_webdav_collections(config: &SyncConfig) -> Result<(), ServerFnError> {
    let segments = config.path.trim_matches('/').split('/').collect::<Vec<_>>();
    if segments.len() < 2 {
        return Ok(());
    }
    let client = http_client()?;
    let mkcol = Method::from_bytes(b"MKCOL").expect("MKCOL is a valid HTTP method");
    for index in 1..segments.len() {
        let path = segments[..index].join("/");
        let url = build_remote_url(&config.endpoint, None, &path)?;
        let mut request = client.request(mkcol.clone(), url);
        if !config.username.trim().is_empty() {
            request = request.basic_auth(&config.username, Some(&config.password));
        }
        let response = request
            .send()
            .await
            .map_err(|error| ServerFnError::new(format!("创建 WebDAV 目录失败: {error}")))?;
        if response.status().is_success()
            || matches!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED | StatusCode::CONFLICT
            )
        {
            continue;
        }
        require_success(response, "创建 WebDAV 目录").await?;
    }
    Ok(())
}

async fn send_s3(
    config: &SyncConfig,
    method: Method,
    body: Vec<u8>,
    condition: &UploadCondition,
) -> Result<Response, ServerFnError> {
    let url = build_s3_url(config)?;
    let payload_hash = sha256_hex(&body);
    let (amz_date, date_stamp) = aws_timestamp()?;
    let host = request_host(&url)?;
    let (canonical_headers, signed_headers) = if method == Method::PUT {
        (
            format!(
                "content-type:application/json\nhost:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n"
            ),
            "content-type;host;x-amz-content-sha256;x-amz-date",
        )
    } else {
        (
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n"),
            "host;x-amz-content-sha256;x-amz-date",
        )
    };
    let canonical_request = format!(
        "{}\n{}\n\n{}\n{}\n{}",
        method.as_str(),
        url.path(),
        canonical_headers,
        signed_headers,
        payload_hash
    );
    let scope = format!("{date_stamp}/{}/s3/aws4_request", config.region.trim());
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let date_key = hmac_sha256(
        format!("AWS4{}", config.secret_key).as_bytes(),
        date_stamp.as_bytes(),
    );
    let region_key = hmac_sha256(&date_key, config.region.trim().as_bytes());
    let service_key = hmac_sha256(&region_key, b"s3");
    let signing_key = hmac_sha256(&service_key, b"aws4_request");
    let signature = hex_encode(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        config.access_key.trim()
    );

    let mut request = http_client()?
        .request(method.clone(), url)
        .header("x-amz-content-sha256", payload_hash)
        .header("x-amz-date", amz_date)
        .header(reqwest::header::AUTHORIZATION, authorization);
    request = apply_upload_condition(request, condition);
    if method == Method::PUT {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
    }
    request
        .send()
        .await
        .map_err(|error| ServerFnError::new(format!("S3 请求失败: {error}")))
}

fn apply_upload_condition(
    request: reqwest::RequestBuilder,
    condition: &UploadCondition,
) -> reqwest::RequestBuilder {
    match condition {
        UploadCondition::Unconditional => request,
        UploadCondition::Match(revision) => request.header(IF_MATCH, revision),
        UploadCondition::Missing => request.header(IF_NONE_MATCH, "*"),
    }
}

fn response_revision(response: &Response) -> String {
    response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 1024)
        .unwrap_or_default()
        .to_string()
}

fn build_remote_url(
    endpoint: &str,
    bucket: Option<&str>,
    path: &str,
) -> Result<reqwest::Url, ServerFnError> {
    let mut url = reqwest::Url::parse(endpoint.trim())
        .map_err(|error| ServerFnError::new(format!("同步地址无效: {error}")))?;
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| ServerFnError::new("同步地址不能作为远程文件目录"))?;
        segments.pop_if_empty();
        if let Some(bucket) = bucket {
            segments.push(bucket.trim());
        }
        for segment in path.trim_matches('/').split('/') {
            segments.push(segment);
        }
    }
    Ok(url)
}

fn build_s3_url(config: &SyncConfig) -> Result<reqwest::Url, ServerFnError> {
    let endpoint = config.endpoint.trim();
    if endpoint.is_empty() || endpoint.contains("amazonaws.com") {
        if config.bucket.contains('.') {
            let base = format!("https://s3.{}.amazonaws.com", config.region.trim());
            return build_remote_url(&base, Some(&config.bucket), &config.path);
        }
        let base = format!(
            "https://{}.s3.{}.amazonaws.com",
            config.bucket.trim(),
            config.region.trim()
        );
        return build_remote_url(&base, None, &config.path);
    }
    let endpoint = parse_http_endpoint(endpoint, true)?;
    build_remote_url(endpoint.as_str(), Some(&config.bucket), &config.path)
}

fn request_host(url: &reqwest::Url) -> Result<String, ServerFnError> {
    let host = url
        .host()
        .ok_or_else(|| ServerFnError::new("S3 地址缺少主机"))?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

async fn require_success(response: Response, operation: &str) -> Result<Response, ServerFnError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let message = read_limited_body(response, 64 * 1024, "读取远程错误响应")
        .await
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default();
    let message = message.trim();
    let detail = if message.is_empty() {
        status.to_string()
    } else {
        format!(
            "{status}: {}",
            message.chars().take(512).collect::<String>()
        )
    };
    Err(ServerFnError::new(format!("{operation}失败: {detail}")))
}

async fn read_limited_body(
    mut response: Response,
    max_bytes: usize,
    operation: &str,
) -> Result<Vec<u8>, ServerFnError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| ServerFnError::new(format!("{operation}失败: {error}")))?
    {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ServerFnError::new(format!(
                "{operation}失败: 响应体超过 {} KiB 限制",
                max_bytes / 1024
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn sync_config_path() -> Result<PathBuf, ServerFnError> {
    profile_path()?
        .parent()
        .map(|directory| directory.join("sync.json"))
        .ok_or_else(|| ServerFnError::new("无法确定同步配置目录"))
}

fn write_private_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ServerFnError> {
    let _guard = sync_write_lock()
        .lock()
        .map_err(|_| ServerFnError::new("同步配置写入锁已损坏"))?;
    let parent = path
        .parent()
        .ok_or_else(|| ServerFnError::new("同步配置目录无效"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| ServerFnError::new(format!("创建同步配置目录失败: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| ServerFnError::new(format!("限制同步配置目录权限失败: {error}")))?;
    }
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(ServerFnError::new("同步配置路径必须是普通文件"));
        }
    }

    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| ServerFnError::new(format!("序列化同步配置失败: {error}")))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| ServerFnError::new(format!("写入同步配置失败: {error}")))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| ServerFnError::new(format!("写入同步配置失败: {error}")))?;
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| ServerFnError::new(format!("替换同步配置失败: {error}")))?;
    }
    std::fs::rename(&temporary, path)
        .map_err(|error| ServerFnError::new(format!("保存同步配置失败: {error}")))
}

fn sync_write_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn unix_timestamp() -> Result<u64, ServerFnError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| ServerFnError::new("系统时间早于 Unix 纪元，无法生成同步版本"))
}

fn aws_timestamp() -> Result<(String, String), ServerFnError> {
    let timestamp = unix_timestamp()?;
    let days = (timestamp / 86_400) as i64;
    let seconds = timestamp % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = seconds % 3_600 / 60;
    let second = seconds % 60;
    let date = format!("{year:04}{month:02}{day:02}");
    Ok((format!("{date}T{hour:02}{minute:02}{second:02}Z"), date))
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u64, u64) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u64, day as u64)
}

fn sha256_hex(input: &[u8]) -> String {
    hex_encode(&Sha256::digest(input))
}

fn hmac_sha256(key: &[u8], input: &[u8]) -> Vec<u8> {
    const BLOCK_SIZE: usize = 64;
    let mut normalized = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_SIZE];
    let mut outer_pad = [0x5c_u8; BLOCK_SIZE];
    for index in 0..BLOCK_SIZE {
        inner_pad[index] ^= normalized[index];
        outer_pad[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(input);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_hash);
    outer.finalize().to_vec()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use proxy_core::AppProfile;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    #[test]
    fn aws_timestamp_uses_utc_calendar_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_vector() {
        let key = [0x0b_u8; 20];
        assert_eq!(
            hex_encode(&hmac_sha256(&key, b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn remote_url_preserves_endpoint_prefix_and_encodes_segments() {
        let url = build_remote_url(
            "https://storage.example.com/root/",
            Some("my bucket"),
            "folder/kitty profile.json",
        )
        .expect("url should build");
        assert_eq!(
            url.as_str(),
            "https://storage.example.com/root/my%20bucket/folder/kitty%20profile.json"
        );
    }

    #[test]
    fn s3_url_uses_virtual_host_for_aws_and_path_style_for_custom_endpoints() {
        let mut config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::S3,
            endpoint: String::new(),
            path: "folder/profile.json".to_string(),
            bucket: "kitty-backup".to_string(),
            region: "ap-east-1".to_string(),
            access_key: "key".to_string(),
            secret_key: "secret".to_string(),
            ..SyncConfig::default()
        };
        assert_eq!(
            build_s3_url(&config)
                .expect("AWS URL should build")
                .as_str(),
            "https://kitty-backup.s3.ap-east-1.amazonaws.com/folder/profile.json"
        );

        config.bucket = "kitty.backup".to_string();
        assert_eq!(
            build_s3_url(&config)
                .expect("dotted AWS bucket URL should build")
                .as_str(),
            "https://s3.ap-east-1.amazonaws.com/kitty.backup/folder/profile.json"
        );

        config.bucket = "kitty-backup".to_string();
        config.endpoint = "http://minio.local:9000".to_string();
        assert_eq!(
            build_s3_url(&config)
                .expect("custom endpoint URL should build")
                .as_str(),
            "http://minio.local:9000/kitty-backup/folder/profile.json"
        );
    }

    #[test]
    fn snapshot_rejects_invalid_custom_rules() {
        let mut snapshot = SyncSnapshot::from_profile(&Default::default(), 0);
        snapshot.custom_rules = vec![proxy_core::CustomRule {
            id: 0,
            enabled: true,
            match_type: proxy_core::CustomRuleMatch::Domain,
            value: "example.com".to_string(),
            action: proxy_core::CustomRuleAction::Direct,
        }];
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[test]
    fn snapshot_rejects_duplicate_subscription_ids() {
        let subscription = proxy_core::Subscription {
            id: 7,
            name: "primary".to_string(),
            source: "https://example.com/subscription".to_string(),
            nodes: Vec::new(),
            proxy_server_nameservers: Vec::new(),
            rejected_count: 0,
        };
        let mut snapshot = SyncSnapshot::from_profile(&Default::default(), 0);
        snapshot.subscriptions = vec![subscription.clone(), subscription];
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[test]
    fn changing_remote_target_resets_sync_metadata() {
        let existing = SyncConfig {
            enabled: true,
            endpoint: "https://dav.example.com/user".to_string(),
            username: "alice".to_string(),
            password: "old-password".to_string(),
            last_sync_at: 42,
            last_remote_revision: "\"revision-1\"".to_string(),
            ..SyncConfig::default()
        };
        let mut password_change = existing.clone();
        password_change.password = "new-password".to_string();
        let preserved = merge_sync_metadata(&existing, &password_change);
        assert_eq!(preserved.last_sync_at, 42);
        assert_eq!(preserved.last_remote_revision, "\"revision-1\"");

        let mut target_change = password_change;
        target_change.path = "other-profile.json".to_string();
        let reset = merge_sync_metadata(&existing, &target_change);
        assert_eq!(reset.last_sync_at, 0);
        assert!(reset.last_remote_revision.is_empty());
    }

    #[tokio::test]
    async fn webdav_push_and_pull_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("listener should have address")
        );
        let stored = Arc::new(Mutex::new(Vec::<u8>::new()));
        let server_stored = Arc::clone(&stored);
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (stream, _) = listener.accept().expect("request should arrive");
                let (request, mut stream) = read_http_request(stream);
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: basic "));
                if request.starts_with("PUT ") {
                    assert!(request.to_ascii_lowercase().contains("if-none-match: *"));
                    let body = request_body(&request);
                    *server_stored.lock().expect("stored body lock") = body.to_vec();
                    write_http_response_with_headers(
                        &mut stream,
                        "201 Created",
                        "ETag: \"revision-1\"\r\n",
                        b"",
                    );
                } else if server_stored.lock().expect("stored body lock").is_empty() {
                    write_http_response(&mut stream, "404 Not Found", b"");
                } else {
                    let body = server_stored.lock().expect("stored body lock").clone();
                    write_http_response_with_headers(
                        &mut stream,
                        "200 OK",
                        "ETag: \"revision-1\"\r\n",
                        &body,
                    );
                }
            }
        });

        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::WebDav,
            endpoint,
            path: "kitty-profile.json".to_string(),
            username: "demo".to_string(),
            password: "secret".to_string(),
            ..SyncConfig::default()
        };
        let snapshot = SyncSnapshot::from_profile(&AppProfile::default(), 0);
        let result = push(&config, snapshot, false)
            .await
            .expect("WebDAV push should succeed");
        let pulled = pull(&config).await.expect("WebDAV pull should succeed");

        server.join().expect("server should exit");
        assert_eq!(pulled.snapshot.updated_at, result.updated_at);
        assert_eq!(pulled.snapshot.format, SYNC_SNAPSHOT_FORMAT);
        assert_eq!(result.remote_revision, "\"revision-1\"");
        assert_eq!(pulled.remote_revision, "\"revision-1\"");
        assert!(!stored.lock().expect("stored body lock").is_empty());
    }

    #[tokio::test]
    async fn conditional_upload_rejects_a_remote_change_after_get() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("listener should have address")
        );
        let remote = serde_json::to_vec(&SyncSnapshot::from_profile(&AppProfile::default(), 42))
            .expect("remote snapshot should serialize");
        let server = std::thread::spawn(move || {
            for request_index in 0..2 {
                let (stream, _) = listener.accept().expect("request should arrive");
                let (request, mut stream) = read_http_request(stream);
                if request_index == 0 {
                    assert!(request.starts_with("GET "));
                    write_http_response_with_headers(
                        &mut stream,
                        "200 OK",
                        "ETag: \"revision-1\"\r\n",
                        &remote,
                    );
                } else {
                    assert!(request.starts_with("PUT "));
                    assert!(request
                        .to_ascii_lowercase()
                        .contains("if-match: \"revision-1\""));
                    write_http_response(&mut stream, "412 Precondition Failed", b"");
                }
            }
        });
        let config = SyncConfig {
            enabled: true,
            endpoint,
            path: "kitty-profile.json".to_string(),
            last_sync_at: 42,
            last_remote_revision: "\"revision-1\"".to_string(),
            ..SyncConfig::default()
        };

        let error = push(
            &config,
            SyncSnapshot::from_profile(&AppProfile::default(), 42),
            false,
        )
        .await
        .expect_err("changed remote must reject conditional upload");

        server.join().expect("server should exit");
        assert!(error.to_string().contains("检查后发生变化"));
    }

    #[tokio::test]
    async fn s3_upload_uses_path_style_and_signed_headers_for_custom_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = format!(
            "http://{}",
            listener.local_addr().expect("listener should have address")
        );
        let (sender, receiver) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("request should arrive");
            let (request, mut stream) = read_http_request(stream);
            sender.send(request).expect("request should be captured");
            write_http_response_with_headers(
                &mut stream,
                "200 OK",
                "ETag: \"revision-1\"\r\n",
                b"",
            );
        });
        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::S3,
            endpoint,
            path: "sync/profile.json".to_string(),
            bucket: "kitty-backup".to_string(),
            region: "us-east-1".to_string(),
            access_key: "access".to_string(),
            secret_key: "secret".to_string(),
            ..SyncConfig::default()
        };

        push(
            &config,
            SyncSnapshot::from_profile(&AppProfile::default(), 0),
            true,
        )
        .await
        .expect("S3 upload should succeed");

        server.join().expect("server should exit");
        let request = receiver.recv().expect("request should be available");
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("PUT /kitty-backup/sync/profile.json "));
        assert!(lower.contains("authorization: aws4-hmac-sha256"));
        assert!(lower.contains("signedheaders=content-type;host;x-amz-content-sha256;x-amz-date"));
        assert!(lower.contains("x-amz-content-sha256:"));
    }

    fn read_http_request(mut stream: TcpStream) -> (String, TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout should set");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut expected = None;
        loop {
            let count = stream.read(&mut buffer).expect("request should read");
            assert!(count > 0, "request ended before its declared body");
            bytes.extend_from_slice(&buffer[..count]);
            if expected.is_none() {
                if let Some(header_end) = find_header_end(&bytes) {
                    let headers = String::from_utf8_lossy(&bytes[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':').and_then(|(name, value)| {
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                        })
                        .unwrap_or(0);
                    expected = Some(header_end + 4 + content_length);
                }
            }
            if expected.is_some_and(|expected| bytes.len() >= expected) {
                break;
            }
        }
        (
            String::from_utf8(bytes).expect("test request should be UTF-8"),
            stream,
        )
    }

    fn request_body(request: &str) -> &[u8] {
        request
            .as_bytes()
            .split(|byte| *byte == b'\r')
            .next_back()
            .unwrap_or_default()
            .strip_prefix(b"\n")
            .unwrap_or_default()
    }

    fn find_header_end(bytes: &[u8]) -> Option<usize> {
        bytes.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn write_http_response(stream: &mut TcpStream, status: &str, body: &[u8]) {
        write_http_response_with_headers(stream, status, "", body);
    }

    fn write_http_response_with_headers(
        stream: &mut TcpStream,
        status: &str,
        extra_headers: &str,
        body: &[u8],
    ) {
        let headers = format!(
            "HTTP/1.1 {status}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(headers.as_bytes())
            .and_then(|_| stream.write_all(body))
            .expect("response should write");
    }
}
