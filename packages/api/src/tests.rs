//! Unit tests for the api crate (moved out of `lib.rs`).

use super::*;

#[test]
fn redacts_credentials_from_error_strings() {
    // reqwest echoes the full request URL (token query + userinfo) into
    // transport errors; none of it may reach the toast.
    let raw =
        "builder error: https://user:pa***@sub.example.com:8443/get?token=***&flag=1 timed out";
    let red = redact_secrets(raw);
    assert!(!red.contains("topsecret"), "{red}");
    assert!(!red.contains("hunter2"), "{red}");
    assert!(!red.contains("flag=1"), "{red}");
    assert!(
        red.contains("[credential-redacted]@sub.example.com:8443"),
        "{red}"
    );
    assert!(red.contains("/get?…"), "{red}");
    assert!(red.ends_with("timed out"), "{red}");
}

#[test]
fn leaves_non_url_text_and_host_only_urls_untouched() {
    assert_eq!(
        redact_secrets("connection refused (os error 61)"),
        "connection refused (os error 61)"
    );
    // no credential, no query: the readable host stays.
    assert_eq!(
        redact_secrets("https://sub.example.com/link/abc"),
        "https://sub.example.com/link/abc"
    );
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn system_proxy_status_only_accepts_kitty_endpoint() {
    let mut proxy = sysproxy::Sysproxy {
        host: "127.0.0.1".to_string(),
        bypass: String::new(),
        port: SYSTEM_PROXY_PORT,
        enable: true,
    };
    assert!(is_kitty_system_proxy(&proxy));

    proxy.port += 1;
    assert!(!is_kitty_system_proxy(&proxy));
    proxy.port = SYSTEM_PROXY_PORT;
    proxy.enable = false;
    assert!(!is_kitty_system_proxy(&proxy));
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn copied_proxy_environment_targets_the_local_mixed_listener() {
    let variables = proxy_environment_variables();

    assert!(variables.contains("http://127.0.0.1:7890"));
    assert!(variables.contains("socks5://127.0.0.1:7890"));
    for name in ["http_proxy", "https_proxy", "all_proxy"] {
        assert!(variables.contains(name));
    }
    assert!(!variables.contains("no_proxy"));

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    assert_eq!(variables.matches(" && ").count(), 2);
    #[cfg(target_os = "windows")]
    assert_eq!(variables.matches("; ").count(), 2);
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn list_running_processes_excludes_self_and_sorts_by_name() {
    let processes = native_list_running_processes().expect("process list should succeed");
    // 当前测试进程自身应被过滤
    let self_pid = std::process::id();
    assert!(
        !processes.iter().any(|p| p.pid == self_pid),
        "self process should be excluded from the list"
    );
    // 至少能列出一些系统进程（本机必然有多个进程在跑）
    assert!(!processes.is_empty(), "expected at least some processes");

    let identities = processes
        .iter()
        .map(|process| (&process.name, &process.exe_path))
        .collect::<Vec<_>>();
    let mut sorted = identities.clone();
    sorted.sort();
    assert_eq!(identities, sorted, "processes should be sorted by identity");

    for p in &processes {
        assert!(!p.name.is_empty(), "name must not be empty");
        assert!(
            p.exe_path.as_deref().is_some_and(|path| !path.is_empty()),
            "exe_path must not be empty"
        );
        assert_eq!(
            p.exe_path
                .as_deref()
                .and_then(|path| std::path::Path::new(path).file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .as_deref(),
            Some(p.name.as_str()),
            "name must match the executable basename"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[test]
fn process_identity_keeps_distinct_paths_and_lowest_pid() {
    let mut processes = std::collections::BTreeMap::new();
    insert_running_process(&mut processes, 20, std::path::Path::new("/opt/one/tool"));
    insert_running_process(&mut processes, 10, std::path::Path::new("/opt/one/tool"));
    insert_running_process(&mut processes, 30, std::path::Path::new("/opt/two/tool"));

    let processes = processes.into_values().collect::<Vec<_>>();
    assert_eq!(processes.len(), 2);
    assert_eq!(processes[0].pid, 10);
    assert_eq!(processes[0].name, "tool");
    assert_eq!(processes[0].exe_path.as_deref(), Some("/opt/one/tool"));
    assert_eq!(processes[1].exe_path.as_deref(), Some("/opt/two/tool"));
}

#[cfg(not(target_arch = "wasm32"))]
struct TestDirectory(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl TestDirectory {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "kitty-pro-{label}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("test directory should be created");
        Self(path)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
struct FakeRestartCore {
    running: bool,
    fail_status: bool,
    fail_status_on_call: Option<usize>,
    status_calls: std::cell::Cell<usize>,
    fail_stop: bool,
    stop_error_after_shutdown: bool,
    fail_start_tags: std::collections::HashSet<String>,
    start_error_after_running_tags: std::collections::HashSet<String>,
    events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
impl FakeRestartCore {
    fn running(events: std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Self {
        Self {
            running: true,
            fail_status: false,
            fail_status_on_call: None,
            status_calls: std::cell::Cell::new(0),
            fail_stop: false,
            stop_error_after_shutdown: false,
            fail_start_tags: std::collections::HashSet::new(),
            start_error_after_running_tags: std::collections::HashSet::new(),
            events,
        }
    }

    fn push_event(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
impl RestartCoreEngine for FakeRestartCore {
    fn restart_is_running(&self) -> Result<bool, String> {
        self.push_event("status");
        let call = self.status_calls.get() + 1;
        self.status_calls.set(call);
        if self.fail_status || self.fail_status_on_call == Some(call) {
            Err("status failed".to_string())
        } else {
            Ok(self.running)
        }
    }

    fn restart_stop(&mut self) -> Result<(), String> {
        self.push_event("stop");
        if self.fail_stop {
            if self.stop_error_after_shutdown {
                self.running = false;
            }
            return Err("stop failed".to_string());
        }
        self.running = false;
        Ok(())
    }

    fn restart_start(&mut self, config: &serde_json::Value) -> Result<(), String> {
        let tag = config["tag"].as_str().unwrap_or("unknown");
        self.push_event(format!("start:{tag}"));
        if self.fail_start_tags.contains(tag) {
            return Err(format!("{tag} failed"));
        }
        self.running = true;
        if self.start_error_after_running_tags.contains(tag) {
            return Err(format!("{tag} response failed"));
        }
        Ok(())
    }

    fn restart_force_shutdown(&mut self) {
        self.push_event("force-shutdown");
        self.running = false;
    }
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
fn restart_fixture() -> (
    serde_json::Value,
    serde_json::Value,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    FakeRestartCore,
) {
    let candidate = serde_json::json!({ "tag": "candidate" });
    let previous = serde_json::json!({ "tag": "previous" });
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let core = FakeRestartCore::running(events.clone());
    (candidate, previous, events, core)
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn restart_checks_candidate_before_stopping_previous_core() {
    let (candidate, previous, events, mut core) = restart_fixture();
    let check_events = events.clone();

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), move |config| {
        check_events
            .lock()
            .unwrap()
            .push(format!("check:{}", config["tag"].as_str().unwrap()));
        Ok(())
    });

    assert_eq!(outcome, CoreRestartOutcome::CandidateRunning);
    assert!(core.running);
    assert_eq!(
        *events.lock().unwrap(),
        ["check:candidate", "status", "stop", "start:candidate"]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate.clone(), Some(previous)),
        Some(candidate)
    );
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn invalid_restart_candidate_leaves_previous_core_untouched() {
    let (candidate, previous, events, mut core) = restart_fixture();

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| {
        Err("invalid candidate".to_string())
    });

    assert!(matches!(
        outcome,
        CoreRestartOutcome::PreviousPreserved { .. }
    ));
    assert!(core.running);
    assert!(events.lock().unwrap().is_empty());
    assert_eq!(
        core_config_after_restart(&outcome, candidate, Some(previous.clone())),
        Some(previous)
    );
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn failed_candidate_restores_previous_config_without_proxy_disable() {
    let (candidate, previous, events, mut core) = restart_fixture();
    core.fail_start_tags.insert("candidate".to_string());

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert!(matches!(
        outcome,
        CoreRestartOutcome::PreviousRestored { .. }
    ));
    assert!(core.running);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "status",
            "stop",
            "start:candidate",
            "status",
            "start:previous"
        ]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate, Some(previous.clone())),
        Some(previous)
    );

    let restore_calls = std::cell::Cell::new(0);
    assert!(finish_core_restart(outcome, || {
        restore_calls.set(restore_calls.get() + 1);
        Ok(())
    })
    .is_err());
    assert_eq!(restore_calls.get(), 0);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn failed_candidate_and_rollback_marks_core_offline_and_disables_proxy() {
    let (candidate, previous, events, mut core) = restart_fixture();
    core.fail_start_tags = ["candidate".to_string(), "previous".to_string()]
        .into_iter()
        .collect();

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert!(matches!(outcome, CoreRestartOutcome::CoreOffline { .. }));
    assert!(!core.running);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "status",
            "stop",
            "start:candidate",
            "status",
            "start:previous",
            "status"
        ]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate, Some(previous)),
        None
    );

    let restore_calls = std::cell::Cell::new(0);
    assert!(finish_core_restart(outcome, || {
        restore_calls.set(restore_calls.get() + 1);
        Ok(())
    })
    .is_err());
    assert_eq!(restore_calls.get(), 1);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn failed_stop_preserves_previous_config_without_proxy_disable() {
    let (candidate, previous, events, mut core) = restart_fixture();
    core.fail_stop = true;

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert!(matches!(
        outcome,
        CoreRestartOutcome::PreviousPreserved { .. }
    ));
    assert!(core.running);
    assert_eq!(*events.lock().unwrap(), ["status", "stop", "status"]);
    assert_eq!(
        core_config_after_restart(&outcome, candidate, Some(previous.clone())),
        Some(previous)
    );

    let restore_calls = std::cell::Cell::new(0);
    assert!(finish_core_restart(outcome, || {
        restore_calls.set(restore_calls.get() + 1);
        Ok(())
    })
    .is_err());
    assert_eq!(restore_calls.get(), 0);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn failed_stop_after_shutdown_continues_with_candidate() {
    // A noisy stop error after the core is confirmed stopped must not
    // resurrect the previous config (e.g. turning TUN off bouncing back
    // to the TUN config); the candidate config is what the user asked for.
    let (candidate, previous, events, mut core) = restart_fixture();
    core.fail_stop = true;
    core.stop_error_after_shutdown = true;

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert!(matches!(outcome, CoreRestartOutcome::CandidateRunning));
    assert!(core.running);
    assert_eq!(
        *events.lock().unwrap(),
        ["status", "stop", "status", "start:candidate"]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate.clone(), Some(previous.clone())),
        Some(candidate)
    );

    let restore_calls = std::cell::Cell::new(0);
    assert!(finish_core_restart(outcome, || {
        restore_calls.set(restore_calls.get() + 1);
        Ok(())
    })
    .is_ok());
    assert_eq!(restore_calls.get(), 0);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn failed_stop_with_unknown_state_forces_shutdown_and_disables_proxy() {
    let (candidate, previous, events, mut core) = restart_fixture();
    core.fail_stop = true;
    core.fail_status_on_call = Some(2);

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert!(matches!(outcome, CoreRestartOutcome::CoreOffline { .. }));
    assert!(!core.running);
    assert_eq!(
        *events.lock().unwrap(),
        ["status", "stop", "status", "force-shutdown"]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate, Some(previous)),
        None
    );

    let restore_calls = std::cell::Cell::new(0);
    assert!(finish_core_restart(outcome, || {
        restore_calls.set(restore_calls.get() + 1);
        Ok(())
    })
    .is_err());
    assert_eq!(restore_calls.get(), 1);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn candidate_start_error_after_running_commits_candidate() {
    let (candidate, previous, events, mut core) = restart_fixture();
    core.start_error_after_running_tags
        .insert("candidate".to_string());

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert_eq!(outcome, CoreRestartOutcome::CandidateRunning);
    assert!(core.running);
    assert_eq!(
        *events.lock().unwrap(),
        ["status", "stop", "start:candidate", "status"]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate.clone(), Some(previous)),
        Some(candidate)
    );
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn candidate_start_with_unknown_state_is_forced_offline() {
    let (candidate, previous, events, mut core) = restart_fixture();
    core.fail_start_tags.insert("candidate".to_string());
    core.fail_status_on_call = Some(2);

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert!(matches!(outcome, CoreRestartOutcome::CoreOffline { .. }));
    assert!(!core.running);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "status",
            "stop",
            "start:candidate",
            "status",
            "force-shutdown"
        ]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate, Some(previous)),
        None
    );

    let restore_calls = std::cell::Cell::new(0);
    assert!(finish_core_restart(outcome, || {
        restore_calls.set(restore_calls.get() + 1);
        Ok(())
    })
    .is_err());
    assert_eq!(restore_calls.get(), 1);
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
#[test]
fn rollback_start_error_after_running_is_still_restored() {
    let (candidate, previous, events, mut core) = restart_fixture();
    core.fail_start_tags.insert("candidate".to_string());
    core.start_error_after_running_tags
        .insert("previous".to_string());

    let outcome = restart_core_transaction(&mut core, &candidate, Some(&previous), |_| Ok(()));

    assert!(matches!(
        outcome,
        CoreRestartOutcome::PreviousRestored { .. }
    ));
    assert!(core.running);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "status",
            "stop",
            "start:candidate",
            "status",
            "start:previous",
            "status"
        ]
    );
    assert_eq!(
        core_config_after_restart(&outcome, candidate, Some(previous.clone())),
        Some(previous)
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn concurrent_profile_saves_are_atomic_and_last_write_wins() {
    let directory = TestDirectory::new("profile-concurrency");
    let path = directory.0.join("profile.json");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut writers = Vec::new();

    for index in 0..8 {
        let path = path.clone();
        let barrier = barrier.clone();
        writers.push(std::thread::spawn(move || {
            let profile = AppProfile {
                selected_tag: format!("concurrent-{index}"),
                config_script: "x".repeat(32 * 1024 + index),
                ..AppProfile::default()
            };
            barrier.wait();
            save_native_profile_to_path(&profile, &path)
        }));
    }
    for writer in writers {
        writer
            .join()
            .expect("profile writer should not panic")
            .expect("concurrent profile save should succeed");
    }

    let concurrent: AppProfile = serde_json::from_slice(
        &std::fs::read(&path).expect("concurrent profile should be readable"),
    )
    .expect("concurrent profile should contain one complete JSON document");
    assert!(concurrent.selected_tag.starts_with("concurrent-"));

    let last = AppProfile {
        selected_tag: "last-write".to_string(),
        ..AppProfile::default()
    };
    save_native_profile_to_path(&last, &path).expect("last profile save should succeed");
    let saved: AppProfile =
        serde_json::from_slice(&std::fs::read(&path).expect("last profile should be readable"))
            .expect("last profile should be valid");
    assert_eq!(saved, last);

    let remaining = std::fs::read_dir(&directory.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(remaining, [std::ffi::OsString::from("profile.json")]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn singbox_cache_file_is_private_and_reused() {
    let directory = TestDirectory::new("singbox-cache");
    let path = directory.0.join("sing-box.db");

    prepare_private_cache_file(&path).expect("cache file should be created");
    std::fs::write(&path, b"persistent sing-box cache").expect("cache fixture should be written");
    prepare_private_cache_file(&path).expect("cache file should be reusable");

    assert_eq!(
        std::fs::read(&path).expect("cache fixture should be readable"),
        b"persistent sing-box cache"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(&path)
                .expect("cache metadata should be readable")
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn singbox_cache_file_rejects_symbolic_links() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("singbox-cache-symlink");
    let target = directory.0.join("target.db");
    let path = directory.0.join("sing-box.db");
    std::fs::write(&target, b"unrelated data").expect("target should be written");
    symlink(&target, &path).expect("cache symlink should be created");

    let error = prepare_private_cache_file(&path).expect_err("cache symlink should be rejected");

    assert!(error.to_string().contains("符号链接"));
    assert_eq!(
        std::fs::read(&target).expect("symlink target should remain readable"),
        b"unrelated data"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_listener_follows_allow_lan_setting() {
    assert_eq!(mixed_listen_address(false), "127.0.0.1");
    assert_eq!(mixed_listen_address(true), "0.0.0.0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_config_applies_javascript_transform() {
    let nodes = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@example.com:443#香港节点",
    )
    .nodes;
    let selected_tag = nodes[0].tag.clone();
    let request = ConnectionRequest {
        nodes,
        selected_tag,
        proxy_server_nameservers: Vec::new(),
        mode: TunnelMode::Rule,
        tun: false,
        allow_lan: false,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: Some(
            "function main(config) {\
                    config.log.level = Object.values(config.__kitty_context.node_names)[0];\
                    return config;\
                }"
            .to_string(),
        ),
        group_selections: Default::default(),
    };

    let config = build_native_config(&request, &SingBoxOptions::default())
        .expect("script should be applied before startup");

    assert_eq!(config["log"]["level"], "香港节点");
    assert!(config.get("__kitty_context").is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn remote_rule_set_downloads_use_system_dns() {
    // The download detour is applied to app-generated rule sets. Keep this
    // coverage without routing the config through a user script: scripts
    // may no longer inject `route.rule_set` (see the rejection test below).
    let mut config = serde_json::json!({
        "route": {
            "rule_set": [{
                "type": "remote",
                "tag": "app-rule-set",
                "format": "binary",
                "url": "https://rules.example.com/app.srs",
                "download_detour": "direct"
            }]
        },
        "outbounds": []
    });
    apply_remote_rule_set_download_detour(&mut config);

    let rule_set = config["route"]["rule_set"]
        .as_array()
        .and_then(|rule_sets| {
            rule_sets
                .iter()
                .find(|rule_set| rule_set["tag"] == "app-rule-set")
        })
        .expect("app rule set should remain in the config");
    assert_eq!(rule_set["download_detour"], "__kitty-rule-set-download");
    let download_outbound = config["outbounds"]
        .as_array()
        .and_then(|outbounds| {
            outbounds
                .iter()
                .find(|outbound| outbound["tag"] == "__kitty-rule-set-download")
        })
        .expect("rule-set download outbound should be generated");
    assert_eq!(download_outbound["type"], "direct");
    assert_eq!(download_outbound["domain_resolver"], "dns-local");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_config_rejects_script_injected_remote_rule_sets() {
    // S7: a config script runs inside QuickJS but its *output* used to be
    // merged wholesale, letting it add a remote `rule_set` whose URL the
    // privileged core would fetch. The output whitelist now rejects it.
    let nodes = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@example.com:443#Node",
    )
    .nodes;
    let request = ConnectionRequest {
        selected_tag: nodes[0].tag.clone(),
        nodes,
        proxy_server_nameservers: Vec::new(),
        mode: TunnelMode::Rule,
        tun: false,
        allow_lan: true,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: Some(
            "function main(config) {\
                    config.route.rule_set.push({\
                        type: 'remote',\
                        tag: 'script-rule-set',\
                        format: 'binary',\
                        url: 'https://rules.example.com/script.srs',\
                        download_detour: 'direct'\
                    });\
                    return config;\
                }"
            .to_string(),
        ),
        group_selections: Default::default(),
    };

    let error = build_native_config(&request, &SingBoxOptions::default())
        .expect_err("script-injected remote rule sets must be rejected");
    let message = error.to_string();
    assert!(
        message.contains("配置脚本不允许修改") || message.contains("route.rule_set"),
        "unexpected error: {message}"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn app_owned_cache_path_overrides_script_output() {
    let nodes = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@example.com:443#Node",
    )
    .nodes;
    let request = ConnectionRequest {
        selected_tag: nodes[0].tag.clone(),
        nodes,
        proxy_server_nameservers: Vec::new(),
        mode: TunnelMode::Global,
        tun: true,
        allow_lan: false,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let options = SingBoxOptions {
        cache_file: Some("/app-data/cache/sing-box.db".to_string()),
        ..SingBoxOptions::default()
    };
    let mut config = serde_json::json!({
        "experimental": {
            "cache_file": {
                "enabled": true,
                "path": "/tmp/untrusted.db",
                "store_fakeip": false,
            }
        }
    });

    apply_owned_cache_config(&mut config, &request, &options);

    assert_eq!(
        config["experimental"]["cache_file"],
        serde_json::json!({
            "enabled": true,
            "path": "/app-data/cache/sing-box.db",
            "store_fakeip": true,
        })
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn app_owned_cache_is_enabled_without_tun() {
    let nodes = proxy_core::parse_subscription(
        "vless://11111111-1111-1111-1111-111111111111@example.com:443#Node",
    )
    .nodes;
    let request = ConnectionRequest {
        selected_tag: nodes[0].tag.clone(),
        nodes,
        proxy_server_nameservers: Vec::new(),
        mode: TunnelMode::Rule,
        tun: false,
        allow_lan: true,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let options = SingBoxOptions {
        cache_file: Some("/app-data/cache/sing-box.db".to_string()),
        ..SingBoxOptions::default()
    };
    let mut config = serde_json::json!({
        "route": {
            "rule_set": [{
                "type": "remote",
                "tag": "script-rule-set",
                "format": "binary",
                "url": "https://rules.example.com/script.srs"
            }]
        }
    });

    apply_owned_cache_config(&mut config, &request, &options);

    assert_eq!(config["experimental"]["cache_file"]["enabled"], true);
    assert_eq!(config["experimental"]["cache_file"]["store_fakeip"], false);
    assert_eq!(config["route"]["rule_set"][0]["tag"], "script-rule-set");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn unfinished_rule_set_transaction_restores_both_old_files() {
    let directory = TestDirectory::new("rule-set-rollback");
    let geosite = directory.0.join("geosite-cn.srs");
    let geoip = directory.0.join("geoip-cn.srs");
    std::fs::write(&geosite, b"old geosite").expect("old geosite should be written");
    std::fs::write(&geoip, b"old geoip").expect("old geoip should be written");
    std::fs::rename(&geosite, rule_set_backup_path(&geosite))
        .expect("geosite backup should be staged");
    std::fs::rename(&geoip, rule_set_backup_path(&geoip)).expect("geoip backup should be staged");
    std::fs::write(&geosite, b"new geosite").expect("new geosite should be installed");
    std::fs::write(&geoip, b"new geoip").expect("new geoip should be installed");
    write_rule_set_transaction(
        &rule_set_transaction_path(&directory.0),
        &RuleSetInstallTransaction {
            geosite_existed: true,
            geoip_existed: true,
        },
    )
    .expect("transaction should be written");

    recover_rule_set_transaction(&geosite, &geoip)
        .expect("unfinished transaction should roll back");

    assert_eq!(std::fs::read(&geosite).unwrap(), b"old geosite");
    assert_eq!(std::fs::read(&geoip).unwrap(), b"old geoip");
    assert!(!rule_set_backup_path(&geosite).exists());
    assert!(!rule_set_backup_path(&geoip).exists());
    assert!(!rule_set_transaction_path(&directory.0).exists());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn committed_rule_set_transaction_keeps_both_new_files() {
    let directory = TestDirectory::new("rule-set-commit");
    let geosite = directory.0.join("geosite-cn.srs");
    let geoip = directory.0.join("geoip-cn.srs");
    std::fs::write(rule_set_backup_path(&geosite), b"old geosite")
        .expect("geosite backup should be written");
    std::fs::write(rule_set_backup_path(&geoip), b"old geoip")
        .expect("geoip backup should be written");
    std::fs::write(&geosite, b"new geosite").expect("new geosite should be installed");
    std::fs::write(&geoip, b"new geoip").expect("new geoip should be installed");
    write_rule_set_transaction(
        &rule_set_transaction_path(&directory.0),
        &RuleSetInstallTransaction {
            geosite_existed: true,
            geoip_existed: true,
        },
    )
    .expect("transaction should be written");
    write_rule_set_commit(&rule_set_commit_path(&directory.0))
        .expect("commit marker should be written");

    recover_rule_set_transaction(&geosite, &geoip)
        .expect("committed transaction should finish cleanup");

    assert_eq!(std::fs::read(&geosite).unwrap(), b"new geosite");
    assert_eq!(std::fs::read(&geoip).unwrap(), b"new geoip");
    assert!(!rule_set_backup_path(&geosite).exists());
    assert!(!rule_set_backup_path(&geoip).exists());
    assert!(!rule_set_transaction_path(&directory.0).exists());
    assert!(!rule_set_commit_path(&directory.0).exists());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn latency_session_returns_only_incremental_results() {
    let (session_id, session) = register_latency_session(2).expect("session should register");
    session.push(NodeLatency {
        tag: "fast".to_string(),
        latency_ms: Some(42),
        error: None,
    });

    let first = poll_native_latency_session(session_id).expect("first poll should succeed");
    assert_eq!(first.completed, 1);
    assert_eq!(first.results.len(), 1);
    assert!(!first.done);

    let second = poll_native_latency_session(session_id).expect("second poll should succeed");
    assert!(second.results.is_empty());
    assert_eq!(second.completed, 1);

    session.push(NodeLatency {
        tag: "slow".to_string(),
        latency_ms: None,
        error: Some("timeout".to_string()),
    });
    session.finish();
    let final_snapshot =
        poll_native_latency_session(session_id).expect("final poll should succeed");
    assert_eq!(final_snapshot.completed, 2);
    assert_eq!(final_snapshot.results.len(), 1);
    assert!(final_snapshot.done);
    assert!(poll_native_latency_session(session_id).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_subscription_preview_runs_in_process() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("native test runtime should start");
    let report = runtime
        .block_on(preview_subscription(
            "trojan://password@example.com:443#Direct".to_string(),
        ))
        .expect("native subscription preview should not require an HTTP backend");

    assert_eq!(report.nodes.len(), 1);
    assert_eq!(report.nodes[0].protocol, proxy_core::ProxyProtocol::Trojan);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_http_proxy_preview_is_not_downloaded_as_a_subscription() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("native test runtime should start");
    let report = runtime
        .block_on(preview_subscription(
            "http://proxy.example.com:8080#HTTP".to_string(),
        ))
        .expect("HTTP proxy URI should parse locally");

    assert_eq!(report.nodes.len(), 1);
    assert_eq!(report.nodes[0].protocol, proxy_core::ProxyProtocol::Http);
    assert_eq!(report.nodes[0].server, "proxy.example.com");
    assert_eq!(report.nodes[0].port, 8080);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_company_proxy_preview_creates_one_socks5_node() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("native test runtime should start");
    let report = runtime
        .block_on(preview_subscription(
            "socks5://198.51.100.20:11080#Company".to_string(),
        ))
        .expect("SOCKS5 proxy URI should parse locally");

    assert_eq!(report.nodes.len(), 1);
    let node = &report.nodes[0];
    assert_eq!(node.name, "Company");
    assert_eq!(node.protocol, proxy_core::ProxyProtocol::Socks5);
    assert_eq!(node.server, "198.51.100.20");
    assert_eq!(node.port, 11080);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_blocking_operations_run_off_the_calling_thread() {
    let calling_thread = std::thread::current().id();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("native test runtime should start");
    let operation_thread = runtime
        .block_on(run_native_blocking("test task failed", || {
            Ok::<_, ApiError>(std::thread::current().id())
        }))
        .expect("blocking operation should complete");

    assert_ne!(calling_thread, operation_thread);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore = "requires KITTY_TEST_SUBSCRIPTION_URL and live network access"]
fn live_native_latency_probe_returns_without_aborting() {
    let source = std::env::var("KITTY_TEST_SUBSCRIPTION_URL")
        .expect("set KITTY_TEST_SUBSCRIPTION_URL to a live subscription");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("native test runtime should start");
    let results = runtime.block_on(async move {
        let mut report = preview_subscription(source).await?;
        report
            .nodes
            .sort_by_key(|node| node.protocol != proxy_core::ProxyProtocol::AnyTls);
        report.nodes.truncate(8);
        if report.nodes.is_empty() {
            return Err(ApiError::new("订阅没有可探测的节点"));
        }
        measure_node_latency(report.nodes, report.proxy_server_nameservers).await
    });
    let results = results.expect("live node probe should return per-node results");

    assert!(!results.is_empty());
    assert!(results
        .iter()
        .all(|result| result.latency_ms.is_some() || result.error.is_some()));
    if std::env::var("KITTY_TEST_REQUIRE_SUCCESS").as_deref() == Ok("1") {
        assert!(
            results.iter().any(|result| result.latency_ms.is_some()),
            "live probe returned no successful nodes: {results:?}"
        );
    }
}

#[test]
fn parses_direct_domain_route_log() {
    let route = parse_route_log(
        "INFO[0001] [12 0ms] outbound/direct[direct]: outbound connection to www.baidu.com:443",
        Vec::new(),
        Some("127.0.0.1".to_string()),
    )
    .expect("direct route should be parsed");

    assert_eq!(route.decision, RouteDecision::Direct);
    assert_eq!(route.host, "www.baidu.com");
    assert_eq!(route.port, Some(443));
    assert_eq!(route.target_kind, RouteTargetKind::Domain);
    assert_eq!(route.outbound_tag, "direct");
    assert_eq!(route.source_ip.as_deref(), Some("127.0.0.1"));
}

#[test]
fn parses_proxy_ip_and_ipv6_route_logs() {
    let proxy = parse_route_log(
        "INFO[0002] outbound/vless[subscription-1-edge]: outbound packet connection to 8.8.8.8:53",
        vec![
            "AI节点".to_string(),
            "美国节点".to_string(),
            "subscription-1-edge".to_string(),
        ],
        Some("192.168.1.23".to_string()),
    )
    .expect("proxy route should be parsed");
    assert_eq!(proxy.decision, RouteDecision::Proxy);
    assert_eq!(proxy.host, "8.8.8.8");
    assert_eq!(proxy.port, Some(53));
    assert_eq!(proxy.target_kind, RouteTargetKind::Ip);
    assert_eq!(proxy.outbound_type, "vless");
    assert_eq!(proxy.outbound_chain[0], "AI节点");
    assert_eq!(proxy.source_ip.as_deref(), Some("192.168.1.23"));

    let ipv6 = parse_route_log(
        "INFO[0003] outbound/direct[direct]: outbound connection to [2001:db8::1]:443",
        Vec::new(),
        None,
    )
    .expect("IPv6 route should be parsed");
    assert_eq!(ipv6.host, "2001:db8::1");
    assert_eq!(ipv6.port, Some(443));
    assert_eq!(ipv6.target_kind, RouteTargetKind::Ip);
}

#[test]
fn parses_block_route_log() {
    let route = parse_route_log(
        "INFO[0004] outbound/block[block]: outbound connection to ads.example.com:443",
        Vec::new(),
        None,
    )
    .expect("block route should be parsed");

    assert_eq!(route.decision, RouteDecision::Block);
    assert_eq!(route.outbound_tag, "block");
}

#[test]
fn ignores_non_outbound_log_lines() {
    assert!(parse_route_log(
        "INFO[0001] inbound/mixed[mixed-in]: inbound connection to example.com:443",
        Vec::new(),
        None,
    )
    .is_none());
}

// ---------------------------------------------------------------------------
// B3 / B15 / B16：统一原子写与崩溃恢复
// ---------------------------------------------------------------------------

/// 列出目录顶层的普通文件名，用于断言没有残留临时/备份文件。
#[cfg(not(target_arch = "wasm32"))]
fn top_level_file_names(directory: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .expect("test directory should be readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn atomic_write_bytes_writes_exact_content() {
    let directory = TestDirectory::new("atomic-write-bytes");
    let path = directory.0.join("config.json");
    let payload = b"hello \xe4\xb8\xad\xe6\x96\x87 \x00 bytes";

    atomic_write_bytes(&path, payload).expect("atomic write should succeed");

    assert_eq!(
        std::fs::read(&path).expect("file should be readable"),
        payload
    );
    let mut names = top_level_file_names(&directory.0);
    names.sort();
    assert_eq!(names, vec!["config.json".to_string()], "不应留下临时文件");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn atomic_write_bytes_cleans_up_temporary_on_failure() {
    let directory = TestDirectory::new("atomic-write-failure");
    // 目标是一个非空目录，最终 rename 必然失败，用来验证失败路径的清理。
    let destination = directory.0.join("blocked.json");
    std::fs::create_dir(&destination).expect("directory should be created");
    std::fs::write(destination.join("keep"), b"x").expect("directory should be non-empty");

    let result = atomic_write_bytes(&destination, b"new content");
    assert!(result.is_err(), "覆盖一个目录必须失败");

    let names = top_level_file_names(&directory.0);
    assert!(
        !names
            .iter()
            .any(|name| name.contains("kitty-atomic") || name.ends_with(".tmp")),
        "失败后临时文件必须被清理: {names:?}"
    );
    assert!(destination.is_dir(), "目标目录应保持不变");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn atomic_replace_overwrites_existing_destination() {
    let directory = TestDirectory::new("atomic-replace-overwrite");
    let destination = directory.0.join("profile.json");
    let temporary = directory.0.join("profile.json.incoming");
    std::fs::write(&destination, b"old content").expect("old file should be written");
    std::fs::write(&temporary, b"new content").expect("temporary should be written");

    atomic_replace(&temporary, &destination).expect("safe overwrite should succeed");

    assert_eq!(
        std::fs::read(&destination).expect("destination should be readable"),
        b"new content"
    );
    assert!(!temporary.exists(), "临时文件应已被 rename 消费");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn atomic_replace_rolls_back_backup_on_rename_failure() {
    let directory = TestDirectory::new("atomic-replace-rollback");
    let destination = directory.0.join("profile.json");
    // 用一个不存在的临时文件制造 rename 失败，触发备份/回滚分支
    // （该分支即 Windows 的安全替换逻辑，这里在非 Windows 上直接覆盖）。
    let missing_temporary = directory.0.join("never-created.tmp");
    std::fs::write(&destination, b"old content").expect("old file should be written");

    let result = atomic_replace(&missing_temporary, &destination);
    assert!(result.is_err(), "rename 失败应返回错误");

    assert_eq!(
        std::fs::read(&destination).expect("destination should be restored"),
        b"old content",
        "回滚后旧内容必须完好"
    );
    assert!(!missing_temporary.exists(), "不应产生临时文件");
    let residue = top_level_file_names(&directory.0)
        .into_iter()
        .any(|name| name.contains("kitty-atomic"));
    assert!(!residue, "回滚后不应留下备份残留");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn atomic_write_json_round_trips() {
    let directory = TestDirectory::new("atomic-write-json");
    let path = directory.0.join("config.json");
    let value = serde_json::json!({"name": "kitty", "count": 3, "enabled": true});

    atomic_write_json(&path, &value).expect("atomic_write_json should succeed");

    let bytes = std::fs::read(&path).expect("config should be readable");
    assert!(bytes.contains(&b'\n'), "应保持 pretty JSON 输出");
    let decoded: serde_json::Value =
        serde_json::from_slice(&bytes).expect("config should be valid JSON");
    assert_eq!(decoded, value);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn recover_interrupted_writes_cleans_residue_and_keeps_history() {
    let directory = TestDirectory::new("recover-residue");
    let config = directory.0.join("config");
    let rules = directory.0.join("rules");
    let backups = config.join("profile-backups");
    std::fs::create_dir_all(&backups).expect("backup directory should be created");
    std::fs::create_dir_all(&rules).expect("rules directory should be created");

    std::fs::write(config.join("profile.json"), b"active profile").unwrap();
    std::fs::write(config.join("sync.json"), b"active sync").unwrap();
    // 本模块产生的残留：一个 .tmp 与一个 .backup。
    std::fs::write(config.join("profile.json.kitty-atomic-1-2-0.tmp"), b"junk").unwrap();
    std::fs::write(
        config.join("sync.json.kitty-atomic-1-2-1.backup"),
        b"old sync",
    )
    .unwrap();
    // MAX_PROFILE_BACKUPS 的历史备份：位于子目录且为 .json，必须保留。
    std::fs::write(backups.join("profile-1000.json"), b"history-1").unwrap();
    std::fs::write(backups.join("profile-2000.json"), b"history-2").unwrap();

    recover_interrupted_writes_in(&config, &rules)
        .expect("recovery on temp directories should succeed");

    let names = top_level_file_names(&config);
    assert!(
        !names.iter().any(|name| name.contains("kitty-atomic")),
        "残留临时/备份文件应被清理: {names:?}"
    );
    assert_eq!(
        std::fs::read(config.join("profile.json")).unwrap(),
        b"active profile"
    );
    assert_eq!(
        std::fs::read(config.join("sync.json")).unwrap(),
        b"active sync",
        "目标文件仍在，旧备份应被删除而非覆盖目标"
    );
    assert_eq!(
        std::fs::read(backups.join("profile-1000.json")).unwrap(),
        b"history-1"
    );
    assert_eq!(
        std::fs::read(backups.join("profile-2000.json")).unwrap(),
        b"history-2"
    );
}

// ---- B7: unified ApiError codes -------------------------------------------

#[test]
fn api_error_code_as_str_from_str_code_roundtrip() {
    let codes = [
        ApiErrorCode::InvalidInput,
        ApiErrorCode::NotFound,
        ApiErrorCode::Conflict,
        ApiErrorCode::Network,
        ApiErrorCode::Storage,
        ApiErrorCode::Privilege,
        ApiErrorCode::Core,
        ApiErrorCode::Sync,
        ApiErrorCode::Update,
        ApiErrorCode::Internal,
    ];
    for code in codes {
        let string = code.as_str();
        assert!(string.starts_with('E'), "{code:?} => {string}");
        assert_eq!(ApiErrorCode::from_str_code(string), Some(code));
        // The decoder strips an optional bracketed prefix too.
        assert_eq!(
            ApiErrorCode::from_str_code(&format!("[{string}]")),
            Some(code)
        );
    }
    assert_eq!(ApiErrorCode::from_str_code("E9999"), None);
    assert_eq!(ApiErrorCode::from_str_code(""), None);
}

#[test]
fn api_error_code_http_status_is_reasonable() {
    let codes = [
        ApiErrorCode::InvalidInput,
        ApiErrorCode::NotFound,
        ApiErrorCode::Conflict,
        ApiErrorCode::Network,
        ApiErrorCode::Storage,
        ApiErrorCode::Privilege,
        ApiErrorCode::Core,
        ApiErrorCode::Sync,
        ApiErrorCode::Update,
        ApiErrorCode::Internal,
    ];
    for code in codes {
        let status = code.http_status();
        assert!((400..=599).contains(&status), "{code:?} => {status}");
    }
    assert_eq!(ApiErrorCode::InvalidInput.http_status(), 400);
    assert_eq!(ApiErrorCode::NotFound.http_status(), 404);
    assert_eq!(ApiErrorCode::Conflict.http_status(), 409);
    assert_eq!(ApiErrorCode::Privilege.http_status(), 403);
    assert_eq!(ApiErrorCode::Network.http_status(), 502);
    assert_eq!(ApiErrorCode::Internal.http_status(), 500);
}

#[test]
fn api_err_code_is_recoverable_from_an_api_error() {
    let codes = [
        ApiErrorCode::InvalidInput,
        ApiErrorCode::NotFound,
        ApiErrorCode::Conflict,
        ApiErrorCode::Network,
        ApiErrorCode::Storage,
        ApiErrorCode::Privilege,
        ApiErrorCode::Core,
        ApiErrorCode::Sync,
        ApiErrorCode::Update,
        ApiErrorCode::Internal,
    ];
    for code in codes {
        let error = api_err(code, "测试消息");
        assert_eq!(error_code(&error), Some(code));
        assert!(error.to_string().contains(code.as_str()));
    }
    // A plain ApiError without a prefix carries no code.
    assert_eq!(error_code(&ApiError::new("普通错误")), None);
}

#[test]
fn api_err_redacts_secret_bearing_urls() {
    let error = api_err(
        ApiErrorCode::Network,
        "下载失败: https://user:pass@host/path?token=secret 请重试",
    );
    let rendered = error.to_string();
    assert!(!rendered.contains("secret"), "{rendered}");
    assert!(!rendered.contains("user:pass@"), "{rendered}");
    assert!(rendered.contains("[credential-redacted]"), "{rendered}");
    // The code must still be recoverable after redaction.
    assert_eq!(error_code(&error), Some(ApiErrorCode::Network));
}

#[test]
fn api_error_display_includes_code_prefix() {
    let error = ApiError {
        code: Some(ApiErrorCode::InvalidInput),
        message: "订阅数量超过 64 个限制".to_string(),
    };
    assert_eq!(format!("{error}"), "[E1001] 订阅数量超过 64 个限制");
}

#[test]
fn api_error_keeps_its_code_without_a_string_round_trip() {
    let error = ApiError {
        code: Some(ApiErrorCode::Conflict),
        message: "远程同步文件已被其他设备修改".to_string(),
    };
    assert_eq!(error_code(&error), Some(ApiErrorCode::Conflict));
    assert!(error.to_string().contains("远程同步文件已被其他设备修改"));
}

#[test]
fn an_unclassified_error_still_reads_a_code_embedded_in_its_message() {
    // Call sites that assemble a message which already carries the marker hand
    // it to `ApiError::new`; the code must still be readable, and `Display`
    // must not double it.
    let error = ApiError::new("[E1007] 内核未能启动");
    assert_eq!(error_code(&error), Some(ApiErrorCode::Core));
    assert_eq!(error.to_string(), "[E1007] 内核未能启动");
}

#[test]
fn a_bare_message_has_no_code_and_is_rendered_verbatim() {
    let error = ApiError::new("当前平台尚未实现系统代理适配");
    assert_eq!(error_code(&error), None);
    assert_eq!(error.to_string(), "当前平台尚未实现系统代理适配");
}
