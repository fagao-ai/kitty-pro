//! B2: 把 api crate 里 32 处重复的 server function 条件编译样板收敛成一行。
//!
//! 原先每个 server fn 都要写：
//!
//! ```ignore
//! #[cfg_attr(
//!     all(
//!         not(any(target_os = "android", target_os = "ios")),
//!         any(target_arch = "wasm32", feature = "server")
//!     ),
//!     post("/api/profile")
//! )]
//! pub async fn save_profile(...) -> ... { ... }
//! ```
//!
//! 现在写 `#[kitty_server_fn(post, "/api/profile")]` 即可。宏展开出的
//! token 与原来**逐字相同**，因此 wasm32 / native / server feature 三种
//! 构建方式的行为完全不变；只是把「何时注册 HTTP 端点」这一决定集中到
//! 一个地方，避免以后新增端点时漏写或写错 cfg。

use proc_macro::TokenStream;

/// 生成 Dioxus server function 的 `#[cfg_attr(..., method(path))]` 包装。
///
/// 用法：`#[kitty_server_fn(get, "/api/profile")]` 或
/// `#[kitty_server_fn(post, "/api/core/restart")]`。
///
/// 参数只有一个 HTTP 方法名与一个路径字面量；两者都做校验，写错时给出
/// 中文编译错误而不是生成一堆难以定位的宏展开报错。
#[proc_macro_attribute]
pub fn kitty_server_fn(attr: TokenStream, item: TokenStream) -> TokenStream {
    let spec = attr.to_string();
    let mut parts = spec.splitn(2, ',');
    let method = parts.next().unwrap_or("").trim();
    let path = parts.next().unwrap_or("").trim();

    let method_ok = matches!(method, "get" | "post" | "put" | "patch" | "delete");
    // 路径必须是 `/api/` 开头的字符串字面量，且不含会被宏注入的字符。
    let path_ok = path.len() > 7
        && path.starts_with("\"/api/")
        && path.ends_with('"')
        && !path[1..path.len() - 1].contains(['"', '\\', '\n', '\r']);

    if !method_ok || !path_ok {
        let message = format!(
            "kitty_server_fn 参数无效：应为 #[kitty_server_fn(get|post|put|patch|delete, \"/api/...\")]，得到 {spec:?}"
        );
        return format!("compile_error!({:?});\n{item}", message)
            .parse()
            .expect("compile_error! 输出必须是合法 token 流");
    }

    // 与原先 32 处 cfg_attr 完全等价。注意这里保留裸 `get`/`post` 名称：
    // 它们由调用点的 `use dioxus::prelude::*;` 引入，展开后作用域不变。
    let expanded = format!(
        "#[cfg_attr(\n    all(\n        not(any(target_os = \"android\", target_os = \"ios\")),\n        any(target_arch = \"wasm32\", feature = \"server\")\n    ),\n    {method}({path})\n)]\n{item}"
    );
    expanded
        .parse()
        .unwrap_or_else(|error| panic!("kitty_server_fn 展开失败: {error}"))
}

#[cfg(test)]
mod tests {
    // 宏本身在 proc-macro crate 里无法直接单元测试（TokenStream 只能在
    // 编译器进程里构造），所以这里只覆盖参数校验的纯函数部分。
    // 真正的展开行为由 api crate 的编译 + 32 个端点注册测试间接保证。

    fn valid_method(method: &str) -> bool {
        matches!(method, "get" | "post" | "put" | "patch" | "delete")
    }

    fn valid_path(path: &str) -> bool {
        path.len() > 7
            && path.starts_with("\"/api/")
            && path.ends_with('"')
            && !path[1..path.len() - 1].contains(['"', '\\', '\n', '\r'])
    }

    #[test]
    fn accepts_the_methods_and_paths_used_by_the_api() {
        for method in ["get", "post", "put", "patch", "delete"] {
            assert!(valid_method(method));
        }
        for path in [
            "\"/api/profile\"",
            "\"/api/core/restart\"",
            "\"/api/sync/pull/commit\"",
            "\"/api/connectivity/probe\"",
        ] {
            assert!(valid_path(path), "{path}");
        }
    }

    #[test]
    fn rejects_malformed_or_injectable_arguments() {
        assert!(!valid_method("options"));
        assert!(!valid_method("POST"));
        for path in [
            "\"/health\"",
            "\"/api/\"",
            "\"/api/a\\\"b\"",
            "\"/api/a\nb\"",
            "api/profile",
        ] {
            assert!(!valid_path(path), "{path:?} should be rejected");
        }
    }
}
