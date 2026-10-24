# 冷启动：JS 解析/执行的极端手段

> **状态：§1 归因已完成（数字见 §1.2）；§2 的 A/B 已实测且为否**
> （`docs/decisions/boot-screen-prerender.md`：静态骨架在 bundle 求值前根本不会被画，
> `first-frame` 结构上不可能提前；Rust 预渲染不做）；**§3–§6 仍是清单**；§8 第 3 条
> 的第一刀（交接 IPC）已实施并复测。** 归因结果推翻了这份文档的默认假设：**parse + exec
> 只占冷启动的 3–4%**，而清单里没有任何一条能碰到两项大头 —— **WebView2 创建窗口
> （63–65%）**与 **React 首次 commit（14–17%）**。§8 的「顺序建议」因此按数字重排，
> §3/§4 前面也各加了一句「按归因，这已经不是你该先做的事」。
>
> 按本仓库的习惯：**没有 §1 的归因数据之前，下面任何一条都不要动手**。
> 每条后面写的是「预期量级 + 代价」，不是承诺。

## 0. 先划掉走不通的路

免得下次重新推一遍：

| 想法 | 为什么不行 |
|---|---|
| 自己写 V8 code cache / bytecode | WebView2 与 WebKit 各自管理缓存目录，进程内拿不到 `v8::ScriptCompiler::CreateCodeCache`。能蹭的只有「第二次启动」（见 §3 的 user data 目录） |
| V8 compile hints（`CreateCompileHintsForScript`，Chrome 给自家 JS 省 lazy-parse 第二遍扫描的那招） | 需要自建 V8 运行时 + 离线产 hint 的工具链，Tauri 拿不到 |
| 自定义 V8 startup snapshot | 同上 |
| React → Preact 换取更小的解析体 | Preact/compat 与 React 19 + React Compiler 的输出不兼容；省下的 ~40KB 解析 ≈ 一帧，重写成本不成比例 |
| 把 boot 拆成更多 `import()` | 每个 chunk 是一次协议往返 + 一个 module record，通常把 parse 成本换成 fetch + 调度成本 |
| `--js-flags=--no-lazy` / `--always-sparkplug` 之类 | 冷启动上基本都是负收益：先把字节码全编出来，正是要避免的事 |

## 1. 归因：一条从进程启动到首帧的瀑布

「冷启动慢」至少混了五笔账，不拆开就会在对的那一层上做错的优化（本仓库已经踩过一次：
`docs/performance-opportunities.md` 里 Windows 枚举那条，先量才发现元数据根本不贵）。

### 1.1 埋点与开关（已实施）

开关 `DAE_STARTUP_PERF=1`，落盘 `DAE_STARTUP_PERF_OUT=<file>`（追加，一次启动一块）。
关闭时每个埋点只多一次判断：`index.html` 的两段内联脚本各一个 `if`，
Rust 侧 `mark_at` 与前端 `tabPerfMark` 都直接返回。

沿用 `tab_perf.rs` 的「env 开关 + mark + 一次 report」，没有第二条报表通道：标记在两侧
都攒成 `(stage, epoch ms)`，前端首帧后 `invoke("tab_perf_report")` 交回，Rust 按
`process-start` 归零、排序、打印。release 构建是 `windows_subsystem = "windows"`，
没有控制台、日志文件还会轮转，所以时间线同时写一份到 `DAE_STARTUP_PERF_OUT`。

| 节点 | 打在哪 | 拆的是哪笔账 |
|---|---|---|
| `process-start` | `lib.rs::run` 第一行 | 零点是**进入 `run`**，不是 OS `CreateProcess`；两者之间是加载器与运行时 |
| `setup` | `.setup` 闭包第一行 | `run` → Tauri 插件初始化完成 |
| `window-config` | `builder.build()` 前一行 | 窗口配置（材质/透明度、注入脚本） |
| `window-built` | `build()?` 之后 | **WebView2 环境 + controller 创建**，页面侧完全看不见的一段 |
| `html-start` | `index.html` 头部第一段脚本 | 文档到达 |
| `html-parsed` / `boot-eval-start` | `body` 末尾、module 标签之前 | HTML/样式/预加载。`DOMContentLoaded` 不能用：module 求值会把它推后，那会把「解析完」和「执行完」量成一件事 |
| `bundle:fetched` | 入口 module 自己在 resource timing 里的 `responseEnd` | 入口 bundle 取回。`dist/index.html` 有 modulepreload，所以它可能早于 `boot-eval-start` |
| `boot-eval-end` | `main.tsx` 模块体第一行（所有 import 已求值） | bundle 解析 + 执行 |
| `bundle` / `locale` / `handoff` / `chunk` / `render` | `bootstrap()` 各步 | 模块顶层、i18n、交接 IPC、explorer 预载、`createRoot` 前 |
| `commit` | `MutationObserver` 第一次触发 | React 首次提交 |
| `first-frame` | commit 后第一个 `rAF` | 首帧（`paint` entry 只在真的上过屏之后才有，见 §1.4） |
| `first-data` | 首个返回数据的 atom/listing | 首屏数据；晚于首帧就说明首帧没有等数据 |

`mark_at` 是这次唯一新增的 Rust 入口，用来记录「时间点已经发生、但 label 还没取到」的
`process-start`：`mark` 是到达即打，表达不了这个。

### 1.2 实测瀑布（Windows 11 / WebView2 / 本机）

口径：`bun tauri build --no-bundle --config src-tauri/tauri.release.conf.json` 的
**普通 release**（没有 PGO/BOLT）；每格是中位数，括号是跨会话（3 组 × 9 次重复启动）的范围。

| 段 | 重复启动（9×3） | 全新 WebView2 配置（5×2） | 说明 |
|---|---:|---:|---|
| `process-start → setup` | 8.7–9.9 ms（2%） | 9.5–9.6 ms | Tauri/插件初始化 |
| `setup → window-config` | 0.0–0.1 ms | 0.0 ms | 材质决策 + 注入脚本 |
| **`window-config → window-built`** | **269.8–308.6 ms（63–65%）** | **288.4–312.9 ms（65%）** | **WebView2 环境 + controller** |
| `window-built → html-start` | 14.1–16.8 ms（3%） | 15.4–15.5 ms | 导航开始 |
| `html-start → boot-eval-start` | 16.6–20.4 ms（4%） | 17.6–18.6 ms | HTML/样式/31 个 modulepreload |
| `boot-eval-start → boot-eval-end` | 15.0–17.0 ms（3–4%） | 15.6–17.9 ms（4%） | 入口 bundle 的解析 + 执行 |
| `boot-eval-end → render`（**改动前**） | 19.7–28.0 ms（5–6%） | 29.2–31.2 ms（6%） | 模块顶层 1.6 + i18n 2.3 + **交接 IPC 26** + chunk 0 |
| **`render → commit`** | **62.1–71.4 ms（14–16%）** | **64.2–69.9 ms（15%）** | React 首次挂载（单个长任务 59–80 ms） |
| `commit → first-frame` | 10.8–13.3 ms（2–3%） | 5.4–14.6 ms（1–3%） | 提交后的第一帧 |
| **合计 `process-start → first-frame`** | **431–481 ms** | **455–491 ms** | |

`parse + exec` = `boot-eval-end − max(boot-eval-start, bundle:fetched)` = **15–18 ms = 3–4%**，
两种配置一样。

**交接 IPC 改掉之后，只有那一行变了**（§8 第 3 条的第一刀；9 次重复 + 5 次全新配置）：

| 段 | 改动前 | 改动后 |
|---|---:|---:|
| `boot-eval-end → render` | 19.7–28.0 ms（5–6%） | **3.9–4.5 ms（1%）** |
| 其中 `handoff`（IPC 等待） | 20–26 ms | **0.0 ms（9 次中位）** |

后面的段不受影响，所以合计的差值就是这 20–26 ms；上表的绝对值仍然按未改前的那几组
会话记，改后那两组的总中位是 457.8–488.5 ms（同期 WebView2 那段偏慢，见 §1.4）。

### 1.3 读出来的五件事

1. **parse 不是瓶颈，而且 V8 code cache 也没在帮忙。** 入口 bundle 510 kB（gzip 160 kB），
   解析+执行 15–22 ms；把 WebView2 配置整个删掉（没有 code cache、没有 localStorage）
   之后还是 15.6–17.9 ms。§4 那整节（砍 parse 体重）的天花板就是这个数，
   §0 里「React → Preact 省 40KB ≈ 一帧」也按同一把尺子失效了。
2. **最大的一笔是 WebView2 创建窗口，63–65%。** 它不是 Rust 的代码：`build()` 里
   `CreateCoreWebView2Environment` + controller + 首个 renderer 起来，页面侧完全没有
   可埋的点。进程表能对上：`msedgewebview2.exe` 的 browser 进程在 `dae.exe` 之后
   ~56–70 ms 出现，其余 renderer/utility 在 160–237 ms 之间起来，而 `build()` 到 270–313 ms
   才返回。这份文档的 §2–§5 没有一条能碰到它；能碰到的只有 §6（常驻/预热），
   而那是产品决策。`docs/decisions/tab-tear-off-warm-pool.md` 在 Linux 上量到
   `SHOWN → bundle ~250 ms` 是同一笔账的另一个平台版本。
3. **第二笔是 React 的首次 commit，14–16%。** 它是一次 59–80 ms 的同步长任务
   （`longtask` 观察器每次都报），不是「等数据」：`first-data` 稳定落在
   `first-frame` **之后** 15–37 ms，首帧画的是骨架。**已拆开**
   （`docs/decisions/first-mount-cost.md`，profiling build + `<Profiler>`）：
   其中 React 的 render 只有 **19 ms**（首屏 Overview 子树 **1.4 ms**），
   另外 **~40 ms 是树插入后第一次 `getBoundingClientRect` 触发的 style+layout**
   （style recalc 0 ms、flush 0 ms、节点数 248；同一份 CSS 在 splash 上只要 4.1 ms）。
   那 40 ms 是一次性冷成本（≥19 ms 是 CJK 字面首次使用，预热能把 40 降到 21 但要花 39），
   必须发生在首帧之前 —— 所以 §2 要省的不是它，§5 也没有可搬的东西。
4. **交接 IPC 已从关键路径上拿掉**（§8 第 3 条的第一刀）：`takeTabHandoff` 原来在
   `createRoot` 之前被 await，第一次 invoke 在这个新 webview 上要 20–26 ms。现在改由
   Rust 在 `initialization_script` 里注入 `window.__DAE_TAB_HANDOFF`（冷撕出窗口带 payload，
   主窗口/池窗口带 `null`），前端同步读取；`take_tab_handoff` 命令保留为
   「窗口没拿到注入」时的回退。实测 `boot-eval-end → render` 从 19.7–28.0 ms 降到
   **3.9–4.5 ms**，`handoff` 标记从 26 ms 降到 **0.0 ms**（9 次中位）。代价是前端多读
   一个全局，以及撕出窗口的 payload 在 Rust 多留一份给回退路径。
5. **`html-start → boot-eval-start` 的 17–20 ms 也比预期贵**：`dist/index.html` 里有
   31 个 `modulepreload` 和一张内联样式表。它和首帧无关（首帧等的是 React），
   但它是「文档到达 → bundle 能开始执行」之间的固定成本，顺带记着。

### 1.4 口径与缺口

- **普通 release，没有 PGO/BOLT。** 两项大头都不在 Rust 代码里（WebView2、V8），
  PGO/BOLT 改不动它们；Rust 侧的 8.7–9.9 ms 才是它们的地盘。
- **重复启动 vs 全新配置**：前者是用户几乎每次都遇到的状态（WebView2 的 user data
  与 code cache 都在）；后者由 harness 把 `EBWebView` 目录移开、逐次删除来模拟首次安装
  （跑完放回，不会丢用户的主题/侧栏状态）。两种情况下结构一样，只有 WebView2 那段
  多 20–40 ms。
- **没有量「OS 页缓存冷」的状态**：清 Windows 文件缓存需要管理员工具，这一格空着。
  跨会话的 431 → 481 ms 漂移就是它的影子，所以表里给的是范围而不是单值。
  还有一组在**重新构建后立即跑**的 9 次（交接 IPC 改掉之后），中位 488.5 ms
  （`window-config → window-built` 325.7 ms、`parse+exec` 仍是 4%）——同一结构，
  只是新二进制被扫描/机器负载更重。
- **`first-paint` 的缺席不是「引擎不支持」，而是「还没画过」。** 原文按 23 次里只出现
  3 次判它不可靠。§2 的 A/B 补上了另一半：没有让帧的 36 次里只出现 1 次（且那一次
  确实在 bundle 求值前有一帧）；在 `createRoot` 前让两帧的 12 次里 12 次都有。
  所以判断「有没有上屏」用 `html-frame-N` 探针（`index.html`，只在 probe 开启时
  注册）更稳；paint entry 的绝对位置仍偏差 30–40 ms，别拿它算段长。详见
  `docs/decisions/boot-screen-prerender.md` §5。

### 1.5 复现

```bash
# 先按 release 构建（PGO/BOLT 可选，见 docs/pgo-bolt.md）
bun tauri build --no-bundle --config src-tauri/tauri.release.conf.json

# 重复启动（用户实际状态）
bun bench/cold-start/run.ts --runs 9

# 首次安装：每轮把 WebView2 配置删掉，跑完把真实配置放回
bun bench/cold-start/run.ts --runs 5 --fresh-profile
```

harness 会启动 exe、等 `DAE_STARTUP_PERF_OUT` 里出现整块时间线、再杀掉整棵进程树
（应用是单实例的：残留实例会让下一次启动变成「转发参数后退出」，所以它先检查再启动）。
原始时间线留在 `.perf/cold-start/run-*.log`。

## 2. 首帧不依赖 JS：Rust 预渲染第一屏 —— **已实测否决**

> **结论（`docs/decisions/boot-screen-prerender.md`，2026-10-24）：不做。** 外壳档
> （静态骨架）已经真的放进 `index.html` 量过：**入口 bundle 求值前引擎几乎不画**
> （两个会话 × 9+9 次交错，36 次里只有 1 次在 `boot-eval-end` 前有帧，且是 splash 臂；
> 骨架臂 0/18；截图从窗口出现到 React 上屏整段空白）。启动是一条连续的主线程链
> （解析 → bundle → 交接/i18n → React 挂载），没有让出一次渲染机会。唯一能看到
> 静态屏的办法是在 `createRoot` 前让出一帧：6/6 次在 commit 前上屏、停留 ~80 ms，
> 但 `first-frame` **+40–53 ms**。所以它不是性能收益，而是「拿 40 ms 等待换 80 ms
> 有内容画面」的产品取舍；**内容档（用 Rust 渲前 30 行）因此没有立足点** —— 这 80 ms
> 的窗口由「让帧」创造，与 HTML 里有什么无关，而 Rust 侧要多复制一份行渲染并与
> 前端样式同步。下面的原始思路保留作背景。

**思路**：让浏览器拿到的 HTML 里已经含第一屏内容，React 的解析/执行不再阻塞首帧。

- Tauri 里可行：`file_system::preview` 已经用 `register_asynchronous_uri_scheme_protocol`
  返回过原始字节（缩略图、图标）。同一个机制可以接管**主文档**：返回一份把第一屏
  行拼进去的 `index.html`。
- 数据来源现成：`file_system::prefetch::warm_startup_data` 在 webview 加载期间就已经在跑，
  它手上的正是「首屏要显示什么」。
- 两档粗糙程度：
  1. **外壳档**（便宜）：侧栏、标签条、空列表骨架在构建期生成进 `index.html`
     （照 `vite.config.ts::themeBootStyles` 的做法），React 只接管交互。
  2. **内容档**（收益最大）：前 ~30 行条目的名字/图标/日期直接渲染成 `<div>`，
     样式沿用列表的 token；`createRoot` 挂载时整块替换成真实列表。
- 代价：Rust 侧要复制一份「行的最小渲染」，并和前端样式保持同步；内容档还要处理
  挂载瞬间的替换闪烁（先隐藏静态块再显示 React 树，或反之）。

## 3. 资产传输层（几毫秒级，改一行就能量）

> 按 §1.2：入口 bundle 从「开始取」到「执行完」只有 15–22 ms，其中还包括与 HTML 解析
> 重叠的部分，这一层的上限是个位数毫秒。可以顺手做，不要当主线。

| 项 | 现状 / 建议 |
|---|---|
| Tauri `compression` feature | **现在是开着的**（`tauri = { features = ["macos-private-api"] }` 用的是默认 features，含 `compression`）：前端资产 brotli 压缩后嵌进 exe，运行时解压。1 MB bundle 解码约 4~8 ms，而 SSD 页入约 1~2 ms；bundle 再大一点、页缓存热的时候，**关掉压缩可能反而更快**。改法：`default-features = false` + 显式列 feature（别丢 `wry`/`custom-protocol`/`x11`/`dbus`/`common-controls-v6`），A/B 一次即可定论 |
| WebView2 user data 目录 | 默认落在 `%LOCALAPPDATA%\<identifier>\EBWebView`，所以**第二次启动已经在吃 V8 code cache**。要确认的是没有把它指到临时目录、也不在启动时清掉 |
| `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` | 唯一能碰 Chromium/V8 flag 的口子。可试：`--disable-features=CalculateNativeWinOcclusion`（省 Windows 遮挡计算）、`--js-flags=--lazy-feedback-allocation`、`--js-flags=--no-concurrent-recompilation`（降首屏抖动）。**别加** `--no-lazy`、`--single-process`、`--disable-gpu` |
| Linux（WebKitGTK） | `WEBKIT_DISABLE_COMPOSITING_MODE=1` 让启动更快但绘制更慢，只在 CI/训练场景用，不要进产品 |
| `build.removeUnusedCommands`（Tauri 2.10） | 现在没设（默认 `false`）。剪掉没注册到的插件命令，二进制与 IPC 注册表都小一点，属顺手项 |

## 4. 砍 parse 的实际体重

> 按 §1.2：parse + exec 占冷启动 3–4%，上限 15–22 ms，删掉 V8 code cache 也不变。
> 除非 bundle 再大一个量级，这一节不要再动。下面保留原样，是当时的前提与做法。

解析时间基本正比于字节数，所以这一节是「找掉不该在 boot 路径上的东西」，不需要技巧：

- 用 rolldown/esbuild 的 metafile 看 boot chunk 的真实构成。重点怀疑对象（按经验排序）：
  `src/bindings.ts`（specta 生成的类型/命令表）、`@tanstack/*`、`i18next` + `react-i18next`、
  `@solar-icons/react`。
- **i18n 目前是同步的**：`src/i18n/index.ts` 里 `resources[DEFAULT_LOCALE] = bundle(zhModules)`
  在 init 时就把中文的命名空间全解析了。改成「boot 只留首屏用到的 1~2 个 namespace，
  其余 `loadLocale()` 挪到 idle」，是这一节唯一还有两位数潜力的点。
- **图标**：`@solar-icons/react/linear` 是子路径 + 具名导入，tree-shaking 理论成立，
  但这类图标包常有 barrel 再导出；在 metafile 里数一下 icon 模块数量即可确认。
  注意 `context-menu.tsx` 还有一条 `@solar-icons/react/lib/types` 的类型导入（无运行时成本）。
- **顶层代码要小**：V8 的 lazy parse 只省**函数体**；顶层的大对象字面量、装饰数据表、
  注册循环都在开头全额付钱。`import.meta.glob` 展开出来的大表尤其如此。
- **模块数**：首屏保持一个 chunk（减少 module record 与多次协议往返），其余再 code-split。
- 不要盲目压缩成一行超长脚本：parse 成本与字节数相关，与「行数」无关；压缩率才是变量。

## 5. 把 JS 的活推到 Rust

前端只做「把 Rust 算好的东西贴到 DOM 上」：

- 行模型、排序键、日期/大小格式化已经在 Rust 侧有对应实现（`listing.rs`、`entry-codec.ts`
  的列式 packet）。可以继续把「格式化后的字符串」也一起下发，前端省掉 `Intl` 调用。
- `i18n` 文案拼接同理：Rust 侧按当前 locale 出成品字符串（代价是 Rust 要持有词表）。
- 极端版：虚拟列表的可视窗口计算也在 Rust，前端滚动时只发 offset。

这一节的收益随列表规模放大，但它**不缩短首帧**，除非第一屏的渲染本身在等 JS 计算
（配合 §2 才有意义）。

## 6. 常驻与预热（产品决策，不是技术优化）

> 现在这笔账有数字了：一个已经 boot 的窗口能省掉 §1.2 里 **269.8–339.1 ms** 的
> WebView2 创建（以及其后的加载/渲染，按 `tab-tear-off-warm-pool.md` 的隐藏窗口探针，
> 显示前都能完成）。它不缩短首次冷启动，只改善之后的每一次。

- 现在已经有进程内的预热窗口池（`docs/decisions/tab-tear-off-warm-pool.md`）和单实例/深链。
  再往前是**驻留**：登录后常驻一个隐藏主窗口，WebView 已 `document_start` 完、boot 代码
  已 JIT；用户点图标时只剩「显示窗口」，量级 50 ms。
- 它不改善首次冷启动的绝对耗时，改善的是**感知**；代价是常驻内存、退出语义、
  更新时如何优雅重启，以及「文件管理器常驻」是否可接受的产品判断。
- 折中且几乎无代价的一条：确认主窗口的 webview 是**尽早**创建的（`tauri.conf.json` 里
  已经 `"create": false`），让 WebView 初始化与 `warm_startup_data` 并行，而不是等
  显示后才建。

## 7. 量法

- 冷启动：`bun bench/cold-start/run.ts`（§1.5）。它跑的是**出货的**那套埋点与 release
  二进制，不是复刻；`--fresh-profile` 模拟首次安装，`--runs` 决定样本数。
  OS 页缓存冷的状态仍未量（见 §1.4）。
- 首屏 A/B：`bun bench/cold-start/run.ts --runs 9 --ab`（splash vs 静态骨架，交错）、
  `--ab --yield`（两臂都在 `createRoot` 前让两帧）。视觉确认用
  `bench/cold-start/bootshot.ps1`（只看有没有画，数字别用它）。
- 首次挂载拆分：`DAE_PROFILE_REACT=1 bun tauri build --no-bundle --config
  src-tauri/tauri.release.conf.json`，再跑 `run.ts`，profiler 的行在
  `.perf/cold-start/run-*.log`（`docs/decisions/first-mount-cost.md`）。
  profiling build 不要出货，跑完重新按普通 release 构建。
- 列表/解析：`bench/listing-pipeline`（纯 Rust，无 GUI 噪声）、`bench/frontend-scans`
  （前端扫描成本）。
- 任何结论按 `docs/decisions/` 的格式记：当时为什么这么定、证据在哪、什么条件下重开。

## 8. 顺序建议（按归因重排）

原顺序是「§1 归因 → parse 大头就 §4+§3，等 JS 大头就 §2+§5」。§1.2 的数字把这道题答了，
而且和两个分支都不完全一样：

1. **§4 关掉**：parse+exec 3–4%，天花板 15–22 ms，而且删掉 code cache 也不变。
2. **§3 只值几毫秒**：入口 bundle 取回约 10–17 ms，还包括与解析重叠的部分；
   压缩开关的收益在 5 ms 量级。顺手可做，别当主线。
3. **真正的主线是两笔，都不在原清单的主推位置**：
   - ~~`takeTabHandoff` 的 26 ms 关键路径等待~~ → **已实施**：改成 `initialization_script`
     注入全局（冷撕出窗口带 payload，主窗口/池窗口带 `null`），实测那一段
     19.7–28.0 → **3.9–4.5 ms**。`take_tab_handoff` 命令保留为回退。
   - **§2「首帧不依赖 JS」**：**已实测，不做**（`docs/decisions/boot-screen-prerender.md`）。
     首帧等的是 React 的 62–82 ms 挂载，不是数据（`first-data` 在首帧之后）；而
     §8 要求的那件「引擎会不会在 deferred module 求值前把预渲染的 DOM 画出来」已经量了：
     **不会**。两个会话各 9+9 次交错里，36 次只有 1 次 `html-frame-N` 早于
     `boot-eval-end`（还是 splash 臂），骨架臂 0/18；
     截图也显示窗口从出现到 React 上屏整段空白（连 splash 的转圈都没上过屏）。原因不是
     引擎挑剔，是启动没有让出渲染空隙。在 `createRoot` 前让两帧（`DAE_BOOT_YIELD=1`，
     作为诊断臂留在仓库）后，骨架 6/6 在 commit 前上屏、停留中位 ~80 ms，代价是
     `first-frame` +40–53 ms。因此：Rust 预渲染不做；要不要付那 40 ms 是产品取舍，不是
     性能优化。`html-frame-N` 探针取代了会缺席、位置也有偏差的 paint entry（见 §1.4）。
4. **§5 对冷启动首帧关闭**（`docs/decisions/first-mount-cost.md`）：原文的前提是
   「第一屏的渲染在等 JS 计算」，实测把它拆成两半 —— 首屏 Overview 的 render 只有
   **1.4 ms**（整棵树 19 ms），而 commit 的 **~40 ms 是首次布局**，不是格式化/排序。
   把行模型、`Intl`、i18n 拼接搬到 Rust，能碰到的上限是那 1.4 ms 的一小部分；
   而且这 40 ms 必须发生在首帧之前，预热只会把它搬家（实测净亏）。
   §5 对「打开几万条目的目录」那一帧仍可能成立，但那是另一条路径，要单独量
   （带 folder handoff 的窗口 + 同一套 profiler）。
5. **§6 是唯一能碰 63–65% 的那条**，但它不改善「首次冷启动」的绝对耗时，只改善
   之后的每一次（常驻/预热）。量到的事实现在很硬：一个已 boot 的窗口能省掉
   269.8–339.1 ms 的 WebView2 创建；值不值得常驻 ~200 MB 是产品判断，不是性能判断。
