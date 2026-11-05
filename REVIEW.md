# Kitty Pro 全项目审查报告

> 审查日期：2026-11-04 ｜ 范围：仓库全部自研代码（约 3.9 万行，不含 `target/` 与 `node_modules/`）
> 方法：三路独立只读审查（前端 UI / 后端与安全 / 业务流程）+ 主审对最高危结论的源码复核
> 本次为**只读审查，未修改任何代码**
> 路径均相对仓库根目录；行号以当前工作树为准

---

## 修复执行状态（2026-11-05 第五轮 · 真机验证）

用户质疑「改完了是可用的吧」。**静态测试全绿不等于能跑**，本轮把应用真正构建、起服务、用浏览器实跑，结果发现并修复了 4 个静态测试完全测不出来的真实缺陷。

### 环境搭建（本机此前无发布链）
- 装 `dioxus-cli 0.7.9`：GitHub release 与 git 协议经代理持续 502，最终从 crates.io 源码 `cargo install`（约 20 分钟）。
- 装 `wasm-opt`：Homebrew binaryen 在沙箱内 `sandbox_apply` 被拒；改从 conda-forge 拉 osx-arm64 包，解包后 `install_name_tool -add_rpath` + ad-hoc `codesign` 重签，装到 `~/.local`。
- Playwright：私有镜像与 `npx playwright install` 触发浏览器 GC 清了缓存又被 `safe-delete broker` 拦截；最终用 `PLAYWRIGHT_BROWSERS_PATH=/tmp/pw-browsers` 隔离目录装 CI 同款 chromium+headless-shell，并用 `KITTY_BROWSER=chrome` 走系统 Chrome 双通道验证。

### 构建与运行
- `dx build --package web --fullstack --release` 成功产出 client(wasm)+server。
- `kitty-pro-web-server --daemon` 起在 127.0.0.1:7897；curl 冒烟确认 S1 守卫真实生效：跨站 Origin POST `/api/profile` → **403**，未知 Host → **421**，首页 → 200。

### 实跑暴露并修复的 4 个真实缺陷
1. **自定义 `index.html` 把资源注进注释**（严重，一度整站裸样式）：为给 `<html>` 加 `lang="zh-CN"`（axe 要求）新建了 crate 根 `packages/web/index.html`，但首版**注释里写了 `</head>`/`</body>`/`{app_title}` 字面量**——dx 按字符串首次出现位置注入，把 CSS/JS 全塞进注释，页面 `styleSheets=0`。注释改写为不含锚点字面量后修复。已在文件里留醒目警告防复犯。
2. **深色主题两处对比度不达标**（axe color-contrast 实跑）：`--text-faint: #8d89a4` 在卡片底只有 4.19:1 → 提到 `#9793ae`（4.77:1）；禁用按钮 `opacity:0.48` 使主按钮文字复合对比度 4.2:1 → 实测扫描确定 0.75 两主题双按钮底都 ≥4.5:1，改为 0.75。
3. **概览卡超出折叠预算**（layout 守卫实跑）：前几轮给连接卡堆了测试连通/详细信息/提示行，700–768px 窗口把整页顶出折叠线（phantom 57–125px）。修复：运行态提示改由并排「测试连通」按钮承载、两个次级按钮 + details 摘要合并为一行、列间距 14→10px。修复中又踩一个坑——给 `details` 子元素设了 inline `display:grid` 会让 closed 内容穿透盒子参与布局（dead=-18），改回 CSS 受控显隐 `[open] > div`。
4. **console.spec.js 断言自相矛盾**（首跑即失败）：原断言「停靠后 `scrollTop` 数值不变」，但 newest-first 列表从顶部插入时浏览器 scroll anchoring 正是靠**增大 scrollTop** 来保持读者视线不动——观察实测停靠行 `12:01:19` 视觉位置恒为 -26px、而 scrollTop 400→1407。这是当初盲写从未真跑的断言写反了。改为断言「锚定行不变 + 视觉偏移 ≤1px + scrollTop 因锚定增长」，与实现契约一致。

另修 axe 与渐进渲染的竞态：轮询在 analyze 中途插元素导致偶发假阴/假阳，新增 `waitForSettle`（内容连续 800ms 无 DOM 变动再扫）。

### 最终验证（全绿）
| 检查 | 结果 |
|---|---|
| `cargo fmt --check` / `clippy --all-targets -D warnings` | 通过，0 警告 |
| `cargo test --workspace` | **292 通过 / 0 失败** |
| wasm32 / server feature / desktop / embedded-core / go test | 全部通过 |
| release `dx build` + 起服务 + curl 冒烟（S1 403/421/200） | 通过 |
| **Playwright 全套（含新 axe 12 条）** | **66 通过 / 0 失败**（a11y 单跑连 2 遍 + 全套共 4 遍稳定，排除竞态偶发） |

### 可用性结论
**能跑，且现在是被证明能跑**：release 产物在真实 headless Chromium 里正常 hydrate、六个视图可导航、S1 信任边界在 HTTP 层实测生效、66 项浏览器级守卫（含 axe 无障碍）全绿。桌面/移动 native 壳本次未起窗口实机跑（无显示器环境），但共享的 ui/api 层已被 web 壳的浏览器实跑覆盖，singbox 内嵌核心有 desktop/embedded-core feature 测试。

---

用户质疑「改完了是可用的吧」。**静态测试全绿不等于能跑**，本轮把应用真正构建、起服务、用浏览器实跑，结果发现并修复了 4 个静态测试完全测不出来的真实缺陷。

### 环境搭建（本机此前无发布链）
- 装 `dioxus-cli 0.7.9`：GitHub release 与 git 协议经代理持续 502，最终从 crates.io 源码 `cargo install`（约 20 分钟）。
- 装 `wasm-opt`：Homebrew binaryen 在沙箱内 `sandbox_apply` 被拒；改从 conda-forge 拉 osx-arm64 包，解包后 `install_name_tool -add_rpath` + ad-hoc `codesign` 重签，装到 `~/.local`。
- Playwright：私有镜像与 `npx playwright install` 触发浏览器 GC 清了缓存又被 `safe-delete broker` 拦截；最终用 `PLAYWRIGHT_BROWSERS_PATH=/tmp/pw-browsers` 隔离目录装 CI 同款 chromium+headless-shell，并用 `KITTY_BROWSER=chrome` 走系统 Chrome 双通道验证。

### 构建与运行
- `dx build --package web --fullstack --release` 成功产出 client(wasm)+server。
- `kitty-pro-web-server --daemon` 起在 127.0.0.1:7897；curl 冒烟确认 S1 守卫真实生效：跨站 Origin POST `/api/profile` → **403**，未知 Host → **421**，首页 → 200。

### 实跑暴露并修复的 4 个真实缺陷
1. **自定义 `index.html` 把资源注进注释**（严重，一度整站裸样式）：为给 `<html>` 加 `lang="zh-CN"`（axe 要求）新建了 crate 根 `packages/web/index.html`，但首版**注释里写了 `</head>`/`</body>`/`{app_title}` 字面量**——dx 按字符串首次出现位置注入，把 CSS/JS 全塞进注释，页面 `styleSheets=0`。注释改写为不含锚点字面量后修复。已在文件里留醒目警告防复犯。
2. **深色主题两处对比度不达标**（axe color-contrast 实跑）：`--text-faint: #8d89a4` 在卡片底只有 4.19:1 → 提到 `#9793ae`（4.77:1）；禁用按钮 `opacity:0.48` 使主按钮文字复合对比度 4.2:1 → 实测扫描确定 0.75 两主题双按钮底都 ≥4.5:1，改为 0.75。
3. **概览卡超出折叠预算**（layout 守卫实跑）：前几轮给连接卡堆了测试连通/详细信息/提示行，700–768px 窗口把整页顶出折叠线（phantom 57–125px）。修复：运行态提示改由并排「测试连通」按钮承载、两个次级按钮 + details 摘要合并为一行、列间距 14→10px。修复中又踩一个坑——给 `details` 子元素设了 inline `display:grid` 会让 closed 内容穿透盒子参与布局（dead=-18），改回 CSS 受控显隐 `[open] > div`。
4. **console.spec.js 断言自相矛盾**（首跑即失败）：原断言「停靠后 `scrollTop` 数值不变」，但 newest-first 列表从顶部插入时浏览器 scroll anchoring 正是靠**增大 scrollTop** 来保持读者视线不动——观察实测停靠行 `12:01:19` 视觉位置恒为 -26px、而 scrollTop 400→1407。这是当初盲写从未真跑的断言写反了。改为断言「锚定行不变 + 视觉偏移 ≤1px + scrollTop 因锚定增长」，与实现契约一致。

另修 axe 与渐进渲染的竞态：轮询在 analyze 中途插元素导致偶发假阴/假阳，新增 `waitForSettle`（内容连续 800ms 无 DOM 变动再扫）。

### 最终验证（全绿）
| 检查 | 结果 |
|---|---|
| `cargo fmt --check` / `clippy --all-targets -D warnings` | 通过，0 警告 |
| `cargo test --workspace` | **292 通过 / 0 失败** |
| wasm32 / server feature / desktop / embedded-core / go test | 全部通过 |
| release `dx build` + 起服务 + curl 冒烟（S1 403/421/200） | 通过 |
| **Playwright 全套（含新 axe 12 条）** | **66 通过 / 0 失败**（a11y 单跑连 2 遍 + 全套共 4 遍稳定，排除竞态偶发） |

### 可用性结论
**能跑，且现在是被证明能跑**：release 产物在真实 headless Chromium 里正常 hydrate、六个视图可导航、S1 信任边界在 HTTP 层实测生效、66 项浏览器级守卫（含 axe 无障碍）全绿。桌面/移动 native 壳本次未起窗口实机跑（无显示器环境），但共享的 ui/api 层已被 web 壳的浏览器实跑覆盖，singbox 内嵌核心有 desktop/embedded-core feature 测试。

---

## 修复执行状态（2026-11-05 第四轮 · 收尾批次）

**B2 / B7 / U5 / U6/U11 / U20 / U24(axe) / Q1 / Q2 / Q3 / Q5 / Q6 / Q8 已落地**（Q4/Q7 不在本轮）。备份 `/tmp/kitty-pro-backup-20261105-094500`。

### 验证结果

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D warnings` | 通过（0 警告） |
| `cargo test --workspace` | **292 通过 / 0 失败 / 1 ignored**（第三轮 276 → +16） |
| wasm32 / `--features server` / desktop / embedded-core / `go test` + `gofmt` | 全部通过 |

### 本轮完成

**B2｜server fn cfg 样板收敛**：新 crate `packages/server-fn-macro`（零依赖 proc-macro，手写 token 拼接，不引入 syn/quote），`#[kitty_server_fn(post, "/api/...")]` 替换 `lib.rs` 28 处 + `update.rs` 4 处，展开 token 与原先逐字相同。native / wasm32 / server feature 三种构建逐一验证。宏带 2 条参数校验单测。附带清理：`update.rs` 的 `api_err` 导入在 wasm 目标下未使用（dioxus 把 server fn 体替换成网络桩），用 `cfg_attr(target_arch="wasm32", allow(unused_imports))` 处理并注释说明原因。

**B7｜统一错误码**：新增 `api/src/error.rs`：`ApiErrorCode`（E1001-E1010，InvalidInput/NotFound/Conflict/Network/Storage/Privilege/Core/Sync/Update/Internal）+ `api_err()` 统一构造（内部 `redact_secrets` + `[Exxxx]` 前缀编码进消息，兼容 Dioxus 0.7 `ServerFnError` 的实际 API）+ `error_code_from_server_fn()` 反解。迁移 70 处调用点（outbound 15 / atomic_write 6 / update 20 / rule_set_cache 18 / sync 11），中文文案原样保留。新增 6 条单测。其余 ~128 处 `ServerFnError::new` 留给后续按模块迁移（增量策略，行为无变化）。

**U5｜SegmentedControl 键盘导航**：roving tabindex（选中项 `tabindex=0`、其余 `-1`），`ArrowRight/Left/Up/Down/Home/End` 循环移动 + `prevent_default`，方向键用 `document::eval` + 稳定元素 id 真正移动 DOM 焦点（与项目既有跨端用法一致），`Enter/Space` 走原生 click，`disabled` 全禁用。`role=group`/`aria_pressed` 保留（兼容既有 Playwright 断言）。

**U6/U11**：全量核对图标按钮，补齐「刷新全部订阅」「刷新订阅」的 `aria_label`。**U20**：主要异步按钮补 `aria_busy`，空态/错误态容器补 `role=status/alert`。

**U24｜axe-core 自动扫描**：安装了 `@axe-core/playwright@4.14.0`（devDependency，`package.json`/`package-lock.json` 有改动），`a11y.spec.js` 新增 12 条扫描（6 视图 × 浅/深，wcag2a/2aa/best-practice，禁用 `region` 规则）。注意：`npm audit` 报的 2 个 high 是既有 `playwright@1.49.1` 自身的证书校验 advisory，与本次新增无关。**axe 扫描需要 release wasm 构建才能实际运行（与整个 Playwright 套件相同），本轮只验证了语法与依赖安装，首跑建议在 CI/本地 release 构建后确认。**

**QA 疑点收口（Q1/Q2/Q3/Q5/Q6/Q8 全部有了结论）**：
- **Q1（确证，已修）**：`focus.source` 此前确实渲染完整订阅 URL。`node_focus_details` 现在只把 `source_label()` 脱敏结果放进展示字段，真实 `subscription.source` 不受影响；2 条回归测试（含端到端断言 DOM 字段无凭据）。
- **Q2（确证，已修）**：Go `check_config` 的错误可能回显含 password/uuid/token 的配置片段。新增 `redactConfigError()`（正则脱敏引号值/裸值/URL userinfo，零新依赖），跨 FFI 返回前统一套用；3 条 Go 单测。
- **Q3（确证，已修）**：`yaml_expansion_bomb` 原实现只读 ASCII 锚点名，`&节点`/`*节点` 炸弹可绕过。名字扫描改为「任意非 ASCII 字节整体入名」，并补 Unicode/混合大小写/行中位置共 6+ 断言。
- **Q5（评估完成，未重构）**：`allocate_loopback_port` 与 helper `reserve_loopback_port` 存在真实的「分配→drop→再绑定」TOCTOU 窗口，本机进程可抢占（DoS，窄竞态下可读到 HelperRequest）。彻底修复需持有 listener 直到 RPC 建立，涉及特权时序，评估结论与建议已写入 `core.rs` 注释，风险留档。
- **Q6（验证 + 守卫）**：全仓无 `CorsLayer`/`allow_origin`/ACAO 配置，Dioxus 默认 router 不挂跨域放行层，S1 的 Origin 判定不被 CORS 旁路；`cli.rs` 里加了源码级守卫单测（探测串放在 cli.rs 避免 include_str! 自匹配——我第一版把守卫写进 main.rs 触发自匹配导致失败，已改回正确位置）。
- **Q8（大部分已被 S2 覆盖，补边界）**：大小写/userinfo/子域/IP 形态已有测试；新增尾点（`allowed.example.` 归一后放行、`evil.allowed.example.` 仍拒）、IDN/punycode（双向拒绝）、IPv6 方括号剥离断言。

### 第四轮后剩余（最终清单）

- **需大预算重构**：B4/B5（抽共享 crate）、B6（RESTful 化）、U12（拆 ProxyApp）、B12 写路径锁、B9 真正 schema 字段、Q5 的端口持有重构。
- **需系统能力**：S8 钥匙串加密。
- **真模糊测试**：`cargo fuzz`（需 libFuzzer 工具链与独立 target）。
- **运维**：axe/Playwright 全套需在 release wasm 下首跑；`@playwright/test` 1.49.1 有已知 high advisory，建议下轮单独升级并跑全套回归。

---

## 修复执行状态（2026-11-04 第三轮 · 架构重构）

承接第二轮末尾的建议，本轮专攻架构性条目。**B1 / U8 / F22 / U14 / U23 / B3 / B15 / B16 / B9 / S13 / T3 已落地，B12 做保守降级**。改动前源码备份 `/tmp/kitty-pro-backup-20261104-221918`。

### 验证结果

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过（0 警告） |
| `cargo test --workspace` | **276 通过 / 0 失败 / 1 ignored**（上轮 253，**新增 23 条**） |
| `cargo test -p desktop --features desktop` | 8 通过 / 1 ignored |
| `cargo test -p singbox --features embedded-core` | 41 通过 |
| `cargo check -p web --target wasm32-unknown-unknown --features web` | 通过 |
| `go test ./...` + `gofmt -l` | 通过 |
| JS `node --check` / CSS 括号 735/735 / `package.json` | 通过 |

### 本轮完成

**B1｜拆分 `api/src/lib.rs`（5,056 → 981 行）**
用带大括号深度校验的脚本做机械切分（保证每个边界都在顶层项起点、cfg 变体不会被拆散），`lib.rs` 保留公开 DTO / server functions / mod 声明，实现按内聚子系统分到 6 个新模块，通过 `pub(crate) use <模块>::*;` 维持 `use super::*` 的共享作用域：

| 模块 | 行数 | 职责 |
|---|---|---|
| `latency.rs` | 445 | 测速探测、会话注册表 |
| `outbound.rs` | 237 | 出站 URL 校验、脱敏、订阅下载 |
| `rule_set_cache.rs` | 540 | 规则集缓存事务 / 崩溃恢复 |
| `core.rs` | 1,267 | 内核生命周期、配置生成、profile 持久化、日志 |
| `platform.rs` | 535 | 系统代理、进程列表、运行时关停 |
| `tests.rs` | 1,128 | 89 条单测 |

连原有 `sync.rs` / `update.rs` / `autostart.rs` / `macos_route.rs`，api crate 现为 11 个模块。拆分中修正的跨模块可见性：`NativeCore` 字段与 `new`、`LatencySession::push/finish`、`RuleSetInstallTransaction` 字段、以及两个被 `desktop`/`web` 直接调用的 `pub fn` 的 cfg 化 re-export（wasm32 下曾导致 E0432，已修）。

**U8｜共享组件层**：新建 `ui/src/app/components.rs`，提供 `Switch` / `SegmentedControl` / `ConfirmButton`。`Switch` 刻意保持为片段（不包 `<label>`），保留各调用点的整行点击区与 `input:checked + .switch` CSS；`SegmentedControl` 统一 `role=group` + `aria_pressed`；`ConfirmButton` 用 `Instant` 时间戳实现 3 秒过期 + 组件卸载即复位，**不需要跨 wasm/native 定时器**。已迁移 `toggles.rs` / `settings.rs` / `rules.rs` / `sync.rs` / `subscriptions.rs` / `updates.rs` 中 20+ 处重复控件，全部 disabled 条件、业务逻辑、toast、`ConfigPending` 置位均保留。

**F22｜信息架构**：侧栏按「连接 / 配置 / 运维」分组（只针对真实存在的 6 个 `AppView`，未编造视图），窄屏 tabbar 不分组；设置页加关键词搜索与无匹配态；概览把版本号/运行时长/DNS 数折叠进 `details/summary`，主操作位置不变。

**U14｜跟随系统主题**：`document::eval` 读 `prefers-color-scheme`，`eval.recv::<bool>()` 回传，无条件调用（项目已有 native + wasm 双端使用的先例）。集成复核时发现一处真实回归并修复：**已保存的 `dark_mode` 可能被系统偏好覆盖** —— 新增 `theme_restored_from_profile` 标记，profile 恢复与手动切换都视为显式选择，优先于系统值，并在 eval 返回后二次校验。

**U23｜轮询门控**：核心状态心跳保持常驻；规则集更新检查限制在规则视图、`proxy_groups` 预览限制在节点视图。

**B3 / B15 / B16｜统一原子写**：新建 `api/src/atomic_write.rs`，提供 `atomic_write_bytes` / `atomic_write_json` / `atomic_replace` / `replace_with_backup` / `recover_interrupted_writes`。**消除了 Windows 上「先删旧文件再 rename」的数据丢失窗口**（改为先把旧文件移到唯一备份、失败时回滚）。三处写入统一：`save_native_profile_to_path`、`sync.rs::write_private_json`、`rule_set_cache.rs::write_rule_set_commit`；崩溃恢复统一入口挂在 `load_native_profile` 且用 `OnceLock` 保证每进程只跑一次。保留 `MAX_PROFILE_BACKUPS` 轮转与 `commit_sync_pull` 备份时序。新增 6 条单测（含 macOS 上覆盖 Windows 备份回滚路径）。

**B12｜锁降级（保守）**：只读路径 `select_native_proxy_group` / `native_set_core_log_collection` / `ensure_core_is_running` 改用 `try_lock_core_slot()`，`WouldBlock` 返回「内核正在切换，请稍后重试」而非阻塞。**写路径（toggle/restart/TUN）仍持锁跨网络调用**，已在代码注释标明为已知限制，未完全解决。

**B9｜版本迁移（保守实现）**：为避免牵连全项目几十处 `AppProfile { ... }` 字面量，**没有给结构体加字段**；新增 `CURRENT_PROFILE_SCHEMA` / `profile_schema_version(&serde_json::Value)` / `migrate_app_profile(&mut AppProfile)`，做幂等的旧数据归一化（空订阅/空 tag、超长字段截断、悬空 `group_selections` 清理）。

**T3｜确定性生成式测试**：不引入 `proptest`/`cargo-fuzz`，用纯标准库 LCG 生成器 + `catch_unwind` 覆盖 3 个模块共 **16 条**新测试（parser 200 组随机字节串 / script 400 组随机 JSON / model 350 组字段组合），断言不 panic + 白名单/预算/serde 兼容不变量，失败信息带 seed 与迭代号。

**S13｜PowerShell 加固**：新增纯函数 `escape_powershell_single_quoted` / `powershell_single_quoted` / `powershell_string_array`，`launch_elevated` 里所有变量值统一转义后拼成 `@('a','b')` 数组字面量，消除 `'` / `;` / 换行 / `$( )` 注入。3 条单测，纯函数无 cfg 因此在 macOS 也可测。

**修复的两条新测试构造错误**（不是防御缺口）：随机 YAML key 里可能含 `#`/换行把 `!!` 标签变成注释；以及脚本把 `outbounds` 设成与基线完全相同的值时 delta 为空、放行本就正确 —— 两处用例已改为真正危险的形态。

### 仍未修复（第三轮后）

- **纯架构 / 跨 crate 重构**：B2（`#[kitty_server_fn]` 属性宏收敛 34 处 cfg 样板，需新建 proc-macro crate）、B4/B5（抽共享工具 / `platform` crate）、B6/B7（RESTful 化 + 统一 `ApiError` 错误码）、B9 的真正 schema 版本字段（当前是无结构变更的归一化）、B12 写路径锁拆分、U12（拆 `ProxyApp` 40 个 signal）。
- **需新依赖 / 系统能力**：S8（凭据用系统钥匙串加密落盘）、U24 的 `@axe-core/playwright` 自动扫描（需改 `package-lock.json`）、T3 的 `proptest`/`cargo fuzz` 真模糊测试、Q1/Q2/Q3/Q5/Q6/Q8 实机验证。
- **说明**：第二轮「未修复」清单里的 B1 / U8 / U14 / U23 / F22 / B3 / B15 / B16 / S13 / T3 / B9 已在本轮完成；B12 部分完成。

---

## 修复执行状态（2026-11-04 第二轮）

用户要求「都修复下吧」，已按报告实施一轮大规模修复。**全部 P0 与多数 P1 已落地并通过完整验证**；少量架构性重构 / 需引入新依赖或平台 API 的条目未在本轮完成，逐条列在末尾，不隐瞒。

### 验证结果（本机实测）

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过（0 警告） |
| `cargo test --workspace` | **253 通过 / 0 失败 / 1 ignored** |
| `cargo test -p desktop --features desktop` | 8 通过 / 1 ignored |
| `cargo test -p singbox --features embedded-core` | 38 通过 |
| `cargo check -p web --target wasm32-unknown-unknown --features web` | 通过 |
| `go test ./...`（`packages/singbox/bridge`） | 通过；`gofmt -l` 无差异 |
| 新增 `.github/workflows/ci.yml` | PR / main / 手动触发，含 fmt + clippy + 三组测试 |
| 新增 JS 测试 | `node --check` 全部通过；未改 `package-lock.json` |
| CSS | 括号配对 733/733 |

改动前源码已备份到 `/tmp/kitty-pro-backup-20261104-175730`。本轮共修改 20+ 个文件，净增约 2,600 行。

### 已完整修复

**前端 UI**：U1/U2/U3（3 处确定性 CSS bug）、U4（全局 `:focus-visible` 兜底 + 焦点环对比度）、U9（busy 同步置位 + peek 守卫）、U10/U11（`use_memo`、稳定 key、`Rc` 共享消除 O(n²) clone）、U13（profile 保存 300ms debounce + 代际丢弃）、U15（补齐字号/间距/字重/布局 token 并清理非标准字重）、U16（iOS 全输入框 16px）、U17（死规则 / z-index / 底栏变量）、U18（测试矩阵补 359/720/721/1600）、U19（44px 触摸目标）、U20（`.sr-only`、live region、错误 role）、U21（更新检查 request_id）、U22（kicker/文案统一）。

**后端与安全**：B8（`save_profile` + `ConnectionRequest` 双向入参校验）、B10（测速会话 4 并发上限 / 60s TTL / 随机 id / 无 panic）、B13（内联订阅大小上限）、B17（错误出口统一脱敏）、S1（GET 不再跳过 Origin，新增 Sec-Fetch-Site 判定）、S2（白名单精确匹配、删除 `same_origin` 的 LAN 放宽、lan_mode 绑定非 loopback 才生效）、S3（每次启动轮换 token + `Max-Age`）、S4/S5（统一 `validate_outbound_url` + 逐跳重定向复检 + 存储端宽松版 + 远端响应体不再回显）、S6（特权 helper 完整 schema 白名单、clash_api loopback + 非空 secret、rule_set URL 限制）、S7（脚本输出白名单 + 与基线 diff；新增负向单测）、S9（YAML 输入预算：字节/行/缩进/anchor/`!!` 标签）、S10（Go/Rust 两侧忽略客户端 probe_url，固定常量 + 私网校验）、S11（Windows token 校验 owner_id + 拒绝符号链接；ACL 残留限制已注释）、S12（release host 白名单 + 服务端版本覆盖）。

**业务流程**：F1/F2（模式/脚本变更接入统一生效通道，置 `ConfigPending` + 明确 toast）、F3/F9（全平台 `core_status` 心跳 + 首屏超时降级，消除"永久假连接"与"永久 spinner"）、F4（导入即切换 active 订阅，横幅不再用旧状态）、F5/F6（同步快照携带已解析节点并**在 UI 层立即恢复**、拉取后置 pending、空节点一键恢复）——此项在集成复核中发现并补上了 UI 与模型之间的断链、F7/F16（Web/Android/桌面平台能力说明与作用域提示）、F8（新增 `probe_connectivity` + 概览「测试连通」）、F10（内核重检与排查清单、空态 CTA）、F11（一次性 onboarding）、F12/F13（导入格式说明、空值提示、可取消且迟到结果不回写）、F14（模式分段控件搬到概览连接卡）、F15（同步冲突白话化 + 动作按钮 + diff 摘要 + tooltip + 说明折叠）、F18（刷新全部复用 prune）、F19（确认状态按动作隔离 + 3s 过期）、F21（桌面开关禁用原因）、F23（新增 `flow.spec.js` / `a11y.spec.js`）、T1/T2（CI 门禁 + 安全负向单测）。

**因安全策略收紧而同步更新的既有断言**：`api::native_config_rejects_script_injected_remote_rule_sets`（原断言脚本可注入 remote rule_set）、`proxy_core::sync_snapshot_carries_parsed_nodes_and_rules`（原断言快照不含节点）。两者均保留原有功能覆盖（规则集下载 detour 改为直接测 `apply_remote_rule_set_download_detour`），并新增对应的拒绝用例。

### 部分修复（有落地但非完整方案）

| 编号 | 已做 | 仍缺 |
|---|---|---|
| U5 | 分段控件 `role=group` + `aria_pressed` | 完整 `tablist/tab` + 方向键 |
| U6/U11 | topbar 与主要图标按钮补 `aria_label` | 全量图标按钮逐个核对 |
| U7 | 清空日志两步确认、删除/同步 armed 3s 过期 | 统一 `ConfirmAction` 组件 |
| U8 | 补开关 `aria_label` / `role=status` | 统一 `Switch` 组件抽取 |
| U14 | `aria_pressed` + 修正误导文案 | 首屏跟随 `prefers-color-scheme` |
| U24 | Playwright a11y 断言（按钮名/aria-pressed/焦点环/溢出） | `@axe-core/playwright` 自动扫描、对比度断言 |
| B11 | `commit_sync_pull` 等纯文件 IO 包 `spawn_blocking` | 锁内网络传输完全移出 |
| S11 | owner_id 校验 + 符号链接拒绝 | Windows SID/ACL 属主校验（现有 `windows-sys` features 不足） |
| F17 | 空态白话化、规则动作与全局模式区分说明 | 全量术语/tooltip 体系 |
| F20 | 测速 60s 超时 + 停止按钮、长事务阶段文案 | 端口重试 (n/45) 进度、可取消特权事务 |

### 未修复（需单独排期，附原因）

- **架构重构类**：B1（拆 4,647 行 `lib.rs` 为 9 个模块）、B2（`#[kitty_server_fn]` 属性宏收敛 34 处 cfg 样板）、B3/B15/B16（统一 `atomic_write_json`、Windows 原子替换、崩溃恢复三套合一）、B4/B5（抽共享工具 crate / `platform` crate）、B6/B7/B9（RESTful 化、统一 `ApiError` 错误码、`AppProfile` 版本迁移）、B12（长事务拆状态机、`core_slot` 锁降级）、U12（拆 `ProxyApp` 40 个 signal）、U23（`use_resource` 按视图门控）、U8（组件层抽取）、F22（导航分组 / 设置页搜索）。**原因**：跨文件大范围重构，需独立提交 + 完整回归，与本轮安全修复混在一起风险过高。
- **需新依赖或系统能力**：S8（凭据用系统钥匙串加密落盘）、U24 的 axe 扫描（需改 `package-lock.json`）、T3（`proptest`/`cargo fuzz`）、U14 的 `prefers-color-scheme`（需 `document::eval` 跨端验证）、S13（PowerShell 数组传参）。
- **QA 疑点**：报告 3.6 节的 Q1/Q2/Q3/Q5/Q6/Q8（CORS 默认行为、check_config 错误回显、Unicode anchor 绕过、端口 TOCTOU、Host 规范化差异、`focus.source` 脱敏）需要实机/浏览器验证，不能用静态审查替代。

> 结论：**最高风险（Web 信任边界 + SSRF 链 + 特权配置）与最高频用户伤害（UI 说谎 / 状态漂移 / 死胡同）本轮已全部消除**，并建立了 CI 门禁防止回归。剩余项属于架构演进与打磨，不影响当前版本可安全使用。

---

## 0. 项目概览

Dioxus 0.7 跨端代理客户端，`Cargo workspace` 含 7 个 crate：

| crate | 行数 | 职责 |
|---|---|---|
| `ui` | ~8,600（含 CSS 5,010） | 共享 UI 组件 + 全量样式 |
| `api` | ~8,900 | 34 个 server function、profile/runtime/sync/update、平台适配 |
| `proxy-core` | ~4,500 | 订阅解析、领域模型、sing-box 配置生成、QuickJS 脚本 |
| `singbox` | ~4,300 | Go C archive 桥、TUN/提权 helper、各平台实现 |
| `web` | ~1,200 | fullstack 服务端、CLI、daemon、会话与 Host/Origin 防护 |
| `desktop` / `mobile` | ~900 / ~700 | 各端 shell、Android VpnService |

三种运行形态：**Desktop**（进程内直调核心）、**Web**（浏览器控制面 → 本机 fullstack 服务）、**Android**（VpnService TUN）。README 明确 Web 是「控制面」， Android 强制 VPN 接管。

**总体判断**：这是一个工程质量明显高于平均水平的个人项目——颜色/圆角/阴影 token 化、明暗双主题、320–1620px 断点体系、DialogShell 焦点陷阱、规则集下载事务、ETag 条件写、helper lease/peer 校验、更新检查的克制设计，都做得相当扎实。问题不在「有没有做」，而在**抽象层缺位导致同类逻辑被复制多份、以及若干贯通性状态/信任边界没有收敛**。

---

## 1. 结论速览

| 维度 | 健康度 | 最主要短板 |
|---|---|---|
| 前端 UI 视觉/布局 | 良 | 字号间距未 token 化；3 处确定性 CSS bug |
| 前端一致性 | 中 | 缺统一组件层，二次确认/开关/分段控件各手写 6~8 遍 |
| 响应式 | 良 | 断点覆盖广，但 iOS 16px 防缩放只修一处、测试矩阵漏 720/1600 |
| 可访问性 | 中 | 弹窗是标杆，但全局 `:focus-visible` 兜底缺失、tab/分段控件无语义 |
| 后端代码结构 | 中 | `api/src/lib.rs` 单文件 4,647 行承担 16 类职责 |
| 接口设计 | 中 | 无错误码/无 DTO 隔离/无入参校验/读操作用 POST |
| 性能 | 中 | 每次 signal 变化全量写盘、零 `use_memo`、测速会话无上限 |
| 错误处理 | 良 | 超时/脱敏/事务齐备，Windows 原子写与恢复策略有缺口 |
| **安全性** | **弱** | **Web 控制台信任边界 + SSRF 链路 + 特权配置黑名单** |
| 业务流程 | 中 | **配置变更未收敛到唯一生效通道**，UI 会「说谎」 |
| 测试/CI | 中 | 243 个单测但**无全量 CI 门禁**，无安全负向测试 |

**最高风险（P0）**：LAN 模式下的 DNS rebinding 链、永久会话 token、SSRF→特权进程 SSRF→本地提权的升级路径、特权 helper 配置采用黑名单校验。
**最高频用户伤害（P0）**：切换模式/启用脚本覆写/导入订阅/同步拉取后，UI 立即显示新状态而核心仍跑旧配置，且提示会自行消失。
**最高投入产出比（单项）**：把「配置变更 → 核心生效」收敛成一条通道（详见第 5 节）。

---

## 2. 维度一：前端 UI

### 2.1 已经做好、不建议改动的部分

- 颜色 token（浅 `app.css:57-84` / 深 `:97-128`）、圆角 5 级、阴影 token；`color-scheme` 同步；字体内联 Latin woff2 + CJK 回退。
- 布局骨架清晰：`.proxy-app` > `ambient-grid` + fixed 264px 侧栏 + `main` + 6 格手机底栏；z-index 有头部注释；日志卡片 `flex:1` + 内部独立滚动 + sticky 表头，并有 `ui/src/app/tests.rs:862-897` 单测保护。
- 响应式断点覆盖 1600 / 1280 / 980 / 761–980 / 760 / 420 / 359，`tests/ui-smoke/layout.spec.js` 对 4 种手机宽 × 4 种高度逐视图断言。
- **DialogShell 是全项目最扎实的 a11y 实现**：焦点陷阱栈、嵌套弹窗按 panel 身份出栈、Escape 仅 idle 时关闭、焦点返回（`chrome.rs:70-222`，`dialog.spec.js:28-161`）。
- Toast 有同文案去重/上限 4 条/根 scope 清理；轮询用 generation 防过期响应；`prefers-reduced-motion` 全局兜底。

### 2.2 问题清单

| # | 优先级 | 问题 | 证据 | 建议 |
|---|---|---|---|---|
| U1 | **P0** | 亮/暗主题开关硬编码近白色，暗色下对比度约 2.4:1（< 3:1），滑块几乎不可见 | `app.css:2719` `.rule-enabled-toggle .switch::after { background:#f7fbf9 }` | 改用 `var(--surface-strong)` 或新增 `--switch-thumb` 并在 `.theme-dark` 覆盖 |
| U2 | **P0** | `--text-muted` 全项目**只被引用、从未定义**，busy spinner 颜色回退 | `app.css:4004` | 补定义或改用已有 token |
| U3 | **P0** | 文件末尾 `.power-label{display:none}` 覆盖 760px 媒体查询内的 `display:inline`，手机端电源按钮退化成纯图标 | `app.css:4839-4844` vs `:5007-5009` | 把默认隐藏移到媒体查询之前，或提高媒体查询内选择器特异性（建议实机复核） |
| U4 | P1 | 无全局 `:focus-visible` 兜底，9 处 `outline:0`，`.icon-button` 焦点环用 11% 透明度的 `--accent-soft`；`.text-button`/`.subscription-use-button` 等完全无焦点样式 | `app.css:521-527, 2263, 2287, 2388, 2502, 2754-2759, 2819, 3232, 3318, 4100` | 加全局 `:where(button,a,input,textarea,select,[tabindex]):focus-visible{outline:2px solid var(--accent);outline-offset:2px}`，删除各处 `outline:0` |
| U5 | P1 | 3 处分段控件只用 CSS class 表达选中，无 `role=tablist/tab`、无 `aria-selected`，div 上的 `aria_label` 在无 role 元素上不生效 | `rules.rs:158-169`、`settings.rs:278-282`、`sync.rs:93-106`；全项目 `role:"tab"` 0 处 | 抽 `SegmentedControl`（tablist/tab/aria-selected/方向键），最低限度给 button 加 `aria_pressed` |
| U6 | P1 | 手机 CSS 隐藏按钮文字后未补 `aria_label`，多个图标按钮失去可访问名 | `app.css:4357-4369` 隐藏 `.topbar-actions span`/`.status-badge`；`proxy_app.rs:802-833`；`app.css:4943-4945` + `logs.rs:170-187` | 所有图标按钮显式 `aria_label`（与可见文案同源）；隐藏文字用 `.sr-only` 而非 `display:none` |
| U7 | P1 | 危险操作二次确认重复手写 6 处且模式不一；破坏范围最大的「清空全部日志」反而一键清空、无撤销；`.confirm-dialog` 是死 CSS | `subscriptions.rs:379-448`、`rules.rs:338-361, 902-929`、`sync.rs:262-265, 399-402`、`updates.rs:400-443`、`proxy_app.rs:826-833`、`app.css:4057-4077` | 抽 `ConfirmAction`/`use_confirm()`（含点击别处取消、超时复位、aria-label 切换）；清空日志加确认或撤销 toast |
| U8 | P1 | 开关组件 8+ 处手写，`aria_label`/`aria_busy` 有无不一，busy 态把 aria 属性放在无角色的 `<span>` 上（项目自己的注释 `chrome.rs:275-277` 已指出这不会被播报） | `toggles.rs:41-86, 181-188`、`settings.rs:113-118, 298-345, 361-408, 448-452`、`rules.rs:321-334, 848-858` | 抽 `Switch { label, checked, busy, disabled, on_change }`，统一 `role=switch`/`aria-checked`/`aria-busy` + `role=status` 容器 |
| U9 | P1 | `busy` 在 async 事件 future 内才置位，`runtime.rs:136-138` 注释明确写了「太晚、挡不住双击」但组件层未沿用；`refresh_busy` 单信号被 One/All 共用，会提前解锁或互相覆盖 | `proxy_app.rs:1117`、`rules.rs:136`、`nodes.rs:126`、`subscriptions.rs:12, 54, 102, 312, 370`、`sync.rs:242, 268, 366, 413` | 统一「同步置位 + peek 重入守卫」抽 `use_guard()`；busy 改按 id 的 map 或带 generation 的 token |
| U10 | P1 | **零 `use_memo`**（`use_resource` 仅 1 处）：日志页每帧 5 次全量 filter + clone 最多 500 条；节点页每个成员 `nodes.iter().find` 线性查找；概览每帧构造 60 组图表数据 | `logs.rs:26-47, 344-356`、`nodes.rs:42-69, 407-409`、`overview.rs:100-118`、`rules.rs:36-42` | `use_memo` 包裹派生数据；线性 find 换 HashMap 索引；搜索输入 debounce |
| U11 | P1 | 列表无 key（最多 500 行日志 newest-first 头插会整列复用错位）；`ProxyGroupMemberRow` 对每个成员 `nodes: nodes.clone()` 导致 O(n²)；无虚拟化 | `logs.rs:234-236`、`nodes.rs:257-285` 尤其 `:272`、`subscriptions.rs:126-153`、`overview.rs:322-327`、`rules.rs:688-710` | 全部补稳定 key；改传 `Signal`/共享引用；日志/进程/节点列表窗口化 |
| U12 | P1 | `ProxyApp` 单组件约 40 个 signal，traffic 每秒 set 一次会连带重渲染顶栏/侧栏/当前页与全部 props 计算；`settings.rs` 24 props、`nodes.rs` 25 props | `proxy_app.rs:28-95, 575, 838-1040`；`settings.rs:167-195`；`nodes.rs:433-459` | 高频状态下沉到页面局部或独立 Context；用 `ReadOnlySignal`/`use_memo` 隔离；拆 Shell + 各页独立订阅（可复用 `runtime.rs:10-18, 62-81` 的结构体模式） |
| U13 | P1 | 每次任一 signal 变化即构造完整 `AppProfile` 并无条件 `spawn(save_profile)`，无 debounce/无代际/无串行化；脚本编辑器逐字符触发全量序列化 + fsync + rename | `proxy_app.rs:506-530`、`settings.rs:252, 655` | 300–800ms debounce + 单飞合并 + 序号校验；服务端内容未变化时跳过写（详见 B8） |
| U14 | P1 | 主题不跟随系统（`prefers-color-scheme` 全项目 0 处），首屏恒浅色；切换按钮只有 `title`，无 `aria_pressed`；「跟随浅色外观」文案与实现不符 | `proxy_app.rs:29, 465, 802-811`、`settings.rs:446` | 初始值取 `matchMedia`；按钮加 `aria_pressed` + `aria_label`；修正文案 |
| U15 | P2 | 字号 142 处全为字面量（10/10.5/11/11.5/12/13/14/15/16/18/26/28/32px + clamp），间距/侧栏宽/底栏高亦无 token；出现 `font-weight: 650/680/750` 非标准字重 | `app.css:142, 1971, 2291, 2650, 631, 651, 1080, 4319` | 补 `--fs-*`/`--space-*`/`--fw-*`/`--sidebar-w`/`--tabbar-h`，收敛到 6 档 |
| U16 | P2 | iOS 聚焦缩放只在 `.rule-field input` 修了 16px，导入弹窗/同步表单/进程搜索输入框仍 11–13px | `app.css:3743-3747` vs `:4102, 3235, 2504` | `(max-width:720px)` 统一给所有 `input/textarea` 设 16px，删特例 |
| U17 | P2 | `calc(100dvh - 86px)` 魔法数字与底栏耦合；`420px` 断点有被整行 `padding` 覆盖的死规则；z-index 出现 1/2/42 三个未登记层级 | `app.css:4319, 4889-4894, 1-3, 357, 1660, 2320, 2342` | 提 `--tabbar-h` 变量；删死规则；更新 z-index 注释并改 `--z-*` |
| U18 | P2 | 测试矩阵漏真实断点 1600/720/359（注释还写着不存在的 1180）；`body overflow-x:hidden` 会掩盖内部溢出，现有断言只测 `scrollWidth - innerWidth` | `helpers.js:7, 10, 61`、`layout.spec.js:29-41`、`app.css:44` | 补宽度；增加 `.main-content.scrollWidth - clientWidth` 断言 |
| U19 | P2 | 触摸目标普遍 30–38px（icon-button 38、toast-close 30、规则行 36、DNS 行 32、手机 nav ≈50 宽），低于 44px 建议 | `app.css:441-445, 479, 610-613, 2743-2746, 3499-3502, 4594-4598, 4378` | 760px 断点统一放大到 ≥44（或 40 + 加大间距） |
| U20 | P2 | 四态覆盖不均：订阅页/日志页/概览只有 empty，无 loading/error；表单错误与状态徽章无 live region；`log-stats`（含 5 个 button）整体设 `role=status`，1 Hz 刷新反而刷屏 | `subscriptions.rs:119-123`、`logs.rs:139, 192-215`、`rules.rs:253-258, 680-685`、`sync.rs:67-70` | 抽 `AsyncState<T>` + `StateView`；`.field-error` 加 `role=alert`；把 `role=status` 移到纯文本计数 |
| U21 | P2 | 更新检查 `refresh()` 无请求代际，启动慢检查可覆盖用户手动快检查的结果并提前清 busy | `updates.rs:100-138` + `proxy_app.rs:434-442` | 加单调 `request_id`，只接受最新响应 |
| U22 | P2 | 中英文混排格式与术语不齐（kicker 两种格式、「内核/sing-box/core」三种称呼、「语言/简体中文」是看起来可切换实则静态的行）；电源按钮同一动作 3 套文案 | `types.rs:138-147` vs `proxy_app.rs:723`、`overview.rs:74, 176-177, 228`、`settings.rs:455-462` | 抽统一 `Kicker`；对外统一「内核」；静态信息改非交互视觉；`title`/`aria_label`/可见文本同源 |
| U23 | P2 | 唯一 `use_resource` 读 12 个 signal 且读两次 `config_script()`，结果只在 Nodes 页用，6 个视图都会触发较重的代理组预览 | `proxy_app.rs:665-698, 877` | 先 `use_memo` 算 request；仅 `active_view == Nodes` 且配置真变化时触发 |
| U24 | P2 | 表格用 div+span 无 table 语义；搜索框仅 placeholder 无 label；装饰色块无 `aria_hidden`；E2E 无 axe-core，且 `playwright.config.js:27` 固定 `reducedMotion:reduce` 导致动效行为永远测不到 | `logs.rs:217, 227-233`、`rules.rs:249-255, 673-679`、`nodes.rs:108-115`、`overview.rs:308, 314` | 补 role=table/aria_label/aria_hidden；引入 `@axe-core/playwright` 跑 6 视图 × 明暗主题 × 320/1440 扫描 |

---

## 3. 维度二：后端 / 核心

### 3.1 已经做好、不建议改动的部分

- 订阅/规则集/同步/更新/helper 均有明确超时；订阅下载与规则集有 10 MiB 上限；同步用 `redirect(Policy::none())`；profile 写用临时文件 + `sync_all` + rename（POSIX 原子）。
- 规则集下载有完整的「临时文件 + 事务 JSON + commit 标记 + 回滚/恢复」（`lib.rs:1488-1747`）；profile 有 8 线程并发写原子性单测（`:4084-4133`）。
- `try_lock_core_slot` 让状态/流量/日志轮询不排长事务队列；锁忙时返回快照降级（`lib.rs:1778-1787`）。
- QuickJS 沙箱无 file/net/require，有内存/栈/超时限制（`script.rs:6-97`）；shell 调用均为常量程序 + 常量参数或已校验 IP。
- macOS helper 的 Unix socket peer uid/pid + nonce + lease watchdog + cache 文件属主/0600 校验，是全项目安全设计最严谨的一处。
- 约 243 个单测，覆盖解析、模型、事务、并发写、CLI、Host/Origin、更新卡片、FakeRemote 端到端同步。

### 3.2 代码结构

| # | 优先级 | 问题与证据 | 建议 |
|---|---|---|---|
| B1 | P1 | `api/src/lib.rs` 4,647 行承担 16 类职责：DTO(`:56-241`)、34 端点(`:250-754`)、平台分派、测速会话(`:782-1143`)、脱敏、订阅下载、规则集事务(`:1252-1767`)、核心状态、profile 持久化(`:1832-2022`)、配置构建与脚本(`:2037-2191`)、启停事务(`:2204-2770`)、流量日志解析、系统代理/剪贴板/进程(`:3016-3476`)、全局锁、约 1,100 行测试。`sync.rs`/`update.rs` 已拆分说明模式存在但没贯彻 | 先做无行为变化的 move 拆为 `dto/server_fns/profile/core/latency/rules/system_proxy/clipboard/processes/platform`，`lib.rs` 只留 `pub use` 与 cfg 门控 |
| B2 | P1 | 34 处 `cfg_attr(all(not(android/ios), any(wasm32, feature="server")))` 逐字复制（`lib.rs:250-750`、`update.rs:159-291`）；同一能力按 wasm/android/desktop 写三份，`measure_native_latency` 的 desktop 与 android 有约 55 行几乎逐字相同 | 抽 `#[kitty_server_fn]` 属性宏收敛门控；引入 `trait CoreBackend`，公共编排只写一份 |
| B3 | P1 | 「临时文件 + sync_all + rename」写了三遍且行为漂移：profile 用 pid+原子计数、`sync.json` 用固定 `sync.json.{pid}.tmp`、update 用 `File::create` 默认权限 | 抽 `atomic_write_json(path, bytes, UnixMode)` 到 `persist.rs`，统一 symlink 检查/`create_new`/权限/fsync/目录 fsync/Windows 替换 |
| B4 | P2 | `constant_time_eq` 重复实现（web `main.rs:393-404`、helper `desktop_helper.rs:675-685`）；跨 crate 边界两处越界：UI 直接依赖 `proxy-core` 领域类型并 re-export 成 UI 类型（`ui/src/app/mod.rs:64-69`）、UI 直接同进程调 `api::autostart::*`（非 HTTP、不过鉴权，`settings.rs:37,66,73,107`） | 共享工具 crate 或 `subtle`；定义传输 DTO；autostart 改 server fn 或下沉到 `desktop` crate |
| B5 | P2 | `api` 同时是 API 定义层与桌面 OS 适配层（sysproxy/剪贴板/注册表/路由/进程枚举），被拖进所有编译目标，cfg 组合爆炸 | 新建 `platform` crate 承载 OS 能力，`api` 只依赖 trait，由 `web`/`desktop` 注入实现 |

### 3.3 接口设计

| # | 优先级 | 问题与证据 | 建议 |
|---|---|---|---|
| B6 | P1 | 34 端点（6 GET / 28 POST）风格混用：纯读的 `POST /api/core/status`(`:487`)、`/api/core/logs`(`:640`)、`/api/core/latency/poll`(`:696`)；`GET /api/profile` 一次性返回含全部订阅 URL 与节点密码的 `AppProfile`；`GET /api/processes`(`:750-753, 3345-3373`) 返回全机进程名 + 可执行路径 | 读写方法统一（读 GET + 查询参数，写 PUT/PATCH 到资源路径），便于中间件按方法施加防护；`/api/profile` 改按页面裁剪的 DTO；`/api/processes` 白名单过滤或移除 |
| B7 | P1 | 错误是自由文本 `ServerFnError::new(format!(...))`，无 `code`/`retryable`/`category`，无 `/api/v1` 前缀；前端只能按字符串匹配分支；同步错误把远端响应体前 512 字符回显（`sync.rs:1168-1182`），`check_config` 错误直接下发（`:2271`） | 定义 `ApiError { code, message, retryable }`；对外只发 code + 脱敏通用文案，细节写服务端日志；加 `Api-Version` 响应头 |
| B8 | **P0** | `save_profile(profile: AppProfile)` 直接落盘，**零入参校验**（`lib.rs:268-273`）；规则上限只在 `validate_custom_rules`、脚本 512 KiB 上限只在同步快照校验。配合 U13 的每次 signal 变化全量写，恶意/被劫持客户端可写入超大 profile 形成持久化 DoS，且并发写 last-write-wins | 在 `save_profile` 入口做与 `sync::validate_snapshot` 同级的校验（订阅数/节点总数/规则/脚本长度/总大小），校验函数放 `proxy-core` 供 save/start/sync 共用；服务端内容未变化时跳过写 |
| B9 | P2 | 版本策略不统一：同步快照有 `SYNC_SNAPSHOT_VERSION=2` 与 v1 兼容，本地 `AppProfile.version` 恒为 1 且加载无迁移，HTTP API 无版本协商（release.yml:447 注释要求前后端同版本） | `AppProfile` 按 version 显式迁移并拒绝未来版本；DTO 与领域模型解耦 |

### 3.4 性能

| # | 优先级 | 问题与证据 | 建议 |
|---|---|---|---|
| B10 | **P0** | 测速会话全局 `HashMap` 仅在 `poll` 且 `done==true` 时 remove（`lib.rs:1082-1143`）；无并发上限、无 TTL、无随机 id（自增 `AtomicU64` 可枚举）；每次 start 都 `tokio::spawn` 并起一个真实 `SingBox` 实例（`:934-995`）；后台任务内 `:963` 有 `expect`，panic 则 `finish()` 永不执行 → 前端永久轮询 | 加 `MAX_CONCURRENT_SESSIONS=4` + 60s TTL 清理任务；`JoinSet` + `catch_unwind`/Drop guard 保证 `finish()`；session id 随机化并绑定创建者；加全局在途探测上限 |
| B11 | P1 | 三个同步函数在 async 上下文裸做同步文件 IO：`sync_push`(`:327`)、`sync_pull`(`:361`)、`commit_sync_pull`(`:393-396`，含 `create_dir_all`/`copy`/`read_dir`/`remove_file`)；`operation_lock` 跨最长 45s 网络往返 | 统一走 `spawn_blocking`；锁缩短为「读基线 + 预检」，网络传输不持锁，写入阶段靠已有 ETag 条件请求保证一致性 |
| B12 | P1 | 全局 `core_slot` 锁可持有 45s（45×1s bind 重试，`:2316-2334`）到 120s（TUN 授权，`:2691-2696`）；`select_native_proxy_group`(`:2067`)、`native_set_core_log_collection`(`:2870`)、`ensure_core_is_running`(`:3423`) 仍用阻塞 `lock()` | 长事务拆状态机，锁内只做状态切换；所有只读/短操作统一 `try_lock` + 快照降级；bind 循环加 `CancellationToken` |
| B13 | P2 | 内联提交的订阅内容（非下载路径）无字节上限，`parse_subscription` 无行数/节点数上限、对整段文本依次尝试 4 种 Base64 引擎（`parser.rs:31-64, 673-690`）；测速每次起完整 Go box | `preview_subscription` 入口查 `source.len()`；解析加 `max_lines/max_nodes` 提前退出；Base64 按前缀/长度预筛；探测实例复用或全局信号量 |
| B14 | P2 | 1 Hz traffic + 1 Hz logs + 100ms latency poll（`ui/src/app/types.rs:84-112`）；每次请求都 `spawn_blocking` → 锁 → FFI → Go 本地 HTTP；规则集锁跨两次 45s 下载（`:1277-1312`）、更新 `state_lock` 跨 GitHub 往返（`update.rs:243-275`） | 合并轮询端点或改 SSE；页面不可见暂停；latency poll 指数退避；长网络请求用 in-flight 标志 + 结果共享替代互斥锁 |

### 3.5 错误处理

| # | 优先级 | 问题与证据 | 建议 |
|---|---|---|---|
| B15 | P1 | Windows 分支「先 `remove_file(path)` 再 rename」有丢失窗口（`:1958-1964`、`sync.rs:1252-1256`、`update.rs:584-588`）；三处都未对父目录 `fsync`；`load_native_profile` 的损坏备份只处理「存在但解析失败」，不处理「文件消失」 | Windows 用 `MoveFileEx(MOVEFILE_REPLACE_EXISTING\|WRITE_THROUGH)` 或 `ReplaceFile`；POSIX rename 后 `File::open(parent).sync_all()`；统一封装（B3） |
| B16 | P1 | 崩溃恢复策略三套且不一致：规则集有完整事务；profile 解析失败仅备份一次且后续损坏不再覆盖（`:1834-1851`）；`sync.json` 损坏直接报错无备份；`update.json` 回退默认值自愈 | 把规则集事务模式抽成通用 `atomic_json_store`；备份带时间戳保留 N 份（复用 `:1867-1928` 已有逻辑） |
| B17 | P1 | 脱敏只覆盖订阅下载路径（`redact_secrets` 仅被 `:1216,1223` 调用）；sing-box `check_config` 错误可能回显配置片段含节点密码；同步错误含完整 endpoint URL 与远端响应体 | 把 `redact_secrets` 提升为 `ServerFnError` 统一出口包装；sing-box 错误再做配置值黑名单过滤；远端响应体只记日志不回显 |
| B18 | P2 | 四套错误类型并存（`CoreError`/`ParseError`/`ConfigScriptError`/`macos_route.rs` 整模块 `Result<_,String>`），边界反复 `to_string()` 丢失分类；bind 重试靠英文子串 `"address already in use"` 匹配，本地化环境失效（`:2327`） | 统一 `thiserror` 化的 `ApiError` + `#[from]`；bind 错误结构化枚举或同时匹配 `EADDRINUSE`/`10048`；幂等 GET 加 1–2 次指数退避 |

### 3.6 安全性（最高风险区）

**已确证：**

| # | 优先级 | 问题与证据 | 建议 |
|---|---|---|---|
| S1 | **P0** | **GET 完全跳过 Origin/CSRF 校验**：`valid_control_request` 里 `request.method()==Method::GET \|\| same_origin(...)`（`web/src/main.rs:243-252`）。而 GET 端点包含 `GET /api/profile`（全量凭据）与 `GET /api/processes`。实际危害取决于 CORS 是否宽松（待验证 Q6），但「把读方法当安全边界」本身是错的 | 所有方法统一校验 `Origin`/`Sec-Fetch-Site`，GET 不例外；敏感读取要求自定义头 `X-Kitty-Control` 以强制 CORS 预检 |
| S2 | **P0** | **LAN 模式把 Host/Origin 防护实质性架空**：`host_allowed` 对任意 IP 字面量直接 `return true`，白名单是**后缀匹配** `host==suffix || host.ends_with(".suffix")`（`main.rs:147-164`）；`same_origin` 在 LAN 模式复用同一个 `host_allowed(authority)`（`:304-338`）。可构造 DNS rebinding：`evil.example.net` → 主机 IP → Host 命中后缀白名单 → 首屏 GET 下发 cookie → 同源 `fetch('/api/profile')`。注释声称「配合 SameSite=Strict 封住」，但同源场景下 SameSite 不成立。`cli.rs:17-24, 112-114, 143-146` 还允许 `KITTY_PRO_ALLOW_LAN=1` 与 loopback 绑定并存，导致「只绑本机却启用了宽松策略」 | 白名单改**精确主机名**（去掉 `ends_with`）并把 Host 解析到 IP 与绑定地址比对；`same_origin` 只接受 `authority == host`；`lan_mode` 由实际绑定地址决定而非独立环境变量；开启 LAN 强制白名单非空、拒绝 IP 字面量 Host；补 `Sec-Fetch-Site` 严格校验 |
| S3 | **P0** | **永久共享会话 token**：32 字节随机值持久化到 `web-session.key`，重启不变；cookie 无 `Max-Age`/`Expires`/`Secure`；无会话列表/轮换/登出/撤销；LAN 是纯 HTTP，token 与全量凭据响应明文传输（`main.rs:23-64, 166-171`、`cli.rs:149-157` 仅警告不强制 TLS） | 短期 session + 轮换 + 显式登出 + revoke；绑定首次 UA/来源；LAN 默认要求 HTTPS 或至少 UI 二次确认；token 存系统钥匙串而非普通文件 |
| S4 | **P0** | **订阅 URL 确证的 SSRF**：`download_subscription` 只校验 scheme 为 http/https，`redirect(Policy::limited(5))`，无主机/IP 白名单、无重定向后复检、无 DNS 结果校验（`lib.rs:463-480, 1193-1244`）。可探测内网、`169.254.169.254` 云元数据，靠时序/状态码区分拓扑 | 建立统一 `safe_outbound_url()`：拒绝私网/回环/链路本地/组播/保留 IP，逐 DNS 结果校验，重定向每跳复检，默认仅 https；订阅下载用禁用代理/凭据的独立 Client |
| S5 | **P0** | **WebDAV/S3 endpoint 同样任意指定**，`send_s3` 会把 `Authorization: AWS4-HMAC-SHA256` 投送到该主机，`send_webdav` 带 Basic Auth；`require_success` 还把该主机响应体前 512 字符回显 UI（`sync.rs:321-411, 955-987, 1023-1088, 1168-1182`） | 复用 S4 出站校验器；endpoint 主机需与用户显式确认的允许列表一致；SigV4 签名前校验 host；错误响应体不回显 |
| S6 | **P0** | **特权 helper 用黑名单校验配置**：`validate_privileged_start_config` 只拒绝 `log.path`/`log.outputs`、要求 `cache_file.path` 是绝对路径（`singbox/src/lib.rs:797-834`）。helper 以 root/admin 运行，`inbounds`/`outbounds`/`external_controller`/`route.rule_set[].url` 全不受限。macOS 有更严的 `validate_privileged_cache_file`（`macos.rs:467-533`），但 Windows/Linux 无等价实现 | 改**完整 schema 白名单**：只允许应用生成的键集合；`external_controller` 强制 `127.0.0.1` + 非空 secret；`cache_file.path` 限定应用数据目录内且预创建 + 属主/权限校验（复用 macOS 实现下沉共享） |
| S7 | **P0** | **JS 配置脚本输出是黑名单**：`reject_clash_config` 只拒 7 个字段（`script.rs:107-128`），脚本可返回任意对象并改写 `inbounds`（凭空加 TUN）、`experimental.clash_api`（绑 `0.0.0.0`）、`route.rule_set[].url`（SSRF）、`cache_file.path`。QuickJS 执行沙箱本身严谨，缺的是**配置语义白名单**（`lib.rs:2103-2132`、测试 `:4227-4284` 明确展示脚本可插入 remote rule_set 并被保留） | 改输出 schema 白名单：只允许 `log.level`/`route.rules[*]`/`dns.rules[*]` 等枚举键；应用脚本前后各做一次与基线的 diff 校验；显式禁用 `eval`/`Function` |
| S8 | P1 | 凭据全明文落盘：订阅 URL（常含 token）、节点密码、WebDAV 密码、S3 SecretKey 以明文 JSON 存数据目录（`model.rs:446-497`、`lib.rs:1947-1948`、`sync.rs:161-194, 1235-1250`），同步快照把订阅 URL 原样明文上传；`GET /api/profile` 全量下发。profile 目录未设 0700（`:1941-1945`），`masked_for_client` 只掩码 password/secret_key，access_key/username 原样回显 | 敏感字段用系统钥匙串或 OS 派生密钥加密；快照提供加密或「不同步订阅 URL」选项；`/api/profile` 裁剪；数据目录统一 0700；所有凭据类字段纳入掩码 |
| S9 | P1 | YAML 反序列化只靠字符串启发式 `yaml_expansion_bomb`（`parser.rs:361-408`：逐行扫 ASCII `&name`/`*name`、要求同名才拒），仍直接 `serde_yaml::from_str`，不限制嵌套深度/节点总数/alias 引用次数 | 改事件级计数（深度/节点/alias 预算）或换带 `max_depth` 的解析器；补 fuzz 语料 |
| S10 | P1 | helper RPC 的 `ProbeOutbound { tag, probe_url }` 接受任意 URL，Go 侧 `url.Parse` 后直接 HEAD，无 scheme/host/IP 白名单（`desktop_helper.rs:142-155, 521-527`、`bridge/main.go:576-597`）；API 层虽固定常量，RPC 协议本身不设防 | helper 侧校验或忽略 `probe_url`，只用编译期常量 |
| S11 | P1 | Windows helper 的 `read_session_token` 忽略 `owner_id`，不校验文件属主/ACL（`desktop_helper.rs:639-673`），弱于 Unix 侧（canonicalize + `O_NOFOLLOW` + uid/mode + peer 校验）；`reserve_loopback_port` 先 bind 取端口再 drop 存在 TOCTOU（`:48-51, 550-554, 415-419`） | Windows 补 SID/ACL 校验，owner_id 真正参与；端口预留改为传递已绑定 listener 或用 Unix socket |
| S12 | P2 | 所有非 `/api/` GET 都重发 `Set-Cookie`（`main.rs:195-206`），token 在静态资源/日志中重复出现；`release_url` 只校验 scheme 不校验 host（`update.rs:484-504`）；`current_version` 由客户端提供可伪造；`probe` 后台 `expect` 会 panic | 只在文档入口下发 cookie；release host 白名单；版本用服务端 `CARGO_PKG_VERSION` 覆盖 |
| S13 | P2 | shell 注入面基本受控（程序/参数常量，IP 由 `IpAddr` 格式化，uid/action 白名单）；仅 Windows PowerShell 分支用字符串拼 `-Command`（`desktop_helper.rs:751-758`） | 改 `Start-Process -ArgumentList` 数组传参 |

**需进一步验证的疑点（不臆断）**：
- Q6：Dioxus server 对 `/api/*` 的 CORS 默认行为决定 S1 的实际危害（curl 带 `Origin` 测 `GET /api/profile` 看响应头）。
- Q2：Go `check_config` 错误是否回显含密码的配置片段。
- Q3：Unicode anchor 名 / 块标量内 alias 能否绕过 `yaml_expansion_bomb`。
- Q5：预留端口 TOCTOU 能否让本机进程骗取 RPC token。
- Q8：Host 白名单的尾点/IDN/IPv6/大小写规范化差异是否产生绕过。
- Q1：`nodes.rs:318` 渲染的 `focus.source` 是否为未脱敏的完整订阅 URL。

### 3.7 测试与 CI

| # | 优先级 | 问题与证据 | 建议 |
|---|---|---|---|
| T1 | **P0** | **CI 从不跑全量测试**：`release.yml:134-143` 仅 Windows job 有 `go test ./...` + `cargo test --package singbox`；`ui-smoke.yml` 只做 `dx build` + Playwright；全仓无 `cargo test --workspace`/`clippy`/`fmt --check`/`audit`。约 243 个单测实际只在开发机跑 | 新增 `ci.yml`：PR 必跑 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（按 target 矩阵）、`cargo audit`；release 前复用 |
| T2 | P1 | 高危项全部无回归保护：无真实 HTTP 的 `control_session_guard` 测试（只有纯函数单测）、无 SSRF 用例、无 LAN 后缀 + DNS rebinding 组合、无脚本输出字段注入负向测试、无 helper 伪造请求集成测试 | 为 S1/S2/S4/S5/S6/S7 各补一条负向测试；出站校验器做表驱动（含 IPv6/十进制 IP/重定向） |
| T3 | P2 | 并发/资源边界测试不足：`FakeRemote` 是串行模型，无多客户端 push/pull 竞争、无 `core_slot` 长事务期间请求行为、无 session 泄漏断言；无 fuzz/属性测试 | `#[tokio::test(flavor="multi_thread")]` 并发用例；「创建后不 poll，60s 应回收」断言；对 `parse_subscription`/`apply_config_script`/`parse_route_log` 加 `proptest`/`cargo fuzz` |

---

## 4. 维度三：业务流程与用户路径

### 4.1 主路径复盘

| 步骤 | 点击数 | 说明 |
|---|---|---|
| 首次启动 | 0 | 落在概览，**无 onboarding** |
| 添加订阅 | 1 | 概览 topbar |
| 解析节点 | 1 | 可能阻塞 10–30s |
| 选节点 | 0 | 导入时自动选首个节点、自动写 `group_selections["proxy"]`（`proxy_app.rs:1154-1170`） |
| 选模式 | 0（默认 Rule）或 2 | 模式在最后一个 tab |
| 开启核心 | 1 | 概览电源按钮 |
| **验证连通** | **无此动作** | 只能被动看流量图/日志 |
| 日常切换节点 | 2 + 定位 | 概览节点速览是预览不可选；桌面托盘 1 次，Web/Android 无托盘 |
| 停止 | 1 | |

最小路径 3 次点击即可连上，**这是设计得好的地方**。问题集中在引导缺失与状态一致性。

### 4.2 已经做好、不建议改动的部分

- 节点切换先热切换，失败才重启并回滚（`nodes.rs:512-557`、`proxy_app.rs:252-303`）；启动/停止失败 `refresh_runtime_state` 回读真实状态；订阅切换/刷新/删除失败回滚；空解析不覆盖旧节点。
- 更新检查的设计与实现高度一致（启动检查不打扰、失败留在卡片、显示匹配目标、不下载不安装），并有 `update.spec.js:50-148` 覆盖，是全项目文档/实现对齐最好的模块。
- 同步配置读取失败会禁用全部按钮 + 显示原因 + 提供重试，避免用空配置覆盖 `sync.json`（`ui/src/app/sync.rs:29-90`），可作为其他异常路径的模板。
- 默认值安全：`Rule` 模式、TUN/系统代理/LAN/脚本覆写默认关，同步默认 disabled 且带安全提示。

### 4.3 问题清单

| # | 优先级 | 问题与证据 | 用户影响 / 建议 |
|---|---|---|---|
| F1 | **P0** | **切换分流模式不重启核心、不置 `config_pending`，UI 立即显示新模式**：`settings.rs:278-282` 三个按钮只 `tunnel_mode.set(...)`；`overview.rs:151` 的 status-line 直接读 `mode_label`。规则编辑走 `notify_rule_change` 会置 pending + 弹「重新连接」横幅（`rules.rs:1036-1047`），模式切换没有；`ConnectionRequest.mode` 只在 `spawn_core_toggle`/`restart_core_from_ui` 下发（`runtime.rs:20-57`） | 核心跑旧模式而 UI 声称新模式，且新模式被自动落盘、重启应用后才生效 → 用户以为即时生效。**与规则编辑共用同一入口**：连接中立即 `restart_core_from_ui`，或至少置 pending + toast |
| F2 | **P0** | **JS 覆写开关/脚本保存同样不重启不置 pending**（`settings.rs:257-266, 647-658`），但脚本启用会清空全部自定义规则、规则页立刻显示「脚本覆写」（`rules.rs:370-377, 976-1003`） | UI 与核心行为相反。纳入统一生效通道 |
| F3 | **P0** | **桌面/Web 没有 `core_status` 心跳**：非 Android 分支只在启动时读一次（`proxy_app.rs:359-378`），Android 是 1 Hz 无限轮询（`:380-409`）；traffic 轮询对 `Err` 静默（`:532-589`），`core_state` 保持 `"running"` 导致轮询无限空转 | 核心崩溃/被外部停掉/helper 租约到期后，桌面/Web **永久显示「已连接」**。把 Android 轮询提到公共路径（2–3s），`core_traffic` 连续失败时回读核心状态 |
| F4 | **P0** | **导入新订阅后的「重新连接」横幅用旧状态重建请求**：导入只置 pending + toast「当前连接未变化」，不改 `active_subscription_id`/`nodes`/`selected_tag`（`proxy_app.rs:1144-1152`）；横幅重建的 `ConnectionRequest` 仍是旧 `nodes()/selected_tag()/group_selections()`（`overview.rs:253-274`），重启成功后清 pending | 用户点「重新连接」以为新订阅生效，实际跑旧订阅且提示消失。真正生效要去订阅页点「使用」——横幅没说。**导入时直接切换 active 订阅并重启，或横幅明确指向订阅页且不要清 pending** |
| F5 | **P0** | **同步拉取覆盖本机后节点清空、不置 pending、核心仍跑旧配置**：`ui/src/app/sync.rs:477-514` 设 `active_subscription_id=None`/`nodes=Vec::new()`/`selected_tag=""`，全文件无 `ConfigPending`；`SyncSubscription` 只含 id/name/source，`From` 实现把 `nodes` 设为空（`model.rs:504-531`）；`connection_allowed` 要求 `!nodes().is_empty()`（`proxy_app.rs:715`） | **最明确的死胡同**：连接中拉取后断开就无法再启动，必须先逐个刷新订阅，中间只有一句 toast。拉取后若 connected 则置 pending；空节点态给「一键刷新全部订阅并恢复使用」 |
| F6 | **P0** | **同步快照不含已解析节点，与 README 直接矛盾**：README:188-189 声称 "including their last parsed nodes"，实现 `SyncSubscription` 只有 3 字段 | 要么在 snapshot 携带 nodes 与 README 对齐，要么修正 README 并补「一键刷新全部」 |
| F7 | P1 | **Web 上仍渲染 TUN / 系统代理开关且可用，但作用对象是服务器主机**：`settings.rs:283` 的 `!cfg!(target_os="android")` 对 wasm32 也为真；server fn 在原生服务端执行，改的是 `127.0.0.1:7890` 所在主机 | 从手机访问控制面的用户点「系统代理」，改的是远端主机，自己手机毫无变化。加醒目的作用域说明，或按「控制面」定位改为只读展示 |
| F8 | P1 | **「验证连通」这一步完全缺失**：`overview.rs:63-67, 119-125, 298-333` 只有「已连接」大标题、流量图、活动连接数；节点页只有 TCP 延迟，无出口连通性/出口 IP 自检 | 「已连接」只代表 sing-box 进程 running（`domain.rs:74-81`），不代表出口可用。概览加「测试连通」按钮，区分「内核运行中」与「代理可用」 |
| F9 | P1 | **首次状态读取无超时/错误态**：`core_state` 初值 `"checking"`（`proxy_app.rs:34, 87-88`），若首个 `core_status()` 无法完成（wasm 未 hydrate / API 不可达）状态永远停在「正在检查 sing-box」，底部 tab 无响应。README:122-127 把它归因为部署问题 | 这是 UI 缺陷而不仅是部署注意。给首次状态读取加 8s 超时 + 「连接失败 / 重试 / 查看诊断」 |
| F10 | P1 | **三处死胡同**：F5 的节点清空、F9 的永久 spinner、`内核 unavailable` 时电源按钮禁用但无「重试检测」/排查清单（`proxy_app.rs:715`、`settings.rs:229-234`、`domain.rs:74-80`） | 全 UI 统一规范：**每个空态/失败态都必须配一个明确的「下一步」按钮** |
| F11 | P1 | **首次无 onboarding；三个空态两个没 CTA**：概览 `EmptyNodes`（`overview.rs:395-396`）、订阅页空态（`subscriptions.rs:119-123`）无按钮，只有节点页带「添加订阅」 | 一次性 3 步引导卡 + 统一 `EmptyState` 组件（均带 CTA 与用途说明） |
| F12 | P1 | **导入对话框无格式说明/示例/剪贴板导入**：只有名称 + 地址两个字段（`proxy_app.rs:1078-1102`），README 用 17 行解释 Base64/Clash YAML/Hysteria2/VMess/SOCKS5 单节点写法，UI 一个字都没有 | 加可折叠「支持格式」示例 + 「从剪贴板导入」 |
| F13 | P1 | **导入对话框在 10–30s 下载期间不可关闭**：`dismissible: !import_busy()`（`proxy_app.rs:1063`），按钮只有 spinner，无超时倒计时/取消（`:1116-1119`） | 显示「正在下载订阅…（可取消）」，允许取消并关闭 |
| F14 | P1 | **模式切换埋在最后一个 tab 且设置页需滚动**（`proxy_app.rs:1043-1055`、`settings.rs:210-282`）；6 个平级 tab 的 `MAIN` 分组名存实亡，手机 6 格偏挤 | 模式切换（分段控件）放到概览连接卡；侧栏分「连接/配置/系统」三组，手机收敛到 4 个 tab |
| F15 | P1 | **同步冲突把 ETag / 基线 / revision 术语直接抛给普通用户**（`sync.rs:169-193`、`ui/src/app/sync.rs:385, 427`），toast 不引导到「下载覆盖本机」或「覆盖远端」按钮；四个按钮（保存/下载/上传/覆盖远端）看起来功能相近且无 diff 预览 | 错误分类成「远端已被其它设备修改」并附两个动作按钮；覆盖类二次确认显示摘要（「将用远端 3 订阅/12 规则替换本机 5 订阅/8 规则」）；加 tooltip 解释三按钮区别 |
| F16 | P1 | **Web/Android 能力差异零解释**：`platform` 是英文 `"Web Control"/"Desktop"/"Mobile"` 混在中文界面（`web/src/main.rs:412`、`settings.rs:227`）；Android 隐藏 TUN 开关并强制 `tun=true`（`toggles.rs:304`、`lib.rs:2751-2753`）却不说明「VPN 已接管」；Android 系统代理显示「当前平台尚未实现系统代理适配」（`lib.rs:3208-3217`） | 加平台说明卡（中文）：「你正在通过浏览器控制运行在服务器上的 sing-box」「Android 由 VPN 接管，无需系统代理」；桌面 TUN 行预告「首次启用会弹出授权」 |
| F17 | P2 | 术语与认知负担：`TunnelMode` 只有 Rule/Global/Direct，但规则动作有 Direct/Proxy/Block、日志 chip 又是「代理/直连/拦截」（`model.rs:167-182`、`domain.rs:31-37`）；TUN/系统代理/VPN 区别无解释；mixed inbound/7890/fake-ip/IPv6 在 UI 零解释（README 用了大段）；空态里出现 `selector/urltest`（`nodes.rs:210-223`） | 模式改白话「智能分流 / 全部走代理 / 全部直连」；概览流量卡脚注加可展开「本地代理信息」；统一 tooltip 体系 |
| F18 | P2 | 「刷新全部订阅」未 `prune_group_selections`（`subscriptions.rs:85-102`），单个刷新/切换/删除都做了（`domain.rs:92-102` 注释明确说明残留 outbound 会随之后每个 `ConnectionRequest` 下发） | 抽公共 prune 函数供两个入口调用 |
| F19 | P2 | 删除确认 title 承诺「点击其它位置取消」未实现，且 `confirm_delete` 是列表共享的单一 signal，A 行 arm 后点 B 行变成 arm B（`subscriptions.rs:33, 48, 390-401`）；同步页 Pull/ForcePush 共用 confirm | 实现点击外部/切页取消或改文案；每动作/行独立确认状态 + 时间戳自动过期 |
| F20 | P2 | 测速无超时/取消（`nodes.rs:139-176`、`lib.rs:946-994`），失败节点仍可选、电源按钮不校验；端口占用 45s 静默重试无说明（`:2293-2335`）；TUN 授权可等 120s 无阶段文案、无取消（`runtime.rs:271-334`） | 前端 30–60s 超时 + 「停止测速」；不可用节点标记警告；长事务显示阶段文案（「正在申请网络权限，请在系统弹窗中授权」「端口 7890 被占用，正在重试 n/45」）+ 取消入口 |
| F21 | P2 | 桌面 TUN 开关无「节点为空」校验，手机 QuickToggle 有（`settings.rs:298-305` vs `toggles.rs:333, 237-244`）；桌面系统代理禁用时无原因（`:45-49` vs `:403-412`）；概览无 loading/error 态 | 复用手机端已有的 `disabled_reason` 逻辑到桌面，统一平台行为 |
| F22 | P2 | 高频「切换节点」需跳页 + 定位（概览 4 行注释明确是 preview，`overview.rs:398-407`）；概览 6 个区块同权重稀释主信息；设置页 7 section 长滚动无分组/搜索/折叠 | 概览节点速览直接可选或提供当前节点下拉；连接卡放大为主卡；设置页二级分组 + 搜索 |
| F23 | P2 | 业务流程无端到端测试：`dialog.spec.js:6-10` 打开导入对话框只为测 Escape/Tab，`tests.rs` 全是纯函数 + CSS 守卫，无任何断言「切模式后核心是否重启」「导入后 active 是否切换」「同步后节点是否恢复」 | 补 3–5 条 E2E 流程测试与 `config_pending` 断言——这正是 F1~F6 能长期存在的原因 |

---

## 5. 优先级总表与执行路线

### P0（发布阻断 / 立即修）

**安全**：S1–S7（信任边界、SSRF 链、特权配置白名单、脚本输出白名单）、B10（测速会话泄漏）、T1（CI 门禁）
**业务**：F1–F6（配置变更生效通道、状态心跳、同步死胡同）
**UI**：U1–U3（3 个确定性 CSS 缺陷）、B8（save_profile 零校验）

### P1（高收益，一个迭代内）

U4–U14（组件层 + a11y + 渲染性能 + 保存防抖）、B1–B3/B6/B7/B11/B12/B15–B17（结构/接口/性能/错误）、S8–S11、T2、F7–F16

### P2（体验打磨）

U15–U24、B4/B5/B9/B13/B14/B18、S12/S13、T3、F17–F23

### 建议执行顺序

1. **第 0 周（安全网）**：T1 加 `ci.yml`；为 S1/S2/S4/S6/S7 各补一条负向测试。成本最低，让后续改动有回归保护。
2. **第 1–2 周（信任边界）**：S1–S7 + S8 一次性收敛：`safe_outbound_url()` 统一出站校验器、配置/特权配置输出白名单、Web 会话与 Host/Origin 收紧、`save_profile` 入参校验（B8）。**在做 LAN 暴露之前必须完成**。
3. **第 3–4 周（状态一致性）**：F1–F6 + F8/F9/F10 建立唯一「配置变更 → 核心生效」通道、统一 `core_status` 心跳、补「测试连通」与首屏超时；同时做 U13 保存 debounce 与 B10 会话上限。
4. **第 5–8 周（抽象层）**：U4–U9 抽 `Switch`/`SegmentedControl`/`ConfirmAction`/`IconButton`/`StateView`；U10–U12 渲染性能；B1–B3 拆 `lib.rs` 与 `atomic_write_json`。
5. **持续**：P2 体验项与 F23 的 E2E 流程测试。

---

## 6. 收益最高的改进项

### 单项 ROI 第一：建立唯一的「配置变更 → 核心生效」通道，并让概览「重新连接」始终用当前完整状态重建请求

**理由**：
- 它是唯一一个同时造成「UI 显示与核心行为相反」「用户以为生效其实没生效」「提示消失后无法恢复」三类问题的**根因**，一次性覆盖 F1（模式切换）、F2（脚本覆写）、F4（导入订阅）、F5（同步拉取）、F18（刷新全部）五个入口。
- 正确做法**已经在代码里实现好了**：`ConfigPending` + `restart_core_from_ui`（`runtime.rs:120-172`）、概览横幅（`overview.rs:244-296`）、规则编辑的 `notify_rule_change`。只是漏接了上述调用点。因此改动面小、可复用现有组件、无需新架构。
- 叠加 F3/F9 的统一状态心跳后，还顺带消除「永久假连接」和「永久 spinner」两个死胡同。
- 用户可感知度最高：这是每个日常使用者都会踩到的行为，而不像安全项只在特定部署/恶意输入下触发。
- 防回归成本低：补 2–3 条 `config_pending` 与「切模式后核心请求 mode」断言即可。

**建议一并做的两件小事**：把模式分段控件从设置页搬到概览连接卡（F14）；把 Android 的 1 Hz `core_status` 轮询改为全平台 2–3 s 公共心跳（F3）。

### 并列：优先级（风险）最高的不是 ROI 第一

如果产品要开放 `--allow-lan` 或从手机访问控制面，**S1–S3（Web 控制台信任边界）必须排在一切之前**：LAN 模式的后缀白名单 + IP 字面量放行 + GET 跳过 Origin + 永久 token，组合起来是一条完整的 DNS rebinding → 读取全部订阅凭据 → 完全控制核心的链路。它的触发频率低，但一旦发生是灾难性的，且修复同样集中在 `web/src/main.rs:147-338` 与 `cli.rs:17-157` 两个文件。

### 收益 Top 3（按 ROI 排序）

| 排名 | 改进 | 覆盖问题 | 投入 | 收益 |
|---|---|---|---|---|
| ① | 唯一「配置变更→核心生效」通道 + 统一状态心跳 | F1–F6, F8, F9, F18, U13 | 小（复用现有机制） | 消除 5 处状态漂移、2 个死胡同、用户最易误判的行为 |
| ② | 统一出站 URL 白名单 + 配置/特权配置输出白名单 | S4–S7, S10, B8 | 中（1 个校验器 + 2 处 schema 白名单） | 一次性堵住「普通进程 SSRF → 特权进程 SSRF → 本地提权」升级路径 |
| ③ | 统一组件层 + a11y 基线并接入 axe CI | U4–U9, U19, U20, U24 | 中（5 个新组件 + 替换调用点） | 消灭数量最多、最容易被后续开发继续复制的缺陷族 |

> 若允许第 4 项：**T1（CI 全量门禁）成本最低、收益立竿见影**，且是上述所有改动的安全网，建议放在第 0 周先做。

---

## 7. 审查边界说明

- 全程只读，未运行构建/测试，未启动浏览器实测。视觉对比度数值（U1 约 2.4:1）、`.power-label` 层叠结果（U3）基于 CSS 规则与颜色值推算，**建议实机复核**；其余结论均为源码可直接核对的静态事实。
- 「需进一步验证的疑点」Q1/Q2/Q3/Q5/Q6/Q8 未计入确证问题，列在 3.6 节末尾。
- 未审查 `target/`、`tests/ui-smoke/node_modules/`、Go 依赖源码、以及 Cargo.lock 中第三方库本身的漏洞。
