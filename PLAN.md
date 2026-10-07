# 迁移计划：标签页拖动改为 webview reparent（浏览器语义）

> 状态：**计划已定，尚未开始改造应用代码**。可行性 PoC 已完成并提交。
> 分支：`try/webview-reparent`（2 个 commit：`648127f`、`56bcbe4`）。`main` 未受影响。
> 本文档自包含，可在新对话里直接接手。原始实测日志见 `NOTES.md`（被 .gitignore 忽略，仅本地）。

---

## 1. 目标与决策

- **目标**：拖动标签页时，把**已有的 webview** 挂到另一个窗口，不重载页面、不丢 JS 状态 —— 与浏览器移动标签页的语义一致。
- **现状**：`src-tauri/src/tab_windows.rs` 把 tab 状态序列化（`serializeTabHandoff`）后**新建窗口 + 新 webview**，由 `src/main.tsx:54` 的 `takeTabHandoff` 复原。JS 上下文全部丢失、有冷启动闪烁。
- **已决定**（用户选定）：**全量迁移到 per-tab webview**，拖动走 `Webview::reparent`。

---

## 2. 已实测结论（Linux / WebKitGTK 2.54.1 / Wayland / niri）

PoC：`src-tauri/src/reparent_poc.rs` + `public/reparent-poc.html`，由 `DAE_POC_REPARENT=1` 门控，
日志写 `/tmp/dae-reparent-poc.jsonl`。

| 场景 | 结果 |
|---|---|
| `Window` + `add_child` 的 webview reparent | ok |
| **真 `WebviewWindow`** 的 webview → 另一个 `WebviewWindow` | ok |
| 拖回来（往返） | ok |
| 关掉被搬空的源窗口 | ok（`dst_still_registered=false`，不炸） |

跨 reparent 全程：`boot` id 不变、`ticks` 22→59→101→149 连续、`scrollY` 保留、
`__TAURI_INTERNALS__.invoke` 存活、JS 堆探针对象存活。→ **同一 JS 上下文，未重载**。

关键结论：

1. `Webview::reparent` 里 `CannotReparentWebviewWindow` 的检查是 `#[cfg(not(feature = "unstable"))]`。
   **开 `unstable` 即整段跳过** → 不需要把窗口改造成 `Window`+`add_child`，真 `WebviewWindow` 也能搬。
   目标窗口用 `app.get_window(label)` 拿 `&Window`。
2. Tauri 记账自动正确：`Window::webviews()` 按 `window_label()` 过滤，移动后计数自动跟上。
3. **JS 侧 `currentWindow.label` 在 reparent 后过期**（init script 只在页面加载时注入一次），
   而 `currentWebview.label` 稳定且正确。→ 迁移后**一律按 webview label 索引**。
4. 内存代价（实测 RSS 求和，`/tmp/tree_rss.py`）：
   - 基线（1 窗口 1 webview）：**838 MB**（dae 350 + WebKitWebProcess 409 + 网络进程 79）
   - PoC（3 个存活 webview）：**1520 MB**，4 个 `WebKitWebProcess`：399/235/231/214 MB
   - → 每多一个 webview ≈ **+200~340 MB RSS**。RSS 重复计共享页，真实私有增量更低，
     **待用 PSS 复核**（脚本 `scripts/dev/webview-memory.py` 已支持）。

---

## 3. 目标架构

```
窗口 (transparent, window_material 不变)
├── shell webview     全窗口、透明。tab bar / sidebar / 状态栏 / 命令面板 / 对话框 / 快捷键
└── content webview   每 tab 一个，内嵌在内容区，auto_resize(true)，叠在 shell 之上
```

- **tab id == content webview label**（稳定，跨 reparent 不变）。
- shell 的 webview label == 窗口 label（沿用现有 `main` 等）。
- 一个窗口可以有 N 个 content webview + 1 个 shell（PoC 已验证单窗口可承载多个 webview）。

**状态归属**

| 归 shell | 归 content |
|---|---|
| 窗口内 tab 列表、激活 tab、窗口布局 | 该 tab 的 surface 渲染（6 种） |
| 设置、主题、i18n | 目录列表、选中项、滚动位置 |
| 命令面板、对话框、右键菜单外壳 | 每 tab 导航历史、内联重命名草稿 |
| 快捷键分发、拖拽编排 | 该 tab 的文件操作上下文 |

现有 `WorkspaceSurface` 共 6 种：`overview` / `recents` / `favorites` / `trash` / `space` / `folder`
（`src/features/workspace/types.ts:11-16`）。终端是**面板**（`src/features/terminal/terminal-panel.tsx`），
不是 tab surface → 不参与 per-tab webview 拆分。

**通信**：两个 webview 不能直接互相 invoke。
- shell ↔ content：经 Rust 中继（`emit_to(EventTarget::webview(label))` 或 `app.get_webview(label).eval(...)`）。
- 需要一个共享的 typed 协议层（TS 共享类型 + Rust 侧事件枚举）。

**Rust 侧索引改造**：现在按 `window.label()` 的地方要改成 webview label。
已知点：`src-tauri/src/tab_windows.rs:182,187`；前端 `src/lib/window-focus.ts`、
`src/lib/app-window.ts` 的 `getAppWindow()`、`src/main.tsx:54` 的 `takeTabHandoff(appWindow.label)`。

---

## 4. 六个硬问题与对策

1. **跨 webview 的 HTML5 DnD 失效**（最大隐性成本）。
   现在依赖同 webview 内的 DnD：`src/features/explorer/file-list.tsx`(onDragStart/onDrop)、
   `explorer-view.tsx`、`sidebar.tsx`（收藏 / Space 落点）、`drag-drop.ts`（hit-test）。
   拆开后文件列表在 content、侧栏与标签栏在 shell → HTML5 DnD **不能跨 webview**。
   对策：自定义拖拽协议
   a) content 检测 dragstart → payload（paths/类型）经 Rust 广播给同窗口 shell 与其他 content；
   b) 拖动期间指针位置：shell 用 `Window::cursor_position()` 轮询，或 content 在 dragover 时上报；
   c) 落点判定：shell 区域自己 hit-test；content 区域让目标 content 在自己的 DOM 里 hit-test；
   d) drop 由各自执行（调 Rust 文件命令）。
2. **OS 拖入路由**：`onDragDropEvent` 是窗口级（`src/features/explorer/use-explorer-events.ts:213`），
   拆开后事件落在 shell。必须按指针位置路由到对应 content webview 再 hit-test。
3. **透明窗口 + 透明 shell webview 叠加**（`window_material`，`src-tauri/src/window_material/`）。
   需验证：shell 透明处能透出 window_material、content 不透明且盖在上层、z-order 正确。
4. **键盘焦点 / 快捷键跨 webview**：焦点在哪个 webview、快捷键由谁处理（建议 shell 统一分发）。
5. **内存随 tab 数线性上涨**（见 §2.4）。需定上限策略（见 §6）。
6. **窗口 label 过期**（见 §2.3）→ 全量改用 webview label，否则会操作到旧窗口。

---

## 5. 分阶段执行计划（每阶段必须可验证）

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **P0 骨架** | 新 entry（shell / content 两种角色，可由 URL query 或 label 区分）+ Rust 侧 content webview 注册表 + 单 tab + 拖动 reparent | 两窗口间拖动一个 tab，`boot` id 不变、`ticks` 连续 |
| **P1 布局** | shell 全窗口透明 + content 内嵌内容区 + `auto_resize(true)` + 多 tab | resize 后 content 跟随；透明处 window_material 正常；z-order 正确 |
| **P2 表面迁移** | 6 种 `WorkspaceSurface` 在 content 里渲染 | 每种 surface 在 content webview 中可用 |
| **P3 通信协议** | 路径栏 / 选中 / 命令 / 右键菜单 / 快捷键的 shell↔content 协议 | 现有交互不回退（逐项对照） |
| **P4 拖拽重建** | 跨 webview DnD + OS 拖入路由（§4.1、§4.2） | 文件拖到侧栏/标签栏/另一窗口都可用 |
| **P5 生命周期与内存** | tab 创建/关闭时的 webview spawn/回收、内存策略、启动时序 | 内存可测、Ctrl+T 无不可接受延迟 |
| **P6 跨平台与打磨** | Windows / macOS 行为、DPI、焦点、拖拽视觉 | 三平台一致 |

工作量级（粗估）：P0 半天、P1 1 天、P2 1–2 天、P3 1–2 天、P4 2–3 天、P5 1 天、P6 待定。
**P4 是体验回退风险最高的部分**，建议单独排期并优先做技术验证。

---

## 6. 待拍板问题（执行前需要用户确认）

1. **内存策略**：接受随 tab 数线性上涨，还是设上限（超出回退内联渲染 → 但那部分 tab 拖出会重载）？
2. **新建 tab 是否立即 spawn webview**？立即 spawn 则 Ctrl+T 有 ~200MB 进程创建延迟；延迟 spawn 则首次拖动要等。
3. **拖动期间的指针定位**用「shell 轮询 `cursor_position()`」还是「content 上报坐标」？（影响 P4 实现与流畅度）
4. **Windows / macOS 如何验证**？目前只有 Linux / niri / WebKitGTK 2.54.1 一台机器，
   Windows（`SetParent`）与 macOS（`addSubview`）行为**完全未测**。

---

## 7. 复现与执行命令

```bash
git checkout try/webview-reparent

# 跑 reparent PoC（会在桌面弹出测试窗口）
DAE_POC_REPARENT=1 bun tauri dev
python3 -c "import json;[print(json.loads(l)) for l in open('/tmp/dae-reparent-poc.jsonl')]"

# 量内存（进程树 RSS + PSS）
python3 scripts/dev/webview-memory.py $(pgrep -f '[t]arget/debug/dae' | head -1)

# 注意：pkill -f "bun tauri dev" 会匹配到自己的 shell（命令行里含该字符串），
# 要用 pkill -f "[t]arget/debug/dae" 这种方括号写法，且不要在同一个命令里同时写启动命令。
```

PoC 代码位置：
- `src-tauri/src/reparent_poc.rs` —— 三个场景（`Window`+`add_child`、真 `WebviewWindow`、往返+关空壳）
- `public/reparent-poc.html` —— JS 探针（boot id / ticks / scrollY / `currentWindow.label` / `currentWebview.label` / 堆探针）
- `src-tauri/src/lib.rs` —— `DAE_POC_REPARENT=1` 时在 setup 里跑，否则零影响
- `src-tauri/Cargo.toml` —— `tauri = { features = ["macos-private-api", "unstable"] }`（`unstable` 仅 PoC 需要）

---

## 8. 回退与隔离

- `unstable` feature 只存在于本分支；PoC 全部由 `DAE_POC_REPARENT` 门控，不设则完全不影响运行。
- 现有 handoff 路径（`tab_windows.rs` + `main.tsx`）**保持可用**，迁移期间可并行对照，随时可回退到 `main`。
- 已知未验证项（不要当成已解决）：Windows/macOS 行为、透明窗口下的 reparent、
  多 tab 时的输入焦点、PSS 真实私有内存增量、DPI 缩放变化下的 `auto_resize`。
