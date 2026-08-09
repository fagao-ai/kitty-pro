# Kitty Pro

Kitty Pro is a Dioxus 0.7 proxy client with a shared Rust subscription and
configuration layer. The native core is a statically linked sing-box Go
archive exposed through the `singbox` Rust crate; the application does not
discover, start, or require a `sing-box` executable at runtime.

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
socks5://100.64.0.2:11080#Company
http://proxy.example.com:8080#HTTP
https://user:password@proxy.example.com:8443#Secure
```

Select the upstream node and use Rule mode to route matching traffic either to
the selected proxy, directly, or to the block outbound. System proxy mode
covers proxy-aware desktop applications. Android VpnService captures system
traffic; desktop TUN additionally requires the platform's network privileges.
Literal IP proxy endpoints are excluded from desktop TUN routes to prevent the
upstream connection from looping back into the tunnel.

`proxy-core` owns parsing and sing-box JSON generation, so every Dioxus shell
uses the same Rust types and configuration logic.

## Go toolchain

sing-box `1.13.14` requires Go 1.24 and currently does not link with Go 1.26.
On macOS the build script automatically selects Homebrew's keg-only
`go@1.24`; a different toolchain can be supplied with `SINGBOX_GO`.

```sh
HTTP_PROXY=http://100.64.0.2:11080 \
HTTPS_PROXY=http://100.64.0.2:11080 \
ALL_PROXY=http://100.64.0.2:11080 \
brew install go@1.24
```

The embedded-core build defaults its Go module downloads to that same proxy
when no `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY` is already configured.

## Run locally

From the repository root, run the Web control surface:

```sh
dx serve --package web --web --fullstack true --port 8080 --open false --interactive false
```

Open <http://127.0.0.1:8080>. The Web UI controls the native sing-box archive
inside the local fullstack server. A browser cannot load a Go/C archive or
create a system TUN interface directly, so its role is intentionally control
plane only.

Run the macOS desktop shell:

```sh
dx serve --package desktop --desktop --open false --interactive false
```

The desktop shell links and calls the same `singbox` Rust API locally.

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

## Build an Android APK

On macOS, install Go 1.24, OpenJDK 17, the Android SDK command-line tools,
and Android NDK. The build script uses the Homebrew defaults, but honors
`JAVA_HOME`, `ANDROID_SDK_ROOT`, and `ANDROID_NDK_HOME` when they are set.

```sh
./scripts/build-android-apk.sh
```

The result is an arm64-v8a APK for Android 7.0 and later at
`dist/android/Kitty-Pro-arm64-v8a.apk`. It contains both the Dioxus JNI
library and the embedded sing-box Go library. The release build is aligned and
signed with `~/.android/kitty-pro-release.jks` when that permanent key exists.
On macOS, its password can be read from the `com.kitty.pro.android.release`
Keychain item. Otherwise local builds reuse `~/.android/debug.keystore`. Set
`ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, and
optionally `ANDROID_KEY_PASSWORD` to use a permanent release certificate.
`ANDROID_VERSION_CODE` can override the automatically increasing version code.

Android upgrades require the same application ID, the same signing key, and a
non-decreasing version code. Keep the release keystore private and backed up:
losing it makes it impossible to update existing installations. Uninstalling
the app clears its subscriptions and settings.

## GitHub Actions packages

The `Build release packages` workflow builds on each operating system's native
GitHub runner and produces:

- macOS: `.app` and `.dmg`
- Windows: `.msi` and NSIS `.exe`
- Linux: `.AppImage` and `.deb`
- Web: macOS and Linux x86_64 fullstack servers, each with its `public` assets
- Android: an arm64-v8a `.apk`

Run it manually from the repository's **Actions** page, or push a tag such as
`v0.1.0`. Manual runs retain downloadable workflow artifacts for 14 days. A
`v*` tag also creates a GitHub Release and attaches the distributable files.

Android release builds require a permanent signing key. The workflow fails
when signing secrets are absent instead of generating a new certificate that
would conflict with earlier installations. Add these GitHub Actions secrets:

- `ANDROID_KEYSTORE_BASE64`: Base64-encoded JKS or PKCS12 keystore
- `ANDROID_KEYSTORE_PASSWORD`: Keystore password
- `ANDROID_KEY_ALIAS`: Signing key alias
- `ANDROID_KEY_PASSWORD`: Key password; may be omitted when it matches the
  keystore password

The macOS app uses ad-hoc signing and the Windows installers are unsigned.
Apple notarization and Windows Authenticode signing require developer
certificates and should be added before public distribution. iOS packaging is
intentionally deferred until its NetworkExtension and signing setup are ready.

## Verification

```sh
cargo test --workspace
cargo test -p singbox --features embedded-core
```

The embedded-core test starts a VLESS configuration through the static bridge
and proves that no external executable is required.

## Platform boundary

The native C archive bridge is validated on macOS and is structured for Linux
and Windows targets. Android passes its `VpnService` TUN descriptor through a
thin `experimental/libbox` platform layer. iOS still requires the equivalent
NetworkExtension integration. This is a platform integration requirement, not
a second proxy implementation; parsing, configuration, and the Rust lifecycle
facade remain shared.
