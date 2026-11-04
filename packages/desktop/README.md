# Development

## 用户登录自启动

原生桌面设置中的“用户登录时启动”默认关闭，读取系统启动项作为状态来源，不写入配置或同步快照，不在应用启动时自动注册。仅启动界面，不自动连接代理或开启 TUN；Web 和 Android 不提供此功能。

`desktop` feature 经 `ui/native-desktop` 传递到 `api/native-desktop`；自启模块及可选依赖仅在 macOS/Windows/Linux 桌面目标启用，Web/Android 不编译此模块。UI 保留异步 `status`/`set_enabled`，阻塞系统操作在后台任务执行。

Linux 直接使用 crates.io 上的 **`auto-launch = 0.6.0`**（XDG Autostart）。Windows 不使用该库：它的关闭和查询会访问 HKLM，普通用户会失败。macOS 也不使用该库写文件：它拼接 XML 时不转义路径。这两处由应用自己写当前用户的 Run 项和 LaunchAgent。不需要管理员权限。AppImage 使用 `APPIMAGE` 指向的真实文件；macOS 磁盘映像、AppTranslocation 和临时位置拒绝注册。

应用负责路径校验、Linux Exec 转义、Windows 命令引用和 255 字符长度限制。查询和关闭不要求当前安装路径仍然适合注册。macOS 在文件不存在时不调用 `launchctl`。设置失败后只有回读结果与操作目标一致才清除错误提示。

**限制**：Linux 只判断 desktop 文件是否存在，不解析 `Hidden`、桌面环境或系统目录里的同名项。关闭会删除用户项，系统目录里的同名项可能重新生效。Windows 只看当前用户的 Run 和 StartupApproved。未覆盖系统设置或第三方工具的所有禁用方式。尚需在各平台人工验证真实登录行为。

## macOS 菜单栏与剪贴板快捷键

macOS 的 WKWebView 自己不响应 `Cmd+C`/`Cmd+V`/`Cmd+A`：只有当某个菜单项占用这个键位、并把 `copy:`/`paste:` 等 selector 发给 first responder 时，输入框才能复制粘贴。`Cmd+Q` 同理，它靠 `terminate:` 生效。此前桌面端用 `.with_menu(None)` 启动，安装后没有任何菜单栏，这些快捷键全部失效，`Cmd+Q` 也无处可去。

配置里三个平台统一 `.with_menu(None)`（这同时挡住 Dioxus 默认菜单栏——它的 `Cmd+Q` 是裸 `terminate:`）。macOS 在根组件第一次 `use_effect` 里自己装一条最小菜单栏：应用菜单（关于/服务/隐藏/最小化/退出）、编辑菜单（撤销/重做/剪切/拷贝/粘贴/全选）、窗口菜单。编辑菜单是剪贴板能用起来的唯一原因，不能删。Windows 和 Linux 到此为止：WebView2 与 WebKitGTK 在页面内自己处理这些键位，不需要菜单。

**菜单栏不能在 `main()` 组装 `Config` 时建。** `PredefinedMenuItem::services` 在 append 的瞬间就发 `+[NSApplication sharedApplication]`，而 tao 要求自己是第一个发这个消息的人（见 `tao::platform_impl::macos::event_loop`），否则 AppKit 会实例化原生 `NSApplication`，tao 那个重写 `sendEvent:` 的子类就永远装不上——`Cmd` 组合键的 `keyUp` 不再送达 WebView。effect 在事件循环内运行，既晚于 `EventLoop::new()`，也晚于 `NSApp.mainMenu` 就位，所以是唯一安全的时机；`MenuEvent` 接收器在这之前已经接好，菜单点击照常作为 muda 事件送达。

`Cmd+Q` 绑到“最小化”而不是退出：窗口关闭本来只是隐藏到托盘，`terminate:` 会直接杀掉正在运行的 sing-box 内核，绕过清理。真正的退出放在 `Opt+Cmd+Q`，复用托盘“退出”的同一个 id，先 `shutdown_native_runtime()` 再结束进程。窗口已经隐藏时再按 `Cmd+Q` 会把窗口显示出来，避免按键静默无效。

所有原生平台都会注入一段捕获阶段的 `contextmenu` 监听：Dioxus 在 release 构建里用初始化脚本注册了全局 `preventDefault()` 来藏调试菜单，顺带也移除了 WebView 自带的编辑菜单——输入框右键没有剪切/拷贝/粘贴，日志里选中文本右键也拷不出来。脚本只对“有选区”或“落在可编辑元素上”的右键 `stopPropagation()`，页面其它位置仍然没有右键菜单。

必须是捕获阶段。Dioxus 的抑制脚本在 document start 执行（早于任何页面脚本），但它是冒泡阶段监听；同一个节点上捕获先于冒泡，所以这里靠阶段取胜，而不是靠注册顺序。

muda 的菜单项只能在主线程创建，所以整条菜单栏、包括 `set_as_windows_menu_for_nsapp()`，都在这一个 effect 里一次做完；菜单栏本身存进 thread local 保持存活（muda 的 `Menu` 一 drop 原生菜单就没了）。键位绑定抽成 `quit_key_bindings()` 数据形式，单元测试才能在不动 muda 的前提下钉住它。

The desktop crate defines the entrypoint for the desktop app along with any assets, components and dependencies that are specific to desktop builds. The desktop crate starts out something like this:

```
desktop/
├─ assets/ # Assets used by the desktop app - Any platform specific assets should go in this folder
├─ src/
│  ├─ main.rs # The entrypoint for the desktop app.It also defines the routes for the desktop platform
│  ├─ views/ # The views each route will render in the desktop version of the app
│  │  ├─ mod.rs # Defines the module for the views route and re-exports the components for each route
│  │  ├─ blog.rs # The component that will render at the /blog/:id route
│  │  ├─ home.rs # The component that will render at the / route
├─ Cargo.toml # The desktop crate's Cargo.toml - This should include all desktop specific dependencies
```

## Dependencies
Since you have fullstack enabled, the desktop crate will be built two times:
1. Once for the server build with the `server` feature enabled
2. Once for the client build with the `desktop` feature enabled

You should make all desktop specific dependencies optional and only enabled in the `desktop` feature. This will ensure that the server builds don't pull in desktop specific dependencies which cuts down on build times significantly.

### Serving Your Desktop App

You can start your desktop app with the following command:

```bash
dx serve
```
