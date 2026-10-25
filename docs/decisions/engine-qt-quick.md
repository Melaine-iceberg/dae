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

## 5. 定案前必须量出来的四组数字

| # | 待验证 | 门槛 | 不达标怎么办 |
|---|---|---|---|
| 1 | Rust `QAbstractListModel` + QML `ListView.reuseItems` 跑 12 万行 + 增量补丁 | **已达标：**每帧工作 p95 最差 **5.68 ms**（3×12 s），PSS 滚动峰值最差 **123.7 MB**（全 CJK）—— §5.1 | 委托改为 `QQuickItem` 自绘 / 分页模型 / 回到 egui |
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

### 8.5 因此仍未解决的一件事

1. **PGO / BOLT 的 C++ 那一半** —— 见 §6，仍未动。

（`window_material` 的 Mica/Acrylic 已接到 `QWindow` 的 HWND 上，见 §5.3。）
