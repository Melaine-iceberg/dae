# 冷启动：JS 解析/执行的极端手段

> **状态：清单，未实施。** 这份文档是「`docs/pgo-bolt.md` 那层做完之后，还想继续抢冷启动」
> 时的备选表，不是结论。Rust 侧（枚举、排序、IPC、PGO/BOLT）基本到顶了，
> `docs/performance-opportunities.md` 已经说过这一点；剩下的账主要在 WebView 里。
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

## 1. 归因：先拿到一条瀑布

「冷启动慢」至少混了五笔账，不拆开就会在对的那一层上做错的优化（本仓库已经踩过一次：
`docs/performance-opportunities.md` 里 Windows 枚举那条，先量才发现元数据根本不贵）。

建议的埋点（`index.html` 内联，必须在任何 module 之前）：

```html
<script>
  performance.mark("html-start");
  // …每收到一个关键节点就 post 一次，或先攒在数组里
</script>
```

| 节点 | 打在哪 |
|---|---|
| `webview-init` | Rust 侧 `finish_tab_window` / 主窗口 ready 回调（和 `tab_perf::mark` 同一个路子） |
| `html-start` / `html-parsed` | `index.html` 头部与 `DOMContentLoaded` |
| `boot-eval-start` / `boot-eval-end` | 入口 module 首行 / 首次 `createRoot` 之前 |
| `first-commit` | 第一次 `useEffect` 提交 |
| `first-data` | 首屏 listing 到达（现有的 packet 通道回调） |
| `first-paint` | 首帧（`requestAnimationFrame` 两次） |

落盘方式：一个 `log_mark` 命令写进 `tauri-plugin-log` 的文件目标，或者批量 `invoke` 一次。
`tab_perf.rs` 已经有 `DAE_TAB_PERF_*` 这套「env 开关 + mark」的先例，直接沿用。

**判据**：`parse + exec` 占冷启动 < 15% → 只做 §3（资产层，几毫秒）；占 30% 以上 → §2、§4。

## 2. 首帧不依赖 JS：Rust 预渲染第一屏

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

| 项 | 现状 / 建议 |
|---|---|
| Tauri `compression` feature | **现在是开着的**（`tauri = { features = ["macos-private-api"] }` 用的是默认 features，含 `compression`）：前端资产 brotli 压缩后嵌进 exe，运行时解压。1 MB bundle 解码约 4~8 ms，而 SSD 页入约 1~2 ms；bundle 再大一点、页缓存热的时候，**关掉压缩可能反而更快**。改法：`default-features = false` + 显式列 feature（别丢 `wry`/`custom-protocol`/`x11`/`dbus`/`common-controls-v6`），A/B 一次即可定论 |
| WebView2 user data 目录 | 默认落在 `%LOCALAPPDATA%\<identifier>\EBWebView`，所以**第二次启动已经在吃 V8 code cache**。要确认的是没有把它指到临时目录、也不在启动时清掉 |
| `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` | 唯一能碰 Chromium/V8 flag 的口子。可试：`--disable-features=CalculateNativeWinOcclusion`（省 Windows 遮挡计算）、`--js-flags=--lazy-feedback-allocation`、`--js-flags=--no-concurrent-recompilation`（降首屏抖动）。**别加** `--no-lazy`、`--single-process`、`--disable-gpu` |
| Linux（WebKitGTK） | `WEBKIT_DISABLE_COMPOSITING_MODE=1` 让启动更快但绘制更慢，只在 CI/训练场景用，不要进产品 |
| `build.removeUnusedCommands`（Tauri 2.10） | 现在没设（默认 `false`）。剪掉没注册到的插件命令，二进制与 IPC 注册表都小一点，属顺手项 |

## 4. 砍 parse 的实际体重

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

- 现在已经有进程内的预热窗口池（`docs/decisions/tab-tear-off-warm-pool.md`）和单实例/深链。
  再往前是**驻留**：登录后常驻一个隐藏主窗口，WebView 已 `document_start` 完、boot 代码
  已 JIT；用户点图标时只剩「显示窗口」，量级 50 ms。
- 它不改善首次冷启动的绝对耗时，改善的是**感知**；代价是常驻内存、退出语义、
  更新时如何优雅重启，以及「文件管理器常驻」是否可接受的产品判断。
- 折中且几乎无代价的一条：确认主窗口的 webview 是**尽早**创建的（`tauri.conf.json` 里
  已经 `"create": false`），让 WebView 初始化与 `warm_startup_data` 并行，而不是等
  显示后才建。

## 7. 量法

- 冷启动：`hyperfine`（或自写的 N 次循环）跑「进程起到首帧」，同一台机器、同一个目录、
  清/不清页缓存两种状态都要量，取中位数；GUI 路径以 §1 的 `first-paint` mark 为准。
- 列表/解析：`bench/listing-pipeline`（纯 Rust，无 GUI 噪声）、`bench/frontend-scans`
  （前端扫描成本）。
- 任何结论按 `docs/decisions/` 的格式记：当时为什么这么定、证据在哪、什么条件下重开。

## 8. 顺序建议

1. §1 归因（一次，几分钟的改动）；
2. 按数字二选一：parse 占大头 → §4 + §3；等 JS 占大头 → §2 + §5；
3. §6 作为独立产品决策单独评估。
