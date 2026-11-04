//! In-app update check against the project's GitHub Releases.
//!
//! The design follows what Magpie's `UpdateService` gets right and stops where
//! that design stops working for us:
//!
//! - One small JSON document describes the newest release, fetched from a
//!   fixed, non-user-supplied URL, compared against the running build, and
//!   reported through an explicit status enum.
//! - Automatic checks are throttled to once a day, remembered across restarts,
//!   and can be switched off.
//! - Anything the check cannot classify - a malformed tag, an asset with no
//!   operating system in its name, an architecture with no published build - is
//!   reported as "nothing to install", never as an invitation to fetch a binary
//!   that cannot run.
//!
//! What is deliberately *not* copied: Magpie downloads the new archive and
//! launches a helper that replaces the installation in place. Kitty Pro ships a
//! signed installer per platform (`.dmg`, `.msi`, `.deb`, an APK behind Android's
//! own installer), each with its own privilege requirements, so the check ends at
//! "here is the release, take it from GitHub" and hands the user a link. The
//! release page is also where the checksums live, which keeps verification with
//! the installer instead of duplicating an installer here.
//!
//! Requests go out from the native side - the desktop process, the Android
//! process, or the self-hosted web server for browser clients - never from a
//! browser, so a page served over LAN cannot be pointed at a third-party host by
//! anything in a profile.

use dioxus::prelude::*;
use proxy_core::{select_asset, GitHubRelease, ReleaseTarget};
use serde::{Deserialize, Serialize};
// Only the throttle reads this, and the throttle exists only where a check can
// reach GitHub: importing it unconditionally warns on the `wasm32` client.
#[cfg(not(target_arch = "wasm32"))]
use proxy_core::UPDATE_CHECK_INTERVAL_SECS;

/// Repository whose releases this build checks, overridable for forks at compile
/// time so a fork never prompts its users to install someone else's binaries:
/// `KITTY_UPDATE_REPO=acme/kitty-pro cargo build`.
///
/// Only the native side reaches GitHub, so a browser build carries the constant
/// without reading it.
#[allow(dead_code)]
const UPDATE_REPO: &str = match option_env!("KITTY_UPDATE_REPO") {
    Some(repo) => repo,
    None => "fagao-ai/kitty-pro",
};

/// The GitHub Releases API payload is small; the cap only exists so a hostile or
/// misconfigured redirect cannot make the app allocate freely.
#[cfg(not(target_arch = "wasm32"))]
const MAX_RELEASE_BYTES: usize = 1024 * 1024;

/// Settings the update card shows and edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSettings {
    /// Contact GitHub at startup and once a day to look for a newer release.
    #[serde(default = "default_auto_check")]
    pub auto_check: bool,
    /// Unix seconds of the last check that reached GitHub, kept across restarts.
    #[serde(default)]
    pub last_check_at: u64,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            auto_check: default_auto_check(),
            last_check_at: 0,
        }
    }
}

const fn default_auto_check() -> bool {
    true
}

/// Which half of the check a status is reporting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppUpdateState {
    /// Nothing has been asked yet.
    #[default]
    Idle,
    /// The running shell has no publishable package to update from.
    Unsupported,
    /// The newest published release is not newer than this build.
    UpToDate,
    /// A newer release exists.
    Available,
    /// The last check failed.
    Failed,
}

/// One release, reduced to what the card renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRelease {
    /// Version with the leading `v` removed, ready to compare. The card renders
    /// it as `v1.2.3`, which is how the tag reads on GitHub.
    pub version: String,
    /// Release title, falling back to the version.
    pub title: String,
    /// Release page. The card links here rather than straight to an asset, so the
    /// notes and the checksums are one click away from the download.
    pub url: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    /// Installer this installation should take, `None` when the release publishes
    /// nothing for this operating system and CPU.
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub asset_size: u64,
}

/// Everything the update card needs, in one read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUpdateStatus {
    pub state: AppUpdateState,
    pub current_version: String,
    /// Newest release GitHub reported, whether or not it is newer than this build.
    #[serde(default)]
    pub latest: Option<AppRelease>,
    pub last_check_at: u64,
    /// The newest release is the one the user chose to ignore. The card stays
    /// readable but stops presenting the update as something to act on.
    #[serde(default)]
    pub snoozed: bool,
    /// `<kind> · <os> · <arch>` the asset matcher was asked for. Shown so "no
    /// package for you" is at least a specific answer.
    pub target: String,
    #[serde(default)]
    pub error: Option<String>,
    /// Unix seconds on the machine that produced this answer.
    ///
    /// Ages are rendered from this rather than from the reader's own clock: a
    /// browser client compiled to `wasm32` has no platform clock at all (reading
    /// one traps), and the machine that checked GitHub is the only one whose
    /// "three hours ago" is worth showing.
    #[serde(default)]
    pub server_time: u64,
}

/// Arguments of [`check_app_update`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUpdateRequest {
    /// The platform string the shell renders, so the check and the UI agree.
    pub platform: String,
    /// `CARGO_PKG_VERSION` of the shell being replaced.
    pub current_version: String,
    /// Ignore the daily throttle: the user pressed the button.
    pub force: bool,
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    get("/api/update/settings")
)]
pub async fn load_update_settings() -> Result<UpdateSettings, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        super::run_native_blocking("读取更新设置失败", || {
            read_state().map(|state| state.settings)
        })
        .await
    }
    #[cfg(target_arch = "wasm32")]
    {
        // A browser has no state file of its own; the server it is talking to owns
        // the schedule, and the web shell shows the server's answer instead.
        Ok(UpdateSettings::default())
    }
}

#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/update/settings")
)]
pub async fn save_update_settings(
    settings: UpdateSettings,
) -> Result<UpdateSettings, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = state_lock().lock().await;
        let mut state = super::run_native_blocking("读取更新状态失败", read_state).await?;
        // Only the switch belongs to the caller. The throttle timestamp stays
        // whatever the last real check wrote, so a client cannot reset the daily
        // window by posting a stale settings object.
        state.settings.auto_check = settings.auto_check;
        let saved = state.clone();
        super::run_native_blocking("保存更新设置失败", move || write_state(&saved)).await?;
        Ok(state.settings)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = settings;
        Err(ServerFnError::new("浏览器端不能写入更新设置"))
    }
}

/// Look for a newer release.
///
/// `force: false` honours the daily throttle and answers from the last result
/// that reached GitHub, which is what makes a startup check free on every
/// subsequent launch. `force: true` is the button in the settings card.
#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/update/check")
)]
pub async fn check_app_update(request: AppUpdateRequest) -> Result<AppUpdateStatus, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = unix_now();

        // A shell with no published package gets an answer without touching the
        // state file or the network.
        let Some(target) = ReleaseTarget::detect(&request.platform) else {
            return Ok(AppUpdateStatus {
                state: AppUpdateState::Unsupported,
                current_version: request.current_version,
                latest: None,
                last_check_at: 0,
                snoozed: false,
                target: request.platform,
                error: None,
                server_time: now,
            });
        };

        // Held for the whole round trip so two browser clients of one web server
        // cannot both reach GitHub, or read a half-written state file.
        let _guard = state_lock().lock().await;
        let mut state = super::run_native_blocking("读取更新状态失败", read_state).await?;
        let due = request.force
            || (state.settings.auto_check && should_check(now, state.settings.last_check_at));

        // A failure is reported but not remembered: it says nothing about whether
        // a newer release exists, and a stale error line would outlive its cause.
        let mut error = None;
        if due {
            match fetch_release(&target).await {
                Ok(latest) => {
                    state.latest = latest;
                    // Only a check that reached GitHub moves the throttle, so an
                    // offline morning does not postpone the next attempt by a day.
                    state.settings.last_check_at = now;
                    // A state file that will not write is worth reporting, but not
                    // worth throwing the answer away: the next launch will just ask
                    // again, which is exactly what an unthrottled check does.
                    if let Err(failure) = super::run_native_blocking("保存更新状态失败", {
                        let state = state.clone();
                        move || write_state(&state)
                    })
                    .await
                    {
                        error = Some(failure.to_string());
                    }
                }
                Err(failure) => error = Some(failure.to_string()),
            }
        }
        Ok(status_for(&target, &request.current_version, &state, error))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = request;
        Err(ServerFnError::new("浏览器端不能直接检查更新"))
    }
}

/// Stop presenting one version as an update until a newer release appears.
#[cfg_attr(
    all(
        not(any(target_os = "android", target_os = "ios")),
        any(target_arch = "wasm32", feature = "server")
    ),
    post("/api/update/dismiss")
)]
pub async fn dismiss_app_update(version: String) -> Result<UpdateSettings, ServerFnError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _guard = state_lock().lock().await;
        let mut state = super::run_native_blocking("读取更新状态失败", read_state).await?;
        let version = version.trim().to_string();
        if version.len() > 64 {
            return Err(ServerFnError::new("版本号过长"));
        }
        state.snoozed_version = version;
        let saved = state.clone();
        super::run_native_blocking("保存更新忽略项失败", move || write_state(&saved)).await?;
        Ok(state.settings)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = version;
        Err(ServerFnError::new("浏览器端不能写入更新设置"))
    }
}

/// Whether a throttled check may reach GitHub.
///
/// A clock that moved backwards (an NTP correction, a wrong timezone on a phone)
/// must not wedge the check forever, so a future timestamp re-checks.
#[cfg(not(target_arch = "wasm32"))]
fn should_check(now: u64, last_check_at: u64) -> bool {
    // Zero is "never", not "the epoch", so a first run is always due.
    if last_check_at == 0 {
        return true;
    }
    // A timestamp in the future means the clock moved backwards - an NTP
    // correction, a phone that flew across a timezone. Re-check rather than wait
    // out an interval measured against a clock that no longer exists.
    now < last_check_at || now - last_check_at >= UPDATE_CHECK_INTERVAL_SECS
}

/// Turn the persisted state into an answer for the running build.
///
/// The version comparison happens here rather than at fetch time so upgrading
/// this app self-corrects a cached "update available" without waiting for the
/// next check.
#[cfg(not(target_arch = "wasm32"))]
fn status_for(
    target: &ReleaseTarget,
    current_version: &str,
    state: &UpdateState,
    error: Option<String>,
) -> AppUpdateStatus {
    let latest = state.latest.clone();
    let newer = latest
        .as_ref()
        .is_some_and(|release| proxy_core::is_newer_version(current_version, &release.version));
    let snoozed = newer
        && latest
            .as_ref()
            .is_some_and(|release| release.version == state.snoozed_version);
    let resolved = if error.is_some() {
        // A failed check says nothing about whether this build is current, so the
        // previous answer stays visible next to the error.
        if newer {
            AppUpdateState::Available
        } else {
            AppUpdateState::Failed
        }
    } else if newer {
        AppUpdateState::Available
    } else if latest.is_some() || state.settings.last_check_at > 0 {
        // Either the newest release is this one, or the check ran against a
        // repository with no published release at all: both mean "you are on the
        // newest build we have".
        AppUpdateState::UpToDate
    } else {
        AppUpdateState::Idle
    };
    AppUpdateStatus {
        state: resolved,
        current_version: current_version.to_string(),
        latest,
        last_check_at: state.settings.last_check_at,
        snoozed,
        target: target.label(),
        error,
        // Stamped here: this function only exists on the native side, which is
        // the only side that has a clock.
        server_time: unix_now(),
    }
}

/// Unix seconds on this machine, zero before the epoch.
///
/// The update card renders ages, and a `wasm32` client cannot read a clock at all
/// (`SystemTime::now()` traps there), so the native side - desktop, Android, or
/// the self-hosted web server - stamps every answer with its own time instead.
#[cfg(not(target_arch = "wasm32"))]
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// Ask GitHub for the newest release and pick the asset for `target`.
#[cfg(not(target_arch = "wasm32"))]
async fn fetch_release(target: &ReleaseTarget) -> Result<Option<AppRelease>, ServerFnError> {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        // GitHub requires a User-Agent; the shell's own version is not known
        // here (the client passes it in the request), and it is not needed.
        .user_agent("kitty-pro-update-check")
        .build()
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    let response = client
        .get(format!(
            "https://api.github.com/repos/{UPDATE_REPO}/releases/latest"
        ))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| ServerFnError::new(format!("检查更新失败: {error}")))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        // A repository with no published release answers 404. That is "nothing to
        // install", not a failure worth showing as one.
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|error| ServerFnError::new(format!("更新服务返回错误: {error}")))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RELEASE_BYTES as u64)
    {
        return Err(ServerFnError::new("更新信息超过 1 MiB 限制"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| ServerFnError::new(format!("读取更新信息失败: {error}")))?;
    if bytes.len() > MAX_RELEASE_BYTES {
        return Err(ServerFnError::new("更新信息超过 1 MiB 限制"));
    }
    let release: GitHubRelease = serde_json::from_slice(&bytes)
        .map_err(|error| ServerFnError::new(format!("更新信息格式错误: {error}")))?;
    Ok(release_for(&release, target))
}

/// Reduce a GitHub release to what this installation needs.
///
/// Reached only from the native fetch (and its tests): a browser build asks its
/// server rather than parsing a release document itself.
#[allow(dead_code)]
fn release_for(release: &GitHubRelease, target: &ReleaseTarget) -> Option<AppRelease> {
    let tag = release.tag_name.trim();
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag).to_string();
    if version.is_empty() || proxy_core::ReleaseVersion::parse(&version).is_none() {
        return None;
    }
    let asset = select_asset(&release.assets, target);
    let title = release
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&format!("v{version}"))
        .to_string();
    Some(AppRelease {
        version,
        title,
        url: release_url(release.html_url.as_deref()),
        notes: release.notes(),
        published_at: release.published_at.clone(),
        prerelease: release.prerelease,
        asset: asset.map(|asset| asset.name.clone()),
        asset_size: asset.map(|asset| asset.size).unwrap_or_default(),
    })
}

/// The release page this card links to.
///
/// GitHub's `html_url` is a field in a document fetched over the network, and it
/// ends up as the `href` of the card's primary action, so it is accepted only as
/// an `http(s)` URL. Anything else - absent, blank, or a scheme like
/// `javascript:` - falls back to the repository's release list, which is a
/// correct answer for every release and is built from a compile-time constant the
/// network cannot influence.
///
/// The rest of this module already refuses to allocate freely on a hostile
/// redirect; a link the user clicks deserves the same suspicion.
fn release_url(html_url: Option<&str>) -> String {
    let fallback = || format!("https://github.com/{UPDATE_REPO}/releases");
    let Some(url) = html_url.map(str::trim) else {
        return fallback();
    };
    // The whole scheme up to `://` has to equal `http` or `https`, compared
    // case-insensitively. That is what rejects `javascript:alert(1)` (no `://` at
    // all, or the wrong scheme), a lookalike like `httpsx://`, and `https://`
    // with nothing after it - an empty authority is not a link either.
    let scheme_end = match url.find("://") {
        Some(index) => index,
        None => return fallback(),
    };
    let scheme = &url[..scheme_end];
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
        || url.len() <= scheme_end + 3
    {
        return fallback();
    }
    url.to_string()
}

/// What is remembered between runs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct UpdateState {
    #[serde(default)]
    settings: UpdateSettings,
    #[serde(default)]
    latest: Option<AppRelease>,
    /// Newest release the user chose to ignore. Compared by version, so the next
    /// release asks again.
    #[serde(default)]
    snoozed_version: String,
}

/// `<data dir>/update.json`, beside `profile.json` and `sync.json`.
#[cfg(not(target_arch = "wasm32"))]
fn update_state_path() -> Result<std::path::PathBuf, ServerFnError> {
    super::profile_path()?
        .parent()
        .map(|directory| directory.join("update.json"))
        .ok_or_else(|| ServerFnError::new("无法确定更新状态目录"))
}

/// Read the remembered state, falling back to defaults for anything unreadable.
///
/// A file that cannot be parsed reads as "nothing remembered" rather than as an
/// error. The alternative is permanent: every read would fail the same way, so
/// the card would sit on "尚未检查" for the rest of the installation's life and
/// the only cure is deleting the file by hand. Falling back also self-repairs,
/// because the next check writes a valid file over it.
///
/// What a fallback costs is a snooze and an opted-out auto-check, both of which
/// are re-settable from the card; what it buys is a feature that still works.
/// Genuine I/O failures on the directory itself (a read-only volume, a path the
/// app cannot stat) still surface, because defaults would not fix those and the
/// write path has to report them anyway.
#[cfg(not(target_arch = "wasm32"))]
fn read_state() -> Result<UpdateState, ServerFnError> {
    let path = update_state_path()?;
    match std::fs::read(&path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UpdateState::default()),
        Err(error) => Err(ServerFnError::new(format!("读取更新状态失败: {error}"))),
    }
}

/// Write the state file without ever leaving a half-written one behind.
///
/// The payload is public information plus one boolean, so it does not need the
/// private-file treatment `sync.json` gets, but it does need to survive a crash:
/// a corrupt file would otherwise black-hole the card until it was deleted by
/// hand. Defaults cover a missing file, so writing is best-effort by design.
#[cfg(not(target_arch = "wasm32"))]
fn write_state(state: &UpdateState) -> Result<(), ServerFnError> {
    use std::io::Write;

    let path = update_state_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| ServerFnError::new("更新状态目录无效"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| ServerFnError::new(format!("创建更新状态目录失败: {error}")))?;
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() {
            return Err(ServerFnError::new("更新状态路径不能是符号链接"));
        }
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| ServerFnError::new(format!("序列化更新状态失败: {error}")))?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = std::fs::File::create(&temporary)
        .map_err(|error| ServerFnError::new(format!("写入更新状态失败: {error}")))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&temporary);
        return Err(ServerFnError::new(format!("写入更新状态失败: {error}")));
    }
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::rename(&temporary, &path)
        .map_err(|error| ServerFnError::new(format!("保存更新状态失败: {error}")))?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn state_lock() -> &'static tokio::sync::Mutex<()> {
    use std::sync::OnceLock;

    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proxy_core::ReleaseKind;

    fn target(kind: ReleaseKind, os: &str, arch: &str) -> ReleaseTarget {
        ReleaseTarget {
            kind,
            os: os.to_string(),
            arch: arch.to_string(),
        }
    }

    fn release(tag: &str, assets: &[&str]) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            name: Some(format!("Release {tag}")),
            body: Some("- something new".to_string()),
            html_url: Some(format!("https://github.example/{tag}")),
            published_at: Some("2026-11-03T00:00:00Z".to_string()),
            prerelease: false,
            assets: assets
                .iter()
                .map(|name| proxy_core::GitHubReleaseAsset {
                    name: (*name).to_string(),
                    size: 1024,
                    browser_download_url: format!("https://download.example/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn a_check_is_due_once_a_day_has_passed() {
        // Never checked is always due, whatever the clock says.
        assert!(should_check(0, 0));
        assert!(should_check(1_730_000_000, 0));
        assert!(!should_check(1_000, 1_000));
        // One second short of the interval is still throttled.
        assert!(!should_check(UPDATE_CHECK_INTERVAL_SECS, 1));
        assert!(should_check(UPDATE_CHECK_INTERVAL_SECS + 1, 1));
        // A clock that jumped backwards must not wedge the check forever.
        assert!(should_check(500, 10_000));
    }

    #[test]
    fn the_macos_release_offers_the_dmg_not_the_web_archive() {
        let release = release(
            "v1.0.9",
            &[
                "Kitty-Pro-web-macos.tar.gz",
                "Kitty-Pro-arm64-v8a.apk",
                "KittyPro_1.0.9_aarch64.dmg",
                "KittyPro_1.0.9_aarch64.dmg.sha256",
            ],
        );
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let app = release_for(&release, &current).expect("release should be usable");

        assert_eq!(app.version, "1.0.9");
        assert_eq!(app.asset.as_deref(), Some("KittyPro_1.0.9_aarch64.dmg"));
        assert_eq!(app.asset_size, 1024);
        assert_eq!(app.notes.as_deref(), Some("- something new"));

        let status = status_for(
            &current,
            "1.0.8",
            &UpdateState {
                latest: Some(app),
                ..UpdateState::default()
            },
            None,
        );
        assert_eq!(status.state, AppUpdateState::Available);
        assert!(!status.snoozed);
    }

    #[test]
    fn being_up_to_date_is_reported_as_such() {
        let release = release("v1.0.9", &["KittyPro_1.0.9_aarch64.dmg"]);
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let app = release_for(&release, &current).expect("release should be usable");
        let status = status_for(
            &current,
            "1.0.9",
            &UpdateState {
                latest: Some(app),
                ..UpdateState::default()
            },
            None,
        );
        assert_eq!(status.state, AppUpdateState::UpToDate);
    }

    #[test]
    fn upgrading_the_app_clears_a_cached_prompt_without_a_new_check() {
        let release = release("v1.0.9", &["KittyPro_1.0.9_aarch64.dmg"]);
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let app = release_for(&release, &current).expect("release should be usable");
        let state = UpdateState {
            latest: Some(app),
            ..UpdateState::default()
        };

        assert_eq!(
            status_for(&current, "1.0.8", &state, None).state,
            AppUpdateState::Available
        );
        assert_eq!(
            status_for(&current, "1.0.9", &state, None).state,
            AppUpdateState::UpToDate
        );
    }

    #[test]
    fn a_release_without_a_package_for_this_target_says_so() {
        let release = release("v1.1.0", &["Kitty-Pro-windows-x86_64.msi"]);
        let current = target(ReleaseKind::Desktop, "linux", "aarch64");
        let app = release_for(&release, &current).expect("release should be usable");

        // The release exists and is newer, but there is nothing to hand over.
        assert_eq!(app.asset, None);
        let status = status_for(
            &current,
            "1.0.9",
            &UpdateState {
                latest: Some(app),
                ..UpdateState::default()
            },
            None,
        );
        assert_eq!(status.state, AppUpdateState::Available);
        assert_eq!(status.target, "桌面版 · linux · aarch64");
    }

    #[test]
    fn ignoring_a_version_holds_until_the_next_release() {
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let state = UpdateState {
            latest: Some(
                release_for(
                    &release("v1.0.9", &["KittyPro_1.0.9_aarch64.dmg"]),
                    &current,
                )
                .expect("fixture"),
            ),
            snoozed_version: "1.0.9".to_string(),
            ..UpdateState::default()
        };

        let status = status_for(&current, "1.0.8", &state, None);
        assert!(status.snoozed);

        let newer = UpdateState {
            latest: Some(
                release_for(
                    &release("v1.0.10", &["KittyPro_1.0.10_aarch64.dmg"]),
                    &current,
                )
                .expect("fixture"),
            ),
            snoozed_version: state.snoozed_version.clone(),
            ..UpdateState::default()
        };
        let status = status_for(&current, "1.0.8", &newer, None);
        assert!(!status.snoozed);
        assert_eq!(status.state, AppUpdateState::Available);
    }

    #[test]
    fn a_failed_check_keeps_the_previous_answer_visible() {
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let state = UpdateState {
            latest: Some(
                release_for(
                    &release("v1.0.9", &["KittyPro_1.0.9_aarch64.dmg"]),
                    &current,
                )
                .expect("fixture"),
            ),
            ..UpdateState::default()
        };

        let failed = status_for(&current, "1.0.8", &state, Some("timeout".to_string()));
        assert_eq!(failed.state, AppUpdateState::Available);
        assert_eq!(failed.error.as_deref(), Some("timeout"));

        let empty = UpdateState {
            settings: UpdateSettings {
                auto_check: true,
                last_check_at: 42,
            },
            ..UpdateState::default()
        };
        let failed = status_for(&current, "1.0.8", &empty, Some("timeout".to_string()));
        assert_eq!(failed.state, AppUpdateState::Failed);
        assert_eq!(failed.last_check_at, 42);
    }

    #[test]
    fn a_release_with_an_unreadable_tag_is_dropped() {
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        for tag in ["", "latest", "v", "1.2.3.4"] {
            assert!(
                release_for(&release(tag, &["KittyPro_1.0.9_aarch64.dmg"]), &current).is_none(),
                "tag {tag:?} should not produce an update"
            );
        }
    }

    #[test]
    fn only_an_http_release_page_ever_becomes_the_link() {
        // `html_url` is a field of a fetched document and lands in the card's
        // primary `href`, so a non-web scheme must not reach the anchor.
        assert_eq!(
            release_url(Some("https://github.example/v1.0.9")),
            "https://github.example/v1.0.9"
        );
        assert_eq!(
            release_url(Some("  http://github.example/v1.0.9  ")),
            "http://github.example/v1.0.9"
        );
        // Case-insensitive scheme, but a lookalike is still rejected.
        assert_eq!(
            release_url(Some("HTTPS://github.example/v1.0.9")),
            "HTTPS://github.example/v1.0.9"
        );
        let fallback = format!("https://github.com/{UPDATE_REPO}/releases");
        for hostile in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "file:///etc/passwd",
            "data:text/html;base64,PHNjcmlwdD4=",
            "httpsx://github.example",
            "https:",
            "//github.example/v1.0.9",
            "",
            "   ",
        ] {
            assert_eq!(
                release_url(Some(hostile)),
                fallback,
                "{hostile:?} must not become a link"
            );
        }
        assert_eq!(release_url(None), fallback);

        // End to end: the card reads this off `AppRelease::url`.
        let mut fixture = release("v1.0.9", &["KittyPro_1.0.9_aarch64.dmg"]);
        fixture.html_url = Some("javascript:alert(1)".to_string());
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let app = release_for(&fixture, &current).expect("fixture");
        assert_eq!(app.url, fallback);
    }

    #[test]
    fn the_state_file_round_trips_and_defaults_are_safe() {
        let written = UpdateState {
            settings: UpdateSettings {
                auto_check: false,
                last_check_at: 100,
            },
            latest: Some(AppRelease {
                version: "1.0.9".to_string(),
                title: "v1.0.9".to_string(),
                url: "https://github.example/v1.0.9".to_string(),
                notes: Some("- new".to_string()),
                published_at: None,
                prerelease: false,
                asset: Some("KittyPro_1.0.9_aarch64.dmg".to_string()),
                asset_size: 7,
            }),
            snoozed_version: String::new(),
        };
        let json = serde_json::to_string(&written).expect("state should serialize");
        let read: UpdateState = serde_json::from_str(&json).expect("state should deserialize");
        assert_eq!(read, written);

        // A missing or hand-edited file must decode to something that never
        // claims an update and never reports a failure.
        let bare: UpdateState = serde_json::from_str("{}").expect("empty state should decode");
        assert_eq!(bare, UpdateState::default());
        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let status = status_for(&current, "1.0.9", &bare, None);
        assert_eq!(status.state, AppUpdateState::Idle);
        assert!(bare.settings.last_check_at == 0 && should_check(0, bare.settings.last_check_at));
    }

    #[test]
    fn a_half_written_state_file_falls_back_instead_of_failing_forever() {
        // An interrupted write leaves a file that parses as nothing. Erroring on
        // it would fail every later read the same way, stranding the card on
        // "尚未检查" until somebody deleted the file by hand.
        let parsed: Result<UpdateState, _> = serde_json::from_slice(b"{ broken");
        let recovered = parsed.unwrap_or_default();
        assert_eq!(recovered, UpdateState::default());

        let current = target(ReleaseKind::Desktop, "macos", "aarch64");
        let status = status_for(&current, "1.0.9", &recovered, None);
        assert_eq!(status.state, AppUpdateState::Idle);
        // Idle with no clock means the card offers a check rather than claiming
        // an age it has no way to know.
        assert_eq!(status.last_check_at, 0);
        assert!(should_check(1_000, recovered.settings.last_check_at));
    }

    #[test]
    fn auto_check_defaults_on_but_survives_being_turned_off() {
        assert!(UpdateSettings::default().auto_check);
        let restored: UpdateSettings =
            serde_json::from_str(r#"{"auto_check":false,"last_check_at":9}"#)
                .expect("settings should decode");
        assert!(!restored.auto_check);
        assert_eq!(restored.last_check_at, 9);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod live {
    use super::*;
    use proxy_core::ReleaseKind;

    /// Pull the real newest release and show what each shell would be offered.
    ///
    /// This is the drift guard for asset naming: rename a package in the release
    /// workflow and the matching target starts printing `asset: None` here.
    #[tokio::test]
    #[ignore = "requires live network access to the GitHub Releases API"]
    async fn live_release_offers_one_package_per_shipped_platform() {
        let targets = [
            ("Desktop", ReleaseKind::Desktop, "macos", "aarch64"),
            ("Desktop", ReleaseKind::Desktop, "windows", "x86_64"),
            ("Desktop", ReleaseKind::Desktop, "linux", "x86_64"),
            ("Mobile", ReleaseKind::Android, "android", "aarch64"),
            ("Web Control", ReleaseKind::WebServer, "linux", "x86_64"),
        ];
        let mut unhandled = Vec::new();
        for (platform, kind, os, arch) in targets {
            let target = ReleaseTarget {
                kind,
                os: os.to_string(),
                arch: arch.to_string(),
            };
            let detected = ReleaseTarget::detect(platform).expect("platform should be updatable");
            assert_eq!(detected.kind, target.kind, "kind for {platform}");
            let release = fetch_release(&target)
                .await
                .expect("live check should succeed")
                .expect("the repository publishes releases");
            eprintln!(
                "{platform} {os}/{arch} -> v{} {:?}",
                release.version, release.asset
            );
            if release.asset.is_none() {
                unhandled.push(target.label());
            }
            assert!(
                proxy_core::is_newer_version(env!("CARGO_PKG_VERSION"), &release.version)
                    || release.version == env!("CARGO_PKG_VERSION"),
                "live release v{} should not be older than this build",
                release.version
            );
        }
        assert!(
            unhandled.is_empty(),
            "no package matched: {}",
            unhandled.join(", ")
        );
    }
}
