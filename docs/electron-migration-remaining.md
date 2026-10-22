# Electron 迁移剩余工作

> 基线：`main` @ `bf5b281`（2026-10-22），并包含工作区中尚未提交的 7 命令补齐。
>
> 迁移目标：Tauri 2 壳 → Electron 主进程 + `dae-core` N-API addon。
>
> 判定方法：把 `src/bindings.ts` 的 97 个生成命令逐项对照 `native/src` 与 `electron/main.cjs` 的 handler；Tauri 侧同名实现作为行为参考。本文列未完成项，刚收尾的关键管线保留在对应小节。

## 状态摘要

- 97 个生成命令已全部进入 handler：除 `start_drag_out` 由 Electron 主进程处理外，其余都在 native addon；`terminal_*`、`read_directory_packets` 等不进 bindings 的通道命令也已实现。
- 文件系统、文件操作、终端、系统剪贴板、原生拖出、快捷方式、标签撕离、主题/强调色、设置、最近使用、搜索、目录大小、Git、回收站、压缩包、连接/云账户、收藏/空间、默认文件管理器均已落到 Electron 侧。
- 系统图标与缩略图管线已迁移：两个自定义协议、前端 URL、native 渲染层和 dae-core 平台后端均已接通，见下节。
- 窗口壳层已迁移：无边框窗口、拖拽区域、Windows 11 Snap Layout 与 macOS 生命周期均已落到 Electron 侧，见下节。
- 剩余工作集中在：**macOS/Linux 平台能力、更新与打包发布**。

## 已完成：7 个命令

| 命令 | 实现位置 | 行为 |
| --- | --- | --- |
| `open_terminal` | `native/src/system_files.rs` | Windows 优先 `wt.exe`，否则新控制台 PowerShell；macOS 用 Terminal.app；Linux 依次尝试 `$TERMINAL`、`xdg-terminal-exec` 和常见终端 |
| `write_files_to_clipboard` / `read_files_from_clipboard` | `native/src/system_files.rs` | Windows 写读 `CF_HDROP` 与 Preferred DropEffect；其他平台按 Tauri 原有行为返回不支持 |
| `start_drag_out` | `electron/main.cjs` + `native/src/system_files.rs` | Windows 在主进程调用 native OLE 拖拽，保留 copy/move/link/any 效果；macOS/Linux 使用 Electron `startDrag`，由目标决定效果 |
| `get_key_modifiers` | `native/src/system_files.rs` | Windows 读取全局 Ctrl/Shift/Alt/Win；其他平台返回默认值 |
| `create_shortcuts` | `native/src/system_files.rs` | Windows 创建 `.lnk`；macOS/Linux 创建符号链接，冲突名追加 ` (2)`、` (3)` |
| `get_package_family_name` | `native/src/system_files.rs` | Windows 查询 Package identity；未以 MSIX 安装时返回 `No package identity` |

`start_drag_out` 在 macOS/Linux 暂未映射 mode 参数（Electron `startDrag` 不提供效果掩码），这是仅剩的行为差异，不影响拖出本身。

## 已完成：系统图标与缩略图管线

- **两个自定义协议都已注册。** `electron/main.cjs` 把 `thumbnail` 和 `fileicon` 声明为 standard/secure/CORS-enabled scheme，并都由 `registerCoreProtocol` 转发到 core 的本地 HTTP 端点（`/thumb`、`/icon`）。`fileicon` 之前完全缺失。
- **URL 在 Electron 下不再走 WebView2 映射。** `src/features/explorer/thumbnail.tsx` 与 `native-icon.tsx` 通过 `isElectronShell` 区分壳层：Electron 请求 `thumbnail://localhost` / `fileicon://localhost`，Tauri on Windows 仍保留 `http://*.localhost` 映射。
- **响应携带真实 MIME 与状态。** core 按渲染结果返回 `image/png`、`image/jpeg` 或 `image/svg+xml`；无生产者/文件消失返回 404，读取或解码失败返回 500。主进程补齐 `Access-Control-Allow-Origin`，与 `corsEnabled` 一起满足 `crossOrigin="anonymous"` 的加载。
- **native 缩略图覆盖前端声明的格式。** `native/src/thumbnails.rs` 支持 jpeg/png/gif/webp/bmp/tiff/ico 的进程内解码、SVG 字节直传、64 MB 源上限和 40 MP 解码上限；超过像素上限的图片改走桌面生产者。
- **桌面缩略图生产者已迁移。** Windows 使用 `IShellItemImageFactory`（PDF 首页、视频首帧、HEIC，均取决于机器是否注册了对应处理器），Linux 使用 freedesktop `.thumbnailer` 与 ffmpeg 兜底；macOS 与 Tauri 现状一致，没有 QuickLook 生产者，PDF/视频/HEIC 会 404 回退到类型图标。
- **系统图标提取已迁移。** `dae-core` 的 `file_icons` 现在接线为共享模块：Windows 路径图标走 Shell item，类型图标先用 `SHDefExtractIconW`、失败时回退 `SHGetFileInfoW + SHGFI_USEFILEATTRIBUTES`；macOS 走 `NSWorkspace`，Linux 走 XDG 图标主题。路径图标、按扩展名的共享类型图标和 freedesktop 图标名都已暴露给 native。
- **渲染不占 Node / 列表线程。** `native/src/render_pool.rs` 使用 3 个专用线程；Windows 线程进入 STA 并在任务之间泵消息，避免 Shell 缩略图提供程序回调死锁。
- **前端已收敛到同一个入口。** `EntryArtwork` 统一决定“缩略图 > 系统图标 > 类型字形”，`RowEntryIcon`、网格、检查器，以及最近使用、概览、内容搜索、回收站、属性对话框和传输冲突对话框都走它；图标样式与列表缩略图偏好因此对所有文件视图一致生效。标签条、面包屑、侧栏树和空间卡片仍使用内置字形：它们是导航外壳，不是文件内容。
- **回收站条目带上了可读取的 payload 路径。** `TrashEntry.path` 在 Windows/macOS 上就是回收站内的真实路径，在 freedesktop 上由 `.trashinfo` 文件名推导出同名 `files/` 条目；`fileicon://` / `thumbnail://` 因此也能画出回收站里文件夹的图标和文件的缩略图。
- **无版本缩略图不再冒充不可变。** 最近使用、内容搜索和回收站目录没有 mtime/size 可做 URL 版本；Electron core 与 Tauri 协议对不带 `v` 的缩略图都回 `no-store`，带版本的列表/网格缩略图仍保持一天 immutable 缓存。

验收：在 Windows 上连续浏览含 JPG/PNG/SVG/PDF/MP4 的目录，网格应出缩略图；含 `.exe`/`.lnk`/未知扩展名的目录应出系统图标；替换文件后缩略图应随 mtime/size 版本刷新。把菜单切到“系统图标”后，最近使用、内容搜索、属性、回收站与冲突对话框也应显示同一套 OS 图标；回收站中已知大小的文件行在开启“列表缩略图”后应出缩略图。

## 已完成：窗口壳层

- **无边框窗口。** Windows/Linux 使用 `frame: false`；macOS 使用 `titleBarStyle: "hidden"` 并在创建后调用 `setWindowButtonVisibility(false)`，保留原生圆角、阴影和全屏过渡，但不再出现第二套红绿灯。Windows/Linux 移除了 Electron 默认应用菜单，避免系统菜单栏出现在无边框窗口内；macOS 保留全局菜单，标准 Edit/Quit 角色仍然可用。
- **拖拽区域。** 渲染层在 Electron 壳写入 `<html data-shell="electron">`，`App.css` 把 Tauri 的 `data-tauri-drag-region="deep"` / `"false"` 映射为 `-webkit-app-region: drag` / `no-drag`；按钮、输入框、标签等交互元素按 Tauri `drag.js` 的同一套规则自动退出拖拽区。映射只在 Electron 生效，Tauri/WebView2 的行为不变。
- **Windows 11 Snap Layout。** `native/src/snap_layout.rs` 在最大化按钮上创建一个透明 `STATIC` 子窗口，命中测试返回 `HTMAXBUTTON`，点击通过 `WM_SYSCOMMAND` 切换最大化；主进程新增 `dae:window:snap-bounds` 接收按钮矩形，以 60ms 指针轮询产生 `tauri-snap://snap/mouseenter|mouseleave`，渲染层据此镜像 `.is-hovered`（原生覆盖层会吞掉 DOM 的 `:hover`）。仅 Windows 11（build ≥ 22000）启用。
- **macOS 生命周期。** 补上 `open-file`（文件解析为所在目录后复用深链入口）与 `activate`；关闭最后一个窗口后应用不再退出，Dock 激活时重建窗口并重新预热撕标签池。

验收：无边框窗口四边可缩放、标签栏空白处可拖动窗口、最大化按钮悬停出 Snap Layout、macOS 红绿灯不重复。

## P1：macOS / Linux 平台能力

- **应用扩展右键菜单为空。** `native/src/shell_commands.rs:5` 明确说明 macOS Services 和 Linux KDE service menus 仍留在 Tauri 壳；Electron 侧固定返回空列表。Tauri 的 `src-tauri/src/shell_commands/macos.rs`、`linux.rs` 可作参考。
- **卷变化监听只有 Windows。** `native/src/volumes.rs:15` 只启动了 Windows 的 `WM_DEVICECHANGE`；Tauri 有 Linux GIO 和 macOS NSWorkspace 实现（`src-tauri/src/file_system/volume_monitor.rs:21`）。macOS/Linux 插拔 U 盘不会即时刷新侧栏。
- **`start_drag_out` 的 mode 未映射。** macOS/Linux 走 Electron `startDrag`，只能拖出文件，不能强制 copy/move/link；Windows 已由 native OLE 完整支持。
- **打开文件夹的入口。** 与上面的 macOS `open-file` 一起补；Windows 目前通过启动参数和 `second-instance` 正常处理。
- **平台行为差异（与 Tauri 一致）。** 文件剪贴板和全局按键查询仍只在 Windows 实现；macOS/Linux 上对应命令按原有行为返回不支持或默认值。

## P1：更新、打包与发布

- **自动更新缺失。** Tauri release 使用 `tauri-plugin-updater` 和 `src-tauri/src/updater.rs`；Electron 没有 electron-updater、更新源或签名配置。
- **Electron 打包只有 Windows `dir`。** `electron-builder.yml:36` 与 `package.json:26` 都只做 `--win --dir`；没有安装包、应用图标、签名、MSIX 和包身份。`get_package_family_name` 已能用，但要安装成 MSIX 才会返回真实 family name。
- **CI 仍是 Tauri。** `.github/workflows/release.yml:94` 使用 `tauri-action`，macOS 检查也跑 Tauri。
- **构建仍依赖 Tauri 生成 bindings。** `src/bindings.ts` 被 gitignore（`.gitignore:38`），只能通过 `bindings:export` 从 `src-tauri` 生成（`package.json:22`）。干净环境下直接跑 `electron:dev` 会缺少该文件，说明 Electron 还没脱离 Tauri 的 schema 源。
- **文档仍是 Tauri 优先。** `README.md:3` 还把 Tauri 写成 shell，开发/发布说明也没有 Electron 路径。

建议把“打包 + 更新 + bindings 来源”作为一个整体收尾，不要分开做：它们都依赖发布产物和签名链路。

## P2：次要项

- `tab_perf_report` 只在 Tauri 侧实现（`src-tauri/src/tab_perf.rs`；前端 `src/lib/tab-perf.ts:62`），`DAE_TAB_PERF=1` 在 Electron 下不会出报告。
- 日志无轮转：Tauri 按 4 MB × 5 轮转，Electron 的 `core.log` / `dae.log` 是无限 append（`electron/main.cjs:1131`、`electron/core/core-entry.cjs:14`）。
- 启动预取未迁移：Tauri 在 webview 加载时预热收藏/空间/最近使用（`src-tauri/src/file_system/prefetch.rs`），Electron 首屏会多走几次 IPC。
- `native/src/shell.rs` 顶部注释还写着 workspace 列表未迁移，已过时，可顺手清理。

## 建议顺序

1. 打包/更新/CI/bindings 来源收尾。
2. macOS/Linux 应用扩展、卷监听、`start_drag_out` mode 等平台差异。
3. 日志轮转、启动预取、tab-perf 等次要项。

## 验证记录

- `cargo check --manifest-path native/Cargo.toml`：通过。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过（dae-core 接线未破坏 Tauri 壳）。
- `cargo test --manifest-path native/Cargo.toml`：59 passed，0 failed（新增缩略图/图标路由、格式覆盖、Windows 图标与 freedesktop 回收站 payload 路径测试）。
- `cargo test --manifest-path src-tauri/Cargo.toml trash`：4 passed，1 ignored，0 failed（schema 增加 `TrashEntry.path` 后仍通过）。
- `cargo test --manifest-path src-tauri/Cargo.toml preview::`：21 passed，0 failed（含新增“无版本缩略图不走 immutable 缓存”测试）。
- `cargo test --manifest-path src-tauri/crates/dae-core/Cargo.toml`：179 passed，7 ignored，0 failed。
- `npm run build`、`npm run lint`、`npm run bindings:export`：通过。
- N-API 快速验证：直接加载 addon 确认 `thumbnail`、`fileIcon`、`typeIcon`、`namedIcon` 可调用；PNG 缩略图返回 `image/png`，`notepad.exe` 返回 Shell 图标，`ext=exe` 返回类型图标，未知路径返回 `null`。
- 真实 Electron 协议验证：在 `dae://localhost` 渲染进程内请求 `thumbnail://` / `fileicon://`——PNG、SVG（`image/svg+xml` 直传）、ICO、MP4 首帧、`notepad.exe` 路径图标和 `ext=exe` 类型图标均返回 200；不存在的路径返回 404；响应带 `Access-Control-Allow-Origin: *`，`<img crossOrigin="anonymous">` 成功加载 64×64。
- 命令覆盖：`src/bindings.ts` 的 97 个命令全部能在 `native/src` 或 `electron/main.cjs` 找到 handler（`git_sync` 与 fetch/push/pull 共用分支）。
- UI 回归（Windows，Electron 真实窗口）：切到“系统图标 + 列表缩略图”后，最近使用显示 Shell 文件夹/驱动器图标与 JPG 缩略图；内容搜索的分组头显示系统文件图标；属性对话框显示系统文件图标；回收站里的 PNG 显示缩略图，且 `TrashEntry.path` 指向回收站 payload；列表、网格和检查器中的缩略图/系统图标未回归。测试条目与临时偏好已清理。
- 窗口壳层构建验证：`cargo check --manifest-path native/Cargo.toml`、`npm run native:build`、`npm run build`、`npm run lint` 通过；加载 addon 确认 `installSnapLayout` / `destroySnapLayout` 存在。
- 窗口壳层实机验证（Windows 11 build 26340）：Electron 真实窗口截图确认系统标题栏与默认菜单栏均已消失；`EnumChildWindows` 确认 Snap Layout 的 `Static` 热区创建在最大化按钮上（窗口坐标 1369,46–1407,90）。
- 仍未验证：无边框窗口的拖拽、四边缩放、最大化点击与 Snap Layout 飞出动画（交互验证被中断，需人工点验）；macOS 的红绿灯隐藏、`open-file`/`activate` 与 Linux 无边框缩放未在实机验证。替换文件后的缩略图刷新（URL 版本逻辑未改，沿用现有 mtime/size 规则）、含 PDF 的网格（本机未注册 PDF 缩略图处理器，会按设计回退到类型图标）仍未验证。
