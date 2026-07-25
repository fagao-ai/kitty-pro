//! Rust lifecycle API for the statically linked sing-box core.
//!
//! The actual networking engine is written in Go by the sing-box project. The
//! `embedded-core` feature compiles a small C ABI bridge and links it into the
//! host application, so no `sing-box` executable is spawned or discovered at
//! runtime.

use proxy_core::{build_singbox_config, ConnectionRequest, SingBoxOptions};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(target_os = "android")]
pub mod android;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    Unavailable,
    Stopped,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreStatus {
    pub state: CoreState,
    pub version: Option<String>,
    pub platform_note: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficStats {
    pub upload_total: u64,
    pub download_total: u64,
    pub active_connections: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    pub embedded_core: bool,
    pub requires_mobile_vpn_bridge: bool,
    pub browser_control_only: bool,
}

impl PlatformCapabilities {
    pub fn current() -> Self {
        Self {
            embedded_core: cfg!(feature = "embedded-core"),
            requires_mobile_vpn_bridge: cfg!(any(target_os = "android", target_os = "ios")),
            browser_control_only: cfg!(target_arch = "wasm32"),
        }
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("当前构建未包含嵌入式 sing-box 内核")]
    EmbeddedCoreUnavailable,
    #[error("嵌入式 sing-box 配置无效: {0}")]
    InvalidConfig(String),
    #[error("嵌入式 sing-box 已经在运行")]
    AlreadyRunning,
    #[error("嵌入式 sing-box 未运行")]
    NotRunning,
    #[error("读取嵌入式 sing-box 流量失败: {0}")]
    TrafficUnavailable(String),
    #[error("配置序列化失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Android VPN 服务错误: {0}")]
    AndroidVpn(String),
}

#[derive(Debug)]
pub struct SingBox {
    #[cfg(feature = "embedded-core")]
    handle: Option<u64>,
    #[cfg(feature = "embedded-core")]
    version: String,
}

impl SingBox {
    pub fn new() -> Result<Self, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            return Ok(Self {
                handle: None,
                version: ffi::version(),
            });
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    /// Kept as the construction entry point for callers that previously used
    /// process discovery. It no longer checks `PATH` or `SING_BOX_PATH`.
    pub fn discover() -> Result<Self, CoreError> {
        Self::new()
    }

    pub fn version(&self) -> Result<String, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            return Ok(self.version.clone());
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    pub fn start(
        &mut self,
        request: &ConnectionRequest,
        options: &SingBoxOptions,
    ) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            if self.handle.is_some() {
                return Err(CoreError::AlreadyRunning);
            }
            let config = build_singbox_config(request, options);
            let content = serde_json::to_string(&config)?;
            let handle = ffi::start(&content)?;
            self.handle = Some(handle);
            return Ok(());
        }

        #[cfg(not(feature = "embedded-core"))]
        {
            let _ = (request, options);
            Err(CoreError::EmbeddedCoreUnavailable)
        }
    }

    pub fn stop(&mut self) -> Result<(), CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            let handle = self.handle.ok_or(CoreError::NotRunning)?;
            ffi::stop(handle)?;
            self.handle = None;
            return Ok(());
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    pub fn is_running(&self) -> Result<bool, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            return Ok(self.handle.is_some());
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }

    pub fn status(&self) -> CoreStatus {
        #[cfg(feature = "embedded-core")]
        {
            return CoreStatus {
                state: if self.handle.is_some() {
                    CoreState::Running
                } else {
                    CoreState::Stopped
                },
                version: Some(self.version.clone()),
                platform_note: None,
            };
        }

        #[cfg(not(feature = "embedded-core"))]
        unavailable_status()
    }

    pub fn traffic(&self) -> Result<TrafficStats, CoreError> {
        #[cfg(feature = "embedded-core")]
        {
            let handle = self.handle.ok_or(CoreError::NotRunning)?;
            let payload = ffi::traffic(handle)?;
            return serde_json::from_str(&payload)
                .map_err(|error| CoreError::TrafficUnavailable(error.to_string()));
        }

        #[cfg(not(feature = "embedded-core"))]
        Err(CoreError::EmbeddedCoreUnavailable)
    }
}

impl Drop for SingBox {
    fn drop(&mut self) {
        if self.is_running().unwrap_or(false) {
            let _ = self.stop();
        }
    }
}

pub fn unavailable_status() -> CoreStatus {
    let capabilities = PlatformCapabilities::current();
    let platform_note = if capabilities.browser_control_only {
        "浏览器不能加载原生网络内核，可连接远程 Kitty Pro 核心服务".to_string()
    } else if capabilities.requires_mobile_vpn_bridge {
        "移动端内核需要通过 iOS NetworkExtension 或 Android VpnService 注入 TUN".to_string()
    } else {
        "当前构建没有链接嵌入式 sing-box 内核".to_string()
    };
    CoreStatus {
        state: CoreState::Unavailable,
        version: None,
        platform_note: Some(platform_note),
    }
}

#[cfg(feature = "embedded-core")]
mod ffi {
    use super::CoreError;
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    unsafe extern "C" {
        fn kitty_singbox_start(config_content: *const c_char) -> u64;
        fn kitty_singbox_stop(handle: u64) -> i32;
        fn kitty_singbox_version() -> *mut c_char;
        fn kitty_singbox_last_error() -> *mut c_char;
        fn kitty_singbox_traffic(handle: u64) -> *mut c_char;
        fn kitty_singbox_free_string(value: *mut c_char);
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_start(
            config_content: *const c_char,
            tun_fd: std::os::raw::c_int,
            data_path: *const c_char,
        ) -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_stop() -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_traffic() -> *mut c_char;
        #[cfg(target_os = "android")]
        fn kitty_singbox_android_probe(
            config_content: *const c_char,
            node_tags_json: *const c_char,
            probe_url: *const c_char,
            data_path: *const c_char,
            result: *mut *mut c_char,
        ) -> *mut c_char;
    }

    pub fn start(config: &str) -> Result<u64, CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::InvalidConfig("配置中包含 NUL 字符".to_string()))?;
        let handle = unsafe { kitty_singbox_start(config.as_ptr()) };
        if handle == 0 {
            return Err(CoreError::InvalidConfig(last_error()));
        }
        Ok(handle)
    }

    pub fn stop(handle: u64) -> Result<(), CoreError> {
        if unsafe { kitty_singbox_stop(handle) } == 0 {
            return Err(CoreError::NotRunning);
        }
        Ok(())
    }

    pub fn version() -> String {
        take_string(unsafe { kitty_singbox_version() })
    }

    pub fn traffic(handle: u64) -> Result<String, CoreError> {
        let payload = take_string(unsafe { kitty_singbox_traffic(handle) });
        if payload.is_empty() {
            return Err(CoreError::TrafficUnavailable(last_error()));
        }
        Ok(payload)
    }

    #[cfg(target_os = "android")]
    pub fn android_start(config: &str, tun_fd: i32, data_path: &str) -> Result<(), CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::InvalidConfig("配置中包含 NUL 字符".to_string()))?;
        let data_path = CString::new(data_path)
            .map_err(|_| CoreError::AndroidVpn("应用数据目录包含 NUL 字符".to_string()))?;
        let error =
            unsafe { kitty_singbox_android_start(config.as_ptr(), tun_fd, data_path.as_ptr()) };
        take_optional_error(error)
    }

    #[cfg(target_os = "android")]
    pub fn android_stop() -> Result<(), CoreError> {
        take_optional_error(unsafe { kitty_singbox_android_stop() })
    }

    #[cfg(target_os = "android")]
    pub fn android_traffic() -> Result<String, CoreError> {
        let payload = take_string(unsafe { kitty_singbox_android_traffic() });
        if payload.is_empty() {
            return Err(CoreError::TrafficUnavailable(last_error()));
        }
        Ok(payload)
    }

    #[cfg(target_os = "android")]
    pub fn android_probe(
        config: &str,
        node_tags_json: &str,
        probe_url: &str,
        data_path: &str,
    ) -> Result<String, CoreError> {
        let config = CString::new(config)
            .map_err(|_| CoreError::InvalidConfig("配置中包含 NUL 字符".to_string()))?;
        let node_tags_json = CString::new(node_tags_json)
            .map_err(|_| CoreError::AndroidVpn("节点标识包含 NUL 字符".to_string()))?;
        let probe_url = CString::new(probe_url)
            .map_err(|_| CoreError::AndroidVpn("探测地址包含 NUL 字符".to_string()))?;
        let data_path = CString::new(data_path)
            .map_err(|_| CoreError::AndroidVpn("应用数据目录包含 NUL 字符".to_string()))?;
        let mut result = std::ptr::null_mut();
        let error = unsafe {
            kitty_singbox_android_probe(
                config.as_ptr(),
                node_tags_json.as_ptr(),
                probe_url.as_ptr(),
                data_path.as_ptr(),
                &mut result,
            )
        };
        take_optional_error(error)?;
        Ok(take_string(result))
    }

    fn last_error() -> String {
        let message = take_string(unsafe { kitty_singbox_last_error() });
        if message.is_empty() {
            "未知错误".to_string()
        } else {
            message
        }
    }

    fn take_string(value: *mut c_char) -> String {
        if value.is_null() {
            return String::new();
        }
        let result = unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned();
        unsafe { kitty_singbox_free_string(value) };
        result
    }

    #[cfg(target_os = "android")]
    fn take_optional_error(value: *mut c_char) -> Result<(), CoreError> {
        if value.is_null() {
            return Ok(());
        }
        Err(CoreError::AndroidVpn(take_string(value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_core_is_constructed_without_a_binary() {
        let core = SingBox::new().expect("embedded core should be linked");
        assert!(!core
            .version()
            .expect("version should be available")
            .is_empty());
        assert_eq!(core.status().state, CoreState::Stopped);
    }

    #[test]
    fn embedded_core_starts_without_an_external_executable() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::thread;
        use std::time::Duration;

        let nodes = proxy_core::parse_subscription(
            "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443#Edge",
        )
        .nodes;
        let request = ConnectionRequest {
            selected_tag: nodes[0].tag.clone(),
            nodes,
            mode: proxy_core::TunnelMode::Rule,
            tun: false,
        };
        let target_listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("local HTTP listener should bind");
        let target_port = target_listener
            .local_addr()
            .expect("local HTTP listener should expose its port")
            .port();
        let target = thread::spawn(move || {
            let (mut stream, _) = target_listener
                .accept()
                .expect("sing-box should connect to the local HTTP listener");
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("local HTTP listener should respond");
        });
        let mixed_port = available_loopback_port();
        let traffic_port = available_loopback_port();
        assert_ne!(mixed_port, traffic_port);
        let options = SingBoxOptions {
            mixed_port,
            listen: "127.0.0.1".to_string(),
            log_level: "error".to_string(),
            traffic_api_port: Some(traffic_port),
            traffic_api_secret: Some("test-traffic-secret".to_string()),
        };
        let mut core = SingBox::new().expect("embedded core should be linked");

        core.start(&request, &options)
            .expect("embedded core should start");
        assert!(core.is_running().expect("core state should be readable"));
        let mut client = TcpStream::connect(("127.0.0.1", mixed_port))
            .expect("mixed inbound should accept HTTP proxy requests");
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("client read timeout should be configured");
        client
            .write_all(
                format!(
                    "GET http://127.0.0.1:{target_port}/ HTTP/1.1\r\nHost: 127.0.0.1:{target_port}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .expect("request should be written to mixed inbound");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .expect("mixed inbound should return the local HTTP response");
        target.join().expect("local HTTP thread should finish");
        assert!(response.starts_with(b"HTTP/1.1 204"));

        thread::sleep(Duration::from_millis(100));
        let traffic = core.traffic().expect("traffic counters should be readable");
        assert!(traffic.upload_total > 0);
        assert!(traffic.download_total > 0);
        core.stop().expect("embedded core should stop");
    }

    fn available_loopback_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("temporary listener should bind")
            .local_addr()
            .expect("temporary listener should expose its port")
            .port()
    }
}
