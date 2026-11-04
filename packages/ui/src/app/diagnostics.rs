//! Part of the `app` module (split from the original monolithic `app.rs`).

use super::*;

/// Subtitle under the Kitty Pro wordmark: which sing-box build is embedded.
pub(crate) fn about_build_label(
    app_version: &str,
    core_version: Option<String>,
    platform: &str,
) -> String {
    let core = match core_version {
        Some(version) => format!("sing-box {version}"),
        None => "sing-box 未就绪".to_string(),
    };
    format!("v{app_version} · {core} · {platform}")
}

/// Plain-text diagnostic blob the user can paste into a bug report. Nothing in
/// here is invented: every line mirrors a state the UI already shows.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_diagnostic_report(
    core_state: String,
    core_version: Option<String>,
    platform: &str,
    tunnel_mode: TunnelMode,
    tun_enabled: bool,
    allow_lan: bool,
    node_count: usize,
    subscription_count: usize,
    rule_count: usize,
    dns_count: usize,
) -> String {
    format!(
        "Kitty Pro 诊断信息\n\
         界面平台: {platform}\n\
         内核状态: {state_label} ({core_state})\n\
         内核版本: {core_version}\n\
         分流模式: {mode}\n\
         TUN: {tun} · 局域网: {lan} ({endpoint})\n\
         节点: {node_count} · 订阅: {subscription_count}\n\
         自定义规则: {rule_count} · 专用 DNS: {dns_count}",
        state_label = core_state_label(&core_state),
        core_version = core_version.unwrap_or_else(|| "未检测到版本".to_string()),
        mode = mode_label(tunnel_mode),
        tun = if tun_enabled { "开启" } else { "关闭" },
        lan = if allow_lan { "开启" } else { "关闭" },
        endpoint = proxy_port_label(allow_lan),
    )
}

/// Script that writes the received string to the clipboard. Falls back to a
/// hidden textarea + execCommand so it still works on non-secure origins.
pub(crate) fn copy_to_clipboard_script() -> String {
    r#"
const text = await dioxus.recv();
try {
    await navigator.clipboard.writeText(text);
} catch (_) {
    const area = document.createElement('textarea');
    area.value = text;
    area.setAttribute('readonly', '');
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.appendChild(area);
    area.select();
    document.execCommand('copy');
    area.remove();
}
await dioxus.send(true);
"#
    .to_string()
}

pub(crate) async fn copy_to_clipboard(text: String) -> Result<(), String> {
    let eval = document::eval(&copy_to_clipboard_script());
    eval.send(text).map_err(|error| error.to_string())?;
    eval.await.map(|_| ()).map_err(|error| error.to_string())
}

/// Footer hint of the log empty state: which core is idle, and whether the
/// collector is waiting for a connection.
pub(crate) fn core_build_hint(core_version: Option<String>, connected: bool) -> String {
    let build = core_version.unwrap_or_else(|| "sing-box".to_string());
    if connected {
        format!("内核 {build} · 日志采集中")
    } else {
        format!("内核 {build} · 日志采集待启动")
    }
}
