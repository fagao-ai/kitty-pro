//! The HTTP surface over `api`: one route per function the interface calls.
//!
//! The deleted Dioxus shell generated these from `#[kitty_server_fn]`; with that
//! macro gone the table below *is* the API surface, and it is the whole of it —
//! add a function to `api`, and the web build cannot reach it until it appears
//! here too. That is the cost of dropping the macro, and the benefit is that this
//! file is the answer to "what can a browser ask this process to do".
//!
//! Wire format, which the client transport in `packages/nsh-app` has to match:
//! a POST sends its arguments as one JSON object keyed by parameter name, and
//! every reply is either the `Ok` value or `{"error": {...}}` with the stable
//! [`ApiErrorCode`] in it. `Ok(())` replies with the JSON literal `null`.

use ::api::{ApiError, ApiErrorCode};
use axum::{
    body::Bytes,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing, Router,
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// Read one named argument out of the request object.
///
/// A missing field and a wrongly-typed one are the same client bug as far as the
/// log is concerned, but they get different messages because "the client sent no
/// `force`" and "the client sent `force` as a string" are different things to fix.
fn read_argument<T: DeserializeOwned>(
    args: &Map<String, Value>,
    name: &str,
) -> Result<T, ApiError> {
    let value = args.get(name).cloned().unwrap_or(Value::Null);
    serde_json::from_value(value)
        .map_err(|error| api_bad_input(format!("参数 {name} 无效: {error}")))
}

/// The request body as an argument object.
///
/// An absent or empty body is an empty object rather than an error: several
/// endpoints take no arguments at all, and a client that sends nothing should not
/// have to invent `{}` to satisfy the parser.
async fn arguments(body: Bytes) -> Result<Map<String, Value>, ApiError> {
    if body.is_empty() {
        return Ok(Map::new());
    }
    serde_json::from_slice(&body)
        .map_err(|error| api_bad_input(format!("请求体必须是 JSON 对象: {error}")))
}

fn api_bad_input(message: String) -> ApiError {
    ApiError {
        code: Some(ApiErrorCode::InvalidInput),
        message,
    }
}

/// Turn an endpoint's result into a reply, keeping the error code readable.
///
/// The status comes from the code's own table, so a caller can branch on either
/// the status or the `[Exxxx]` prefix in the message; the code is also sent as a
/// field because a client that logs a bare 500 learns nothing.
fn reply<T: serde::Serialize>(result: Result<T, ApiError>) -> Response {
    match result {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(body) => (
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
                .into_response(),
            Err(error) => reply::<T>(Err(api_internal(format!("序列化响应失败: {error}")))),
        },
        Err(error) => {
            let code = error.code().unwrap_or(ApiErrorCode::Internal);
            let body = serde_json::json!({
                "error": { "code": code.as_str(), "message": error.message },
            });
            (
                StatusCode::from_u16(code.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response()
        }
    }
}

fn api_internal(message: String) -> ApiError {
    ApiError {
        code: Some(ApiErrorCode::Internal),
        message,
    }
}

/// Declare one endpoint per line and build the router from them.
///
/// `stringify!` on the parameter name is what makes the JSON key match the Rust
/// parameter, so the table cannot drift from `api`'s signatures silently: rename
/// an argument there and the call stops compiling or the field arrives missing.
///
/// Arguments follow a comma rather than parentheses because a `path` fragment may
/// not be followed by `(` in a macro matcher — Rust reserves that to keep `a::b`
/// unambiguous.
macro_rules! endpoints {
    ($( $method:ident $path:literal => $fun:path $(, $( $arg:ident : $ty:ty ),* )? ; )*) => {
        pub fn router() -> Router {
            Router::new()
            $(
                .route(
                    $path,
                    routing::$method({
                        async fn handler(body: Bytes) -> Response {
                            let arguments = match arguments(body).await {
                                Ok(arguments) => arguments,
                                Err(error) => return reply::<()>(Err(error)),
                            };
                            // An endpoint with no arguments still pays for the parse
                            // above, so a malformed body fails it the same way it fails
                            // one that reads fields; then the object is genuinely unused.
                            let _ = &arguments;
                            $(
                                $(
                                    let $arg: $ty =
                                        match read_argument(&arguments, stringify!($arg)) {
                                            Ok(value) => value,
                                            Err(error) => return reply::<()>(Err(error)),
                                        };
                                )*
                            )?
                            reply($fun($($($arg),*)?).await)
                        }
                        handler
                    }),
                )
            )*
        }
    };
}

// The 32 endpoints, in the paths the interface already calls. These are the same
// routes the deleted `#[kitty_server_fn]` attributes registered, so an older
// cached page still reaches an endpoint that exists.
endpoints! {
    get "/api/profile" => api::load_profile;
    post "/api/profile" => api::save_profile, profile: proxy_core::AppProfile;
    post "/api/connectivity/probe" => api::probe_connectivity;

    get "/api/sync/config" => api::load_sync_config;
    post "/api/sync/config" => api::save_sync_config, config: api::SyncConfig;
    post "/api/sync/push" => api::sync_push, config: api::SyncConfig,
        snapshot: proxy_core::SyncSnapshot,
        force: bool;
    post "/api/sync/pull" => api::sync_pull, config: api::SyncConfig;
    post "/api/sync/pull/commit" => api::commit_sync_pull, config: api::SyncConfig,
        current_profile: proxy_core::AppProfile,
        snapshot: proxy_core::SyncSnapshot,
        remote_revision: String;

    post "/api/subscriptions/preview" => api::preview_subscription, source: String;

    post "/api/core/status" => api::core_status;
    post "/api/core/toggle" => api::set_core_enabled, enabled: bool,
        request: Option<proxy_core::ConnectionRequest>;
    post "/api/core/restart" => api::restart_core, request: proxy_core::ConnectionRequest;
    post "/api/core/tun/prepare" => api::prepare_tun_mode, request: proxy_core::ConnectionRequest;
    post "/api/core/tun/release" => api::release_tun_mode;
    post "/api/rules/update" => api::update_rule_sets, force: bool;
    post "/api/config-script/validate" => api::validate_config_script, request: proxy_core::ConnectionRequest;
    post "/api/proxy-groups/preview" => api::preview_proxy_groups, request: proxy_core::ConnectionRequest;
    post "/api/proxy-groups/select" => api::select_proxy_group, group: String, outbound: String;
    get "/api/core/traffic" => api::core_traffic;
    post "/api/core/logs" => api::core_logs, cursor: u64;
    post "/api/core/logs/collection" => api::set_core_log_collection, enabled: bool;
    post "/api/core/latency" => api::measure_node_latency, nodes: Vec<proxy_core::ProxyNode>,
        proxy_server_nameservers: Vec<String>;
    post "/api/core/latency/start" => api::start_node_latency, nodes: Vec<proxy_core::ProxyNode>,
        proxy_server_nameservers: Vec<String>;
    post "/api/core/latency/poll" => api::poll_node_latency, session_id: u64;

    get "/api/system-proxy/status" => api::system_proxy_status;
    post "/api/system-proxy" => api::set_system_proxy, enabled: bool;
    post "/api/environment/copy" => api::copy_proxy_environment_variables;
    get "/api/processes" => api::list_running_processes;

    get "/api/update/settings" => api::update::load_update_settings;
    post "/api/update/settings" => api::update::save_update_settings, settings: api::update::UpdateSettings;
    post "/api/update/check" => api::update::check_app_update, request: api::update::AppUpdateRequest;
    post "/api/update/dismiss" => api::update::dismiss_app_update, version: String;
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn parts_of(response: Response) -> (StatusCode, String) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("a readable body")
            .to_vec();
        (status, String::from_utf8(bytes).expect("a UTF-8 body"))
    }

    #[tokio::test]
    async fn an_ok_value_is_the_whole_body() {
        let probe = api::ConnectivityProbe {
            ok: true,
            detail: "出口可用".into(),
        };
        let (status, body) = parts_of(reply(Ok(probe))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, r#"{"ok":true,"detail":"出口可用"}"#);
    }

    #[tokio::test]
    async fn a_unit_ok_replies_json_null_not_an_empty_body() {
        // `save_profile` and friends return `()`. An empty body is not valid JSON,
        // so a client that parses every reply would fail on every write.
        let (status, body) = parts_of(reply(Ok(()))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "null");
    }

    #[tokio::test]
    async fn an_error_carries_its_code_in_both_the_status_and_the_body() {
        let error = api::api_err(api::ApiErrorCode::NotFound, "没有这个节点");
        let (status, body) = parts_of(reply::<()>(Err(error))).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "status from the code's table"
        );
        assert!(body.contains("\"E1002\""), "{body}");
        assert!(body.contains("没有这个节点"), "{body}");
    }

    #[tokio::test]
    async fn an_unclassified_error_still_gets_a_status_and_a_code() {
        let (status, body) = parts_of(reply::<()>(Err(ApiError::new("内核未响应")))).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body.contains("E1010"), "{body}");
    }

    #[tokio::test]
    async fn a_malformed_body_is_rejected_before_any_handler_runs() {
        let error = arguments(Bytes::from("not json"))
            .await
            .expect_err("must reject");
        assert_eq!(error.code(), Some(ApiErrorCode::InvalidInput));
    }

    #[tokio::test]
    async fn an_empty_body_is_no_arguments_not_a_parse_error() {
        let arguments = arguments(Bytes::from("")).await.expect("empty is fine");
        assert!(arguments.is_empty());
    }

    #[test]
    fn a_missing_argument_names_itself() {
        // "absent" and "wrong type" are different client bugs; both must say which
        // field they mean.
        let error: ApiError = read_argument::<bool>(&Map::new(), "force").unwrap_err();
        assert!(error.message.contains("force"), "{error}");
        assert_eq!(error.code(), Some(ApiErrorCode::InvalidInput));

        let mut wrong = Map::new();
        wrong.insert("force".into(), Value::String("yes".into()));
        let error: ApiError = read_argument::<bool>(&wrong, "force").unwrap_err();
        assert!(error.message.contains("force"), "{error}");
    }

    #[test]
    fn the_table_declares_every_endpoint_the_interface_calls() {
        let declared = declared_endpoints();
        // The deleted macro registered 32; this table replaces it one for one.
        assert_eq!(declared.len(), 32, "{declared:?}");
        for required in [
            "GET /api/profile",
            "POST /api/core/toggle",
            "POST /api/sync/pull/commit",
            "GET /api/update/settings",
            "POST /api/environment/copy",
        ] {
            assert!(declared.contains(&required.to_string()), "{required}");
        }
    }

    /// The table's own lines, read back out of this file.
    ///
    /// `Router` cannot list its routes, so the declaration is what gets checked —
    /// which is the right target: the bug this guards is a line missing from the
    /// table, and that is exactly what this sees.
    fn declared_endpoints() -> Vec<String> {
        let source = include_str!("routes.rs");
        let table = source
            .split_once("endpoints! {")
            .expect("the table")
            .1
            .split_once("\n}")
            .expect("the table's end")
            .0;
        table
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let (method, rest) = line.split_once(char::is_whitespace)?;
                if !matches!(method, "get" | "post") {
                    return None;
                }
                let path = rest.split('"').nth(1)?;
                Some(format!("{} {path}", method.to_uppercase()))
            })
            .collect()
    }
}
