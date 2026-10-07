# 存档：标签页拖动改为 webview reparent 的可行性 PoC（**路线已否决**）

> ## ⛔ 决定（2026-10-07，最终）：**不迁移**，保留现有「序列化 handoff + 新建窗口」重建式拖动
>
> 否决理由（用户评估）：
>
> 1. wry 的 `reparent` 依赖 `tauri` 的 `unstable` feature，是**不稳定接口**，跨平台兼容面不可控
>    （Windows `SetParent` / macOS `addSubview` 完全未测，见 §6.4）；
> 2. **每个 tab 一个 webview**，内存随 tab 数线性上涨 —— 实测每 tab ≈ **200 MB PSS**（§2.4），
>    4 tab ≈ 0.8 GB、8 tab ≈ 1.8 GB，代价过高；
> 3. 拖动逻辑要**从头重写**（§4.1/§4.2 的跨 webview DnD 与 OS 拖入路由、§6.3 的坐标来源），
>    而 §4.3 还要求手工 `set_bounds`（否则 GTK 上下平分多个 webview）；
> 4. 收益不成立：现有重建式拖动**并不慢**，冷启动闪烁不足以支撑上述代价。
>
> **附注（2026-10-07 核对 `main` 后补）**：PoC §6.3 当时把「Wayland 下 `window.screenX/screenY`
> 恒为 `[0,0]`」列为需拍板的阻塞点，结论是「只能走 Rust 侧 `outer_position()` + `cursor_position()`」。
> **`main` 的现有实现早就是这个做法**（原生拖动 + 33 ms 悬停轮询），所以这条不是新增工作量，
> 也意味着迁移在交互层几乎拿不到额外收益 —— 又一个否决理由。
>
> **因此**：`main` 继续用 `src-tauri/src/tab_windows.rs` 的原生拖动 + payload 交接 + `src/main.tsx:54`
> `takeTabHandoff` 复原。**本文档 §3、§5、§6 不再是要执行的计划**，只作 PoC 记录与结论留存；
> 唯一幸存的可执行项是 §4.7（见下）。
>
> ---
>
> 以下为原 PoC 文档。可行性 PoC 已完成并提交，2026-10-07 复测通过
> （PSS 复核 + 场景 4：**跑真 app bundle 的 webview 被 reparent，JS 上下文存活**，见 §2.5）。
> 分支：`try/webview-reparent`（5 个 commit，清单见 §9）。`main` 未受影响。
> 本文档自包含，可在新对话里直接接手。原始实测日志见 `NOTES.md`（被 .gitignore 忽略，仅本地）。
>
> ### ✅ 与标签页方案解耦、仍然有效的发现：§4.7 关窗后 `WebKitWebProcess` 不退出
>
> **这个 bug 属于现有架构**，与是否 reparent 无关：现在每开一个标签窗口再关掉，就有概率留下一个
> web 进程。根因已定位到 wry 的 GObject 引用环（`wry/src/webkitgtk/`），6 行 patch 本地验证有效
> （注册 webview 数 3/4 → 3/3，tree PSS 735 → 669 MB），仅影响 GTK 系。
> 上游未修 → 怎么带 fix 见 §6.5。**这是唯一值得单独排期的遗留项。**
> **2026-10-07 拍定：先记账，不排期**（不引入 vendor wry 的长期维护成本，等它真的影响日常
> 使用再动手）。埋点：开开关关多个标签窗口后，看 `WebKitWebProcess` 个数与 tree PSS 是否回落。
>
> **复测新增的坑**：关掉窗口的 `WebKitWebProcess` 不退出（§4.7，现有架构已存在，根因定位到 wry
> 的 GObject 引用环，6 行 patch 本地验证有效，怎么带待拍板 §6.5）；`WebviewWindow::webviews()`
> 计数不按窗口过滤（§2.2）；新父窗口标题不跟随 `document.title`（§4.8）；
> 同一窗口的多个 webview 是**上下平分而不是叠加**，布局必须显式设 bounds（§4.3、§2.5）。

---

## 1. 目标与决策

- **目标**：拖动标签页时，把**已有的 webview** 挂到另一个窗口，不重载页面、不丢 JS 状态 —— 与浏览器移动标签页的语义一致。
- **现状**（已于 2026-10-07 对照 `main` 核实）：前端 `serializeTabHandoff(tabId)`
  （`src/features/explorer/tabs.ts:122`）把 tab 全部状态序列化成 payload，经
  `commands.mergeTabIntoWindow(target, payload, x, y)`（`explorer-tabs.tsx:736`）交给 Rust，
  Rust 侧暂存后递给接收窗口，接收窗口**新建窗口 + 新 webview**，前端用
  `commands.takeTabHandoff(appWindow.label)`（`src/main.tsx:54`）复原。
  → JS 上下文丢失、有冷启动闪烁，**但指针定位/跨窗口悬停已解决**（见下方附注）。
- **拖动本身是原生 OS 拖动**：`run_native_tab_drag`（`src-tauri/src/tab_windows.rs:440`）调
  `drag::start_drag`（`vendor/drag`）；拖动期间 hover 监控循环按 33 ms
  （`TAB_HOVER_POLL_INTERVAL`，:203）用 `outer_position()` + `cursor_position()` 广播
  `TabDragHover`/`TabDragLeave`。
- ~~**已决定**：全量迁移到 per-tab webview，拖动走 `Webview::reparent`。~~
  **已被本文档顶部 2026-10-07 的否决推翻**：迁移不做，§4.1–§4.8 的对策问题随之作废
  （§4.7 除外，它是现有架构的独立 bug）。

---

## 2. 已实测结论（Linux / WebKitGTK 2.54.1 / Wayland / niri）

PoC：`src-tauri/src/reparent_poc.rs` + `public/reparent-poc.html`。
`DAE_POC_REPARENT=1` 跑静态探针页的三个场景（场景 1–3）；
`DAE_POC_REALTAB=1` 单独跑**场景 4：真 app bundle 的 webview**（两个 `window_material` 窗口载 `index.html`）。
日志都写 `/tmp/dae-reparent-poc.jsonl`。

| 场景 | 结果 |
|---|---|
| `Window` + `add_child` 的 webview reparent | ok |
| **真 `WebviewWindow`** 的 webview → 另一个 `WebviewWindow` | ok |
| 拖回来（往返） | ok |
| 关掉**被搬空的源窗口** | ok（`dst_still_registered=false`，不炸）；但见 §4.7 —— 场景 3 那个窗口当时其实**没被搬空**，关掉后它的 web 进程没退出 |
| reparent 后目标窗口是否真的出图 | ok（grim 截图：页面在新父窗口里正常绘制，非空白；被搬空的源窗口是纯黑） |
| reparent 后新父窗口标题跟随 `document.title` | **不跟随**（见 §4.8） |
| **场景 4：跑真 app 的 webview 被 reparent**（React 树 / IPC / 445 个 DOM 节点） | **ok**，见 §2.5 |
| 场景 4：一个窗口同时承载 2 个真 app webview | 能渲染，但**不是叠加而是上下平分**（841×910 → 各 841×455），见 §4.3 |

跨 reparent 全程：`boot` id 不变、`ticks` 22→59→101→149 连续、`scrollY` 保留、
`__TAURI_INTERNALS__.invoke` 存活、JS 堆探针对象存活。→ **同一 JS 上下文，未重载**。
复测再次得到同样结论：场景 1 `boot=d1579b5h` ticks 24→59；场景 2 `boot=bxjct7v6` ticks 23→59→102→150。

关键结论：

1. `Webview::reparent` 里 `CannotReparentWebviewWindow` 的检查是 `#[cfg(not(feature = "unstable"))]`。
   **开 `unstable` 即整段跳过** → 不需要把窗口改造成 `Window`+`add_child`，真 `WebviewWindow` 也能搬。
   目标窗口用 `app.get_window(label)` 拿 `&Window`。
2. Tauri 记账自动正确 —— 但**只有 `Window::webviews()` 正确**：它按 `window_label()` 过滤，移动后计数自动跟上。
   `WebviewWindow::webviews()` 返回的是**全 app 的 webview 数**（复测 `ww-built` 事件里 src/dst 都是 4，
   当时全 app 正好 4 个）。→ 判「这个窗口还剩几个 tab」只能用 `app.get_window(label).webviews()`，见 §4.7。
3. **JS 侧 `currentWindow.label` 在 reparent 后过期**（init script 只在页面加载时注入一次），
   而 `currentWebview.label` 稳定且正确。→ 迁移后**一律按 webview label 索引**。
4. 内存代价（复测已补 PSS。PSS 按比例分摊共享页，是「这个 webview 消失能还回多少」的诚实数字）：
   - 基线（1 窗口 1 webview，两次独立运行）：tree RSS **718~738 MB** / tree PSS **447~483 MB**
     （dae 188~198 + 主 app webview 205~245 + 网络进程 36~40，PSS 口径）
   - PoC（3 个存活 webview + 1 个未回收）：tree RSS **1478 MB** / tree PSS **735 MB**
   - → **空页面**每多一个 webview ≈ +250 MB RSS / **+90 MB PSS**（RSS 口径高估约 2.7 倍）
   - → 场景 4 直接量「真 app bundle 当 content webview」：主 app webview + 2 个 tab webview
     （3 个注册 webview / 3 个 web 进程，三次独立运行）tree RSS **1483~1570 MB** /
     tree PSS **804~877 MB**；逐进程看三个 app webview 各自 **186~196 MB PSS**
     （dae 本体 209.9 + 网络进程 36.2 + 3×webview）→ **每 tab 边际 ≈ 190 MB PSS**。
     参照主 app webview 自身的 205~245 MB PSS → **按每 tab ≈ 200 MB PSS 规划**，
     8 个 tab ≈ 1.8 GB PSS（还没算 §4.7 的泄漏）。
     （早先想用 dev-invoke 调 `tear_off_tab` 量真实标签窗口的那次尝试作废——那条路径会在 worker
     线程碰 GTK 直接 panic，见 §7。场景 4 用真窗口真 bundle 重做，取代了它。）

### 2.5 场景 4：真 app bundle 的 webview 被 reparent（`DAE_POC_REALTAB=1`）

补的是 P0 里「PoC 搬的是静态探针页，不是真 bundle」那条缺口。两个 `window_material::configure`
的 `WebviewWindow`（`decorations(false)`，载 `index.html`），把源窗口里跑着完整 React app 的那个
webview `reparent` 到目标窗口，再搬回、再搬出，最后关掉**已被搬空**的源窗口。三次独立运行一致
（`boot` 分别 `dtid3iyt` / `jiusrv5f` / `tl3yd2mx`，`ticks` 序列 0→67→125→173→221 完全重合；
下表的 `viewport` / `screen` 从第二次起才采集）：

| 观测 | before | after reparent | back（搬回源窗口） | out-again（再搬出） | after-close |
|---|---|---|---|---|---|
| `boot`（页面加载时生成的随机 id） | `jiusrv5f` | 同左 | 同左 | 同左 | 同左 |
| `ticks`（`setInterval` 计数，重载即归零） | 0 | 67 | 125 | 173 | 221 |
| DOM 节点数 | 445 | 445 | 445 | 445 | 445 |
| `viewport` | `[841, 910]` | **`[841, 455]`** | `[841, 910]` | **`[841, 455]`** | `[841, 455]` |
| `screenX/screenY` | `[0, 0]` | `[0, 0]` | `[0, 0]` | `[0, 0]` | `[0, 0]` |
| `currentWindow.label` | `poc-rt-src` | **仍是 `poc-rt-src`** | — | — | — |
| `currentWebview.label` | `poc-rt-src` | `poc-rt-src`（父窗口变 `poc-rt-dst`） | — | — | — |
| 源窗口 `webviews()` | 1 | 0 | 未采 | 0（故能安全关） | 窗口已注销 |
| 目标窗口 `webviews()` | 1 | 2 | 未采 | 2 | 2 |

- **真 app 的 JS 上下文完整存活**：`boot` 不变、`ticks` 单调连续、445 个 DOM 节点没有重建、
  reparent 之后 `__TAURI_INTERNALS__.invoke('poc_report')` 仍能把日志打回 Rust。
  → 迁移方案的核心前提在真实 bundle 上成立，不只是静态页。
- **§2.3 的 label 过期在真 app 上重演**：搬完后 `currentWindow.label` 仍是 `poc-rt-src`，
  `currentWebview.label` 才跟着父窗口走 → 「一律按 webview label 索引」不是洁癖，是必须。
- **记账跟着走**，`rt-close-empty-shell` ok、`src_still_registered=false`、不 panic
  → **搬空之后关源窗口是安全的**，这正是「拖出成独立窗口」和「拖入合并」两条路径收尾要做的动作。
  且此场景**没触发 §4.7 的泄漏**（全 app 注册 3 个 webview / `WebKitWebProcess` 3 个）
  —— 因为被关的那个窗口当时确实没有 webview。泄漏只发生在「关一个还带着活 webview 的窗口」时。
- **但 webview 不会自动占满新父窗口**：进去后目标窗口把面积**上下平分**给它的两个 webview（§4.3）。
  React 树在 455 px 高的视口里正常重排、没崩 → 只是布局问题，但 reparent 后**必须显式 `set_bounds`**。
- `screenX/screenY` 在 Wayland 下恒为 `[0,0]`（§6.3）；`hasFocus` 恒为 false。
  本机没有注入键鼠的工具，**focus 是否跨 reparent 存活仍未验证**。

---

## 3. ~~目标架构~~（随路线否决作废，仅作记录）

```
窗口 (transparent, window_material 不变)
├── shell webview     全窗口、透明。tab bar / sidebar / 状态栏 / 命令面板 / 对话框 / 快捷键
└── content webview   每 tab 一个，占内容区，叠在 shell 之上（**需显式 set_bounds，见 §4.3**）
```

- **tab id == content webview label**（稳定，跨 reparent 不变）。
- shell 的 webview label == 窗口 label（沿用现有 `main` 等）。
- 一个窗口可以有 N 个 content webview + 1 个 shell（PoC 已验证单窗口可承载多个 webview）。
- ⚠️ 但 GTK 的容器**不会自动叠加**：场景 4 实测一个窗口有 2 个 webview 时是**上下平分**
  （841×910 → 各 841×455），不是 overlay → 「shell 全窗口 + content 盖在上面」这套布局
  必须由 Rust 显式管 bounds，不能指望 `auto_resize`，见 §4.3。

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

## 4. 八个硬问题与对策（**历史记录**：这些是当时预见的坑，大部分已随方案作废；
§4.7 例外，它是现有架构的独立 bug，见文首）

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
   **场景 4 已排除「自动叠加」这个乐观假设**：目标窗口有 2 个 webview 时，GTK 容器把可用面积
   **上下平分**（841×910 → 各 841×455），谁都不是覆盖谁。→ 布局得由 Rust 显式算：
   窗口 resize / tab 数变化 / reparent 之后都要重新 `set_bounds`，shell 走全窗口、
   content 走内容区矩形。透明与 z-order 本身仍未测（本机 `window_material` 在 Wayland 上另有条件）。
4. **键盘焦点 / 快捷键跨 webview**：焦点在哪个 webview、快捷键由谁处理（建议 shell 统一分发）。
5. **内存随 tab 数线性上涨**（见 §2.4，PSS 口径每 tab ≈ 200 MB）。需定上限策略（见 §6）。
6. **窗口 label 过期**（见 §2.3）→ 全量改用 webview label，否则会操作到旧窗口。
7. **关掉窗口不回收 web 进程**（复测新发现，**迁移的前置阻塞项**）。
   干净证据来自 PoC 场景 3（应用内 `window.close()`，不经任何外部通道）：关闭后 Tauri 注册表只剩
   3 个 webview（`main`、`poc-webview`、`poc-ww-src`），但 `WebKitWebProcess` **4 个**，
   6 分钟后仍在（RSS 各 231~245 MB）。被关的 `poc-ww-dst` 确实从 niri 窗口列表里消失了，
   即窗口真销毁、进程没走。新建 webview 时**没有复用**残留进程
   → 不是 WebKit 预热池，是有引用没放掉。
   现有架构同样中招（tear-off 已经是每窗一个 webview），per-tab webview 会把「关一个 tab 泄漏一份」变成主路径。

   **根因已定位，修法已在本地验证 —— 是 wry 0.57.0 的 GObject 引用环：**
   `wry-0.57.0/src/webkitgtk/mod.rs:315` 把 `webview.clone()`（强引用）交给 `attach_ipc_handler`，
   `:730` 又把它塞进 `manager.connect_script_message_received` 的 `'static` 闭包，
   而那个 `UserContentManager` 归 webview 自己持有（`:726`）
   → `WebView → UCM → 闭包 → WebView` 引用计数永不为零；
   `InnerWebView::drop`（`:100-103`）只调 `gtk_widget_destroy`、不做 finalize，
   于是 WebKit 永不释放 web 进程。Tauri 侧是干净的：
   `WebviewWrapper::drop`（`tauri-runtime-wry-2.12.1/src/lib.rs:2392`）正常放掉 `Rc<WebView>`，
   Linux 上刻意保留 `WebContext` 只是为了复用网络进程（tauri#14626）。
   **验证**：把那一处捕获改成 `downgrade()` / `upgrade()`（6 行 diff，
   存于 `/home/melaine/vendor/wry-ipcweak.patch`，patch 后的副本在
   `/home/melaine/vendor/wry-0.57.0-ipcweak`），PoC 立刻从「注册 3 / 进程 4」变成 **3 / 3**，
   三个 reparent 场景与 IPC 全部照旧，tree PSS 735 → 669 MB。
   上游 `wry` 的 `dev` 分支目前**仍是老代码**（`mod.rs:316/738/743`），没有「升级即修」这条路 →
   怎么带这个 fix 见 §6.5。
   另：判「窗口是否已被搬空」只能用 `app.get_window(label).webviews()`（见 §2.2），
   用 `WebviewWindow::webviews()` 会得到全 app 的数目，空壳判定永远不成立。
8. **新父窗口标题不跟随页面**。reparent 后页面反复改 `document.title`，
   `titles-later` 里 src/dst 的标题仍是 Rust 设的 `"PoC source"` / `"PoC target"`。
   → tab 标题必须由 content 经 Rust 中继上报给 shell，不能指望窗口 title。

---

## 5. ~~分阶段执行计划~~（随路线否决作废，不再执行）

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **P-1 进程回收** | §4.7 根因已定位（wry 的 GObject 环），本地 6 行 patch 已验证有效；待拍板怎么带（§6.5） | PoC 侧已达成：注册 webview 数 == `WebKitWebProcess` 数（3/3）。还需在**真实标签窗口**上复验（关掉后进程数与 tree PSS 都回落） |
| **P0 骨架** | 新 entry（shell / content 两种角色，可由 URL query 或 label 区分）+ Rust 侧 content webview 注册表 + 单 tab + 拖动 reparent | 两窗口间拖动一个 tab，`boot` id 不变、`ticks` 连续。**跑真实 app bundle 的 webview 已被 PoC 场景 4 验证（§2.5），P0 只剩「把 PoC 的程序化 reparent 接进真 UI 拖动」** |
| **P1 布局** | shell 全窗口透明 + content 占内容区 + 多 tab | resize / tab 增减 / reparent 之后由 Rust 重新 `set_bounds`，content 尺寸正确；透明处 window_material 正常；z-order 正确。**注意 §2.5：不显式设 bounds 时 GTK 会把窗口上下平分给多个 webview，不是叠加** |
| **P2 表面迁移** | 6 种 `WorkspaceSurface` 在 content 里渲染 | 每种 surface 在 content webview 中可用 |
| **P3 通信协议** | 路径栏 / 选中 / 命令 / 右键菜单 / 快捷键的 shell↔content 协议 | 现有交互不回退（逐项对照） |
| **P4 拖拽重建** | 跨 webview DnD + OS 拖入路由（§4.1、§4.2） | 文件拖到侧栏/标签栏/另一窗口都可用 |
| **P5 生命周期与内存** | tab 创建/关闭时的 webview spawn/回收、内存策略、启动时序 | 内存可测、Ctrl+T 无不可接受延迟 |
| **P6 跨平台与打磨** | Windows / macOS 行为、DPI、焦点、拖拽视觉 | 三平台一致 |

工作量级（粗估）：P-1 0.5 天（根因已定位，只剩「怎么带 fix」＋真窗口复验）、P0 半天、P1 1 天、P2 1–2 天、P3 1–2 天、P4 2–3 天、P5 1 天、P6 待定。
**P4 是体验回退风险最高的部分**，建议单独排期并优先做技术验证。**P-1 是内存策略的前提**，先做。

---

## 6. 待拍板问题（**已作废** —— 仅 §6.5 的 wry fix 仍然待拍）

1. **内存策略**：接受随 tab 数线性上涨，还是设上限（超出回退内联渲染 → 但那部分 tab 拖出会重载）？
   量级参考（PSS 口径）：**每 tab ≈ 200 MB**（场景 4 用真 bundle 实测边际 170~205 MB，见 §2.4），
   4 tab ≈ 0.8 GB、8 tab ≈ 1.8 GB。
2. **新建 tab 是否立即 spawn webview**？立即 spawn 则 Ctrl+T 有 ~200MB 进程创建延迟；延迟 spawn 则首次拖动要等。
   复测后新增一条：§4.7 的 fix 让「关 tab 就真回收进程」成立，所以**池化/复用 webview**
   不再是唯一能同时解决泄漏和 Ctrl+T 延迟的办法了 —— 这个取舍要重新拍。
3. **拖动期间的指针定位**用「shell 轮询 `cursor_position()`」还是「content 上报坐标」？（影响 P4 实现与流畅度）
   场景 4 已把第二个选项**排除**：Wayland 下 `window.screenX/screenY` 恒为 `[0,0]`（客户端拿不到自己在
   合成器里的绝对位置），content webview 自报坐标等于自报一个常数 → **只能走 Rust 侧**
   `Window::outer_position()` + `Window::cursor_position()` 自己算，shell 轮询那条也依赖同一个 Rust 事实。
   **场景 4 已经否掉了「content 上报」这条路**：Wayland 下页面里的 `window.screenX/screenY` 恒为
   `[0,0]`（两次运行 5 次采样全 0），content webview 无从报告自己在屏幕上的绝对位置。
   → 只能用 Rust 的 `Window::outer_position()` + `Window::cursor_position()` 自己算命中区域，
   content 至多上报「指针在我自己视口内的相对坐标」，再由 Rust 加上窗口原点。
4. **Windows / macOS 如何验证**？目前只有 Linux / niri / WebKitGTK 2.54.1 一台机器，
   Windows（`SetParent`）与 macOS（`addSubview`）行为**完全未测**。
5. **怎么带 §4.7 的 wry fix**？上游未修，三条路：
   a) 像 `vendor/drag` 那样把 wry 拷进 `vendor/wry`（1.1 MB，每次升 wry 要重新对 diff）；
   b) 把这 6 行提到上游 wry，期间用 `[patch.crates-io] wry = { git = ... }` 顶着；
   c) 先不带 fix，改用「关 tab 只隐藏、webview 池化复用」绕开 —— 但那等于把泄漏转成常驻，
      且 §6.1 的上限策略要跟着重写。
   注：这个引用环在 `wry/src/webkitgtk/` 里，**只影响 GTK 系（Linux/BSD）**；
   Windows 的 WebView2、macOS 的 WKWebView 是另一套析构，所以 §6.4 的跨平台验证里
   「关 tab 到底还不还内存」三平台各要单独量。

---

## 7. 复现与执行命令

```bash
git checkout try/webview-reparent

# 跑 reparent PoC（会在桌面弹出测试窗口）
DAE_POC_REPARENT=1 bun tauri dev
python3 -c "import json;[print(json.loads(l)) for l in open('/tmp/dae-reparent-poc.jsonl')]"

# 跑场景 4：两个真 app 窗口 + 程序化 reparent（同样会弹窗口，等 6s 让 React 起来）
DAE_POC_REALTAB=1 bun tauri dev
# 探针事件名以 rt- 开头（rt-before / rt-post / rt-after / rt-close-empty-shell …）

# 量内存（进程树 RSS + PSS）
python3 scripts/dev/webview-memory.py $(pgrep -f '[t]arget/debug/dae' | head -1)

# 不用鼠标也能驱动 app：dev 构建起了 tauri-plugin-dev-invoke，直接 HTTP 调命令
# （Origin 必须是 http://localhost:1420；参数用 camelCase，如 grabX/cursorX）
curl -s -X POST http://127.0.0.1:3030/invoke/<command> \
  -H 'content-type: application/json' -H 'Origin: http://localhost:1420' -d '{...}'
# **但别用它调碰 GTK 的命令**：`tear_off_tab` 经 dev-invoke 会在 worker 线程上执行，
# `attach_tab_drop_target`（tab_windows.rs:165）里的 `gtk::TargetEntry::new` 直接 panic
# `GTK may only be used from the main thread.`，窗口建到一半、从未 show，进程还留着一堆半成品。
# → 想量「真实标签窗口」只能靠真人拖标签，或在 setup 里另起一个不碰裸 GTK 的场景
#   （已做：`DAE_POC_REALTAB=1`，全程走 `tauri::async_runtime` + builder/`reparent`，
#   由 runtime 自己派发回主线程，所以不复现这个 panic。见 §2.5）。
# 顺带：`tear_off_tab` 注释里「非 async 命令跑在主线程」这个前提，对 dev-invoke 这条入口不成立。

# 视觉确认 reparent 后是否真的出图（niri 是平铺的，PoC 窗口会被排到屏幕外，先聚焦再截图）
niri msg --json windows | python3 -c "import sys,json;[print(w['id'],w['app_id'],w['title']) for w in json.load(sys.stdin) if w['app_id']=='dae']"
niri msg action focus-window --id <窗口id> && sleep 1 && grim /tmp/shot.png

# 复现 §4.7 的根因验证（patch 后的 wry 副本在店外，不进仓库）
ls /home/melaine/vendor/wry-0.57.0-ipcweak /home/melaine/vendor/wry-ipcweak.patch
# 挂上：src-tauri/Cargo.toml 的 [patch.crates-io] 加一行
#   wry = { path = "/home/melaine/vendor/wry-0.57.0-ipcweak" }
# 然后 cargo build --manifest-path src-tauri/Cargo.toml（约 50s），再跑一遍 PoC：
#   注册 webview 数 == WebKitWebProcess 数 即为修复（改前 3/4，改后 3/3）
# 验证完记得 git checkout -- src-tauri/Cargo.toml src-tauri/Cargo.lock

# 注意：pkill -f "bun tauri dev" 会匹配到自己的 shell（命令行里含该字符串），
# 要用 pkill -f "[t]arget/debug/dae" 这种方括号写法，且不要在同一个命令里同时写启动命令。
# vite 那个子进程的 comm 是 "MainThread"，按名字杀不到；收尾要按端口取 pid：
#   kill $(ss -ltnp | grep 1420 | grep -oP 'pid=\K[0-9]+')
```

PoC 代码位置：
- `src-tauri/src/reparent_poc.rs` —— 场景 1–3（`Window`+`add_child`、真 `WebviewWindow`、往返+关空壳），
  以及场景 4 `run_real_tab`（两个真 app 窗口 + 往返 + 关掉搬空的壳，`DAE_POC_REALTAB=1`）
- `public/reparent-poc.html` —— JS 探针（boot id / ticks / scrollY / `currentWindow.label` / `currentWebview.label` / 堆探针）
- 场景 4 不走静态页，探针由 `probe()` 注入真 bundle（`index.html`），字段与上面同源
- `src-tauri/src/lib.rs` —— `DAE_POC_REPARENT=1` / `DAE_POC_REALTAB=1` 时在 setup 里跑，两个都不设则零影响
- `src-tauri/Cargo.toml` —— `tauri = { features = ["macos-private-api", "unstable"] }`（`unstable` 仅 PoC 需要）

---

## 8. 回退与隔离

- `unstable` feature 只存在于本分支；PoC 全部由 `DAE_POC_REPARENT` / `DAE_POC_REALTAB` 门控，
  两个都不设则完全不影响运行。
  复测确认：不设该变量时 app 正常起窗、内存与 `main` 口径一致，PoC 零影响。
- 现有 handoff 路径（`tab_windows.rs` + `main.tsx`）**保持可用**，迁移期间可并行对照，随时可回退到 `main`。
- PoC 场景 3 的名字有误导（`reparent_poc.rs:253` 注释写「关掉被搬空的源窗口」）：关闭时那个窗口
  **还带着自己的 webview**（`ww-back-post` 里 `dst_webviews=1`），并没有被搬空。
  它证明的是「关邻窗不影响被搬走的 webview」（ticks 到 150），顺带暴露了 §4.7 的进程不回收。
  **真正把源窗口搬空再关的是场景 4**（`rt-before-close-shell` 里 `src_webviews=0`）。
- 已知未验证项（不要当成已解决）：Windows/macOS 行为、透明窗口（`window_material`）下的 reparent
  与 z-order/叠加、鼠标真实拖动标签（P0 未实现，PoC 是 Rust 侧程序化 reparent）、
  多 tab 时的键盘焦点/输入可达（场景 4 里 `document.hasFocus()` 恒为 false；本机 niri 无 `send-keys`，
  也没装 ydotool/wtype，注入不了）、DPI 缩放变化下的 content bounds。
  ~~跑真实 app 的 webview 被 reparent~~ —— **已验证，见 §2.5**。

---

## 9. 归档说明（2026-10-07）

- 本文档原为分支 `try/webview-reparent` 上的 `PLAN.md`。路线否决后移入 `main` 的
  `docs/decisions/`，作为决策记录与实测证据留存。
  **正文基本保持当时原文**；仅在 2026-10-07 归档时改了两处：① 顶部加了否决决定与附注；
  ② §1 「现状」按 `main` 实际代码核实重写（原文对现有拖动机制的描述不准）。
- PoC 代码不在 `main` 上，仍在本地分支 `try/webview-reparent`（无 upstream）：
  `648127f` spike → `56bcbe4` 真 `WebviewWindow`/往返/关闭 → `ba501b8` 本文档 →
  `dc3d865` 真 app bundle → `39277bd` 内存代价。含 `src-tauri/src/reparent_poc.rs`、
  `public/reparent-poc.html`、`scripts/dev/webview-memory.py`，以及 `Cargo.toml` 里
  仅 PoC 需要的 `tauri` `unstable` feature。
  **`main` 停在 `92c9bb3`，从未包含任何 PoC 代码或 `unstable` feature —— 无需回退。**
- 原始实测日志（`/tmp/dae-reparent-poc.jsonl` 的摘录）在 `NOTES.md`，该文件被 gitignore，
  只存在于当时那台机器上；复现命令见 §7。
- 与标签页方案解耦、仍然有效的遗留项：**§4.7**（已拍定：先记账，不排期）。
