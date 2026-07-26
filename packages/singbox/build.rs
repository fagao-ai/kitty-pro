use std::env;
use std::path::PathBuf;
use std::process::Command;

const DEFAULT_PROXY: &str = "http://100.64.0.2:11080";

fn main() {
    println!("cargo:rerun-if-changed=bridge/go.mod");
    println!("cargo:rerun-if-changed=bridge/main.go");
    println!("cargo:rerun-if-changed=bridge/android_bridge.go");
    println!("cargo:rerun-if-env-changed=HTTP_PROXY");
    println!("cargo:rerun-if-env-changed=HTTPS_PROXY");
    println!("cargo:rerun-if-env-changed=ALL_PROXY");
    println!("cargo:rerun-if-env-changed=NO_PROXY");
    println!("cargo:rerun-if-env-changed=GOPROXY");
    println!("cargo:rerun-if-env-changed=SINGBOX_GO");

    if env::var_os("CARGO_FEATURE_EMBEDDED_CORE").is_none() {
        return;
    }
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("missing Cargo target OS");
    if target_os == "wasm" {
        return;
    }
    if !matches!(
        target_os.as_str(),
        "android" | "macos" | "linux" | "windows"
    ) {
        panic!("embedded sing-box is not supported for target OS: {target_os}");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));
    let bridge_dir = manifest_dir.join("bridge");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing output dir"));
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let archive = if target_os == "windows" && target_env == "gnu" {
        // The GNU linker used by x86_64-pc-windows-gnu does not discover
        // MSVC-style .lib files when Cargo links `static=kitty_singbox`.
        out_dir.join("libkitty_singbox.a")
    } else if target_os == "windows" {
        out_dir.join("kitty_singbox.lib")
    } else if target_os == "android" {
        out_dir.join("libkitty_singbox.so")
    } else {
        out_dir.join("libkitty_singbox.a")
    };
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("missing Cargo target arch");
    let go_arch = match target_arch.as_str() {
        "aarch64" => "arm64",
        "arm" => "arm",
        "x86_64" => "amd64",
        "x86" => "386",
        other => panic!("unsupported embedded sing-box architecture: {other}"),
    };
    let go_os = match target_os.as_str() {
        "android" => "android",
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        _ => unreachable!(),
    };

    let go = go_executable();
    let mut command = Command::new(&go);
    command
        .current_dir(&bridge_dir)
        .env("GOOS", go_os)
        .env("GOARCH", go_arch);

    if target_os == "android" {
        command
            .env("CGO_ENABLED", "1")
            .env("CC", android_clang(&target_arch));
        if target_arch == "arm" {
            command.env("GOARM", "7");
        }
    }

    // The workspace proxy is useful on the developer network, but it is not
    // reachable from public CI runners. Keep it opt-in for CI while retaining
    // the local default; `KITTY_USE_DEFAULT_PROXY=1` can force it anywhere.
    let use_default_proxy = env::var("KITTY_USE_DEFAULT_PROXY")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or_else(|_| env::var_os("CI").is_none());
    if use_default_proxy {
        for variable in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            if env::var_os(variable).is_none() {
                command.env(variable, DEFAULT_PROXY);
            }
        }
    }
    if env::var_os("GOPROXY").is_none() {
        command.env("GOPROXY", "https://proxy.golang.org,direct");
    }
    if target_os == "macos" && env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() {
        command.env("MACOSX_DEPLOYMENT_TARGET", "11.0");
    }

    let build_mode = if target_os == "android" {
        "c-shared"
    } else {
        "c-archive"
    };
    let status = command
        .args([
            "build",
            "-buildmode",
            build_mode,
            "-tags",
            "with_gvisor,with_quic,with_wireguard,with_utls,with_clash_api,badlinkname,tfogo_checklinkname0",
            "-ldflags",
            "-checklinkname=0 -X github.com/sagernet/sing-box/constant.Version=kitty-pro-embedded",
            "-o",
        ])
        .arg(&archive)
        .arg(".")
        .status()
        .expect("Go 1.24.x is required to build the embedded sing-box core");
    if !status.success() {
        panic!("failed to build the embedded sing-box core");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    if target_os == "android" {
        println!("cargo:rustc-link-lib=dylib=kitty_singbox");
    } else {
        println!("cargo:rustc-link-lib=static=kitty_singbox");
    }
    if target_os == "macos" {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=SystemConfiguration");
    }
    if target_os == "linux" {
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=resolv");
    }
    if target_os == "android" {
        println!("cargo:rustc-link-lib=android");
        println!("cargo:rustc-link-lib=log");
    }
}

fn android_clang(target_arch: &str) -> PathBuf {
    let target = match target_arch {
        "aarch64" => "aarch64-linux-android",
        "arm" => "armv7a-linux-androideabi",
        "x86_64" => "x86_64-linux-android",
        "x86" => "i686-linux-android",
        other => panic!("unsupported Android architecture: {other}"),
    };
    let target_key = target.replace('-', "_");
    for variable in [
        format!("CC_{target_key}"),
        format!(
            "CARGO_TARGET_{}_LINKER",
            target.to_ascii_uppercase().replace('-', "_")
        ),
    ] {
        if let Some(path) = env::var_os(&variable) {
            return PathBuf::from(path);
        }
    }

    let ndk_root = [
        "ANDROID_NDK_HOME",
        "ANDROID_NDK_ROOT",
        "NDK_HOME",
        "DX_ANDROID_NDK_HOME",
    ]
    .into_iter()
    .find_map(|variable| env::var_os(variable).map(PathBuf::from))
    .or_else(|| {
        #[cfg(target_os = "macos")]
        {
            Some(PathBuf::from("/opt/homebrew/share/android-ndk"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    })
    .expect("set ANDROID_NDK_HOME to build the embedded Android sing-box core");
    let host_tag = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "darwin-arm64"
        } else {
            "darwin-x86_64"
        }
    } else if cfg!(target_os = "linux") {
        "linux-x86_64"
    } else {
        panic!("set a target-specific C compiler when building Android from this host")
    };
    let api_level = env::var("ANDROID_NATIVE_API_LEVEL").unwrap_or_else(|_| "24".to_string());
    let compiler = ndk_root
        .join("toolchains")
        .join("llvm")
        .join("prebuilt")
        .join(host_tag)
        .join("bin")
        .join(format!("{target}{api_level}-clang"));
    if compiler.is_file() {
        return compiler;
    }

    #[cfg(target_os = "macos")]
    {
        let rosetta_compiler = ndk_root
            .join("toolchains")
            .join("llvm")
            .join("prebuilt")
            .join("darwin-x86_64")
            .join("bin")
            .join(format!("{target}{api_level}-clang"));
        if rosetta_compiler.is_file() {
            return rosetta_compiler;
        }
    }

    panic!("Android NDK compiler was not found: {}", compiler.display());
}

fn go_executable() -> PathBuf {
    if let Some(path) = env::var_os("SINGBOX_GO") {
        return PathBuf::from(path);
    }

    // sing-box 1.13 currently targets Go 1.24. Later Go releases can reject
    // its TLS linkname bridge, so use Homebrew's keg-only Go 1.24 when present.
    #[cfg(target_os = "macos")]
    {
        let homebrew_go = PathBuf::from("/opt/homebrew/opt/go@1.24/bin/go");
        if homebrew_go.is_file() {
            return homebrew_go;
        }
    }

    PathBuf::from("go")
}
