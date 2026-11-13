//! Wire format, asset selection, and version comparison for the update check.
//!
//! The check asks GitHub for the repository's newest release, so one endpoint
//! answers every shell: each release carries one installer per platform (`.dmg`,
//! `.msi`, `.zip`, `.AppImage`, `.deb`, `.tar.gz`, `.apk`) plus a `.sha256`
//! sibling for each. Everything about that answer which has to agree between the
//! native fetcher and the settings page lives here - the deserializable shape,
//! which asset belongs to the running install, and what "newer" means - so the
//! HTTP layer stays a transport.
//!
//! Pure and dependency-free on purpose: the UI crate compiles this for `wasm32`
//! too, and these rules are worth pinning down with tests rather than by clicking
//! through a release page.

use serde::{Deserialize, Serialize};

/// How often a background check may contact GitHub.
///
/// A release ships at most every few days, so a shorter interval only spends the
/// user's data and burns the API's anonymous rate limit. The manual button in the
/// settings page ignores this.
pub const UPDATE_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Extensions that name something a user can install.
const INSTALLER_SUFFIXES: &[&str] = &[
    ".apk",
    ".appimage",
    ".deb",
    ".dmg",
    ".exe",
    ".msi",
    ".tar.gz",
    ".zip",
];
/// Suffixes that always mean "not an installer", however they are spelled.
const NON_INSTALLER_SUFFIXES: &[&str] = &[".sha256", ".sha512", ".asc", ".sig"];

/// Names a whole `-`/`_`/`.`-delimited segment of an asset name.
///
/// Substring tests are wrong here: `web` has to mean the self-hosted server, and
/// a name like `Kitty-Pro-webkit-x64.dmg` or `kitty-pro-resource-bag.tar.gz`
/// would otherwise be misclassified by a longer word that happens to contain the
/// token. Asset names in this project are delimited that way already, so an
/// exact segment is both stricter and what the naming scheme actually promises.
fn names_segment(name: &str, token: &str) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|segment| segment == token)
}

/// Which kind of installation a release asset replaces.
///
/// The three shells update differently - a desktop package, an APK, a server
/// archive - so an asset of the wrong kind is never a candidate. The operating
/// system and CPU are matched separately, because a Windows x86_64 `.msi` and a
/// Linux x86_64 `.deb` agree on the CPU and must still never be swapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKind {
    Desktop,
    Android,
    WebServer,
}

impl ReleaseKind {
    /// Kind of the running shell, from the platform string the UI reports.
    ///
    /// `None` for a shell this check cannot serve, which the caller reports as
    /// unsupported instead of offering somebody else's binary.
    pub fn from_platform(platform: &str) -> Option<Self> {
        match platform.trim().to_ascii_lowercase().as_str() {
            "android" | "mobile" => Some(Self::Android),
            "web" | "web control" | "web-server" => Some(Self::WebServer),
            "desktop" | "macos" | "darwin" | "windows" | "linux" => Some(Self::Desktop),
            _ => None,
        }
    }

    /// Kind an asset name declares, `None` when it declares none.
    pub fn from_asset_name(name: &str) -> Option<Self> {
        let name = name.to_ascii_lowercase();
        if !is_installable_name(&name) {
            return None;
        }
        // The self-hosted control server is the only kind whose name carries this
        // token, and it must never reach a desktop install.
        if names_segment(&name, "web") {
            return Some(Self::WebServer);
        }
        if name.ends_with(".apk") || names_segment(&name, "android") {
            return Some(Self::Android);
        }
        // A bare `.zip` that names no operating system is ambiguous, so it is
        // declined rather than guessed at: every desktop asset here names its OS.
        declared_os_tokens(&name).is_some().then_some(Self::Desktop)
    }

    /// Human-readable label for the settings page.
    pub fn label(self) -> &'static str {
        match self {
            Self::Desktop => "桌面版",
            Self::Android => "Android",
            Self::WebServer => "Web 服务端",
        }
    }
}

/// Operating systems an asset name can declare.
const OS_TOKENS: &[(&str, &[&str])] = &[
    ("macos", &["macos", "darwin", "osx", ".dmg"]),
    ("windows", &["windows", ".msi", ".exe"]),
    ("linux", &["linux", "ubuntu", ".deb", ".appimage"]),
    ("android", &["android", ".apk"]),
];

/// Every OS a name declares; `None` when it declares none.
fn declared_os_tokens(name: &str) -> Option<Vec<&'static str>> {
    let found = OS_TOKENS
        .iter()
        .filter(|(_, tokens)| tokens.iter().any(|token| name.contains(token)))
        .map(|(os, _)| *os)
        .collect::<Vec<_>>();
    (!found.is_empty()).then_some(found)
}

/// CPU groups an asset name can declare, with every spelling each goes by.
///
/// Matched as substrings, which is why none of these may be a substring of
/// another group's spelling: `arm` is missing from the `armv7` row for exactly
/// that reason, and [`arch_group`] maps the bare `arm` a running 32-bit process
/// reports to this group instead.
const ARCH_GROUPS: &[(&str, &[&str])] = &[
    ("aarch64", &["aarch64", "arm64", "armv8", "v8a"]),
    ("x86_64", &["x86_64", "amd64", "x64"]),
    ("armv7", &["armv7", "armeabi", "arm32"]),
];

/// Spellings a running process uses for a CPU no asset name would use, mapped to
/// the group they belong to. `std::env::consts::ARCH` is the only input here.
const ARCH_ALIASES: &[(&str, &str)] = &[("arm", "armv7")];

/// The installation being checked: what the running shell is, per compile-time
/// constants rather than anything the network can influence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseTarget {
    pub kind: ReleaseKind,
    /// `std::env::consts::OS` of the process that will be replaced.
    pub os: String,
    /// `std::env::consts::ARCH` of the same process.
    pub arch: String,
}

impl ReleaseTarget {
    /// Build the target from the UI's platform string plus the native process's
    /// own OS/CPU. Returns `None` when the running shell is not updatable.
    pub fn detect(platform: &str) -> Option<Self> {
        let kind = ReleaseKind::from_platform(platform)?;
        Some(Self {
            kind,
            os: normalize_os(std::env::consts::OS),
            arch: std::env::consts::ARCH.to_string(),
        })
    }

    /// Label used in the "no build for this system" message.
    pub fn label(&self) -> String {
        format!("{} · {} · {}", self.kind.label(), self.os, self.arch)
    }
}

/// `std::env::consts::OS` spells the same systems the asset names use, except for
/// macOS, so only that one needs aligning.
fn normalize_os(os: &str) -> String {
    match os.trim().to_ascii_lowercase().as_str() {
        "macos" | "darwin" => "macos".to_string(),
        other => other.to_string(),
    }
}

/// Whether a name is an installer rather than a checksum or a source bag.
fn is_installable_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if NON_INSTALLER_SUFFIXES.iter().any(|s| name.ends_with(s)) {
        return false;
    }
    // GitHub's tarball of the source tree shares `.tar.gz` with the web archives.
    // Segmented so a name that merely contains the word - `resource`,
    // `open-source-audit` - is not mistaken for the source bag.
    if names_segment(&name, "source") || names_segment(&name, "sources") {
        return false;
    }
    INSTALLER_SUFFIXES
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// Normalize a `std::env::consts::ARCH`-style string to a group name.
pub fn arch_group(arch: &str) -> Option<&'static str> {
    let arch = arch.trim().to_ascii_lowercase();
    // Rust spells 32-bit ARM `arm`, which is deliberately not a token in
    // `ARCH_GROUPS`: it is a substring of both `arm64` and `aarch64`, so adding it
    // there would make every 64-bit package claim to fit an armv7 device. The two
    // sides therefore spell one CPU differently, and this is where they are
    // reconciled. Without it `arch_group("arm")` answers `None`, and
    // `asset_matches_target` treats an unknown CPU as "matches nothing" — so an
    // armv7 phone is quietly told there is no build for it, forever, however the
    // asset is named.
    if let Some(group) = ARCH_ALIASES
        .iter()
        .find_map(|(spelling, group)| (*spelling == arch.as_str()).then_some(*group))
    {
        return Some(group);
    }
    ARCH_GROUPS.iter().find_map(|(group, tokens)| {
        (*group == arch.as_str() || tokens.contains(&arch.as_str())).then_some(*group)
    })
}

/// Groups an asset name declares; empty when it declares none.
fn listed_arch_groups(name: &str) -> Vec<&'static str> {
    let name = name.to_ascii_lowercase();
    ARCH_GROUPS
        .iter()
        .filter(|(_, tokens)| tokens.iter().any(|token| name.contains(token)))
        .map(|(group, _)| *group)
        .collect()
}

/// Whether an asset is an installer for one target installation.
///
/// Names are the only signal the release API gives, so an asset must agree on the
/// kind, the operating system, and the CPU. Anything that cannot be classified is
/// declined: a silent "no update" beats a binary that cannot run. An asset that
/// declares nothing about a dimension is treated as covering it (a universal or
/// single-build release).
pub fn asset_matches_target(asset_name: &str, target: &ReleaseTarget) -> bool {
    if ReleaseKind::from_asset_name(asset_name) != Some(target.kind) {
        return false;
    }
    let name = asset_name.to_ascii_lowercase();
    if let Some(oses) = declared_os_tokens(&name) {
        // Android and desktop assets both name their OS; the web archives name
        // the OS their server binary runs on, so the rule is uniform.
        if !oses.contains(&target.os.as_str()) {
            return false;
        }
    } else if target.kind == ReleaseKind::Desktop {
        // Desktop installers are always per-OS here; an unlabelled one would be
        // a naming regression, and guessing is how a Mac ends up with an MSI.
        return false;
    }
    // "Universal" spans the CPUs of one operating system, never the operating
    // systems themselves, so it is consulted only after the OS has agreed: a
    // `Kitty-Pro-universal.AppImage` is still not something a Mac should install.
    if name.contains("universal") {
        return true;
    }
    let listed = listed_arch_groups(&name);
    if listed.is_empty() {
        return true;
    }
    match arch_group(&target.arch) {
        Some(local) => listed.contains(&local),
        None => false,
    }
}

/// Prefer the asset a user should actually run when a platform publishes several:
/// an installer over a portable archive over anything else.
fn asset_priority(name: &str) -> u8 {
    let name = name.to_ascii_lowercase();
    if [".msi", ".dmg", ".apk"].iter().any(|s| name.ends_with(s)) {
        0
    } else if [".appimage", ".deb"].iter().any(|s| name.ends_with(s)) {
        1
    } else {
        2
    }
}

/// The asset this installation should download, `None` when the release has none.
pub fn select_asset<'a>(
    assets: &'a [GitHubReleaseAsset],
    target: &ReleaseTarget,
) -> Option<&'a GitHubReleaseAsset> {
    assets
        .iter()
        .filter(|asset| asset_matches_target(&asset.name, target))
        .min_by_key(|asset| asset_priority(&asset.name))
}

/// One release as returned by `GET /repos/{owner}/{repo}/releases/latest`.
///
/// Only the fields the check reads are declared; serde ignores the rest of the
/// (large) GitHub payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<GitHubReleaseAsset>,
}

impl GitHubRelease {
    /// Release notes trimmed for a dialog.
    ///
    /// GitHub's own changelog footer is a compare link the dialog already offers
    /// as a button, so it is dropped instead of rendered as raw Markdown.
    pub fn notes(&self) -> Option<String> {
        let body = self.body.as_deref()?.trim();
        if body.is_empty() {
            return None;
        }
        let trimmed = body
            .lines()
            .filter(|line| !line.trim_start().starts_with("**Full Changelog**"))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }
}

/// One file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    pub browser_download_url: String,
}

/// Seconds since the Unix epoch for the timestamps GitHub emits in release
/// payloads (`2026-11-03T11:10:46Z`).
///
/// Read as UTC: a trailing designator or offset is accepted and dropped, which is
/// exact for `Z` stamps and at most a few hours out on an offset one - close
/// enough for a "released 3 days ago" label, which is the only thing that reads
/// this. `None` for anything unrecognizable, so a caller can say "unknown"
/// instead of inventing a date.
///
/// Handwritten rather than pulled in as a date crate: this compiles into the
/// `wasm32` client, which has no clock of its own and pays for every dependency
/// twice.
pub fn unix_from_rfc3339(value: &str) -> Option<u64> {
    let value = value.trim();
    let (date, time) = value.split_once(['T', 't'])?;
    // `11:10:46Z`, `11:10:46.123Z`, and `11:10:46+08:00` all start with the same
    // three fields, so everything after them is dropped. A negative offset leaves
    // an unparseable hour, which is rejected below rather than misread.
    let time = time.split(['Z', 'z', '+', '.']).next().unwrap_or(time);
    let mut dates = date.split('-').map(|part| part.parse::<i64>().ok());
    let (Some(Some(year)), Some(Some(month)), Some(Some(day))) =
        (dates.next(), dates.next(), dates.next())
    else {
        return None;
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Bounded so an absurd year cannot overflow the day count below. A stamp this
    // far outside the range of real releases is a malformed value, and the only
    // thing that reads it renders differences of hours and days.
    if !(1970..=9999).contains(&year) {
        return None;
    }
    let mut times = time.split(':').map(|part| part.parse::<u64>().ok());
    let (Some(Some(hour)), Some(Some(minute)), Some(Some(second))) =
        (times.next(), times.next(), times.next())
    else {
        return None;
    };
    // A leap second is folded into the last minute rather than rejected.
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let seconds = days_from_civil(year, month, day)
        .checked_mul(86_400)?
        .checked_add((hour * 3_600 + minute * 60 + second) as i64)?;
    u64::try_from(seconds).ok()
}

/// Days since the Unix epoch for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`, which stays correct across century boundaries).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A release is newer when its version is greater, or when the versions match but
/// the running build is a pre-release of it.
///
/// `current` is the running shell's package version, which may carry a
/// pre-release tag (`1.1.0-rc.1`); `latest` is a release tag. Unparseable input
/// never counts as an update: a malformed tag must not nag on every launch.
pub fn is_newer_version(current: &str, latest: &str) -> bool {
    let Some(current) = ReleaseVersion::parse(current) else {
        return false;
    };
    let Some(latest) = ReleaseVersion::parse(latest) else {
        return false;
    };
    latest > current
}

/// A released version with enough structure to order two of them.
///
/// Handwritten instead of pulling in `semver`: this crate is compiled into the
/// `wasm32` client and the Android shell as well, and the ordering only has to
/// cover the tags this project publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    /// Pre-release identifiers, empty for a final release. Ordered by the usual
    /// rules: a pre-release sorts before its final version, and identifiers
    /// compare numerically when both sides are numeric, lexically otherwise.
    pre: Vec<PreIdentifier>,
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                // `1.1.0` is newer than `1.1.0-rc.1`: a pre-release promises a
                // version that has not shipped yet.
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PreIdentifier {
    Numeric(u64),
    Alpha(String),
}

impl ReleaseVersion {
    /// Parse `1.2.3`, `v1.2.3`, or `1.2.3-rc.1+build.7`.
    ///
    /// Build metadata is dropped: it never decides which binary to install. A
    /// missing minor or patch reads as zero, so `v2` is `2.0.0`.
    pub fn parse(value: &str) -> Option<Self> {
        let mut value = value.trim();
        value = value.strip_prefix(['v', 'V']).unwrap_or(value);
        if value.is_empty() {
            return None;
        }
        // Build metadata goes first because it can follow a pre-release: in
        // `1.1.0-rc.1+build.7` only `rc.1` is the pre-release.
        if let Some((rest, build)) = value.split_once('+') {
            if build.is_empty() {
                return None;
            }
            value = rest;
        }
        let (numbers, pre) = match value.split_once('-') {
            // A trailing `-` is a typo, not an unlabelled pre-release.
            Some((numbers, pre)) => (numbers, (!pre.is_empty()).then_some(pre)?),
            None => (value, ""),
        };
        // Missing components read as zero, a non-numeric one rejects the tag.
        let mut parts = numbers.split('.');
        let mut numbers = [0_u64; 3];
        for slot in numbers.iter_mut() {
            let Some(part) = parts.next() else { break };
            *slot = parse_number(part)?;
        }
        // `1.2.3.4` is rejected rather than silently read as `1.2.3`: a tag this
        // function does not understand should not order at all.
        if parts.next().is_some() {
            return None;
        }
        let [major, minor, patch] = numbers;
        let pre = if pre.is_empty() {
            Vec::new()
        } else {
            pre.split('.')
                .map(|identifier| {
                    // `rc..1` has an empty identifier and is not a version.
                    if identifier.is_empty() {
                        return None;
                    }
                    Some(match identifier.parse::<u64>() {
                        Ok(number) => PreIdentifier::Numeric(number),
                        Err(_) => PreIdentifier::Alpha(identifier.to_ascii_lowercase()),
                    })
                })
                .collect::<Option<Vec<_>>>()?
        };
        Some(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

fn parse_number(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The asset list of the live `v1.0.9` release, verbatim, so the matcher is
    /// tested against the names this project really publishes.
    const RELEASE_ASSETS: &[&str] = &[
        "Kitty-Pro-arm64-v8a.apk",
        "Kitty-Pro-arm64-v8a.apk.sha256",
        "Kitty-Pro-web-linux-x86_64.tar.gz",
        "Kitty-Pro-web-linux-x86_64.tar.gz.sha256",
        "Kitty-Pro-web-macos.tar.gz",
        "Kitty-Pro-web-macos.tar.gz.sha256",
        "Kitty-Pro-windows-x86_64.msi",
        "Kitty-Pro-windows-x86_64.msi.sha256",
        "Kitty-Pro-windows-x86_64.zip",
        "Kitty-Pro-windows-x86_64.zip.sha256",
        "kitty-pro_1.0.9_amd64.deb",
        "kitty-pro_1.0.9_x86_64.AppImage",
        "KittyPro_1.0.9_aarch64.dmg",
    ];

    fn fixtures() -> Vec<GitHubReleaseAsset> {
        RELEASE_ASSETS
            .iter()
            .map(|name| GitHubReleaseAsset {
                name: (*name).to_string(),
                size: 1,
                browser_download_url: format!("https://example.invalid/{name}"),
            })
            .collect()
    }

    fn target(kind: ReleaseKind, os: &str, arch: &str) -> ReleaseTarget {
        ReleaseTarget {
            kind,
            os: os.to_string(),
            arch: arch.to_string(),
        }
    }

    fn select(kind: ReleaseKind, os: &str, arch: &str) -> Option<String> {
        select_asset(&fixtures(), &target(kind, os, arch)).map(|asset| asset.name.clone())
    }

    /// The names `nsh package` produces, which the release workflow renames to.
    /// `Desktop` installs come from the NSH host on every platform, so this is the
    /// set an update check really sees from here on.
    fn select_from(kind: ReleaseKind, os: &str, arch: &str, names: &[&str]) -> Option<String> {
        let assets: Vec<GitHubReleaseAsset> = names
            .iter()
            .map(|name| GitHubReleaseAsset {
                name: (*name).to_string(),
                size: 1,
                browser_download_url: format!("https://example.invalid/{name}"),
            })
            .collect();
        select_asset(&assets, &target(kind, os, arch)).map(|asset| asset.name.clone())
    }

    #[test]
    fn the_native_shell_package_names_select_their_own_platform() {
        let names = [
            "Kitty-Pro-macos-aarch64.zip",
            "Kitty-Pro-windows-x86_64.zip",
            "Kitty-Pro-linux-x86_64.tar.gz",
        ];
        for (os, arch, expected) in [
            ("macos", "aarch64", "Kitty-Pro-macos-aarch64.zip"),
            ("windows", "x86_64", "Kitty-Pro-windows-x86_64.zip"),
            ("linux", "x86_64", "Kitty-Pro-linux-x86_64.tar.gz"),
        ] {
            assert_eq!(
                select_from(ReleaseKind::Desktop, os, arch, &names).as_deref(),
                Some(expected),
                "{os}/{arch} must get its own archive"
            );
        }
    }

    #[test]
    fn a_bare_app_bundle_is_never_offered_as_an_installer() {
        // NSH emits `Kitty Pro.app` next to its zip. Publishing the directory's
        // name would hand a user something no installer can open.
        assert_eq!(ReleaseKind::from_asset_name("Kitty Pro.app"), None);
    }

    /// Every name `.github/workflows/release.yml` publishes, and what each platform
    /// is handed. The workflow's header says an edit to a name there must be an edit
    /// here too: these are the names, so the pairing has one place to fail. It is not
    /// the frozen `RELEASE_ASSETS` fixture above, which records what v1.0.9 really
    /// shipped and must keep doing that — a matcher that still serves an old release
    /// is a property worth testing on its own.
    ///
    /// Written from the workflow's own `archive_name`s, the desktop job's
    /// `Kitty-Pro-<id>-<arch>` template and `build-android.sh`'s
    /// `<app>-<abis>-<version>.apk`, so a rename that only touches one of the three
    /// breaks here rather than on a user's device.
    #[test]
    fn the_published_asset_set_answers_every_platform() {
        let names = [
            // macOS publishes both halves of the same bundle: the archive this
            // workflow has always had to write around NSH's bare `.app`, and the image
            // a person drags onto Applications. The zip is deliberately listed first:
            // `select_asset` breaks a tie by order, so an expectation of the dmg can
            // only be met by `asset_priority` ranking an installer above an archive.
            // With the dmg in front this would assert nothing but which string happens
            // to be first in this array, and it did just that until it was swapped.
            "Kitty-Pro-macos-aarch64.zip",
            "Kitty-Pro-macos-aarch64.dmg",
            "Kitty-Pro-windows-x86_64.zip",
            "Kitty-Pro-linux-x86_64.tar.gz",
            "Kitty-Pro-web-linux-x86_64.tar.gz",
            "Kitty-Pro-web-macos-aarch64.tar.gz",
            "Kitty-Pro-arm64-v8a-armeabi-v7a-1.0.16.apk",
            // The sidecars ride along on the release page and must never be chosen.
            "Kitty-Pro-macos-aarch64.dmg.sha256",
            "Kitty-Pro-macos-aarch64.zip.sha256",
            "Kitty-Pro-arm64-v8a-armeabi-v7a-1.0.16.apk.sha256",
        ];
        for (kind, os, arch, expected) in [
            (
                ReleaseKind::Desktop,
                "macos",
                "aarch64",
                "Kitty-Pro-macos-aarch64.dmg",
            ),
            (
                ReleaseKind::Desktop,
                "windows",
                "x86_64",
                "Kitty-Pro-windows-x86_64.zip",
            ),
            (
                ReleaseKind::Desktop,
                "linux",
                "x86_64",
                "Kitty-Pro-linux-x86_64.tar.gz",
            ),
            (
                ReleaseKind::WebServer,
                "linux",
                "x86_64",
                "Kitty-Pro-web-linux-x86_64.tar.gz",
            ),
            (
                ReleaseKind::WebServer,
                "macos",
                "aarch64",
                "Kitty-Pro-web-macos-aarch64.tar.gz",
            ),
            (
                ReleaseKind::Android,
                "android",
                "aarch64",
                "Kitty-Pro-arm64-v8a-armeabi-v7a-1.0.16.apk",
            ),
            // `std::env::consts::ARCH` on a 32-bit Android build is `arm`, which is
            // why ARCH_ALIASES exists at all.
            (
                ReleaseKind::Android,
                "android",
                "arm",
                "Kitty-Pro-arm64-v8a-armeabi-v7a-1.0.16.apk",
            ),
        ] {
            assert_eq!(
                select_from(kind, os, arch, &names).as_deref(),
                Some(expected),
                "{os}/{arch} running the {} channel",
                kind.label()
            );
        }
        // That each name answers only its own channel is `kinds_do_not_cross_contaminate`'s
        // job; this test pins the set, not the boundaries.
    }

    /// `scripts/build-android.sh` names the APK after the ABIs it carries, and this is
    /// the check that the name means what it says. One fat APK serves both CPUs, so
    /// each has to match; the point of the test is the third case, because an APK
    /// without ARM libraries must never be offered to an ARM device — the file
    /// downloads, the package manager refuses it, and the card that promised an update
    /// has left the user with nothing.
    #[test]
    fn a_multi_abi_apk_names_every_cpu_it_carries() {
        let fat = "Kitty-Pro-arm64-v8a-armeabi-v7a-0.1.0.apk";
        assert!(
            asset_matches_target(fat, &target(ReleaseKind::Android, "android", "aarch64")),
            "an arm64-v8a device must accept the APK that carries its library"
        );
        assert!(
            asset_matches_target(fat, &target(ReleaseKind::Android, "android", "arm")),
            "an armv7 device must accept the APK that carries armeabi-v7a"
        );
        assert!(
            !asset_matches_target(fat, &target(ReleaseKind::Android, "android", "x86_64")),
            "an x86_64 device must not be offered an APK with no library for it"
        );
        // The single-ABI spelling the script also produces, e.g. `--abi arm64-v8a`.
        let one = "Kitty-Pro-arm64-v8a-0.1.0.apk";
        assert!(asset_matches_target(
            one,
            &target(ReleaseKind::Android, "android", "aarch64")
        ));
        assert!(!asset_matches_target(
            one,
            &target(ReleaseKind::Android, "android", "arm")
        ));
        // No CPU token at all reads as "fits anything", which is exactly why the
        // script does not emit such a name. Recorded so the behaviour is not
        // mistaken for a bug later.
        let unnamed = "Kitty-Pro-0.1.0.apk";
        for arch in ["aarch64", "arm", "x86_64"] {
            assert!(
                asset_matches_target(unnamed, &target(ReleaseKind::Android, "android", arch)),
                "an unlabelled APK matches every CPU, which is {arch}'s problem too"
            );
        }
    }

    /// What a phone sees while the release workflow publishes only desktops, which is
    /// the state this repository is in. Worth a test because the answer has to be
    /// *nothing*, not the nearest thing: offering a `.zip` of a macOS app to a phone
    /// is worse than saying there is no build, and the settings card's
    /// "nothing to install" depends on this rather than on any filter upstream.
    #[test]
    fn a_release_without_an_apk_offers_a_phone_nothing() {
        let desktop_only = [
            "Kitty-Pro-macos-aarch64.zip",
            "Kitty-Pro-windows-x86_64.zip",
            "Kitty-Pro-linux-x86_64.tar.gz",
        ];
        for arch in ["aarch64", "arm", "x86_64"] {
            assert_eq!(
                select_from(ReleaseKind::Android, "android", arch, &desktop_only),
                None,
                "{arch} must not be offered a desktop archive"
            );
        }
    }

    #[test]
    fn every_platform_gets_exactly_its_own_installer() {
        assert_eq!(
            select(ReleaseKind::Desktop, "macos", "aarch64").as_deref(),
            Some("KittyPro_1.0.9_aarch64.dmg")
        );
        assert_eq!(
            select(ReleaseKind::Desktop, "windows", "x86_64").as_deref(),
            // Windows publishes an installer and a portable archive: the
            // installer wins.
            Some("Kitty-Pro-windows-x86_64.msi")
        );
        // Linux publishes two equally-preferred packages; either is correct as
        // long as it is one of them and not a Windows or macOS file.
        let linux = select(ReleaseKind::Desktop, "linux", "x86_64");
        assert!(
            matches!(
                linux.as_deref(),
                Some("kitty-pro_1.0.9_amd64.deb") | Some("kitty-pro_1.0.9_x86_64.AppImage")
            ),
            "unexpected linux asset {linux:?}"
        );
        assert_eq!(
            select(ReleaseKind::Android, "android", "aarch64").as_deref(),
            Some("Kitty-Pro-arm64-v8a.apk")
        );
        assert_eq!(
            select(ReleaseKind::WebServer, "linux", "x86_64").as_deref(),
            Some("Kitty-Pro-web-linux-x86_64.tar.gz")
        );
        assert_eq!(
            select(ReleaseKind::WebServer, "macos", "aarch64").as_deref(),
            Some("Kitty-Pro-web-macos.tar.gz")
        );
    }

    #[test]
    fn a_cpu_or_os_the_release_has_no_build_for_matches_nothing() {
        // Only x86_64 Linux packages exist today, so an arm64 box must be told to
        // wait rather than be handed an amd64 binary.
        assert_eq!(select(ReleaseKind::Desktop, "linux", "aarch64"), None);
        assert_eq!(select(ReleaseKind::Desktop, "linux", "riscv64"), None);
        assert_eq!(select(ReleaseKind::Desktop, "macos", "x86_64"), None);
        assert_eq!(select(ReleaseKind::Android, "android", "x86_64"), None);
        assert_eq!(select(ReleaseKind::WebServer, "windows", "x86_64"), None);
    }

    #[test]
    fn checksums_and_source_bags_are_never_offered() {
        for name in RELEASE_ASSETS {
            if name.ends_with(".sha256") {
                assert_eq!(
                    ReleaseKind::from_asset_name(name),
                    None,
                    "{name} classified as installable"
                );
                for kind in [
                    ReleaseKind::Desktop,
                    ReleaseKind::Android,
                    ReleaseKind::WebServer,
                ] {
                    assert!(
                        !asset_matches_target(name, &target(kind, "linux", "x86_64")),
                        "{name} offered to {kind:?}"
                    );
                }
            }
        }
        assert!(!asset_matches_target(
            "source-archive.tar.gz",
            &target(ReleaseKind::WebServer, "linux", "x86_64")
        ));
    }

    #[test]
    fn a_longer_word_cannot_borrow_a_classification_token() {
        // The matcher reads names, so a token has to be a whole segment of one.
        // These each contain a meaningful token inside a longer word; each must
        // be judged on what it actually is, not on the substring.
        //
        // A `web` substring used to hand a desktop build the server archive, and
        // a `source` substring used to decline a real installer.
        assert_eq!(
            ReleaseKind::from_asset_name("Kitty-Pro-webkit2-x86_64.dmg"),
            Some(ReleaseKind::Desktop),
            "webkit must not classify a dmg as the web server"
        );
        assert!(asset_matches_target(
            "Kitty-Pro-webkit2-x86_64.dmg",
            &target(ReleaseKind::Desktop, "macos", "x86_64")
        ));
        // A `.deb` whose name contains the word `resource` used to be declined
        // outright, because "resource" contains "source".
        assert_eq!(
            ReleaseKind::from_asset_name("kitty-pro-resource-linux-x86_64.deb"),
            Some(ReleaseKind::Desktop),
            "\"resource\" must not be read as the source bag"
        );
        assert!(asset_matches_target(
            "kitty-pro-resource-linux-x86_64.deb",
            &target(ReleaseKind::Desktop, "linux", "x86_64")
        ));
        // The tokens still work when they are their own segment.
        assert_eq!(
            ReleaseKind::from_asset_name("Kitty-Pro-web-linux-x86_64.tar.gz"),
            Some(ReleaseKind::WebServer)
        );
        assert!(!asset_matches_target(
            "kitty-pro-source-linux-x86_64.tar.gz",
            &target(ReleaseKind::WebServer, "linux", "x86_64")
        ));
    }

    #[test]
    fn kinds_do_not_cross_contaminate() {
        // A server archive is never the answer for a desktop install, even though
        // its name says "macos".
        assert!(!asset_matches_target(
            "Kitty-Pro-web-macos.tar.gz",
            &target(ReleaseKind::Desktop, "macos", "aarch64")
        ));
        assert!(asset_matches_target(
            "Kitty-Pro-web-macos.tar.gz",
            &target(ReleaseKind::WebServer, "macos", "aarch64")
        ));
        assert_eq!(
            ReleaseKind::from_asset_name("Kitty-Pro-arm64-v8a.apk"),
            Some(ReleaseKind::Android)
        );
        assert_eq!(ReleaseKind::from_asset_name("README.txt"), None);
        // An extension alone is not a kind: a stray `.zip` names no OS.
        assert_eq!(ReleaseKind::from_asset_name("Kitty-Pro.zip"), None);
    }

    #[test]
    fn unlabelled_assets_stay_eligible() {
        // A future release may publish one build per OS with no CPU in the name,
        // or a universal build.
        assert!(asset_matches_target(
            "Kitty-Pro-macos.dmg",
            &target(ReleaseKind::Desktop, "macos", "aarch64")
        ));
        assert!(asset_matches_target(
            "Kitty-Pro-macos.dmg",
            &target(ReleaseKind::Desktop, "macos", "x86_64")
        ));
        assert!(!asset_matches_target(
            "Kitty-Pro-macos.dmg",
            &target(ReleaseKind::Desktop, "windows", "x86_64")
        ));
        assert!(asset_matches_target(
            "Kitty-Pro-universal.dmg",
            &target(ReleaseKind::Desktop, "macos", "x86_64")
        ));
        // Universal spans the CPUs of one operating system, never the operating
        // systems: a Linux universal build is still not something a Mac installs.
        assert!(!asset_matches_target(
            "Kitty-Pro-universal.AppImage",
            &target(ReleaseKind::Desktop, "macos", "aarch64")
        ));
        assert!(!asset_matches_target(
            "Kitty-Pro-universal.deb",
            &target(ReleaseKind::Desktop, "windows", "x86_64")
        ));
        assert!(asset_matches_target(
            "Kitty-Pro-universal.AppImage",
            &target(ReleaseKind::Desktop, "linux", "aarch64")
        ));
    }

    #[test]
    fn platform_strings_map_to_kinds() {
        assert_eq!(
            ReleaseKind::from_platform("Desktop"),
            Some(ReleaseKind::Desktop)
        );
        assert_eq!(
            ReleaseKind::from_platform("Mobile"),
            Some(ReleaseKind::Android)
        );
        assert_eq!(
            ReleaseKind::from_platform("Web Control"),
            Some(ReleaseKind::WebServer)
        );
        // An iOS shell would need TestFlight, not an APK.
        assert_eq!(ReleaseKind::from_platform("iOS"), None);
    }

    #[test]
    fn versions_order_across_tags_prereleases_and_padding() {
        assert!(is_newer_version("1.0.9", "1.0.10"));
        assert!(is_newer_version("v1.0.9", "v1.2.0"));
        assert!(!is_newer_version("1.0.10", "1.0.9"));
        assert!(!is_newer_version("1.0.9", "1.0.9"));
        // A pre-release build is behind its own final release.
        assert!(is_newer_version("1.1.0-rc.1", "1.1.0"));
        assert!(!is_newer_version("1.1.0", "1.1.0-rc.1"));
        assert!(is_newer_version("1.1.0-rc.1", "1.1.0-rc.2"));
        // Missing components read as zero; build metadata is ignored.
        assert!(is_newer_version("1.0", "1.0.1"));
        assert!(is_newer_version("1.0.0", "v2"));
        // `1` is not a typo: it is `1.0.0`, so it orders equal to a full tag
        // rather than being rejected. Pinning that here keeps it out of the
        // "unparseable" list below, where it only passed by coincidence.
        assert!(!is_newer_version("1.0.0", "1"));
        assert!(is_newer_version("0.9.0", "1"));
        assert!(is_newer_version("1", "1.0.1"));
        assert!(!is_newer_version("1.0.0", "1.0.0+build.7"));
        assert!(!is_newer_version("1.0.0+b", "1.0.0"));
        assert!(is_newer_version("1.0.0+b", "1.0.1"));
        assert!(!is_newer_version("1.1.0-rc.1", "1.1.0-rc.1+build.2"));
        assert!(is_newer_version("1.1.0-rc", "1.1.0-rc.1"));
    }

    #[test]
    fn unparseable_versions_never_announce_an_update() {
        for latest in [
            "", " ", "latest", "1..2", "1.2.3.4", "1.x.3", "-rc.1", "v", "1.2.3-",
        ] {
            assert!(
                !is_newer_version("1.0.0", latest),
                "bad tag {latest:?} reported as newer"
            );
        }
        assert!(!is_newer_version("nightly", "9.9.9"));
    }

    #[test]
    fn release_notes_drop_the_changelog_footer() {
        let release: GitHubRelease = serde_json::from_str(
            r#"{
                "tag_name": "v1.0.9",
                "body": "- fixed something\n\n**Full Changelog**: https://github.com/x/y/compare/v1.0.8...v1.0.9",
                "assets": []
            }"#,
        )
        .expect("release fixture should deserialize");
        assert_eq!(release.notes().as_deref(), Some("- fixed something"));

        let bare: GitHubRelease = serde_json::from_str(r#"{"tag_name": "v1.0.9", "body": "  "}"#)
            .expect("bare release fixture should deserialize");
        assert_eq!(bare.notes(), None);
    }
    #[test]
    fn release_timestamps_parse_the_formats_github_sends() {
        assert_eq!(
            unix_from_rfc3339("2026-11-03T11:10:46Z"),
            Some(1_793_704_246)
        );
        // Fractional seconds and an explicit offset are accepted; the offset is
        // read as UTC, which is all an age label needs.
        assert_eq!(
            unix_from_rfc3339("2026-11-03T11:10:46.500Z"),
            Some(1_793_704_246)
        );
        assert_eq!(
            unix_from_rfc3339("2026-11-03T11:10:46+08:00"),
            Some(1_793_704_246)
        );
        // Century and leap boundaries are where a hand-written calendar breaks.
        assert_eq!(unix_from_rfc3339("2000-02-29T00:00:00Z"), Some(951_782_400));
        assert_eq!(unix_from_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            unix_from_rfc3339("2100-12-31T23:59:59Z"),
            Some(4_133_980_799)
        );
    }

    #[test]
    fn unreadable_timestamps_are_none_rather_than_a_wrong_date() {
        for value in [
            "",
            "yesterday",
            "2026-11-03",
            "2026-11-03 11:10:46",
            "2026-13-01T00:00:00Z",
            "2026-00-10T00:00:00Z",
            "2026-11-32T00:00:00Z",
            "2026-11-03T25:00:00Z",
            "2026-11-03T11:61:00Z",
            "2026-11-03T11:10:46-05:00",
            // Years no real release carries are malformed rather than parsed: an
            // unbounded year overflows the day arithmetic below.
            "99999999999999-01-01T00:00:00Z",
            "0001-01-01T00:00:00Z",
            "1969-12-31T23:59:59Z",
        ] {
            assert_eq!(
                unix_from_rfc3339(value),
                None,
                "{value:?} should be rejected"
            );
        }
    }
}
