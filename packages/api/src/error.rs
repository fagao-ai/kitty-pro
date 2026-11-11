//! Unified, machine-readable error codes for the api crate's public functions.
//!
//! [`ApiError`] is the crate's own type: an api call is a plain async function now
//! that the NSH app is the only shell, so nothing wraps these messages in a
//! transport layer any more. That is a behaviour change worth knowing about — the
//! Dioxus shells saw `Display` output that read
//! "error running server function: … (details: None)", and a shell that prints
//! `error.to_string()` gets the bare message today.
//!
//! The category lives in a field, not in the string: [`api_err`] records the
//! `ApiErrorCode` structurally, while `Display` still renders the `[Exxxx]` prefix
//! that the codes have always carried, so anything matching on the rendered text
//! keeps working. [`error_code`] reports the code and additionally decodes a
//! `[Exxxx]` marker embedded in an unclassified message, which is what keeps the
//! contract intact for errors that were built by [`ApiError::new`] from an
//! already-formatted string.

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

    /// A sensible HTTP status per category. `ApiError::new` always reports
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

/// A failed api call: a user-facing message plus, where the site knew one, the
/// [`ApiErrorCode`] it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    /// `None` for the many sites that only know how to phrase a message; see
    /// [`ApiError::new`].
    pub code: Option<ApiErrorCode>,
    pub message: String,
}

impl ApiError {
    /// An unclassified failure. Prefer [`api_err`] where the category is known.
    ///
    /// The message goes through `redact_secrets`, because the majority of these
    /// carry text echoed back by `reqwest` or the OS — subscription URLs with
    /// userinfo or `?token=` must never reach a toast.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: redact_secrets(&message.into()),
        }
    }

    /// The category, and for an unclassified error the `[Exxxx]` marker embedded
    /// in its message. The fallback keeps the code readable in the case that
    /// matters: an error that was assembled from an already-formatted string.
    pub fn code(&self) -> Option<ApiErrorCode> {
        if let Some(code) = self.code {
            return Some(code);
        }
        let mut tail = self.message.as_str();
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
}

/// Build an [`ApiError`] with a known category.
pub fn api_err(code: ApiErrorCode, message: impl Into<String>) -> ApiError {
    ApiError {
        code: Some(code),
        ..ApiError::new(message)
    }
}

/// The category of an [`ApiError`], if it has one.
pub fn error_code(error: &ApiError) -> Option<ApiErrorCode> {
    error.code()
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.code {
            Some(code) => write!(f, "[{}] {}", code.as_str(), self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ApiError {}
