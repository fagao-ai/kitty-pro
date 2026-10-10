use dioxus::prelude::*;
use ui::ProxyApp;

#[cfg(not(target_arch = "wasm32"))]
use dioxus_server::axum::{
    body::Body,
    extract::{Request, State},
    http::{header, uri::Authority, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};

#[cfg(not(target_arch = "wasm32"))]
mod cli;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");

#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_WEB_PORT: &str = "7897";

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

    match cli::prepare() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("错误: {error}");
            std::process::exit(2);
        }
    }

    let session = ControlSession::new().expect("control session token should be generated");
    dioxus_server::serve(move || {
        let session = session.clone();
        async move {
            let mut router = dioxus_server::router(App);
            if let Some((path, token)) = cli::readiness_route() {
                router = router.route(&path, get(move || async move { token }));
            }
            Ok(router.layer(middleware::from_fn_with_state(
                session,
                control_session_guard,
            )))
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
    parse_loopback_authority(authority).is_some()
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_loopback_authority(value: &str) -> Option<Authority> {
    let authority = value.parse::<Authority>().ok()?;
    // Authority 的语法解析允许 userinfo 和非法端口，不能仅靠 port() 判断。
    let host = authority.host();
    if value.contains('@') {
        return None;
    }
    let suffix = value.strip_prefix(host)?;
    if !suffix.is_empty() {
        let port = suffix.strip_prefix(':')?;
        if port.is_empty()
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || !authority.port_u16().is_some_and(|port| port != 0)
        {
            return None;
        }
    }
    let loopback = host == "localhost"
        || host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(host)
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    loopback.then_some(authority)
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
    let Some(origin) = parse_loopback_authority(origin_authority) else {
        return false;
    };
    let Some(dev) = parse_loopback_authority(dev_server_authority) else {
        return false;
    };
    match (origin.port_u16(), dev.port_u16()) {
        (Some(origin_port), Some(dev_port)) => origin_port == dev_port,
        _ => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn dev_server_authority() -> Option<String> {
    let ip = std::env::var("DIOXUS_DEVSERVER_IP").ok()?;
    let port = std::env::var("DIOXUS_DEVSERVER_PORT").ok()?;
    dev_server_authority_from_parts(&ip, &port)
}

#[cfg(not(target_arch = "wasm32"))]
fn dev_server_authority_from_parts(ip: &str, port: &str) -> Option<String> {
    let host = match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(ip)) => format!("[{ip}]"),
        _ => ip.to_string(),
    };
    let value = format!("{host}:{port}");
    let authority = parse_loopback_authority(&value)?;
    authority.port_u16()?;
    Some(value)
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
    fn loopback_ip_environment_matches_host_guard() {
        assert!(is_loopback_authority("127.0.0.2:7897"));
        assert!(is_loopback_authority("[::1]:7897"));
        assert!(!is_loopback_authority("192.168.1.1:7897"));
        assert!(!is_loopback_authority("0.0.0.0:7897"));
        assert!(!is_loopback_authority("127.0.0.1:65536"));
        assert!(!is_loopback_authority("127.0.0.1:0"));
        assert!(!is_loopback_authority("localhost:"));
        assert!(!is_loopback_authority("localhost:invalid"));
        assert!(!is_loopback_authority("user@127.0.0.1:7897"));
    }

    #[test]
    fn dev_server_authority_formats_ipv6_without_mutating_environment() {
        assert_eq!(
            dev_server_authority_from_parts("::1", "7897").as_deref(),
            Some("[::1]:7897")
        );
        assert_eq!(
            dev_server_authority_from_parts("127.0.0.2", "7897").as_deref(),
            Some("127.0.0.2:7897")
        );
        assert!(dev_server_authority_from_parts("::1", "65536").is_none());
        assert!(dev_server_authority_from_parts("192.168.1.1", "7897").is_none());
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
    fn proxied_requests_preserve_other_security_checks() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: Some("[::1]:8080".to_string()),
        };
        assert!(valid_control_request(
            &request(
                Method::POST,
                "127.0.0.1:50622",
                Some("http://127.0.0.2:8080"),
                "secret"
            ),
            &session
        ));
        for (host, origin, token) in [
            ("127.0.0.1:50622", Some("http://localhost:9090"), "secret"),
            ("127.0.0.1:50622", Some("http://evil.test:8080"), "secret"),
            ("127.0.0.1:50622", Some("ftp://localhost:8080"), "secret"),
            ("127.0.0.1:50622", Some("http://localhost:8080"), "wrong"),
            ("evil.test:50622", Some("http://localhost:8080"), "secret"),
            ("127.0.0.1:50622", None, "secret"),
        ] {
            assert!(!valid_control_request(
                &request(Method::POST, host, origin, token),
                &session
            ));
        }
    }

    #[test]
    fn proxied_dev_server_requests_accept_loopback_origin_aliases() {
        // dx serve rewrites the Host header to the internal loopback port, so
        // the browser Origin must be matched against the configured dev server
        // authority instead of the request Host.
        assert!(dev_server_origin_matches(
            "localhost:8080",
            "127.0.0.1:8080"
        ));
        assert!(dev_server_origin_matches(
            "127.0.0.1:8080",
            "127.0.0.1:8080"
        ));
        assert!(dev_server_origin_matches("[::1]:8080", "127.0.0.1:8080"));
        assert!(dev_server_origin_matches(
            "127.0.0.1:8080",
            "localhost:8080"
        ));
        assert!(dev_server_origin_matches(
            "localhost:8080",
            "localhost:8080"
        ));
        // A different local port is a different site and must stay rejected:
        // SameSite cookies still travel between same-site localhost origins.
        assert!(!dev_server_origin_matches(
            "localhost:9090",
            "127.0.0.1:8080"
        ));
        assert!(!dev_server_origin_matches(
            "example.com:8080",
            "127.0.0.1:8080"
        ));
        assert!(!dev_server_origin_matches("127.0.0.1", "127.0.0.1:8080"));
        assert!(!dev_server_origin_matches("localhost:8080", ""));
        for alias in ["127.0.0.2:8080", "[::1]:8080", "[0:0:0:0:0:0:0:1]:8080"] {
            assert!(dev_server_origin_matches(alias, "localhost:8080"));
            assert!(dev_server_origin_matches("localhost:8080", alias));
        }
        for invalid in [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "::1:8080",
            "localhost:",
            "localhost:invalid",
            "localhost:65536",
            "localhost:0",
            "192.168.1.1:8080",
            "0.0.0.0:8080",
            "[::]:8080",
            "example.com:8080",
            "localhost.evil.test:8080",
            "localhost@evil.test:8080",
            "evil.test@localhost:8080",
        ] {
            assert!(
                !dev_server_origin_matches(invalid, "localhost:8080"),
                "{invalid}"
            );
            assert!(
                !dev_server_origin_matches("localhost:8080", invalid),
                "{invalid}"
            );
            assert!(!dev_server_origin_matches(invalid, invalid), "{invalid}");
        }
    }
}
