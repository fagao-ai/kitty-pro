//! The backend, called here or over there.
//!
//! Every screen reaches the proxy through this module rather than through `api`
//! directly. Native builds get the real functions — this is a native process, so a
//! call is a call, exactly as before this module existed. A browser build gets HTTP
//! against `packages/server`, because a page cannot load a Go archive: under `wasm32`
//! every `api` function is a stub that refuses, which is precisely what the server
//! exists to do instead.
//!
//! The signatures are the same on both sides down to the error type. `api` compiles
//! for the browser, so `api::ApiError` is available there, and a screen that matches
//! on an error code keeps working when the same screen is served to a phone and to a
//! page. That equivalence is the whole design: nothing above this module knows which
//! side it is on.
//!
//! One table generates both arms, so an endpoint cannot be wired to one side only.
//! The parameter names are what make the wire format: `stringify!` of them produces
//! the JSON keys, which is the same rule `packages/server/src/routes.rs` applies, so
//! the two ends agree by construction rather than by whoever last remembered to check.

use api::ApiError;

/// The endpoints, mirroring `packages/server/src/routes.rs` one line for one line.
macro_rules! backend {
    ($( $method:literal $path:literal => $name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $out:ty = $call:path ; )*) => {
        $(
            /// Documented once, in the table; the body is the platform decision.
            pub async fn $name( $( $arg: $ty ),* ) -> Result<$out, ApiError> {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // The arguments are named so the pass-through does not warn about
                    // the ones an endpoint happens to ignore, and so this arm reads as
                    // the call it is.
                    $( let _ = &$arg; )*
                    $call( $( $arg ),* ).await
                }
                #[cfg(target_arch = "wasm32")]
                {
                    // Assembled by hand rather than with `json!`, because a key has to
                    // come from `stringify!` and `json!` will not match that as a key
                    // token. `from_iter` rather than inserts so the many endpoints with
                    // no arguments do not leave a `mut` nothing wrote to.
                    let arguments = ::serde_json::Map::from_iter([
                        $(
                            (
                                stringify!($arg).to_owned(),
                                ::serde_json::to_value(&$arg).map_err(|error| {
                                    // English, and out of the translation table, on
                                    // purpose: this fires only when one of *our* types
                                    // refuses to serialize, which is a bug in this repo
                                    // rather than something a user can act on. The
                                    // crate's `expect` messages are English for the same
                                    // reason, and the i18n sweep skips non-Chinese
                                    // literals.
                                    ApiError::new(format!(
                                        "argument {} failed to serialize: {error}",
                                        stringify!($arg)
                                    ))
                                })?,
                            ),
                        )*
                    ]);
                    $crate::backend::request::<$out>(
                        $method,
                        $path,
                        ::serde_json::Value::Object(arguments),
                    )
                    .await
                }
            }
        )*
    };
}

/// Ask the server, and turn its reply back into the native shape.
///
/// The session cookie is the browser's to send: same-origin requests carry it, which
/// is why the server insists the page comes from its own listener.
#[cfg(target_arch = "wasm32")]
async fn request<T: serde::de::DeserializeOwned>(
    method: &str,
    path: &str,
    arguments: serde_json::Value,
) -> Result<T, ApiError> {
    use nsh_core::platform::{HttpBody, HttpRequest};

    let mut request = HttpRequest::new(method, path);
    // A GET with a body is what a browser refuses to send, and every GET in the
    // table takes no arguments, so this stays `Empty` there rather than inventing a
    // body the server would have to ignore.
    if method != "GET" {
        request
            .headers
            .push(("content-type".into(), "application/json".into()));
        request.body = HttpBody::Bytes(arguments.to_string().into_bytes());
    }

    let response = nsh_core::prelude::fetch(request)
        .await
        .map_err(ApiError::new)?;

    let text = String::from_utf8_lossy(&response.body).into_owned();
    if let Ok(value) = serde_json::from_str::<T>(&text) {
        return Ok(value);
    }
    // Anything that is not the expected value is the server's refusal: it answers
    // `{"error":{…}}` with a status taken from the code's own table.
    Err(serde_json::from_str::<WireError>(&text)
        .map(WireError::into_api_error)
        .unwrap_or_else(|_| {
            ApiError::new(crate::i18n::fill_at(
                crate::i18n::lang_untracked(),
                "服务端返回了无法理解的内容（HTTP {}）",
                &[&response.status()],
            ))
        }))
}

/// The server's error reply.
#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
struct WireError {
    error: WireErrorBody,
}

#[cfg(target_arch = "wasm32")]
#[derive(serde::Deserialize)]
struct WireErrorBody {
    code: String,
    message: String,
}

#[cfg(target_arch = "wasm32")]
impl WireError {
    /// Restore the code so the screens' existing `match` on it still works. A code
    /// the client does not know is not a reason to lose the message with it.
    fn into_api_error(self) -> ApiError {
        ApiError {
            code: api::ApiErrorCode::from_str_code(&self.error.code),
            message: self.error.message,
        }
    }
}

backend! {
    "GET"  "/api/profile" => load_profile() -> proxy_core::AppProfile = api::load_profile;
    "POST" "/api/profile" => save_profile(profile: proxy_core::AppProfile) -> () = api::save_profile;
    "POST" "/api/connectivity/probe" => probe_connectivity() -> api::ConnectivityProbe = api::probe_connectivity;

    "GET"  "/api/sync/config" => load_sync_config() -> api::SyncConfig = api::load_sync_config;
    "POST" "/api/sync/config" => save_sync_config(config: api::SyncConfig) -> api::SyncConfig = api::save_sync_config;
    "POST" "/api/sync/push" => sync_push(config: api::SyncConfig, snapshot: proxy_core::SyncSnapshot, force: bool) -> api::SyncResult = api::sync_push;
    "POST" "/api/sync/pull" => sync_pull(config: api::SyncConfig) -> api::SyncPullResult = api::sync_pull;
    "POST" "/api/sync/pull/commit" => commit_sync_pull(config: api::SyncConfig, current_profile: proxy_core::AppProfile, snapshot: proxy_core::SyncSnapshot, remote_revision: String) -> api::SyncResult = api::commit_sync_pull;

    "POST" "/api/subscriptions/preview" => preview_subscription(source: String) -> proxy_core::ParseReport = api::preview_subscription;

    "POST" "/api/core/status" => core_status() -> api::ApiCoreStatus = api::core_status;
    "POST" "/api/core/toggle" => set_core_enabled(enabled: bool, request: Option<proxy_core::ConnectionRequest>) -> api::ApiCoreStatus = api::set_core_enabled;
    "POST" "/api/core/restart" => restart_core(request: proxy_core::ConnectionRequest) -> api::ApiCoreStatus = api::restart_core;
    "POST" "/api/core/tun/prepare" => prepare_tun_mode(request: proxy_core::ConnectionRequest) -> () = api::prepare_tun_mode;
    "POST" "/api/core/tun/release" => release_tun_mode() -> () = api::release_tun_mode;
    "POST" "/api/rules/update" => update_rule_sets(force: bool) -> api::RuleSetUpdateResult = api::update_rule_sets;
    "POST" "/api/config-script/validate" => validate_config_script(request: proxy_core::ConnectionRequest) -> () = api::validate_config_script;
    "POST" "/api/proxy-groups/preview" => preview_proxy_groups(request: proxy_core::ConnectionRequest) -> Vec<proxy_core::ProxyGroup> = api::preview_proxy_groups;
    "POST" "/api/proxy-groups/select" => select_proxy_group(group: String, outbound: String) -> () = api::select_proxy_group;
    "GET"  "/api/core/traffic" => core_traffic() -> api::CoreTraffic = api::core_traffic;
    "POST" "/api/core/logs" => core_logs(cursor: u64) -> api::CoreLogBatch = api::core_logs;
    "POST" "/api/core/logs/collection" => set_core_log_collection(enabled: bool) -> () = api::set_core_log_collection;
    "POST" "/api/core/latency" => measure_node_latency(nodes: Vec<proxy_core::ProxyNode>, proxy_server_nameservers: Vec<String>) -> Vec<api::NodeLatency> = api::measure_node_latency;
    "POST" "/api/core/latency/start" => start_node_latency(nodes: Vec<proxy_core::ProxyNode>, proxy_server_nameservers: Vec<String>) -> u64 = api::start_node_latency;
    "POST" "/api/core/latency/poll" => poll_node_latency(session_id: u64) -> api::LatencyProbeSnapshot = api::poll_node_latency;

    "GET"  "/api/system-proxy/status" => system_proxy_status() -> api::SystemProxyStatus = api::system_proxy_status;
    "POST" "/api/system-proxy" => set_system_proxy(enabled: bool) -> api::SystemProxyStatus = api::set_system_proxy;
    "POST" "/api/environment/copy" => copy_proxy_environment_variables() -> () = api::copy_proxy_environment_variables;
    "GET"  "/api/processes" => list_running_processes() -> Vec<api::RunningProcess> = api::list_running_processes;

    "GET"  "/api/update/settings" => load_update_settings() -> api::update::UpdateSettings = api::update::load_update_settings;
    "POST" "/api/update/settings" => save_update_settings(settings: api::update::UpdateSettings) -> api::update::UpdateSettings = api::update::save_update_settings;
    "POST" "/api/update/check" => check_app_update(request: api::update::AppUpdateRequest) -> api::update::AppUpdateStatus = api::update::check_app_update;
    "POST" "/api/update/dismiss" => dismiss_app_update(version: String) -> api::update::UpdateSettings = api::update::dismiss_app_update;
}
