//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

pub(crate) const LATENCY_CACHE_KEY: &str = "kitty-pro.node-latency.v1";

pub(crate) type LatencyCacheEntry = (String, String, u64);

pub(crate) fn load_latency_cache_script() -> String {
    r#"
const key = __CACHE_KEY__;
try {
    const raw = localStorage.getItem(key);
    const cached = raw ? JSON.parse(raw) : null;
    if (cached && Array.isArray(cached.results)) {
        dioxus.send(cached.results);
    } else {
        localStorage.removeItem(key);
        dioxus.send([]);
    }
} catch (_) {
    localStorage.removeItem(key);
    dioxus.send([]);
}
"#
    .replace("__CACHE_KEY__", &format!("{LATENCY_CACHE_KEY:?}"))
}

pub(crate) fn save_latency_cache_script() -> String {
    r#"
const key = __CACHE_KEY__;
const results = await dioxus.recv();
try {
    if (Array.isArray(results) && results.length > 0) {
        localStorage.setItem(key, JSON.stringify({ savedAt: Date.now(), results }));
    } else {
        localStorage.removeItem(key);
    }
} catch (_) {}
"#
    .replace("__CACHE_KEY__", &format!("{LATENCY_CACHE_KEY:?}"))
}

pub(crate) fn restore_cached_latencies(
    nodes: &[ProxyNode],
    entries: Vec<LatencyCacheEntry>,
) -> HashMap<String, NodeLatency> {
    let cached_by_tag = entries
        .iter()
        .map(|(tag, endpoint, latency_ms)| ((tag.as_str(), endpoint.as_str()), *latency_ms))
        .collect::<HashMap<_, _>>();
    // Match on (tag, endpoint) only. The previous endpoint-only fallback
    // pasted a deleted subscription's number onto any new node sharing that
    // address - "已测速" values that belonged to a different provider.

    nodes
        .iter()
        .filter_map(|node| {
            let endpoint = node.endpoint();
            cached_by_tag
                .get(&(node.tag.as_str(), endpoint.as_str()))
                .map(|latency_ms| {
                    (
                        node.tag.clone(),
                        NodeLatency {
                            tag: node.tag.clone(),
                            latency_ms: Some(*latency_ms),
                            error: None,
                        },
                    )
                })
        })
        .collect()
}

pub(crate) fn latency_cache_entries(
    nodes: &[ProxyNode],
    results: &HashMap<String, NodeLatency>,
) -> Vec<LatencyCacheEntry> {
    nodes
        .iter()
        .filter_map(|node| {
            results
                .get(&node.tag)
                .and_then(|result| result.latency_ms)
                .map(|latency_ms| (node.tag.clone(), node.endpoint(), latency_ms))
        })
        .collect()
}

pub(crate) async fn persist_latency_cache(
    nodes: &[ProxyNode],
    results: &HashMap<String, NodeLatency>,
) {
    let eval = document::eval(&save_latency_cache_script());
    if eval.send(latency_cache_entries(nodes, results)).is_ok() {
        let _ = eval.await;
    }
}

pub(crate) fn format_latency(result: Option<&NodeLatency>) -> (String, &'static str, String) {
    match result {
        Some(NodeLatency {
            latency_ms: Some(latency),
            ..
        }) => (
            format!("{latency} ms"),
            "latency success",
            "节点可用".to_string(),
        ),
        Some(NodeLatency {
            error: Some(error), ..
        }) => ("失败".to_string(), "latency error", error.clone()),
        _ => (
            "-- ms".to_string(),
            "latency pending",
            "尚未探测".to_string(),
        ),
    }
}

/// Latency meter shown next to the millisecond figure: the bar shrinks as the
/// probe gets slower, and stays empty until a node is actually measured.
pub(crate) fn latency_meter(result: Option<&NodeLatency>) -> (u8, &'static str) {
    match result.and_then(|latency| latency.latency_ms) {
        Some(ms) if ms < 80 => (100, "fast"),
        Some(ms) if ms < 180 => (76, "fast"),
        Some(ms) if ms < 350 => (52, "medium"),
        Some(_) => (28, "slow"),
        None => (0, "empty"),
    }
}
