#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}}"
ndk_root="${ANDROID_NDK_HOME:-/opt/homebrew/share/android-ndk}"
java_home="${JAVA_HOME:-/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home}"
build_tools="$sdk_root/build-tools/35.0.0"
output_dir="$repo_root/dist/android"
stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/kitty-pro-apk.XXXXXX")"
keystore="${ANDROID_DEBUG_KEYSTORE:-$HOME/.android/debug.keystore}"

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

if [[ ! -f "$keystore" ]]; then
    mkdir -p "$(dirname -- "$keystore")"
    keytool -genkeypair \
        -keystore "$keystore" \
        -storepass android \
        -keypass android \
        -alias androiddebugkey \
        -keyalg RSA \
        -keysize 2048 \
        -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US"
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
if ! rg -q 'namespace = "dev\.dioxus\.main"' "$gradle_build_file"; then
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

# Dioxus bundles the Rust JNI library, but not the extra shared object emitted
# by the Go c-shared bridge. Place it in the Gradle JNI source set before APK
# assembly so Android's loader can resolve libmain.so's dependency.
core_library="$(find "$repo_root/target/aarch64-linux-android" -type f -path '*/build/singbox-*/out/libkitty_singbox.so' -print | tail -n 1)"
if [[ -z "$core_library" ]]; then
    echo "The embedded sing-box Android library was not produced" >&2
    exit 1
fi
core_stage="$android_project/app/src/main/jniLibs/arm64-v8a"
mkdir -p "$core_stage"
cp "$core_library" "$core_stage/libkitty_singbox.so"
(
    cd "$android_project"
    ./gradlew --no-daemon :app:assembleDebug
)

source_apk="$android_project/app/build/outputs/apk/debug/app-debug.apk"
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
    --ks-key-alias androiddebugkey \
    --ks-pass pass:android \
    --key-pass pass:android \
    --out "$signed_apk" \
    "$unsigned_apk"
apksigner verify --verbose --print-certs "$signed_apk"

echo "APK: $signed_apk"
