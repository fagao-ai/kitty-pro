//! The web control surface's request guard, ported verbatim from the deleted
//! `packages/web/src/main.rs` along with its 19 tests.
//!
//! This is a security boundary, so it was extracted mechanically rather than
//! rewritten: the semantics below (exact-host allow-listing, strict same-origin,
//! constant-time token compare) each close a hole an earlier revision had, and
//! those are recorded in the comments here rather than in git archaeology.
//!
//! Two things carried over that no longer apply and were left alone rather than
//! "cleaned up": the `target_arch = "wasm32"` gates, since this crate is native
//! only, and `dev_server_authority`, which existed because `dx serve` proxied to
//! an internal port and rewrote `Host`. There is no dev server any more, so it
//! reads `None` and the branch under it is unreachable.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, uri::Authority, HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
};

#[cfg(not(target_arch = "wasm32"))]
/// 每次进程启动都生成新的 32 字节随机会话 token，并覆盖本地数据目录下的
/// web-session.key（权限 0600）。不再读取旧值：进程重启后旧 cookie 立即失效，
/// 等价于按进程轮换 token，避免永久共享 token 在 LAN 明文链路中长期有效。
/// 文件路径逻辑与权限设置保持不变。
#[cfg(not(target_arch = "wasm32"))]
fn persisted_session_token() -> Result<String, getrandom::Error> {
    // 不再读取旧值：每次启动都生成新 token，进程重启即轮换。
    let path = session_token_path();
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
    // E2E isolation, the same convention `api`'s `profile_path` and
    // `system_proxy_sentinel_path` follow (b40bbc0): a test or CI server started with
    // KITTY_PROFILE_DIR must not write into the developer's own data directory. This
    // file is only a record — the token is generated fresh every start and never read
    // back — so relocating it costs nothing and stops `cargo test` from rotating the
    // real app's session out from under it.
    if let Ok(dir) = std::env::var("KITTY_PROFILE_DIR") {
        return Some(session_token_path_in(std::path::Path::new(&dir)));
    }
    directories::ProjectDirs::from("com", "kitty", "kitty-pro")
        .map(|dirs| session_token_path_in(dirs.data_local_dir()))
}

fn session_token_path_in(directory: &std::path::Path) -> std::path::PathBuf {
    directory.join("web-session.key")
}

#[derive(Clone)]
pub(crate) struct ControlSession {
    token: String,
    dev_server_authority: Option<String>,
    lan_mode: bool,
    allowed_hosts: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ControlSession {
    pub(crate) fn new() -> Result<Self, getrandom::Error> {
        Ok(Self {
            token: persisted_session_token()?,
            dev_server_authority: dev_server_authority(),
            // LAN 宽松策略只在「实际绑定非 loopback 地址」时开启：cli.rs 的
            // lan_mode_enabled() 已把 KITTY_PRO_ALLOW_LAN 开关与 prepare()
            // 规范化后的绑定 IP 绑定，避免只绑本机却启用宽松 Host/Origin 策略。
            lan_mode: crate::cli::lan_mode_enabled(),
            allowed_hosts: crate::cli::allowed_host_suffixes(),
        })
    }

    /// A session with a known token and no environment reading, for tests that only
    /// need to know how the guard reacts to a request.
    #[cfg(test)]
    pub(crate) fn for_test(token: &str) -> Self {
        Self {
            token: token.to_string(),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: Vec::new(),
        }
    }

    /// LAN 模式下的 Host 判定：loopback 恒放行；非 loopback 时只有「精确命中
    /// 主机名白名单」或「标准格式的 IP 字面量直连」才放行。后缀/子域不再匹配，
    /// 杜绝 evil.allowed.example 之类的 DNS rebinding 绕过；authority_host 已
    /// 拒绝 userinfo，IP 字面量仅接受 IpAddr::from_str 能解析的标准格式。
    fn host_allowed(&self, authority: &str) -> bool {
        if parse_loopback_authority(authority).is_some() {
            return true;
        }
        if !self.lan_mode {
            return false;
        }
        // authority_host 拒绝 userinfo（含 '@'）与非法端口，并统一小写。
        let Some(host) = authority_host(authority) else {
            return false;
        };
        // IP 直连不受白名单限制（README 约定），但只认标准格式。
        if host.parse::<std::net::IpAddr>().is_ok() {
            return true;
        }
        // 精确匹配，大小写不敏感；不再用 ends_with(".{suffix}") 放开子域。
        self.allowed_hosts
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed.as_str()))
    }

    fn cookie(&self) -> String {
        // 12 小时（43200s）有效期，缩短会话 token 的暴露窗口；保留
        // HttpOnly / SameSite=Strict / Path=/。刻意不加 Secure：LAN 模式是纯
        // HTTP，加 Secure 后浏览器根本不会保存该 cookie，控制面会完全不可用。
        // LAN 链路上 token 与全量凭据均为明文传输，建议用 Tailscale/隧道或
        // 反向代理 TLS 保护，而不是指望 cookie 属性。
        format!(
            "kitty_pro_control={}; Max-Age=43200; Path=/; HttpOnly; SameSite=Strict",
            self.token
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn control_session_guard(
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

/// 所有方法（含 GET）统一校验会话与请求来源，GET 不再短路跳过 Origin 校验。
/// 判定顺序：
/// 1. Host 必须存在、命中 host_allowed，且带正确会话 cookie；
/// 2. Sec-Fetch-Site 若存在且不是 same-origin/none，直接拒绝（跨站/同站跨源）；
/// 3. Origin 若存在，必须与 Host 严格同源（或命中 dev server 回环别名）；
/// 4. 两个头都不存在时（curl/旧客户端/非浏览器 fetch）仅放行纯 loopback 访问。
#[cfg(not(target_arch = "wasm32"))]
fn valid_control_request(request: &Request<Body>, session: &ControlSession) -> bool {
    let headers = request.headers();
    let Some(host) = headers.get(header::HOST).and_then(header_text) else {
        return false;
    };
    if !session.host_allowed(host) || !has_session_cookie(headers, &session.token) {
        return false;
    }

    let sec_fetch_site = headers.get("sec-fetch-site");
    if let Some(value) = sec_fetch_site {
        let allowed =
            header_text(value).is_some_and(|site| site == "same-origin" || site == "none");
        if !allowed {
            return false;
        }
    }

    if headers.get(header::ORIGIN).is_some() {
        return same_origin(headers, host, session);
    }

    if sec_fetch_site.is_none() {
        // 旧客户端 / curl / 非浏览器 fetch：没有任何来源元数据，只信纯本机访问。
        return parse_loopback_authority(host).is_some();
    }

    // 同域 fetch 通常带 Sec-Fetch-Site: same-origin 而不带 Origin，放行正常 UI。
    true
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
    // 严格同源，大小写不敏感。删除了 LAN 模式复用 host_allowed(authority) 的
    // 放宽分支：那是 DNS rebinding 的关键缺口，白名单命中不等于同源。
    if authority.eq_ignore_ascii_case(host) {
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
    false
}

/// 从 authority（可能含端口、IPv6 方括号）取出规范化后的主机部分。
/// 与 `parse_loopback_authority` 同样的严格口径：拒绝 userinfo（含 '@'）。
///
/// Q8 规范化口径（统一在 host_allowed 之前完成，不放宽白名单）：
/// 1. 拒绝 userinfo；2. IPv6 方括号内的内容必须是合法 IP 字面量（剥离方括号
///    后再做 `IpAddr::from_str`），`]` 之后只允许空或 `:数字端口`，杜绝用
///    `[allowed.example]` 之类的括号形式伪装；3. 剥掉绝对域名尾部的单个根点
///    （`allowed.example.` 与 `allowed.example` 是同一主机），统一小写；
/// 4. 返回后仍只做「精确主机名」`eq_ignore_ascii_case` 匹配与 IP 字面量直连，
///    不放宽到子域/后缀，也不新增白名单条目。IDN/punycode 未登记外域依旧拒绝。
#[cfg(not(target_arch = "wasm32"))]
fn authority_host(authority: &str) -> Option<String> {
    if authority.contains('@') {
        return None;
    }
    let host = if let Some(rest) = authority.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        // 方括号内必须是合法 IP 字面量；方括号后只能是空或 :数字端口。
        if host.parse::<std::net::IpAddr>().is_err() {
            return None;
        }
        if !suffix.is_empty() {
            let port = suffix.strip_prefix(':')?;
            if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
        }
        host
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => host,
            Some((_, _)) => return None,
            None => authority,
        }
    };
    // 尾点规范化 + 小写：只统一表示形式，不改变上述精确匹配语义。
    let host = host.strip_suffix('.').unwrap_or(host);
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn request_with_site(
        method: Method,
        host: &str,
        origin: Option<&str>,
        sec_fetch_site: Option<&str>,
        token: &str,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/core/tun/prepare")
            .header(header::HOST, host)
            .header(header::COOKIE, format!("kitty_pro_control={token}"));
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        if let Some(site) = sec_fetch_site {
            builder = builder.header("sec-fetch-site", site);
        }
        builder.body(Body::empty()).expect("request should build")
    }

    fn request(method: Method, host: &str, origin: Option<&str>, token: &str) -> Request<Body> {
        request_with_site(method, host, origin, None, token)
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
    fn requests_without_fetch_metadata_only_allow_loopback() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: true,
            allowed_hosts: vec!["allowed.example".to_string()],
        };
        // 规则 4：Origin 与 Sec-Fetch-Site 都不存在（curl/旧客户端），纯本机
        // loopback 访问放行。
        assert!(valid_control_request(
            &request_with_site(Method::POST, "127.0.0.1:8080", None, None, "secret"),
            &session
        ));
        // 非 loopback 主机名即便命中白名单，没有任何来源元数据也必须拒绝。
        assert!(!valid_control_request(
            &request_with_site(Method::POST, "allowed.example:8080", None, None, "secret"),
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
        for (host, origin, site, token) in [
            (
                "127.0.0.1:50622",
                Some("http://localhost:9090"),
                None,
                "secret",
            ),
            (
                "127.0.0.1:50622",
                Some("http://evil.test:8080"),
                None,
                "secret",
            ),
            (
                "127.0.0.1:50622",
                Some("ftp://localhost:8080"),
                None,
                "secret",
            ),
            (
                "127.0.0.1:50622",
                Some("http://localhost:8080"),
                None,
                "wrong",
            ),
            (
                "evil.test:50622",
                Some("http://localhost:8080"),
                None,
                "secret",
            ),
            // 无 Origin 但标记跨站：仍必须拒绝（规则 2），不再依赖「缺 Origin 即拒绝」。
            ("127.0.0.1:50622", None, Some("cross-site"), "secret"),
        ] {
            assert!(!valid_control_request(
                &request_with_site(Method::POST, host, origin, site, token),
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
            allowed_hosts: vec!["office-mac".to_string(), "example.net".to_string()],
        };
        // loopback 恒放行
        assert!(session.host_allowed("localhost:8080"));
        assert!(session.host_allowed("127.0.0.1:8080"));
        // 白名单精确主机名放行（大小写不敏感）
        assert!(session.host_allowed("office-mac:8080"));
        assert!(session.host_allowed("OFFICE-MAC:8080"));
        assert!(session.host_allowed("example.net:8080"));
        // 后缀/子域不再放行（防 evil.allowed.example 绕过）
        assert!(!session.host_allowed("office-mac.example.net:8080"));
        assert!(!session.host_allowed("foo.example.net:8080"));
        assert!(!session.host_allowed("evil.example.net:8080"));
        // IP 字面量直连放行（tailnet / LAN 地址）
        assert!(session.host_allowed("198.51.100.9:8080"));
        assert!(session.host_allowed("192.168.1.3:8080"));
        assert!(session.host_allowed("[::1]:8080"));
        // 未登记的外域主机名拒绝（防 DNS rebinding）
        assert!(!session.host_allowed("evil.test:8080"));
        assert!(!session.host_allowed("not-office-mac:8080"));
        // userinfo 伪装的 host 拒绝
        assert!(!session.host_allowed("evil.test@office-mac:8080"));
    }

    #[test]
    fn loopback_mode_rejects_non_loopback_host() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: vec!["example.net".to_string()],
        };
        assert!(session.host_allowed("localhost:8080"));
        // 未开启 LAN 模式时白名单不生效，外域与非 loopback 一律拒绝
        assert!(!session.host_allowed("office-mac:8080"));
        assert!(!session.host_allowed("192.168.1.3:8080"));
    }

    #[test]
    fn get_with_cross_site_origin_is_rejected() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: Vec::new(),
        };
        // S1：GET 不再短路跳过 Origin 校验。旧实现 `GET || same_origin(..)`
        // 会让下面带跨站 Origin 的 GET 直接通过。
        assert!(!valid_control_request(
            &request(
                Method::GET,
                "127.0.0.1:8080",
                Some("http://evil.test:8080"),
                "secret"
            ),
            &session
        ));
        // 严格同源的 GET 仍放行。
        assert!(valid_control_request(
            &request(
                Method::GET,
                "127.0.0.1:8080",
                Some("http://127.0.0.1:8080"),
                "secret"
            ),
            &session
        ));
    }

    #[test]
    fn sec_fetch_site_cross_site_is_rejected() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: Vec::new(),
        };
        for bad in ["cross-site", "same-site", "bogus"] {
            assert!(
                !valid_control_request(
                    &request_with_site(
                        Method::GET,
                        "127.0.0.1:8080",
                        Some("http://127.0.0.1:8080"),
                        Some(bad),
                        "secret"
                    ),
                    &session
                ),
                "{bad} 必须被拒绝"
            );
        }
        for good in ["same-origin", "none"] {
            assert!(
                valid_control_request(
                    &request_with_site(
                        Method::GET,
                        "127.0.0.1:8080",
                        Some("http://127.0.0.1:8080"),
                        Some(good),
                        "secret"
                    ),
                    &session
                ),
                "{good} 应放行"
            );
        }
    }

    #[test]
    fn same_origin_fetch_without_origin_is_allowed() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: Vec::new(),
        };
        // 同域 GET fetch 通常不带 Origin，但带 Sec-Fetch-Site: same-origin/none：
        // 规则 2/3 不应破坏正常 UI 请求。
        for site in ["same-origin", "none"] {
            assert!(valid_control_request(
                &request_with_site(Method::GET, "127.0.0.1:8080", None, Some(site), "secret"),
                &session
            ));
        }
    }

    #[test]
    fn same_origin_ignores_lan_allowlist() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: true,
            allowed_hosts: vec!["allowed.example".to_string()],
        };
        // 严格同源仍放行。
        assert!(valid_control_request(
            &request(
                Method::GET,
                "allowed.example:8080",
                Some("http://allowed.example:8080"),
                "secret"
            ),
            &session
        ));
        // S2：删除 LAN 放宽分支后，Origin 命中白名单/IP 直连但 != Host 必须拒绝。
        // 旧实现 same_origin 里 `lan_mode && host_allowed(authority)` 会放行。
        assert!(!valid_control_request(
            &request(
                Method::GET,
                "127.0.0.1:8080",
                Some("http://192.168.1.3:8080"),
                "secret"
            ),
            &session
        ));
    }

    #[test]
    fn lan_allowlist_uses_exact_host_match() {
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: true,
            allowed_hosts: vec!["allowed.example".to_string()],
        };
        // 精确主机名放行，大小写不敏感。
        assert!(session.host_allowed("allowed.example:8080"));
        assert!(session.host_allowed("ALLOWED.EXAMPLE:8080"));
        // 后缀/子域一律拒绝：DNS rebinding 的关键绕过点。
        assert!(!session.host_allowed("evil.allowed.example:8080"));
        assert!(!session.host_allowed("sub.allowed.example:8080"));
        assert!(!session.host_allowed("allowed.example.evil.test:8080"));
        // IP 字面量 Host 放行（README 约定 IP 直连无需白名单）。
        assert!(session.host_allowed("192.168.1.3:8080"));
        assert!(session.host_allowed("[2001:db8::1]:8080"));
        // 非标准 IP 字面量（十进制整数）不放行。
        assert!(!session.host_allowed("3232235779:8080"));
        // userinfo 伪装拒绝。
        assert!(!session.host_allowed("evil.test@allowed.example:8080"));
    }

    #[test]
    fn host_allowlist_normalizes_trailing_dot_and_rejects_lookalikes() {
        // Q8：Host 头规范化差异不产生绕过。尾点被剥掉后仍是精确匹配；
        // 白名单本体带尾点同样归一（写入方与比对方口径一致）。
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: true,
            allowed_hosts: vec!["allowed.example".to_string()],
        };
        assert!(session.host_allowed("allowed.example.:8080"));
        // 尾点不能被用来伪装成后缀之外的另一个主机。
        assert!(!session.host_allowed("evil.allowed.example.:8080"));
        assert!(!session.host_allowed("allowed.example.evil.test.:8080"));
        // IDN：authority_host 不做 punycode 转换，白名单存什么就精确比什么。
        // Unicode 形式与 xn-- 形式都不是 allowed.example，双向都拒绝。
        assert!(!session.host_allowed("allowed.exämple:8080"));
        assert!(!session.host_allowed("xn--exmple-cua.example:8080"));
        // 大写 IDN 规范化后仍与白名单精确比对失败。
        assert!(!session.host_allowed("ALLOWED.EXAMPLE.EVIL.TEST:8080"));
        // authority_host 对 IPv6 剥离方括号、对尾点统一剥离。
        assert_eq!(authority_host("[::1]:8080").as_deref(), Some("::1"));
        assert_eq!(
            authority_host("allowed.example.:8080").as_deref(),
            Some("allowed.example")
        );
        assert_eq!(authority_host("user@allowed.example:8080"), None);
    }

    // Q6 的源码级 CORS 守卫在 cli.rs 的
    // `dioxus_router_never_enables_permissive_cors`：它检查 main.rs 源码，
    // 因此探测字符串必须放在 cli.rs（放这里会先匹配到本文件自身）。

    #[test]
    fn cookie_sets_limited_lifetime_without_secure() {
        let session = ControlSession {
            token: "a".repeat(64),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: Vec::new(),
        };
        let cookie = session.cookie();
        assert!(cookie.contains("kitty_pro_control="));
        assert!(cookie.contains("Max-Age=43200"));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        // 纯 HTTP LAN 下不能加 Secure；也不加 Expires。
        assert!(!cookie.contains("Secure"));
        assert!(!cookie.contains("Expires"));
    }

    #[test]
    fn session_token_rotates_on_each_startup() {
        // S3：每次启动都生成新 token 并覆盖 web-session.key，不再读取旧值。
        // 旧实现第二次调用会命中首次写入的文件而返回同一个 token。
        //（写入应用数据目录，写失败静默忽略，不影响断言。）
        let first = persisted_session_token().expect("token should be generated");
        let second = persisted_session_token().expect("token should be generated");
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(second.len(), 64);
        assert!(second.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second, "重启应轮换 token，不能复用旧值");
    }

    #[test]
    fn get_api_profile_cross_origin_cors_is_rejected_by_strict_same_origin() {
        // Q6 / S1 运行时回归：构造带正确会话 cookie、但 Origin 为跨站
        // https://evil.example 的 GET /api/profile 请求。Dioxus server 默认不
        // 开启任何跨域放行配置，且 valid_control_request 的严格同源判定必须
        // 对该请求返回 false——cookie 正确也不能替代同源。用例名明确写出 CORS
        // 语义，配合 cli.rs 的源码守卫防止以后误加通配来源放行配置。
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: false,
            allowed_hosts: Vec::new(),
        };
        let request = Request::builder()
            .method(Method::GET)
            .uri("/api/profile")
            .header(header::HOST, "127.0.0.1:8080")
            .header(header::ORIGIN, "https://evil.example")
            .header(header::COOKIE, "kitty_pro_control=secret")
            .body(Body::empty())
            .expect("request should build");
        assert!(!valid_control_request(&request, &session));
    }

    #[test]
    fn host_allowlist_normalizes_case_trailing_dot_ipv6_and_userinfo() {
        // Q8：白名单规范化差异的正向/负向用例。收紧或补规范化，不放宽白名单。
        let session = ControlSession {
            token: "secret".to_string(),
            dev_server_authority: None,
            lan_mode: true,
            allowed_hosts: vec!["allowed.example".to_string()],
        };
        // 大小写：ASCII 大小写不敏感（既有口径）。
        assert!(session.host_allowed("allowed.example:8080"));
        assert!(session.host_allowed("ALLOWED.EXAMPLE:8080"));
        // 尾点：绝对 FQDN 的单个根点是同一主机的另一种写法，规范化后精确命中。
        assert!(session.host_allowed("allowed.example.:8080"));
        assert!(session.host_allowed("ALLOWED.EXAMPLE.:8080"));
        // 但绝不因此放开子域/后缀（防 DNS rebinding），这是「不要放宽白名单」。
        assert!(!session.host_allowed("evil.allowed.example.:8080"));
        assert!(!session.host_allowed("allowed.example.evil.test.:8080"));
        // IPv6 带方括号：剥离方括号后按 IP 字面量直连放行（README 约定）。
        assert!(session.host_allowed("[::1]:8080"));
        assert!(session.host_allowed("[2001:db8::1]:8080"));
        // 方括号内不是合法 IP、或方括号后畸形 → 拒绝，不借括号伪装。
        assert!(!session.host_allowed("[allowed.example]:8080"));
        assert!(!session.host_allowed("[::1]evil:8080"));
        // 含 userinfo（evil@allowed.example）恒拒绝。
        assert!(!session.host_allowed("evil@allowed.example:8080"));
        assert!(!session.host_allowed("evil@ALLOWED.EXAMPLE.:8080"));
        // punycode / IDN 未登记外域一律拒绝，精确匹配不放宽。
        assert!(!session.host_allowed("xn--e1afmkfd.example:8080"));
        assert!(!session.host_allowed("例子.测试:8080"));

        // 直接断言 authority_host 的规范化口径。
        assert_eq!(
            authority_host("allowed.example.:8080").as_deref(),
            Some("allowed.example")
        );
        assert_eq!(
            authority_host("ALLOWED.EXAMPLE.").as_deref(),
            Some("allowed.example")
        );
        assert_eq!(
            authority_host("[2001:db8::1]:8080").as_deref(),
            Some("2001:db8::1")
        );
        assert_eq!(authority_host("evil@allowed.example"), None);
        assert_eq!(authority_host("[allowed.example]:8080"), None);
    }
}
