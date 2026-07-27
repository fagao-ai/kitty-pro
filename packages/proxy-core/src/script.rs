use rquickjs::{CatchResultExt, Context, Runtime, Value as JsValue};
use serde_json::Value;
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
    reject_clash_config(&output)?;
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
