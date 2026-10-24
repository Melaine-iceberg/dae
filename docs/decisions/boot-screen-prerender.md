# 预渲染第一屏（`cold-start-js.md` §2）：**已实测否决，Rust 预渲染不做**

> **状态：已实测，不改产品路径。** 静态骨架按 §8 第 3 条放进了 `index.html`，
> 用一个二进制里的运行时开关做了 A/B；结论是它**在 bundle 求值前根本没被画出来**，
> 而 `first-frame` 结构上不可能因此提前（它量的是 commit 之后的第一帧）。
>
> 唯一能让静态屏出现的是**在 `createRoot` 之前让出一帧**（仓库里以
> `DAE_BOOT_YIELD=1` 保留为诊断臂）：6/6 次在 commit 前上屏、停留约 80 ms，
> 代价是 `first-frame` **+40–53 ms**。那是「用 40 ms 的等待换 80 ms 的有内容画面」
> 的产品取舍，不是性能收益，也不是 Rust 该做的事。
>
> 顺带修正 `cold-start-js.md` §1.4 对 paint entry 的读法：它没有「不可靠」，
> 它是在如实报告「没有画过」。复现命令见 §7；A/B 与截图脚本在 `bench/cold-start/`。

## 0. 结论先行

1. **外壳档（静态骨架）单独放进 HTML = 零收益。** 两个会话各 9+9 次交错运行：
   36 次里只有 **1 次**在 `boot-eval-end` 前有帧（会话 1 的 splash 臂，见 §3.3），
   骨架臂 **0/18**；也就是说**入口 bundle 求值前引擎没有产生过帧**。截图也对得上：
   从窗口创建（~300 ms）到 React 首帧（~440 ms）窗口是纯空白（材质底），
   连原本的转圈都没出现过（§4）。
2. **原因不是引擎不肯画，是启动没有让它画的空隙。** HTML 到达 → 解析（18–20 ms）
   → bundle 求值（16–19 ms）→ 交接/i18n/chunk（4 ms）→ React 首次挂载（62–74 ms）
   是一条连续的主线程任务链；中间的任务间隙都比一个 vsync 短。
3. **让出两帧就够，成本却不止两帧。** `DAE_BOOT_YIELD=1`（`createRoot` 前
   `rAF` 两次）后：骨架 6/6 在 commit 前上屏，停留中位 **79.5 ms**（splash 臂
   75.6 ms），`commit → first-frame` 从 16.6–17.4 ms 降到 **1.4 ms**（合成器已经在
   产帧）。但 `first-frame` 本身从 432–440 ms 退到 **477–491 ms**：等的是引擎的
   **第一帧**（`chunk` 348.8 → 第一帧 389.4，40 ms），再加一帧，不是 2×16.7。
4. **所以 Rust 预渲染第一屏不做**：它改变不了「第一帧什么时候来」，而让第一帧
   之前有内容可画并不需要 Rust —— HTML 里已经有骨架了。内容档（Rust 渲染前
   ~30 行）只是在这个 ~80 ms 窗口里加细节，代价是 Rust 复制一份行渲染并与前端
   样式保持同步（§2 原文自己列的那笔账）。
5. **原本的 splash 一直没起作用**：它的旋转动画和静态骨架一样，都在 commit 之后
   才被画，等于窗口空白 ~140 ms 后直接出现 React 界面。这不是这次改坏的，
   是 §1.4「paint entry 只有 3/23 次出现」的另一种读法（§5）。

## 1. 被测的到底是什么

`docs/cold-start-js.md` §8 第 3 条把 A/B 的问题写成两句，实际上要分开：

| 问题 | 量法 | 结论 |
|---|---|---|
| `first-frame` 会不会提前？ | 现有 `DAE_STARTUP_PERF=1` 的时间线 | **不可能**：`first-frame` 的定义就是 commit 之后第一个 `rAF`，静态屏再早也被 React 在 commit 时清掉 |
| 引擎会不会在 deferred module 求值前把预渲染的 DOM 画出来？ | `index.html` 里的 `requestAnimationFrame` 探针（`html-frame-N`）+ 截图 | **不会**（没有让帧时）；**会**（让帧后） |

所以 A/B 的判据不是 `first-frame` 的差值，而是：

- `html-frame-N` 与 `boot-eval-end` / `commit` 的先后（每轮一个布尔值，数 N/M 次）；
- 静态屏在 commit 前停留多久（`commit − 最早一帧`，每轮一个毫秒数，取中位）；
- `first-frame` 的差值只用来说明「让它上屏要付多少」。

## 2. 测法：一个二进制、交错运行

两个 build 隔一小时各跑一遍，会被会话漂移吃掉（同一台机器上 `window-built` 段
两次会话能差 15–20 ms）。所以骨架**固定在 `index.html` 里**，arm 由 Rust 的
`initialization_script` 在页面任何脚本之前注入：

| 开关 | 作用 |
|---|---|
| `DAE_BOOT_SKELETON=1` | 页面显示静态骨架，隐藏 splash（默认关闭，出货行为不变） |
| `DAE_BOOT_YIELD=1` | `createRoot` 前让出两帧（诊断用，默认关闭） |

`bench/cold-start/run.ts --ab` 把两个 arm 交错跑（每轮交换先后），`--runs` 是
每臂次数；`--yield` 给两臂都打开让帧。两侧都显式写 `DAE_BOOT_*=0/1`，防止
从父进程继承到另一臂的设置。

`first-paint` 的坑（§1.4）本来会让这个 A/B 量不出结论，所以加了一个不依赖 paint
entry 的探针：`html-parsed` 时连续申请 3 个 `rAF`，每个回调记一条 `html-frame-N`。
`rAF` 只在引擎真的走渲染步骤时触发，因此「第一帧早于 `boot-eval-end`」就是
「静态屏在 JS 跑完之前已经画了」。

## 3. 数字

口径：Windows 11 / WebView2 / 本机，普通 release（无 PGO/BOLT），
`DAE_STARTUP_PERF=1`。除特别说明外是每臂中位，ms since process start。

### 3.1 无让帧：A/B 本身（会话 1，9+9 次交错）

| | splash（今天） | 静态骨架 | Δ |
|---|---:|---:|---:|
| `html-parsed` / `boot-eval-start` | 325.0 | 326.8 | +1.8 |
| `boot-eval-end` | 342.7 | 344.3 | +1.6 |
| `render` | 346.8 | 349.2 | +2.4 |
| `commit` | 415.8 | 420.2 | +4.4 |
| **`first-frame`** | **432.4** | **437.6** | **+5.2** |
| `commit → first-frame` | 16.6 | 17.4 | +0.8 |
| `html-frame-1` | 430.6 | 437.3 | +6.7 |
| `paint:first-paint` | 438.7（仅 1/9 次有） | 无 | — |
| **在 `boot-eval-end` 前有帧** | **1/9** | **0/9** | |
| **有画面早于 `commit`** | **1/9** | **0/9** | |

会话 2 重跑同一组（9+9）复现：`first-frame` 437.7 / 440.4（Δ +2.7），
`frames before bundle eval` **0/9 / 0/9**，`screen before React commit` **0/9 / 0/9**，
**两臂 18 次里一条 paint entry 都没有**。

也就是说骨架那 +2.7~5.2 ms 不是「提前」，而是它被实例化、布局却没画出来的成本
（两臂都解析同一份 HTML，只有 `display` 不同，所以差值不是标记的体积，而是
隐藏子树之外的样式/挂载代价）。它落在噪声里，但不是收益。

### 3.2 让帧之后：静态屏真的出现了（会话 3，6+6 次交错，`--yield`）

| | splash + yield | 骨架 + yield |
|---|---:|---:|
| `html-parsed` | 340.0 | 346.5 |
| `boot-eval-end` | 358.9 | 362.7 |
| `html-frame-1` | 402.2 | 408.4 |
| `paint:first-paint` | 443.3 | 453.5 |
| `commit` | 475.7 | 489.8 |
| **`first-frame`** | **476.8** | **491.2** |
| `commit → first-frame` | 1.1 | 1.4 |
| **在 commit 前有画面** | **6/6** | **6/6** |
| **静态屏停留（中位）** | **75.6 ms** | **79.5 ms** |

会话 3 的 WebView2 段本身偏慢（`window-built` 中位 300 ms，会话 1 是 282 ms），
所以绝对值和上面那张表不能逐格比。可比值有两个：

- **让帧的代价**：`first-frame` 从 432–440 → 477–491，即 **+40–53 ms**。构造上两次
  `rAF` 是 2 帧，但实际等的是「引擎的第一帧」：一轮里 `chunk` 348.8 ms，
  第一条 `html-frame-1` 389.4 ms（**40 ms**），再加一帧。第一帧来的早晚不受
  HTML 里有什么影响 —— 这就是 §0 第 4 条的根据。
- **骨架一旦真的上屏，同会话里比 splash 多花 ~8 ms**（`html-parsed` 已晚 6.5 ms，
  `commit` 晚 14.1 ms）：一次真正的布局 + 绘制 + ~25 个脉冲动画。仍然是毫秒级，
  但方向是「更慢」，不是「更快」。

`commit → first-frame` 掉到 1.4 ms 也值得记一笔：一旦引擎开始产帧，commit 之后
那一帧是立刻有的；没有让帧时那 16–17 ms 其实是「合成器还没热身」，不是 React 的事。

### 3.3 唯一一次「骨架之外提前画了」

会话 1 的 splash 臂有 1/9 次 `html-frame-1` 落在 `boot-eval-end` 之前（404.9 vs
416.6，那一轮 `html-parsed` 本身晚到 403.5、commit 晚到 489.8），并且是 18 次里
唯一一次出现 `paint:first-paint`（438.7）的运行。会话 2 重跑没有再出现。
读法是「启动链偶尔会被一次外部延迟切成两段，切点上正好落进一个 vsync」，
不是可以设计的东西；6+6 次让帧实验才是稳定的对照。

## 4. 截图：窗口里到底有什么

`bench/cold-start/bootshot.ps1` 从 launch 起按固定偏移截屏（只作视觉确认，
截屏本身会扰动时间，数字一律用 `run.ts`）：

| 时刻（相对 launch） | 无让帧，两臂相同 | `DAE_BOOT_YIELD=1` + 骨架 |
|---|---|---|
| ~390–430 ms | 空白（Win11 材质底，透过透明窗口看到桌面） | 空白 |
| ~460 ms | 空白 | **骨架**（标签条/侧栏/空列表） |
| ~510 ms | 空白 | **骨架** |
| ~520–560 ms | React 界面 | React 界面 |

无让帧时，从窗口出现到 React 上屏**整段都是空白**：splash 的转圈和骨架一样，
一帧都没上过屏。这与 §1.2 的 `first-data` / `commit` 位置一致，也和 §1.4 里
paint entry 的稀有程度一致。

## 5. 顺带修正 `cold-start-js.md` §1.4

原文：「`paint:first-paint` 在 WebView2 上不可靠（23 次里只出现 3 次，而且位置
互相矛盾）」。

实测补上了缺席的那一半：**没有让帧的 36 次里只出现 1 次**，而那一次恰好是唯一
一次在 bundle 求值前有帧的运行；**让帧的 12 次里 12 次都有** `paint:first-paint`
（8 次还带 `first-contentful-paint`）。所以它不是随机缺失，而是「页面上屏之前
引擎不记 paint entry」。位置偏差仍在（让帧那组 `first-paint` 比 `html-frame-1`
晚 ~40 ms，截图显示真正看见骨架也在两者之间），所以**先后顺序可信、绝对时刻
不要太当真**；判断「有没有画」用 `html-frame-N` 更稳。

## 6. 什么条件下重开

- **产品愿意用 ~40–55 ms 的 `first-frame` 换 ~80 ms 的有内容画面**：那时要做的
  是「让帧」这条路（一帧还是两帧、`rAF`+`setTimeout` 还是双 `rAF` 都还没 A/B），
  不是 Rust 预渲染。骨架已经在 `index.html` 里，`DAE_BOOT_YIELD` 把它跑起来。
- **引擎的第一帧能更早**（比如 WebView2 的 controller 可见性/合成器预热被上游改掉）：
  §3.2 的代价会跟着变，值得重测。
- **内容档要证明它比骨架值**：唯一可能的读数是在那 ~80 ms 里用户能否分辨
  「骨架」和「前 30 行真实条目」。那要先有骨架 + 让帧的产品决定。
- **首帧的定义如果改了**（例如以后用「首内容帧」当冷启动指标）：结论要重算，
  但 `html-frame-N` 探针仍然适用。

## 7. 复现

```bash
# release（普通，无 PGO/BOLT）
bun tauri build --no-bundle --config src-tauri/tauri.release.conf.json

# A/B：splash vs 静态骨架，9 次/臂，交错
bun bench/cold-start/run.ts --runs 9 --ab

# 让帧诊断：两臂都在 createRoot 前让两帧
bun bench/cold-start/run.ts --runs 6 --ab --yield

# 单臂
bun bench/cold-start/run.ts --runs 9 --skeleton

# 视觉确认（会扰动时间，只看「有没有画」）
powershell -NoProfile -ExecutionPolicy Bypass -File bench/cold-start/bootshot.ps1 -Skeleton 1
$env:DAE_BOOT_YIELD="1"
powershell -NoProfile -ExecutionPolicy Bypass -File bench/cold-start/bootshot.ps1 -Skeleton 1
```

`run.ts` 的输出里：

- `frames before bundle eval`：`html-frame-N` 早于 `boot-eval-end` 的运行数；
- `screen before React commit` / `static screen before commit (N/M)`：早于 `commit` 的运行数与停留中位；
- read-out 分三支：没有让帧时两者都是 0，就是「静态屏从未上屏，内容档没有立足点」；
  只有后者非 0 时，说明画面是让帧换来的（`first-frame` 更晚）。

## 8. 仓库里现在留了什么

- `index.html`：骨架标记 + 样式（`__DAE_*__` 由 `vite.config.ts` 从 `App.css` 注入：
  chrome / fill / ink / rail 宽度 / strip 高度），`html[data-boot-skeleton]` 默认不开；
  以及 `html-frame-N` 探针（只在 `DAE_STARTUP_PERF=1` 时注册）。
- `src-tauri/src/lib.rs`：`boot_ab_script()` 注入两个开关（都是 opt-in）。
- `src/main.tsx`：`__DAE_BOOT_YIELD` 的两帧让步（opt-in，6 行）。
- `bench/cold-start/`：A/B、让帧、截图三个入口。

出货路径一行没变：默认仍是 splash，`first-frame` 与 §1.2 的 431–481 ms 一致。
若确定不做「骨架 + 让帧」，把 `index.html` 的骨架块、`boot_ab_script()`、
`main.tsx` 的让步和 `bootshot.ps1` 一起删掉即可；`html-frame-N` 探针建议留着
（它取代了会缺席、位置也有偏差的 paint entry，成本是一次 `if`）。
