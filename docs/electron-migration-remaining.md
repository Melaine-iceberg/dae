# Electron 迁移剩余工作

> 基线：`main` @ `bf5b281`（2026-10-22）
>
> 迁移目标：Tauri 2 壳 → Electron 主进程 + `dae-core` N-API addon。
>
> 判定方法：把 `src/bindings.ts` 的 97 个生成命令逐项对照 `native/src` 与 `electron/main.cjs` 的 handler；Tauri 侧同名实现作为行为参考。本文只列未完成项。

## 状态摘要

- 97 个生成命令中已有 90 个进入 handler；`terminal_*`、`read_directory_packets` 等不进 bindings 的通道命令也已实现。
- 文件系统、文件操作、终端、标签撕离、主题/强调色、设置、最近使用、搜索、目录大小、Git、回收站、压缩包、连接/云账户、收藏/空间、默认文件管理器均已落到 Electron 侧。
- 剩余工作集中在：**7 个命令、系统图标/缩略图、窗口壳层、macOS/Linux 平台能力、更新与打包发布**。

## P0：用户日常会直接撞到

### 1. 仍有 7 个命令没有 handler

这些命令会落到 `native/src/lib.rs` 的兜底分支，报 `unknown command`。命令名来自 `src/bindings.ts`，调用点都在实际 UI 路径上。

| 命令 | 前端调用点 | 影响 | Tauri 参考实现 |
| --- | --- | --- | --- |
| `open_terminal` | `src/features/explorer/entry-context-menu.tsx:406`、`explorer-path-bar.tsx:262`、`explorer-view.tsx:456` | 右键或路径栏“在系统终端中打开”失效 | `src-tauri/src/file_system/commands.rs:816` |
| `write_files_to_clipboard` | `src/features/explorer/use-explorer-clipboard.ts:27` | 无法把文件放到系统剪贴板，Explorer 等应用收不到粘贴内容 | `src-tauri/src/file_system/system_files.rs:23` |
| `read_files_from_clipboard` | `src/features/explorer/use-explorer-clipboard.ts:80` | 无法读取 Explorer 等应用复制的文件，只能使用应用内镜像 | `src-tauri/src/file_system/system_files.rs:41` |
| `start_drag_out` | `src/features/explorer/file-list.tsx:598` | 把文件拖出窗口到其他应用时没有原生拖拽 | `src-tauri/src/file_system/system_files.rs:117` |
| `get_key_modifiers` | `src/features/explorer/use-explorer-events.ts:182`、`:252` | 外部拖入时无法感知 Ctrl/Shift，只能按同卷移动、跨卷复制 | `src-tauri/src/file_system/system_files.rs:81` |
| `create_shortcuts` | `src/features/explorer/use-transfers.ts:140` | “创建快捷方式/链接”失败 | `src-tauri/src/file_system/system_files.rs:137` |
| `get_package_family_name` | `src/features/settings/settings-dialog.tsx:656` | 关于页身份行报错；只有 MSIX 安装才有意义，和打包一起处理 | `src-tauri/src/lib.rs:63` |

建议把前六个一起做：它们都属于系统 Shell 集成，Windows 侧可复用 `system_files.rs`，macOS/Linux 再按平台补齐。

### 2. 系统图标与缩略图管线

- **`fileicon://` 协议没有注册。** Electron 目前只注册了 `dae` 和 `thumbnail`（`electron/main.cjs:27`、`:849`）；Tauri 的 `fileicon` 在 `src-tauri/src/file_system/preview.rs:323`。所有使用系统图标的行都会退回绘制图标。
- **Windows 上的 URL 会被绕过。** 前端仍按 Tauri/WebView2 的映射请求 `http://thumbnail.localhost` / `http://fileicon.localhost`（`src/features/explorer/thumbnail.tsx:110`、`native-icon.tsx:26`），但 Electron 主进程没有拦截这两个 HTTP host。按当前代码，Windows 上连已注册的 `thumbnail` 协议也不会命中。
- **原生缩略图只覆盖一部分图片。** `native/src/lib.rs:168` 直接 `image::open`，`native/Cargo.toml:18` 只启用 jpeg/png/webp；前端认为可用的 GIF、BMP、TIFF、ICO 也会失败。
- **桌面缩略图生产能力未迁移。** SVG 直传、PDF 首页、视频首帧、HEIC/AVIF，以及 Linux `.thumbnailer`、Windows Shell 缩略图、macOS QuickLook 都没有对应实现；Tauri 侧在 `src-tauri/src/file_system/preview.rs`、`src-tauri/src/desktop_thumbnails.rs`。
- **系统图标提取未迁移。** Tauri 的 `src-tauri/src/file_icons.rs` 及三个平台后端还没有进入 native addon。
- **协议 CORS 需一起验证。** `thumbnail` scheme 没有声明 `corsEnabled`，而前端以 `crossOrigin="anonymous"` 请求；主进程虽然回写了 `Access-Control-Allow-Origin`，跨源是否放行仍需实测。

验收：在 Windows 上连续浏览含 JPG/PNG/SVG/PDF/MP4 的目录，网格应出缩略图；含 `.exe`/`.lnk`/未知扩展名的目录应出系统图标；替换文件后缩略图应随 mtime/size 版本刷新。

## P1：窗口壳层

- **窗口不是无边框。** Tauri 配置是 `decorations: false`（`src-tauri/tauri.conf.json:19`），前端自绘窗口控件（`src/components/window-controls.tsx:14`）。Electron 的 `createWindow` 没有 `frame: false` 或 `titleBarStyle`（`electron/main.cjs:1086`），当前会出现系统标题栏与自绘控件并存；macOS 上还会多一套原生红绿灯。
- **没有拖拽区域实现。** 前端仍使用 `data-tauri-drag-region`（如 `src/features/explorer/explorer-tabs.tsx:489`、`explorer-toolbar.tsx:175`），Electron 侧没有对应的 `-webkit-app-region` 或 `startDragging` IPC。
- **没有 Windows 11 Snap Layout。** Tauri 用 `tauri-plugin-snap-layout` 挂在 `#window-maximize`（`src-tauri/src/lib.rs:364`；按钮 id 见 `window-controls.tsx:183`），Electron 需要另行接入或明确放弃。
- **macOS 生命周期不完整。** 主进程只处理了 `second-instance` 和 `open-url`（`electron/main.cjs:1214`），没有 `open-file`/`activate`；Finder 以 dae 打开文件夹时没有入口。

验收：无边框窗口四边可缩放、标签栏空白处可拖动窗口、最大化按钮悬停出 Snap Layout、macOS 红绿灯不重复。

## P1：macOS / Linux 平台能力

- **应用扩展右键菜单为空。** `native/src/shell_commands.rs:5` 明确说明 macOS Services 和 Linux KDE service menus 仍留在 Tauri 壳；Electron 侧固定返回空列表。Tauri 的 `src-tauri/src/shell_commands/macos.rs`、`linux.rs` 可作参考。
- **卷变化监听只有 Windows。** `native/src/volumes.rs:15` 只启动了 Windows 的 `WM_DEVICECHANGE`；Tauri 有 Linux GIO 和 macOS NSWorkspace 实现（`src-tauri/src/file_system/volume_monitor.rs:21`）。macOS/Linux 插拔 U 盘不会即时刷新侧栏。
- **打开文件夹的入口。** 与上面的 macOS `open-file` 一起补；Windows 目前通过启动参数和 `second-instance` 正常处理。

## P1：更新、打包与发布

- **自动更新缺失。** Tauri release 使用 `tauri-plugin-updater` 和 `src-tauri/src/updater.rs`；Electron 没有 electron-updater、更新源或签名配置。
- **Electron 打包只有 Windows `dir`。** `electron-builder.yml:36` 与 `package.json:26` 都只做 `--win --dir`；没有安装包、应用图标、签名、MSIX 和包身份。`get_package_family_name` 和关于页身份行依赖这里一起解决。
- **CI 仍是 Tauri。** `.github/workflows/release.yml:94` 使用 `tauri-action`，macOS 检查也跑 Tauri。
- **构建仍依赖 Tauri 生成 bindings。** `src/bindings.ts` 被 gitignore（`.gitignore:38`），只能通过 `bindings:export` 从 `src-tauri` 生成（`package.json:22`）。干净环境下直接跑 `electron:dev` 会缺少该文件，说明 Electron 还没脱离 Tauri 的 schema 源。
- **文档仍是 Tauri 优先。** `README.md:3` 还把 Tauri 写成 shell，开发/发布说明也没有 Electron 路径。

建议把“打包 + 更新 + bindings 来源”作为一个整体收尾，不要分开做：它们都依赖发布产物和签名链路。

## P2：次要项

- `tab_perf_report` 只在 Tauri 侧实现（`src-tauri/src/tab_perf.rs`；前端 `src/lib/tab-perf.ts:62`），`DAE_TAB_PERF=1` 在 Electron 下不会出报告。
- 日志无轮转：Tauri 按 4 MB × 5 轮转，Electron 的 `core.log` / `dae.log` 是无限 append（`electron/main.cjs:1071`、`electron/core/core-entry.cjs:14`）。
- 启动预取未迁移：Tauri 在 webview 加载时预热收藏/空间/最近使用（`src-tauri/src/file_system/prefetch.rs`），Electron 首屏会多走几次 IPC。
- `native/src/shell.rs` 顶部注释还写着 workspace 列表未迁移，已过时，可顺手清理。

## 建议顺序

1. 七个命令（先 Windows，再补 macOS/Linux）→ 系统图标与缩略图协议。
2. 无边框窗口、拖拽区域、Snap Layout、macOS 生命周期。
3. 打包/更新/CI/bindings 来源收尾。
4. macOS/Linux 应用扩展、卷监听等平台差异。
5. 日志轮转、启动预取、tab-perf 等次要项。

## 验证记录

- `cargo check --manifest-path native/Cargo.toml`：通过。
- `cargo test --manifest-path native/Cargo.toml`：46 passed，0 failed。
- 命令缺口来自静态比对，尚未对 7 个缺失命令做 UI 回归；macOS/Linux 相关项未在实机验证。
