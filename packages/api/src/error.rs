//! Unified, machine-readable error codes for client-facing server functions.
//!
//! `ServerFnError` (Dioxus 0.7, re-exported from `dioxus_fullstack_core`) is an
//! enum. We confirmed its real API: it has a
//! `ServerError { message, code, details }` variant carrying an HTTP `code`,
//! plus `ServerFnError::new(impl ToString)` (status 500, `details: None`) as the
//! only constructor that is stable across every target, including wasm32.
//!
//! Tradeoff: we deliberately do NOT construct the `ServerError { .. }` variant
//! by hand. Relying on that internal shape would couple us to one Dioxus
//! release and would not help the UI, which surfaces the **message string** in
//! toasts (B6/B7). Instead `api_err` embeds the `ApiErrorCode` into the message
//! as a `[Exxxx]` prefix, and `error_code_from_server_fn` decodes it from
//! `ServerFnError::to_string()`. Because that Display output wraps the message
//! ("error running server function: … (details: None)"), the marker is not at
//! byte 0; the decoder scans bracketed tokens instead of assuming a start
//! position. This keeps one code path on native and wasm32 alike.

use super::*;

/// Stable, UI-parseable error codes. The string form (`E1001`…) is part of the
/// wire contract between the api crate and the shells, so never reorder or
/// reuse a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ApiErrorCode {
    InvalidInput,
    NotFound,
    Conflict,
    Network,
    Storage,
    Privilege,
    Core,
    Sync,
    Update,
    Internal,
}

impl ApiErrorCode {
    /// The stable on-wire code, e.g. `"E1001"`.
    pub fn as_str(self) -> &'static str {
        match self {
            ApiErrorCode::InvalidInput => "E1001",
            ApiErrorCode::NotFound => "E1002",
            ApiErrorCode::Conflict => "E1003",
            ApiErrorCode::Network => "E1004",
            ApiErrorCode::Storage => "E1005",
            ApiErrorCode::Privilege => "E1006",
            ApiErrorCode::Core => "E1007",
            ApiErrorCode::Sync => "E1008",
            ApiErrorCode::Update => "E1009",
            ApiErrorCode::Internal => "E1010",
        }
    }

    /// A sensible HTTP status per category. `ServerFnError::new` always reports
    /// 500 at the transport layer; this table is the catalog the UI (and any
    /// future HTTP-layer integration) uses to classify a code.
    pub fn http_status(self) -> u16 {
        match self {
            ApiErrorCode::InvalidInput => 400,
            ApiErrorCode::NotFound => 404,
            ApiErrorCode::Conflict | ApiErrorCode::Sync => 409,
            ApiErrorCode::Network | ApiErrorCode::Update => 502,
            ApiErrorCode::Privilege => 403,
            ApiErrorCode::Storage | ApiErrorCode::Core | ApiErrorCode::Internal => 500,
        }
    }

    /// Reverse of [`ApiErrorCode::as_str`]. Accepts the bare code and, for
    /// convenience, a bracketed `[E1001]` token.
    pub fn from_str_code(code: &str) -> Option<Self> {
        let code = code.trim().trim_start_matches('[').trim_end_matches(']');
        match code {
            "E1001" => Some(ApiErrorCode::InvalidInput),
            "E1002" => Some(ApiErrorCode::NotFound),
            "E1003" => Some(ApiErrorCode::Conflict),
            "E1004" => Some(ApiErrorCode::Network),
            "E1005" => Some(ApiErrorCode::Storage),
            "E1006" => Some(ApiErrorCode::Privilege),
            "E1007" => Some(ApiErrorCode::Core),
            "E1008" => Some(ApiErrorCode::Sync),
            "E1009" => Some(ApiErrorCode::Update),
            "E1010" => Some(ApiErrorCode::Internal),
            _ => None,
        }
    }
}

/// A structured error before it is flattened into a `ServerFnError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
}

/// Build a `ServerFnError` whose message carries a `[Exxxx]` code prefix.
///
/// Every message is passed through `redact_secrets` here, so subscription URLs
/// (userinfo / `?token=`) echoed by `reqwest` can never reach a toast. Call
/// sites therefore pass the raw Chinese message and do not redact themselves.
pub fn api_err(code: ApiErrorCode, message: impl Into<String>) -> ServerFnError {
    let raw: String = message.into();
    let redacted = redact_secrets(&raw);
    ServerFnError::new(format!("[{}] {}", code.as_str(), redacted))
}

/// Recover the code that [`api_err`] embedded in a `ServerFnError`.
///
/// `ServerFnError::to_string()` wraps the message, so the `[Exxxx]` marker is
/// not necessarily at the start; we scan bracketed tokens and resolve the
/// first one that is a known code.
pub fn error_code_from_server_fn(error: &ServerFnError) -> Option<ApiErrorCode> {
    let rendered = error.to_string();
    let mut tail = rendered.as_str();
    while let Some(open) = tail.find('[') {
        let after = &tail[open + 1..];
        let Some(close) = after.find(']') else {
            break;
        };
        if let Some(code) = ApiErrorCode::from_str_code(&after[..close]) {
            return Some(code);
        }
        tail = &after[close + 1..];
    }
    None
}

impl From<ApiError> for ServerFnError {
    fn from(value: ApiError) -> Self {
        api_err(value.code, value.message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)
    }
}
