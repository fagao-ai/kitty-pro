#!/usr/bin/env bash
# Build the APK.
#
# NSH packages Android apps itself: `nsh package --target android` builds the
# app's Rust crate into libnsh.so and assembles a signed APK with aapt2 and
# apksigner, with no Gradle. That is most of what Kitty Pro needs, and it is
# used as-is here. What it cannot do is add anything of its own to the app:
#
#   * A VpnService, which is how the core carries traffic on Android. NSH's
#     manifest template has one Activity and no service, and its runtime is a
#     prebuilt AAR whose manifest is pinned to its own package, so there is no
#     supported way to declare ours or to ask for BIND_VPN_SERVICE.
#   * libkitty_singbox.so, the Go core. packages/singbox/build.rs builds it
#     with -buildmode c-shared for Android and links it as a dynamic library,
#     and NSH copies only the one library it built. The APK it produces on its
#     own therefore fails at startup: libnsh.so names libkitty_singbox.so in
#     its NEEDED entries and nothing in the archive provides it.
#
# So this script runs NSH's packaging for the native half and assembles the
# package around it with the same tools, still no Gradle. The Kotlin under
# packages/mobile/android is the bridge the deleted Dioxus shell shipped with,
# retargeted: libnsh.so rather than libmain.so, a KittyApplication to hand the
# core its JavaVM (NSH loads the library from an ordinary Activity, so nothing
# runs ndk-glue, which is what would have done it), and an R generated from our
# own resources rather than wry's.
#
# Everything here is compiled for real, and the result has been installed and
# driven on an Android 15 emulator — connect through to a live tun0 included.
# What has not been seen is a physical device, or the 32-bit ABI running at all:
# the emulator image is arm64, so armeabi-v7a is compiled and link-checked only.
# See packages/nsh-app/README.md for the whole list, including what the emulator
# could not settle either.
#
# Needs: the Android SDK (ANDROID_HOME) with build-tools and a platform, an NDK
# for the core's C compiler and Rust's linkers, a JDK 17 or newer, Rust's
# android targets, the nsh CLI, Go, and a Kotlin compiler (KOTLIN_HOME, or
# $HOME/.local/kotlinc). jq reads nsh.json and `cargo metadata`.
#
# Usage: scripts/build-android.sh [-o OUT] [--skip-nsh] [--abi arm64-v8a]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app="$root/packages/nsh-app"
bridge="$root/packages/mobile/android"
work="${KITTY_ANDROID_WORK:-$root/target/android}"
out="$root/dist/android"
# `nsh package` has no profile flag and always builds the release runtime, so
# the Go core is built the same way: one profile for both halves of the APK.
skip_nsh=false
abis=(arm64-v8a armeabi-v7a)
# The SDK revision the script builds against. A newer one present on the
# machine is fine and is used when the pinned one is missing; the pin exists so
# a CI runner that installs exactly these gets the same result.
build_tools_want=35.0.1
platform_want=android-35

while [[ $# -gt 0 ]]; do
    case "$1" in
        -o) out="$2"; shift 2 ;;
        --skip-nsh) skip_nsh=true; shift ;;
        --abi)
            IFS=, read -r -a abis <<<"$2"
            shift 2
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

step() { printf '\033[36m==>\033[0m %s\n' "$*"; }
die() {
    echo "error: $*" >&2
    exit 1
}

# --- toolchain -------------------------------------------------------------

# Say which SDK won: the usual failure on a machine with two of them is that the
# stale one is found first.
if [[ -z "${ANDROID_HOME:-}" ]]; then
    for guess in "$HOME/Library/Android/sdk" "$HOME/Android/Sdk" /opt/android-sdk; do
        if [[ -d $guess ]]; then
            export ANDROID_HOME=$guess
            break
        fi
    done
fi
[[ -n "${ANDROID_HOME:-}" && -d ${ANDROID_HOME:-} ]] ||
    die "ANDROID_HOME is unset and no SDK is in the usual places; install the Android SDK"

pick_revision() {
    # Prefer the pinned revision, else the newest installed.
    local kind=$1 want=$2
    if [[ -d $ANDROID_HOME/$kind/$want ]]; then
        echo "$want"
        return
    fi
    ls -1 "$ANDROID_HOME/$kind" 2>/dev/null | sort -V | tail -1
}
build_tools="$(pick_revision build-tools "$build_tools_want")"
platform="$(pick_revision platforms "$platform_want")"
[[ -n "$build_tools" ]] ||
    die "no build-tools installed: sdkmanager \"build-tools;$build_tools_want\""
[[ -n "$platform" ]] ||
    die "no platform installed: sdkmanager \"platforms;$platform_want\""
bt="$ANDROID_HOME/build-tools/$build_tools"
android_jar="$ANDROID_HOME/platforms/$platform/android.jar"
[[ -f $android_jar ]] || die "$android_jar is missing"

ndk="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-${NDK_HOME:-}}}"
if [[ -z "$ndk" ]]; then
    ndk="$(ls -1d "$ANDROID_HOME"/ndk/*/ 2>/dev/null | sort -V | tail -1)"
    ndk="${ndk%/}"
fi
[[ -n "$ndk" && -d $ndk ]] ||
    die "no NDK: sdkmanager \"ndk;27.0.12077973\", or set ANDROID_NDK_HOME"

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) llvm_dir=darwin-arm64 ;;
    Darwin-x86_64) llvm_dir=darwin-x86_64 ;;
    *) llvm_dir="linux-$(uname -m)" ;;
esac
# NDK r27 ships a single darwin host directory and Rosetta runs it on Apple
# Silicon, so falling back is correct rather than a workaround.
[[ -d "$ndk/toolchains/llvm/prebuilt/$llvm_dir" ]] || llvm_dir=darwin-x86_64
llvm="$ndk/toolchains/llvm/prebuilt/$llvm_dir/bin"
[[ -d $llvm ]] || die "no NDK LLVM toolchain under $ndk/toolchains/llvm/prebuilt"

if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/javac" ]]; then
    javac="$JAVA_HOME/bin/javac"
    java_bin="$JAVA_HOME/bin/java"
    keytool="${JAVA_HOME}/bin/keytool"
else
    javac="$(command -v javac || true)"
    java_bin="$(command -v java || true)"
    keytool="$(command -v keytool || true)"
fi
[[ -n "$javac" && -n "$java_bin" ]] || die "no JDK: install JDK 17 or newer, or set JAVA_HOME"
command -v go >/dev/null || die "go is not installed; the embedded core is a Go archive"
# `jq` reads nsh.json and `cargo metadata`, and the release workflow already
# depends on it, so this is not a new requirement — but a missing `jq` fails the
# version lookup three steps later as a blank string, which reads like a broken
# manifest rather than a missing tool.
command -v jq >/dev/null || die "jq is not installed; it reads nsh.json and cargo metadata"

# The core is built against 24, which is what packages/singbox/build.rs
# defaults to. The UI is built against 28 because that is NSH runtime's own
# minSdkVersion, and a lower number would let the APK install on a device where
# libnsh.so cannot load.
core_api=24
ui_api=28

# Resolve the NSH installation, check it against the crates, and make sure the
# Android runtime is there. The script owns that check because the failure it
# prevents — a runtime and a libnsh.so from different NSH releases — is one this
# build cannot see and a phone answers with a crash.
step "nsh"
nsh_home="$("$root/scripts/setup-nsh.sh" android)"

# The NDK spells the 32-bit ARM triple `armv7a-linux-androideabi`; the other two
# end in `-linux-android`. Naming them per target beats deriving the suffix.
export ANDROID_NDK_HOME="$ndk"
export ANDROID_NATIVE_API_LEVEL=$core_api
ndk_clang_for() {
    case "$1" in
        aarch64-linux-android) echo aarch64-linux-android ;;
        armv7-linux-androideabi) echo armv7a-linux-androideabi ;;
        x86_64-linux-android) echo x86_64-linux-android ;;
        *) die "no NDK triple for Rust target $1" ;;
    esac
}
# bionic names its per-architecture header directory after the ABI, and 32-bit
# ARM is the one place where that differs from the compiler's name: the NDK
# spells the compiler `armv7a-linux-androideabi24-clang` while the headers live
# in `usr/include/arm-linux-androideabi`.
ndk_include_dir_for() {
    case "$1" in
        aarch64-linux-android) echo aarch64-linux-android ;;
        armv7-linux-androideabi) echo arm-linux-androideabi ;;
        x86_64-linux-android) echo x86_64-linux-android ;;
        *) die "no NDK include directory for Rust target $1" ;;
    esac
}

ndk_sysroot="$ndk/toolchains/llvm/prebuilt/$llvm_dir/sysroot"
for target in aarch64-linux-android armv7-linux-androideabi x86_64-linux-android; do
    triple="$(ndk_clang_for "$target")"
    compiler="$llvm/${triple}${core_api}-clang"
    [[ -x $compiler ]] || die "the NDK has no compiler at $compiler"
    # macOS still ships bash 3.2, which has no `${var^^}`, so the upper-case
    # spelling of the linker variable is built with tr rather than expansion.
    underscored="${target//-/_}"
    export "CC_${underscored}=$compiler"
    export "CARGO_TARGET_$(printf '%s' "$target" | tr '[:lower:]-' '[:upper:]_')_LINKER=$compiler"
    export "AR_${underscored}=$llvm/llvm-ar"

    # The JS engine in the proxy core generates its bindings at build time for
    # Android, and bindgen reads headers with the *host* clang unless it is told
    # otherwise. Left alone it keeps the `-target aarch64-unknown-linux-android`
    # flag but searches the host's /usr/include, so on Linux the first bionic
    # header that glibc implements differently — today `stdio.h`, which pulls in
    # `bits/libc-header-start.h` — ends the build inside a C++ `expect()`. On
    # macOS it silently reads Apple's headers instead, which is worse: the build
    # succeeds and proves nothing about what CI will compile. Pointing bindgen at
    # the NDK sysroot makes both machines compile the same declarations.
    bindgen_args_variable="BINDGEN_EXTRA_CLANG_ARGS_${underscored}"
    if [[ -z "${!bindgen_args_variable:-}" ]]; then
        include_dir="$(ndk_include_dir_for "$target")"
        [[ -d "$ndk_sysroot/usr/include/$include_dir" ]] ||
            die "the NDK sysroot has no bionic headers at $ndk_sysroot/usr/include/$include_dir"
        printf -v "$bindgen_args_variable" '%s' \
            "--sysroot='$ndk_sysroot' --target=${triple}${core_api} \
-I'$ndk_sysroot/usr/include' \
-I'$ndk_sysroot/usr/include/$include_dir'"
        export "$bindgen_args_variable"
    fi
done

rust_target_for() {
    case "$1" in
        arm64-v8a) echo aarch64-linux-android ;;
        armeabi-v7a) echo armv7-linux-androideabi ;;
        x86_64) echo x86_64-linux-android ;;
        *) die "no Rust target for ABI $1" ;;
    esac
}

for abi in "${abis[@]}"; do
    target="$(rust_target_for "$abi")"
    rustup target list --installed 2>/dev/null | grep -qx "$target" ||
        die "Rust target $target is missing: rustup target add $target"
done

# --- versions --------------------------------------------------------------

# The crate's version, which is what the release workflow stamps from the tag
# and what the in-app update check compares against. Read from cargo rather than
# from `nsh.json` because cargo is the one manifest this script cannot get wrong:
# `nsh.json` is stamped by the same hand, but only when a release workflow runs
# it, and a build from a tree nobody stamped should answer with the version the
# code actually carries.
version="$(cargo metadata --locked --no-deps --format-version 1 |
    jq -er '.packages[] | select(.name == "kitty-pro") | .version')"
# The integer Android compares on upgrade, and it has to be higher than the one on
# the phone or the package manager refuses the update: no error a user would
# recognise, and nothing fails in CI to say so.
#
# Epoch minutes, as `0e51542` chose them and as every APK up to v1.0.13 shipped
# with them (v1.0.13 carries 29897976). A clock is monotonic by construction, so
# this needs no floor to compare against and no release can be stamped lower than
# the last; the version-derived number this script used for a while put 1.0.14 at
# 1000014, below every APK already installed, which is the bug the clock exists to
# avoid. It is not reproducible -- two builds of one version differ -- which is
# what `KITTY_VERSION_CODE` is for: the value goes in the manifest NSH stamps and
# in the `aapt2 link` below from one variable, so the two cannot disagree about
# which build is newer.
#
# Bounded below Android's own ceiling, as the original was: past it the package
# manager declines the APK at install time.
build_number="${KITTY_VERSION_CODE:-$(($(date -u +%s) / 60))}"
[[ "$build_number" =~ ^[1-9][0-9]*$ ]] && ((build_number <= 2100000000)) ||
    die "the Android versionCode must be an integer from 1 to 2100000000, got '$build_number'"
app_name="$(jq -er '.name' "$app/nsh.json")"
application_id="$(jq -er '.id' "$app/nsh.json")"

mkdir -p "$work" "$out"
gen="$work/gen"
classes="$work/classes"
stage="$work/stage"

# --- 1. the app, as NSH builds it ------------------------------------------

step "nsh package --target android"
nsh_args=(--target android --app-version "$version" --build "$build_number")
for abi in "${abis[@]}"; do nsh_args+=(--abi "$abi"); done
if $skip_nsh; then
    step "reusing the libnsh.so from the last run"
else
    (cd "$app" && nsh package "${nsh_args[@]}" -o "$work/nsh")
fi
nsh_apk="$(find "$work/nsh" -name '*.apk' -print 2>/dev/null | head -1)"
[[ -n "$nsh_apk" ]] || die "nsh package left no APK to take the libraries from"
rm -rf "$work/nsh-x"
mkdir -p "$work/nsh-x"
(cd "$work/nsh-x" && unzip -o -q "$nsh_apk")
[[ -d "$work/nsh-x/lib" ]] || die "the packaged APK carries no lib/ directory"

# --- 2. the Go core, which NSH does not package ----------------------------

rm -rf "$stage"
mkdir -p "$stage"
for abi in "${abis[@]}"; do
    [[ -f "$work/nsh-x/lib/$abi/libnsh.so" ]] ||
        die "the packaged APK has no lib/$abi/libnsh.so"
    mkdir -p "$stage/lib/$abi"
    cp "$work/nsh-x/lib/$abi/libnsh.so" "$stage/lib/$abi/"

    # Build the bridge crate for this ABI so build.rs runs its Go cross-compile,
    # then take the artifact it left. Asking for singbox rather than the app
    # keeps this cheap once step 1 has built everything else.
    step "the embedded core for $abi"
    (cd "$root" && cargo build --release -p singbox --target "$(rust_target_for "$abi")" >/dev/null)
    core_dir="$root/target/$(rust_target_for "$abi")/release/build"
    core="$(find "$core_dir" -name libkitty_singbox.so -print 2>/dev/null | head -1)"
    [[ -n "$core" ]] ||
        die "no libkitty_singbox.so under $core_dir: the Go build produced nothing"
    cp "$core" "$stage/lib/$abi/"
done

# --- 3. resources, and the R they imply ------------------------------------

step "resources"
rm -rf "$gen" "$classes" "$work/res.zip"
mkdir -p "$gen" "$classes"
"$bt/aapt2" compile --dir "$bridge/res" -o "$work/res.zip"
"$bt/aapt2" link -o "$work/base.apk" \
    -I "$android_jar" \
    --manifest "$bridge/AndroidManifest.xml" \
    -R "$work/res.zip" \
    --java "$gen" \
    --min-sdk-version "$ui_api" \
    --target-sdk-version "${platform#android-}" \
    --version-code "$build_number" \
    --version-name "$version" \
    --auto-add-overlay
# R.class rather than a Kotlin reference to it: the resource constants have to
# exist before the bridge compiles, and javac is the shortest way to one.
"$javac" -nowarn -proc:none -d "$classes" -cp "$android_jar" "$gen/com/kitty/pro/R.java"

# The manifest's package is the app's identity and has to agree with the id the
# runtime is told about and with the `app_id` the root component sets. Three
# places, one string; a mismatch installs under one name and stores its profile
# under another.
manifest_package="$(sed -n 's/.*package="\([^"]*\)".*/\1/p' "$bridge/AndroidManifest.xml" | head -1)"
[[ "$manifest_package" == "$application_id" ]] ||
    die "AndroidManifest.xml packages $manifest_package but nsh.json says $application_id"

# --- 4. the bridge ---------------------------------------------------------

# Kotlin rather than a rewrite in Java: this is the DNS adapter and the tunnel
# code that already shipped on a phone, and translating it would be a chance to
# reintroduce a bug for the sake of one fewer compiler.
step "kotlin"
# The compiler as jars rather than a kotlinc launcher: that is how it ships on
# Maven Central, and it needs no second SDK to install. Its own runtime pieces —
# stdlib, reflect, coroutines, IntelliJ's trove — go on the *compiler's* classpath
# beside it, which is what a kotlinc script would have done.
kotlin_lib="${KOTLIN_HOME:+$KOTLIN_HOME/lib}"
kotlin_lib="${kotlin_lib:-$HOME/.local/kotlinc/lib}"
[[ -f "$kotlin_lib/kotlin-compiler.jar" ]] ||
    die "no Kotlin compiler in $kotlin_lib (set KOTLIN_HOME, or install one
     there). The bridge is the code that carries the traffic; rewriting it in
     Java to drop this dependency would trade logic that has run on a phone for
     logic that has not."
kotlin_stdlib="$kotlin_lib/kotlin-stdlib.jar"
[[ -f $kotlin_stdlib ]] || die "$kotlin_stdlib is missing"

"$java_bin" -cp "$kotlin_lib/*" org.jetbrains.kotlin.cli.jvm.K2JVMCompiler \
    -nowarn -no-stdlib -jvm-target 1.8 \
    -Xsam-conversions=class -Xstring-concat=inline \
    -cp "$android_jar:$kotlin_stdlib:$classes" \
    -d "$classes" "$bridge"/*.kt

# --- 5. one dex: the bridge and NSH's Java runtime -------------------------

# NSH's classes are the ones the manifest names as the entry point, so they have
# to be in the package. The runtime directory is the same one `nsh package`
# compiled libnsh.so against, which is why this is that file rather than the AAR —
# and why it is read out of the installation the `nsh` on PATH belongs to: an APK
# whose dex comes from a different NSH than its libnsh.so is a JNI mismatch that
# only shows up as a crash on a device.
step "dex"
nsh_classes="$nsh_home/runtime/android/release/classes.dex"
[[ -f $nsh_classes ]] || die "NSH's runtime is missing $nsh_classes (nsh component add android)"
rm -rf "$work/dex" && mkdir -p "$work/dex"
"$java_bin" -cp "$bt/lib/d8.jar" com.android.tools.r8.D8 --release --min-api "$ui_api" \
    --lib "$android_jar" --output "$work/dex" \
    "$nsh_classes" "$kotlin_stdlib" \
    $(find "$classes" -name '*.class')
[[ -f "$work/dex/classes.dex" ]] || die "d8 produced no classes.dex"
cp "$work/dex/classes.dex" "$stage/classes.dex"

# What the runtime reads at startup to learn which app it is hosting.
mkdir -p "$stage/assets/nsh"
jq -nc --arg id "$application_id" '{id: $id}' >"$stage/assets/nsh/app.json"

# --- 6. assemble, align, sign ----------------------------------------------

step "assemble"
unsigned="$work/unsigned.apk"
rm -f "$unsigned"
# Start from the linked APK so the compiled resources and resources.arsc are the
# ones aapt2 produced, then add what aapt2 does not know about.
cp "$work/base.apk" "$unsigned"
chmod u+w "$unsigned"
(cd "$stage" && zip -q "$unsigned" classes.dex assets/nsh/app.json)
# Stored, not deflated, and aligned to 4096 by zipalign -p below: the manifest
# asks for extractNativeLibs because the loader has to resolve
# libkitty_singbox.so out of the archive, and Android refuses to install a
# native library that is compressed in an APK that claims otherwise.
(cd "$stage" && find lib -type f -print0 | xargs -0 zip -q -X -0 "$unsigned")

aligned="$work/aligned.apk"
"$bt/zipalign" -f -p 4 "$unsigned" "$aligned"

if [[ -n "${KITTY_KEYSTORE:-}" ]]; then
    keystore="$KITTY_KEYSTORE"
    # `apksigner` without `--ks-key-alias` takes the *first* key it finds. In a
    # release keystore that has more than one, that is a different key from the one
    # the last APK was signed with, and Android then refuses the upgrade with
    # INSTALL_FAILED_UPDATE_INCOMPATIBLE on every installed device. Silent, and it
    # would ship, because the build itself succeeds — so a release keystore names
    # its key or the build stops here.
    [[ -n "${KITTY_KEY_ALIAS:-}" ]] ||
        die "KITTY_KEYSTORE is set without KITTY_KEY_ALIAS; refusing to guess which key signs a release"
else
    # The debug keystore in the home directory, which is where `nsh package`
    # keeps its own and where every other Android tool puts one. Minting a
    # second key here would build a perfectly good APK that Android refused to
    # install over a build of the same app signed by NSH —
    # INSTALL_FAILED_UPDATE_INCOMPATIBLE, signatures do not match — which
    # matters because step 1 *is* a build of the same app signed by NSH, and
    # testing both back to back on one device is the normal thing to do. The
    # alias and both passwords are the debug defaults, the ones that key is
    # created with wherever it is created.
    keystore="${KITTY_DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}"
    [[ -f $keystore ]] || {
        mkdir -p "$(dirname "$keystore")"
        "$keytool" -genkeypair -keystore "$keystore" -storepass android \
            -keypass android -alias androiddebugkey -keyalg RSA -keysize 2048 \
            -validity 10000 -dname "CN=Android Debug,O=Android,C=US" >/dev/null 2>&1
    }
fi
# Named after the ABIs actually inside it, which is not cosmetics: the in-app
# update check has nothing else to go on. `proxy_core::update` reads the CPU
# groups out of the asset's file name and declines an asset whose groups do not
# include the device's, so a file called `Kitty-Pro-0.1.0.apk` would be offered
# to every Android device that ever asked, including one whose CPU this APK has
# no library for. Listing them is how the name tells the truth: an arm64 device
# sees `arm64` and `v8a`, an x86_64 emulator sees `x86_64`, and neither is
# offered the other's binary. One package rather than one per ABI, because the
# fat APK is what installs on either, and the matcher supports a name that
# declares several groups.
abi_name="$(IFS=-; echo "${abis[*]}")"
signed="$out/${app_name// /-}-$abi_name-$version.apk"
sign_args=(sign --ks "$keystore"
    --ks-pass "pass:${NSH_KEYSTORE_PASSWORD:-android}"
    --key-pass "pass:${KITTY_KEY_PASSWORD:-${NSH_KEYSTORE_PASSWORD:-android}}")
[[ -z "${KITTY_KEY_ALIAS:-}" ]] || sign_args+=(--ks-key-alias "$KITTY_KEY_ALIAS")
"$bt/apksigner" "${sign_args[@]}" --out "$signed" "$aligned"

step "verify"
"$bt/apksigner" verify "$signed"
for abi in "${abis[@]}"; do
    "$llvm/llvm-readelf" -d "$stage/lib/$abi/libnsh.so" |
        grep NEEDED |
        sed 's/^.*\[/'" $abi"' libnsh.so needs [/'
done

printf '\n\033[32m%s\033[0m (%s)\n' "$signed" "$(du -h "$signed" | cut -f1)"
