# 引擎从 WebView 换成 Qt Quick：**已定方向，spike 未做**

> **状态：方向已定，证据未到。** 分支 `refactor/qt-quick`。本文记录的是
> 「为什么离开系统 WebView」「为什么是 Qt Quick 而不是 egui / Slint / GPUI」
> 「许可证为什么是 GPL-3.0-or-later」，以及**定案前必须量出来的四组数字**。
> 其中的门槛数字全部是待验证的假设，不是结论 —— 与 `docs/decisions/README.md`
> 「先量，再改」的约定一致：这份文档的第一版是**假设清单**，spike 结束后回填实测值。

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
   跟随、组字取消）。这是 egui 最弱、而本项目不能妥协的一块。
2. **`QAbstractListModel` + `ListView.reuseItems`** 是 12 万行列表的原生解，
   KDE 系文件管理器用的就是这套；`listing.rs` 的增量打补丁直接映射成
   `beginInsertRows` / `dataChanged`。
3. **多窗口成本是 10–20 MB 量级，不是 200 MB。** 撕出标签变成「开一个窗口」，
   预热池这套机械装置的**存在理由消失**。
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

已完成的第 1 片（本分支）：`file_system/types.rs`（600 行）与
`file_system/error.rs` 进入 `dae-core`，`src-tauri` 侧只剩两个垫片。
选它们是因为它们是**契约** —— 后面的每个模块都依赖它们，先搬完就能一直往里搬。

## 5. 定案前必须量出来的四组数字

| # | 待验证 | 门槛 | 不达标怎么办 |
|---|---|---|---|
| 1 | Rust `QAbstractListModel` + QML `ListView.reuseItems` 跑 12 万行 + 增量补丁 | 滚动 p95 ≤ 8 ms，PSS ≤ 250 MB | 委托改为 `QQuickItem` 自绘 / 分页模型 / 回到 egui |
| 2 | QML 自定义样式 `TextInput` 里的中文 IME | 预编辑不丢字、候选窗不偏位、组字可取消 | 该输入框回退到 `QTextEdit`-based 原生控件 |
| 3 | Win11 无边框 + Mica + 自绘标题栏 + 拖动/缩放/贴靠 | 全正常，DWM 切换不闪；复用 `window_material` 的 raw Win32 | 保留系统标题栏（放弃自绘 chrome） |
| 4 | 单窗口撕出 / 关闭的内存与延迟增量 | ≤ 20 MB / 窗口；撕出 ≤ 150 ms | 恢复预热池（但这意味着内存目标放弃） |

对照基线（现有实测）：**447–483 MB PSS**（1 窗口）、**+190–200 MB / tab**、
WebView2/WebKitGTK 创建 **270–340 ms**、预热池 **+200 MB**。

## 6. 已知代价（写在前面，不做事后解释）

- 视图层是 QML + JS，不是 Rust。心智模型与 React 接近，但确实是第二种语言。
  边界严格：QML 只放布局、委托、动画，逻辑全在 Rust。
- 双工具链：Qt 安装（CI 用 aqtinstall）、`windeployqt` / `macdeployqt` / `linuxdeploy`，
  安装包预计 **+60–120 MB**。
- PGO / BOLT 要改造：`scripts/opt/pgo.mjs` 只 profile Rust 侧，cxx-qt 生成的 C++
  需单独加 `-fprofile-instr-generate`；先不管 C++ 侧，量过再决定。
- 内存不会到 Slint 那个量级。目标从「30 MB」改成「**≤ 250 MB 且不随标签线性增长**」。

## 7. 回退

旧壳在 P4 之前**保持可构建、可发布**。任何一个 spike 门槛不达标，回退成本是
`git revert` 这一串垫片 + 放弃 `qml/`，`dae-core` 的产出**不浪费** ——
它可以原样喂给 egui 或 Slint。这是把「先剥 core」放在「先定引擎」之前的唯一理由。
