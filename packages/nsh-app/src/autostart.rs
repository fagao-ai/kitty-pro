//! Start-at-login, behind one pair of calls.
//!
//! `api::autostart` exists only where the api crate builds it — a desktop target with the
//! `native-desktop` feature — so the store cannot name it directly: an Android or iOS build
//! would fail to compile over a row it already hides. The two arms below are the whole
//! platform decision, in one file, in the same shape `packages/api` uses for its own
//! per-target functions.
//!
//! Errors come back as `String` rather than the backend's `api::ApiError`: this crate does not
//! want a backend type in its signature, and every caller here only wants the text to put on
//! screen.

/// Whether this build can offer a login item at all, so the row is absent rather than dead.
pub const SUPPORTED: bool = cfg!(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux"
));

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub async fn status() -> Result<bool, String> {
    api::autostart::status()
        .await
        .map_err(|error| error.to_string())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub async fn set_enabled(enabled: bool) -> Result<bool, String> {
    api::autostart::set_enabled(enabled)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub async fn status() -> Result<bool, String> {
    Err("当前平台没有登录启动项".to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub async fn set_enabled(_enabled: bool) -> Result<bool, String> {
    Err("当前平台没有登录启动项".to_owned())
}
