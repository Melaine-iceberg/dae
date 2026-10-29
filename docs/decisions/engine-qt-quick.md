# 引擎从 WebView 换成 Qt Quick：**已定方向，门槛 1–4 已过**

> **状态：方向已定，四组门槛全部通过（§5.1–§5.4）。** 分支 `refactor/qt-quick`。
> 本文记录的是「为什么离开系统 WebView」「为什么是 Qt Quick 而不是 egui / Slint / GPUI」
> 「许可证为什么是 GPL-3.0-or-later」，以及**定案前必须量出来的四组数字**。
> 其中的门槛数字全部是待验证的假设，不是结论 —— 与 `docs/decisions/README.md`
> 「先量，再改」的约定一致：这份文档的第一版是**假设清单**，spike 结束后回填实测值。
> §8 是已经回填的那部分：Qt 6.12 + cxx-qt 0.10 在本机编译、链接并跑通了 QML → Rust 调用。

## 0. 结论先行

1. **离开系统 WebView 的理由是结构性的，不是调优能解决的。** 实测在
   `tab-drag-webview-reparent.md` §2.4：**每多一个 webview ≈ +190–200 MB PSS**，
   基线（1 窗口 1 webview）tree PSS **447–483 MB**；预热窗口池再 **+200 MB**
   （`tab-tear-off-warm-pool.md` §3.5）。WebView2 创建本身占冷启动的 **63–65%**
   （`cold-start-js.md` §1.2）。这三条都是「每多一个视图就多一个浏览器进程模型」
   的直接后果，调参调不掉。
2. **保留 Rust 后端成立，而且成本比看起来低。** 84 个 Rust 文件里 **34 个零
   `tauri::` 引用**，389 处 `tauri::` 绝大多数是注入点（`AppHandle` / `State` /
   `Window` / `Emitter`），不是逻辑。真正要重写的是**前端 36.7k LOC**。
3. **选 Qt Quick（QML 视图）+ cxx-qt（Rust 后端对象），不选 Qt Widgets 写 Rust。**
   KDAB（cxx-qt 维护方）在 issue #1330 里的原话就是：QtWidgets 的 API 大量使用
   裸指针，用 CXX 写会引入大量 `unsafe`，**建议 UI 用 C++/QML 写，Rust 通过
   `QAbstractListModel` / `QObject` 提供后端**。cxx-qt 0.10 的官方示例里就有
   Rust 实现的 `QAbstractListModel`。
4. **许可证定为 GPL-3.0-or-later。** 开源前提下，GPLv3 与 Qt 的 GPLv3 选项兼容，
   不触发 LGPL 的「允许重链接」义务，将来若要静态链接成一个 exe 也不需要买商业授权。
   代价是 macOS App Store 彻底出局 —— 这条在 `macos-private-api`
   （`Cargo.toml` 里为透明窗口开着的 feature）时就已经付出过了。
5. **顺序不变：先剥 `dae-core`。** 引擎无关，两条路都能吃这份工作；它也是这次迁移里
   唯一**不会因为选错引擎而返工**的部分。

## 1. 被替换掉的到底是什么

| 层 | 现在 | 换成 | 是否重写 |
|---|---|---|---|
| 领域逻辑（列举、监听、搜索、传输、归档、云、SFTP/SMB、图标、右键命令） | Rust，43.4k LOC | 原样搬进 `dae-core` | 否，只剥注入点 |
| 数据契约（`DirectoryEntry` 等） | Rust + specta → TS | 原样，specta 只在双壳期保留 | 否 |
| IPC | 107 个 `#[tauri::command]` + 列式 packet + base64 缩略图 | 同进程直接调用；`QQuickImageProvider` 直接交 `QImage` | 是，**而且是删代码** |
| 视图 | React 19 + Tailwind + base-ui + xterm.js（36.7k LOC） | QML + Qt Quick Controls | 是 |
| 窗口 / 标签 | 每个 webview 一套，预热池弥补撕出延迟 | 一个进程多个 `QQuickWindow`，按需创建 | 是，**而且删掉整个池** |
| 终端 | xterm.js | `alacritty_terminal` + 自绘网格 | 是，排在 P3 |

`entry_codec.ts`（519 行）、`bindings.ts`（1506 行）、列式交接、缩略图 base64、
预热窗口池、`tab_perf.rs` 的撕出计时 —— 这些**随架构一起消失**，不是被移植。

## 2. 为什么是 Qt Quick

选型的第一原则是「这份代码要对冲的风险是跨平台窗口/输入/文本，而不是绘制速度」。

1. **中文 IME 不用自己造。** `QInputMethod` 是三平台原生级实现（预编辑串、候选窗
   跟随、组字取消）。这是 egui 最弱、而本项目不能妥协的一块。Windows 侧已实测
   （§5.2）：预编辑与候选窗都成立；唯一要带进壳的是换焦点前取消组合的原生守卫。
2. **`QAbstractListModel` + `ListView.reuseItems`** 是 12 万行列表的原生解，
   KDE 系文件管理器用的就是这套；`listing.rs` 的增量打补丁直接映射成
   `beginInsertRows` / `dataChanged`。
3. **多窗口成本是 5–15 MB 量级，不是 200 MB。** 实测第一个额外窗口 +13.74 MB PSS
   （含一次性缓存），热态每个窗口 +5.12–6.24 MB，关闭后私有提交不增长（§5.4）。
   撕出标签变成「开一个窗口」，预热池这套机械装置的**存在理由消失**。
4. **无障碍**（Windows UIA / macOS AX）现成。egui 侧只有 accesskit 的部分实现。
5. **Qt 6 在 6.x 内二进制兼容**，正是 WebKitGTK「每个发行版一个版本」的反面。
6. **原生 DnD 双向、原生菜单栏、高 DPI、`.ts` 翻译、图片格式插件**内置。
   现有 `drag` crate（OLE 拖出）、`shell_commands`（Win11 右键）、
   `window_material`（Mica/Acrylic 的 raw Win32）都能接到 `QWindow` 的 HWND 上。

### 被否掉的三个选项

| 选项 | 否决理由 |
|---|---|
| **egui** | 单语言、零 IPC、跨平台最稳，但 IME / 文本编辑 / 观感是它公开的弱项，而本题恰是「中文输入 + 美学」。有团队因为 `TextEdit` 性能把整个编辑器重写。 |
| **Slint** | 内存最优（软件光栅实测 ~30 MB，对比 egui ~135 MB），但富文本是硬短板：markdown 预览、代码高亮、属性表都要逐 run 着色；且 GPL/商业授权。 |
| **GPUI** | 性能与观感天花板最高，但 **Windows 后端仍属开发中**，而 Windows 是本项目的主战场（MSIX、大量 `#[cfg(windows)]`）。把日程押在别人的进度上不划算。 |

## 3. 许可证：GPL-3.0-or-later

- Qt 6 是 GPLv3 / LGPLv3 / 商业三选一。**GPLv3 应用可以直接吃 Qt 的 GPLv3 选项**，
  不需要 LGPL 的「允许用户替换 Qt 库 / 动态链接」义务，将来想静态链接成单文件也合法。
- 代价：macOS App Store 与 GPLv3 不兼容。这条已经在
  `tauri = { features = ["macos-private-api"] }`（透明窗口所需）时付过了。
- 退路：若将来要闭源分发，改 `LICENSE` 为 LGPLv3 + 动态链接，或买 Qt 商业授权；
  `cxx-qt` 本身是 MIT/Apache-2.0，不构成额外限制。
- 注意：Qt 的 **LTS 补丁在开源侧通常延迟 12 个月**。所以「6.12 LTS」对开源项目
  意味着**版本号稳定**，而不是「补丁比主线快」；跟 6.12 主线补丁，不要死守某个
  6.12.x 小版本。

## 4. 目标结构

```
crates/dae-core/   # 无引擎：types、error、file_system、vfs、watch、search、
                   # archive、cloud、sftp、smb、icons、shell_commands、settings、pty
crates/dae-qt/     # cxx-qt 桥：QAbstractListModel（目录/搜索）、QObject 门面、
                   # QQuickImageProvider（缩略图，替代 base64）、qt_thread 派发
apps/dae/          # QGuiApplication + QQmlApplicationEngine（新二进制）
qml/               # 视图：shell / tabs / list / grid / miller / dialogs / settings
src-tauri/         # 旧壳，迁移期保留，用于 A/B 与回退；P4 之后删除
```

两条约束写在代码里，不写在文档里：

- `crates/dae-core` **不得**出现 `tauri::`、`winit`、`qt*`、任何序列化 IPC 格式。
- 视图**不得**持有领域状态的真相；状态在 Rust，QML 只读 + 发意图。

### 迁移手法：strangler，而不是大搬家

1. `git mv` 一个模块进 `dae-core`；
2. 在原路径留 `pub use dae_core::<module>::*;` 垫片，**call site 一个都不改**；
3. 最后一个调用者搬完后删垫片。

已完成的切片（本分支，按依赖顺序）：

1. **契约**：`file_system/types.rs`（600 行）与 `file_system/error.rs` 进入 `dae-core`；
   `src-tauri` 侧只剩垫片。选它们是因为后面的每个模块都依赖它们。
2. **接缝**：`dae_core::host::{Host, Event}` + `TauriHost`；`system_theme` /
   `system_accent` 两个 watcher 改走 `&dyn Host`，**模块里的 `tauri_specta::Event`
   import 被删掉**——这是“不再自己发事件”的编译期证明。
3. **进度**：`FileOperationKind` / `FileOperationPhase` 进 `dae-core::progress`；
   `FileOperationProgressReporter` 与 `emit_preparing` 收 `Arc<dyn Host>` /
   `&dyn Host`，17 个构造点改为 `host::of(&app)`（每次操作一个 `Arc`，不是每次
   emit 一个）。wire 结构体留在壳里，由 `TauriHost` 字段对字段地填充。
4. **目录观察与列举**：`watch.rs` / `listing.rs` 的 emit 全改走接缝
   （`DirectoryChanged` / `DirectoryEntries`）。这一刀同时切掉了**托管状态**：
   `DirectoryWatcher`、`DirectoryListingState`、`GitStatusCache` 从
   `app.state::<T>()` 变成各自模块里的 `OnceLock`（一进程一实例，和原来同寿命），
   于是 `arm_local_watcher` / `open_streamed_listing` / `unwatch_directory` /
   `cancel_directory_listing` / `get_git_status` 都不再收 `AppHandle`。
   `listing.rs` 只留 `tauri::ipc::Channel` —— 那是 packet 通道，随 `entry_codec`
   一起在 Qt 壳落地时删除。

5. **其余领域状态**：`UndoRedoState`、`DirectorySizeState`、`FileHashState`、
   `RecentsState`、`FileSearchState`、`StartupPrefetch` 同样换成模块内 `OnceLock`；
   对应三个事件（`UndoRedoChanged` / `DirectorySizeProgress` / `FileHashProgress`）
   走接缝，`FileHashDigests` 进 `dae-core::hashing`。
   **结果是领域侧已经没有任何代码再去摸 Tauri 的托管状态**——剩下的两个
   `app.state::<…>` 都在 `tab_windows.rs`（窗口池，随 Tauri 一起删），
   三个 `.manage()` 也都是壳状态（深链缓冲、标签窗口、终端字节流）。

6. **命令层一分为二**：原 `file_system/commands.rs`（1046 行）拆成
   `commands.rs`（350 行，全是 Tauri 包装：拿注入的 `AppHandle` → 换成接缝 → 调
   `service::*`）与 `service.rs`（914 行，**一个 Tauri 类型都不提**，`spawn_blocking`
   也换成了 `tokio::task::spawn_blocking`）。长文档注释留在包装上，因为 specta 会把它
   抄进 `src/bindings.ts`——那是 TS 消费者读文档的地方。
   验收方式不是"看起来对"：**重新生成 `src/bindings.ts` 后逐字节对比，唯一差异是一个
   早先提交里加的文档注释**，没有任何命令名/参数/返回类型/事件变化。

7. **命令全部收口到包装文件**：`git`、`preview`、`search`、`open_with`、
   `system_files`、`archive`、`trash_view` 的 27 个命令也照 slice 6 拆了——函数留在
   各自模块且引擎无关，包装集中在 `file_system/commands.rs`（45 个）。
   `trash_view` 是个特例：同样四个命令名在两个 cfg 门控的平台模块里各有一份，
   包装只写一份，靠 `pub use` 在对应平台解析。
   **`start_drag_out` 故意留在壳里**：原生拖出需要窗口 + 主线程（OLE modal loop
   必须拥有窗口所在的 apartment），那是真的引擎形状，Qt 侧会用 `QWindow` 重做这
   三步。这是 `file_system` 里唯一"因为真的需要、而不是因为惯性"而依赖窗口的操作。

8. **`dae_core::paths` + 再 29 个命令**：把 `tauri::Manager::path()` 的
   `{系统配置目录}/{identifier}` 规则搬进 `dae-core`——Windows 漫游 `%APPDATA%`、
   macOS `$HOME/Library/Application Support`、Linux `$XDG_CONFIG_HOME`（须为绝对路径）
   否则 `$HOME/.config`。三条分支各自是纯函数，用合成输入测试，而不是靠跑测试的机器。
   这一条比别处更要紧：这个目录里放的是 `settings.toml`、收藏、最近记录、连接、
   known_hosts——两个壳算得不一样，用户升级后就是"全新安装"。
   两道保险：`the_identifier_matches_the_bundle` 编译期读 `tauri.conf.json` 校验
   identifier 没漂；实测在本机解析到 `%APPDATA%\com.melaine.dae`，而
   `settings.toml` / `recent-items.json` / `spaces.json` 本来就在那里。
   `get_system_places` 是唯一故意留下的命令：它经 `Manager::path()` 解析桌面/文档/下载
   等七个目录，把这套规则也搬进 core 是另一个决定，不该夹带在这一刀里。

9. **文件系统整体搬入 `dae-core`**：按[施工单](../slice-8-core-extraction.md)执行，
   两处波次按真实依赖调整——`local` 必须与 `git` / `transfer` / `vfs` 同批
   （`local/directory` 会作废 Git 状态缓存），`sidebar` / `prefetch` 必须与
   `recents` / `spaces` 同批（`prefetch` 同时在读写两侧）。六个 wire 事件收进壳的
   `ipc_events.rs`；列举管线拆成 core 的 `Batch`/`BatchTarget` 与壳的 `ChannelTarget`；
   `entry_codec` 随行模型先进 core，打包那半等 Qt 壳落地时与 `listing_packets.rs`
   一起删除。`get_system_places` 移到 `places.rs`，`warm_startup_data` 改为接收已解析的
   places，`write_atomic` 进 `dae_core::fs::atomic`。
   **实测**：`dae-core` 单独 `cargo test` **187 通过 / 8 忽略**；`src-tauri` nextest
   **167 通过 / 2 跳过**；重新生成的 `src/bindings.ts` **逐字节不变**；
   `bunx tsc --noEmit` 为 0。`src-tauri/src/file_system/` 只剩 10 个 `.rs`：
   `commands`、`preview`、`system_files`、`volume_monitor{,.rs}`、`places`、
   `listing_packets`、`ipc_events`。

累计（切片 1→9）：`tauri::` 引用 **389 → 265**、`tauri::AppHandle` **125 → 80**、
`app.state::<…>` **33 → 2**、`.manage()` **12 → 3**。

切片 8 本身只减了 2 处 `tauri::`，因为搬走的 51 个文件在切片 3–8 里已经全部接缝化
（这正是验收标准 #4 里“267 → ≤180”没有发生的理由：该预估假设搬移会带走 Tauri 引用，
但那些文件早已是零引用）。剩余 265 处：**95 处在 `commands.rs`**（IPC 壳本身）、
**139 处在 `src-tauri/src/` 根下的壳模块**（`tab_windows`、`default_manager`、
`window_material`、`terminal`、深链、更新器等，随 webview 一起退役或重写）、
其余 **31 处**在 `preview` / `system_files` / `places` / `volume_monitor`。

## 4.1 下一刀的执行手册

切片八已完成，施工单保留作验收记录与踩坑清单：
[`../slice-8-core-extraction.md`](../slice-8-core-extraction.md)。
§5 的四组门槛（12 万行列表、中文 IME、无边框 + Mica、撕出/关闭）已经全部
量完（§5.1–§5.4）。

### 4.2 骨架已落地（2026-10-25）

`crates/dae-qt` 与 `apps/dae` 已建，门槛里验证过的三块搬成产品模块：

- `crates/dae-qt/src/model.rs` — `DirectoryListModel`（门槛 1）。加了
  generation 与旧流取消：导航到新目录后，旧目录迟到的批次不会插进新列表。
- `crates/dae-qt/cpp/window.h` — **一张按 HWND 索引的状态表**（门槛 3/4）。
  首窗与撕出窗走同一套样式、背景、命中测试和 `WM_NCCALCSIZE`，不再有
  「主窗口 State + 次窗口 SecondaryState」两套路径。
- `crates/dae-qt/cpp/ime.h` — 换焦点前 `CPS_CANCEL` 守卫（门槛 2），随窗口
  attach 安装，点击别处或 Tab 时先取消组合，Qt 的 `reset()` 没有东西可提交。
- `apps/dae` + 根 `qml/` — `QGuiApplication` + `QQmlApplicationEngine`；
  「新建窗口」撕出第二个 HWND，两个视图共用同一个模型。
- 仓库根 `Cargo.toml` 建立工作区；`src-tauri` 暂排除，保留自己的 `target/`
  与锁文件（见该文件依赖注释）。
- spike 的 `run*.sh` 已删，`results/` 与 README 的实测记录保留；产品代码
  不含脚本化栅门、合成滚动负载或假目录。

构建需要 Qt 6.12 的 `qmake`：`QMAKE="K:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"`，
并把 Qt 的 `bin/` 放进 `PATH`，然后 `cargo run -p dae-app`。

**平台边界（2026-10-28）**：`crates/dae-qt` 在 Windows、Linux、macOS 都能通过
编译，但原生实现首先只在 Windows 实测过。`cpp/window.h`、`cpp/ime.h` 的入口点
现已按 `Q_OS_WIN` 分支：Windows 走 Win32 / DWM / IMM32（原实现不变），其余平台
保留同名空实现（校验 token、强制创建平台窗口），由 QML 的
`startSystemMove` / `startSystemResize` 接管无边框窗口的移动与缩放。

| | Windows | Linux / macOS |
|---|---|---|
| 边框、移动、缩放 | `WM_NCHITTEST` 原生命中测试 | Qt 无边框窗口 + QML system move/resize |
| 背景材质 | Mica / Acrylic（DWM，用户可选） | Linux Wayland：`ext-background-effect-v1`；macOS：`NSVisualEffectView`；不支持时实色 |
| 输入法换焦点守卫 | `CPS_CANCEL`（门槛 2） | 不安装；尚无实测等价物 |
| `dae://` 注册 | 注册表写入 | 未实现；单实例 socket 仍可用 |

因此 Linux/macOS 的发行物是「能启动、能移动缩放的 Qt 壳」，而不是 Windows 观感的
逐像素复制；上表每一项仍需各自的门槛，不以未实测的移植冒充产品代码。CI 的
`qt-check.yml` 在 Linux/macOS 上编译 `dae-app`，让 release 矩阵不再是这两个平台
的第一位编译器。

### 4.3 真路径已打通：配置 → 目录 → 导航（2026-10-25）

骨架的起点原来是 `QStandardPaths::HomeLocation`，绕过了旧壳已经写下的所有
文档。`crates/dae-qt/src/config.rs` 把它换成同一条规则
（[`dae_core::paths::config_dir`](../../crates/dae-core/src/paths.rs)）：

- **配置目录**：`%APPDATA%\com.melaine.dae`；读 `settings.toml` 的
  `[shortcuts].app.pathJump`（`Mod` 翻成 Qt 的 `Ctrl`），并读
  `recent-items.json` 里最近一个仍然存在的目录。
- **起点优先级**：命令行传入的目录/文件 → 最近访问 → home。文件参数取父目录；
  坏路径、缺失文件与坏 TOML 都回落到默认值，不阻塞启动。
- **明暗**：`cpp/window.h` 的 `dae_qt_system_dark()` 读
  `QStyleHints::colorScheme()`，Mica/Acrylic 的 immersive-dark 不再硬编码。
- **视图**：标题跟随当前目录；状态栏显示启动来源、`settings.toml` 是否读到
  与真实配置目录；路径栏 Enter、双击目录、`Ctrl+P` 聚焦（绑定来自真实
  `settings.toml`）都可用。`FileList` 点击时把焦点从路径栏移开，IME 守卫
  仍在换焦点前取消组合。

材质决策没有改：本机 build 26340 仍按 Tauri 的规则选 **Acrylic**
（`DWMWA_SYSTEMBACKDROP_TYPE` read-back **3**）；22000–22522 走 Mica
（read-back **2**），门槛 3 的强制 Mica 路径保留。`DAE_WINDOW_MATERIAL=mica`
（或 `acrylic` / `none`）可以覆写机器判定，供 A/B 与设计对照使用；实测强制
Mica 后 read-back 为 **2**。无边框窗口、原生 move/size loop、`HTMAXBUTTON`
与 `CPS_CANCEL` 守卫都直接复用 §5.3 / §5.2 的实现。

**2026-10-29 更新**：材质不再由机器能力单独决定，而是用户设置
（`blur` + `windowMaterial`，默认 `true` / `mica`）。机器判定只决定所选材质能否
合成：Acrylic 在 22000–22522 回退 Mica，低于 22000 或系统关闭透明效果时回退实色。
Linux Wayland 新增 `ext-background-effect-v1` 背景模糊；合成器未提供该协议时保持
实色。macOS 在 Qt 视图后方插入 `NSVisualEffectView`（behind-window 混合），深浅色
跟随窗口主题。`DAE_WINDOW_MATERIAL` 仍是 A/B 覆写入口。

**独立 exe 的构建与部署**（Windows-first）：

```powershell
$env:QMAKE = "K:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"
$env:PATH  = "K:/Qt/6.12.0/msvc2022_64/bin;$env:PATH"
cargo build --release -p dae-app
New-Item -ItemType Directory -Force dist/qt
Copy-Item target/release/dae.exe dist/qt/
windeployqt --release --no-translations --compiler-runtime --qmldir qml dist/qt/dae.exe
```

release 用 `windows_subsystem = "windows"`，没有控制台窗口；`dae.exe` 的 PE
导入表只有 Qt 6 与系统 DLL（**没有 Tauri / WebView2**），运行时的模块列表里
也没有它们。`dist/qt` 约 **70 MB**（Qt DLL + QML 插件 + MSVC runtime），exe
本体 **3.3 MB**。

**端到端冒烟：29/29 通过**（Windows 11 build 26340 / 3840×2160 @150%）：

| 项目 | 实测 |
|---|---|
| 启动 | 标题 = `C:\Users\Mt342\.cargo\binstall`，来自真实 `recent-items.json`；状态栏显示 `settings.toml` 与 `%APPDATA%\com.melaine.dae` |
| 配置隔离 | 把 `APPDATA` 指向假 profile（不同 recent + `Mod+Shift+P`）后，启动目录变为 `K:\code\dae`；`Ctrl+P` 不再生效，`Ctrl+Shift+P` 生效 |
| 路径导航 | `Ctrl+P` → `C:\Windows` → Enter，标题切换；双击 `appcompat` 进入下一层 |
| 原生 chrome | style `0x16cf0000`（THICKFRAME/MAXIMIZEBOX）、`WS_EX_LAYERED`；hit test caption **2** / max **9** / min **1** / close **1** / left **10** / content **1**；拖动 +135/+94 px、右边缘加宽 +95 px |
| 最大化 | 自绘按钮 → client **3840×2088 = 工作区**；还原回原尺寸 |
| 材质 / 明暗 | 默认 `DWMWA_SYSTEMBACKDROP_TYPE` = **3**（本机 Acrylic）；`DAE_WINDOW_MATERIAL=mica` 强制后 = **2**（Mica）；immersive-dark = **1**（跟随系统） |
| IME | 微软拼音 `nihao` 出现候选窗（`1 你好` …），Esc 取消后路径文字保持不变 |
| 关闭 | `WM_CLOSE` 后进程退出，无残留控制台窗口 |

旧壳同时复验：`cargo build --manifest-path src-tauri/Cargo.toml` 通过（仅保留
原有的 `TabDropTarget` dead-code 警告），A/B 的两条路径都仍然可构建。

### 4.4 标签与撕出：直接复用门槛 4，不建池（2026-10-26）

标签是 QML 对象，不是窗口：`qml/Tab.qml` 持有一个 `DirectoryListModel` 和
自己的选择集，**没有 QObject 父对象**，所以撕出时整个对象（连同 12 万行）只是
换一个窗口引用，不复制、不重新枚举。选择存在 tab 上而不是窗口上，切标签、
撕出、合并都跟着走。

`qml/ShellWindow.qml` 是唯一窗口实现，主窗与撕出窗都实例化它；`TearOffWindow.qml`
只是 `ShellWindow { firstWindow: false }`。因为两者互相引用，撕出窗口按 URL
`Qt.createComponent(..., Component.PreferSynchronous)` 加载，规避 QML 静态循环。

拖动交给平台自己的拖放循环：`TabStrip` 越过 6 px 阈值后调用
`Shell.startTabDrag()`，原生侧（`cpp/drag.cpp`）用 `QDrag` 跑
OLE `DoDragDrop` / `NSDraggingSession` / X11·Wayland DnD。拖动图是按标签
标题与主题色绘制、并按窗口 DPR 光栅化的 chip，**热区就是按下的抓取点**
（`setHotSpot` 使用逻辑像素，Qt 各平台后端负责换算），所以预览和 Tauri 的
ghost 一样按抓取点跟随光标，而不是居中。平台循环在指针离开窗口后仍然有效，
这正是 `MouseArea` 做不到的部分。

每个 `ShellWindow` 有一个匹配 `application/x-dae-tab` 的窗口级 `DropArea`：
拖动经过时只显示插入标记，落下时只把「源标签、目标窗口、落点」记进
`Shell.js`；源标签在 `QDrag::exec()` 返回后统一执行重排 / 合并，模态循环
运行期间不会改动任何一个窗口的标签列表。没有任何窗口接收的释放（桌面、
其它应用）由最终光标位置判定为撕出；循环在窗口内被取消（Esc）时标签留在
原处。旧的 16 ms 轮询（`WindowFromPoint` + `GetAsyncKeyState`）与
`Qt.Tool` 悬浮预览窗已删除，拖动路径只依赖跨平台 Qt API。
没有预热窗口池：撕出按需创建窗口，`WindowController.attach()` 给新 HWND 写同一套
边框、背景与命中测试（门槛 4 的路径原样复用）。

旧轮询路径实测（2026-10-26，Windows 11 build 26340 / 150%）：`+` 新建标签 → 2 标签；
把第二个标签拖出窗口 → **2 个 HWND**；再拖回第一个窗口 → **1 个 HWND**。
脚本与截图在 `.perf/tab-actions.ps1`（gitignored）。改用 `QDrag` 后，构建、
77 项单元测试与独立 exe 启动均通过；跨窗口/撕出的交互复验需在真实输入下
重做，本机注入输入在 150% 缩放的多窗口环境里无法稳定命中目标。

### 4.5 终端：alacritty_terminal + 自绘网格（2026-10-26）

- `crates/dae-core/src/pty.rs` — `portable-pty` 起真实 shell（Windows 依次尝试
  `pwsh` / `powershell` / `cmd`），`alacritty_terminal` 的 `Term` + `Processor`
  维护网格。`PtyListener` 把 `Event::PtyWrite` 写回 PTY，并回答
  `TextAreaSizeRequest`——缺了这一步 PowerShell/Starship 的 prompt 不会出现。
  调色板覆盖 16 色、256 色立方与灰阶、命名色。
- `crates/dae-qt/src/terminal.rs` — `TerminalSession` QObject 把快照压成
  `DAET` v2 帧（21 字节帧头 + 13 字节/单元格，帧头带历史行数），输出用
  `AtomicBool` 合并，一帧一次 `updated()`，不会让一次 `ls` 排满 Qt 事件队列。
- `cpp/terminal_item.{h,cpp}` — `QQuickPaintedItem` 自绘：背景按行合并成 run，
  文本按样式/颜色合并成 run；宽字符、astral codepoint（`QString::fromUcs4`）、
  粗/斜/下划线/删除线、块/下划线/竖线光标与闪烁、IME 预编辑与提交、滚轮、
  按键转义序列。字号、行高、字体族作为属性，来自 `settings.toml`。
- `qml/Tab.qml` — **每个标签一个会话**（`TerminalSession` 随 `Tab` 对象走，
  不挂在窗口上）。面板第一次在这个标签里打开时才 `start()`；切标签只换绑定，
  隐藏面板只改高度，shell 都继续运行。撕出标签时会话跟着标签对象进新窗口，
  不重启；`dispose()` 关闭标签时才 `kill()`。
- `qml/TerminalPanel.qml` — 只绑定 `tab`，由它派生 `session` 与 `cwd`；切换
  标签时若面板打开，新标签的 shell 按该标签当前路径启动。
- 选区与复制：鼠标拖选、双击选词；`Ctrl+Shift+C`、`Ctrl+Insert`（macOS
  `Cmd+C`）复制，有选区时 `Ctrl+C` 也复制（Windows Terminal 的行为），
  否则仍是中断字节。任意非修饰键清除高亮。选区只覆盖当前视口，滚到哪就能
  选到哪。
- 滚动条：`DAET` v2 帧头新增历史行数，painter 在右侧画 6 px 轨道与滑块，
  拖动按绝对偏移调用 `scrollTo`（滚轮也走同一条 `updated()` 重绘路径，
  修掉了此前滚轮不动画面只在下次输出时才刷新的问题）。
- Nerd Font 私有区：主字体缺字时（`inFontUcs4` 为假）在已安装字体里查找
  含该码点的家族，优先名字带 `Nerd` 的字体，按码点缓存结果，命中的单元格
  用回退字体绘制；没有任何字体包含该字形时仍是方框，那是字体确实缺失。

实测（2026-10-25/26）：真实 PowerShell + Starship prompt 正常显示；`echo hello`、
`echo 你好` 回显与配色正确；隐藏再打开后 `echo survived` 仍接在同一会话上；
把字号保存为 14 后新终端按 14 号绘制。

2026-10-28 复验（Windows 11 build 26340）：打开终端后新建标签，进程枚举到
两个 `pwsh` + 两个 `OpenConsole`，即每标签一个会话，切回第一个标签内容仍在；
拖选高亮正常，`Ctrl+C` 把选区写入系统剪贴板（粘贴到路径栏核对）。本机
`[terminal]` 未配置 fontFamily（主字体 Cascadia Mono），Starship prompt 的
Nerd Font 图标正常显示，回退字体路径生效。滚动条在历史为空时不出现；有真实
历史后的拖动尚未在自动化输入下复验。

### 4.6 命令与平台模块：设置、位置、文件操作（2026-10-26）

**设置进 core。** `crates/dae-core/src/settings.rs` 从 Tauri 壳原样搬入
（同样的 registry、默认快捷键、8–24 字号/0.8–3.0 行高钳制、原子写）；
`src-tauri/src/settings.rs` 只剩两个 `#[tauri::command]` 包装和 crate 内访问器。
两个壳现在读写同一份 `%APPDATA%\com.melaine.dae\settings.toml`，不会各写各的。

**系统位置进 core。** `dae_core::fs::sidebar::system_places()` 用 `dirs` 解析
七个已知目录；Tauri 的 `PathResolver` 也是 `dirs`，所以两个壳在同一台机器上
得到同一组路径，与 `recent-items.json` 的键一致。

**命令面 `Workspace` 单例**（`crates/dae-qt/src/workspace.rs`）：

- 文件：新建文件夹/文件、重命名、移到回收站、永久删除、创建副本、复制/剪切/
  粘贴、撤销/重做。全部直接调用 `dae_core::fs::service`；冲突默认 `KeepBoth`
  （同目录粘贴不会互相覆盖）。
- 剪贴板：应用内镜像（`copy`/`cut`/`link` + 路径列表）与系统文件剪贴板同步。
  复制/剪切时本地路径以 `CF_HDROP` + `Preferred DropEffect` 写入系统剪贴板，
  资源管理器、聊天窗口可直接粘贴；粘贴优先读取系统剪贴板（另一应用复制的内容
  覆盖应用内镜像，应用自己写入的列表仍用镜像以保留远程路径），
  `DROPEFFECT_LINK` 与“粘贴快捷方式”都走 `create_shortcuts`。剪切是一次性的，
  复制可以连续粘贴到多个目录。
- 位置：收藏读写、最近访问记录；侧栏模型在收藏变化时刷新。
- 回收站：`TrashModel` 异步列出系统回收站（Windows 直接解析 `$I` 元数据），
  `restoreTrashEntries` / `deleteTrashEntries` / `emptyTrash` 三个命令走
  `dae_core::fs::trash_view`，沿用同一套进度与 toast；永久删除与清空先弹
  `ConfirmDialog`。
- 打开：默认程序（`ShellExecuteW` / `open` / `xdg-open`）、文件管理器定位
  （Explorer `/select,` / `open -R` / `xdg-open` 父目录）、系统终端、系统
  “打开方式”对话框（Windows `SHOpenWithDialog`；macOS/Linux 走 §4.10 的自绘
  选择器）。
- 设置：终端字号/行高/字体族保存后立即应用到所有已挂载的 `TerminalItem`。
- 系统外观：强调色（`HKCU\...\DWM` 的 `AccentColor` ABGR 与
  `ColorizationColor` ARGB）和明暗偏好（`...\Themes\Personalize` 的
  `AppsUseLightTheme`）都走 Windows 原生 watcher，变化经 `qt_thread()` 队列写回
  `Workspace.accent` / `Workspace.systemDark`，QML 与 Mica/Acrylic 背景随之更新。
- 磁盘：`WM_DEVICECHANGE` 的卷到达/移除经 350 ms 去抖后发 `volumesChanged`，
  每个窗口的侧栏调用已有的 `SidebarModel.refresh()` 重新读取磁盘。
- 线程：一个进程级 Tokio runtime；`Host` 事件经 `CxxQtThread::queue` 回到 Qt
  线程，更新 `busy`、进度、`canUndo`/`canRedo`，并发出
  `operationFinished` / `operationFailed`。

**视图**：`qml/Sidebar.qml` + `sidebar_model.rs` 用一个带 section 的
`QAbstractListModel` 画位置/收藏/磁盘/WSL/网络/云盘/最近，并在固定位置提供
回收站入口；
`TrashView.qml` 画回收站（名称/原位置/删除时间/大小、多选、右键菜单、
恢复/永久删除/清空）；`FileList` 支持 Ctrl/Shift 多选、
F2、Delete、Shift+Delete、Ctrl+C/X/V、Ctrl+A、F5、Backspace、Enter；
`ContextMenu.qml` 提供行菜单、空白区菜单与侧栏菜单；`NamePrompt.qml` 负责
新建/重命名；`SettingsPanel.qml` 编辑终端排版并注册默认文件管理器；
`Toast.qml` 显示结果。

端到端实测（2026-10-26，`.perf/phase3-test.ps1`，Windows 11 build 26340）：

| 步骤 | 结果 |
|---|---|
| Ctrl+Shift+N → 输入名字 → 确定 | 目录创建，列表刷新并选中，toast「已创建」 |
| F2 → 输入新名字 → 确定 | 改名成功，选中新名字，toast「已重命名」 |
| Ctrl+C → 进入子目录 → Ctrl+V | 目录复制到目标，路径栏与标题随导航更新 |
| 右键行 / 右键空白 | 行菜单（打开/新标签/新窗口/定位/打开方式/终端/复制/剪切/粘贴/副本/重命名/收藏/回收站/永久删除）与空白菜单（新建/粘贴/全选/刷新/终端） |
| Ctrl+A → Delete | 两项进入回收站，列表清空 |
| Ctrl+Z | 两项恢复，toast「已撤销」 |
| Ctrl+Shift+Z | 再次删除 |
| Ctrl+, → 字号改 14 → 保存 | `settings.toml` 写入 `fontSize = 14`，终端按 14 号绘制 |

**§4.6 之后已经补齐的**（代码已落地，不再算待办）：

- **WSL**：`SidebarModel.refresh()` 在磁盘之后的同一后台任务里读取
  `dae_core::fs::sidebar::list_wsl_distros()`，有发行版时在「磁盘」之后插入
  `WSL` 分组，每行导航到 `\\wsl$\<name>`（访问即启动已停止的发行版）；
  没有发行版时不插入任何行，分组头随之消失，非 Windows 平台因此不出现。
- **搜索**：`qml/SearchBar.qml` + `crates/dae-qt/src/search_model.rs`，文件名
  与内容搜索各一个模型，结果复用同一个 `FileList` 委托。
- **预览**：`qml/PreviewPanel.qml` + `crates/dae-qt/src/preview.rs`；图片由
  `thumbnail://` provider 解码并缓存。
- **压缩包**：`qml/ArchivePasswordDialog.qml` +
  `Workspace.compressEntries` / `extractArchive`，加密包走同一套密码重试。
- **Git**：`DirectoryListModel` 在导航/刷新后读取 `get_git_status`（导航时同时
  读 `list_git_branches`），新增 `gitStatus` 角色画列表的 A/M/U 徽标；状态栏分支
  chip 显示当前分支与 ↑/↓ 差值并提供同步按钮，`qml/GitPanel.qml` 提供本地/远程
  分支切换、新建（可选创建后切换）与获取/拉取/推送/同步；失败经
  `gitOperationFailed` 转成 toast 或新建表单内联错误。
- **打开方式选择器**：`qml/OpenWithDialog.qml` + `OpenWithModel`；macOS/Linux
  自绘，Windows 仍用原生对话框（详见 §4.10）。
- **默认文件管理器**：`crates/dae-qt/src/default_manager.rs` 接管旧壳
  `src-tauri/src/default_manager.rs` 的三个平台分支，经 `Workspace` 的
  `refreshDefaultManager` / `setDefaultFileManager` / `unsetDefaultFileManager`
  异步读写系统关联，设置面板内联显示「已是默认 / 已注册待确认 / 不是默认」、
  平台补充说明（Windows 需在系统设置确认，macOS 不能自行取消）与失败原因。
  Windows 注册 `dae.folder` ProgID、`Directory\OpenWithProgids`、右键
  「Open with dae」并读 `Directory\UserChoice` 判断是否已生效；Linux 写
  `~/.local/share/applications/dae-folder.desktop` 后调 `xdg-mime`，并只从
  `mimeapps.list` 删自己那一行；macOS 用 LaunchServices 的
  `LSSetDefaultRoleHandlerForContentType("public.folder")`，`unset` 交给系统。
  `packaging/qt/Info.plist` 因此新增 `CFBundleDocumentTypes`（`public.folder`，
  `Alternate`），否则 LaunchServices 不把 dae 认作候选。
- **自动更新**：`dae_core::update` + `Updater` 单例 + `qml/UpdateDialog.qml`
  （详见 §4.10）。
- **右键菜单补齐**：属性、复制路径、预览、移动到、应用扩展（已安装应用的
  Shell 命令）与批量重命名。三个平台后端
  （`dae_core::fs::shell_commands`，原 `src-tauri/src/shell_commands/`）整体
  搬进 core，旧壳只剩两个 `#[tauri::command]` 包装；macOS 需要在主线程进入
  services 机制，因此 `Host` 多了 `run_on_ui_thread`，Tauri 用
  `run_on_main_thread`、Qt 用 `CxxQtThread::queue` 实现。属性对话框
  （`qml/PropertiesDialog.qml` + `crates/dae-qt/src/properties.rs`）覆盖常规
  信息、POSIX 权限/DOS 属性编辑、递归应用，以及 core 早已实现的校验和与目录
  大小两条流式任务；`qml/BulkRenameDialog.qml` 把旧壳的
  `planBulkRename`（序号/替换/大小写）原样搬成 QML 预览，
  `Workspace.renameEntriesBatch` 复用 core 的暂存式批量改名；
  `qml/MoveToDialog.qml` 复用 `DirectoryListModel` 选目标目录后走同一条移动
  队列。菜单项带图标（`data:` URL）与分组标题，行菜单在异步命令到达时就地
  刷新而不是关掉重开。Windows 的「属性」直接开系统自己的属性表
  （`ShellExecuteExW` + `properties` verb，modeless；其余平台仍用自绘对话框），
  应用扩展里没有自带图标的命令回落到包内 `Square44x44Logo` / `Properties/Logo`
  的 PNG，分组标题也用它，因此每一行都有应用图标。

**仍然没铺的**（core 已有实现，缺 Qt 视图/命令包装，或只在旧壳里）：暂无。
默认文件管理器注册已随 §4.6 的补齐清单落地。文件图标与
系统缩略图已在 §4.7 落地（含 Linux 的 `.thumbnailer`），网络存储/云盘连接
管理已在 §4.8 落地，`dae://` 深链与单实例已在 §4.9 落地，打开方式选择器与
自动更新已在 §4.10 落地，右键菜单的属性/校验和/目录大小/移动/应用扩展/批量
重命名已在 §4.6 的补齐清单里落地。

### 4.7 文件图标与系统缩略图（2026-10-26）

图标抽取整体搬进 `dae-core::fs::icons`（三平台后端原样），`xdg` 一并搬进
`dae-core::xdg`，旧壳通过薄垫片复用同一份实现：`file_system/preview.rs` 与
`shell_commands/windows.rs` 不再自带 COM/GDI 代码（`shell_commands` 整体随后
也搬进了 `dae-core::fs`，见 §4.6 的补齐清单）。Windows 的
`IShellItemImageFactory` + DIB→PNG 移进 `dae_core::fs::windows_image`，系统
缩略图入口是 `dae_core::fs::thumbnails`。

**类型图标换了一条 Win32 路径。** 旧实现在
`SHDefExtractIconW("type.<ext>", 0, 0, …)` 上取扩展名图标；在 Windows 11
build 26340 上它对任何扩展名都返回 `E_FAIL`（实测 json/txt/rs/exe/docx/zzz
全部失败，而真实路径的 `notepad.exe` 成功）。`extract_type` 改为
`SHGetFileInfoW(…, SHGFI_USEFILEATTRIBUTES | SHGFI_ICON | SHGFI_LARGEICON)`：
同样只看名字、不需要文件存在，对上述全部扩展名返回图标，未知扩展名回落到
通用文档图标。尺寸只有大/小两档（32 / 16 px），其余交给
`QQuickImageProvider` 缩放；18 dp @1.5× 要 27 px，32 px 的源不需要放大。

**Qt 侧**：

- `crates/dae-qt/src/icons.rs` — `image://fileicon/path/<path>?dir=&v=&size=`
  与 `image://fileicon/type/<ext>?size=`。目录、快捷方式和可执行文件按路径取
  （各自图标可能不同），普通文件按扩展名共享一次请求；命中与未命中都记，
  LRU 512 项。
- `cpp/image_provider.cpp` — `FileIconImageProvider` 在 Qt 的图像加载线程上
  调用 Rust；`DirectoryListModel` / `SearchModel` 新增 `iconSource` 角色。
- `qml/FileList.qml` — 18 dp 的 `Image` 覆盖原来的色块；图标未就绪或失败时
  色块仍在，所以每一行永远有东西画。
- `thumbnail://` 在 Qt 自己解不了（PDF / 视频 / HEIC）时回落到
  `dae_core::fs::thumbnails`，且缩略图请求带 `SIIGBF_THUMBNAILONLY`：没有真实
  缩略图就返回空，预览面板显示「无法生成缩略图」，而不是把 384 px 的图标当成
  首页缩略图。`PreviewController.hasThumbnail` 相应加入桌面可渲染扩展名。

**Linux 的 `.thumbnailer` 也进了 core（2026-10-29）。** 旧壳的
`src-tauri/src/desktop_thumbnails.rs` `git mv` 到
`crates/dae-core/src/fs/thumbnails/linux.rs`，与 Windows 后端共用同一个
`dae_core::fs::thumbnails` 入口。API 从「平台各自返回 `Vec<u8>`」统一成
`extract(path, size, modified) -> Option<Thumbnail>`：`Thumbnail` 带 mime，
`modified` 是调用方已有的源文件 mtime——Linux 的共享缓存
（`$XDG_CACHE_HOME/thumbnails/`）按「缓存文件不旧于源文件」判新鲜度，失败记录
同理，因此不必在渲染线程上再 `stat` 一次；Windows 的
`IShellItemImageFactory` 忽略该值。旧壳的 `mod desktop_thumbnails` 已删除，
`preview.rs` 的 Windows/Linux 两个分支并成一个；Qt 侧
`dae_qt_render_system_thumbnail(path, size, modifiedMs)` 把 `QFileInfo`
已经读到的 mtime 传进来。

**实测（2026-10-26，Windows 11 build 26340 / 150%）**：

- `dae-core` **223 通过 / 8 忽略**（新增 notepad.exe 图标、`.json` 类型图标、
  缩略图扩展名分类）；`dae-qt` **43 通过**；旧壳 `cargo check` 通过。
- debug 构建启动后列表显示真实图标：仓库根的文件夹图标、`crates-v1.json` 的
  JSON 图标、`C:\Windows\System32\WindowsPowerShell\v1.0` 里
  `powershell.exe` 的 PowerShell 图标、`.ps1xml` / `.config` 的文档图标；
  stderr 无 provider 错误。
- PDF 预览在这台机器上显示「无法生成缩略图」：本机没有注册 PDF 缩略图处理器
  （旧壳同样拿不到首页）。这条验证的是「没有处理器时不冒充缩略图」，不是
  处理器本身；装了 Edge/Adobe 缩略图处理器的机器走同一调用。

**遗留与复测**：

- Linux 的 `.thumbnailer` 已不再留壳：19 个模块测试随文件进了 `dae-core`，
  其中 15 个在 Windows 主机上以 `cfg(test)` 编译运行（`/bin/sh` 进程、`PATH`
  查找和真实生产者的 4 个仍按 `cfg(unix)` gated）。旧壳 `cargo check` 通过，
  `cargo test -p dae-core --lib` 全量 299 passed（`fs::transfer` 的并发峰值用例
  在本机首次偶发失败，复跑通过，与本次改动无关）。
- 图标接入后的 12 万行复测已通过：见 §5.1.1。work p95 1.60–1.66 ms，与无图标的
  1.64–1.82 ms 在噪声内；PSS +3–4 MB。

### 4.8 网络存储与云盘连接管理（2026-10-26）

**存储注册表进 Qt 进程。** `connections.rs` 与 `cloud::accounts` 早就在
`dae-core`，但 Qt 壳从未初始化它们。`dae-qt::init_stores()` 用 `Once`
调用两份 `init()`，由 `Workspace` 构造和 `SidebarModel.refresh()` 共用；
两个壳因此读同一份 `%APPDATA%\com.melaine.dae\{connections,cloud_accounts}.json`
和同一组钥匙串条目。

**命令面**（`crates/dae-qt/src/workspace.rs`）：

- `testConnection` / `saveConnection` / `deleteConnection`：SMB 与 SFTP。
  测试使用对话框里刚输入的凭据，不落盘；保存把密码交给系统钥匙串，
  `rememberPassword=false` 时只留在本次进程内（与旧壳一致）。
- `authorizeCloudAccount` / `deleteCloudAccount`：Google Drive、OneDrive、
  Dropbox 的 loopback OAuth 可达 5 分钟，因此命令立即返回，结果经
  `cloudAuthorizeFinished(ok, idOrMessage)` 回到 Qt 线程。
- `connectionsChanged`：新增/删除后通知侧栏刷新；保存成功的
  `connectionSaveFinished` 携带连接 id，供 QML 直接导航。
- 连接失败文案在 `friendly_error()` 里翻译成可操作的句子（服务器不可达、
  账号密码被拒、缺 SFTP 账号、无 SFTP 子系统）。

**视图**：

- `qml/Sidebar.qml` + `sidebar_model.rs`：在磁盘之后新增「网络存储」「云存储」
  两组，行尾常驻「连接网络存储…」「添加网盘账户…」动作行，因此空分组也有
  添加入口；右键菜单对连接/云盘分别提供「移除连接」「移除云盘账户」。
- `qml/ConnectDialog.qml`：SMB/SFTP 协议切换、地址/端口/账号/密码、
  「记住密码」、测试连接与保存；端口空表示协议默认值，非法端口和空地址
  在本地拦下，测试结果与保存错误分别回显。
- `qml/CloudAccountDialog.qml`：三家网盘卡片、Client ID/Secret、服务商
  引导文案与固定回调 `http://127.0.0.1:51888`；复制按钮通过 1×1 的
  `TextInput` 写入系统剪贴板。授权期间显示等待提示，失败保留对话框。
- 保存成功或授权成功后直接导航到 `smb://` / `sftp://` / `cloud://` 根路径。

**远程路径**：`DirectoryListModel.load()` 先用 `vfs::scheme_of()` 分流；
非 `Local` 的路径交给 `vfs::resolve(path).read_dir()`（远程后端本来就一次性
返回），本地继续走原来的流式列举。`parentOf()` 识别 `scheme://authority`
根，不再把 `smb://host:445` 的上一级算成 `smb:/`。列表为空且 `status`
不是「0 项」时，`FileList` 显示该状态而不是「空目录」，连接失败不再伪装成
空文件夹。

**实测（2026-10-26，Windows 11 build 26340 / 150%）**：

| 步骤 | 结果 |
|---|---|
| 空地址点「测试连接」 | 「请输入服务器地址」；端口 70000 报「端口必须是 1-65535」 |
| `127.0.0.1:1` 测试 | 失败态显示「无法连接服务器，请检查地址、端口和网络」 |
| 保存 `127.0.0.1:1` | 对话框关闭，侧栏出现 `127.0.0.1 · SMB`，并导航到 `smb://127.0.0.1:1` |
| 右键该行 → 移除连接 | 行消失，toast「已移除连接」，`connections.json` 变为 `[]` |
| 云盘对话框切 OneDrive | Client Secret 标签与引导文案同步切换；空 Client ID 报错 |
| 复制重定向 URI → 粘贴 | Client ID 字段得到 `http://127.0.0.1:51888` |

`dae-qt` **133 通过**（当前全量：连接字段解析、校验文案、侧栏行映射、错误翻译与搜索错误映射）。

**远程语义（2026-10-29 补全）**：

- 名称搜索与内容搜索都只走本地磁盘：`fs::search::search_directory()` 在
  `vfs::is_local_path()` 为假时直接返回 `fs.search_local_only`，内容搜索继续
  返回 `fs.content_search_local_only`；SMB/SFTP/云盘后端不再覆盖
  `FileSystemBackend::search()`，默认实现统一拒绝。Qt 壳在远程标签页禁用搜索
  图标与 `explorer.focusSearch`，进入远程路径时收起搜索栏并清空查询；命令
  面板不启动文件名搜索，也不提供「在文件内容中搜索」兜底。
- 远程预览只走文本：`preview::read_text_preview()` 通过 VFS 读取至多
  256 KiB；缩略图、媒体标签、Qt 媒体播放，以及「用本地应用打开 / 在文件
  管理器中显示」仍按 `is_local_path()` 关闭，回车提示先复制到本地。
- 回收站与终端对远程禁用：右键菜单和命令面板不再提供「移到回收站」和
  「在终端中打开」；Delete 在远程改走永久删除确认；终端按钮与快捷键在远程
  变灰，核心 `trash_entries()` / `open_terminal()` 仍以 `fs.trash_local_only` /
  `fs.terminal_local_dir_only` 兜底。
- 验证：`fs::search` 的单元测试覆盖名称/内容搜索的远程拒绝；远程冒烟测试
  删除后端 `search()` 调用并断言 `fs.search_local_only`。回归
  `cargo test -p dae-core --lib` **301 passed / 10 ignored**，
  `cargo test -p dae-qt --lib` **133 passed**；改动 QML 通过 `qmllint`
  （只有既有的 unqualified-access / incompatible-type 警告）。

**已知边界**：FTP/WebDAV 仍在 `vfs::Scheme` 与连接对话框协议表中保留，
`vfs::resolve()` 返回不支持、没有后端实现（`ConnectDialog` 标注「即将支持」）。
真实 SMB/SFTP/云盘登录需要可达服务器与 OAuth 凭据，本次只验证了失败路径、
保存/删除与本地 UI 状态。

### 4.9 `dae://` 深链与单实例（2026-10-26）

**单实例在 Qt 进程里落地。** `cpp/activation.cpp` 用 `QLockFile`
（`%TEMP%\dae-qt-activation-<home hash>.lock`）判定第一个进程；胜者建立
`QLocalServer`（`UserAccessOption`），并把对象注册成 QML 单例
`dae.activation/Activation`。后来者把 argv[1..] 连同自己的 cwd 用带版本号的
`QDataStream` 发过去，调用 `AllowSetForegroundWindow(ownerPid)` 后以 0 退出，
不创建引擎或窗口。若转发早于第一个窗口完成，路径留在 `Activation` 的 pending
列表；QML 的 `Component.onCompleted` 再取，启动竞态不会丢。

**深链解析。** `crates/dae-qt/src/activation.rs` 支持 query、path 与裸 path
三种拼法；只解 `%XX`，不再把 `+` 当空格；相对路径按启动进程的 cwd 解析，文件
落到父目录，不存在的目标丢弃。`config.rs` 的启动目录与
`StartSource::DeepLink` 共用这套解析。Windows 上胜者写
`HKCU\Software\Classes\dae`（`URL Protocol`、`DefaultIcon`、
`shell\open\command` 指向当前 exe），便携版不需要安装器；每次启动按 exe 实际
位置重写，移动后第一次启动即恢复。

**外部打开与「从标签恢复」。** `qml/Shell.js` 记住最后活动窗口，并跨窗口查找
同一路径；`ShellWindow.qml` 的 `receiveActivation()` 先用
`dae_qt_focus_window`（`SW_RESTORE` + `SetForegroundWindow`）把窗口连同原有标签
恢复到前台，目标已在某个标签里就切过去（不复制），否则在活动窗口新开标签。
启动参数也走同一条 pending 管线，所以冷启动 URL 与 `dae.exe <dir>` 都只会给
起始目录开一个标签。状态栏的启动来源新增「深链」。

**实测（2026-10-26，Windows 11 build 26340 / 150%）**：

| 步骤 | 结果 |
|---|---|
| 启动 A；再 `dae.exe "dae://open?path=B"` | 第二个进程退出码 0，只剩 1 个进程；原窗口标题变 B，标签数由 1 变 2，A 保留 |
| 再深链 A | 标题回 A，仍 2 个标签（复用已有标签，不复制） |
| 再深链一次 A | 仍 2 个标签，焦点在 A |
| 最小化后深链 C | `IsIconic=false`，标题 C，标签数 3；窗口恢复到前台 |
| 冷启动 `Start-Process "dae://open?path=B"` | 由协议启动一个新进程，标题 B，状态栏「启动: 深链」 |
| 运行中由 Explorer 打开 `dae://…` | 走注册命令启动的第二个进程转发后退出，原窗口切换目标，仍 1 个进程 |
| `HKCU\Software\Classes\dae\shell\open\command` | 指向当前构建的 exe + `"%1"` |

`dae-qt` **58 通过**（新增三种 URL 拼法、相对 cwd、文件→父目录、`+` 不解码、
去重、注册命令引号与启动来源标签）。

**已知边界**：协议注册在运行期由 Windows 的 HKCU 路径完成；macOS 的
`CFBundleURLTypes`（`dae://`）与 Linux 的 `.desktop`
`x-scheme-handler/dae` 由 §4.10 的发布包携带。深链只接受已存在的本地目录，
远程 `smb://` / `cloud://` 目标不通过 URL 进入。单实例按用户隔离，没有
「强制新实例」开关。

### 4.10 打开方式选择器与自动更新（2026-10-26）

**打开方式：Windows 继续用系统对话框，macOS/Linux 用自绘选择器。**
`Workspace.openWithPickerSupported` 按目标平台取 `false` / `true`；右键
「打开方式」在 Windows 仍调用 `Workspace.openWith()`（`SHOpenWithDialog`
同时提供“始终使用”关联），其余平台打开 `qml/OpenWithDialog.qml`。数据由
`crates/dae-qt/src/open_with_model.rs` 提供：`list_open_with_apps()` 的
`default` / `recommended` / `other` 三组被压成带 section header 的扁平
`QAbstractListModel`，过滤按应用名做大小写不敏感包含匹配，组内没有命中时连
header 一起消失；默认应用同时出现在“推荐”组并带 `isDefault` 徽标，条目总数
超过 8 个才显示过滤框。确认后调用
`Workspace.openWithApp(path, appId, setDefault)`，由 core 的 `open_with_app`
写关联或直接启动；错误经 `errors.rs` 映射成「这个项目已经不存在」/
「当前系统不支持选择打开方式」。

图标复用已有的 `image://fileicon/` provider，只新增 `name/<icon>` 分支：
Linux 的 `icon_name` 可能是冒号分隔的候选集，原样交给
`dae_core::fs::icons::named` 依次尝试；macOS 没有图标名，委托用应用名首字母
占位。键盘上下选择、Enter 确认、Esc 取消、双击直接打开，都与其它对话框一致。

**自动更新用独立 feed，不碰 Tauri 的 `latest.json`。** 地址是
`https://github.com/Melaine-iceberg/dae/releases/latest/download/latest-qt.json`，
格式与 Tauri 同形：

```json
{
  "version": "0.6.0",
  "notes": "…",
  "pub_date": "2026-10-26T00:00:00Z",
  "platforms": {
    "windows-x86_64": { "url": "dae-0.6.0-windows-x86_64.zip", "signature": "…" }
  }
}
```

平台键为 `windows-x86_64` / `windows-aarch64` / `darwin-x86_64` /
`darwin-aarch64` / `linux-x86_64` / `linux-aarch64`。更新包是普通 zip：
Windows/Linux 的根目录内容镜像安装目录（必须含 `platforms/`），macOS 的 zip
内放一个 `dae.app`。签名沿用发布用的 minisign 公钥（常量
`UPDATE_PUBLIC_KEY` 在 `dae-core/src/update/mod.rs`）；下载后先验签、再解压，
zip 拒绝绝对路径与 `..`，Unix symlink 拒绝绝对目标和越界 `..`。

**安装由 detached 脚本完成。** 应用进程不覆盖正在映射的 Qt DLL / `.so` /
exe。`InstallLayout` 从 `current_exe()` 反推安装位置：Windows/Linux 要求 exe
同目录有 `platforms/`（所以 `target/debug` 不会自更新），macOS 要求 exe 位于
`dae.app/Contents/MacOS`。新包先解到同卷的 `.dae-update-<pid>-<nanos>` staging，
确认包内确有将被启动的可执行文件，然后启动脚本：

- Windows：`powershell -File` + `Wait-Process`，复制 staging → 安装目录，
  `Start-Process` 重启；失败写 `%TEMP%\dae-update.log`。
- Linux：`/bin/sh` 轮询 `kill -0` 等进程退出，`cp -R -p` 覆盖后重启；日志
  `$TMPDIR/dae-update.log`。
- macOS：旧 `.app` 改名 → 移入新 bundle → 失败回滚 → `open -n`。

路径全部以 argv 传给脚本，不做字符串插值，空格与引号由 OS 处理。

**UI。** `crates/dae-qt/src/updater.rs` 注册成 QML 单例 `Updater`
（`supported` / `checking` / `checked` / `available` / `latestVersion` /
`notes` / `downloading` / `installProgress` / `restarting` / `error`）。第一个
窗口在 4 s 后静默检查一次，失败只写 stderr；发现新版本时 toast 提示。状态栏
的「检查更新 / 更新可用 / 更新中… / 重启中…」按钮打开
`qml/UpdateDialog.qml`，显示当前版本、release notes、下载进度与安装提示。
安装脚本启动成功后 `Updater.restartRequested` 让第一个窗口 `Qt.quit()`，
脚本随即替换文件并重启。

**测试开关。** debug 构建默认不启用更新（`cfg!(not(debug_assertions))`），且
部署目录旁边必须有 `platforms/`；`DAE_UPDATE_DEV=1` 放开前者，
`DAE_UPDATE_ENDPOINT` 替换 feed 地址（例如本地 HTTP 服务器），用于不重新
打包就走通检查与下载。

**验证（2026-10-26，Windows 本机 debug 构建）**：`dae-core` 236 通过、
`dae-qt` 68 通过；新增覆盖 feed 选版（含“已是最新时忽略缺失平台”）、缺平台、
可选字段、minisign 样例、zip 路径穿越拒绝、staging 唯一性与脚本参数化。
debug 构建启动 10 s 无 QML 加载错误。Windows 的“打开方式”仍由系统对话框
承担；自绘选择器的 macOS/Linux 渲染与关联写入未在本机实测。

**发布流水线（2026-10-26 已闭环）**：`.github/workflows/release.yml` 新增
`qt` 矩阵，分别用 `windeployqt`、linuxdeploy + Qt 插件、`macdeployqt` 部署
Qt 运行库，按上面的布局生成 `dae-<version>-<platform>.zip`。zip 用发布私钥
（`TAURI_SIGNING_PRIVATE_KEY`）经 `tauri signer sign` 签名；`qt-feed` 作业
合并各平台 artifact，运行 `scripts/qt-update-feed.mjs` 校验 zip 布局与签名
后写出 `latest-qt.json`，最后先上传 zip、再上传 feed，全部追加到 Tauri 作业
创建的 draft release。feed 的 `signature` 直接内嵌 Tauri 生成的 base64
`.sig`；`dae-core` 的验签同时接受 base64 与明文 minisign。macOS 的
`CFBundleURLTypes` 与 Linux 的 `x-scheme-handler/dae` 随打包模板一起输出。
发布仍由人工点击 publish；release 公开后
`releases/latest/download/latest-qt.json` 即成为正式版的更新源。

### 4.11 分配器随壳迁移：mimalloc 进 Qt 壳（2026-10-27）

**搬的是什么。** Tauri 壳的 `#[global_allocator]`（`src-tauri/src/main.rs`）原样
搬到 `apps/dae/src/main.rs`，依赖从 `src-tauri/Cargo.toml` 复制到
`apps/dae/Cargo.toml`，`default-features = false` 保持不变——`secure` 会给每次
分配加 canary，直接改掉 §4.1 引用过的 −31% / −38% 那组数字。

**为什么位置重要。** `#[global_allocator]` 由链接最终可执行文件的 crate 安装；
`dae-qt`、`dae-core` 在这里以 rlib 形式进来，把声明放进它们会让分配器 shim 变成
链接顺序依赖（并在将来某天被编成 cdylib 时变成第二份堆）。因此它留在 bin 里。

**能力边界。** 它只覆盖 Rust 侧分配。QML 引擎、`QString`、`QQuickItem` 与场景图
的分配发生在预编译 Qt DLL 内部，其 import table 直指 `ucrtbase.dll`，exe 里的
`malloc` 符号拦不住；Windows + 动态 Qt 下"整进程换分配器"不可达，除非静态 Qt +
`-DMI_OVERRIDE` 自建。收益本身不在这里：48.3 → 32.6 ms 全部来自 `dae-core` 的
目录枚举路径，随 crate 一起搬过来了。

**顺手补上的一处遗漏。** 根 `Cargo.toml` 原先只有 `[workspace]`，没有
`[profile.release]`；`src-tauri` 被 `exclude`，它的 `panic = "abort"` /
`lto = "fat"` / `codegen-units = 1` / `strip = true` 从来不作用于 `apps/dae`。
现在根清单按字段对齐复制了一份，两个壳在 `src-tauri` 退役前保持一致。

**验证（2026-10-27，本机 debug）**：`cargo build -p dae-app` 通过；新
`target/debug/dae.exe` 内可见 `mimalloc_version` / `mimalloc: error:` 等字符串，
确认 C 运行库已链入；另用一次性探针 crate（同配置、`[global_allocator]` +
`Vec` 分配）实测可链接并正常运行，证明该属性在 bin target 上成立。跨语言释放
失配的静态核对结论不变：`crates/` 下没有 `into_raw` / `Box::leak` /
`mem::forget` 这类把所有权丢给 C 侧释放的写法。注意本机增量构建需要给
`cxx-qt-build` 一个 qmake（`QMAKE=` 指到 `bin/qmake.exe`，或把 `bin` 放进
`PATH`，CI 走的是后者）；`QT_ROOT_DIR` 只有部署步骤读，构建阶段不读——见
`release.yml` 上方的注释。

**关于 qmake 的角色。** 它在这里不是构建系统，只是一个元数据查询器：
`qt-build-utils` 调 `qmake -query QT_INSTALL_HEADERS / QT_INSTALL_LIBS /
QT_INSTALL_PREFIX` 拿到头文件与库路径，然后交给 `cc` crate 去调 `cl.exe`。
0.10 只有 `qmake` 与 `qt_minimal`（下载一份预编译 Qt）两个后端，没有 CMake
后端；仓库里也没有任何 `CMakeLists.txt`。`moc` / `rcc` / `qmltyperegistrar` /
`qmlcachegen` 同样是被直接当 exe 调用，不经过 CMake 或 qmake 生成的 Makefile。

### 4.12 空间、概览、命令面板与侧栏目录树（2026-10-27）

**空间是单例，不是每窗快照。** `crates/dae-core/src/fs/spaces.rs:35` 的
`list_spaces` / `create_space` / `add_item` 等已经是完整实现，Qt 侧只补
`crates/dae-qt/src/spaces_model.rs`：`#[qml_singleton]` + `QAbstractListModel`，
`count` / `revision` / `error` 三个属性，`changed()` 信号。空间文档只有几 KB，
创建空间后还要在同一事件里拿到 id 打开视图，因此读写是同步的；异步化只会让
"新建空间并添加"多一次时序分支。`revision` 让 `Tab.qml` 的标题这类"调用
invokable 的绑定"在重命名后重新求值。

**概览是扁平的分段行。** `overview_model.rs` 把收藏、最近、空间、项目四个来源
摊平成一张表，角色是 `section/kind/name/path/detail/icon/isDir/branch/
changeText/timeText`，QML 侧一个 `ListView` 加 `section.property` 即可。
基本面（收藏/最近/空间）同步构建，页面同一事件可用；最近目录背后的 git 项目
异步扫描，`generation` 防止旧扫描覆盖新刷新。项目按仓库根去重，最多四个，变更
数只统计 `Added/Modified/Untracked`。

**表面（surface）取代单布尔。** `Tab.qml` 的 `trash: bool` 换成
`surface: "folder" | "trash" | "overview" | "space"` + `spaceId`，并保留
`readonly property bool trash` 等高阶布尔供旧绑定读取。窗口的路径栏、搜索、
终端、预览、分栏统一以 `folderActive` 为门槛；非文件夹表面只显示
`surfaceTitle`。从概览/空间/回收站新建标签页会继承当前表面（`addTabHere`），
外部 `dae://` 激活只复用 `folder` 标签（`Shell.js`），不会把概览悄悄替换成目录。

**命令面板复用模型，不另建数据源。** `CommandPalette.qml` 每次重建都从
`Spaces`、本窗口的 `OverviewModel`、静态命令表和自己的 `SearchModel` 取候选；
输入非空时做有序子序列 fuzzy 排序（词首、连续命中加权），再按分组装回
`section`，保证标题不会浮在无关行上。文件搜索在 220 ms 防抖后异步跑，结果按
`SearchModel.query` 与当前输入对齐后才并入，避免旧查询的回包串台。命令 id 是
动作名而不是菜单项名；"移到回收站"与"打开回收站"因此是
`trash-selection` 和 `trash` 两个 id。

**侧栏目录树按卷懒加载。** `sidebar_tree_model.rs` 每个卷一个模型：根目录只在
展开时读，子目录只在点击 chevron 时读，展开集合与缓存都在模型里，delegate 被
`ListView` 回收不会丢状态。`SidebarTree.qml` 用 `Repeater` 渲染模型已经摊平的
`depth` 行；不嵌套第二个 `ListView`，滚轮只属于侧栏本身。`Sidebar.qml` 增加
固定的"概览"行、空间 section 行（`kind === "space"`）、新建空间行，以及卷行
的 chevron；展开状态存在 `Sidebar` 根对象，`Loader` 折叠时释放该卷的树。

**验证（2026-10-27，本机 debug）**：`cargo test -p dae-qt` 114 项全过；
`cargo build -p dae-app` 通过；在 Windows 平台实跑应用做 QML 冒烟：分别以
`query = "doc"` 打开命令面板、打开概览面、打开预设空间面，并实例化
`rootPath = C:\` 的 `SidebarTree`，均无 QML 运行时错误。侧栏 chevron 的鼠标
交互未做自动化点击，只核对了模型单测（展开后的深度优先顺序、隐藏目录过滤）与
`Loader` 的绑定路径。

### 4.13 设置页与国际化（2026-10-27）

**设置面板分五页。** `qml/SettingsPanel.qml` 现在是
`appearance / shortcuts / terminal / defaultManager / logs`：外观页选
`system|light|dark` 与 `system|brand` 强调色，语言页选
`system|zh-CN|en`；快捷键页按 Rust 动作目录分三组列出 20 个动作，可录制、
清空、单条重置或全部重置，并在提交前检测同表冲突；终端页保留原来的字号/
行高/字体族与保存按钮；默认文件管理器页沿用平台注册命令；日志页显示
`appLogDir` 并提供“打开目录”。

**外观与其他设置统一存储。** 快捷键、终端排版、主题、强调色与语言都写
`%APPDATA%\com.melaine.dae\settings.toml`（`dae_core::settings` 的 20 个
action id 与默认值），Tauri 壳读写同一份文档；旧 `appearance-qt.ini` 只在启动时
导入 TOML 中缺失的外观字段，不再写入。三项外观默认值均为 `system`。
`ShellWindow` 把 `dark`/`accent` 传给所有窗口与自绘控件，
`Main.qml` 与撕出窗口不各自读偏好。

**快捷键是注册表，不是散落的 `Keys` 处理器。** `qml/Shortcut.js` 负责
`Mod+...` 与 Qt 键名的双向转换、录制、显示与匹配（Qt 序列要求单字母大写，
例如 `Ctrl+K`）；`ShellWindow` 暴露
`shortcutBinding/shortcutDisplay/shortcutSequence/shortcutMatches`，全局
动作、`FileList`、右键菜单、`CommandPalette`、`FilterMenu`、`OverviewView`
都从 `Workspace.shortcutsJson` 取当前绑定。录制时 `SettingsPanel.capturing`
让 ShellWindow 的 Escape 暂时失效，Esc 只取消录制、不关闭面板。

**翻译不经过 lrelease。** `qml/i18n/dae_en.ts` 与 `dae_zh_CN.ts` 由
`lupdate` 生成/更新并打进 QRC（`/i18n`）；`crates/dae-qt/cpp/translator.cpp`
用 `QXmlStreamReader` 直接解析 TS 的 `<context>/<message>`，安装后调用
`QQmlEngine::retranslate()`。中文是源语言，运行时不装 translator；
`system` 跟随 `QLocale::system()`，`zh*` 用中文，其余用英文。32 个 QML
文件的可见文案全部改为 `qsTr()`（686 条 message，英文全部填充），
`Sidebar`/`Overview`/状态栏把共享后端返回的常见固定串（`N 项`、`今天`、
操作 toast、进度词等）在显示层映射；用户数据（收藏名、空间名、文件名）不译。

**验证（2026-10-27，本机 debug）**：`cargo test -p dae-qt` 119 项全过；
`cargo check -p dae-app --all-targets` 与 `cargo build -p dae-app` 通过；
`qmllint` 无新增告警（仅既有的 unqualified access 提示）。实跑切换到
English 后截图核对：设置五页、侧栏、列表表头（Name/Type/Size/Modified）、
状态栏（`199 items`）、命令面板快捷键提示均为英文；录制快捷键写回
`settings.toml`，冲突提示、清空与重置生效；`Ctrl+K` 打开命令面板，
录制中按 Esc 只退出录制。后续更新：外观偏好已统一写入 `settings.toml`，
`friendly_error()` 与 Rust 侧状态、进度和操作结果均返回英文，QML 显示层再按当前语言映射。

### 4.14 预览深度：Markdown、代码高亮与内嵌媒体（2026-10-28）

**文本渲染在后端做，QML 只负责显示。** `crates/dae-qt/src/preview_markup.rs` 把
Markdown 译成 `QTextDocument` 支持的 HTML 子集（pulldown-cmark：表格、删除线、
任务列表、脚注、定义列表），把源码交给 syntect 着色（`default-fancy`，纯 Rust，
不引入 onig）。主题切换不需要重读文件：`PreviewController` 缓存源文本与
`SourceKind`，QML 的 `setDark()` 只重跑渲染。原始 HTML、远程图片与非
`http/https/mailto` 链接一律降级为文本；相对链接和图片按源文档路径解析成绝对
路径/`file://` URL，点击本地链接走 `Workspace.openPath`，网页链接走
`Qt.openUrlExternally`。

**代码高亮限制在 64 KB 读取窗口内**，与旧壳一致；普通文本仍是可选择的
`PlainText`，Markdown/代码用只读 `RichText`。Markdown 里的围栏代码块走同一个
syntect 渲染器，未识别的语言退化为等宽纯文本。

**媒体在面板内播放。** `qml/MediaPreview.qml` 用 Qt Multimedia 的
`MediaPlayer` + `VideoOutput`/`AudioOutput`：视频以缩略图作海报，点击后播放；
音频显示图标；两者共享播放/暂停、拖动进度、时间与静音控件。不做自动播放，
切换选中项即 `stop()`；远程条目不进入播放器，仍只显示元数据。组件通过
`Loader` 按 `mediaKind/mediaUrl` 懒加载，模块缺失时显示「当前构建不支持媒体播放」
而不是让整个预览面板加载失败。

**QtMultimedia 成为构建/部署依赖。** 本机 Qt 6.12 已通过 MaintenanceTool 装
`qt.qt6.6120.addons.qtmultimedia`；`release.yml` 的 `install-qt-action` 增加
`modules: qtmultimedia`，三种部署步骤都校验 `qml/QtMultimedia` 存在。
若 `target/debug` 里已有一份旧的 `windeployqt` 部署（本机曾如此），QML 导入路径会
固定到 `target/debug/qml` 而缺少 QtMultimedia；重新跑一次
`windeployqt --release --qmldir qml target/debug/dae.exe` 即可，临时也可用
`QML_IMPORT_PATH` / `QT_PLUGIN_PATH` 指向 Qt 安装目录。release 由
`windeployqt` / `macdeployqt` / `linuxdeploy` 自动带入。Qt 富文本表格只有在
`border="1"` 且
`cellspacing ≥ 1` 时才画网格，`border-collapse:collapse` 反而会让边框消失，
这是 §4.14 实测后写进渲染器的约束。

**验证（2026-10-28，Windows 11 build 26340 / 150%）**：`dae-qt` 129 项单测通过
（新增 Markdown 结构/转义、相对链接、代码着色、文件 URL 与媒体分类）；
`qmllint` 对两个新视图无语法告警（`dae` 模块与 `Loader.item` 的动态成员除外）；
真机选中 `README.md` 看到标题/粗斜体/列表/表格/链接/本地图片/围栏代码高亮，
选中 `.rs` 看到注释与关键字着色；`fire.mp4` 播放出画面、进度与时间推进，
3 秒 WAV 播放时进度条推进、按钮切到暂停。

## 5. 定案前必须量出来的四组数字

| # | 待验证 | 门槛 | 不达标怎么办 |
|---|---|---|---|
| 1 | Rust `QAbstractListModel` + QML `ListView.reuseItems` 跑 12 万行 + 增量补丁 | **已达标：**每帧工作 p95 最差 **5.68 ms**（3×12 s），PSS 滚动峰值最差 **123.7 MB**（全 CJK）—— §5.1；画真实 OS 图标后同会话 A/B：work **1.60–1.66 ms**，PSS **+3–4 MB** —— §5.1.1 | 委托改为 `QQuickItem` 自绘 / 分页模型 / 回到 egui |
| 2 | QML 自定义样式 `TextInput` 里的中文 IME | **已达标：**7 场景 214 项检查全过；预编辑与 `GCS_COMPSTR` 全程一致、候选锚点偏差 ≤ 0.43 px、Esc 与换焦点取消都成立（需壳里的 `CPS_CANCEL` 守卫）—— §5.2 | 该输入框回退到 `QTextEdit`-based 原生控件（未触发） |
| 3 | Win11 无边框 + Mica + 自绘标题栏 + 拖动/缩放/贴靠 | **已达标：** 63 项检查全过；拖动/缩放走原生 modal loop，Snap Layouts 悬停出菜单并贴到左半屏；DWM 明暗切换 34 帧采样无闪、0 `WM_ERASEBKGND` —— §5.3 | 保留系统标题栏（未触发） |
| 4 | 单窗口撕出 / 关闭的内存与延迟增量 | **已达标：** 6 轮开/关，撕出延迟最差 **76.82 ms**；第二窗口 PSS 增量最差 **13.74 MB**（冷启动一轮），热态 +5.12–6.24 MB；关闭后私有提交 6 轮仅 +0.56 MB —— §5.4 | 恢复预热池（未触发） |

对照基线（现有实测）：**447–483 MB PSS**（1 窗口）、**+190–200 MB / tab**、
WebView2/WebKitGTK 创建 **270–340 ms**、预热池 **+200 MB**。

### 5.1 门槛 1 实测：12 万行列表（2026-10-25，Windows 11）

**通过。** 12 万个真实目录条目经 `open_streamed_listing`（512 条头 + 18 个流式批次，
共 19 批）进入 Rust `QAbstractListModel`，QML `ListView.reuseItems: true` 渲染。
负载包含每秒一次的随机跳转和每 250 ms 对 24 个可见行的一次 `dataChanged`。

| 指标 | 3 次 12 s（vsync off） | 门槛 |
|---|---:|---:|
| **每帧工作 p95**（GUI 忙时 + 渲染 sync+paint） | **3.93 / 5.68 / 2.14 ms** | ≤ 8 ms |
| GUI 线程忙时 p95 | 2.84 / 4.71 / 1.15 ms | — |
| 渲染线程 sync+paint p95 | 1.09 / 0.98 / 0.99 ms | — |
| `frameSwapped` 间隔 p95（120 Hz） | 9.77 / 12.23 / 9.03 ms | — |
| **PSS** | **61.2 / 58.2 / 57.8 MB** | ≤ 250 MB |
| 委托对象创建数 | 236（三次相同） | 证明 `reuseItems` 生效 |

补充实测：

- **全 CJK 名字**（120k，最坏情况）：工作 p95 **2.93 ms**，PSS 结束值 **115.6 MB**、
  滚动中峰值 **123.7 MB**。首个 CJK 名字触发字体回退，一次性增加约 52 MB
  （loaded 40.7 MB → settle 结束 92.4 MB），滚动前几秒字形缓存继续升温到 ~124 MB。
- **这不是泄漏**：混合名字的 60 s 无跳转运行从 begin 的 47.4 MB 涨到 ~55 MB 后
  平坦，结束时 56.7 MB；60 s 带跳转的运行在 44.1 MB 起步，全程在 57–58.6 MB 之间，
  没有出现字体回退台阶。混合 fixture 只有在随机跳转恰好落到 CJK 名字时才会付这笔
  52 MB，已提交的运行没有命中。真实壳应当在启动时预载 CJK 回退字体，否则滚动到
  第一个中文名字会同步付出这笔加载。
- **vsync on** 的同一负载：工作 p95 **2.47 ms**，与 vsync off 持平——瓶颈不在垂直同步。
- **236 个委托** ≈ 2,600 px `cacheBuffer` 覆盖的行数；12–60 s 的滚动没有新增委托，
  `reuseItems` 确实在复用。

**8 ms 门槛量的是「每帧工作量」，不是 `frameSwapped` 间隔。** 本机显示器是 120 Hz，
空列表的间隔也是 8.33 ms；实测 `QSG_NO_VSYNC=1` 在 D3D11 后端也解不开呈现节流。
所以间隔只能说明有没有掉帧，成本必须拆开量：GUI 线程用
`QAbstractEventDispatcher` 的 `awake → aboutToBlock` 忙时，渲染线程用
`beforeSynchronizing → afterRendering` 的 sync + paint。GUI 线程在 sync 期间阻塞，
两段互斥，把两个 p95 相加是保守的每帧成本。

复现、fixture 生成和全部原始 JSON：[`bench/qt-list-spike/`](../../bench/qt-list-spike/README.md)。

### 5.1.1 图标接入后的复测（2026-10-26）

行委托现在画真实的 OS 图标（§4.7）。spike 链接 `crates/dae-qt` 并注册同一个
provider，`--icons on|off` 在同一会话里交替跑三次；夹具与 §5.1 相同（120,000
条、24 个可见行、每秒随机跳转、每 250 ms 补丁）。

| 12 s | work p95（无图标 / 有图标） | PSS（无 / 有） |
|---|---:|---:|
| #1 | 1.82 / **1.68 ms** | 59.1 / 62.8 MB |
| #2 | 1.64 / **1.60 ms** | 59.3 / 63.3 MB |
| #3 | 1.65 / **1.66 ms** | 58.9 / 62.1 MB |
| 全 CJK | 1.98–2.02 / **1.99 ms** | 106.3–109.0 / 110.1 MB |
| 30 s soak（有图标） | **1.55 ms** | 111.3 MB |

- **每帧工作没有可测增量。** 三对 on/off 的 work p95 差在 ±0.14 ms 内；有图标的
  三次数值甚至略低，这是运行间噪声，不作为提速解释。8 ms 门槛仍然宽裕。
- **PSS +3–4 MB。** 夹具只有文件，所有行共享 `type/<ext>` URL，整个运行只有
  14 次 provider 调用和 14 张纹理；增量来自这些纹理和 provider 的 512 项字节缓存。
- **30 s soak 的 111 MB 是 CJK 字体回退**：第 6 s 一次随机跳转落到中文名，
  PSS 50.5 → 109.2 MB 后 25 s 持平，不是图标累积。
- **绝对值低于 §5.1 的 10-25 记录**（当时的 2.1–5.7 ms 不再复现）；同一负载在
  同一台机器上受会话状态影响，门槛判断用同会话 A/B，不看跨日绝对值。
- **覆盖范围**：文件夹具只测类型图标。目录/快捷方式按路径取图标的那条路没有
  被这条 A/B 覆盖；它受可见行数与 512 项缓存约束，改动时应另建目录夹具。

原始 JSON：[`bench/qt-list-spike/results/`](../../bench/qt-list-spike/results/) 下的
`win11-120k-{icons,noicons}-off-*.json`、`win11-120k-cjk-noicons-*.json`、
`win11-120k-icons-cjk.json`、`win11-120k-icons-off-30s.json`；窗口截图
`win11-120k-icons-window.png`（图标确实在跑，不是 provider 静默失败）。

### 5.2 门槛 2 实测：中文 IME（2026-10-25，Windows 11）

**通过。** 7 个场景、214 项检查全部通过。环境：Windows 11 Insider / Qt 6.12.0 /
3840×2160 @ 150%（DPR 1.5）/ 微软拼音（HKL `0x08040804`，`TextInputHost.exe`）。

三个输入框都是自定义样式（自绘背景、焦点环、光标 delegate、选区颜色）：
A 是路径栏，`textChanged` 实时回写镜像；B 贴屏幕右下角；C 在 `contentY=140`
的 `Flickable` 里。探针在 Rust/cxx-qt 侧，直接读 IMM32 而不只信 QML：

- QML `TextInput.preeditText` vs `ImmGetCompositionStringW(GCS_COMPSTR)`；
- 提交后的 `text` 增量 vs `GCS_RESULTSTR`；
- Qt 交给 IME 的 `CANDIDATEFORM`（`CFS_EXCLUDE`，客户端像素）vs
  `QInputMethod.cursorRectangle() × devicePixelRatio`；
- `SendInput` 注入真实按键；候选窗出现时用 BitBlt + `CAPTUREBLT` 抓桌面。

| 场景 | 输入 | 预编辑（QML = IMM32） | 结果 |
|---|---|---|---|
| 正常提交 | `nihao` | `ni'hao` | 提交 `你好` |
| 退格纠正 | `nihao` → 2×Backspace → `ao` | `ni'hao` → `nih` → `ni'hao` | 提交 `你好` |
| Esc 取消 | `zhongwen` | `zhong'wen` | 预编辑清空、文本保持 `keep`、无提交 |
| 快速连打（35 ms/键） | `zhongwenshuru` | `zhong'wen'shu'ru` | 提交 `中文输入` |
| 屏幕右下角 | `nihao` | `ni'hao` | 候选窗翻到光标上方，未越界 |
| 点击换焦点 | `shijie`，再点 A | `shi'jie` | 预编辑清空、B 保持 `keep`、无原始拼音 |
| 滚动容器 | `wenjian` | `wen'jian` | 提交 `文件`，候选窗跟随内容偏移 |

- **预编辑不丢字**：在每个记录点（快速连打取整串），QML `preeditText` 与
  `GCS_COMPSTR` 的字母序列完全一致；68 次注入（字母、空格、Esc、退格）全部送达；
  提交增量与 `GCS_RESULTSTR` 逐字相同。IME 自己加的隔音撇号（`ni'hao`）两边都在。
- **候选窗不偏位**：7 个组合快照里，`CANDIDATEFORM` 与
  `cursorRectangle × 1.5` 的最大偏差 **0.43 px**（x）、**0 px**（y）。
  150% 缩放下成立；`contentY=140` 的滚动偏移被计入；窗口贴屏幕右下角时
  IME 把候选窗翻到输入框上方。`ImmGetCandidateList` 在 TextInputHost 的
  IMM 桥上返回空，候选词内容与最终窗口位置用截图核对。
- **组字可取消**：Esc 后 `WM_IME_ENDCOMPOSITION`、预编辑清空、文本不变、无提交。
- **失焦不是免费的**：直接点另一个输入框时，Qt 6.12 的
  `QWindowsInputContext::reset()` 会把**原始组合串**（`shi'jie`）当作 commit
  string 发给旧控件，而不是取消。这不是 `TextInput` 的问题：`reset()` 在平台
  输入上下文里，`QTextEdit` 收到的是同一个 `QInputMethodEvent`。spike 里加了一个
  约 50 行的窗口事件过滤器：鼠标按下到焦点项之外或按 Tab 时，先
  `ImmNotifyIME(NI_COMPOSITIONSTR, CPS_CANCEL)`，再让 Qt 换焦点；验证后 B 保持
  `keep`、预编辑清空。**壳里必须带这个守卫。**

**结论：保留 QML 自定义样式 `TextInput`，不触发 `QTextEdit` 回退。**
门槛 2 没有推翻文本层结构；唯一新增的是上面那个「换焦点前取消组合」的原生守卫。
复现、脚本、原始 JSON 与三张只含应用内容的截图：
[`bench/qt-ime-spike/`](../../bench/qt-ime-spike/README.md)。

### 5.3 门槛 3 实测：无边框 + Mica + 自绘标题栏（2026-10-25，Windows 11）

**通过。** 63 项检查全过。环境：Windows 11 Version 26H2 / build 26340 /
Qt 6.12.0 / D3D11 / 3840×2160 @ 150%（工作区 3840×2088）。

窗口用 `QQuickWindow::setDefaultAlphaBuffer(true)` + QML `color: "transparent"`，
再由 `cpp/window_probe.h` 走 winit/Tauri 的同一组 raw Win32：
`DwmEnableBlurBehindWindow`（空 blur region，让 DWM 在 client 后面合成）+
`DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW/
TRANSIENTWINDOW)`。材质决策照搬 `window_material/windows.rs`（`CurrentBuild`、
`EnableTransparency`、22000/22523 两条线），本机判定是 Acrylic；门槛另外强制
测了 Mica，两个方向的 read-back 分别是 **2** 和 **3**，`none` 是 **1**。

| 项目 | 实测 |
|---|---|
| 拖动 | 真实鼠标 **+80, +55 px**；`WM_NCLBUTTONDOWN` = HTCAPTION；`WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` 各 1 |
| 缩放 | 左边 **+120 px**（宽 +120、x −120）；右下角 **+60 × +40**；八个 resize code 全部命中 |
| 最大化 | 自绘按钮 → client 正好是工作区 **(0,0,3840,2088)**；最大化时边缘返回 HTCLIENT；还原回到原 rect |
| Snap Layouts | 悬停最大化按钮 1.5 s 后 flyout 出现（屏幕 diff **25,291 px**）；点左半区后窗口 = **(0,0,1920,2088)** |
| DWM 切换 | 34 帧 / ~20 ms；暗 **25.09**、亮 **155.99**；中间 18 帧是 DWM 自己的 cross-fade（约 150–200 ms）；黑/白/越界帧 0；`WM_ERASEBKGND` 0；Qt 重绘 **1** 帧（采样前的状态栏更新） |

**这不是“没有动画”，而是“没有闪”。** 切换只改 immersive-dark，不动 QML，
所以任何异常帧都只可能来自 DWM；实测是一段单调的 cross-fade，没有黑帧、白帧
或超出两端 ±6 的帧。第一次测量时采样步骤走 QML plan，plan 的属性写入让 Qt
每帧重绘一次，掩盖了 DWM 的贡献；改成单个 20 ms Timer 后 Qt 只落了 1 帧。
Snap flyout 由 DWM/Shell 合成，**不出现在 `EnumWindows` 里**，所以“菜单出现了”
是用悬停前后的屏幕像素差来判定的，而不是窗口枚举。

**三件只有接上 Windows 才知道的事**（壳里必须照此实现）：

1. **最大化按钮必须常驻 `HTMAXBUTTON`。** Snap Layouts 的悬停菜单只认
   `WM_NCHITTEST` 的这个返回值；Windows 不会在按下时重跑 hit test，所以
   “悬停返回 HTMAXBUTTON、按下返回 HTCLIENT”这种取巧不成立。代价是这个
   按钮拿不到 QML hover，壳里要用 `WM_NCMOUSEMOVE` + `TME_NONCLIENT` 转发
   悬停状态。最小化/关闭按钮则可以是普通 client 区域。
2. **DefWindowProc 的 non-client 按钮跟踪会吞掉 `WM_NCLBUTTONUP`。**
   过滤器从头到尾看不到 up（实测 `nc_up=0`）。在跟踪循环里调用
   `ShowWindow`/`showMaximized` 会让 Qt 重入并崩溃；正确做法是轮询
   `GetAsyncKeyState(VK_LBUTTON)`，等物理按键抬起、循环退出后再调用
   `QWindow::showMaximized()/showNormal()`。同理，过滤器里不要自己调
   `DefWindowProc` 跑 move loop——让它由 Qt 的 WndProc 在返回后调用。
3. **最大化的 client 不等于工作区。** `WS_THICKFRAME` + Qt frameless 的
   `WM_NCCALCSIZE=0` 让最大化窗口的 client 等于 window rect，而 Windows 会把
   后者扩到工作区外加一条不可见 resize border——150% 下每边 11 px。
   必须在 `WM_NCCALCSIZE` 里对 `IsZoomed` 的窗口把 client 设成
   `MONITORINFO.rcWork`，否则内容从三边压到任务栏下面。

另外，`WM_MOVING`/`WM_SIZING` 不会进 `QAbstractNativeEventFilter`（它们在
Qt 的 `isInputMessage` 里被排除）；判断原生 modal loop 是否跑过要看
`WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE`。复现、脚本、逐帧数据和截图：
[`bench/qt-window-spike/`](../../bench/qt-window-spike/README.md)。

### 5.4 门槛 4 实测：撕出 / 关闭的增量（2026-10-25，Windows 11）

**通过。** 167 项检查全过。源窗口与撕出窗口共享同一个 120,000 行
`QAbstractListModel`——撕出移动视图，不复制行数据；第二个窗口走 §5.3 的
同一条原生路径（按 HWND 建样式、应用 Mica、应答 hit test）。环境同 §5.3：
Windows 11 Version 26H2 / build 26340 / Qt 6.12.0 / D3D11 / 3840×2160 @ 150%。

| 指标 | 6 轮实测 | 门槛 |
|---|---:|---:|
| **撕出延迟**（request → 新窗口首帧） | 67.21 / 59.08 / 60.56 / 76.82 / 58.52 / 59.40 ms，最差 **76.82 ms** | ≤ 150 ms |
| **PSS 增量**（开窗 − 同轮基线） | **+13.74 MB**（冷），之后 +6.24 / +5.48 / +5.79 / +5.12 / +5.27 MB | ≤ 20 MB |
| 关闭后 PSS（相对同轮基线） | 首轮 +9.49 MB，之后 +0.61 / +0.47 / +1.56 / −0.54 / −0.66 MB | — |
| 6 轮关闭后 PSS | 105.92 → 109.70 MB（**+3.78 MB**） | — |
| 6 轮关闭后私有提交 | 178.64 → 179.20 MB（**+0.56 MB**） | — |

- **冷启动一轮、热态五轮。** 第一个额外窗口付 **~14 MB PSS / ~25 MB 私有提交**
  的一次性缓存（渲染循环、QML 类型、字体），关闭后留下约 **9.5 MB PSS**；第 2–6 轮
  只加 **5.12–6.24 MB**，关闭回到各自基线 **+1.6 MB** 以内。私有提交 6 轮持平
  （+0.56 MB），所以关闭后 PSS 的 +3.78 MB 是常驻共享页/页缓存与采样噪声，不是窗口泄漏。
  把六轮平均会掩盖这个二分，因此表里把冷轮和热态区间分开写。
- **不需要预热池。** 旧池用 **+200 MB** 换 ~65 ms 中位收养；这里真实窗口
  **59–77 ms**，且 12 万行模型共用，没有复制。延迟拆开看：
  QML 实例化 **0.8–1.6 ms**、建原生窗口 + 样式 + Mica 并显示 **10–18 ms**、首帧渲染
  （管线 / 交换链 / 纹理上传）**~47–66 ms**——最贵的是首帧，不是建窗。第一轮建窗最慢
  （显示 18.1 ms），最慢的首帧出现在第 4 轮（76.8 ms），这是 D3D 首次呈现的正常波动。
- **多窗口路径逐项复测**（都作用在第二个 HWND，而不是沿用源窗口的状态）：
  `WS_THICKFRAME | WS_MAXIMIZEBOX` 存在、client 覆盖整窗；DWM 背板读回 **2**、
  暗色 **true**；`WS_EX_LAYERED` + blur-behind 成立；hit test 的 caption **2**、
  close **1**、max **9**、left border **10**；真实鼠标拖动 **+60, +40 px**，
  该 HWND 自己收到 `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE`；帧计数按窗口；
  `WM_CLOSE` → QML `closing` → 销毁后源窗口继续工作。
- **“撕出”的边界**：150 ms 从脚本请求窗口开始量，不含触发它的那一次拖拽；
  拖拽本身已由 §5.3 的原生 modal loop 验证。测量期间源窗口保持打开并继续渲染，
  所以 PSS 增量是新视图的**总额**；真实撕出若同时关掉源标签，只会更便宜。

复现、6 轮逐项 JSON 和每轮窗口截图：
[`bench/qt-window-spike/`](../../bench/qt-window-spike/README.md)。

## 6. 已知代价（写在前面，不做事后解释）

- 视图层是 QML + JS，不是 Rust。心智模型与 React 接近，但确实是第二种语言。
  边界严格：QML 只放布局、委托、动画，逻辑全在 Rust。
- 双工具链：Qt 安装（CI 用 aqtinstall）、`windeployqt` / `macdeployqt` / `linuxdeploy`，
  安装包预计 **+60–120 MB**。
- PGO / BOLT 要改造：`scripts/opt/pgo.mjs` 只 profile Rust 侧，cxx-qt 生成的 C++
  需单独加 `-fprofile-instr-generate`；先不管 C++ 侧，量过再决定。
- 内存不会到 Slint 那个量级。目标从「30 MB」改成「**≤ 250 MB 且不随标签线性增长**」。
  门槛 1 实测 12 万行：混合名字 ~57 MB、全 CJK ~116 MB（滚动峰值 123.7 MB），
  没有随滚动时间增长（§5.1）。门槛 4 实测多窗口：第一个额外窗 +13.74 MB（含一次性
  缓存），热态每个窗 +5.12–6.24 MB，关闭后私有提交不增长（§5.4）。
- **多窗口的冷/热成本要分开看**（§5.4）：第一次撕出会留下 ~9.5 MB PSS / ~25 MB
  私有提交的一次性 Qt/D3D 缓存，之后的窗口关闭后回到基线。把冷轮的 14 MB 当成
  「每窗口成本」会高估，把六轮平均又会低估第一轮的观感；壳的预算按稳态 5–6 MB
  算，启动后首个撕出按 15 MB 留余量即可。
- **Windows IME 的失焦语义**：Qt 6.12 的 `QWindowsInputContext::reset()` 在焦点对象
  变化时把原始组合串提交给旧控件；壳里必须在换焦点前调用 `CPS_CANCEL`（§5.2 的
  守卫，约 50 行）。换 `QTextEdit` 不能规避——行为在平台输入上下文，不在文本控件。
- **自绘标题栏的原生细节**（§5.3）：最大化按钮必须常驻 `HTMAXBUTTON`，而
  DefWindowProc 的 NC 按钮跟踪会吞掉 `WM_NCLBUTTONUP`——壳里必须等物理按键
  释放后再调用窗口状态 API；这个按钮的 hover 也要用 `WM_NCMOUSEMOVE` 自己
  转发。最大化时 `WM_NCCALCSIZE` 还必须把 client 修正到工作区。

## 7. 回退

旧壳在 P4 之前**保持可构建、可发布**。任何一个 spike 门槛不达标，回退成本是
`git revert` 这一串垫片 + 放弃 `qml/`，`dae-core` 的产出**不浪费** ——
它可以原样喂给 egui 或 Slint。这是把「先剥 core」放在「先定引擎」之前的唯一理由。

## 8. 附：工具链已实测（Qt 6.12.0 + cxx-qt 0.10 + VS 18 Insiders）

§6 里最大的未知数是「双工具链」：cxx-qt 要在 Rust 的构建里塞进 C++ 编译和
Qt 的链接。这条链现在通了，复现代码在 `bench/qt-toolchain/`。

### 8.1 安装：aqt 的 PyPI 版本装不了 6.11+

**这是这次踩到的最贵的坑，写下来给 CI 用。** Qt 从 **6.11 起改了 Windows
仓库布局**：不再有 `qt6_6110/qt6_6110/` 这一层，改成每种编译器一个子目录
（`qt6_6120/qt6_6120_msvc2022_64/`）。aqtinstall 的 **PyPI 最新版 3.3.0 不认这个布局**
（[#1007](https://github.com/miurahr/aqtinstall/issues/1007)、
[#1022](https://github.com/miurahr/aqtinstall/issues/1022)），它去要
`qt6_6120/qt6_6120/Updates.xml`，404，然后报一句容易误读的
「Failed to download checksum … This may happen on unofficial mirrors」。
修复只在 git master 上（`v3.3.1.dev166`），没有发版。

实际可用的命令：

```bash
uvx --from "git+https://github.com/miurahr/aqtinstall" aqt install-qt \
  windows desktop 6.12.0 win64_msvc2022_64 -O C:/Qt \
  --archives qtbase qtdeclarative qtsvg qttools qttranslations d3dcompiler_47 opengl32sw \
  -m qtshadertools qtimageformats
```

两个必须记住的分类（弄错就是上面那条 404 同级的报错）：

| | 名字 | 传参方式 |
|---|---|---|
| **base archives**（默认装，可省略） | `qtbase qtdeclarative qtsvg qttools qttranslations d3dcompiler_47 opengl32sw` | `--archives …` |
| **模块 / addons** | `qtshadertools qtimageformats`（还有 qtcharts / qtmultimedia / qtwebengine …） | `-m …` |

`-m qtdeclarative` 会失败：它已经是 base archive，不是模块。`qtdoc` 被显式排掉
（200 MB 文档，spike 用不上）。

装完 **2.5 GB**，落在 `C:/Qt/6.12.0/msvc2022_64`。没有 Qt 账号（`aqt` 走的是
开源仓库），没有 Qt Creator。

### 8.2 装出来的东西（已核对）

- `qmake6` / `qmllint` / `qmlls` / `windeployqt` / `qml` 齐全 —— `qmlls` 是 VS Code
  做 QML 类型检查的那一环。
- `QtQuick/Controls` 风格有 **Basic / Fusion / FluentWinUI3**（最后一个正好是
  Windows 11 观感，可以作为设计的对照物）。
- `QtQuick/Effects` 在（`effectsplugin.dll`、`BlurItem.qml`）—— 圆角/模糊/阴影
  的 `MultiEffect` 可用，这是"美学"那一半的前提。

### 8.3 ABI 风险已排除

Qt 6.12 的官方 Windows 二进制是 **v143（MSVC 14.4x）** 编的，这台机器是
**VS 18 Insiders / MSVC 14.52／x64**。§6 里把它列为"可能需要退到 VS 2022 Build
Tools"的风险 —— **实测不需要**：cxx-qt 0.10 生成的 C++ 用 14.52 编译，与 Qt 的
静态导入库链接干净，没有 LNK 警告。

### 8.4 端到端证据

`bench/qt-toolchain/` 是一个 5 文件的最小 QML 应用：Rust 里 `#[qobject]` 的类型
由 `cxx-qt-build` 注册成 QML 模块 `dae.smoke`，QML 在 `Component.onCompleted`
里调 `incrementNumber()`。跑出来的输出：

```
rust: starting
rust: number -> 2
```

第二行只可能来自「QML 解析到模块 → 实例化 Rust 支撑的 QObject → 调用
`#[qinvokable]`」这一整条链。**`qml/` 这一层写 QML、`crates/dae-qt` 这一层写
Rust 对象的分工，从这一刻起是有实测支撑的，不再是纸面方案。**

### 8.5 工具链统一到官方安装器（2026-10-26）

§8.1 的 aqt 安装记录保留为踩坑历史；当前工具链以本节为准。Qt 现在由官方
Qt Online Installer 装在 `K:/Qt`：

- 组件：`qtbase qtdeclarative qtsvg qttools qttranslations qtshadertools
  qtimageformats`，另加 **Agentic Development Tools → QML Live Preview for
  AI Agents**（提供 `bin/QmlPreviewMcpServer.exe`）。
- 安装器同时带 `K:/Qt/Tools/{CMake_64,Ninja,QtCreator}`，不再依赖单独的
  CMake/Ninja 安装。
- 构建仍用 `QMAKE="K:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"` 并把该 `bin/`
  放进 `PATH`；`dae-app` 已在新 Qt 上完成 debug 构建并冒烟启动。
- `QmlPreviewMcpServer.exe` 已注册到 Qoder（用户级 `qmlpreview`），工具名为
  `run_qmlPreview`。
- 实时预览需要 `QT_QML_DEBUG`。`crates/dae-qt/build.rs` **只在 debug
  profile** 定义它，`cpp/qml_debug.cpp` 包含 `<QtQml/qqmldebug.h>` 以装入
  `QQmlDebuggingEnabler`；release 构建不定义，调试服务不会随包发布。

### 8.6 因此仍未解决的一件事

1. **PGO / BOLT 的 C++ 那一半** —— 见 §6，仍未动。

（`window_material` 的 Mica/Acrylic 已接到 `QWindow` 的 HWND 上，见 §5.3。）
