# Kitty Pro

Kitty Pro is a Dioxus 0.7 proxy client with a shared Rust subscription and
configuration layer. The native core is a statically linked sing-box Go
archive exposed through the `singbox` Rust crate; the application does not
discover, start, or require a `sing-box` executable at runtime.

## Supported subscription inputs

- Base64 and unpadded Base64 share-link subscriptions
- Clash YAML `proxies` subscriptions
- Hysteria2, VMess, VLESS, Trojan, and Shadowsocks nodes

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
signed with the local Android debug certificate by default. Set
`ANDROID_KEYSTORE_PATH`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, and
optionally `ANDROID_KEY_PASSWORD` to use a release certificate.

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

Android builds fall back to a generated debug certificate, which is suitable
for direct installation but not Play Store publication. Add these GitHub
Actions secrets to sign with an existing release key:

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
