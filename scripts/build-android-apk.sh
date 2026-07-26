#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}}"
ndk_root="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-${NDK_HOME:-}}}"
java_home="${JAVA_HOME:-}"
build_tools_version="${ANDROID_BUILD_TOOLS_VERSION:-35.0.0}"
build_tools="$sdk_root/build-tools/$build_tools_version"
output_dir="$repo_root/dist/android"
stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/kitty-pro-apk.XXXXXX")"
build_variant="${ANDROID_BUILD_VARIANT:-release}"

if [[ -z "$java_home" ]]; then
    apple_silicon_java_home="/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home"
    intel_java_home="/usr/local/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home"
    if [[ -d "$apple_silicon_java_home" ]]; then
        java_home="$apple_silicon_java_home"
    elif [[ -d "$intel_java_home" ]]; then
        java_home="$intel_java_home"
    elif [[ "$(uname -s)" == "Darwin" ]] &&
        detected_java_home="$(/usr/libexec/java_home -v 17 2>/dev/null)"; then
        java_home="$detected_java_home"
    elif command -v java >/dev/null 2>&1; then
        java_home="$(dirname -- "$(dirname -- "$(readlink -f "$(command -v java)")")")"
    fi
fi

if [[ -z "$ndk_root" && -d "$sdk_root/ndk" ]]; then
    ndk_root="$(find "$sdk_root/ndk" -mindepth 1 -maxdepth 1 -type d -print | sort | tail -n 1)"
fi
if [[ -z "$ndk_root" && -d /opt/homebrew/share/android-ndk ]]; then
    ndk_root="/opt/homebrew/share/android-ndk"
fi
if [[ ! -d "$build_tools" && -d "$sdk_root/build-tools" ]]; then
    build_tools="$(find "$sdk_root/build-tools" -mindepth 1 -maxdepth 1 -type d -print | sort | tail -n 1)"
fi

if [[ -n "${ANDROID_KEYSTORE_PATH:-}" ]]; then
    keystore="$ANDROID_KEYSTORE_PATH"
    keystore_alias="${ANDROID_KEY_ALIAS:?ANDROID_KEY_ALIAS is required with ANDROID_KEYSTORE_PATH}"
    keystore_password="${ANDROID_KEYSTORE_PASSWORD:?ANDROID_KEYSTORE_PASSWORD is required with ANDROID_KEYSTORE_PATH}"
    key_password="${ANDROID_KEY_PASSWORD:-$keystore_password}"
    generate_debug_keystore=false
else
    keystore="${ANDROID_DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}"
    keystore_alias="androiddebugkey"
    keystore_password="android"
    key_password="android"
    generate_debug_keystore=true
fi

cleanup() {
    rm -rf "$stage_dir"
}
trap cleanup EXIT

for tool in "$java_home/bin/java" "$build_tools/apksigner" "$build_tools/zipalign"; do
    if [[ ! -x "$tool" ]]; then
        echo "Missing Android build tool: $tool" >&2
        exit 1
    fi
done

if [[ ! -d "$ndk_root/toolchains/llvm" ]]; then
    echo "Missing Android NDK: $ndk_root" >&2
    exit 1
fi

export JAVA_HOME="$java_home"
export ANDROID_HOME="$sdk_root"
export ANDROID_SDK_ROOT="$sdk_root"
export ANDROID_NDK_HOME="$ndk_root"
export ANDROID_NDK_ROOT="$ndk_root"
export PATH="$java_home/bin:$sdk_root/platform-tools:$build_tools:$PATH"

if [[ "$generate_debug_keystore" == true && ! -f "$keystore" ]]; then
    mkdir -p "$(dirname -- "$keystore")"
    keytool -genkeypair \
        -keystore "$keystore" \
        -storepass "$keystore_password" \
        -keypass "$key_password" \
        -alias "$keystore_alias" \
        -keyalg RSA \
        -keysize 2048 \
        -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US"
fi
if [[ ! -f "$keystore" ]]; then
    echo "Android keystore does not exist: $keystore" >&2
    exit 1
fi

cd "$repo_root"
dx bundle \
    --package kitty-pro \
    --android \
    --package-types apk \
    --release \
    --out-dir "$stage_dir"

android_project="$repo_root/target/dx/kitty-pro/release/android/app"
if [[ ! -x "$android_project/gradlew" ]]; then
    echo "Dioxus did not produce an Android Gradle project" >&2
    exit 1
fi

# Dioxus' Android runtime classes and Rust JNI bindings live in
# dev.dioxus.main. The generated namespace follows the application identifier,
# which puts BuildConfig and R in the wrong package when the identifier differs.
# Keep the installable application ID below unchanged and align only Gradle's
# source namespace with the Dioxus runtime package.
gradle_build_file="$android_project/app/build.gradle.kts"
perl -0pi -e 's/namespace\s*=\s*"[^"]+"/namespace = "dev.dioxus.main"/' "$gradle_build_file"
if ! grep -q 'namespace = "dev\.dioxus\.main"' "$gradle_build_file"; then
    echo "Unable to configure the Dioxus Android namespace" >&2
    exit 1
fi

# Dioxus owns the WebView shell. Overlay the VPN service and manifest before
# asking Gradle to produce the actual application package.
android_source="$repo_root/packages/mobile/android"
cp "$android_source/AndroidManifest.xml" "$android_project/app/src/main/AndroidManifest.xml"
cp "$android_source/MainActivity.kt" "$android_project/app/src/main/kotlin/dev/dioxus/main/MainActivity.kt"
mkdir -p "$android_project/app/src/main/kotlin/com/kitty/pro"
cp "$android_source/KittyVpnBridge.kt" "$android_project/app/src/main/kotlin/com/kitty/pro/KittyVpnBridge.kt"
cp "$android_source/KittyVpnService.kt" "$android_project/app/src/main/kotlin/com/kitty/pro/KittyVpnService.kt"
cp -R "$android_source/res/." "$android_project/app/src/main/res/"

# Dioxus bundles the Rust JNI library, but not the extra shared object emitted
# by the Go c-shared bridge. Place it in the Gradle JNI source set before APK
# assembly so Android's loader can resolve libmain.so's dependency.
core_library="$(find "$repo_root/target" -type f \
    -path '*/build/singbox-*/out/libkitty_singbox.so' \
    -print 2>/dev/null | tail -n 1)"
if [[ -z "$core_library" ]]; then
    echo "The embedded sing-box Android library was not produced" >&2
    exit 1
fi
core_stage="$android_project/app/src/main/jniLibs/arm64-v8a"
mkdir -p "$core_stage"
cp "$core_library" "$core_stage/libkitty_singbox.so"
case "$build_variant" in
    debug)
        gradle_task=assembleDebug
        gradle_options=()
        source_apk="$android_project/app/build/outputs/apk/debug/app-debug.apk"
        ;;
    release)
        gradle_task=assembleRelease
        # Keep release signing and packaging deterministic on constrained CI
        # runners; code shrinking is optional for this distributable APK.
        gradle_options=(-x lintVitalRelease -x minifyReleaseWithR8)
        source_apk="$android_project/app/build/outputs/apk/release/app-release-unsigned.apk"
        ;;
    *)
        echo "Unsupported Android build variant: $build_variant" >&2
        exit 1
        ;;
esac
(
    cd "$android_project"
    ./gradlew --no-daemon ":app:$gradle_task" "${gradle_options[@]}"
)

if [[ ! -f "$source_apk" ]]; then
    echo "Gradle did not produce an Android APK" >&2
    exit 1
fi

mkdir -p "$output_dir"
unsigned_apk="$output_dir/Kitty-Pro-arm64-v8a-unsigned.apk"
signed_apk="$output_dir/Kitty-Pro-arm64-v8a.apk"

zipalign -f -p 4 "$source_apk" "$unsigned_apk"
apksigner sign \
    --ks "$keystore" \
    --ks-key-alias "$keystore_alias" \
    --ks-pass "pass:$keystore_password" \
    --key-pass "pass:$key_password" \
    --out "$signed_apk" \
    "$unsigned_apk"
apksigner verify --verbose --print-certs "$signed_apk"

echo "APK: $signed_apk"
