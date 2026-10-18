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

/// 会话密钥持久化到本地数据目录，与 profile.json 同目录（权限 0600）。
/// 服务重启（dx watch 重建、崩溃恢复、daemon 重启）后密钥不变，已登录的
/// 远程设备不会因旧 cookie 失效而全线 403。文件不存在则生成并落盘。
#[cfg(not(target_arch = "wasm32"))]
fn persisted_session_token() -> Result<String, getrandom::Error> {
    let path = session_token_path();
    if let Some(path) = &path {
        if let Ok(stored) = std::fs::read_to_string(path) {
            let stored = stored.trim().to_string();
            if stored.len() == 64 && stored.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(stored);
            }
        }
    }
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random)?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    if let Some(path) = &path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
            {
                let _ = file.write_all(token.as_bytes());
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::write(path, token.as_bytes());
        }
    }
    Ok(token)
}

#[cfg(not(target_arch = "wasm32"))]
fn session_token_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("com", "kitty", "kitty-pro")
        .map(|dirs| dirs.data_local_dir().join("web-session.key"))
}

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

    // Clear a system proxy that a previous Kitty Pro run left pointing at a
    // dead core after it crashed or was killed before cleanup.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    api::cleanup_stale_system_proxy_sentinel();

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
    lan_mode: bool,
    allowed_hosts: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ControlSession {
    fn new() -> Result<Self, getrandom::Error> {
        Ok(Self {
            token: persisted_session_token()?,
            dev_server_authority: dev_server_authority(),
            lan_mode: cli::lan_mode_enabled(),
            allowed_hosts: cli::allowed_host_suffixes(),
        })
    }

    /// LAN 模式下的 Host 判定：loopback 或命中白名单后缀才放行；LAN 模式下
    /// IP 字面量直连（tailnet/LAN 地址）始终放行。仅拒绝未登记的主机名，
    /// 在放开绑定的同时保留防 DNS rebinding 的兜底。
    fn host_allowed(&self, authority: &str) -> bool {
        if parse_loopback_authority(authority).is_some() {
            return true;
        }
        if !self.lan_mode {
            return false;
        }
        let Some(host) = authority_host(authority) else {
            return false;
        };
        // IP 直连不受白名单限制。
        if host.parse::<std::net::IpAddr>().is_ok() {
            return true;
        }
        self.allowed_hosts
            .iter()
            .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
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
    let host_ok = request
        .headers()
        .get(header::HOST)
        .and_then(header_text)
        .is_some_and(|host| session.host_allowed(host));
    if !host_ok {
        return (StatusCode::MISDIRECTED_REQUEST, "host not allowed").into_response();
    }

    let is_api = request.uri().path().starts_with("/api/");
    if is_api && !valid_control_request(&request, &session) {
        log_control_rejection(&request, &session);
        return (StatusCode::FORBIDDEN, "invalid local control session").into_response();
    }

    let issue_cookie = request.method() == Method::GET && !is_api;
    let mut response = next.run(request).await;
    if issue_cookie {
        // 面板 HTML 无版本号且每次都会重发会话 cookie；不带缓存指令时
        // iOS Safari/WebView 会启发式缓存旧 HTML，用户刷新拿不到新构建的
        // hash 资源，界面停留在旧版本。no-store 同时满足会话安全要求。
        if let Ok(value) = HeaderValue::from_str("no-store") {
            response.headers_mut().insert(header::CACHE_CONTROL, value);
        }
        if let Ok(cookie) = HeaderValue::from_str(&session.cookie()) {
            response.headers_mut().append(header::SET_COOKIE, cookie);
        }
    }
    response
}

/// 403 诊断：打印被拒 API 请求的关键头，用于排查远程设备的会话问题。
/// 只输出 Host/Origin/是否带 cookie，不泄露 token 内容。
#[cfg(not(target_arch = "wasm32"))]
fn log_control_rejection(request: &Request<Body>, session: &ControlSession) {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(header_text)
        .unwrap_or("(missing)");
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(header_text)
        .unwrap_or("(none)");
    let cookie_state = match request.headers().get_all(header::COOKIE).iter().next_back() {
        Some(_) if has_session_cookie(request.headers(), &session.token) => "token-valid",
        Some(_) => "present-but-wrong-token",
        None => "missing",
    };
    eprintln!(
        "[kitty-pro] 403 reject path={} method={} host={} origin={} cookie={} lan_mode={} allowed_hosts={:?}",
        request.uri().path(),
        request.method(),
        host,
        origin,
        cookie_state,
        session.lan_mode,
        session.allowed_hosts,
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn valid_control_request(request: &Request<Body>, session: &ControlSession) -> bool {
    let Some(host) = request.headers().get(header::HOST).and_then(header_text) else {
        return false;
    };
    if !session.host_allowed(host) || !has_session_cookie(request.headers(), &session.token) {
        return false;
    }

    request.method() == Method::GET || same_origin(request.headers(), host, session)
}

#[cfg(not(target_arch = "wasm32"))]
fn header_text(value: &HeaderValue) -> Option<&str> {
    value.to_str().ok().map(str::trim)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
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
fn same_origin(headers: &HeaderMap, host: &str, session: &ControlSession) -> bool {
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
    if let Some(dev) = session.dev_server_authority.as_deref() {
        if dev_server_origin_matches(authority, dev) {
            return true;
        }
    }
    // LAN 模式：Origin 主机命中白名单/loopback/IP 直连即视为同源。攻击者的
    // 外域不在名单内会被拒；配合 SameSite=Strict 的会话 Cookie，跨站请求
    // 根本带不上 Cookie，DNS rebinding 与 CSRF 两条路径都被封住。
    if session.lan_mode && session.host_allowed(authority) {
        return true;
    }
    false
}

/// 从 authority（可能含端口、IPv6 方括号）取出主机部分。
/// 与 `parse_loopback_authority` 同样的严格口径：拒绝 userinfo。
#[cfg(not(target_arch = "wasm32"))]
fn authority_host(authority: &str) -> Option<String> {
    if authority.contains('@') {
        return None;
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, _) = rest.split_once(']')?;
        return Some(host.to_ascii_lowercase());
    }
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => host,
        Some((_, _)) => return None,
        None => authority,
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
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
            lan_mode: false,
            allowed_hosts: Vec::new(),
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
            lan_mode: false,
            allowed_hosts: Vec::new(),
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
            lan_mode: false,
            allowed_hosts: Vec::new(),
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
            lan_mode: false,
            allowed_hosts: Vec::new(),
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

    #[test]
    fn lan_mode_host_allowlist() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: true,
            allowed_hosts: vec!["hezz-macmini".to_string(), "et.net".to_string()],
        };
        // loopback 恒放行
        assert!(session.host_allowed("localhost:8080"));
        assert!(session.host_allowed("127.0.0.1:8080"));
        // 白名单主机名与其子域放行
        assert!(session.host_allowed("hezz-macmini:8080"));
        assert!(session.host_allowed("hezz-macmini.et.net:8080"));
        assert!(session.host_allowed("foo.et.net:8080"));
        // IP 字面量直连放行（tailnet / LAN 地址）
        assert!(session.host_allowed("100.64.0.249:8080"));
        assert!(session.host_allowed("192.168.50.3:8080"));
        assert!(session.host_allowed("[::1]:8080"));
        // 未登记的外域主机名拒绝（防 DNS rebinding）
        assert!(!session.host_allowed("evil.test:8080"));
        assert!(!session.host_allowed("nothezz-macmini:8080"));
        // userinfo 伪装的 host 拒绝
        assert!(!session.host_allowed("evil.test@hezz-macmini:8080"));
    }

    #[test]
    fn loopback_mode_rejects_non_loopback_host() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: vec!["et.net".to_string()],
        };
        assert!(session.host_allowed("localhost:8080"));
        // 未开启 LAN 模式时白名单不生效，外域与非 loopback 一律拒绝
        assert!(!session.host_allowed("hezz-macmini:8080"));
        assert!(!session.host_allowed("192.168.50.3:8080"));
    }
}
