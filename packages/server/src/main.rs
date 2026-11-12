//! Kitty Pro's web control surface: this process is the proxy, the page is its remote.
//!
//! A browser cannot load a Go archive or raise a TUN interface, so the web build is
//! necessarily two halves — the NSH app compiled to WebAssembly, and this native
//! process that owns `api`, `proxy-core` and `singbox` and answers `/api/*`. That is
//! the shape the deleted Dioxus `packages/web` had; what changed is only who serves
//! the page, which is now `nsh package --target web` writing a static directory that
//! this binary serves beside the API.
//!
//! Serving both from one listener is not a packaging preference. The guard in
//! `guard.rs` rejects a request whose `Origin` does not equal its `Host`, so a page
//! hosted anywhere else could never call the API: same origin is what the security
//! model assumes.

mod cli;
mod guard;
mod routes;

use axum::{
    extract::State,
    handler::Handler,
    http::{header, StatusCode, Uri},
    middleware,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use guard::ControlSession;
use std::{path::PathBuf, sync::Arc};
use tower_http::services::ServeDir;

const DEFAULT_WEB_PORT: &str = "7897";

/// Where `nsh package --target web` left the site.
///
/// Resolved next to the executable because the release archive unpacks to a directory
/// the user chooses; `KITTY_WEB_DIR` overrides it for a layout that installs the assets
/// elsewhere. Deliberately not baked in at compile time — a `env!("CARGO_MANIFEST_DIR")`
/// default would name a build machine.
fn site_directory() -> PathBuf {
    if let Some(dir) = std::env::var_os("KITTY_WEB_DIR") {
        return PathBuf::from(dir);
    }
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    executable
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("public")
}

/// Run the privileged TUN helper when these are its arguments.
///
/// First thing `main` does, before binding: the helper is *this executable*
/// re-launched with admin rights and one flag, and a helper that fell through to the
/// listener would fail to bind a port the parent already holds rather than do its job.
fn dispatch_tun_helper() {
    #[cfg(target_os = "macos")]
    let code = singbox::macos::run_helper_from_args();
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let code = singbox::desktop_helper::run_helper_from_args();
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let code: Option<i32> = None;
    if let Some(exit_code) = code {
        std::process::exit(exit_code);
    }
}

fn main() {
    dispatch_tun_helper();

    let address = match cli::prepare() {
        Ok(Some(address)) => address,
        Ok(None) => return,
        Err(error) => {
            eprintln!("错误: {error}");
            std::process::exit(2);
        }
    };

    // A run that died before it could clean up leaves the OS pointing at a listener
    // that no longer exists; clear it before this process starts claiming that port.
    api::cleanup_stale_system_proxy_sentinel();

    let session = ControlSession::new().expect("control session token should be generated");
    let router = build_router(session, Arc::new(site_directory()));

    let runtime = tokio::runtime::Runtime::new().expect("a tokio runtime");
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .unwrap_or_else(|error| panic!("无法监听 {address}: {error}"));
        println!("Kitty Pro 控制面: http://{address}");
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .expect("the web server stopped unexpectedly");
    });

    // Normal shutdown only. A SIGKILL still cannot clean up, which is what the
    // sentinel above exists to recover from.
    if let Err(error) = api::shutdown_native_runtime() {
        eprintln!("退出前清理本机运行时失败: {error}");
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::{in_api_namespace, names_a_file};

    #[test]
    fn app_routes_get_the_shell_and_files_get_a_404() {
        for route in ["/", "/settings", "/nodes/detail", "/状态"] {
            assert!(!names_a_file(route), "{route} is an app route");
        }
        // Every asset NSH's own build emitted, named the way it names them.
        for file in [
            "/index.html",
            "/nsh.89070fe10a.js",
            "/nsh.cb2cc9ca6e.wasm",
            "/nsh-syntaxes.ea0da56d04.bin",
            "/manifest.webmanifest",
            "/icons/32.96af0cdbbe.png",
            "/browser.991ab24c49/animation.js",
        ] {
            assert!(names_a_file(file), "{file} is a file");
        }
    }

    #[test]
    fn the_api_namespace_includes_its_own_root() {
        for path in ["/api", "/api/", "/api/core", "/api/nope"] {
            assert!(in_api_namespace(path), "{path} is the API namespace");
        }
        assert!(
            !in_api_namespace("/apiary"),
            "an app route may start with api"
        );
    }

    #[test]
    fn an_unknown_endpoint_never_answers_with_the_shell() {
        // A 200 carrying HTML is what makes a wrong `/api/...` look like success to
        // a client that then fails to parse it, so this outranks the dot test.
        for path in ["/api", "/api/", "/api/nope", "/api/core"] {
            assert!(names_a_file(path), "{path} must 404 as an endpoint");
        }
    }
}

fn build_router(session: ControlSession, site: Arc<PathBuf>) -> Router {
    // The shell is ServeDir's *own* fallback, so a real file wins and only a miss
    // consults it. Not the router's: `Router::fallback_service(assets)` followed by
    // `Router::fallback(shell)` silently discards `assets` — the second call
    // overwrites `catch_all_fallback` — which would serve the shell for every asset
    // URL and still pass everything except the asset test below. (Here the type
    // system happens to catch it too, because the shell needs the site state; the
    // trap is live for any router whose fallback shares the state type.)
    let assets = ServeDir::new(site.as_ref().clone())
        .precompressed_br()
        .precompressed_gzip()
        .fallback(serve_shell.with_state(site));

    let mut router = Router::new().merge(routes::router());
    // `--daemon`'s parent polls this to learn the listener is live. It exists only in
    // the child, where the token environment variable is set, and it answers before
    // any cookie exists — see the guard's note on readiness.
    if let Some((path, token)) = cli::readiness_route() {
        router = router.route(&path, get(move || async move { token }));
    }
    router
        .fallback_service(assets)
        .layer(middleware::from_fn_with_state(
            session,
            guard::control_session_guard,
        ))
}

/// Whether a path is a file that is genuinely missing rather than an app route.
///
/// NSH routes on the client, so `/settings` is a real URL a user can bookmark and
/// only `index.html` can answer it. Two things must not get that treatment:
/// `/api/*`, because a mistyped endpoint that answers 200-with-HTML is how a client
/// ends up "succeeding" with a page instead of data; and anything with a dot in its
/// last segment, which is how every asset NSH emits is named
/// (`nsh.<hash>.wasm`, `index.html`, `icons/32.<hash>.png`) — serving the shell for a
/// missing `.wasm` turns one bad file into a page that loads and then does nothing,
/// which is far harder to diagnose than a 404 naming it.
fn names_a_file(path: &str) -> bool {
    in_api_namespace(path)
        || path
            .rsplit('/')
            .next()
            .is_some_and(|last| last.contains('.'))
}

/// Whether a path belongs to the API namespace, including the bare `/api`.
///
/// `starts_with("/api/")` alone leaves `/api` outside, which is exactly the request a
/// mistyped base URL produces — and it would be answered with the shell.
fn in_api_namespace(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

/// Answer an app route with the shell, and everything [`names_a_file`] rejects with a 404.
async fn serve_shell(State(site): State<Arc<PathBuf>>, uri: Uri) -> Response {
    let path = uri.path();
    if names_a_file(path) {
        return resource_missing(path);
    }
    match std::fs::read(site.join("index.html")) {
        // No `Cache-Control` here on purpose: the guard sets `no-store` on every GET
        // that is not an API call, which covers this response, and doing it twice from
        // two places is how one of them later drifts.
        Ok(body) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            Html(String::from_utf8_lossy(&body).into_owned()),
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            "未找到界面文件：请把站点目录放在可执行文件旁的 public/，或用 KITTY_WEB_DIR 指定",
        )
            .into_response(),
    }
}

/// A 404 that stays JSON, for the two kinds of miss that reach it.
///
/// Both are "the thing you named is not here", and neither should get HTML: a
/// client that probes `/api/<typo>` and receives a page reports a parse error
/// instead of the truth, and a missing `.wasm` served as the shell is worse still.
fn resource_missing(path: &str) -> Response {
    let message = if path.starts_with("/api/") {
        format!("没有这个接口: {path}")
    } else {
        format!("没有这个文件: {path}")
    };
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({ "error": { "code": "E1002", "message": message } }).to_string(),
    )
        .into_response()
}

/// A site on disk, so asset serving is tested against real files rather than a
/// stub that would agree to anything.
#[cfg(test)]
fn test_site() -> std::sync::Arc<std::path::PathBuf> {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("kitty-server-test-{}", std::process::id()));
    fs::create_dir_all(dir.join("icons")).expect("a site directory");
    fs::write(
        dir.join("index.html"),
        "<!doctype html><title>Kitty Pro test shell</title>",
    )
    .expect("a shell");
    fs::write(dir.join("nsh.abc123.wasm"), b"\0asm fake").expect("an asset");
    fs::write(dir.join("icons/32.def456.png"), b"png-bytes").expect("an icon");
    std::sync::Arc::new(dir)
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use axum::http::{header::HeaderValue, Method};
    use tower::util::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn router() -> Router {
        build_router(ControlSession::for_test(TOKEN), test_site())
    }

    async fn send(request: axum::http::Request<axum::body::Body>) -> (StatusCode, String) {
        let response = router().oneshot(request).await.expect("the router answers");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("a readable body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    fn request(method: &Method, path: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "127.0.0.1:7897")
            .body(axum::body::Body::empty())
            .expect("a request")
    }

    fn authorized(
        mut request: axum::http::Request<axum::body::Body>,
    ) -> axum::http::Request<axum::body::Body> {
        request.headers_mut().insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("kitty_pro_control={TOKEN}")).expect("a cookie"),
        );
        request
    }

    #[tokio::test]
    async fn a_real_asset_is_served_from_disk_not_answered_with_the_shell() {
        // The composition test. Every other response in this file still looks right
        // if the asset server stops running — the shell answers `/nsh.*.wasm` with a
        // 200 and some HTML — so this is the only thing that would notice.
        //
        // The wiring it protects against: `Router::fallback` after
        // `Router::fallback_service` replaces the asset server rather than backing it
        // up (confirmed against axum 0.8.9: both assign `catch_all_fallback`, and a
        // probe shows the later call wins in either order). That mistake was written
        // here during development and never compiled, so this test did not catch a
        // shipped bug — it is here because the failure it guards is silent, and
        // because a router that later shares one state type would not fail to
        // compile.
        let (status, body) = send(authorized(request(&Method::GET, "/nsh.abc123.wasm"))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "\0asm fake", "the file, not the shell");
    }

    #[tokio::test]
    async fn an_authorized_api_call_reaches_its_handler() {
        // What is being tested is the guard's decision, not the endpoint's answer, so
        // the assertion is "not refused" and the endpoint is chosen for costing
        // nothing: `/api/processes` is the one GET in the table that touches no file
        // at all (an in-memory process scan). `/api/core/status` would reach sing-box
        // discovery and `/api/update/settings` would read the developer's real
        // `update.json`; neither belongs in a unit test.
        let (status, _) = send(authorized(request(&Method::GET, "/api/processes"))).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "the guard must let this through"
        );
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "the route must be registered"
        );
    }

    #[tokio::test]
    async fn an_api_call_without_the_cookie_is_refused_before_any_handler_runs() {
        let (status, _) = send(request(&Method::POST, "/api/core/status")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_deep_link_gets_the_shell_and_a_missing_file_gets_a_json_404() {
        let (status, body) = send(authorized(request(&Method::GET, "/settings"))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("test shell"), "{body}");

        let (status, body) = send(authorized(request(&Method::GET, "/gone.wasm"))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.starts_with('{'), "JSON, not HTML: {body}");
    }
}
