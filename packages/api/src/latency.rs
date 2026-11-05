//! Node latency probing and session registry.
//!
//! Split out of `lib.rs`; the crate root re-exports everything through
//! `pub(crate) use ...::*`, so sibling modules keep using `use super::*`.

use super::*;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn run_native_blocking<T, F>(
    failure_message: &'static str,
    operation: F,
) -> Result<T, ServerFnError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ServerFnError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| ServerFnError::new(format!("{failure_message}: {error}")))?
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
pub(crate) async fn run_native_blocking<T, F>(
    _failure_message: &'static str,
    operation: F,
) -> Result<T, ServerFnError>
where
    F: FnOnce() -> Result<T, ServerFnError>,
{
    operation()
}

#[allow(dead_code)]
#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) async fn measure_native_latency(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
    if nodes.is_empty() {
        return Err(ServerFnError::new("没有可探测的节点"));
    }
    if nodes.len() > MAX_LATENCY_NODES {
        return Err(ServerFnError::new(format!(
            "单次最多探测 {MAX_LATENCY_NODES} 个节点"
        )));
    }

    let node_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let request = ConnectionRequest {
        nodes,
        selected_tag: node_tags.first().cloned().unwrap_or_default(),
        proxy_server_nameservers,
        mode: TunnelMode::Global,
        tun: false,
        allow_lan: false,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let probe_options = SingBoxOptions {
        log_level: "error".to_string(),
        ..SingBoxOptions::default()
    };
    let mut config = proxy_core::build_singbox_config(&request, &probe_options);
    config["inbounds"] = serde_json::json!([]);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    tokio::task::spawn_blocking(move || {
        singbox::SingBox::probe_config(&config, &node_tags, LATENCY_CHECK_URL)
            .map(|results| {
                results
                    .into_iter()
                    .map(|result| NodeLatency {
                        tag: result.tag,
                        latency_ms: result.latency_ms,
                        error: result.error,
                    })
                    .collect()
            })
            .map_err(|error| ServerFnError::new(error.to_string()))
    })
    .await
    .map_err(|error| ServerFnError::new(format!("节点探测任务失败: {error}")))?
}

#[allow(dead_code)]
#[cfg(target_os = "android")]
pub(crate) async fn measure_native_latency(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
    if nodes.is_empty() {
        return Err(ServerFnError::new("没有可探测的节点"));
    }
    if nodes.len() > MAX_LATENCY_NODES {
        return Err(ServerFnError::new(format!(
            "单次最多探测 {MAX_LATENCY_NODES} 个节点"
        )));
    }

    let node_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let request = ConnectionRequest {
        nodes,
        selected_tag: node_tags.first().cloned().unwrap_or_default(),
        proxy_server_nameservers,
        mode: TunnelMode::Global,
        tun: false,
        allow_lan: false,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let probe_options = SingBoxOptions {
        log_level: "error".to_string(),
        ..SingBoxOptions::default()
    };
    let mut config = proxy_core::build_singbox_config(&request, &probe_options);
    config["inbounds"] = serde_json::json!([]);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    let config = serde_json::to_string(&config)
        .map_err(|error| ServerFnError::new(format!("序列化探测配置失败: {error}")))?;
    tokio::task::spawn_blocking(move || {
        singbox::probe_outbounds(&config, &node_tags, LATENCY_CHECK_URL)
            .map(|results| {
                results
                    .into_iter()
                    .map(|result| NodeLatency {
                        tag: result.tag,
                        latency_ms: result.latency_ms,
                        error: result.error,
                    })
                    .collect()
            })
            .map_err(|error| ServerFnError::new(error.to_string()))
    })
    .await
    .map_err(|error| ServerFnError::new(format!("节点探测任务失败: {error}")))?
}

#[allow(dead_code)]
#[cfg(target_arch = "wasm32")]
pub(crate) async fn measure_native_latency(
    _nodes: Vec<ProxyNode>,
    _proxy_server_nameservers: Vec<String>,
) -> Result<Vec<NodeLatency>, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接运行节点延迟探测"))
}

#[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
pub(crate) async fn start_native_latency_session(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<u64, ServerFnError> {
    validate_latency_nodes(&nodes)?;
    let node_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let request = ConnectionRequest {
        nodes,
        selected_tag: node_tags.first().cloned().unwrap_or_default(),
        proxy_server_nameservers,
        mode: TunnelMode::Global,
        tun: false,
        allow_lan: false,
        custom_rules: Vec::new(),
        dns_overrides: Vec::new(),
        config_script: None,
        group_selections: Default::default(),
    };
    let probe_options = SingBoxOptions {
        log_level: "error".to_string(),
        ..SingBoxOptions::default()
    };
    let mut config = proxy_core::build_singbox_config(&request, &probe_options);
    config["inbounds"] = serde_json::json!([]);
    config["route"]["auto_detect_interface"] = serde_json::Value::Bool(false);
    let core = tokio::task::spawn_blocking(move || {
        let mut core = singbox::SingBox::new().map_err(|error| error.to_string())?;
        core.start_config(&config)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(core)
    })
    .await
    .map_err(|error| ServerFnError::new(format!("启动节点探测任务失败: {error}")))?
    .map_err(ServerFnError::new)?;

    let (session_id, session) = register_latency_session(node_tags.len())?;
    let core = std::sync::Arc::new(core);
    tokio::spawn(async move {
        // B10: a Drop guard sets `done` even if this task panics, so polling
        // can never hang on a session whose worker aborted early.
        let _finish_guard = FinishOnDrop(std::sync::Arc::clone(&session));
        let mut tasks = tokio::task::JoinSet::new();
        let concurrency =
            std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_LATENCY_PROBES));
        for (index, tag) in node_tags.into_iter().enumerate() {
            let core = std::sync::Arc::clone(&core);
            let concurrency = std::sync::Arc::clone(&concurrency);
            tasks.spawn(async move {
                if index > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        index as u64 * LATENCY_PROBE_STAGGER_MS,
                    ))
                    .await;
                }
                // B10: the semaphore is never closed, but handle its error
                // instead of panicking so a poisoned task cannot strand the
                // whole session.
                let Ok(_permit) = concurrency.acquire_owned().await else {
                    return NodeLatency {
                        tag: tag.clone(),
                        latency_ms: None,
                        error: Some("节点探测任务被取消".to_string()),
                    };
                };
                let fallback_tag = tag.clone();
                match tokio::task::spawn_blocking(move || {
                    core.probe_outbound(&tag, LATENCY_CHECK_URL)
                })
                .await
                {
                    Ok(Ok(result)) => NodeLatency {
                        tag: result.tag,
                        latency_ms: result.latency_ms,
                        error: result.error,
                    },
                    Ok(Err(error)) => NodeLatency {
                        tag: fallback_tag,
                        latency_ms: None,
                        error: Some(error.to_string()),
                    },
                    Err(error) => NodeLatency {
                        tag: fallback_tag,
                        latency_ms: None,
                        error: Some(format!("节点探测任务失败: {error}")),
                    },
                }
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok(result) = result {
                session.push(result);
            }
        }
        // `session.finish()` is guaranteed by `_finish_guard` on drop.
    });
    Ok(session_id)
}

#[cfg(target_os = "android")]
pub(crate) async fn start_native_latency_session(
    nodes: Vec<ProxyNode>,
    proxy_server_nameservers: Vec<String>,
) -> Result<u64, ServerFnError> {
    validate_latency_nodes(&nodes)?;
    let total = nodes.len();
    let fallback_tags = nodes
        .iter()
        .map(|node| node.tag.clone())
        .collect::<Vec<_>>();
    let (session_id, session) = register_latency_session(total)?;
    tokio::spawn(async move {
        for (node_batch, fallback_batch) in nodes
            .chunks(MAX_CONCURRENT_LATENCY_PROBES)
            .zip(fallback_tags.chunks(MAX_CONCURRENT_LATENCY_PROBES))
        {
            let results =
                measure_native_latency(node_batch.to_vec(), proxy_server_nameservers.clone())
                    .await
                    .unwrap_or_else(|error| {
                        fallback_batch
                            .iter()
                            .map(|tag| NodeLatency {
                                tag: tag.clone(),
                                latency_ms: None,
                                error: Some(error.to_string()),
                            })
                            .collect()
                    });
            for result in results {
                session.push(result);
            }
        }
        session.finish();
    });
    Ok(session_id)
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn start_native_latency_session(
    _nodes: Vec<ProxyNode>,
    _proxy_server_nameservers: Vec<String>,
) -> Result<u64, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接启动节点探测"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn validate_latency_nodes(nodes: &[ProxyNode]) -> Result<(), ServerFnError> {
    if nodes.is_empty() {
        return Err(ServerFnError::new("没有可探测的节点"));
    }
    if nodes.len() > MAX_LATENCY_NODES {
        return Err(ServerFnError::new(format!(
            "单次最多探测 {MAX_LATENCY_NODES} 个节点"
        )));
    }
    Ok(())
}

/// B10: cap concurrent probe sessions and reap abandoned ones.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAX_LATENCY_SESSIONS: usize = 4;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const LATENCY_SESSION_TTL: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct LatencySession {
    results: std::sync::Mutex<Vec<NodeLatency>>,
    completed: std::sync::atomic::AtomicUsize,
    total: usize,
    done: std::sync::atomic::AtomicBool,
    created_at: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl LatencySession {
    pub(crate) fn push(&self, result: NodeLatency) {
        if let Ok(mut results) = self.results.lock() {
            results.push(result);
        }
        self.completed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn finish(&self) {
        self.done.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// B10: guarantees `done` is set even if the background task panics, so the
/// frontend never polls a session that can never finish.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct FinishOnDrop(std::sync::Arc<LatencySession>);

#[cfg(not(target_arch = "wasm32"))]
impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.finish();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn register_latency_session(
    total: usize,
) -> Result<(u64, std::sync::Arc<LatencySession>), ServerFnError> {
    let session_id = random_session_id()?;
    let session = std::sync::Arc::new(LatencySession {
        results: std::sync::Mutex::new(Vec::new()),
        completed: std::sync::atomic::AtomicUsize::new(0),
        total,
        done: std::sync::atomic::AtomicBool::new(false),
        created_at: std::time::Instant::now(),
    });
    let mut sessions = latency_sessions()
        .lock()
        .map_err(|_| ServerFnError::new("节点探测会话锁已损坏"))?;
    // Reap expired sessions before enforcing the concurrency cap so an
    // abandoned session cannot permanently occupy a slot.
    let now = std::time::Instant::now();
    sessions.retain(|_, existing| {
        now.saturating_duration_since(existing.created_at) < LATENCY_SESSION_TTL
    });
    if sessions.len() >= MAX_LATENCY_SESSIONS {
        return Err(ServerFnError::new("节点探测任务过多，请稍后重试"));
    }
    sessions.insert(session_id, std::sync::Arc::clone(&session));
    Ok((session_id, session))
}

/// B10: session ids are random u64 values, not enumerable counters.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn random_session_id() -> Result<u64, ServerFnError> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes)
        .map_err(|error| ServerFnError::new(format!("生成节点探测会话 ID 失败: {error}")))?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn poll_native_latency_session(
    session_id: u64,
) -> Result<LatencyProbeSnapshot, ServerFnError> {
    let session = latency_sessions()
        .lock()
        .map_err(|_| ServerFnError::new("节点探测会话锁已损坏"))?
        .get(&session_id)
        .cloned()
        .ok_or_else(|| ServerFnError::new("节点探测会话不存在或已结束"))?;
    let results = {
        let mut results = session
            .results
            .lock()
            .map_err(|_| ServerFnError::new("节点探测结果锁已损坏"))?;
        std::mem::take(&mut *results)
    };
    let done = session.done.load(std::sync::atomic::Ordering::Acquire);
    let snapshot = LatencyProbeSnapshot {
        results,
        completed: session.completed.load(std::sync::atomic::Ordering::Relaxed),
        total: session.total,
        done,
    };
    if done {
        latency_sessions()
            .lock()
            .map_err(|_| ServerFnError::new("节点探测会话锁已损坏"))?
            .remove(&session_id);
    }
    Ok(snapshot)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn poll_native_latency_session(
    _session_id: u64,
) -> Result<LatencyProbeSnapshot, ServerFnError> {
    Err(ServerFnError::new("浏览器目标不能直接读取节点探测"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn latency_sessions(
) -> &'static std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<LatencySession>>> {
    static SESSIONS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<u64, std::sync::Arc<LatencySession>>>,
    > = std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}
