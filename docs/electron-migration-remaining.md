# Electron 迁移剩余工作

> 基线：`main` @ `6453bf6`（2026-10-22）。本次完成“更新、打包与发布”收尾。
>
> 迁移目标：Tauri 2 壳 → Electron 主进程 + `dae-core` N-API addon。
>
> 判定方法：把 `src/bindings.ts` 的 97 个生成命令逐项对照 `native/src` 与 `electron/main.cjs` 的 handler；Tauri 侧同名实现作为行为参考。本文列未完成项，刚收尾的关键管线保留在对应小节。

## 状态摘要

- 97 个生成命令已全部进入 handler：`start_drag_out` 由 Electron 主进程处理，macOS 的 `invoke_shell_command` 由主进程转交 addon 执行，其余都在 native addon；`terminal_*`、`read_directory_packets` 等不进 bindings 的通道命令也已实现。
- 文件系统、文件操作、终端、系统剪贴板、原生拖出、快捷方式、标签撕离、主题/强调色、设置、最近使用、搜索、目录大小、Git、回收站、压缩包、连接/云账户、收藏/空间、默认文件管理器均已落到 Electron 侧。
- 系统图标与缩略图管线已迁移：两个自定义协议、前端 URL、native 渲染层和 dae-core 平台后端均已接通，见下节。
- 窗口壳层已迁移：无边框窗口、拖拽区域、Windows 11 Snap Layout 与 macOS 生命周期均已落到 Electron 侧，见下节。
- macOS/Linux 平台能力已迁移：应用扩展右键菜单、卷变化监听、启动打开文件夹均已落到 Electron 侧，见下节。
- **更新、打包与发布已迁移**：NSIS / dmg+zip / AppImage+deb / MSIX 全部由 Electron 构建产出，`electron-updater` 在启动后后台检查更新，release CI 已从 `tauri-action` 换成 Electron 打包并上传更新元数据；`src/bindings.ts` 已入库，干净环境不再需要 Tauri 才能构建 Electron。见“已完成：更新、打包与发布”一节。
- 迁移已无 P1 缺口，剩余为 P2 次要项（`tab_perf_report`、日志轮转、启动预取）。

## 已完成：7 个命令

| 命令 | 实现位置 | 行为 |
| --- | --- | --- |
| `open_terminal` | `native/src/system_files.rs` | Windows 优先 `wt.exe`，否则新控制台 PowerShell；macOS 用 Terminal.app；Linux 依次尝试 `$TERMINAL`、`xdg-terminal-exec` 和常见终端 |
| `write_files_to_clipboard` / `read_files_from_clipboard` | `native/src/system_files.rs` | Windows 写读 `CF_HDROP` 与 Preferred DropEffect；其他平台按 Tauri 原有行为返回不支持 |
| `start_drag_out` | `electron/main.cjs` + `native/src/system_files.rs` | Windows 在主进程调用 native OLE 拖拽，保留 copy/move/link/any 效果；macOS/Linux 使用 Electron `startDrag`，效果由拖放目标按平台惯例决定 |
| `get_key_modifiers` | `native/src/system_files.rs` | Windows 读取全局 Ctrl/Shift/Alt/Win；其他平台返回默认值 |
| `create_shortcuts` | `native/src/system_files.rs` | Windows 创建 `.lnk`；macOS/Linux 创建符号链接，冲突名追加 ` (2)`、` (3)` |
| `get_package_family_name` | `native/src/system_files.rs` | Windows 查询 Package identity；未以 MSIX 安装时返回 `No package identity` |

`start_drag_out` 的 mode 在 Windows 上由 OLE 强制；macOS/Linux 的 Electron `startDrag` 不提供效果掩码，因此把 copy/move/link 交给拖放目标决定，这与 Finder/文件管理器的拖放语义一致，见“macOS / Linux 平台能力”一节。

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
- **Windows 应用身份。** 打包构建在 ready 后调用 `app.setAppUserModelId("com.melaine.dae")`，使快捷方式、任务栏分组和 toast 身份与安装器写入的 AppUserModelId 一致。

验收：无边框窗口四边可缩放、标签栏空白处可拖动窗口、最大化按钮悬停出 Snap Layout、macOS 红绿灯不重复。

## 已完成：macOS / Linux 平台能力

- **应用扩展右键菜单。** macOS Services 与 Linux KDE service menus 都已进入 `native/src/shell_commands/`，三个平台共用同一套 `ShellCommand` 行、选择判定与分组规则。
  - 平台无关的解析代码随迁移进入 native：`desktop_entry.rs`（`MimeType`/`Actions`/`Exec` 字段码展开）和 `plist.rs`（`NSServices`/UTI 与命令 id）。两者都在任意宿主上编译并有单元测试。
  - macOS 的发现、`Info.plist` 扫描和 UTI 匹配在 core 进程完成；`NSPerformService` 需要主进程的 `NSApplication` 和 UI 线程，因此 `electron/main.cjs` 在 `dae:invoke` 中拦截 `invoke_shell_command`（仅 darwin）：先同步调用 `resolveShellService` 校验 id，再 `setTimeout` 排队执行 `performShellService`，IPC 立即返回，不会被慢速 Automator 服务卡住。addon 侧的 `invoke` 明确拒绝在 core 中执行。
  - Linux 在 core 中直接按 `Exec` 展开并 `spawn`（不经 shell，避免文件名注入语义）；图标通过 `thumbnail`/`fileicon` 同一套主题渲染返回 `data:` URL。
- **卷变化监听。** `native/src/volumes.rs` 现在启动三个平台的后端：Windows `WM_DEVICECHANGE`、macOS `NSWorkspace` 的 mount/unmount/rename 通知、Linux GIO `VolumeMonitor`（专用线程 + GLib 主循环，转发到 core 的事件队列）。三者都发送 `volumes-changed`，侧栏按原有防抖重新读取磁盘列表。
- **启动时打开文件夹。** Windows/Linux 走启动参数与 `second-instance`，macOS 走 `open-file`；文件参数会被解析成所在目录，再复用深链入口，见窗口壳层一节。
- **平台行为差异（与 Tauri 一致）。** 文件剪贴板和全局按键查询仍只在 Windows 实现；macOS/Linux 上对应命令按原有行为返回不支持或默认值。
- **`start_drag_out` 的 mode 按平台委托。** Windows 由 native OLE 完整支持 copy/move/link/any；macOS/Linux 使用 Electron `startDrag`，该 API 不提供效果掩码，copy/move/link 由拖放目标按平台惯例和修饰键决定，与 Finder/文件管理器的原生拖放语义一致，不属于迁移缺口。若以后需要跨平台强制效果：macOS 可用原生 `NSDraggingSource` 掩码补齐（需先重做 macOS 修饰键映射），Linux 因 Electron 只暴露 X11 `Window`、没有可用的 GTK 窗口，暂无低成本方案。

## 已完成：更新、打包与发布

### 打包

- **electron-builder 目标。** `electron-builder.yml` 现在产出：
  - Windows：NSIS 安装包（`dae-<version>-x64-setup.exe`）+ blockmap；
  - macOS：dmg（首次安装）+ zip（Squirrel.Mac 更新产物），Apple Silicon；
  - Linux：AppImage（更新产物）+ deb（包管理器安装）。
- **应用图标与版本资源。** `win.icon` / `mac.icon` / `linux.icon` 指向已入库的 `src-tauri/icons/icon.ico|icns|png`，electron-builder 同时把图标和版本写进 exe/应用包；NSIS 使用非一键向导、可选安装目录、桌面与开始菜单快捷方式，并继续注册 `dae://` 协议。
- **生产依赖只含 electron-updater。** 渲染层依赖全部由 Vite 打进 `dist/`，因此原有前端包移入 `devDependencies`；`dependencies` 只剩 `electron-updater` 及其传递依赖。electron-builder 的 node_modules 收集因此不会把 React/xterm 等塞进 asar（实测 app.asar 4.2 MB，addon 与 core entry 保持 unpacked）。
- **MSIX。** `scripts/pack-msix.mjs` 把 `dist-electron/win-unpacked` 连同 `Assets/` 暂存到 `dist-electron/msix-layout`，再让 winapp 用根目录的 `Package.appxmanifest` 打包；`pack:msix` 在本地自动生成开发证书，CI 通过 `MSIX_CERT` / `MSIX_CERT_PASSWORD` 使用正式证书。`scripts/sync-msix-version.mjs` 以 `package.json` 为准同步四段版本号，Tauri 清单漂移只警告，因为 Electron 发布不再读取它们。

### 自动更新

- **`electron/updater.cjs`。** 打包构建在启动 3 秒后检查更新；`electron-updater` 自动下载，`update-downloaded` 时以静默模式安装并重启，与 Tauri `updater::spawn_startup_check` 的行为一致。开发构建、未打包运行不检查。
- **只在能自我替换的安装形态上检查。** MSIX 通过 `process.windowsStore` 识别并跳过（文件由包管理器拥有）；Linux 只在 `APPIMAGE` 环境变量存在时检查（deb 跳过）。
- **更新源使用 generic provider。** `electron-builder.yml` 的 publish 指向 `https://github.com/Melaine-iceberg/dae/releases/latest/download`，客户端请求平台对应的 `latest.yml` / `latest-mac.yml` / `latest-linux.yml`。没有继续用 electron-updater 的 GitHub provider：后者会先抓 `releases/latest` HTML 来解析 tag，在当前网络实测返回 406，并把整段 releases feed 写进错误；直链就是 Tauri 更新源使用的同一条路径。
- **日志可读。** 更新日志写入 `dae.log`，剥掉 electron-updater 附带的堆栈和 HTTP 响应体，只保留单行；`checkForUpdates()` 与 `error` 事件对同一次失败去重；debug 流仅在 `DAE_UPDATER_DEBUG=1` 时开启。
- **签名与更新链路。** Windows NSIS 即使未做 Authenticode，也会用 `latest.yml` 的 sha512 校验更新包；macOS Squirrel.Mac 必须使用已签名并公证的 bundle，因此 CI 在 `MAC_CSC_LINK` + `APPLE_*` 齐全时自动加 `-c.mac.notarize=true`，缺失时仍可产出可首次安装的未签名 dmg。Linux AppImage 原地更新，deb 交给包管理器。注意更新完整性模型与 Tauri 不同：Tauri 用 minisign 公钥直接校验更新包，electron-updater 没有等价物，`latest.yml` 的 sha512 与 GitHub TLS 是基线，签名后的安装包由操作系统再验一层；这条链路的信任假设写在这里，避免以后误以为还有 minisign。

### 发布 CI 与 bindings

- **release workflow 已换成 Electron。** `.github/workflows/release.yml` 不再调用 `tauri-action`：tag 先校验 `package.json`，三个平台各自安装依赖、构建 native addon 与渲染层，再 `electron-builder --publish never` 打包；Windows 额外跑 MSIX，最后把安装包、blockmap 和 `latest*.yml` 附到同一个 draft release。手工发布 draft 是唯一的上线动作，发布后 `releases/latest/download` 才会指向新元数据。
- **签名秘密全部可选。** `WIN_CSC_LINK` / `WIN_CSC_KEY_PASSWORD`（Windows Authenticode）、`MAC_CSC_LINK` / `MAC_CSC_KEY_PASSWORD`（Developer ID）、`APPLE_ID` / `APPLE_APP_SPECIFIC_PASSWORD` / `APPLE_TEAM_ID`（公证）、`MSIX_CERTIFICATE` / `MSIX_CERTIFICATE_PASSWORD`（MSIX）。CI 在调用 electron-builder 前 unset 空值，避免空 `CSC_LINK` 被当成证书路径。
- **bindings 已脱离构建依赖。** `src/bindings.ts` 移出 `.gitignore` 并入库（1445 行，由 `bindings:export` 重新生成）；`electron:dev` / `electron:pack` / release 不再要求先编译 `src-tauri`。macOS check workflow 新增 `bindings:check`：重新导出并 `git diff --exit-code`，防止命令 schema 改动后入库文件悄悄过期。

## P2：次要项

- `tab_perf_report` 只在 Tauri 侧实现（`src-tauri/src/tab_perf.rs`；前端 `src/lib/tab-perf.ts:62`），`DAE_TAB_PERF=1` 在 Electron 下不会出报告。
- 日志无轮转：Tauri 按 4 MB × 5 轮转，Electron 的 `core.log` / `dae.log` 是无限 append（`electron/main.cjs:1317`、`electron/core/core-entry.cjs:14`）。`electron/updater.cjs` 也写同一个 `dae.log`，长时间运行会把更新记录和 shell 日志放在一个无界文件里。
- 启动预取未迁移：Tauri 在 webview 加载时预热收藏/空间/最近使用（`src-tauri/src/file_system/prefetch.rs`），Electron 首屏会多走几次 IPC。

## 建议顺序

1. 日志轮转（`dae.log`、`core.log`），顺便给更新器日志分文件或打 tag。
2. 启动预取、`tab_perf_report` 等次要项。

## 验证记录

### 本次“更新、打包与发布”改动

- `bun install`：`electron-updater@6.8.9` 进入 `dependencies`，原前端依赖移入 `devDependencies`，锁文件更新。
- `node --check electron/main.cjs`、`node --check electron/updater.cjs`：通过。
- `npm run build`：通过（含 `tsc`）。
- `npx electron-builder --win --dir`、`npx electron-builder --win --publish never`：通过。产出 `dae-0.4.0-x64-setup.exe`（118,990,598 B）、`.exe.blockmap`、`latest.yml`；`win-unpacked/resources/app-update.yml` 为 `provider: generic` + `releases/latest/download`，asar 内包含 `electron/updater.cjs` 与 `node_modules/electron-updater`，addon / core-entry / 32px 图标位于 `app.asar.unpacked`。
- `scripts/pack-msix.mjs`：通过。`Package.appxmanifest` 版本由 `0.3.0.0` 同步到 `0.4.0.0`，产出 `dist-electron/dae-0.4.0-x64.msix`（约 165 MB），开发证书签名成功（证书未受信任，`Get-AuthenticodeSignature` 返回 `UnknownError` 属预期）。
- 打包应用实机启动（Windows 11 build 26340）：`win-unpacked/dae.exe` 正常出窗口，core utility process 启动，`dae.log` 出现更新器记录。
- 自动更新联调：把打包产物的 `app-update.yml` 临时指向本地 HTTP 源，源内放当前版本的 `latest.yml`；启动后日志为 `Update for version 0.4.0 is not available (latest version: 0.4.0...)` / `up to date (0.4.0)`，无多余堆栈。恢复正式源后对当前 GitHub 最新 release（仍是 Tauri 产物，无 `latest.yml`）得到单行 `Cannot find channel "latest.yml" update info: HttpError: 404`，属预期，首个 Electron release 发布后消失。
- `bun run bindings:export`：重新生成 `src/bindings.ts`（1445 行）并入库；`.gitignore` 移除该文件。

### 既有验证（迁移主体）

- `cargo check --manifest-path native/Cargo.toml`：通过（含 `resolveShellService` / `performShellService`）。
- `cargo check --manifest-path src-tauri/Cargo.toml`：通过（dae-core 的 `xdg::data_roots` 放宽为 `pub` 未破坏 Tauri 壳）。
- `cargo test --manifest-path native/Cargo.toml -- --test-threads=1`：89 passed，0 failed（新增 `desktop_entry` 服务菜单解析、`plist` 服务声明解析、Linux MIME 判定；两个 Windows Shell 图标测试在默认并行下偶发 Shell COM 互斥失败，串行运行稳定）。
- macOS 框架代码类型检查：Windows 上无法为 `aarch64-apple-darwin` 完成整 crate `cargo check`（`ring` 的 C 构建需要 Apple SDK），因此在临时校验 crate 中以 `#[path]` 引入真实的 `native/src/shell_commands/{plist,macos}.rs` 与 `native/src/volumes.rs`，对 `objc2`/`objc2-app-kit`/`objc2-foundation`/`objc2-uniform-type-identifiers`/`block2` 0.3/0.6 实际绑定做 `cargo check --target aarch64-apple-darwin`：通过。尚未在真实 macOS 上运行。
- Linux 卷监听尚未在 Linux 宿主编译：`linux.rs` 与 `desktop_entry.rs` 已在 Windows 的 `cfg(test)` 下编译并通过测试；GIO 后端逐项对照了 gio/glib 0.18 的 `VolumeMonitor` 信号与 `MainLoop` API。`.github/workflows/native-check.yml` 会在 Ubuntu 上编译整个 addon，仍需首次 CI 或真机验证。
- `npm run native:build`、`node --check electron/main.cjs`、`npm run build`、`npm run lint`：通过；加载 release addon 确认 `resolveShellService` / `performShellService` 已导出，Windows 上按设计返回 `fs.shell_command_not_macos`。
- `cargo test --manifest-path src-tauri/Cargo.toml trash`：4 passed，1 ignored，0 failed（schema 增加 `TrashEntry.path` 后仍通过）。
- `cargo test --manifest-path src-tauri/Cargo.toml preview::`：21 passed，0 failed（含新增“无版本缩略图不走 immutable 缓存”测试）。
- `cargo test --manifest-path src-tauri/crates/dae-core/Cargo.toml`：179 passed，7 ignored，0 failed。
- 命令覆盖：`src/bindings.ts` 的 97 个命令全部能在 `native/src` 或 `electron/main.cjs` 找到 handler（`git_sync` 与 fetch/push/pull 共用分支）。
- UI 回归（Windows，Electron 真实窗口）：切到“系统图标 + 列表缩略图”后，最近使用显示 Shell 文件夹/驱动器图标与 JPG 缩略图；内容搜索的分组头显示系统文件图标；属性对话框显示系统文件图标；回收站里的 PNG 显示缩略图，且 `TrashEntry.path` 指向回收站 payload；列表、网格和检查器中的缩略图/系统图标未回归。测试条目与临时偏好已清理。
- 窗口壳层构建验证：`cargo check --manifest-path native/Cargo.toml`、`npm run native:build`、`npm run build`、`npm run lint` 通过；加载 addon 确认 `installSnapLayout` / `destroySnapLayout` 存在。
- 窗口壳层实机验证（Windows 11 build 26340）：Electron 真实窗口截图确认系统标题栏与默认菜单栏均已消失；`EnumChildWindows` 确认 Snap Layout 的 `Static` 热区创建在最大化按钮上（窗口坐标 1369,46–1407,90）。

### 仍未验证

- 真实签名安装包与一次完整的“下载更新 → 静默安装 → 重启”闭环（本地只验证到更新源解析与版本判定；跨版本升级需要发布一个新 tag）。
- macOS 的 Developer ID 签名 / 公证、Squirrel.Mac 更新，以及 Windows Authenticode；均依赖仓库 secrets，首次带签名的 tag 才能验证。
- Linux 的 AppImage 更新与 deb 安装；以及 macOS / Linux 两个 CI 发布腿。
- MSIX 实际安装后的 `process.windowsStore` 跳过路径、包身份与 AppData 虚拟化行为。
- 无边框窗口的拖拽、四边缩放、最大化点击与 Snap Layout 飞出动画（交互验证被中断，需人工点验）；macOS 的红绿灯隐藏、`open-file`/`activate`、Services 列表与执行，Linux 的无边框缩放、KDE service menu 与 GIO 卷监听未在实机验证。替换文件后的缩略图刷新（沿用现有 mtime/size 规则）、含 PDF 的网格仍未验证。
