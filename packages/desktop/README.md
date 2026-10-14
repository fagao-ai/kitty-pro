# Development

## 用户登录自启动

原生桌面设置中的“用户登录时启动”默认关闭，读取系统启动项作为状态来源，不写入配置或同步快照，不在应用启动时自动注册。仅启动界面，不自动连接代理或开启 TUN；Web 和 Android 不提供此功能。

`desktop` feature 经 `ui/native-desktop` 传递到 `api/native-desktop`；自启模块及可选依赖仅在 macOS/Windows/Linux 桌面目标启用，Web/Android 不编译此模块。UI 保留异步 `status`/`set_enabled`，阻塞系统操作在后台任务执行。

Linux 直接使用 crates.io 上的 **`auto-launch = 0.6.0`**（XDG Autostart）。Windows 不使用该库：它的关闭和查询会访问 HKLM，普通用户会失败。macOS 也不使用该库写文件：它拼接 XML 时不转义路径。这两处由应用自己写当前用户的 Run 项和 LaunchAgent。不需要管理员权限。AppImage 使用 `APPIMAGE` 指向的真实文件；macOS 磁盘映像、AppTranslocation 和临时位置拒绝注册。

应用负责路径校验、Linux Exec 转义、Windows 命令引用和 255 字符长度限制。查询和关闭不要求当前安装路径仍然适合注册。macOS 在文件不存在时不调用 `launchctl`。设置失败后只有回读结果与操作目标一致才清除错误提示。

**限制**：Linux 只判断 desktop 文件是否存在，不解析 `Hidden`、桌面环境或系统目录里的同名项。关闭会删除用户项，系统目录里的同名项可能重新生效。Windows 只看当前用户的 Run 和 StartupApproved。未覆盖系统设置或第三方工具的所有禁用方式。尚需在各平台人工验证真实登录行为。

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
