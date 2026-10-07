# 预热窗口池：让标签撕出看起来即时

> **状态：已实施并手工验证通过（`DAE_TAB_POOL=1` 开启，默认关闭）。**
> 两份未验的量都已量到：收养成本 **103ms**（§5.1），隐藏期可完成的工作见 §1.2。
> 实现于 `tab_windows.rs`（池 + 状态机 + 收养分支，7 个单测）与前端
> `explorer-tabs.tsx` / `tabs.ts`（收养监听 + 替换语义 + 就绪握手）。
> 手工验证：撕出、合并、再撕出均无异常，体感显著变快（本轮未取数）。
> **下一步待定：是否改为默认开启。**
> 其余数字均为生产构建实测（Linux / WebKitGTK / Wayland / niri / 混合显卡），
> 复现命令见 §7。

## 0. 结论先行

标签撕出目前约 **530ms** 才有内容（中位）。其中：

- **~390ms 可以预先付掉**（实测：隐藏窗口在无人观看时完成了从 bundle 加载到 React commit 的全过程）
- **~93ms 必须发生在显示之后**（绘制）
- 代价：**+200MB 常驻**，加一套「收养已启动窗口」的机制

所以池子的目标是：把用户感知的等待从 ~530ms 压到 **~100–150ms**。

## 1. 实测依据

### 1.1 成本分解（生产构建，多次撕出取中位）

| 段 | 中位 | 范围 | 性质 |
|---|---:|---|---|
| RELEASE → BUILT | ~20ms | 18–22 | 纯 Rust（建窗 + spawn） |
| **SHOWN → bundle** | **~250ms** | 213–404 | **webview 冷启动 + 模块加载** |
| bundle → render | ~17ms | 3–43 | locale / 交接 IPC / chunk |
| render → commit | ~145ms | 94–168 | React 挂载 + 中间的空档 |
| commit → paint | ~95ms | 7–101 | 布局与绘制 |
| **合计** | **~530ms** | 268–707 | |

`SHOWN → bundle` 是单项最大成本，但**波动也最大**（213–404ms，取决于系统缓存状态）——
这是池子收益最不确定的一环。

**React 的实际工作量只有 19–61ms**（React `<Profiler>` 的 `actualDuration`）。
`render → commit` 那 ~157ms 里大部分是**等待**，不是计算。所以**优化组件没有意义** ——
上限就是这几十毫秒。

### 1.2 隐藏窗口探针（决定性）

`DAE_TAB_PERF_HIDE_MS=4000` 让撕出的窗口建好后 **4 秒不显示**，测它自己干了多少活。
四次结果一致：

| 标记 | 时间 | 显示前完成了？ |
|---|---:|---|
| SHOW_DEFERRED | 20.0 | — |
| bundle | 245.0 | ✅ |
| locale / handoff / chunk | 246–260 | ✅ |
| render / tick | 260–261 | ✅ |
| **commit** | **395.0** | ✅ |
| query[git-status]:success | 399.0 | ✅ |
| ——— **SHOWING** | **4021.4** | ← 4 秒后才显示 |
| **first-frame** | **4114.0** | ❌ **只有绘制被推迟** |
| gfx:visible | 4114.0 | 首帧时窗口已可见 |

**结论：除了绘制，全部工作都能在隐藏状态下完成。** 显示后只剩 **93ms**。

这也说明 `requestAnimationFrame` 在隐藏时**确实被节流**（`gfx:visible` 证明首帧只发生在
可见之后）—— 但 React 的首次挂载不需要 rAF，所以挂载照样完成了。

### 1.3 已排除的可能

| 假设 | 结论 |
|---|---|
| React 渲染是瓶颈 | ❌ 仅 19–61ms |
| 列表内容量影响渲染 | ❌ 已虚拟化，只挂 ~40 行 |
| 等 lazy chunk | ❌ 首帧是 Overview，eager import |
| 等数据 | ❌ `query[...]` 在 first-frame 之后才到 |
| 软渲染更慢？ | ❌ 反证：`WEBKIT_DISABLE_DMABUF_RENDERER=1` 让应用卡到不可用，说明 GPU 路径正常 |
| 主线程忙于长任务 | ⚠️ **无依据**：WebKit 不支持 longtask（`longtask:unsupported`），观察器报不出任何东西 |

## 2. 方案形状

### 2.1 空壳池 vs 预启动池

| | 空壳池 | **预启动池（选定）** |
|---|---|---|
| 池内窗口状态 | 刚建好，未加载页面 | 已 boot、React 已 commit |
| 能省 | 冷启动 ~256ms | **~390ms** |
| 剩余等待 | ~275ms | **~93ms + 收养开销** |
| 复杂度 | 低 | 高（需要收养） |

由 §1.2，空壳池白白浪费了「React 挂载也能在隐藏时完成」这一事实，所以选**预启动池**。

### 2.2 收养流程

```
用户拖动标签 → 判定为撕出
  ├─ 池中有已启动窗口？
  │    ├─ 有 → 收养：把它的 React 树重新指向目标标签（见 §3.1）
  │    │        → 定位到指针处 → show() → 绘制（~93ms）
  │    └─ 无 → 回退到现有冷路径（见 §3.4）
  └─ 异步补充池（见 §3.3）
```

### 2.3 可以复用的既有机制

- **交接 payload**：新窗口取「要显示什么」的既有通道（`takeTabHandoff`），收养应当复用它，
  而不是新造一条
- **注入锚点**：`initialization_script` 已在用（材质与埋点），池窗口建窗时可直接沿用
- **`linux_graphics` / `window_material` 的 `configure`**：建窗时的既有装饰路径
- **⭐ 收养本身**：`merge_tab_into_window(target, payload, x, y)`
  （`tab_windows.rs:267`）已经存在，它只做一件事 —— 向目标窗口 emit 一个事件，
  由接收方前端的 `mergeTabFromHandoff`（`tabs.ts:186`）把标签加进去。
  **「把标签移进一个已存在的窗口」就是收养**，吸回（soak）走的就是这条路。
  ⇒ 池化缺的**不是收养机制**，而是：① 池窗口的生命周期；② 抑制 refetch（§3.2）；
  ③ 定位与 show。

  **由此带来的好处**：收养成本**不再需要先造池子才能测** —— 它可以在真实的吸回上量。
  埋点 `adopt` / `adopt-painted` 已加（`tabPerfAdoptProbe`）。

  ```bash
  # 开两个窗口，把一个标签从 A 拖进 B（吸回），看 B 的输出：
  DAE_PROFILE_REACT=1 bun run tauri:build:release --no-bundle
  DAE_TAB_PERF=1 ./src-tauri/target/release/dae
  #   adopt           t+…
  #   adopt-painted   t+…   ← 两者之差即收养成本，与 §5.1 的 93ms 预算对比
  ```

## 3. 硬问题

### 3.1 收养时的状态重置（最大的一块）

被收养的窗口**已经是一个独立的 app 实例**：自己的 tabs store、query cache、文件监听、
窗口 label、已经跑过的 `bootstrap()`。收养意味着把「它现在显示的这个标签」换成目标标签。

需要重置或重新指向的东西：

| 项目 | 风险 |
|---|---|
| tabs store（当前标签、顺序） | 池窗口有自己的默认标签，必须换成目标标签 |
| query cache | 里面的数据是**别的目录**的；不清理会短暂显示错内容 |
| 文件系统监听 | 可能挂在池窗口的初始目录上 |
| 窗口 label | 交接与事件路由都按 label 走，收养后是否要改？ |
| `bootstrap()` 的幂等性 | 池窗口已经跑过一次，收养是「第二次」，必须确认可重入 |

**这是整个方案复杂度的来源**，也是 §5.1 要实测的东西。

### 3.2 显示时的 refetch 风暴（**已实测**）

SHOWN 之后观测到：

```
SHOWN                        4021.5
query[git-status]:success    4059.0   ← 又请求了一遍
query[git-branches]:success  4060.0
first-frame                  4114.0
```

窗口变为可见触发了 react-query 的重新取数（`refetchOnWindowFocus` 一类）。
**收养时必须抑制这次重取**（或复用在途请求），否则它会吃掉刚省下的时间，
在慢目录上甚至可能比冷启动更糟。

### 3.3 池的大小与补充时机

- **大小**：1 个大概够（撕出是串行的）。要确认「连续快速撕出多个」时的行为。
- **补充时机**：立刻补会与用户的下一次操作抢 CPU（而补充本身是 ~390ms 的重活）；
  空闲时补则可能来不及。建议**延迟 + 空闲**策略，并接受池空时走回退路径。

### 3.4 回退必须保留

池为空、收养失败、或平台不支持时，**必须能走回现有的冷路径**。这意味着池是
**纯增量优化**，不是替换 —— 否则池的任何 bug 都会让撕出彻底不可用。

### 3.5 内存预算，以及与已知泄漏的相互作用

- 每个 webview 进程实测约 **200MB PSS**（需用 `docs/decisions/poc/webview-reparent/webview-memory.py` 重测，
  当时的数字是旧机器/旧版本）
- 池常驻 1 个 = **+200MB**
- **与 §4.7 的泄漏相互作用**：`WebKitWebProcess` 在窗口关闭后可能不退出
  （wry 的 GObject 引用环，见 `tab-drag-webview-reparent.md` §4.7）。
  池化会**减少**窗口的创建/销毁次数，所以对泄漏是中性偏好；
  但如果收养后反复关闭池窗口，泄漏会累积。**建议池化与 §4.7 一起评估。**

## 4. 必须满足的约束

- **隐藏窗口不得抢焦点、不得出现在任务栏/窗口切换器**（GTK 需显式设置 skip-taskbar 一类属性）
- **定位**：niri 是可滚动平铺合成器，窗口定位语义与常规 WM 不同；收养后必须落在指针处，
  需要按合成器验证（本项目已有 `tab_windows.rs` 的原生拖动定位逻辑可参考）
- **多显示器 / 多工作区**：池窗口建在哪个输出上？跨输出收养是否要重建？
- **平台**：以上结论全部来自 **Linux / WebKitGTK**。Windows（WebView2）与 macOS（WKWebView）
  的隐藏窗口行为**未测**，可能完全不同 —— 池化在其它平台是否成立需要单独验证。

## 5. 未验与风险

### 5.1 收养成本（唯一的关键未知）

**已实测（2026-10-07，一次样本）：收养成本 = 103ms**

在真实吸回上量的（两个窗口，标签从 A 拖进 B），接收方窗口已启动 22 秒：

```
adopt                        21992.0
  → 第一次 React 提交          +33ms
  → React 实际工作（mount 27 + nested 12 + 1）   43ms
  → 查询等待（git-status 重取）                  42ms   ← §3.2 的陷阱真的发生了
  → 最后一次提交 → 画面                           9ms
adopt-painted                22095.0   （+103ms）
```

两点：
1. **§3.2 在真实收养中发生了**：`adopt` 后立刻触发一批 `query[git-status]`，
   而渲染**等了它们 ~42ms** —— 占收养成本的 **40%**。抑制重取后收养可能降到 ~60ms。
2. Profiler 在旧版的 7–19ms 预期**偏乐观**，真实 React 工作是 43ms。

**样本局限**：这次接收窗口是**可见**的，所以 103ms 含一次真实绘制；
池窗口是隐藏的，React 工作（43ms）可在隐藏期存下，绘制则要等 show。
**结论对池子有利**：`adopt` 必须与「目标标签在体内的随机性」一起设计 —— 即池化必须先知道要显示什么。

### 5.2 池窗口的数据是「别人的」

池窗口的 query cache 里是它初始目录的数据。收养后目标目录必须重新取数 ——
若该目录慢，这可能是新的瓶颈。§1.2 里目标面的数据在 399ms 就绪，但那是**池窗口自己的**默认目录。

### 5.3 冷启动数字的可移植性

`SHOWN → bundle` 的 256ms 有较大波动（243–404ms，取决于系统缓存状态）。
在其它机器、其它版本上可能显著不同，从而改变池子的收益。

## 6. 建议的实施顺序

每一步都应可独立验证，且**失败时可随时停下**：

1. ~~**只测收养成本**~~ **已完成（见 §5.1）：103ms，远低于作废阀值**，方案继续。
   已用真实吸回路径测出（`tabPerfAdoptProbe`），无需先造池子。
2. **池 + 回退**（池大小 1）：加池与补充逻辑，保留冷路径作为回退。
3. **抑制 refetch**：修 §3.2 —— 实测占收养成本 **40%**，优先级应从第 3 步提到
   与第 2 步并列。
4. 视情况：多显示器、非 Linux 平台。

### 收益估计（基于实测，修正早前的乐观估值）

| | 用户等待 |
|---|---:|
| 现状（冷路径） | **~530ms** |
| 池化：收养（可在隐藏期存下大部分）+ show → 绘制 | **~180ms** |
| 若再抑制 refetch（§3.2） | **~140ms** |

即约 **3 倍提升**，代价 **+200MB**。（早前估的「~93ms」漏算了收养本身。）

## 7. 复现命令

```bash
# 生产构建（必须带 overlay，否则 updater 配置为 null，二进制一启动就 panic）
bun run tauri:build:release --no-bundle

# 基础测量（中位 ~530ms）
DAE_TAB_PERF=1 ./src-tauri/target/release/dae

# 隐藏窗口探针：证明 ~390ms 可在显示前付掉（§1.2）
DAE_TAB_PERF_HIDE_MS=4000 DAE_TAB_PERF=1 ./src-tauri/target/release/dae

# React 子树耗时（需要 profiling 构建；只有相对占比有意义）
DAE_PROFILE_REACT=1 bun run tauri:build:release --no-bundle
DAE_TAB_PERF=1 ./src-tauri/target/release/dae

# 量内存（进程树 RSS + PSS）
python3 docs/decisions/poc/webview-reparent/webview-memory.py
```

埋点默认关闭；不设 `DAE_TAB_PERF` 时全部 inert（Rust 不记录，前端直接 early-return）。
`DAE_TAB_PERF_HIDE_MS` 不设时解析为 0，**走与出货完全相同的 `show()` 调用点**。

## 8. 相关文档

- `docs/decisions/tab-drag-webview-reparent.md` —— 被否决的 reparent 路线，
  以及 §4.7 的 `WebKitWebProcess` 泄漏（与 §3.5 相互作用）
- `docs/decisions/poc/webview-reparent/README.md` —— 归档的 PoC 与内存测量脚本
