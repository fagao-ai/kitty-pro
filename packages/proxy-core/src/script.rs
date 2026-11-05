use rquickjs::{CatchResultExt, Context, Runtime, Value as JsValue};
use serde_json::{Map, Value};
use std::time::{Duration, Instant};
use thiserror::Error;

const MAX_SCRIPT_BYTES: usize = 256 * 1024;
const MAX_CONFIG_BYTES: usize = 10 * 1024 * 1024;
const MEMORY_LIMIT_BYTES: usize = 32 * 1024 * 1024;
const STACK_LIMIT_BYTES: usize = 512 * 1024;
const EXECUTION_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Error)]
pub enum ConfigScriptError {
    #[error("脚本内容为空")]
    Empty,
    #[error("脚本超过 {MAX_SCRIPT_BYTES} 字节限制")]
    SourceTooLarge,
    #[error("配置超过 {MAX_CONFIG_BYTES} 字节限制")]
    ConfigTooLarge,
    #[error("无法创建 JavaScript 运行时: {0}")]
    Runtime(String),
    #[error("JavaScript 执行失败: {0}")]
    Execution(String),
    #[error("main(config) 必须返回 JSON 对象")]
    InvalidResult,
    #[error("脚本输出超过 {MAX_CONFIG_BYTES} 字节限制")]
    ResultTooLarge,
    #[error("脚本输出不是有效 JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("检测到 Clash/Mihomo 专用配置字段，Kitty Pro 使用 sing-box，请改写: {0}")]
    ClashConfig(String),
    #[error("配置脚本不允许修改 {0}")]
    ForbiddenConfigKey(String),
}

/// Executes a config-only JavaScript transform in an isolated QuickJS runtime.
///
/// The script must declare `main(config)` and return a JSON object. No file,
/// network, process, module loader, or Node.js APIs are installed in the runtime.
pub fn apply_config_script(source: &str, config: Value) -> Result<Value, ConfigScriptError> {
    if source.trim().is_empty() {
        return Err(ConfigScriptError::Empty);
    }
    if source.len() > MAX_SCRIPT_BYTES {
        return Err(ConfigScriptError::SourceTooLarge);
    }

    let input = serde_json::to_string(&config)?;
    if input.len() > MAX_CONFIG_BYTES {
        return Err(ConfigScriptError::ConfigTooLarge);
    }

    let runtime = Runtime::new().map_err(|error| ConfigScriptError::Runtime(error.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
    runtime.set_max_stack_size(STACK_LIMIT_BYTES);
    let started_at = Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || {
        started_at.elapsed() >= EXECUTION_TIMEOUT
    })));
    let context =
        Context::full(&runtime).map_err(|error| ConfigScriptError::Runtime(error.to_string()))?;

    let output = context.with(|ctx| {
        let input = ctx
            .json_parse(input)
            .catch(&ctx)
            .map_err(|error| ConfigScriptError::Execution(error.to_string()))?;
        ctx.globals()
            .set("__kitty_config", input)
            .catch(&ctx)
            .map_err(|error| ConfigScriptError::Execution(error.to_string()))?;

        let program = format!(
            "\"use strict\";\n{source}\n\n\
             if (typeof main !== \"function\") {{\n\
               throw new TypeError(\"脚本必须定义 main(config) 函数\");\n\
             }}\n\
             globalThis.__kitty_result = main(globalThis.__kitty_config);"
        );
        ctx.eval::<(), _>(program)
            .catch(&ctx)
            .map_err(|error| ConfigScriptError::Execution(error.to_string()))?;

        let result: JsValue = ctx
            .globals()
            .get("__kitty_result")
            .catch(&ctx)
            .map_err(|error| ConfigScriptError::Execution(error.to_string()))?;
        if !result.is_object() || result.is_array() {
            return Err(ConfigScriptError::InvalidResult);
        }
        let json = ctx
            .json_stringify(result)
            .catch(&ctx)
            .map_err(|error| ConfigScriptError::Execution(error.to_string()))?
            .ok_or(ConfigScriptError::InvalidResult)?;
        json.to_string()
            .map_err(|error| ConfigScriptError::Execution(error.to_string()))
    })?;

    if output.len() > MAX_CONFIG_BYTES {
        return Err(ConfigScriptError::ResultTooLarge);
    }
    let output: Value = serde_json::from_str(&output)?;
    // 保留 Clash 字段黑名单作为额外防线；先跑它，保证 Clash 专用字段仍以
    // ClashConfig 变体报错，再对脚本输出做严格白名单校验。
    reject_clash_config(&output)?;
    // 应用把完整配置传给脚本、并把脚本输出当作完整配置使用，因此这里只针对
    // “相对基线发生变化的键”做白名单校验：未改动的 inbounds/outbounds/
    // experimental 不会出现在这个变更集里，不会误伤正常脚本。
    let delta = config_script_delta(&config, &output);
    reject_unknown_config_keys(&delta)?;
    Ok(output)
}

fn reject_clash_config(config: &Value) -> Result<(), ConfigScriptError> {
    let Some(config) = config.as_object() else {
        return Err(ConfigScriptError::InvalidResult);
    };
    let mut fields = ["proxy-groups", "proxy-providers", "rule-providers", "rules"]
        .into_iter()
        .filter(|field| config.contains_key(*field))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(dns) = config.get("dns").and_then(Value::as_object) {
        for field in ["fake-ip-filter", "nameserver-policy", "direct-nameserver"] {
            if dns.contains_key(field) {
                fields.push(format!("dns.{field}"));
            }
        }
    }
    if fields.is_empty() {
        Ok(())
    } else {
        Err(ConfigScriptError::ClashConfig(fields.join(", ")))
    }
}

/// 配置脚本只允许修改应用真正合并的键（与 `api::build_native_config` 对齐）。
/// 其余 inbounds / outbounds / experimental / route.rule_set / cache_file /
/// 未知顶层键一律拒绝，避免脚本凭空加 TUN、把 clash_api 绑到 0.0.0.0、
/// 插入远程规则集 URL（SSRF）或改写缓存文件路径。
const ALLOWED_SCRIPT_KEYS: &[&str] = &[
    "log.level",
    "log.disabled",
    "log.timestamp",
    "route.rules",
    "dns.rules",
    "dns.servers",
    "__kitty_context",
];

/// 计算脚本输出相对输入基线发生变化的键（递归），返回只包含变更项的 JSON
/// 对象。被删除的键用 Null 标记。这样白名单校验只聚焦脚本真正改动的部分。
fn config_script_delta(baseline: &Value, output: &Value) -> Value {
    fn diff(before: &Value, after: &Value) -> Value {
        match (before.as_object(), after.as_object()) {
            (Some(before_map), Some(after_map)) => {
                let mut delta = Map::new();
                for (key, after_value) in after_map {
                    match before_map.get(key) {
                        None => {
                            delta.insert(key.clone(), after_value.clone());
                        }
                        Some(before_value) if before_value != after_value => {
                            match (before_value, after_value) {
                                (Value::Object(_), Value::Object(_)) => {
                                    let nested = diff(before_value, after_value);
                                    if nested.as_object().is_some_and(|map| !map.is_empty()) {
                                        delta.insert(key.clone(), nested);
                                    }
                                }
                                _ => {
                                    delta.insert(key.clone(), after_value.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                for key in before_map.keys() {
                    if !after_map.contains_key(key) {
                        delta.insert(key.clone(), Value::Null);
                    }
                }
                Value::Object(delta)
            }
            _ => after.clone(),
        }
    }
    diff(baseline, output)
}

/// 校验配置对象（在 `apply_config_script` 中传入与基线对比后的变更集）只包含
/// 白名单键路径。任何越权修改都用中文「配置脚本不允许修改 {key}」拒绝。
fn reject_unknown_config_keys(config: &Value) -> Result<(), ConfigScriptError> {
    fn path_matches(path: &[String], allowed: &str) -> bool {
        let segments: Vec<&str> = allowed.split('.').collect();
        path.len() <= segments.len()
            && path
                .iter()
                .zip(segments)
                .all(|(left, right)| left.as_str() == right)
    }
    fn is_allowed(path: &[String]) -> bool {
        ALLOWED_SCRIPT_KEYS
            .iter()
            .any(|allowed| path_matches(path, allowed))
    }
    fn is_leaf(path: &[String]) -> bool {
        ALLOWED_SCRIPT_KEYS.iter().any(|allowed| {
            let segments: Vec<&str> = (*allowed).split('.').collect();
            path.len() == segments.len()
                && path
                    .iter()
                    .zip(segments)
                    .all(|(left, right)| left.as_str() == right)
        })
    }
    fn walk(value: &Value, path: &mut Vec<String>) -> Result<(), ConfigScriptError> {
        let Some(object) = value.as_object() else {
            return Ok(());
        };
        for (key, child) in object {
            path.push(key.clone());
            if !is_allowed(path) {
                return Err(ConfigScriptError::ForbiddenConfigKey(path.join(".")));
            }
            if is_leaf(path) {
                // 叶路径（如 route.rules / dns.servers 数组）整体放行，不再下钻。
            } else if child.is_object() {
                walk(child, path)?;
            } else {
                // 容器路径被改成非对象，属于越权的结构性修改。
                return Err(ConfigScriptError::ForbiddenConfigKey(path.join(".")));
            }
            path.pop();
        }
        Ok(())
    }
    let mut path = Vec::new();
    walk(config, &mut path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deterministic_test_support::Lcg;
    use serde_json::json;

    #[test]
    fn applies_object_and_array_transforms() {
        let output = apply_config_script(
            r#"
                const main = (config) => {
                    config.route.rules.unshift({
                        domain_suffix: ["example.com"],
                        action: "route",
                        outbound: "proxy",
                    });
                    return config;
                };
            "#,
            json!({"route": {"rules": []}}),
        )
        .expect("script should transform config");

        assert_eq!(
            output["route"]["rules"][0]["domain_suffix"][0],
            "example.com"
        );
    }

    #[test]
    fn rejects_missing_main() {
        let error = apply_config_script("const answer = 42;", json!({}))
            .expect_err("main should be required");
        assert!(error.to_string().contains("main(config)"));
    }

    #[test]
    fn rejects_non_object_result() {
        let error = apply_config_script("function main() { return null; }", json!({}))
            .expect_err("null should be rejected");
        assert!(matches!(error, ConfigScriptError::InvalidResult));
    }

    #[test]
    fn rejects_clash_specific_output() {
        let error = apply_config_script(
            "function main(config) { config['proxy-groups'] = []; return config; }",
            json!({}),
        )
        .expect_err("Clash output should not be silently ignored");
        assert!(matches!(error, ConfigScriptError::ClashConfig(_)));
    }

    #[test]
    fn interrupts_runaway_scripts() {
        let error = apply_config_script("function main() { while (true) {} }", json!({}))
            .expect_err("runaway script should time out");
        assert!(matches!(error, ConfigScriptError::Execution(_)));
    }

    #[test]
    fn script_can_change_only_route_rules() {
        let output = apply_config_script(
            "function main(config) { config.route.rules.push({action:'sniff'}); return config; }",
            json!({ "route": { "rules": [] }, "inbounds": [], "outbounds": [] }),
        )
        .expect("a route.rules-only change should pass");
        assert_eq!(output["route"]["rules"][0]["action"], "sniff");
        assert_eq!(output["inbounds"], json!([]));
    }

    #[test]
    fn script_may_only_modify_whitelisted_keys() {
        let output = apply_config_script(
            r#"
                const main = (config) => {
                    config.route.rules.push({
                        domain_suffix: ["example.com"],
                        action: "route",
                        outbound: "direct",
                    });
                    config.log.level = "debug";
                    config.dns.servers = ["1.1.1.1"];
                    return config;
                };
            "#,
            json!({
                "route": { "rules": [] },
                "log": { "level": "info", "timestamp": true },
                "dns": { "servers": [], "rules": [] }
            }),
        )
        .expect("whitelisted changes should pass");
        assert_eq!(
            output["route"]["rules"][0]["domain_suffix"][0],
            "example.com"
        );
        assert_eq!(output["log"]["level"], "debug");
        assert_eq!(output["dns"]["servers"][0], "1.1.1.1");
    }

    #[test]
    fn script_cannot_insert_remote_rule_set() {
        // 安全策略收紧：脚本曾可插入 remote rule_set 并保留，现在必须被拒绝。
        let error = apply_config_script(
            r#"
                const main = (config) => {
                    config.route.rule_set = config.route.rule_set || [];
                    config.route.rule_set.push({
                        type: "remote",
                        tag: "evil",
                        url: "http://169.254.169.254/latest/meta-data/"
                    });
                    return config;
                };
            "#,
            json!({ "route": { "rules": [], "rule_set": [] } }),
        )
        .expect_err("remote rule_set injection must be rejected");
        assert!(matches!(error, ConfigScriptError::ForbiddenConfigKey(_)));
        assert!(error
            .to_string()
            .contains("配置脚本不允许修改 route.rule_set"));
    }

    #[test]
    fn script_cannot_add_inbounds_or_experimental() {
        let inbounds = apply_config_script(
            "function main(config) { config.inbounds = [{type:'tun'}]; return config; }",
            json!({ "inbounds": [] }),
        )
        .expect_err("adding an inbound must be rejected");
        assert!(matches!(inbounds, ConfigScriptError::ForbiddenConfigKey(_)));

        let experimental = apply_config_script(
            "function main(config) { config.experimental = {clash_api:{external_controller:'0.0.0.0:9090', secret:'x'}}; return config; }",
            json!({}),
        )
        .expect_err("rewriting experimental must be rejected");
        assert!(matches!(
            experimental,
            ConfigScriptError::ForbiddenConfigKey(_)
        ));
    }

    #[test]
    fn script_cannot_rewrite_cache_file_path() {
        let error = apply_config_script(
            "function main(config) { config.experimental.cache_file.path = '/tmp/evil.db'; return config; }",
            json!({ "experimental": { "cache_file": { "enabled": true, "path": "/app-data/cache/sing-box.db" } } }),
        )
        .expect_err("rewriting cache_file must be rejected");
        assert!(matches!(error, ConfigScriptError::ForbiddenConfigKey(_)));
    }

    #[test]
    fn reject_unknown_config_keys_whitelist() {
        assert!(reject_unknown_config_keys(&json!({ "route": { "rules": [] } })).is_ok());
        assert!(reject_unknown_config_keys(&json!({ "log": { "level": "debug" } })).is_ok());
        for forbidden in [
            json!({ "inbounds": [] }),
            json!({ "outbounds": [] }),
            json!({ "experimental": {} }),
            json!({ "route": { "rule_set": [] } }),
            json!({ "dns": { "fake-ip-filter": [] } }),
            json!({ "ntp": {} }),
        ] {
            let error =
                reject_unknown_config_keys(&forbidden).expect_err("forbidden key must be rejected");
            assert!(error.to_string().contains("配置脚本不允许修改"));
        }
    }

    // ===== T3：确定性生成式测试（纯标准库 LCG，不引入 rand/proptest）=====

    /// 递归生成确定性的随机 JSON（对象/数组/标量/深层嵌套），带深度上限避免爆栈。
    fn random_json(lcg: &mut Lcg, depth: usize) -> Value {
        if depth >= 5 {
            return Value::Null;
        }
        match lcg.next() % 6 {
            0 => Value::Null,
            1 => json!((lcg.next() % 1000) as i64),
            2 => json!(lcg.text(8)),
            3 => Value::Bool(lcg.next().is_multiple_of(2)),
            4 => {
                let mut array = Vec::new();
                for _ in 0..(lcg.next() % 4) {
                    array.push(random_json(lcg, depth + 1));
                }
                Value::Array(array)
            }
            _ => {
                let mut object = Map::new();
                for _ in 0..(lcg.next() % 4) {
                    object.insert(format!("k{}", lcg.next() % 64), random_json(lcg, depth + 1));
                }
                Value::Object(object)
            }
        }
    }

    #[test]
    fn reject_unknown_config_keys_never_panics_on_generated_json() {
        const SEED: u64 = 0x4a53_4f4e_4445_4c54; // "JSONDELT"
        let mut lcg = Lcg::new(SEED);
        for iteration in 0..150 {
            let value = random_json(&mut lcg, 0);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                reject_unknown_config_keys(&value)
            }));
            assert!(
                outcome.is_ok(),
                "seed={SEED:#x} iteration={iteration}: reject_unknown_config_keys panicked"
            );
            // Ok / Err 都可接受：随机键大多不在白名单，本用例只验证不 panic。
        }
    }

    #[test]
    fn generated_forbidden_keys_and_unknown_keys_are_rejected() {
        const SEED: u64 = 0x464f_5242_4944_4445; // "FORBIDDE"
        let mut lcg = Lcg::new(SEED);
        // 表驱动：S7 输出白名单必须拒绝的越权键路径。
        let forbidden_cases: Vec<Value> = vec![
            json!({ "inbounds": [] }),
            json!({ "outbounds": [] }),
            json!({ "experimental": { "clash_api": {} } }),
            json!({ "route": { "rule_set": [] } }),
            json!({ "ntp": {} }),
            json!({ "cache_file": { "path": "x" } }),
            json!({ "dns": { "fake-ip-filter": [] } }),
        ];
        for iteration in 0..150 {
            let mut delta = match random_json(&mut lcg, 0) {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            // 叠加表驱动禁用片段 + 随机未知顶层键 + 随机深层噪声。
            if let Value::Object(extra) = &forbidden_cases[iteration % forbidden_cases.len()] {
                for (key, child) in extra {
                    delta.insert(key.clone(), child.clone());
                }
            }
            delta.insert(
                format!("unknown_{}", lcg.next() % 1000),
                random_json(&mut lcg, 1),
            );
            let value = Value::Object(delta);
            let error = reject_unknown_config_keys(&value).expect_err(&format!(
                "seed={SEED:#x} iteration={iteration}: 含禁用键的增量必须被拒绝"
            ));
            assert!(
                error.to_string().contains("配置脚本不允许修改"),
                "seed={SEED:#x} iteration={iteration}: 错误应指出越权键, got {error}"
            );
        }
    }

    #[test]
    fn apply_config_script_generated_inputs_never_panic() {
        const SEED: u64 = 0x5155_4943_4b4a_5300; // "QUICKJS"
        let mut lcg = Lcg::new(SEED);
        let identity = "function main(config) { return config; }";
        for iteration in 0..100 {
            let config = random_json(&mut lcg, 0);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                apply_config_script(identity, config)
            }));
            assert!(
                outcome.is_ok(),
                "seed={SEED:#x} iteration={iteration}: apply_config_script panicked"
            );
            // 输出与输入相同 → delta 为空；含 Clash 字段返回 ClashConfig，
            // 非对象输入返回 InvalidResult，二者都不是 panic。
        }
    }

    #[test]
    fn apply_config_script_whitelist_passes_and_forbidden_outputs_rejected() {
        const SEED: u64 = 0x5343_5249_5054_574c; // "SCRIPTWL"
        let mut lcg = Lcg::new(SEED);
        // 只改 route.rules / log.level 的白名单用例必须通过。
        let whitelist_scripts: &[&str] = &[
            "function main(c) { c.route = c.route || {}; c.route.rules = c.route.rules || []; c.route.rules.push({action:'sniff'}); return c; }",
            "function main(c) { c.log = c.log || {}; c.log.level = 'debug'; return c; }",
        ];
        // 任何含 inbounds / outbounds / experimental / route.rule_set / 未知键的输出必须被拒绝。
        let forbidden_scripts: &[&str] = &[
            "function main(c) { c.inbounds = [{type:'tun'}]; return c; }",
            // 必须与基线里的 `"outbounds": []` 不同：若脚本把字段设成与基线
            // 完全相同的值，delta 为空、配置未发生任何变化，放行是正确的。
            "function main(c) { c.outbounds = [{type:'direct'}]; return c; }",
            "function main(c) { c.experimental = {clash_api:{}}; return c; }",
            "function main(c) { c.route = c.route || {}; c.route.rule_set = []; return c; }",
            "function main(c) { c.ntp = {}; return c; }",
        ];
        for iteration in 0..100 {
            let mut baseline = json!({
                "route": { "rules": [] },
                "log": { "level": "info" },
                "inbounds": [],
                "outbounds": []
            });
            // 随机噪声键加入基线，验证 delta 计算对无关键稳健。
            let noise = format!("noise_{}", lcg.next() % 1000);
            baseline
                .as_object_mut()
                .expect("baseline is an object")
                .insert(noise, Value::Null);
            if iteration % 3 == 0 {
                let script = whitelist_scripts[iteration % whitelist_scripts.len()];
                let output = apply_config_script(script, baseline);
                assert!(
                    output.is_ok(),
                    "seed={SEED:#x} iteration={iteration}: 只改白名单键必须通过: {output:?}"
                );
            } else {
                let script = forbidden_scripts[iteration % forbidden_scripts.len()];
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    apply_config_script(script, baseline)
                }));
                let Ok(result) = outcome else {
                    panic!("seed={SEED:#x} iteration={iteration}: apply_config_script panicked");
                };
                assert!(
                    result.is_err(),
                    "seed={SEED:#x} iteration={iteration}: 含禁用键的输出必须被拒绝"
                );
            }
        }
    }
}
