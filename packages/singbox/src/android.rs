//! Android `VpnService` bridge for the embedded sing-box core.

use crate::{ffi, CoreError, CoreState, CoreStatus, LogBatch, ProbeResult, TrafficStats};
use jni::{
    objects::{JClass, JObject, JString, JValue},
    sys::{jint, jstring},
    JNIEnv, JavaVM,
};
use std::{path::PathBuf, ptr};

const BRIDGE_CLASS: &str = "com.kitty.pro.KittyVpnBridge";
const DNS_BRIDGE_CLASS: &str = "com.kitty.pro.AndroidDnsBridge";

pub fn start(config: &str) -> Result<(), CoreError> {
    let config = prepare_android_config(config)?;
    let result = with_env(|env, activity| {
        let config = env.new_string(&config).map_err(jni_error)?;
        let bridge = bridge_class(env, &activity)?;
        env.call_static_method(
            &bridge,
            "start",
            "(Landroid/app/Activity;Ljava/lang/String;)I",
            &[
                JValue::Object(&activity),
                JValue::Object(&JObject::from(config)),
            ],
        )
        .and_then(|value| value.i())
        .map_err(jni_error)
    })?;

    match result {
        0 | 1 => Ok(()),
        _ => Err(CoreError::AndroidVpn(
            "无法请求 Android VPN 授权".to_string(),
        )),
    }
}

pub fn stop() -> Result<(), CoreError> {
    with_env(|env, activity| {
        let bridge = bridge_class(env, &activity)?;
        env.call_static_method(
            &bridge,
            "stop",
            "(Landroid/content/Context;)V",
            &[JValue::Object(&activity)],
        )
        .map_err(jni_error)
        .map(|_| ())
    })
}

pub fn status() -> Result<CoreStatus, CoreError> {
    let value = call_string("status")?;
    let version = Some(ffi::version());
    match value.as_str() {
        "running" => Ok(CoreStatus {
            state: CoreState::Running,
            version,
            platform_note: Some("Android VPN 已连接".to_string()),
        }),
        "authorization" => Ok(CoreStatus {
            state: CoreState::Stopped,
            version,
            platform_note: Some("请在系统对话框中允许 Kitty Pro 创建 VPN".to_string()),
        }),
        "starting" => Ok(CoreStatus {
            state: CoreState::Stopped,
            version,
            platform_note: Some("Android VPN 正在启动".to_string()),
        }),
        "stopped" => Ok(CoreStatus {
            state: CoreState::Stopped,
            version,
            platform_note: Some("Android VPN 未连接".to_string()),
        }),
        error => Ok(CoreStatus {
            state: CoreState::Stopped,
            version,
            platform_note: Some(error.to_string()),
        }),
    }
}

pub fn traffic() -> Result<TrafficStats, CoreError> {
    let payload = call_string("traffic")?;
    if payload.is_empty() {
        return Ok(TrafficStats::default());
    }
    serde_json::from_str(&payload).map_err(|error| CoreError::TrafficUnavailable(error.to_string()))
}

pub fn logs(cursor: u64) -> Result<LogBatch, CoreError> {
    let payload = ffi::android_logs(cursor)?;
    serde_json::from_str(&payload).map_err(|error| CoreError::LogsUnavailable(error.to_string()))
}

pub fn set_log_enabled(enabled: bool) -> Result<(), CoreError> {
    ffi::android_set_log_enabled(enabled);
    Ok(())
}

pub fn select_outbound(group: &str, outbound: &str) -> Result<(), CoreError> {
    ffi::android_select_outbound(group, outbound)
}

pub fn probe(
    config: &str,
    node_tags: &[String],
    probe_url: &str,
) -> Result<Vec<ProbeResult>, CoreError> {
    let config = prepare_android_config(config)?;
    let data_path = files_dir()?;
    let data_path = data_path
        .to_str()
        .ok_or_else(|| CoreError::AndroidVpn("应用数据目录不是有效 UTF-8".to_string()))?;
    let node_tags = serde_json::to_string(node_tags)?;
    let payload = ffi::android_probe(&config, &node_tags, probe_url, data_path)?;
    serde_json::from_str(&payload).map_err(|error| CoreError::AndroidVpn(error.to_string()))
}

pub fn files_dir() -> Result<PathBuf, CoreError> {
    with_env(|env, activity| {
        let directory = env
            .call_method(&activity, "getFilesDir", "()Ljava/io/File;", &[])
            .and_then(|value| value.l())
            .map_err(jni_error)?;
        let path = env
            .call_method(&directory, "getAbsolutePath", "()Ljava/lang/String;", &[])
            .and_then(|value| value.l())
            .map_err(jni_error)?;
        let path = JString::from(path);
        env.get_string(&path)
            .map(|value| PathBuf::from(String::from(value)))
            .map_err(jni_error)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_kitty_pro_KittyVpnService_nativeStart(
    mut env: JNIEnv,
    _service: JObject,
    config: JString,
    tun_fd: jint,
    data_path: JString,
) -> jstring {
    let result = (|| {
        let config: String = env.get_string(&config).map_err(jni_error)?.into();
        let data_path: String = env.get_string(&data_path).map_err(jni_error)?.into();
        ffi::android_start(&config, tun_fd, &data_path)
    })();
    jni_result_string(&mut env, result)
}

#[no_mangle]
pub extern "system" fn Java_com_kitty_pro_KittyVpnService_nativeStop(
    mut env: JNIEnv,
    _service: JObject,
) -> jstring {
    jni_result_string(&mut env, ffi::android_stop())
}

#[no_mangle]
pub extern "system" fn Java_com_kitty_pro_KittyVpnService_nativeTraffic(
    mut env: JNIEnv,
    _service: JObject,
) -> jstring {
    match ffi::android_traffic() {
        Ok(value) => env
            .new_string(value)
            .map(|value| value.into_raw())
            .unwrap_or(ptr::null_mut()),
        Err(error) => jni_result_string(&mut env, Err(error)),
    }
}

fn call_string(method: &str) -> Result<String, CoreError> {
    with_env(|env, activity| {
        let bridge = bridge_class(env, &activity)?;
        let value = env
            .call_static_method(&bridge, method, "()Ljava/lang/String;", &[])
            .and_then(|value| value.l())
            .map_err(jni_error)?;
        let value = JString::from(value);
        env.get_string(&value)
            .map(|value| value.into())
            .map_err(jni_error)
    })
}

fn bridge_class<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'local>,
) -> Result<JClass<'local>, CoreError> {
    load_class(env, activity, BRIDGE_CLASS)
}

fn load_class<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject<'local>,
    class_name: &str,
) -> Result<JClass<'local>, CoreError> {
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
        .and_then(|value| value.l())
        .map_err(jni_error)?;
    let class_name = env.new_string(class_name).map_err(jni_error)?;
    let class_name = JObject::from(class_name);
    let class = env
        .call_method(
            &loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&class_name)],
        )
        .and_then(|value| value.l())
        .map_err(jni_error)?;
    Ok(JClass::from(class))
}

fn prepare_android_config(config: &str) -> Result<String, CoreError> {
    let mut value: serde_json::Value = serde_json::from_str(config)?;
    let Some(servers) = value
        .get_mut("dns")
        .and_then(|dns| dns.get_mut("servers"))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(config.to_string());
    };
    if !servers
        .iter()
        .any(|server| server.get("type").and_then(|value| value.as_str()) == Some("local"))
    {
        return Ok(config.to_string());
    }

    let port = android_local_dns_port()?;
    for server in servers
        .iter_mut()
        .filter(|server| server.get("type").and_then(|value| value.as_str()) == Some("local"))
    {
        let Some(server) = server.as_object_mut() else {
            continue;
        };
        server.insert("type".to_string(), serde_json::json!("udp"));
        server.insert("server".to_string(), serde_json::json!("127.0.0.1"));
        server.insert("server_port".to_string(), serde_json::json!(port));
        // A loopback DNS transport already uses sing-box's direct dialer.
        // sing-box 1.13 rejects detouring it through an empty direct outbound.
        server.remove("detour");
        server.remove("prefer_go");
    }
    serde_json::to_string(&value).map_err(CoreError::from)
}

fn android_local_dns_port() -> Result<u16, CoreError> {
    with_env(|env, activity| {
        let dns_bridge = load_class(env, &activity, DNS_BRIDGE_CLASS)?;
        let port = env
            .call_static_method(
                &dns_bridge,
                "localDnsPort",
                "(Landroid/content/Context;)I",
                &[JValue::Object(&activity)],
            )
            .and_then(|value| value.i())
            .map_err(jni_error)?;
        if let Some(port) = u16::try_from(port).ok().filter(|port| *port > 0) {
            return Ok(port);
        }

        let error = env
            .call_static_method(&dns_bridge, "lastError", "()Ljava/lang/String;", &[])
            .and_then(|value| value.l())
            .map_err(jni_error)?;
        let error = if error.is_null() {
            "未知错误".to_string()
        } else {
            let error = JString::from(error);
            env.get_string(&error)
                .map(String::from)
                .map_err(jni_error)?
        };
        Err(CoreError::AndroidVpn(format!(
            "启动 Android 系统 DNS 适配器失败: {error}"
        )))
    })
}

fn with_env<T>(
    operation: impl for<'local> FnOnce(&mut JNIEnv<'local>, JObject<'local>) -> Result<T, CoreError>,
) -> Result<T, CoreError> {
    let context = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(jni_error)?;
    vm.attach_current_thread()
        .map_err(jni_error)
        .and_then(|mut env| {
            let activity = unsafe { JObject::from_raw(context.context().cast()) };
            operation(&mut env, activity)
        })
}

fn jni_result_string(env: &mut JNIEnv, result: Result<(), CoreError>) -> jstring {
    match result {
        Ok(()) => ptr::null_mut(),
        Err(error) => env
            .new_string(error.to_string())
            .map(|value| value.into_raw())
            .unwrap_or(ptr::null_mut()),
    }
}

fn jni_error(error: jni::errors::Error) -> CoreError {
    CoreError::AndroidVpn(error.to_string())
}
