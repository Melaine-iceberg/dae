# 首次挂载的 62–74 ms：render 19 ms + 首次布局 ~40 ms —— **§5 没有可搬的东西**

> **状态：已实测，`cold-start-js.md` §5 对冷启动首帧关闭。** 用
> `DAE_PROFILE_REACT=1` 的 profiling build + 新加的 `Sidebar` / `TabStrip` /
> `Overview` 三个 `<Profiler>` 把 `render → commit` 拆开：**React 自己只花
> 17–23 ms，其余 ~40 ms 是树插入后第一次 `getBoundingClientRect` 触发的
> style+layout**。这 40 ms 是一次性冷成本（CJK 字面首次使用 + 首次布局），
> 必须发生在首帧之前；提前预热只是把它搬家（预热 39 ms、React 树的首次布局降到
> 21 ms，总量更差）。所以：**没有可搬到 Rust 的活**，也没有能省下这 40 ms 的
> 代码改动。首帧唯一的杠杆仍然是 `cold-start-js.md` §6（不冷启动）。
>
> 复现见 §6。原始时间线在 `.perf/cold-start/`（gitignore）。

## 0. 结论先行

1. **`render → commit` 的构成**（5 次中位）：`app[mount] actualDuration` **19.4 ms**，
   commit 阶段 **~42 ms**，其余 ~8 ms 是调度。`app[mount]` 里最大的是
   `ExplorerTabs` 13.5 ms，再往下 `Sidebar` 3.4 / `TabStrip` 3.0 / `Surface` 1.6 /
   **`Overview` 1.4**。
2. **§5 没有立足点**：冷启动首屏是 Overview，它的整棵子树 render 只有
   **1.2–2.0 ms**。行模型、日期/大小格式化、`Intl` 这些都在目录列表里，
   而冷启动首帧时列表还没数据（`first-data` 在首帧之后 30–35 ms）。
3. **commit 的 42 ms 全是首次布局**：`getComputedStyle` 触发的 style recalc
   **0.0 ms**，两次 `getBoundingClientRect` **40.3 ms**，`offsetWidth` flush **0.0 ms**。
   树只有 **248 个节点**，所以这不是节点数的问题。
4. **也不是样式表应用**：同一份 117 KB CSS（`sheets=3, rules=322`）在只有 splash
   的文档上首次布局只要 **4.1 ms**；`createRoot` 前再强制一次布局只要 **0.1 ms**。
5. **它是一次性冷成本，且预热是净亏**：同一棵树再强制一次全量 style+layout 只要
   0–3.5 ms。提前塞一段同字体的中文并强制布局：400 字重那行自己花 **14.8 ms**，
   树的首次布局 40.3 → 32.8 ms；400/520/600 三行花 **38.9 ms**，首次布局 → **21.2 ms**。
   省下的永远少于付出去的。
6. **因此**：`§5`（把 JS 的活推到 Rust）在冷启动首帧上没有内容可搬；那 40 ms 是
   引擎在首帧前必须做的第一次布局，Rust 预渲染、内容档、分段 commit 都碰不到它
   （分段 commit 只改变“首帧画多少”，不减少总工作量）。唯一能跳过它的是 §6：
   一个已经 boot 的窗口已经付过这笔钱。

## 1. 测法

- `DAE_PROFILE_REACT=1 bun tauri build --no-bundle --config src-tauri/tauri.release.conf.json`
  —— `vite.config.ts` 把 `react-dom/client` 换成 `react-dom/profiling`，
  `<Profiler>` 的 `actualDuration` 才有值。**这个 build 不能出货。**
- 新增三个 profiler：`Sidebar` / `TabStrip`（`explorer-tabs.tsx`）、`Overview`
  （`workspace-surface.tsx`）。`TabPerfProfiler` 在 probe 关闭时原样返回 children，
  零成本。
- `TabPerfProfiler.onRender` 额外记一条 `commit:<id>:end`（wall clock）：React 从
  layout 阶段调用 `onRender`，所以它和 `app[mount]`（记在 `commitTime`，即 commit
  开始）把 commit 阶段夹出来。
- `main.tsx` 的 first-frame 里记 `dom-nodes:<n>`。
- `useTabPill` 的 layout effect 记 `pill:layout:start/end` —— 它是整棵树里第一个
  强制布局的地方。
- 一次性诊断（**未留在代码里**）：在 `boot-eval-end` 和 `createRoot` 前各强制一次
  布局（`layout:probe1/2`），以及在 probe1 里先塞隐藏中文预热字体。

`run.ts --runs 5` 跑 release 二进制；profilng build 的绝对值略慢，但同一次运行内
的拆分不受影响（总 `first-frame` 与普通 release 的 431–481 ms 落在同一区间）。

## 2. 数字

### 2.1 render 侧（`app[mount] actualDuration`，5 次）

| profiler | ms | 说明 |
|---|---:|---|
| `app` | **19.4**（16.4–23.1） | 整棵树 |
| `ExplorerTabs` | 13.5 | 外壳 |
| `Sidebar` | 3.4 | 侧栏 |
| `TabStrip` | 3.0 | 标签条（含 1 个 tab） |
| `Surface` | 1.6 | 当前 tab 的表面 |
| `Overview` | **1.4** | 冷启动首屏本体 |

`app − ExplorerTabs ≈ 6 ms` 是 `TextContextMenu` / `Toaster` / lazy 边界的空壳；
`ExplorerTabs − (Sidebar + TabStrip + Surface) ≈ 5.5 ms` 是状态栏、inspector 等。

### 2.2 commit 侧（同一次运行，ms since process start）

| 标记 | 时间 | 差 |
|---|---:|---:|
| `app[mount]`（commit 开始） | 384.8 | — |
| 各子树 `commit:*:end` | 386.0 | **+1.2 ms**（mutation + 其余 layout effects） |
| `pill:layout:start` | 386.0 | |
| `pill:style:end`（先 `getComputedStyle`） | 386.0 | **0.0 ms** |
| `pill:rects:end`（两次 `getBoundingClientRect`） | 429.4 | **+40.3 ms** |
| `pill:flush:end`（`offsetWidth`） | 429.6 | **0.0 ms** |
| `commit`（MutationObserver） | 430.3 | +0.7 ms |
| `dom-nodes` | — | **248** |

`render → commit` 的 69.8 ms = 调度 ~8 + render 19.4 + commit 42（其中首次布局 40.3）。

### 2.3 这 40 ms 不是样式表、不是节点数

| 实验 | 结果 |
|---|---|
| 同一份 CSS、只有 splash 的文档首次布局（`layout:probe1`） | **4.1 ms**（`sheets=3, rules=322`） |
| `createRoot` 前再强制一次（`layout:probe2`） | **0.1 ms** |
| 首次布局后，把 `<html>` 弄脏再全量重算（同 248 节点） | **0–3.5 ms** |
| 先塞 400 字重中文并布局 | 预热 **14.8 ms**，树的首次布局 40.3 → **32.8 ms** |
| 先塞 400/520/600 三行中文并布局 | 预热 **38.9 ms**，树的首次布局 → **21.2 ms** |

读法：

- 4.1 ms 说明样式表本身（117 KB、`@property`、`color-mix`）不贵；贵的是把它应用到
  **React 插入的这棵树**上并做第一次布局。
- 0–3.5 ms 说明不是「每次重算都贵」——`@property` / `color-mix` / 62 个 `:has()`
  都没有把 recalc 变成病理状态。
- 预热能把 40 ms 砍到 21 ms，说明其中 **≥19 ms 是 CJK 字面的首次使用**
  （字体面初始化 + 文字塑形；splash 没有文字，所以它只要 4.1 ms）。
- 剩下的 ~21 ms 没有继续细分：可能是首屏用到的其余字面/字重（520/600 是另一套
  face），也可能是第一次布局的固定开销。无论哪一种，它都在首帧之前、且预热不划算。

## 3. 为什么预热不是解法

首帧之前的主线程是一条串行链：解析 → bundle → 交接/i18n → render → **首次布局** →
帧。把首次布局提前（预热、Rust 预渲染、更早的骨架）只会把同样的工作搬到更早的位置：

- 预热那 38.9 ms 本身就占主线程，总 `first-frame` 从 440–448 退到 456–473 ms；
- 省下的只有「树的首次布局」那一段（40 → 21 ms），净亏。

这也解释了 `boot-screen-prerender.md` 里为什么「让出一帧」的代价是 40–55 ms 而不是
33 ms：让帧之后引擎要做的第一帧同样包含这笔冷布局。

## 4. 对 `§5` 的判决

`cold-start-js.md` §5 的前提是「第一屏的渲染在等 JS 计算」。实测把它拆成两半：

- **在等的不是数据**：`first-data` 在首帧之后 30–35 ms。
- **在等的 JS 只有 19 ms，其中首屏 Overview 只有 1.4 ms**。把行模型、格式化、
  `Intl`、i18n 拼接搬到 Rust，能碰到的上限是这 1.4 ms 里的一小部分。

所以 §5 对**冷启动首帧**关闭。它对「打开一个几万条目的目录」那一帧仍然可能有意义
（那里的行渲染是真的在算数据），但那是另一条路径，得单独量：把冷启动换成
「带 folder handoff 的窗口」跑同一套 profiler 即可（`bootsOnFolder` 分支会挂载
explorer surface）。

## 5. 什么条件下重开

- **首屏开始渲染真实数据**（例如冷启动直接打开一个目录）：那时 `Overview` 的
  1.4 ms 会变成列表的 render，§5 的候选才重新出现。量法：同一套 profiler，
  窗口带 folder handoff。
- **CJK 字面不再是首屏文本**（英文 locale）：40 ms 里那 ≥19 ms 的字体成本会变，
  值得复测，但结论方向不会变（剩下的仍是首次布局）。
- **WebView2/Chromium 改了首次布局或字体初始化的实现**：§2.3 的两张表是基准。
- **React 侧 render 涨到 30 ms 以上**：那才值得再拆 profiler 子树。

## 6. 复现

```bash
# profiling build（不要出货）
DAE_PROFILE_REACT=1 bun tauri build --no-bundle --config src-tauri/tauri.release.conf.json

# 5 次，读 app[mount] / commit:*:end / pill:layout / dom-nodes
bun bench/cold-start/run.ts --runs 5
grep -hE "\[mount\]|commit:|pill:|dom-nodes" .perf/cold-start/run-*.log

# 跑完恢复普通 release
bun tauri build --no-bundle --config src-tauri/tauri.release.conf.json
```

`run.ts` 的表格只印 SPANS + 几个固定行；profiler 的行在原始日志里
（`.perf/cold-start/run-*.log`），这是故意的——它们每次运行的名字都带毫秒数。

## 7. 仓库里留了什么

- `Sidebar` / `TabStrip` / `Overview` 三个 `<Profiler>`：probe 关闭时零成本，
  下次拆挂载不用再改代码。
- `TabPerfProfiler.onRender` 的 `commit:<id>:end` 行。
- `dom-nodes:<n>`（first-frame 时）。
- `useTabPill` 的 `pill:layout:start/end`。
- 一次性诊断（`layout:probe1/2`、中文预热、全量重算探针）**没有留在代码里**；
  要复现按 §2.3 在 `main.tsx` 里临时加回，跑完删掉。
