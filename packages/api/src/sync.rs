use super::{profile_path, SyncConfig, SyncProviderKind, SyncResult};
use dioxus::prelude::ServerFnError;
use proxy_core::{
    normalize_dns_override, sync_bundle_directory as bundle_directory, validate_custom_rules,
    CustomRule, DnsOverride, SyncConfigScript, SyncSnapshot, SyncSubscription,
    SYNC_SNAPSHOT_FORMAT, SYNC_SNAPSHOT_VERSION,
};
use reqwest::header::{ETAG, IF_MATCH, IF_NONE_MATCH};
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_SYNC_BYTES: usize = 20 * 1024 * 1024;
const LEGACY_SYNC_SNAPSHOT_VERSION: u32 = 1;
const MAX_CONFIG_SCRIPT_BYTES: usize = 512 * 1024;

#[derive(Deserialize)]
struct SyncSnapshotEnvelope {
    format: String,
    version: u32,
}

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

/// 回显给前端的凭据掩码。前端把原样带回的掩码在保存时还原为已存
/// 真值，凭据因此永远不离开服务端——LAN 模式是纯 HTTP，响应体在局域网
/// 内可被被动抓包，密码与 S3 SecretKey 不能原样下发。
pub(super) const SECRET_PLACEHOLDER: &str = "••••••••";

pub(super) fn load_native_sync_config() -> Result<SyncConfig, ServerFnError> {
    let path = sync_config_path()?;
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| ServerFnError::new(format!("同步配置损坏: {error}")))?;
            migrate_legacy_endpoint(&mut value);
            serde_json::from_value(value)
                .map_err(|error| ServerFnError::new(format!("同步配置损坏: {error}")))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SyncConfig::default()),
        Err(error) => Err(ServerFnError::new(format!("读取同步配置失败: {error}"))),
    }
}

/// Older builds stored one shared `endpoint` for both providers. Route it to the
/// field belonging to the saved provider so the other provider starts blank and
/// an address never leaks into a form the user has not filled in.
fn migrate_legacy_endpoint(value: &mut serde_json::Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let Some(legacy) = map.remove("endpoint") else {
        return;
    };
    let endpoint = legacy.as_str().unwrap_or_default().trim().to_string();
    if endpoint.is_empty() {
        return;
    }
    // The stored provider is still in `map`; WebDAV is the serde default.
    let key = if map.get("provider").and_then(|v| v.as_str()) == Some("s3") {
        "s3_endpoint"
    } else {
        "webdav_endpoint"
    };
    if map
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
    {
        map.insert(key.to_string(), serde_json::Value::String(endpoint));
    }
}

/// 把非空的密码/SecretKey 替换为掩码，供任何要下发到前端的副本使用。
pub(super) fn masked_for_client(config: &SyncConfig) -> SyncConfig {
    let mut masked = config.clone();
    if !masked.password.is_empty() {
        masked.password = SECRET_PLACEHOLDER.to_string();
    }
    if !masked.secret_key.is_empty() {
        masked.secret_key = SECRET_PLACEHOLDER.to_string();
    }
    masked
}

/// 前端提交的掩码字段还原为磁盘上的真值；不含掩码时不读磁盘。
fn resolve_secrets(config: &SyncConfig) -> Result<SyncConfig, ServerFnError> {
    if config.password != SECRET_PLACEHOLDER && config.secret_key != SECRET_PLACEHOLDER {
        return Ok(config.clone());
    }
    let existing = load_native_sync_config()?;
    let mut resolved = config.clone();
    if resolved.password == SECRET_PLACEHOLDER {
        resolved.password = existing.password;
    }
    if resolved.secret_key == SECRET_PLACEHOLDER {
        resolved.secret_key = existing.secret_key;
    }
    Ok(resolved)
}

pub(super) fn save_native_sync_config(config: &SyncConfig) -> Result<SyncConfig, ServerFnError> {
    let config = resolve_secrets(config)?;
    let existing = load_native_sync_config()?;
    let saved = merge_sync_metadata(&existing, &config);
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

/// What a sync target currently holds, read without pulling the content itself.
enum RemoteState {
    Absent,
    /// The pre-bundle single document at exactly `config.path`.
    Legacy {
        updated_at: u64,
        revision: String,
    },
    Bundle {
        updated_at: u64,
        revision: String,
        /// Section object names the remote already stores. Content addressing
        /// means a matching name is a matching body, so those can be skipped.
        objects: HashSet<String>,
    },
}

/// The three conditions that make a remote safe to write over: this device owns
/// a baseline for it, the store hands out revisions to compare, and that revision
/// has not moved since. Anything else needs an explicit overwrite.
fn require_baseline(config: &SyncConfig, revision: &str) -> Result<(), ServerFnError> {
    if config.last_sync_at == 0 && config.last_remote_revision.is_empty() {
        return Err(ServerFnError::new(
            "远程同步文件已存在，但本机没有该目标的同步基线，请先下载或明确选择覆盖上传",
        ));
    }
    if revision.is_empty() {
        return Err(ServerFnError::new(
            "远端存储未提供 ETag，无法保证安全上传，请使用明确覆盖上传",
        ));
    }
    if !config.last_remote_revision.is_empty() && revision != config.last_remote_revision {
        return Err(ServerFnError::new(
            "远程同步文件已被其他设备修改，请先下载或明确选择覆盖上传",
        ));
    }
    Ok(())
}

/// Refuse to overwrite a remote that another device has updated since this
/// device last synced, whether it now holds a bundle or a legacy document.
fn require_not_newer(config: &SyncConfig, updated_at: u64) -> Result<(), ServerFnError> {
    if updated_at > config.last_sync_at {
        return Err(ServerFnError::new(format!(
            "远程配置更新于本机上次同步之后（远程版本 {updated_at}），请先下载或明确选择覆盖上传",
        )));
    }
    Ok(())
}

/// Read enough about the remote to decide a conditional write. Deliberately
/// fetches only the manifest: `push` needs freshness, not another copy of data
/// it already holds, and a bundle can be several objects.
async fn remote_state(config: &SyncConfig) -> Result<RemoteState, ServerFnError> {
    if let Some((manifest, revision)) = download_manifest(config).await? {
        return Ok(RemoteState::Bundle {
            updated_at: manifest.updated_at,
            revision,
            objects: manifest.files.into_iter().map(|file| file.name).collect(),
        });
    }
    if let Some(legacy) = download_legacy_snapshot(config).await? {
        return Ok(RemoteState::Legacy {
            updated_at: legacy.snapshot.updated_at,
            revision: legacy.remote_revision,
        });
    }
    Ok(RemoteState::Absent)
}

pub(super) async fn push(
    config: &SyncConfig,
    mut snapshot: SyncSnapshot,
    force: bool,
) -> Result<SyncResult, ServerFnError> {
    validate_config(config)?;
    validate_snapshot(&snapshot)?;
    snapshot.version = SYNC_SNAPSHOT_VERSION;

    let mut condition = UploadCondition::Unconditional;
    let mut known_objects = HashSet::new();
    if !force {
        match remote_state(config).await? {
            RemoteState::Absent
                if config.last_sync_at > 0 || !config.last_remote_revision.is_empty() =>
            {
                return Err(ServerFnError::new(
                    "远程同步文件已在本机上次同步后被删除，请明确选择覆盖上传以重新创建",
                ));
            }
            RemoteState::Absent => condition = UploadCondition::Missing,
            RemoteState::Legacy {
                updated_at,
                revision,
            } => {
                require_baseline(config, &revision)?;
                require_not_newer(config, updated_at)?;
                // A legacy target has no manifest object to guard with an ETag,
                // so the first bundle write asserts the manifest is absent: if an
                // old build re-published the single document in the meantime, this
                // create fails instead of silently shadowing it.
                condition = UploadCondition::Missing;
            }
            RemoteState::Bundle {
                updated_at,
                revision,
                objects,
            } => {
                require_baseline(config, &revision)?;
                require_not_newer(config, updated_at)?;
                condition = UploadCondition::Match(revision);
                known_objects = objects;
            }
        }
    }

    let now = unix_timestamp()?;
    snapshot.updated_at = now.max(config.last_sync_at.saturating_add(1));
    let mut remote_revision = upload_bundle(config, &snapshot, &condition, &known_objects).await?;
    if remote_revision.is_empty() {
        // Some stores answer a PUT without an ETag. Re-read the manifest to see
        // whether the revision we just wrote is actually there; the manifest is
        // byte-identical when the store preserved our payload, and only the
        // manifest is fetched, never the sections.
        if let Ok(Some((manifest, revision))) = download_manifest(config).await {
            if manifest.updated_at == snapshot.updated_at {
                remote_revision = revision;
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
        dns_rule_count: snapshot.dns_overrides.as_ref().map(Vec::len),
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
        || left.path.trim_matches('/') != right.path.trim_matches('/')
    {
        return false;
    }
    match left.provider {
        SyncProviderKind::WebDav => {
            left.webdav_endpoint.trim() == right.webdav_endpoint.trim()
                && left.username.trim() == right.username.trim()
        }
        SyncProviderKind::S3 => {
            left.s3_endpoint.trim() == right.s3_endpoint.trim()
                && left.bucket.trim() == right.bucket.trim()
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
            parse_http_endpoint(&config.webdav_endpoint, false)?;
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
            if !config.s3_endpoint.trim().is_empty() {
                parse_http_endpoint(&config.s3_endpoint, true)?;
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
    if !matches!(
        snapshot.version,
        LEGACY_SYNC_SNAPSHOT_VERSION | SYNC_SNAPSHOT_VERSION
    ) {
        return Err(ServerFnError::new(format!(
            "不支持的同步快照版本: {}",
            snapshot.version
        )));
    }
    validate_custom_rules(&snapshot.custom_rules)
        .map_err(|error| ServerFnError::new(format!("远程自定义规则无效: {error}")))?;
    // Absent sections are not validated: nothing to check, and applying such a
    // document leaves the matching local state untouched.
    let dns_overrides = snapshot.dns_overrides.clone().unwrap_or_default();
    if dns_overrides.len() > 256 {
        return Err(ServerFnError::new("远程专用 DNS 规则数量超过 256 条限制"));
    }
    let mut dns_rule_suffixes = HashSet::with_capacity(dns_overrides.len());
    for rule in &dns_overrides {
        let (domain_suffix, server) = normalize_dns_override(&rule.domain_suffix, &rule.server)
            .map_err(|error| ServerFnError::new(format!("远程专用 DNS 规则无效: {error}")))?;
        if !dns_rule_suffixes.insert(domain_suffix.clone()) {
            return Err(ServerFnError::new(format!(
                "远程专用 DNS 域名后缀重复: {domain_suffix}"
            )));
        }
        if rule.server != server {
            return Err(ServerFnError::new("远程专用 DNS 服务器与规范化结果不一致"));
        }
    }
    if snapshot
        .config_script
        .as_ref()
        .is_some_and(|script| script.script.len() > MAX_CONFIG_SCRIPT_BYTES)
    {
        return Err(ServerFnError::new(format!(
            "远程配置脚本超过 {} KiB 限制",
            MAX_CONFIG_SCRIPT_BYTES / 1024
        )));
    }
    if snapshot.subscriptions.len() > 1024 {
        return Err(ServerFnError::new("远程订阅数量超过 1024 个限制"));
    }
    let mut subscription_ids = HashSet::with_capacity(snapshot.subscriptions.len());
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
        if subscription.source.trim().is_empty() {
            return Err(ServerFnError::new(format!(
                "远程订阅来源为空: {}",
                subscription.name
            )));
        }
    }
    Ok(())
}

fn decode_snapshot(bytes: &[u8]) -> Result<SyncSnapshot, ServerFnError> {
    let envelope: SyncSnapshotEnvelope = serde_json::from_slice(bytes)
        .map_err(|error| ServerFnError::new(format!("远程同步文件无效: {error}")))?;
    if envelope.format != SYNC_SNAPSHOT_FORMAT {
        return Err(ServerFnError::new(format!(
            "不支持的同步快照格式: {}",
            envelope.format
        )));
    }
    if !matches!(
        envelope.version,
        LEGACY_SYNC_SNAPSHOT_VERSION | SYNC_SNAPSHOT_VERSION
    ) {
        return Err(ServerFnError::new(format!(
            "不支持的同步快照版本: {}",
            envelope.version
        )));
    }

    let snapshot = serde_json::from_slice(bytes)
        .map_err(|error| ServerFnError::new(format!("远程同步文件无效: {error}")))?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
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

/// A sync target is a folder of small documents plus a manifest that names them.
///
/// One JSON blob per device was the original layout and it has two problems: any
/// new section rewrites the whole object, and a half-finished upload leaves a
/// corrupt file that every device then downloads. Here each section is its own
/// object, and `manifest.json` is written last, so a bundle is visible to other
/// devices exactly when it is complete. Objects are content-addressed
/// (`dns-overrides-<hash>.json`), so publishing a new revision never overwrites
/// files an older manifest still points at.
const SYNC_BUNDLE_FORMAT: &str = "kitty-pro-sync-bundle";
const SYNC_BUNDLE_VERSION: u32 = 1;
const MANIFEST_NAME: &str = "manifest.json";
/// Sections this build knows how to place in a snapshot. A manifest naming
/// anything else is rejected rather than ignored: silently dropping a section
/// this version does not understand would delete it on the next push.
const BUNDLE_SECTIONS: [&str; 4] = [
    "subscriptions",
    "custom-rules",
    "dns-overrides",
    "config-script",
];
/// Far above the four sections that exist, but finite: the manifest is remote
/// input, and each entry it names is a request this client would make.
const MAX_BUNDLE_FILES: usize = 16;

#[derive(Serialize, Deserialize)]
struct BundleManifest {
    format: String,
    version: u32,
    updated_at: u64,
    #[serde(default)]
    files: Vec<BundleFile>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
struct BundleFile {
    section: String,
    name: String,
    size: u64,
    sha256: String,
}

fn bundle_path(config: &SyncConfig, name: &str) -> String {
    format!("{}/{}", bundle_directory(&config.path), name)
}

/// Section name plus a content hash, so identical content across pushes reuses
/// one object and different content never collides.
fn bundle_object_name(section: &str, payload: &[u8]) -> String {
    format!("{section}-{}.json", &sha256_hex(payload)[..16])
}

/// The sections a snapshot carries, in a fixed order. A `None` section writes no
/// object at all, which is how "this device never had the feature" stays
/// distinguishable from "this device has zero rules".
fn bundle_sections(snapshot: &SyncSnapshot) -> Result<Vec<(String, Vec<u8>)>, ServerFnError> {
    let mut sections = Vec::new();
    sections.push((
        "subscriptions".to_string(),
        json_payload(&snapshot.subscriptions)?,
    ));
    sections.push((
        "custom-rules".to_string(),
        json_payload(&snapshot.custom_rules)?,
    ));
    if let Some(dns_overrides) = &snapshot.dns_overrides {
        sections.push(("dns-overrides".to_string(), json_payload(dns_overrides)?));
    }
    if let Some(config_script) = &snapshot.config_script {
        sections.push(("config-script".to_string(), json_payload(config_script)?));
    }
    Ok(sections)
}

fn json_payload<T: Serialize>(value: &T) -> Result<Vec<u8>, ServerFnError> {
    serde_json::to_vec_pretty(value)
        .map_err(|error| ServerFnError::new(format!("序列化同步内容失败: {error}")))
}

/// Pull a bundle, falling back to the legacy single document.
async fn download_snapshot(
    config: &SyncConfig,
) -> Result<Option<DownloadedSnapshot>, ServerFnError> {
    match download_manifest(config).await? {
        Some(manifest) => download_bundle(config, manifest).await.map(Some),
        None => download_legacy_snapshot(config).await,
    }
}

async fn download_manifest(
    config: &SyncConfig,
) -> Result<Option<(BundleManifest, String)>, ServerFnError> {
    let path = bundle_path(config, MANIFEST_NAME);
    let response = match config.provider {
        SyncProviderKind::WebDav => {
            send_webdav(
                config,
                &path,
                Method::GET,
                None,
                &UploadCondition::Unconditional,
            )
            .await?
        }
        SyncProviderKind::S3 => {
            send_s3(
                config,
                &path,
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
    let response = require_success(response, "读取远程同步清单").await?;
    let revision = response_revision(&response);
    let bytes = read_limited_body(response, MAX_SYNC_BYTES, "读取远程同步清单").await?;
    let manifest: BundleManifest = serde_json::from_slice(&bytes)
        .map_err(|error| ServerFnError::new(format!("远程同步清单无效: {error}")))?;
    if manifest.format != SYNC_BUNDLE_FORMAT || manifest.version != SYNC_BUNDLE_VERSION {
        return Err(ServerFnError::new(format!(
            "不支持的远程同步清单: {} v{}",
            manifest.format, manifest.version
        )));
    }
    validate_manifest_shape(&manifest)?;
    Ok(Some((manifest, revision)))
}

/// Structural checks on a manifest, run before anything it names is fetched.
///
/// The manifest is the only trusted index in a bundle: every object it points at
/// is verified by size and hash afterwards, but the list itself has to hold up
/// on its own, or a corrupt or hostile one turns a pull into an unbounded loop of
/// requests and lets two entries claim the same section.
fn validate_manifest_shape(manifest: &BundleManifest) -> Result<(), ServerFnError> {
    if manifest.files.len() > MAX_BUNDLE_FILES {
        return Err(ServerFnError::new(format!(
            "远程同步清单条目过多（{} 项）",
            manifest.files.len()
        )));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    for file in &manifest.files {
        if !BUNDLE_SECTIONS.contains(&file.section.as_str()) {
            return Err(ServerFnError::new(format!(
                "远程同步清单包含未知内容: {}",
                file.section
            )));
        }
        if !seen.insert(file.section.as_str()) {
            return Err(ServerFnError::new(format!(
                "远程同步清单内容重复: {}",
                file.section
            )));
        }
        validate_bundle_file_name(&file.name)?;
        // The object name embeds the digest this client would verify against, so
        // a mismatch is a corrupt manifest rather than a renamed object.
        if !file.name.starts_with(&file.section)
            || file.sha256.len() != 64
            || !file.sha256.chars().all(|char| char.is_ascii_hexdigit())
        {
            return Err(ServerFnError::new(format!(
                "远程同步清单文件 {} 与内容 {section} 不匹配",
                file.name,
                section = file.section
            )));
        }
    }
    Ok(())
}
async fn download_bundle(
    config: &SyncConfig,
    manifest: (BundleManifest, String),
) -> Result<DownloadedSnapshot, ServerFnError> {
    let (manifest, revision) = manifest;
    let mut payloads: HashMap<String, Vec<u8>> = HashMap::new();
    let mut total = 0_usize;
    for file in &manifest.files {
        let bytes = download_object(config, &bundle_path(config, &file.name)).await?;
        total = total.saturating_add(bytes.len());
        if total > MAX_SYNC_BYTES {
            return Err(ServerFnError::new("远程同步内容超过 20 MiB 限制"));
        }
        if bytes.len() as u64 != file.size {
            return Err(ServerFnError::new(format!(
                "远程同步文件 {} 大小不符（清单 {}，实际 {}）",
                file.section,
                file.size,
                bytes.len()
            )));
        }
        let digest = sha256_hex(&bytes);
        if digest != file.sha256 {
            return Err(ServerFnError::new(format!(
                "远程同步文件 {} 校验失败，请重新上传",
                file.section
            )));
        }
        payloads.insert(file.section.clone(), bytes);
    }

    let snapshot = SyncSnapshot {
        format: SYNC_SNAPSHOT_FORMAT.to_string(),
        version: SYNC_SNAPSHOT_VERSION,
        updated_at: manifest.updated_at,
        subscriptions: take_section(&mut payloads, "subscriptions")?
            .map(deserialize_section::<Vec<SyncSubscription>>)
            .transpose()?
            .unwrap_or_default(),
        custom_rules: take_section(&mut payloads, "custom-rules")?
            .map(deserialize_section::<Vec<CustomRule>>)
            .transpose()?
            .unwrap_or_default(),
        dns_overrides: take_section(&mut payloads, "dns-overrides")?
            .map(deserialize_section::<Vec<DnsOverride>>)
            .transpose()?,
        config_script: take_section(&mut payloads, "config-script")?
            .map(deserialize_section::<SyncConfigScript>)
            .transpose()?,
    };
    validate_snapshot(&snapshot)?;
    Ok(DownloadedSnapshot {
        snapshot,
        remote_revision: revision,
    })
}

fn take_section(
    payloads: &mut HashMap<String, Vec<u8>>,
    section: &str,
) -> Result<Option<Vec<u8>>, ServerFnError> {
    payloads
        .remove(section)
        .map(|payload| {
            if payload.len() > MAX_SYNC_BYTES {
                return Err(ServerFnError::new(format!("远程同步文件 {section} 过大")));
            }
            Ok(payload)
        })
        .transpose()
}

fn deserialize_section<T: serde::de::DeserializeOwned>(bytes: Vec<u8>) -> Result<T, ServerFnError> {
    serde_json::from_slice(&bytes)
        .map_err(|error| ServerFnError::new(format!("远程同步文件无效: {error}")))
}

fn validate_bundle_file_name(name: &str) -> Result<(), ServerFnError> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && name.ends_with(".json")
        && name.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || "-_.".contains(character)
        })
        && !name.starts_with('.')
        && !name.contains("..");
    if ok {
        Ok(())
    } else {
        Err(ServerFnError::new(format!("远程同步文件名无效: {name}")))
    }
}

async fn download_object(config: &SyncConfig, path: &str) -> Result<Vec<u8>, ServerFnError> {
    let response = match config.provider {
        SyncProviderKind::WebDav => {
            send_webdav(
                config,
                path,
                Method::GET,
                None,
                &UploadCondition::Unconditional,
            )
            .await?
        }
        SyncProviderKind::S3 => {
            send_s3(
                config,
                path,
                Method::GET,
                Vec::new(),
                &UploadCondition::Unconditional,
            )
            .await?
        }
    };
    if response.status() == StatusCode::NOT_FOUND {
        return Err(ServerFnError::new(format!(
            "远程同步文件缺失: {path}，请重新上传"
        )));
    }
    let response = require_success(response, "下载远程同步文件").await?;
    read_limited_body(response, MAX_SYNC_BYTES, "下载远程同步文件").await
}

/// The pre-bundle layout: one JSON document at exactly the configured path.
/// Still read so a remote nobody has re-uploaded since keeps working; pushes
/// always write a bundle.
async fn download_legacy_snapshot(
    config: &SyncConfig,
) -> Result<Option<DownloadedSnapshot>, ServerFnError> {
    let response = match config.provider {
        SyncProviderKind::WebDav => {
            send_webdav(
                config,
                &config.path,
                Method::GET,
                None,
                &UploadCondition::Unconditional,
            )
            .await?
        }
        SyncProviderKind::S3 => {
            send_s3(
                config,
                &config.path,
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
    let snapshot = decode_snapshot(&bytes)?;
    Ok(Some(DownloadedSnapshot {
        snapshot,
        remote_revision,
    }))
}

async fn upload_object(
    config: &SyncConfig,
    path: &str,
    payload: Vec<u8>,
    condition: &UploadCondition,
) -> Result<String, ServerFnError> {
    let response = match config.provider {
        SyncProviderKind::WebDav => {
            ensure_webdav_collections(config, path).await?;
            send_webdav(config, path, Method::PUT, Some(payload), condition).await?
        }
        SyncProviderKind::S3 => send_s3(config, path, Method::PUT, payload, condition).await?,
    };
    if response.status() == StatusCode::PRECONDITION_FAILED {
        return Err(ServerFnError::new(
            "远程同步文件在检查后发生变化，已取消上传，请先下载后重试",
        ));
    }
    let response = require_success(response, "上传远程同步文件").await?;
    Ok(response_revision(&response))
}

/// Write a bundle: every section object first, the manifest last.
///
/// Section names carry a content hash, so a push never rewrites a file an older
/// manifest still points at, and a reader can only ever see a complete set: the
/// manifest it validates against appears after all of its objects. Objects a
/// previous revision owned are left in place rather than deleted, because
/// another device may be downloading exactly that revision; expire them with a
/// bucket lifecycle rule.
///
/// `known_objects` names the section objects the remote already stores, so an
/// unchanged section costs no request: editing one DNS rule re-uploads that one
/// object and the manifest, not the whole profile.
async fn upload_bundle(
    config: &SyncConfig,
    snapshot: &SyncSnapshot,
    condition: &UploadCondition,
    known_objects: &HashSet<String>,
) -> Result<String, ServerFnError> {
    let sections = bundle_sections(snapshot)?;
    let mut files = Vec::with_capacity(sections.len());
    let mut total = 0_usize;
    for (section, payload) in sections {
        total = total.saturating_add(payload.len());
        if total > MAX_SYNC_BYTES {
            return Err(ServerFnError::new("同步内容超过 20 MiB 限制"));
        }
        let name = bundle_object_name(&section, &payload);
        // Content-addressed name, so "already stored under this name" means
        // "already stores exactly these bytes".
        if !known_objects.contains(&name) {
            let path = bundle_path(config, &name);
            upload_object(
                config,
                &path,
                payload.clone(),
                &UploadCondition::Unconditional,
            )
            .await?;
        }
        files.push(BundleFile {
            section,
            size: payload.len() as u64,
            sha256: sha256_hex(&payload),
            name,
        });
    }

    let manifest = BundleManifest {
        format: SYNC_BUNDLE_FORMAT.to_string(),
        version: SYNC_BUNDLE_VERSION,
        updated_at: snapshot.updated_at,
        files,
    };
    let payload = json_payload(&manifest)?;
    upload_object(
        config,
        &bundle_path(config, MANIFEST_NAME),
        payload,
        condition,
    )
    .await
}

fn http_client() -> Result<reqwest::Client, ServerFnError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("kitty-pro-sync/2")
        .build()
        .map_err(|error| ServerFnError::new(format!("创建同步客户端失败: {error}")))
}

async fn send_webdav(
    config: &SyncConfig,
    path: &str,
    method: Method,
    body: Option<Vec<u8>>,
    condition: &UploadCondition,
) -> Result<Response, ServerFnError> {
    let url = build_remote_url(&config.webdav_endpoint, None, path)?;
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

/// Create the directories above an object. A single-file path needs its
/// parents; a bundle also needs its own directory, which is why the object name
/// is excluded rather than every segment but the last two.
async fn ensure_webdav_collections(config: &SyncConfig, path: &str) -> Result<(), ServerFnError> {
    let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    if segments.len() < 2 {
        return Ok(());
    }
    let client = http_client()?;
    let mkcol = Method::from_bytes(b"MKCOL").expect("MKCOL is a valid HTTP method");
    for index in 1..segments.len() {
        let collection = segments[..index].join("/");
        let url = build_remote_url(&config.webdav_endpoint, None, &collection)?;
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
    path: &str,
    method: Method,
    body: Vec<u8>,
    condition: &UploadCondition,
) -> Result<Response, ServerFnError> {
    let url = build_s3_url(config, path)?;
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

fn build_s3_url(config: &SyncConfig, path: &str) -> Result<reqwest::Url, ServerFnError> {
    let endpoint = config.s3_endpoint.trim();
    if endpoint.is_empty() || endpoint.contains("amazonaws.com") {
        if config.bucket.contains('.') {
            let base = format!("https://s3.{}.amazonaws.com", config.region.trim());
            return build_remote_url(&base, Some(&config.bucket), path);
        }
        let base = format!(
            "https://{}.s3.{}.amazonaws.com",
            config.bucket.trim(),
            config.region.trim()
        );
        return build_remote_url(&base, None, path);
    }
    let endpoint = parse_http_endpoint(endpoint, true)?;
    build_remote_url(endpoint.as_str(), Some(&config.bucket), path)
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// A tiny path-keyed HTTP store. The bundle layout issues several requests per
    /// push, which the earlier "expect exactly N requests" servers cannot express,
    /// so this serves GET/PUT/MKCOL on any path, honours `If-Match` and
    /// `If-None-Match: *`, and records the order writes arrived in.
    /// One stored object: its ETag and body, keyed by request target. Spelled
    /// once because the fake remote threads it through four signatures.
    type StoredFile = (String, Vec<u8>);

    struct FakeRemote {
        endpoint: String,
        files: Arc<Mutex<HashMap<String, StoredFile>>>,
        writes: Arc<Mutex<Vec<String>>>,
        requests: Arc<Mutex<Vec<String>>>,
        reject_next_put: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
    }

    impl FakeRemote {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("fake remote should bind");
            let address = listener.local_addr().expect("fake remote address");
            listener
                .set_nonblocking(true)
                .expect("fake remote should be nonblocking");
            let files = Arc::new(Mutex::new(HashMap::new()));
            let writes = Arc::new(Mutex::new(Vec::new()));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let reject_next_put = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let (server_files, server_writes, server_requests, server_reject, server_stop) = (
                Arc::clone(&files),
                Arc::clone(&writes),
                Arc::clone(&requests),
                Arc::clone(&reject_next_put),
                Arc::clone(&stop),
            );

            std::thread::spawn(move || {
                while !server_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (files, writes, requests, reject) = (
                                Arc::clone(&server_files),
                                Arc::clone(&server_writes),
                                Arc::clone(&server_requests),
                                Arc::clone(&server_reject),
                            );
                            std::thread::spawn(move || {
                                let (request, mut stream) = read_http_request(stream);
                                // Only the request head is kept; bodies are stored
                                // in `files`, and nothing here needs both.
                                let head = request
                                    .split_once("\r\n\r\n")
                                    .map_or_else(|| request.clone(), |(head, _)| head.to_string());
                                lock_shared(&requests).push(head);
                                serve_fake_request(&request, &files, &writes, &reject, &mut stream);
                            });
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                endpoint: format!("http://{address}"),
                files,
                writes,
                requests,
                reject_next_put,
                stop,
            }
        }

        /// Make the next PUT fail with 412, to exercise the window between the
        /// precondition check in `push` and the write itself.
        fn reject_next_put(&self) {
            self.reject_next_put.store(true, Ordering::SeqCst);
        }

        /// Seed an object, bypassing the wire, to stage a remote revision.
        fn put(&self, path: &str, body: Vec<u8>) {
            lock_shared(&self.files).insert(
                format!("/{path}"),
                ("\"revision-remote\"".to_string(), body),
            );
        }

        fn paths(&self) -> Vec<String> {
            let mut keys = lock_shared(&self.files).keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys
        }

        fn writes(&self) -> Vec<String> {
            lock_shared(&self.writes).clone()
        }

        /// Every request head the server saw, in arrival order.
        fn requests(&self) -> Vec<String> {
            lock_shared(&self.requests).clone()
        }

        fn body(&self, path: &str) -> Option<Vec<u8>> {
            lock_shared(&self.files)
                .get(path)
                .map(|entry| entry.1.clone())
        }
    }

    impl Drop for FakeRemote {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }

    /// `Mutex::lock` across the helper methods: a poisoned test lock is fine to
    /// read, since the test itself is the only writer and it cannot be mid-write.
    fn lock_shared<T>(value: &Arc<Mutex<T>>) -> std::sync::MutexGuard<'_, T> {
        value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn serve_fake_request(
        request: &str,
        files: &Arc<Mutex<HashMap<String, StoredFile>>>,
        writes: &Arc<Mutex<Vec<String>>>,
        reject_next_put: &Arc<AtomicBool>,
        stream: &mut TcpStream,
    ) {
        let mut words = request.split_whitespace();
        let (Some(method), Some(target)) = (words.next(), words.next()) else {
            return;
        };
        let lowered = request.to_ascii_lowercase();
        let existing = files
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(target)
            .cloned();

        match method {
            "MKCOL" => write_fake_response(stream, "201 Created\r\n", b""),
            "GET" | "HEAD" => match existing {
                Some((etag, body)) => write_fake_response(
                    stream,
                    &format!("200 OK\r\nETag: {etag}\r\nContent-Type: application/json\r\n"),
                    &body,
                ),
                None => write_fake_response(stream, "404 Not Found\r\n", b""),
            },
            "PUT" => {
                if reject_next_put.swap(false, Ordering::SeqCst) {
                    write_fake_response(stream, "412 Precondition Failed\r\n", b"");
                    return;
                }
                if lowered.contains("if-none-match: *") && existing.is_some() {
                    write_fake_response(stream, "412 Precondition Failed\r\n", b"");
                    return;
                }
                if let Some(expected) = header_value(&lowered, "if-match") {
                    match &existing {
                        Some((etag, _)) if *etag == expected => {}
                        _ => {
                            write_fake_response(stream, "412 Precondition Failed\r\n", b"");
                            return;
                        }
                    }
                }
                let body = request_body(request).to_vec();
                writes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(target.to_string());
                files
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(
                        target.to_string(),
                        ("\"revision-remote\"".to_string(), body),
                    );
                write_fake_response(stream, "201 Created\r\nETag: \"revision-remote\"\r\n", b"");
            }
            other => write_fake_response(stream, &format!("405 {other} Not Allowed\r\n"), b""),
        }
    }

    fn header_value(lowered_request: &str, name: &str) -> Option<String> {
        lowered_request.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }

    /// Every reply closes the connection, so a client that pipelines or reuses
    /// keeps working against a one-request-per-connection server.
    fn write_fake_response(stream: &mut TcpStream, status_and_headers: &str, body: &[u8]) {
        let response = format!(
            "HTTP/1.1 {status_and_headers}Connection: close\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    #[test]
    fn aws_timestamp_uses_utc_calendar_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
    }

    #[test]
    fn a_configured_file_path_maps_to_a_sibling_folder() {
        assert_eq!(bundle_directory("kitty-pro-sync.json"), "kitty-pro-sync");
        assert_eq!(
            bundle_directory("sync/kitty-profile.JSON"),
            "sync/kitty-profile"
        );
        // A path that already names a folder stays one.
        assert_eq!(bundle_directory("sync/kitty"), "sync/kitty");
        assert_eq!(bundle_directory("/a/b.json"), "a/b");
    }

    #[test]
    fn object_names_are_content_addresses() {
        let payload = br#"[{"id":1}]"#;
        let name = bundle_object_name("dns-overrides", payload);
        assert_eq!(name, bundle_object_name("dns-overrides", payload));
        assert!(name.starts_with("dns-overrides-"));
        assert!(name.ends_with(".json"));
        assert_ne!(name, bundle_object_name("dns-overrides", b"[{\"id\":2}]"));
        assert_ne!(name, bundle_object_name("custom-rules", payload));
    }

    /// Sections a snapshot does not carry write no object at all. That is what
    /// keeps "this build never had the feature" distinct from "zero rules", and
    /// it is the shape a reader needs in order to preserve local DNS on a pull.
    #[test]
    fn absent_sections_write_no_objects() {
        let mut legacy = SyncSnapshot::from_profile(&AppProfile::default(), 1);
        legacy.dns_overrides = None;
        legacy.config_script = None;
        let legacy_sections = bundle_sections(&legacy).expect("legacy sections should build");
        assert_eq!(
            section_names(&legacy_sections),
            vec!["subscriptions", "custom-rules"]
        );

        let modern = SyncSnapshot::from_profile(&AppProfile::default(), 1);
        let modern_sections = bundle_sections(&modern).expect("modern sections should build");
        assert_eq!(
            section_names(&modern_sections),
            vec![
                "subscriptions",
                "custom-rules",
                "dns-overrides",
                "config-script"
            ]
        );
    }

    fn section_names(sections: &[(String, Vec<u8>)]) -> Vec<&str> {
        sections.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// Manifest names become request paths, so they cannot be allowed to walk
    /// out of the bundle folder.
    #[test]
    fn manifest_names_are_confined_to_the_bundle() {
        assert!(validate_bundle_file_name("dns-overrides-abcd.json").is_ok());
        assert!(validate_bundle_file_name("../manifest.json").is_err());
        assert!(validate_bundle_file_name("a/b.json").is_err());
        assert!(validate_bundle_file_name("/etc/passwd.json").is_err());
        assert!(validate_bundle_file_name(".hidden.json").is_err());
        assert!(validate_bundle_file_name("UPPER.json").is_err());
        assert!(validate_bundle_file_name("no-extension").is_err());
    }

    /// The manifest is the one index in a bundle that is not itself hashed, and
    /// every entry it names is a request. It has to hold up before any of that
    /// is fetched.
    #[test]
    fn manifest_shape_is_checked_before_anything_is_downloaded() {
        let file = |section: &str| BundleFile {
            section: section.to_string(),
            name: format!("{section}-aaaaaaaaaaaaaaaa.json"),
            size: 2,
            sha256: "a".repeat(64),
        };
        let manifest = |files: Vec<BundleFile>| BundleManifest {
            format: SYNC_BUNDLE_FORMAT.to_string(),
            version: SYNC_BUNDLE_VERSION,
            updated_at: 1,
            files,
        };

        assert!(validate_manifest_shape(&manifest(vec![
            file("subscriptions"),
            file("dns-overrides"),
        ]))
        .is_ok());

        // A section this build cannot place must fail rather than be dropped:
        // dropping would delete it on the next push.
        assert!(validate_manifest_shape(&manifest(vec![file("future-things")])).is_err());
        // Two entries for one section would otherwise resolve last-one-wins.
        assert!(validate_manifest_shape(&manifest(vec![
            file("custom-rules"),
            file("custom-rules"),
        ]))
        .is_err());
        // Names must belong to the section they claim, and digests be digests.
        let mut mismatched = file("subscriptions");
        mismatched.name = "custom-rules-aaaaaaaaaaaaaaaa.json".to_string();
        assert!(validate_manifest_shape(&manifest(vec![mismatched])).is_err());
        let mut short_digest = file("subscriptions");
        short_digest.sha256 = "deadbeef".to_string();
        assert!(validate_manifest_shape(&manifest(vec![short_digest])).is_err());
        // Bounded, so a huge list cannot turn one pull into thousands of GETs.
        let many = manifest(vec![file("subscriptions"); MAX_BUNDLE_FILES + 1]);
        assert!(validate_manifest_shape(&many).is_err());
    }

    #[test]
    fn client_view_masks_non_empty_secrets_only() {
        let mut config = SyncConfig::default();
        let plain = masked_for_client(&config);
        assert!(plain.password.is_empty() && plain.secret_key.is_empty());

        config.password = "webdav-pass".into();
        config.secret_key = "s3-secret".into();
        let masked = masked_for_client(&config);
        assert_eq!(masked.password, SECRET_PLACEHOLDER);
        assert_eq!(masked.secret_key, SECRET_PLACEHOLDER);
        // 非密字段原样保留（前端回显需要）。
        assert_eq!(masked.webdav_endpoint, config.webdav_endpoint);
        assert_eq!(masked.s3_endpoint, config.s3_endpoint);
        assert_eq!(masked.username, config.username);
        assert_eq!(masked.access_key, config.access_key);
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
    fn legacy_shared_endpoint_migrates_to_saved_provider() {
        let mut s3: serde_json::Value = serde_json::json!({
            "provider": "s3",
            "endpoint": "https://account.r2.cloudflarestorage.com",
        });
        migrate_legacy_endpoint(&mut s3);
        assert!(s3.get("endpoint").is_none());
        assert_eq!(
            s3["s3_endpoint"],
            serde_json::json!("https://account.r2.cloudflarestorage.com")
        );
        assert!(s3.get("webdav_endpoint").is_none());

        let mut webdav: serde_json::Value = serde_json::json!({
            "provider": "webdav",
            "endpoint": "https://dav.example.com/user",
        });
        migrate_legacy_endpoint(&mut webdav);
        assert_eq!(
            webdav["webdav_endpoint"],
            serde_json::json!("https://dav.example.com/user")
        );
        assert!(webdav.get("s3_endpoint").is_none());

        // A split config that already carries the new field wins; the legacy
        // value is dropped instead of resurrecting an old address.
        let mut split: serde_json::Value = serde_json::json!({
            "provider": "webdav",
            "endpoint": "https://old.example.com/user",
            "webdav_endpoint": "https://new.example.com/user",
        });
        migrate_legacy_endpoint(&mut split);
        assert_eq!(
            split["webdav_endpoint"],
            serde_json::json!("https://new.example.com/user")
        );

        let mut blank: serde_json::Value = serde_json::json!({ "endpoint": "  " });
        migrate_legacy_endpoint(&mut blank);
        assert!(blank.get("webdav_endpoint").is_none());
    }

    #[test]
    fn provider_switch_keeps_each_address_separate() {
        let saved = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::S3,
            s3_endpoint: "https://account.r2.cloudflarestorage.com".to_string(),
            bucket: "backup".to_string(),
            region: "auto".to_string(),
            access_key: "access".to_string(),
            secret_key: "secret".to_string(),
            last_sync_at: 42,
            last_remote_revision: "\"revision-1\"".to_string(),
            ..SyncConfig::default()
        };
        // The user only clicked the WebDAV tab: no address, no shared target.
        let switched = SyncConfig {
            provider: SyncProviderKind::WebDav,
            ..saved.clone()
        };
        assert!(switched.webdav_endpoint.is_empty());
        assert!(!same_remote_target(&saved, &switched));
        let merged = merge_sync_metadata(&saved, &switched);
        assert_eq!(merged.last_sync_at, 0);
        assert!(merged.last_remote_revision.is_empty());
        // Saving that switch must not validate the S3 address as a WebDAV URL.
        assert!(validate_config(&switched).is_err());
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
            s3_endpoint: String::new(),
            path: "folder/profile.json".to_string(),
            bucket: "kitty-backup".to_string(),
            region: "ap-east-1".to_string(),
            access_key: "key".to_string(),
            secret_key: "secret".to_string(),
            ..SyncConfig::default()
        };
        assert_eq!(
            build_s3_url(&config, &config.path.clone())
                .expect("AWS URL should build")
                .as_str(),
            "https://kitty-backup.s3.ap-east-1.amazonaws.com/folder/profile.json"
        );

        config.bucket = "kitty.backup".to_string();
        assert_eq!(
            build_s3_url(&config, &config.path.clone())
                .expect("dotted AWS bucket URL should build")
                .as_str(),
            "https://s3.ap-east-1.amazonaws.com/kitty.backup/folder/profile.json"
        );

        config.bucket = "kitty-backup".to_string();
        config.s3_endpoint = "http://minio.local:9000".to_string();
        assert_eq!(
            build_s3_url(&config, &config.path.clone())
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
    fn snapshot_accepts_process_paths_from_other_desktop_platforms() {
        let mut snapshot = SyncSnapshot::from_profile(&Default::default(), 0);
        snapshot.custom_rules = vec![
            proxy_core::CustomRule {
                id: 1,
                enabled: true,
                match_type: proxy_core::CustomRuleMatch::ProcessPath,
                value: r"C:\Program Files\Telegram Desktop\Telegram.exe".to_string(),
                action: proxy_core::CustomRuleAction::Proxy,
            },
            proxy_core::CustomRule {
                id: 2,
                enabled: true,
                match_type: proxy_core::CustomRuleMatch::ProcessPath,
                value: "/Applications/Telegram.app/Contents/MacOS/Telegram".to_string(),
                action: proxy_core::CustomRuleAction::Direct,
            },
        ];

        assert!(validate_snapshot(&snapshot).is_ok());
    }

    #[test]
    fn snapshot_decoder_accepts_v1_and_writes_v2() {
        let mut legacy = SyncSnapshot::from_profile(&Default::default(), 7);
        legacy.version = LEGACY_SYNC_SNAPSHOT_VERSION;
        let bytes = serde_json::to_vec(&legacy).expect("legacy snapshot should serialize");

        let decoded = decode_snapshot(&bytes).expect("v1 snapshot should remain readable");
        assert_eq!(decoded.version, LEGACY_SYNC_SNAPSHOT_VERSION);
        assert_eq!(
            SyncSnapshot::from_profile(&Default::default(), 0).version,
            SYNC_SNAPSHOT_VERSION
        );
    }

    #[test]
    fn snapshot_decoder_rejects_future_version_before_payload() {
        let bytes = br#"{
            "format": "kitty-pro-sync",
            "version": 3,
            "custom_rules": [{
                "id": 1,
                "enabled": true,
                "match_type": "future_process_match",
                "value": "Telegram",
                "action": "proxy"
            }]
        }"#;

        let error = decode_snapshot(bytes).expect_err("future snapshot should be rejected");
        assert!(
            error.to_string().contains("不支持的同步快照版本: 3"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn snapshot_rejects_duplicate_subscription_ids() {
        let subscription = proxy_core::SyncSubscription {
            id: 7,
            name: "primary".to_string(),
            source: "https://example.com/subscription".to_string(),
        };
        let mut snapshot = SyncSnapshot::from_profile(&Default::default(), 0);
        snapshot.subscriptions = vec![subscription.clone(), subscription];
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[test]
    fn changing_remote_target_resets_sync_metadata() {
        let existing = SyncConfig {
            enabled: true,
            webdav_endpoint: "https://dav.example.com/user".to_string(),
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
    async fn webdav_push_and_pull_round_trips_a_bundle() {
        let remote = FakeRemote::start();
        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::WebDav,
            webdav_endpoint: remote.endpoint.clone(),
            path: "kitty-profile.json".to_string(),
            username: "demo".to_string(),
            password: "secret".to_string(),
            ..SyncConfig::default()
        };
        let profile = AppProfile {
            subscriptions: vec![proxy_core::Subscription {
                id: 3,
                name: "Primary".to_string(),
                source: "https://example.com/sub?token=secret".to_string(),
                nodes: Vec::new(),
                proxy_server_nameservers: Vec::new(),
                rejected_count: 0,
            }],
            dns_overrides: vec![proxy_core::DnsOverride {
                id: 1,
                enabled: true,
                domain_suffix: "example.net".to_string(),
                server: "192.0.2.53".to_string(),
            }],
            config_script_enabled: true,
            config_script: "function main(c) { return c; }".to_string(),
            ..AppProfile::default()
        };

        let result = push(&config, SyncSnapshot::from_profile(&profile, 0), false)
            .await
            .expect("WebDAV push should succeed");
        let pulled = pull(&config).await.expect("WebDAV pull should succeed");

        assert_eq!(pulled.snapshot.updated_at, result.updated_at);
        assert_eq!(pulled.remote_revision, "\"revision-remote\"");
        // Every section survives, not just the two the single-document layout
        // could carry.
        let expected = SyncSnapshot::from_profile(&profile, pulled.snapshot.updated_at);
        assert_eq!(pulled.snapshot, expected);

        // Stored as a folder: a manifest plus one object per section.
        let paths = remote.paths();
        assert!(
            paths.contains(&"/kitty-profile/manifest.json".to_string()),
            "manifest missing from {paths:?}"
        );
        assert!(paths
            .iter()
            .any(|path| path.starts_with("/kitty-profile/dns-overrides-")));
        assert!(paths
            .iter()
            .any(|path| path.starts_with("/kitty-profile/config-script-")));
        assert!(paths
            .iter()
            .any(|path| path.starts_with("/kitty-profile/subscriptions-")));
    }

    /// The manifest is what makes a bundle atomic: a reader only ever sees it
    /// once every object it names is already uploaded.
    #[tokio::test]
    async fn bundle_manifest_is_uploaded_last() {
        let remote = FakeRemote::start();
        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::WebDav,
            webdav_endpoint: remote.endpoint.clone(),
            path: "sync/profile.json".to_string(),
            ..SyncConfig::default()
        };

        push(
            &config,
            SyncSnapshot::from_profile(&AppProfile::default(), 0),
            true,
        )
        .await
        .expect("bundle push should succeed");

        let writes = remote.writes();
        assert!(
            writes.len() >= 2,
            "expected section writes plus a manifest: {writes:?}"
        );
        assert_eq!(
            writes.last().expect("at least one write").as_str(),
            "/sync/profile/manifest.json",
            "the manifest must be the final write"
        );
        assert!(writes[..writes.len() - 1]
            .iter()
            .all(|path| !path.ends_with("manifest.json")));
    }

    /// The payoff of splitting the blob into objects: a second push that changes
    /// one section re-uploads that section and the manifest, and nothing else.
    #[tokio::test]
    async fn push_reuploads_only_the_changed_section() {
        let remote = FakeRemote::start();
        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::WebDav,
            webdav_endpoint: remote.endpoint.clone(),
            path: "profile.json".to_string(),
            ..SyncConfig::default()
        };

        let first = push(
            &config,
            SyncSnapshot::from_profile(
                &AppProfile {
                    dns_overrides: vec![proxy_core::DnsOverride {
                        id: 1,
                        enabled: true,
                        domain_suffix: "example.net".to_string(),
                        server: "192.0.2.1".to_string(),
                    }],
                    ..AppProfile::default()
                },
                0,
            ),
            true,
        )
        .await
        .expect("first push should succeed");
        let seen = remote.writes().len();

        // Carry the checkpoint forward so the second push runs its preflight and
        // learns which objects the remote already holds.
        let second_config = SyncConfig {
            last_sync_at: first.updated_at,
            last_remote_revision: first.remote_revision.clone(),
            ..config.clone()
        };
        push(
            &second_config,
            SyncSnapshot::from_profile(
                &AppProfile {
                    dns_overrides: vec![proxy_core::DnsOverride {
                        id: 1,
                        enabled: true,
                        domain_suffix: "example.net".to_string(),
                        server: "192.0.2.2".to_string(),
                    }],
                    ..AppProfile::default()
                },
                0,
            ),
            false,
        )
        .await
        .expect("second push should succeed");

        let new_writes = &remote.writes()[seen..];
        assert!(
            new_writes
                .iter()
                .any(|path| path.starts_with("/profile/dns-overrides-")),
            "the changed section should re-upload: {new_writes:?}"
        );
        assert!(
            !new_writes.iter().any(|path| path.contains("subscriptions")),
            "unchanged subscriptions must not re-upload: {new_writes:?}"
        );
        assert!(
            !new_writes.iter().any(|path| path.contains("custom-rules")),
            "unchanged rules must not re-upload: {new_writes:?}"
        );
        assert_eq!(
            new_writes.last().map(String::as_str),
            Some("/profile/manifest.json"),
            "the manifest stays the commit point"
        );
    }

    /// A remote written by an older build is a single document at exactly
    /// `config.path`. It must still download, and the first push must create a
    /// manifest rather than clobber it blind.
    #[tokio::test]
    async fn legacy_single_document_still_downloads_and_upgrades() {
        let remote = FakeRemote::start();
        // Verbatim shape of an object written by a pre-DNS build: v1, and no
        // `dns_overrides` key at all. Serializing a modern snapshot would not
        // reproduce it, because a modern snapshot always carries the section.
        let legacy = br#"{
            "format": "kitty-pro-sync",
            "version": 1,
            "updated_at": 42,
            "subscriptions": [],
            "custom_rules": [],
            "active_subscription_id": null,
            "selected_tag": "",
            "group_selections": {}
        }"#
        .to_vec();
        remote.put("kitty-profile.json", legacy.clone());

        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::WebDav,
            webdav_endpoint: remote.endpoint.clone(),
            path: "kitty-profile.json".to_string(),
            last_sync_at: 42,
            last_remote_revision: "\"revision-remote\"".to_string(),
            ..SyncConfig::default()
        };

        let pulled = pull(&config)
            .await
            .expect("legacy document should still download");
        assert_eq!(pulled.snapshot.updated_at, 42);
        // A v1 document carries no DNS section, which is precisely the case the
        // pull guard has to survive.
        assert!(pulled.snapshot.dns_overrides.is_none());

        push(
            &config,
            SyncSnapshot::from_profile(&AppProfile::default(), 0),
            false,
        )
        .await
        .expect("pushing over a legacy target should succeed");
        assert!(remote
            .paths()
            .contains(&"/kitty-profile/manifest.json".to_string()));
        // The legacy object is left in place so a device still on an older build
        // can read it until it upgrades too.
        assert_eq!(
            remote.body("/kitty-profile.json").as_deref(),
            Some(&legacy[..])
        );

        // A reader now gets the bundle, whose manifest always lists the modern
        // sections, so the same pull returns a snapshot that carries them.
        let upgraded = pull(&config).await.expect("bundle should download");
        assert!(upgraded.snapshot.dns_overrides.is_some());
        assert!(upgraded.snapshot.updated_at > 42);
    }

    /// The exact failure this guard exists for: a remote object written before
    /// DNS overrides shipped carries no `dns_overrides` key at all (this shape is
    /// verbatim from a real R2 bucket, v1). Pulling it used to clear the local
    /// rules, and the autosave then persisted the loss.
    #[test]
    fn legacy_remote_document_keeps_local_dns_rules_on_pull() {
        let legacy = br#"{
            "format": "kitty-pro-sync",
            "version": 1,
            "updated_at": 1786457924,
            "subscriptions": [],
            "custom_rules": [],
            "active_subscription_id": null,
            "selected_tag": "",
            "group_selections": {}
        }"#;

        let snapshot = decode_snapshot(legacy).expect("legacy document should decode");
        assert!(snapshot.dns_overrides.is_none());
        validate_snapshot(&snapshot).expect("legacy document should validate");

        let mut profile = AppProfile {
            dns_overrides: vec![proxy_core::DnsOverride {
                id: 1,
                enabled: true,
                domain_suffix: "et.net".to_string(),
                server: "100.100.100.101".to_string(),
            }],
            ..AppProfile::default()
        };
        snapshot.apply_to_profile(&mut profile);
        assert_eq!(
            profile.dns_overrides.len(),
            1,
            "a legacy pull must not delete locally configured DNS rules"
        );
    }

    /// A modern round-trip must still carry both sections, so deleting a rule on
    /// one device does reach the other.
    #[test]
    fn modern_round_trip_still_propagates_dns_rules() {
        let source = AppProfile {
            dns_overrides: vec![proxy_core::DnsOverride {
                id: 2,
                enabled: true,
                domain_suffix: "example.net".to_string(),
                server: "192.0.2.53".to_string(),
            }],
            config_script_enabled: true,
            config_script: "function main(c) { return c; }".to_string(),
            ..AppProfile::default()
        };
        let snapshot = SyncSnapshot::from_profile(&source, 5);
        let bytes = serde_json::to_vec(&snapshot).expect("snapshot should serialize");
        let decoded = decode_snapshot(&bytes).expect("modern document should decode");
        validate_snapshot(&decoded).expect("modern document should validate");

        let mut target = AppProfile::default();
        decoded.apply_to_profile(&mut target);
        assert_eq!(target.dns_overrides, source.dns_overrides);
        assert!(target.config_script_enabled);
        assert_eq!(target.config_script, source.config_script);
    }

    /// `push` reads the remote, then writes guarded by its revision. If another
    /// device publishes in between, the write must fail rather than stomp it.
    #[tokio::test]
    async fn bundle_upload_rejects_a_remote_change_after_the_check() {
        let remote = FakeRemote::start();
        let manifest = serde_json::to_vec(&BundleManifest {
            format: SYNC_BUNDLE_FORMAT.to_string(),
            version: SYNC_BUNDLE_VERSION,
            updated_at: 42,
            files: Vec::new(),
        })
        .expect("manifest should serialize");
        remote.put("kitty-profile/manifest.json", manifest);

        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::WebDav,
            webdav_endpoint: remote.endpoint.clone(),
            path: "kitty-profile.json".to_string(),
            last_sync_at: 42,
            last_remote_revision: "\"revision-remote\"".to_string(),
            ..SyncConfig::default()
        };

        // The preflight GET sees the revision we match; the manifest PUT that
        // follows is rejected, exactly as a real conditional write would be.
        remote.reject_next_put();
        let error = push(
            &config,
            SyncSnapshot::from_profile(&AppProfile::default(), 42),
            false,
        )
        .await
        .expect_err("changed remote must reject conditional upload");
        assert!(error.to_string().contains("检查后发生变化"), "{error}");
    }

    #[tokio::test]
    async fn s3_bundle_uses_path_style_and_signed_headers_for_custom_endpoint() {
        let remote = FakeRemote::start();
        let config = SyncConfig {
            enabled: true,
            provider: SyncProviderKind::S3,
            s3_endpoint: remote.endpoint.clone(),
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

        let requests = remote.requests();
        assert!(!requests.is_empty(), "expected signed requests");
        // Path style: bucket first, then the bundle folder.
        assert!(
            requests
                .iter()
                .any(|head| head.starts_with("PUT /kitty-backup/sync/profile/")),
            "unexpected request line: {:?}",
            requests.first()
        );
        let first = requests.first().expect("request should exist");
        let lower = first.to_ascii_lowercase();
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
}
