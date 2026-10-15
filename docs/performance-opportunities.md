# 性能机会清单

> **状态：清单**。第 1 条（名称搜索的忽略规则）、第 2 条（Git 徽标的全工作区扫描）、
> 第 3 条（前端每 publish 的全量重扫）、第 4 条（内容搜索的每文件 `Searcher`）
> 与第 5 条（零散）**都已处理**，数字见各自那一节。第 2 条里「与徽标共用 400 ms 窗口」的
> 分支查询本次也量了（并且修掉了量出来的 13 倍），第 5 条里原写着「预览响应复制字节
> 是一行的事」——**这句是错的**，5.1 记下了原因和真正能省的那一次。
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

> 一处例外：Windows 枚举。它曾经被量过、做过，然后整批退出工作区；**本次已恢复实施**
> （`local/windows_directory.rs` + `bench/listing-pipeline/` +
> `docs/decisions/windows-directory-enumeration.md`）。`std::fs::read_dir` 在 Windows 上已经用
> `FindFirstFileExW`，只是**故意不传** `FIND_FIRST_EX_LARGE_FETCH`
> （`library/std/src/sys/fs/windows.rs` 的注释明说「保守」）。那批实测：
>
> | | 35,803 条 | 120,000 条 |
> |---|---|---|
> | walk（只枚举） | 14.15 → **9.76 ms（−31%）** | 55.78 → **40.32 ms（−28%）** |
> | 整条流水线（`std+key` → `win32+cmp`） | 17.99 → **12.64 ms（−30%）** | 69.65 → **50.71 ms（−27%）** |
> | 首批 512 条 | 0.135 → 0.106 ms（**只省 29 µs**） | — |
>
> 也就是说它买的是「列表读完」，不是「第一屏」；那批还顺手把排序从 `name_sort_key`
> （每行 2 次分配）换成 `name_order` 比较，再省 2 个点。恢复后复跑对得上当时的中位：
> 镜像 harness `std+key` 18.83 ms / `win32+cmp` **13.08 ms**，出货函数那一侧 35,803 条
> **14.13 ms**（当初 13.75 / 14.87）。绝对值随卷的热度整体漂移，比值稳定。
>
> 这里原先写着「当前代码是 `std::fs::read_dir` + 逐条 `metadata()`」，暗示每条一个系统调用，
> 是错的：Windows 上 `DirEntry::metadata()` 直接取枚举已经带回来的 `WIN32_FIND_DATA`。
> 本树里 `reports_where_a_large_listing_spends_its_time` 的 `std walk` / `std rows` 两行
> 量到 **0.16–0.19 µs/条**（35,803 条：13.5 → 20.2 ms；120,000 条：55.2 → 74.9 ms）——
> 两个数量级低于一次系统调用，也正是 `PARALLEL_ENTRY_THRESHOLD` 在 Windows 上是 `None`
> 的原因（Unix 上同一行就是 `lstat`，阈值 `Some(128)` 就是为它设的）。所以「列举到顶了」
> 在 Windows 上还差的那一块不是元数据，而是枚举本身的系统调用次数 —— 就是上面那一列。

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
4. 顺带量了与它共用窗口的分支查询（下面那节），并修掉量出来的一处 13 倍浪费。

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
  **本次量过了**，并且修掉了量出来的那个大头：见下。

**分支查询（本次量过，并修掉了一处 13 倍的浪费）**：它和徽标共用同一个 400 ms 窗口
（`git-refresh.ts` 一次事件同时失效两个 key），也就是说每个窗口它至少跑一次。

1. **每个本地分支一次 `git_branch.is_head()` 是错的**：libgit2 里它就是
   `git_repository_head()`，也就是每个分支重新读一遍 `.git/HEAD`、再查一次引用
   （`libgit2/src/libgit2/branch.c` 的 `git_branch_is_head`）。合成的 1,000 本地分支
   仓里，单这一句就让整个调用花 **83.1–83.6 ms**，而同一个仓里其它部分加起来 ~4 ms。
   改成「HEAD 的符号目标只读一次，分支的完整引用名等于它就是当前分支」——
   分离 HEAD 没有符号目标，于是没有分支算当前分支（新增
   `a_detached_head_leaves_no_branch_current` 钉住这一条）——之后是 **5.6–6.2 ms**。

2. 剩下的账（`reports_the_cost_of_a_branch_listing`，同一台机器）：

   | 形状 | 调用总时间 | open | 列引用 | ahead/behind | 同仓的徽标扫描 |
   |---|---|---|---|---|---|
   | `K:/code/dae`（478 tracked，2 本地 / 15 远程） | 2.47–2.58 ms | 0.84–0.89 | 0.84–0.89 | 0.26 | 4.91–5.34 ms |
   | 合成 20,000 tracked（1 个本地分支） | 2.09–2.30 ms | 1.43–1.56 | 0.28–0.34 | 0.18–0.21 | 32.4–32.7 ms |
   | 合成 1,000 分支 + 4,000 tag（全在 packed-refs） | 5.6–6.2 ms（改前 **83.1–83.6**） | 1.44–1.48 | 2.51–2.54 | 0.19–0.24 | 2.24–2.39 ms |
   | 合成 10,000 分支 + 40,000 tag | 57.57 ms | 1.44 | 37.53 | 0.20 | 2.22 ms |

   结论分两句：分支查询**不随 worktree 大小走**（20k tracked 仍是 2.1 ms），
   但**随 ref 总数走**——libgit2 的分支迭代器要先遍历**所有** refs 再按前缀过滤，
   而 `branches()` 建了两个迭代器（本地一个、远程一个），也就是整个 refdb 走两遍：
   每遍 ~0.37 µs/ref，两遍合计 ~0.75 µs/ref。所以它不需要徽标那种缓存：常见仓
   2–3 ms，比已经缓存过的徽标读取（~1.7 ms）略贵一点，远低于徽标扫描本身；只有
   分支/tag 上万的仓才会到几十毫秒。那是下一刀（两次遍历合成一次、按前缀分类），
   本次没做。`git-status.tsx` 里那句「单次代价也是同量级的 git2 全仓库遍历」已按实测改写。

**复现**：

```bash
cd src-tauri
# 真实仓：worktree 根 + 四个目录，逐目录读取 vs 缓存复用
DAE_GIT_BENCH=K:/code/dae \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan
# 合成大仓（首次会在临时目录里建一次，之后复用）
DAE_GIT_BENCH_FILES=20000 \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_git_status_scan
# 分支查询：同一台机器上的三种形状（真实仓 / 合成 tracked / 合成 ref-heavy）
DAE_GIT_BENCH=K:/code/dae \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_branch_listing
DAE_GIT_BENCH_FILES=20000 \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_branch_listing
DAE_GIT_BENCH_REFS=1000 \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_branch_listing
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

## 5. 零散（都是小钱，记着就行）—— 已处理

### 5.1 预览/缩略图响应里的字节拷贝 —— 不是一行，但其中一次可以省

**位置**：`src-tauri/src/file_system/preview.rs:536`（`render_response` 里
`body(Cow::Owned(rendered.bytes.clone()))`）和 `render_loop` 的等待者扇出
（同文件 `:489`）。

**原判断**（以及为什么它是错的）**：这里原先写着「每个响应克隆整张图/图标字节，
可以不克隆」。字节不是本地 `Vec`，而是 `Arc<RenderedThumbnail>`——缩略图/图标缓存共享的
就是这一份；而 tauri 的 `UriSchemeResponder::respond` 只收
`http::Response<Cow<'static, [u8]>>`。`Cow` 要 `'static`，借不到 `Arc` 里的字节，
只能给出 `Owned(Vec)`。所以把 `.clone()` 删掉是**编译不过**的：

```
error[E0507]: cannot move out of an `Arc`
   --> src\file_system\preview.rs:536:30
    |
536 |             .body(Cow::Owned(rendered.bytes))
    |                              ^^^^^^^^^^^^^^ move occurs because value has type `Vec<u8>`, which does not implement the `Copy` trait
```

`Arc` 是缓存的既成事实（`lookup_cache` / `store_cache` 各持一个 handle，引用计数不会
降到 1），所以「每个响应一份自有字节」是 `Cow` + 缓存的**结构下限**，不是这次的浪费。

**真正能省的那一次**：`render_loop` 把构造好的响应 `clone()` 给**每一个**等待者，
包括最后那一个——于是每个请求付 1 次构造拷贝 + N 次扇出拷贝。改成最后一个等待者
直接拿走响应本体，就成了 N 次，正好是下限（常见情形是同一 URL 没有并发重复请求，
即 N = 1，于是 2 次拷贝 → 1 次）。

**实测**：这里没有数字，因为它不是可观测的对比：省下的是一次 memcpy + 一次分配，
一张 256 px 缩略图几十 KB、一个图标几 KB，远在 `bench/frontend-scans/` 那类 harness 的
噪声之下。它买的是「减半」，不是「变快」。

**验证**：`cargo check --lib` 通过（对比：去掉 `.clone()` 会 E0507）；
`cargo test --lib render_tests` 18 个全过，含等待者去重与「每个等待者恰好被回应一次」。

### 5.2 拖拽的目录集 —— 已经不用改

**位置**：`src/features/explorer/file-list.tsx:610`（`pointermove` → `resolveDragTarget`
→ `draggableDirectoryPaths`）与 `src/features/explorer/listing-view.ts::directoryPathSet`。

**结论**：`directoryPathSet` 本来就是**按视图缓存**的
（`directoryPathCache: WeakMap<ListingView, Set<string>>`，注释里写明了它就是为
`pointermove` 准备的），所以一次拖拽里的反复移动只扫一次；`pointerup` 那两处
（加入收藏 / 加入空间）吃的是同一个缓存。原先这一条想修的就是缓存本身，而它已经在。

剩下的是缓存躲不掉的那一种：**流式期间每个批次都是一个新视图**（一个快照 = 一帧），
于是拖拽会每帧扫一次。实测（`bench/frontend-scans/`，同一台机器；35,803 行、
每 8 行一个目录 ⇒ 4,476 个目录）：

| 测量 | 结果 |
|---|---|
| 冷扫一个快照（35,803 行） | 1.00–1.09 ms |
| 同一视图再问（缓存命中） | **0.000 ms** |
| 一次流式列举的累计（8 个 publish） | **3.05 ms** |
| 120,000 行（同样每 8 行一个目录） | 冷 2.98 ms / 累计 4.71 ms |

也就是说：**只有拖拽和流式重叠时**才付每帧 ~1 ms、整段 ~3 ms。按本节的口径这仍是
小钱，所以留在这里不改。真要抹掉，做法是在 `pointerdown` 时把这个手势的目录集冻结
（`sourcePaths` 本来就是整个手势固定的），代价是拖拽期间流进来的新目录不参与
「能不能放到收藏/空间」的判断——那点新鲜度就是它换来的东西，和 §2 的五秒窗口同一笔账。

**复现**：`bun bench/frontend-scans/scan-cost.ts`（harness 调的是**出货的**
`directoryPathSet`，warm 那行同时确认缓存命中）。

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

# 分支查询（§2）：真实仓 / 合成 tracked / 合成 ref-heavy（分支 + tag，packed）
DAE_GIT_BENCH=K:/code/dae \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_branch_listing
DAE_GIT_BENCH_FILES=20000 \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_branch_listing
DAE_GIT_BENCH_REFS=1000 \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_branch_listing

# 内容搜索：整棵树的中位 ms + 每文件固定成本（§4）
DAE_CONTENT_SEARCH_BENCH=K:/code/dae/node_modules DAE_CONTENT_SEARCH_BENCH_QUERY=zzqqxx_no_such_token \
  cargo test --lib --release -- --ignored --nocapture reports_the_cost_of_a_content_search

# 列举：std walk / std rows / streamed（出货形状）/ listing 四行（§0 的例外）
# 注意这是 cargo test 二进制，没有 mimalloc：会分配的两行比出货二进制偏大
DAE_LISTING_BENCH=K:/dae-listing-bench/big \
  cargo test --lib --release -- --ignored --nocapture reports_where_a_large_listing_spends_its_time

# 前端全量扫描（§3、§5.2 的拖拽目录集）
bun bench/frontend-scans/scan-cost.ts
bun bench/frontend-scans/scan-cost.ts --entries 120000

# 预览响应：编译期就证「构造处那次拷贝挪不掉」（§5.1）
cd src-tauri && cargo check --lib && cargo test --lib render_tests
```

## 7. 什么条件下重新评估

- **第 1 条**：如果产品决定要「在依赖目录里也能搜」，那就不是忽略规则的问题，而是
  「两段式搜索」（先项目内、再按需扩到忽略目录）的设计问题。
- **第 2 条**：如果 `directory_untracked` 契约被简化（例如徽标改成按 status 的
  `WT_NEW` 折叠语义重写），pathspec 那条路就重新打开。另外，如果「首次进入某目录
  必须看到刚刚在别处发生、而没有任何 watcher 看到的改动」变成硬要求，那 5 秒复用
  窗口要重新谈——它现在就是拿这点新鲜度换来的。分支查询那第二条（整个 refdb 走两遍）
  只在真的遇到 ref 上万的仓时才值得再动：那时把它合成一次遍历（按前缀分类）就能
  减半，`DAE_GIT_BENCH_REFS` 那两行是复测的数据点。
- **第 3 条**：如果 type-ahead 被换成「跳转列表 / 命令面板」这类一次性扫描，
  「按视图缓存」这件事要重新设计（一次性扫描本来也不该缓存）。selection prune 的最差情况
  （选中的行靠后，或已被删）只有在视图能便宜地回答「行集没变」时才会消失——
  现在不能，因为排序是置换，`sharedRowCount` 只认前缀增长。
- **第 4 条**：如果内容搜索改成常驻的索引/常驻 worker，这个分配问题自然消失（那份
  常驻 worker 本来就会持有一个 searcher）。另外，如果以后有人换了更快的磁盘或关掉实时
  防护，`big` 那种单层空文件树的数字会变，但 `build  N x build_searcher` 那一行只跟树的
  大小有关，可以当基准继续用。
- **第 5 条**：5.1 里那次「每响应一份拷贝」只有在 tauri 的 responder 收得下可共享的
  body（即不用 `Cow<'static, [u8]>`，而是一个能共享所有权的 `'static` 体）时才会彻底消失，
  在那之前 N 次就是下限。5.2 只有在「拖拽与流式重叠」真的出现在 profile 里、
  开始和指针移动的可感知延迟抢帧时才值得改（冻结手势的目录集），否则就保持现状。
- **§0 的例外（Windows 枚举）已实施**（`windows_directory.rs` + `bench/listing-pipeline/`），
  重开条件写在 `docs/decisions/windows-directory-enumeration.md` §8：`std` 自己传上这个 flag、
  Windows 之外出现真正的批量读接口、或者列举不再在关键路径上。复测按那份文档 §7 的两条
  harness 跑，记住 `cargo test --lib` 没有 mimalloc、绝对值比镜像 harness 大 ~30%。
