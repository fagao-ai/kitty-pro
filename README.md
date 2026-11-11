# Kitty Pro

Kitty Pro is a native proxy client: an [NSH](https://nsh.dev) app in Rust,
with a shared subscription and configuration layer. The native core is a
statically linked sing-box Go archive exposed through the `singbox` Rust crate;
the application does not discover, start, or require a `sing-box` executable at
runtime.

The desktop app is one process: interface, backend and core, with no server, no
WebAssembly and no HTTP hop. The interface is `packages/nsh-app`.

A browser can drive the same core instead. A page cannot load a Go archive or
raise a TUN interface, so that build is two halves: `packages/server` is a native
process that owns the proxy and answers `/api/*`, and it serves the NSH site next
to the API. See [Run as a web control surface](#run-as-a-web-control-surface).

The Dioxus shells that used to sit here (`packages/ui`, `web`, `desktop`,
`mobile`) have been deleted.

## Supported subscription inputs

- Base64 and unpadded Base64 share-link subscriptions
- Clash YAML `proxies` subscriptions
- Hysteria2, VMess, VLESS, Trojan, Shadowsocks, HTTP/HTTPS proxy, and SOCKS5 nodes

HTTP and SOCKS upstream proxies can be imported as a single-node source. Use
the actual upstream protocol in the URL. For a corporate endpoint that exposes
SOCKS5, prefer `socks5://`; Kitty Pro's local mixed inbound still accepts HTTP,
HTTPS, and SOCKS5 clients. Add a fragment to give the node a useful display
name:

```text
socks5://198.51.100.20:11080#Company
http://proxy.example.com:8080#HTTP
https://user:password@proxy.example.com:8443#Secure
```

Select the upstream node and use Rule mode to route matching traffic either to
the selected proxy, directly, or to the block outbound. System proxy mode
covers proxy-aware desktop applications. Android VpnService captures system
traffic; desktop TUN additionally requires the platform's network privileges.
Literal IP proxy endpoints are excluded from desktop TUN routes to prevent the
upstream connection from looping back into the tunnel.

### IPv6 behavior

Domains routed DIRECT (geosite-cn plus custom direct rules) are answered
IPv4-only, so applications never receive AAAA records for them. This avoids
the classic failure where WeChat and other Tencent apps prefer IPv6 for media
uploads and get stuck when the host has no real IPv6 connectivity (text
traffic over IPv4 keeps working while image uploads fail). Clients with
built-in IPv6 server lists (WeChat's Tencent endpoints) still attempt IPv6
connections; those fail immediately with `no route to host` and fall back to
IPv4.

Proxied domains keep the fake-ip IPv6 range (`fc00::/18`), but the client
only ever talks to a fake address that the TUN restores to the original
domain before dialing through the proxy tunnel, so no real IPv6 packets leave
the host on the proxied path either. IPv6-only destinations remain reachable
because the remote proxy server performs the IPv6 dial.

`proxy-core` owns parsing and sing-box JSON generation, so the interface and
the engine work from the same Rust types and configuration logic.

## Go toolchain

sing-box `1.13.14` requires Go 1.24 and currently does not link with Go 1.26.
On macOS the build script automatically selects Homebrew's keg-only
`go@1.24`; a different toolchain can be supplied with `SINGBOX_GO`.

```sh
HTTP_PROXY=http://proxy.example.com:11080 \
HTTPS_PROXY=http://proxy.example.com:11080 \
ALL_PROXY=http://proxy.example.com:11080 \
brew install go@1.24
```

The build never assumes a proxy. Set `HTTP_PROXY`, `HTTPS_PROXY`, and
`ALL_PROXY` as usual, or export `KITTY_BUILD_PROXY=http://host:port` to apply
one proxy to the Go build when those three are unset.

## Run locally

From the repository root:

```sh
cargo run -p nsh-app-kitty --features desktop   # a real window
nsh run                                        # hot reload, every target (needs the nsh CLI)
cargo test -p nsh-app-kitty                    # the screens, headless under NSH's engine
```

`desktop` is a Cargo feature because the Linux host needs GTK 4.14 development
packages, which CI runners do not carry; `cargo check --workspace` stays green
without them.

The app links and calls the same `singbox` Rust API in its own process.

## Run as a web control surface

A browser cannot load a Go archive or raise a TUN interface, so the web build is
two halves: a native process that owns the proxy and answers `/api/*`, and the
NSH site served next to it from the same listener. Serving both from one port is
required, not a preference — the request guard rejects any call whose `Origin`
does not equal its `Host`.

```sh
# 1. the interface
(cd packages/nsh-app && nsh package --target web -o /tmp/kitty-web)

# 2. the process that runs the proxy, pointed at the site just built
KITTY_WEB_DIR="/tmp/kitty-web/Kitty Pro" ./target/debug/kitty-pro-web-server
# → Kitty Pro 控制面: http://127.0.0.1:7897
```

`--port`, `--daemon`, `--log-file` and `--allow-lan` behave as the deleted Dioxus
web server had them: binding a non-loopback address needs `--allow-lan` (or
`KITTY_PRO_ALLOW_LAN=1`), and hostnames beyond IP-literal access need
`KITTY_PRO_ALLOWED_HOSTS` as an exact-match list. The session cookie is generated
per process, so restarting the server signs out every page.

The page reaches this process through `packages/nsh-app/src/backend.rs`: one table
of endpoints with two arms, so a native build calls `api` directly and a browser
build posts to `/api/*`. No screen knows which side it is on — the signatures,
including the error type, are the same on both.

Verified in a real headless Chrome rather than only by unit test: the overview
renders the server's own answers (the core's `EMBEDDED · 1.13.14` version, a
`发现新版本 v1.0.13` banner from the update check, this machine's actual
system-proxy setting), and a write made from inside the page — `POST /api/profile`,
read back, `profile.json` on disk — round-trips with the session cookie attached.
No console errors.

On desktop systems, enabling TUN performs configuration validation and asks
for network privileges immediately. Starting the core then reuses that
authorized helper instead of prompting a second time. The GUI itself remains
unprivileged:

- Windows starts a separate elevated core through the standard UAC prompt.
- Linux starts a separate root core through `pkexec`; a working polkit agent
  and `/dev/net/tun` are required.
- macOS keeps using its separately authorized helper and session-scoped DNS.

The helper accepts commands only through a random local session and holds a
lifecycle lease from the GUI. Closing Kitty Pro releases the lease, stops the
embedded core, and removes TUN routes even if the normal shutdown request
cannot complete. AppImage/portable builds do not install a persistent root
service; Linux desktops without polkit should install the distribution's
polkit package and agent before using TUN.

## Remote rule and subscription sync

The Settings page can synchronize custom rules and subscriptions, including
their last parsed nodes, through WebDAV or an S3-compatible object store. TUN,
system proxy, LAN exposure, and appearance settings remain device-local.

- WebDAV accepts an HTTP/HTTPS base URL, optional Basic Auth credentials, and
  creates missing collections in the configured remote path.
- S3 uses AWS Signature Version 4. Leave the endpoint empty for AWS S3; custom
  MinIO, R2, NAS, and other compatible endpoints use path-style object URLs.
- Each provider keeps its own address field, so switching between WebDAV and S3
  never reuses or overwrites the address saved for the other provider.
- Normal uploads use the remote ETag with conditional writes, so another
  device cannot overwrite the object between the version check and upload.
  Changing the provider, endpoint, bucket/account, or object path resets the
  local sync baseline. Stores that do not expose ETags require the separate,
  two-step confirmed remote overwrite action.
- Downloads persist the merged local profile before advancing the remote
  revision. The UI provides a separate two-step confirmation before replacing
  local rules and subscriptions.

Remote snapshots contain subscription URLs and proxy credentials. Use a
trusted server and HTTPS. Storage credentials stay in Kitty Pro's private
local `sync.json` and are never included in the remote snapshot.

## GitHub Actions packages

The `Build release packages` workflow runs `nsh package` on each operating
system's native GitHub runner and publishes one archive per platform:

- macOS (Apple Silicon): `Kitty-Pro-macos-aarch64.zip`, holding the ad hoc
  signed `Kitty Pro.app`
- Windows (x86_64): `Kitty-Pro-windows-x86_64.zip`
- Linux (x86_64): `Kitty-Pro-linux-x86_64.tar.gz`

Each archive gets a `.sha256` sidecar. Run the workflow manually from the
repository's **Actions** page, or push a tag such as `v1.0.14`; a `v*` tag also
creates a GitHub Release and attaches the archives.

Only the desktop app is published. `packages/server` builds, is tested and drives a
browser, but the workflow does not package it into a release archive yet. Android
and iOS have no build at all, for the unrelated reasons recorded in
`packages/nsh-app/README.md`.

The macOS app is ad hoc signed and the Windows and Linux builds are unsigned.
Apple notarization and Windows Authenticode signing require developer
certificates and should be added before public distribution.

## Checking for updates

The Settings page asks GitHub for the newest release of the repository, compares
it against the running shell's package version, and offers the release page when
something newer exists. Checks run at startup and at most once a day, remembered
in `<data dir>/update.json` beside `profile.json`, and can be switched off; the
button on the card ignores the throttle.

The check reads only GitHub's own API and never downloads or installs anything.
Each platform ships an archive that the user opens themselves, so the last step
is a link - which is also where the `.sha256` file published next to every
asset stays useful.

Which asset belongs to which install is decided from the file names the release
workflow produces, so a renamed package breaks the match and shows up in the
ignored live test rather than as a silent wrong download:

```sh
cargo test -p api --lib live_release -- --ignored
```

Anything the check cannot classify - a malformed tag, an asset that names no
operating system, a CPU with no published build - is reported as "nothing to
install", never as an invitation to fetch a binary that cannot run. The card
shows the target it matched against (`桌面版 · macos · aarch64`) so "no package
for you" is a specific answer.

A check the user did not start only ever announces a release worth installing.
Failure belongs to the card, which reads "检查失败" with the reason, and to the
manual button's toast; a startup check that cannot reach GitHub stays quiet,
because a red banner over an app the user is busy opening, on every launch, is
nagging about a fault that is already written on the card.

A fork checks its own releases by building with
`KITTY_UPDATE_REPO=acme/kitty-pro`, which stops it from prompting users to
install someone else's binaries.

## Verification

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p singbox --features embedded-core
```

The embedded-core test starts a VLESS configuration through the static bridge
and proves that no external executable is required.

Layout and dialog behaviour are covered by the crate's own headless tests,
which run the real NSH engine:

```sh
cargo test -p nsh-app-kitty
```

`nsh test` does the same from the app's directory and adds NSH's own build
diagnostics. There is no browser suite: the UI is compiled native views, so
there is no page for one to drive.

## Platform boundary

The native C archive bridge is validated on macOS and is structured for Linux
and Windows targets. Android passes its `VpnService` TUN descriptor through a
thin `experimental/libbox` platform layer. iOS still requires the equivalent
NetworkExtension integration. This is a platform integration requirement, not
a second proxy implementation; parsing, configuration, and the Rust lifecycle
facade remain shared.
