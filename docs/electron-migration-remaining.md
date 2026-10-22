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
- **标签页拖拽已改成 HTML5 拖放**：跨窗口合并与撕离走 Chromium 的拖拽循环，X11/XWayland 与 Windows/macOS 可用；**原生 Wayland 真机鼠标拖拽仍不可用**（合成输入可跑通），与 VS Code 同类问题，见“已完成：标签页拖拽改为 HTML5 拖放”一节。主进程的 hover 会话与原生拖拽命令已删除。
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

## 已完成：标签页拖拽改为 HTML5 拖放（原生 Wayland 受 Chromium 限制）

**问题。** Tauri 侧标签拖拽是自绘手势 + 平台原生拖拽会话（Windows OLE、macOS `NSDraggingSession`、Linux GTK `gtk_drag_begin`）。Electron 三条腿只剩两条：`webContents.startDrag` 只能携带文件、不能带自定义 MIME，Wayland 下还拒绝不了全局坐标；而 Linux 的 `wait_for_tab_drag_release` 在 `native/src/file_ops/mod.rs` 里本来就恒为 `false`（没有进程级按键状态），所以 Electron 的 Linux 分支只能“一离开窗口就撕离”，merge 与插入指示器根本不会发生，`screen.getCursorScreenPoint()` 在原生 Wayland 上还是官方不支持项。

**做法。** 把跨窗口那半段交给 Chromium 自己的拖放：标签芯片 `draggable`，`dragstart` 写入自定义 MIME `application/x-dae-tab`（JSON：`source` 窗口 label、`token`、`payload` = `serializeTabHandoff`），平台拖拽循环负责跨窗口投递、拖拽图与拖拽光标；窗口级的 `dragover`/`drop` 负责合并与落点。这是理论上唯一能在 Wayland 下跨窗口的机制——指针在拖拽期间被合成器抓走，兄弟窗口收不到任何 pointer 事件。

> **已知限制：原生 Wayland 下真机鼠标仍拖不动。** WSLg/Weston + `--ozone-platform=wayland` 实测：真鼠标按下拖动无效（用户实测）；用 CDP 合成鼠标事件时 `dragstart`/`dragover`/实时重排能跑通，说明渲染层接线是好的，断点在 Chromium 的 Wayland 拖拽源（合成器不回报 action，Chromium 把 drop 当取消）。VS Code 的“Wayland 上完全拖不动”（#156723）2024-11-28 已以上游修复关闭，但标签跨窗口拖拽同样没有实现；本条属于 Electron/Chromium 层面，不在本次壳层迁移范围内。默认的 XWayland 后端不受影响。下一步若要补：在原生 Wayland（`isWaylandSession()`）时改回纯指针手势做**窗口内重排**（不再跨窗口），或等 Chromium 的 `xdg-toplevel-drag`/DnD action 补齐；也可以让 Linux 发行版包强制 `--ozone-platform=x11`。
>
> **上游状态（2026-10 核对，均为 Chromium Gerrit 已合入的 CL）。** `xdg-toplevel-drag` 早已落地，但只服务 Chrome 自己的标签撕离（CL 5088752，2024-09-12；随后 2024-10 的 `tabdrag: handle drops using wl_data_source.dnd_drop_performed` 等），**未暴露给网页内容**，Electron 也没有对应 API。真正卡住本条的是 2025-10-15 的 `Ozone/Wayland: Treat DND drop performed with NONE action as a cancellation`（CL 7002773，为绕开 KWin 在成功 drop 后发 action=0 的 issue 447037092）——是**有意行为**，不是待修的 bug。2026 年的 Wayland 拖拽改动都在 Chrome 自身拖拽与 XDG FileTransfer portal 上（CL 7475473 / 7670558 / 8305973 / 8602873），没有任何“让 HTML5 拖拽源在 Wayland 上返回 action”的条目。Electron 侧对应请求是 [#54650](https://github.com/electron/electron/issues/54650)（2026-10-05 新开、仍 open，请求暴露 xdg-toplevel-drag 让窗口跟随指针）；#49907（同窗口 DnD 后输入卡死）以 `not_planned` 关闭，#53794（Wayland 下 `getCursorScreenPoint()` 无效）2026-09-16 关闭。结论：短期不要等上游，按上面的退路处理；若 #54650 落地，撕离/合并可以重做。

- **窗口内重排。** `dragover` 用 `tabReorderIndexAt` 实时 `moveTab`，`stripAutoScrollStep` 边缘自动滚动（光标停住也会继续，靠 rAF 自己续帧）；拖拽图是芯片的克隆（`setTabDragImage`），免得 `data-dragging` 的淡出被拍进 OS 拖拽图。
- **跨窗口合并。** 目标窗口在 `drop` 里自己 `mergeTabFromHandoff`（插入点由它自己的 `tabInsertionIndexAt` 量），然后回调 `claimTabHandoff`；主进程把 `tab-handoff-claimed` 转给来源窗口。
- **为什么需要 claim。** 拖拽结果（`dragend.dropEffect`）不可信：Chromium 的 Wayland 后端用合成器回的 `wl_data_source.action` 推导它，并把缺省 action 当作“取消”（`crbug.com/447037092`），于是“拖到另一个窗口”和“拖到桌面”在来源窗口看来一模一样。若按结果关标签，Wayland 上会双开；若按结果撕离，又会双开一个窗口。claim 把“谁收下了”变成应用内确定的事实，代价是 `dragend` 里最多等 150ms（`HANDOFF_CLAIM_GRACE_MS`）再决定撕离。
- **撕离。** 只有离开过窗口的拖拽在无人认领时才撕离；`Escape` 在窗口内取消仍然是放回原处。落点：X11/Windows 由主进程读实时指针（`tabWindowBounds`）；原生 Wayland 下客户端不能定位 toplevel、也读不到全局指针，因此只给尺寸、位置交给合成器（`isWaylandSession()`）。
- **落点判定不能靠 `drop` 事件（Wayland 实测）。** 原生 Wayland（WSLg/Weston）下，同窗口松手**不会**产生 `drop`：合成器不回 `wl_data_source.action`，Chromium 把“drop performed 但 action = 0”当作取消，于是来源窗口只收到 `dragleave` + `dragend`（`dropEffect: "none"`）。X11 上也有一种同构情况：实时重排会把被拖的芯片在 DOM 里挪位，源节点移动后 Chromium 收到 release 也不再派发 `drop`（单标签不重排的那次拖拽反而有 `drop`，可在日志里对照）。因此“松手是否在本窗口”改由**离开时刻**判定：`dragleave` 把 hover 计数清零时记 `leftAt`，`dragend` 时若 `leftAt` 为 `null` 或距今 ≤ 300ms（`IN_WINDOW_RELEASE_GRACE_MS`），就是本窗口松手（teardown 的 leave 与 `dragend` 几乎同时），补做最后一次重排；否则才进入 claim 等待与撕离。跨窗口仍以 claim 为准，误判只会多留一个标签，不会双开。
- **主进程不再提供 hover 会话。** 插入指示器直接用目标窗口自己的 `dragover` 计算；`merge_tab_into_window` 保留为命令面，但渲染层不再调用（合并由接收窗口本地完成并 claim）。
- **`DAE_TAB_DRAG_DEBUG=1`** 打开逐阶段日志（`start` / `enter` / `over` / `drop-own` / `drop-foreign` / `leave-window` / `end` / `settle-*` / `claim`），配合 `ELECTRON_ENABLE_LOGGING=1` 与 Chromium 自己的日志一起输出——没有它，"拖拽从未开始"和"拖拽到了别处"在页面里无法区分。
- **删掉的东西。** `startTabDrag`/hover 轮询会话/`tab_drag_outside`/`tab_drag_uses_frontend_bounds`/`tab_drag_hover_ack` 与 `tab-drag-hover|leave` 事件；这些命令名在主进程保留为显式报错，避免旧调用悄悄落到 core。native 的 `wait_for_tab_drag_release` 与 Tauri 的 `tab_windows` 原样保留（Tauri 壳仍可编译，但渲染层不再调用）。
- **行为变化。** 旧手势的“向下拖出标签带即撕离”随手势一起消失：现在只有离开窗口才撕离，与浏览器一致；`+`、关闭按钮、窗口拖拽区（`-webkit-app-region: drag`）不是投放目标，标签拖到标题栏的空白处不会被接受。

验收（需真机）：

1. 窗口内左右拖动标签实时换位，拖到两端继续自动滚动；`Escape` 放回原位。
2. 两个窗口之间互拖标签：目标窗口出现接受环 + 插入线，落下后标签在目标窗口的落点处出现、来源窗口同步消失（不双开）。
3. 拖到桌面松手：来源窗口少一个标签，新窗口出现（Linux 原生 Wayland 下位置由合成器决定）。
4. 拖到别的应用松手：同 3，新窗口出现在该处（X11/Windows）或合成器位置（Wayland）。
5. Linux 默认后端（XWayland）必过；`--ozone-platform=wayland` 已知拖不动（见上方限制），只需确认不误撕离、不刷 panic。
6. 原生 Wayland 的撕离/合并**不做验收**，直到 Chromium 的 Wayland DnD 可用。

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

### 本次：标签页拖拽重写 + 首个 Linux 产物

- **`get_system_accent` / `get_system_theme` 在 Linux 上必 panic。** `system.rs` 的 Linux 后端用私有 current-thread runtime `block_on` 去读 XDG portal，而这两个命令是在 addon 的 napi tokio runtime 上调用的，于是 `Cannot start a runtime from within a runtime`——渲染层启动时就报 `Unable to follow the system theme/accent`。两个 watcher 线程也共用同一个 static runtime，属于同一类隐患。已改成 async：`system::accent()/theme()` 变 async 并被 dispatch `.await`，Linux 后端直接 `await` portal 读取；两个 watcher 各自持有自己的 runtime（`portal_runtime()`），共享 `block_on` 删除。
- **标签拖拽的落点判定也在这一轮修正。** 见“已完成：标签页拖拽改为 HTML5 拖放”的落点判定一条。WSLg + Weston 实测：Wayland 下同窗口松手无 `drop`，X11 下实时重排后也无 `drop`。
- **Linux 的 addon 本来编译不过。** `native/src/open_with.rs` 只在 Windows 定义了 `open_system_with_dialog`，非 Windows 缺少同名分支，`cargo build` 直接 `E0425`——Tauri 侧一直有 `#[cfg(not(target_os = "windows"))]` 的 `fs.open_with_unsupported` 存根，迁移时漏了。已按 Tauri 的写法补回；Windows 构建仍走 `SHOpenWithDialog`。
- **WSL（Fedora 44）从零构建通过。** `bun install --frozen-lockfile` → `node node_modules/electron/install.js`（bun 默认不跑 postinstall，`electronDist` 需要 Linux 分发）→ `node scripts/native-build.mjs`（cargo release，`gio`/`gtk` 系依赖由 Fedora 的 glib2 开发包提供）→ `npm run build` → `npx electron-builder --linux rpm --x64 --publish never`，产出 `dae-0.4.0-x86_64.rpm`（93 MB，安装后 311 MB）。
- **产物检查。** `rpm -qRp` 的依赖（gtk3、nss、libnotify、at-spi2-core、libXScrnSaver、xdg-utils、libXtst、libuuid）在 Fedora 44 均可解析；解包后 `/opt/dae/dae` 与 `chrome_crashpad_handler` 无缺失动态库；addon 在纯 Node 中可加载并导出 `invoke`/`invokeRaw`/`nextEvents`/`startFileDrag`/`installSnapLayout`/`resolveShellService` 等全部入口。
- **仍未在 Linux 实机人工点验**：跨窗口合并、撕离窗口本身、自动滚动与 GNOME/KDE 的 Wayland 均未人工验证。已自动验证（CDP 合成鼠标，WSLg）：默认 X11 与 `--ozone-platform=wayland` 两种后端下“窗口内实时重排”与“不误撕离”；真鼠标在原生 Wayland 下仍拖不动（见上方已知限制）。


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
- **HTML5 标签拖拽的跨窗口段未真机验证。** Linux 默认 XWayland 已用 CDP 合成鼠标自动验证（窗口内重排、不误撕离），但真正的人工拖拽（跨窗口合并、拖到桌面撕离、自动滚动）与 Windows/macOS 上的一切都未点验。要确认的核心两点：渲染层 `dragstart` 写入的自定义 MIME 能否被兄弟窗口读到（`dragover.types` / `drop.getData`），以及不同合成器是否派发 `drop`。原生 Wayland 已知不可用（见上方限制），退路是让原生 Wayland 改回“指针手势只做窗口内重排”，跨窗口改用显式“合并到窗口…”入口。
