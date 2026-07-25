use std::env;
use std::path::PathBuf;
use std::process::Command;

const DEFAULT_PROXY: &str = "http://100.64.0.2:11080";

fn main() {
    println!("cargo:rerun-if-changed=bridge/go.mod");
    println!("cargo:rerun-if-changed=bridge/main.go");
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
    if !matches!(target_os.as_str(), "macos" | "linux" | "windows") {
        panic!(
            "embedded sing-box for {target_os} must be built through the mobile libbox pipeline"
        );
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));
    let bridge_dir = manifest_dir.join("bridge");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing output dir"));
    let archive = if target_os == "windows" {
        out_dir.join("kitty_singbox.lib")
    } else {
        out_dir.join("libkitty_singbox.a")
    };
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("missing Cargo target arch");
    let go_arch = match target_arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        "x86" => "386",
        other => panic!("unsupported embedded sing-box architecture: {other}"),
    };
    let go_os = match target_os.as_str() {
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

    // The bridge can download Go modules on a clean developer or CI machine.
    // Respect an explicitly configured proxy, otherwise use the workspace proxy.
    for variable in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        if env::var_os(variable).is_none() {
            command.env(variable, DEFAULT_PROXY);
        }
    }
    if env::var_os("GOPROXY").is_none() {
        command.env("GOPROXY", "https://proxy.golang.org,direct");
    }
    if target_os == "macos" && env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() {
        command.env("MACOSX_DEPLOYMENT_TARGET", "11.0");
    }

    let status = command
        .args([
            "build",
            "-buildmode=c-archive",
            "-tags",
            "with_gvisor,with_quic,with_wireguard,with_utls,with_clash_api,badlinkname,tfogo_checklinkname0",
            "-ldflags",
            "-X github.com/sagernet/sing-box/constant.Version=kitty-pro-embedded",
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
    println!("cargo:rustc-link-lib=static=kitty_singbox");
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
