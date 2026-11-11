# kitty-server

The web control surface: this process runs the proxy, and a browser page drives it.

A browser cannot load a Go archive or raise a TUN interface, so `api` compiled to
WebAssembly can parse a profile and nothing else — every backend function is a stub
under `wasm32`. This binary is where those functions actually run.

```sh
cargo run -p kitty-server -- --port 7897
KITTY_WEB_DIR=/tmp/site ./target/debug/kitty-pro-web-server --daemon
```

## Layout

| File | What |
| --- | --- |
| `routes.rs` | The 32 endpoints, and the wire format both halves must agree on |
| `guard.rs` | Session token, `Host`/`Origin`/`Sec-Fetch-Site` checks. Ported verbatim |
| `cli.rs` | `--port`/`--daemon`/`--allow-lan`, and the listener address. Ported verbatim |
| `main.rs` | Process start, the TUN helper, listener, static site, shutdown |

`guard.rs` and `cli.rs` came out of the deleted `packages/web` unchanged apart from
module paths, and they carry their own 21 tests. Treat edits there as security
changes, not refactors: the exact-host matching, the strict same-origin comparison
and the constant-time token compare each close a hole an earlier revision had, and
the reason is in the comment above the line that fixes it.

## Wire format

`POST` bodies are one JSON object keyed by parameter name. A reply is the `Ok`
value directly, or `{"error":{"code":"E1002","message":"…"}}` with the status taken
from `ApiErrorCode::http_status`. `Ok(())` is the JSON literal `null`, never an
empty body — an empty body is not valid JSON and would fail every client that parses
unconditionally.

## Two things that are easy to get wrong

**Same origin is load-bearing.** The guard rejects a request whose `Origin` does not
equal its `Host`, so the site must be served by this process. Adding a `CorsLayer`
to let a page on another port call the API would not be a configuration change; it
would remove the defence the guard is the second half of.

**The router's fallback is ServeDir's own.** `Router::fallback_service(assets)`
followed by `Router::fallback(shell)` silently discards `assets` — both assign
`catch_all_fallback`, and the later call wins. `a_real_asset_is_served_from_disk…`
is the test that would notice, and it is the only one: every other response still
looks correct when the asset server stops running.
