# 性能机会清单

> **状态：清单**。第 1 条（名称搜索的忽略规则）、第 2 条（Git 徽标的全工作区扫描）、
> 第 3 条（前端每 publish 的全量重扫）与第 4 条（内容搜索的每文件 `Searcher`）
> **已实施**，数字见各自那一节；第 5 条（零散）仍待办，其中预览响应复制字节那条
> 是一行的事。
>
> 这份文档记录的是「主干已经打磨过之后，还剩哪些量级够大的浪费」——
> 不是猜测清单，每条的「证据」都指向可复现的 harness 或代码里的事实。
> 判断标准沿用 `docs/decisions/` 的习惯：**先量，再改**；量不到就先记着。

## 0. 已经做过的（不要重复提）

| 已经优化过 | 证据 |
|---|---|
| 批次流式列举 + 列式 packet 通道（5 MB 列表 25 ms 解析 → 0.56 ms 交接） | `src-tauri/src/file_system/listing.rs`、`src/features/explorer/entry-codec.ts` |
| 排序移出主线程（35,803 条 ~123 ms → ~5 ms 主线程） | `src/features/explorer/sorted-entries.ts`、`entry-sort-worker.ts` |
| 行对象缓存 + 视图访问器（不物化整份列表） | `src/features/explorer/listing-view.ts` |
| 目录变更走「按名字打补丁」而不是重读 | `navigation.ts::patch`、`watch.rs` |
| IO 机制迁移（IOCP / io_uring）已实测否决 | `docs/decisions/completion-io.md` |
| 冷启动：locale、explorer chunk、启动预取、预热窗口池 | `main.tsx`、`prefetch.rs`、`docs/decisions/tab-tear-off-warm-pool.md` |

这一层的结论是：**列举、排序、IPC、变更通知基本到顶了**。下面几条的共性不是「算法不够好」，
而是「某条路径还没人对它做过判断」——要么还在全量遍历不该遍历的东西，要么每帧都在重算同一份东西。

> 一处例外：Windows 枚举的「`FindFirstFileExW` + `FIND_FIRST_EX_LARGE_FETCH`」
> 曾经实测过（35,803 条 24.2 ms → 13.7 ms，−43%），但那批改动已从工作区回退，
> 当前代码是 `std::fs::read_dir` + 逐条 `metadata()`。所以「列举到顶了」不适用于 Windows，
> 那一块重新开工时应当从这次测量接着走。

---

## 1. 名称搜索会走进 `.git` / `node_modules` / `target` —— 已实施

**位置**：`src-tauri/src/file_system/local/search.rs`（`standard_filters(false)`）、
前端 `directory-search.tsx` → `commands.searchDirectory`。

**现象**：递归名称搜索关掉了 `ignore` 的全部默认过滤，于是从项目根搜一个名字会把
整个 `node_modules` 走一遍。同一个仓库里：

| | 条目数 |
|---|---|
| `node_modules/` | **72,106** 个文件 + 3,527 个目录 |
| 项目自身（`git ls-files`） | 460 个文件 |

**这就是 99.4% 的遍历量花在依赖目录上。** 后果是两个方向的：

**（a）命中少的时候——慢。** 查询 `explorer`（项目里只有 17 条命中，不会触发上限）
走完整棵树，中位 **41.81 ms**（`min/max 39.88 / 43.17`），换来 17 条结果。

**（b）命中多的时候——快但错。** `MAX_SEARCH_RESULTS = 200` 是**先到先得**截断，
排序发生在截断*之后*，而 `ignore` 的 walker 是多线程的、到达顺序不确定：

| 查询 | 结果 | 其中来自 `node_modules` | truncated |
|---|---|---|---|
| `readme` | 200 | **194** | true |
| `index` | 200 | **180** | true |
| `json` | 200 | **141** | true |

`readme` 那一行就是病态的例子：项目自己的 6 条命中（`README.md`、`docs/`、
`bench/`）只因为它们**恰好先到达**才留在结果里；并行 walk 换个顺序，用户要找的那份
`README.md` 就可能根本不出现在列表里。

对照：内容搜索一直都有这份忽略表，`ALWAYS_IGNORED = [".git", "node_modules", "target"]`
——名称搜索只是没对齐。本次把这份表提到 `local/ignores.rs`，两种模式共用一处定义。

**改动**（本次）：

1. `search.rs` 用与内容搜索同一份规则（提到 `local/ignores.rs`，一处定义，两种模式不会打架）。
   规则相对于 walk 根，所以**在 `node_modules/x` 里搜索仍然能搜到**——被排除的是根的下级，
   不是根本身。
2. 截断改为「先收集候选，再按既有的 match_rank 排序，最后截断到 200」，
   `truncated` 改为诚实地表示「还有没显示的命中」。

**改动后的实测**（同一台机器、同一命令、`--release`；`sources` 是命中真正来自哪个目录）：

| 查询 | 改动前 | 改动后 |
|---|---|---|
| `explorer`（命中少 → 走完整棵树） | 17 条 / 中位 **41.81 ms**（39.88–43.17） / `{node_modules: 1, src: 16}` | 16 条 / 中位 **4.27 ms**（3.63–5.66） / `{src: 16}` |
| `readme` | 200 条 / truncated / `node_modules` 占 **194** | **6 条** / 不截断 / `{README.md: 1, bench: 3, docs: 2}` |
| `index` | 200 条 / truncated / `node_modules` 占 **180** | **2 条** / 不截断 |
| `json` | 200 条 / truncated / `node_modules` 占 **141** | **27 条** / 不截断 |

`explorer` 从 17 条变 16 条，是因为去掉的那一条本来就来自 `node_modules`。
`json` 从 200 条变 27 条，缩掉的部分除了 `node_modules`，还有改动前来自 `bench/` 的 32 条——
那些是 `bench/*/target/` 里的 Cargo 产物，同样不该出现在「项目里的 json」里。

代价侧：`MAX_SEARCH_CANDIDATES = 5 × 200`，只在命中密集的树上才会真正多走一段；
命中稀疏时（绝大多数情况）walk 的成本在「走到命中」，不在「数命中」。

**测试**：`search.rs` 新增 3 个：跳过依赖/VCS 目录、以被跳过目录为根时仍能搜到里面、
命中数超过显示上限时按 rank 保留（钉子钉在「先排序再截断」这个契约上）。内容搜索的
原有测试覆盖了共享规则重构。

**复现**：

```bash
cd src-tauri
# 名称搜索基准（中位 ms + 结果来源分布）
DAE_SEARCH_BENCH=K:/code/dae DAE_SEARCH_BENCH_QUERY=readme \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_name_search
```

**还没做的两部分**：

- 搜索框里「包含忽略目录」的开关（默认关）。后端要加一个命令参数，
  会带动 `bindings.ts` 与 UI/i18n 的改动，属于产品决定，不在这次改动里。
- 每条被访问条目的名字仍然会 `to_lowercase()` 分配一次（`search.rs` 的匹配处）。
  在「用户确实要搜一个巨大目录」时值得补一条 ASCII 快路径。

---

## 2. Git 徽标 = 每个目录一次全工作区扫描 —— 已实施

**位置（改前）**：`src-tauri/src/file_system/git.rs::git_status`（`options.pathspec` 的注释
写明了为什么不能限制扫描范围）、`src/features/explorer/git-status.tsx`（query key 是
`[git-status, directoryPath]`）、`src/features/workspace/projects-section.tsx`。

**现象**：libgit2 的默认 status 要把 worktree 里每个 tracked 文件 stat 一遍，
而结果只用到「当前目录的直接子项」。也就是说，在单仓多包的大仓里，
**每进入一个目录就付一次 O(worktree) 的扫描**，再把 99% 的结果丢掉。

放大它的三件事：

- query key 带 `directoryPath`，所以每个新目录都是一次新扫描；`staleTime` 只有 5 s，
  来回导航很容易反复付。
- 目录变更事件（400 ms trailing debounce）后 `invalidateQueries` 再付一次。
- 它跑在 blocking 池上不假，但它和列举、传输**抢同一块盘** —— 表现出来是
  「列表读取变慢」，而不是「徽标慢」。

还有一处比导航更糟：工作区概览挂载时会把最近 8 个目录各探一次
（`projects-section.tsx` 的 `DETECTION_LIMIT = 8`），这些目录常常都在**同一个**
monorepo 里 —— 也就是同一份全仓扫描跑 8 次。

**为什么不能简单地加 pathspec**：`git.rs` 的注释已经写明——pathspec 会让 libgit2
从该目录内部起步，未跟踪目录不再折叠成一条记录，`directory_untracked` 契约就破了。
所以要走这条路，得先决定那个契约怎么改。

**改动前实测**（`reports_the_cost_of_a_git_status_scan`，同一台机器）：

| worktree | tracked | 单个目录的读取 | 那次读取给出的徽标 |
|---|---|---|---|
| `K:/code/dae` | 460 | 5.19–5.50 ms | 0–5 条 |
| 合成仓（`DAE_GIT_BENCH_FILES=20000`） | 20,000 | 33.6–35.9 ms | 0–10 条 |
| 下界：`git status --porcelain`（子进程） | 460 / 20,000 | 19.7 / 46.4 ms | — |

也就是说：同一个仓里 root → d000 → d001 → d002 → d003 这一串导航，20k tracked 时
是 **~173 ms** 的全工作区扫描，每一步只为了两三条徽标。

pathspec 那条路（方向 2）实测：受限扫描 11.7–14.9 ms vs 33.6–43.3 ms，确实便宜，
但 `d003/nested` 那一行把契约破在哪里写得很清楚 —— `directory_untracked` 从
**true 变成 false**（记录数 1 → 0），也就是「整个目录未跟踪时所有子项标 U」这条
前端契约丢了，和 `untracked_subdirectory_aggregates_on_parent_and_resolves_inside`
钉住的正是同一件事。所以这次走方向 1。

**改动**（本次，方向 1）：

1. 扫描与过滤拆开：`scan_worktree_status`（O(worktree)，唯一昂贵的一步）+
   `derive_directory_status`（O(记录数)）。过滤就是原来那段代码原样搬过去，
   **缓存只换数据来源、不换算法**，所以逐目录的答案与改前逐字相同。
2. 新增 `GitStatusCache`（managed state，`lib.rs`）：每个 worktree root 存一份最近扫描
   （`Arc<[记录]>`），键是「watcher 变更计数 + `.git/index` 的 (mtime, len) + HEAD 的
   target」，外加 **5 秒复用窗口**。
   - watcher 变更计数由本地目录 watcher 上报变更时递增（`note_change`）：
     未跟踪文件不动索引，只有这条信号能抓到它；同一处清空整个缓存（扫描很大，
     留着只是推迟重算）。
   - index / HEAD 两个键覆盖 watcher 看不到的 git 操作：`add`、`commit`、`checkout`，
     包括在集成终端里跑的那些。
   - 5 秒就是前端已经声明的 `GIT_STALE_TIME_MS`：窗口内复用，窗口外重扫，
     不会显示比 UI 自己已经容忍的更旧的徽标。
   - 同一个 worktree 的 slot 锁在扫描期间持有，所以两个面同时看一个仓只扫一次
     ——工作区概览那 8 个并发探针就是这个形状。
3. 前端一行没动：query key、`staleTime`、400 ms debounce 都保持原样，因为每次请求
   现在只花 ~1 ms。

**改动后实测**（同一台机器、同一条导航序列）：

| 测量 | 改动前 | 改动后 |
|---|---|---|
| `dae`（460 tracked）每次目录读取 | 5.19–5.50 ms | 首次 5.20 ms，**之后 0.89–0.98 ms** |
| 20k tracked 每次目录读取 | 33.6–35.9 ms | 首次 34.8 ms，**之后 1.64–1.74 ms** |
| 20k：root→d000→d001→d002→d003 一串联 | **~173 ms** | **~42 ms** |
| 复用后剩下的 1.6 ms 是什么 | — | 1.52 ms 是 discover/打开仓库 + 分支 + 键，过滤 ~0.04 ms |

**测试**：`git.rs` 新增 6 个。同一 worktree 的第二个目录复用同一份扫描（`Arc::ptr_eq`，
并先钉住「两次 discovery 拼出的 worktree 字符串必须一致」，否则每个目录各占一个 slot）；
watcher 上报的变更让扫描失效（并先钉住「没有上报就抓不到未跟踪文件」——这正是 watcher
必须调 `note_change` 的理由）；watcher 看不到的 index/HEAD 变化同样失效；超过窗口不复用；
未跟踪目录契约经缓存仍然成立；四个并发读者只付一次扫描。原有 15 个行为测试与那个
ignored harness 全部通过——行为测试走的是无缓存的参考路径（`#[cfg(test)] git_status`）。

**交易掉的、以及还没做的**：

- **新鲜度**：缓存的新鲜度就来自事件流。没有任何 watcher 在看的目录里发生的改动，
  如果落在一次扫描之后、且还在 5 秒窗口内，首次进入那个目录可能看到一份旧扫描；
  窗口之外，或索引/HEAD 变了，或任何 watcher 报了变更，都会重扫。改动前「每次进入
  都重扫」是更保守的新鲜度，这是这次换掉的东西，所以窗口取的是前端已经容忍的那 5 秒。
- **复用的下界**是每次请求重新 discover/打开仓库（20k 仓 1.5 ms）。要再往下压，
  得让前端把 worktree root 一起传过来，或者按目录路径缓存「目录 → worktree」。
- `directory_untracked` 契约保持不变，pathspec 那条路仍然关着，理由就是上面那行实测。
- 分支查询（`list_git_branches`，列引用 + ahead/behind）不在这次缓存里。
  它和徽标共用那个 400 ms 窗口，但代价量级不同（列引用，不是 stat 整棵树），
  这次没有量过，所以没动。

**复现**：

```bash
cd src-tauri
# 真实仓：worktree 根 + 四个目录，逐目录读取 vs 缓存复用
DAE_GIT_BENCH=K:/code/dae \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan
# 合成大仓（首次会在临时目录里建一次，之后复用）
DAE_GIT_BENCH_FILES=20000 \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan
```

---

## 3. 前端每个 publish 都做一次全量重扫 —— 已实施

**位置（改前）**：

- `src/features/explorer/file-list.tsx:452` —— `useMemo(() => allNames(entries), [entries])`
- `src/features/explorer/use-explorer-selection.ts:83` —— `new Set(allPaths(displayedListing))`

**现象**：两处都是「为了一个低频功能，每个批次重建整张表」：

- type-ahead 的名字表**只**在用户按字母键时用得到，但每个发布快照都重建一次；
- selection prune 只在有选中项时需要，但只要有选中项，每个批次都会重建整条路径集。

而流式列举期间「一个快照 = 一帧」，所以这两个 O(n) 扫描会和批次数相乘。
对照：整条排序 worker 路径的预算是 ~5 ms——**这里白付的是它的三倍**，
而且每个快照还额外留下一个 3.5 万条字符串的数组等着 GC（用户从不按字母键时也一样）。

**改动**（本次）：

1. `allNames` 本身改成**按视图缓存**（`listing-view.ts` 的 `nameLists`），
   并把 `file-list.tsx` 里的 eager `useMemo` 拿掉——表格改由**第一次按键**构建，
   之后的按键直接读缓存。批量重命名对话框那句 `allNames(...)` 也跟着变成零成本。
2. selection prune 改用新的 `listing-view.ts::presentPaths(view, paths)`：
   走到**最后一个被选中的行**就停，不再先造出整条路径数组再建 Set。
   代价于是和被选中行在列表中的位置成正比，而不是和列表长度成正比。
   这段扫描提到 `listing-view.ts` 是为了让它能被 harness 直接量（前端没有测试运行器）。

**改动后的实测**（`bench/frontend-scans/`，同一台机器，多次运行的区间；35,803 行）：

| 测量 | 改前 | 改后 |
|---|---|---|
| type-ahead 名字表，每次 publish | 3.3–3.6 ms | **0**（不再构建） |
| type-ahead 名字表，一次按键 | — | 首次构建 2.6–3.1 ms，同快照后续按键 **0.000 ms** |
| 一次流式列举的累计（两处相加） | **≈16 ms** | **0**（打字时才付一次 ~3 ms） |
| selection prune，选中的行在第一行 | 5.8–6.8 ms | **0.001 ms** |
| selection prune，选中的行在中间 | 5.8–6.8 ms | 2.2 ms |
| selection prune，选中的行在最后 / 已被删 | 5.8–6.8 ms | 4.5–7.8 ms（最差情况不变） |

**还没解决的**：selection prune 的最差情况（选中行靠后，或选中的文件已被删）
仍要读完整条列表，因为它要在列表里确认「确实不在了」。
要再往前一步，需要在视图里同步维护一份 path → index 的索引（那本身也是一次扫描），
除非排序视图能便宜地回答「行集没变」——它现在不能：排序是置换，
`sharedRowCount` 只认「前缀增长」，两个排序视图之间一般返回 `null`。

**验证**：`bun run lint`、`bun run build`（含 `tsc`）均通过；
前端没有测试运行器，所以行为验证靠 harness：它调用的是**出货的** `allNames` /
`presentPaths`（不是复刻），并额外确认缓存命中（warm 为 0）。

**复现**：

```bash
bun bench/frontend-scans/scan-cost.ts
bun bench/frontend-scans/scan-cost.ts --entries 120000
```

---

## 4. 内容搜索：每个文件都新建一个 `Searcher` —— 已实施

**位置（改前）**：`src-tauri/src/file_system/local/content_search.rs:115`。

`builder.build_parallel().run(closure_factory)` 的 factory 是**每线程**调用一次，
但 `SearcherBuilder::new()...build()` 写在了返回的闭包**内部**，于是每个被扫到的文件都
重新分配一套 searcher。而 `Searcher` 持有的全是 buffer（读 buffer、行 buffer、转码
scratch），每个文件的状态要么由每次搜索新建（装行号的 `Core`），要么在入口被 reset
（`LineBufferReader::new` → `LineBuffer::clear`，二进制偏移一并清），所以复用本来就是
这个类型的设计意图。

**改动**（本次）：searcher 提到 factory 里，按线程 `&mut` 复用（`search_path` 要
`&mut self`，闭包是 `FnMut`，其余一行不动）。配置提到 `build_searcher()` 一处定义，
为的是让 harness 量的是**出货的那份配置**而不是复刻。前端无涉。

**实测**（`reports_the_cost_of_a_content_search`；改动前后各重建一次二进制，
所以机器自己的漂移也在数字里，每格是 3 次运行的区间）：

| 树 | 文件数（搜索真正访问的） | 改动前 | 改动后 |
|---|---|---|---|
| 本仓库 `node_modules/`（嵌套、真实内容、K:） | 13,008 | 67.9–73.7 ms | **53.3–53.5 ms（−24%）** |
| `%TEMP%/dae-git-bench-20000`（200 层目录、1 字节文件、C:） | 20,020 | 614–693 ms | **543–569 ms（−15%）** |
| `K:/dae-listing-bench/big`（单层目录、空文件、K:） | 35,803 | 391–707 ms（单次最低 154） | **123–147 ms（单次最低 119）** |
| `K:/dae-listing-bench/huge`（单层、空文件、K:） | 120,000 | 4.44–4.61 s | 4.34–4.38 s（无差别） |

**被删掉的那一笔可以直接量**：harness 里的一行 `build  N x build_searcher`（无任何 I/O，
只构建并丢弃 searcher）是 **0.65 µs/文件**，线性得很准：13,008 → 9 ms，
20,020 → 13 ms，35,803 → 22 ms，120,000 → 78 ms。

几处值得写清楚的：

- `node_modules` 那棵树是唯一真实的（嵌套目录 + 有内容的文件），也是改动后的运行都
  **紧到 ±2 ms** 的：去掉每文件一次分配后，总耗时也不再抖。它测到的 24% 略大于纯
  `build` 的 13%，多出来的部分是新分配出来的 buffer 在读取时还要付「第一次碰到」
  （缺页 / 缓存冷）再释放——复用的那份是热的。
- 120,000 那个单层目录里**测不到差别**（两种写法都是 ~4.4 s）。那不是这个改动的错，
  而是那棵树每文件 ~36 µs 的成本几乎全在 walker/OS 上（同一棵树 35.8k 时只有 ~11 µs/文件，
  3.35 倍的文件量付了 30 倍时间；两种写法都一样）。也就是说：**这个改动值多少，取决于
  那棵树每文件的固定成本里有多少真是 searcher**。
- 这个 harness 的收数比 §1/§2 的噪（Windows 上打开一个文件的成本逐次被拉伸），
  所以上面每一格都给了区间，结论按「形状」读：0.65 µs/文件的分配消失了，在打开文件
  便宜、文件又多的树上（`big`、`node_modules`）看得见，在 walker/OS 主导的树上看不见。
- `files` 那一列是搜索真正访问的文件数，不是目录里有几个：`node_modules` 在 §1 里数到
  的 72,106 个文件里只有 13,008 个进搜索——ripgrep 的 standard filters 会跳过隐藏项与
  被忽略项，这是内容搜索一贯的行为。

**测试**：新增 2 个。一个把**同一份 searcher** 依次用到两个文件上（长的在前，正是线
程会走的顺序）并断言行号各自从 1 起、且回头再搜第一个文件仍然对——行号是复用最先
会露出来的那处状态；另一个走出货入口（`search_file_contents_sync`）一次扫两个都命中
的文件，钉住整条 walk 下的同一件事。原有 4 个内容搜索测试不变。

**复现**：

```bash
cd src-tauri
# 真实内容：本仓库的依赖树（查询用一个不会命中的词，否则 100 个命中就够截断）
DAE_CONTENT_SEARCH_BENCH=K:/code/dae/node_modules DAE_CONTENT_SEARCH_BENCH_QUERY=zzqqxx_no_such_token \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_content_search
# 或者任意一棵大树（空文件的最能放大每文件固定成本）
DAE_CONTENT_SEARCH_BENCH=K:/dae-listing-bench/big \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_content_search
```

---

## 5. 零散（都是小钱，记着就行）

- `file_system/preview.rs`（响应构造处）`body(Cow::Owned(rendered.bytes.clone()))`：
  每个响应克隆整张图/图标字节，可以不克隆。
- `file-list.tsx` 的拖拽在 `pointermove` 上走 `resolveDragTarget` → `directoryPathSet`，
  而该缓存的键是 view，view 每个批次都换——流式期间拖拽可能每帧全扫一次目录。

---

## 6. 复现命令汇总

```bash
# 名称搜索：中位 ms + 结果来源分布（§1）
cd src-tauri
DAE_SEARCH_BENCH=K:/code/dae DAE_SEARCH_BENCH_QUERY=readme \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_name_search

# Git 徽标：逐目录扫描 vs 缓存复用（§2）
DAE_GIT_BENCH=K:/code/dae \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan
DAE_GIT_BENCH_FILES=20000 \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan

# 内容搜索：整棵树的中位 ms + 每文件固定成本（§4）
DAE_CONTENT_SEARCH_BENCH=K:/code/dae/node_modules DAE_CONTENT_SEARCH_BENCH_QUERY=zzqqxx_no_such_token \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_content_search

# 前端全量扫描（§3）
bun bench/frontend-scans/scan-cost.ts
bun bench/frontend-scans/scan-cost.ts --entries 120000
```

## 7. 什么条件下重新评估

- **第 1 条**：如果产品决定要「在依赖目录里也能搜」，那就不是忽略规则的问题，而是
  「两段式搜索」（先项目内、再按需扩到忽略目录）的设计问题。
- **第 2 条**：如果 `directory_untracked` 契约被简化（例如徽标改成按 status 的
  `WT_NEW` 折叠语义重写），pathspec 那条路就重新打开。另外，如果「首次进入某目录
  必须看到刚刚在别处发生、而没有任何 watcher 看到的改动」变成硬要求，那 5 秒复用
  窗口要重新谈——它现在就是拿这点新鲜度换来的。
- **第 3 条**：如果 type-ahead 被换成「跳转列表 / 命令面板」这类一次性扫描，
  「按视图缓存」这件事要重新设计（一次性扫描本来也不该缓存）。selection prune 的最差情况
  （选中的行靠后，或已被删）只有在视图能便宜地回答「行集没变」时才会消失——
  现在不能，因为排序是置换，`sharedRowCount` 只认前缀增长。
- **第 4 条**：如果内容搜索改成常驻的索引/常驻 worker，这个分配问题自然消失（那份
  常驻 worker 本来就会持有一个 searcher）。另外，如果以后有人换了更快的磁盘或关掉实时
  防护，`big` 那种单层空文件树的数字会变，但 `build  N x build_searcher` 那一行只跟树的
  大小有关，可以当基准继续用。
