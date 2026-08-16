use dioxus::prelude::*;
use ui::ProxyApp;

#[cfg(not(target_arch = "wasm32"))]
use dioxus_server::axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    #[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
    if let Some(exit_code) = singbox::macos::run_helper_from_args() {
        // The authorized helper must exit before Dioxus tries to bind the
        // Web server's listener a second time.
        std::process::exit(exit_code);
    }
    #[cfg(all(
        not(target_arch = "wasm32"),
        any(target_os = "windows", target_os = "linux")
    ))]
    if let Some(exit_code) = singbox::desktop_helper::run_helper_from_args() {
        // The elevated helper must not initialize a second fullstack server.
        std::process::exit(exit_code);
    }

    let session = ControlSession::new().expect("control session token should be generated");
    dioxus_server::serve(move || {
        let session = session.clone();
        async move {
            Ok(
                dioxus_server::router(App).layer(middleware::from_fn_with_state(
                    session,
                    control_session_guard,
                )),
            )
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn main() {
    dioxus::launch(App);
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct ControlSession {
    token: String,
    dev_server_authority: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ControlSession {
    fn new() -> Result<Self, getrandom::Error> {
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random)?;
        Ok(Self {
            token: random.iter().map(|byte| format!("{byte:02x}")).collect(),
            dev_server_authority: dev_server_authority(),
        })
    }

    fn cookie(&self) -> String {
        format!(
            "kitty_pro_control={}; Path=/; HttpOnly; SameSite=Strict",
            self.token
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn control_session_guard(
    State(session): State<ControlSession>,
    request: Request,
    next: Next,
) -> Response {
    let loopback_host = request
        .headers()
        .get(header::HOST)
        .and_then(header_text)
        .is_some_and(is_loopback_authority);
    if !loopback_host {
        return (StatusCode::MISDIRECTED_REQUEST, "loopback host required").into_response();
    }

    let is_api = request.uri().path().starts_with("/api/");
    if is_api && !valid_control_request(&request, &session) {
        return (StatusCode::FORBIDDEN, "invalid local control session").into_response();
    }

    let issue_cookie = request.method() == Method::GET && !is_api;
    let mut response = next.run(request).await;
    if issue_cookie {
        if let Ok(cookie) = HeaderValue::from_str(&session.cookie()) {
            response.headers_mut().append(header::SET_COOKIE, cookie);
        }
    }
    response
}

#[cfg(not(target_arch = "wasm32"))]
fn valid_control_request(request: &Request<Body>, session: &ControlSession) -> bool {
    let Some(host) = request.headers().get(header::HOST).and_then(header_text) else {
        return false;
    };
    if !is_loopback_authority(host) || !has_session_cookie(request.headers(), &session.token) {
        return false;
    }

    request.method() == Method::GET
        || same_origin(
            request.headers(),
            host,
            session.dev_server_authority.as_deref(),
        )
}

#[cfg(not(target_arch = "wasm32"))]
fn header_text(value: &HeaderValue) -> Option<&str> {
    value.to_str().ok().map(str::trim)
}

#[cfg(not(target_arch = "wasm32"))]
fn is_loopback_authority(authority: &str) -> bool {
    authority
        .parse::<dioxus_server::axum::http::uri::Authority>()
        .map(|authority| {
            matches!(
                authority.host(),
                "127.0.0.1" | "localhost" | "[::1]" | "::1"
            )
        })
        .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn has_session_cookie(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(header_text)
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .any(|(name, value)| name == "kitty_pro_control" && constant_time_eq(value, expected))
}

#[cfg(not(target_arch = "wasm32"))]
fn same_origin(headers: &HeaderMap, host: &str, dev_server_authority: Option<&str>) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(header_text) else {
        return false;
    };
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return false;
    }
    let Some(authority) = uri.authority() else {
        return false;
    };
    let authority = authority.as_str();
    if authority == host {
        return true;
    }
    // The dev server proxies requests to an internal loopback port and
    // rewrites the `Host` header, so the browser's Origin no longer matches
    // the request Host. Accept the same dev server reached through any
    // loopback alias (http://localhost:8080 vs http://127.0.0.1:8080) while
    // still rejecting cross-site origins from other local ports.
    match dev_server_authority {
        Some(dev) => dev_server_origin_matches(authority, dev),
        None => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn dev_server_origin_matches(origin_authority: &str, dev_server_authority: &str) -> bool {
    if origin_authority == dev_server_authority {
        return true;
    }
    let Some((dev_host, dev_port)) = dev_server_authority.rsplit_once(':') else {
        return false;
    };
    let Some((origin_host, origin_port)) = origin_authority.rsplit_once(':') else {
        return false;
    };
    dev_port == origin_port
        && matches!(dev_host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
        && matches!(origin_host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

#[cfg(not(target_arch = "wasm32"))]
fn dev_server_authority() -> Option<String> {
    let ip = std::env::var("DIOXUS_DEVSERVER_IP").ok()?;
    let port = std::env::var("DIOXUS_DEVSERVER_PORT").ok()?;
    let authority = format!("{ip}:{port}");
    is_loopback_authority(&authority).then_some(authority)
}

#[cfg(not(target_arch = "wasm32"))]
fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[component]
fn App() -> Element {
    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        ProxyApp { platform: "Web Control".to_string(), desktop_tray: None }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn request(method: Method, host: &str, origin: Option<&str>, token: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/core/tun/prepare")
            .header(header::HOST, host)
            .header(header::COOKIE, format!("kitty_pro_control={token}"));
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder.body(Body::empty()).expect("request should build")
    }

    #[test]
    fn post_requires_loopback_same_origin_and_session_cookie() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
        };
        assert!(valid_control_request(
            &request(
                Method::POST,
                "127.0.0.1:8080",
                Some("http://127.0.0.1:8080"),
                "secret"
            ),
            &session
        ));
        assert!(!valid_control_request(
            &request(
                Method::POST,
                "127.0.0.1:8080",
                Some("http://localhost:9999"),
                "secret"
            ),
            &session
        ));
        assert!(!valid_control_request(
            &request(
                Method::POST,
                "127.0.0.1:8080",
                Some("http://127.0.0.1:8080"),
                "wrong"
            ),
            &session
        ));
        assert!(!valid_control_request(
            &request(
                Method::POST,
                "example.com",
                Some("http://example.com"),
                "secret"
            ),
            &session
        ));
    }

    #[test]
    fn post_without_origin_is_rejected() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
        };
        assert!(!valid_control_request(
            &request(Method::POST, "localhost:8080", None, "secret"),
            &session
        ));
    }

    #[test]
    fn configured_loopback_dev_server_origin_is_allowed() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: Some("127.0.0.1:8080".to_string()),
        };
        assert!(valid_control_request(
            &request(
                Method::POST,
                "127.0.0.1:50622",
                Some("http://127.0.0.1:8080"),
                "secret"
            ),
            &session
        ));
    }

    #[test]
    fn proxied_dev_server_requests_accept_loopback_origin_aliases() {
        // dx serve rewrites the Host header to the internal loopback port, so
        // the browser Origin must be matched against the configured dev server
        // authority instead of the request Host.
        assert!(dev_server_origin_matches("localhost:8080", "127.0.0.1:8080"));
        assert!(dev_server_origin_matches("127.0.0.1:8080", "127.0.0.1:8080"));
        assert!(dev_server_origin_matches("[::1]:8080", "127.0.0.1:8080"));
        assert!(dev_server_origin_matches("127.0.0.1:8080", "localhost:8080"));
        assert!(dev_server_origin_matches("localhost:8080", "localhost:8080"));
        // A different local port is a different site and must stay rejected:
        // SameSite cookies still travel between same-site localhost origins.
        assert!(!dev_server_origin_matches("localhost:9090", "127.0.0.1:8080"));
        assert!(!dev_server_origin_matches("example.com:8080", "127.0.0.1:8080"));
        assert!(!dev_server_origin_matches("127.0.0.1", "127.0.0.1:8080"));
        assert!(!dev_server_origin_matches("localhost:8080", ""));
    }
}
