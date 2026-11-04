# 列表外壳的剩余开销：审计，以及完成路径的实施

> **状态：审计完成；第 1 项（§2、§3）、§4 的 watcher 补丁、§5 的批量选择/前缀
> 与 §6 的选区表示都已实施；§9 的 app 内 qmlprofiler 复测（2026-11-03）之后，
> §10 又落地了四刀（可见视图按需、完成排序与交互式排序移出 GUI 线程、终端输出按帧合并），§7 的次级清单
> 只记录、未动。**
> 枚举与 IO 不在本文范围之内——它们已经有实测与结论
> （[`windows-directory-enumeration.md`](windows-directory-enumeration.md)、
> [`completion-io.md`](completion-io.md)）。本文问的是另一个问题：
> **目录读出来之后，从批次进模型到行画出来，时间花在哪。**
>
> 数字来源分四类，正文逐处标注：
> **仓库 bench**（`bench/listing-pipeline/`、`docs/decisions/engine-qt-quick.md` §5.1）、
> **单元测量**（`model::tests::reports_where_the_completion_sort_spends_its_time`，
> `--release`）、**转写基准**（审计阶段用比较器转写成的独立程序，§2.2 末尾说明了
> 它与单元测量的偏差），**结构**（只由代码推出，未计时）。app 内整段已用
> qmlprofiler 复测（§9）。
>
> §1–§6 里的行号是**审计当时**（§10 实施前）的位置；那次改动让 `model.rs` 整体
> 位移，按行号找不到符号时就按相邻的函数名读。§7 的表格改用符号名。

## 0. 结论先行

枚举已经是这个仓库量化最扎实的一段：35,803 条目全流水线 **12.64 ms**、
120,000 条目 **50.71 ms**（仓库 bench），没有可捡的便宜。剩下的开销在 shell 侧，
按预期收益排：

| 优先级 | 位置 | 机制 | 状态 |
|---|---|---|---|
| 1 | `model.rs` 完成时排序 | GUI 线程全量重排，比较器每次比较都分配；12 万条 147.5 ms → 34.0 ms（单元测量；app 内 §9 复测中位 66.6 ms） | **已实施**（§2；§10.2 大目录移到 worker） |
| 2 | `model.rs` 的 `index_of_path` | 每次查找为每行 `format!` 一个完整路径 | **已实施**（§3） |
| 3 | `model.rs` 的 watcher | 事件带着精确子项名，被丢弃后整目录重读 | **已实施**（§4） |
| 4 | `FileList.qml` / `FileGrid.qml` | 全选、前缀、Shift 范围、框选逐行跨 FFI 调用 | **已实施**（§5） |
| 5 | `Tab.qml` 选区 | JS map 全量复制、状态栏全量遍历、delegate 每次变更跑 JS | **已实施**（§6） |
| 6 | `model.rs` 的 `set_order` | 点列头在 GUI 线程重排；12 万条 60–100 ms | **已实施**（§10.4） |
| 7 | 次级清单 | 图标同步加载、git 徽标线性回退、预览随选区抖动、Overview 同步 IO… | 未做（§7） |

§9 的复测与 §10 的三刀之后，完成路径的结论要连着读：第 1 项的同步排序现在只用于
≤2048 行和小目录；大目录在 worker 上排序，重置只重建可见视图的 delegate。

## 1. 前提：一次列表的完成路径

`open_streamed_listing` 先回 512 条头，其余按批次（翻倍到 8192）在 reader/emitter
两个线程上流出（`crates/dae-core/src/fs/listing.rs:34-39`），每批由
`DirectoryListingCursor::take_rows` 用 `name_order` 排一次
（`crates/dae-core/src/fs/local/directory.rs:151-155`），经 `QtBatchTarget` 一批一个
Qt 事件进模型，模型用 `begin_insert_rows` 增量追加。最后一批带 `done`，模型在
`append_rows` 里调 `sort_at_completion`。

问题就在这里：**增量插入的结果在完成时被整段丢弃。**

## 2. 完成时重排：为什么必然发生，代价多少（已实施）

### 2.1 触发条件

`sort_at_completion` 只在 `rows_are_sorted` 失败时动作，而两个
来源让它在现实中几乎必然失败：

1. **批次拼接无序。** 每批各自有序，批次之间按枚举顺序拼接；超过 512 条的目录
   就是 k 段有序串接。`rows_are_sorted` 一遇到第一处逆序就返回，所以这个判断本身
   很便宜。（只属于本地流式读；远端后端整目录一次交付，见 §9 的“适用面”。）
2. **后端顺序 ≠ 视图顺序。** 后端按「dir / symlink / file / other 分档 + 小写字典序」
   （`name_order`），视图按 `folders_first` + `natural_cmp`（数字感知、大小写折叠）。
   即使只有一个批次，`IMG_10` 与 `IMG_2`、symlink 与 file 的交错都会让两边不同。
   默认排序就是 `name` 升序 + 文件夹优先（`ViewOrder::default`、`prefs.sort_key`
   默认 `"name"`），所以这正是最常见的路径。

失败后走 `resort_rows`：`about_to_reload` → `beginResetModel`
→ `sort_by` → `endResetModel`。视图因此丢掉全部 delegate 并靠
`indexOfPath` + `positionViewAtIndex` 恢复滚动（`FileList.qml` 的 `restoreScroll`）。

### 2.2 代价

比较器（`compare_rows`/`sort_rows` 与 `natural_cmp`）
有三处重复劳动：

- `natural_cmp` 每次都走 `chars().flat_map(char::to_lowercase)`，没有 ASCII 快路径；
- 数字段用 `digit_run` 拼成 `String` 再比较——**每次比较、每侧各一次分配**；
- `sort_by` 是稳定排序，另要一份 n/2 的临时缓冲区。

排序成本用模型自己的单测量（`model::tests::reports_where_the_completion_sort_spends_its_time`，
`--release`，5 次取平均，语料 `IMG_<n%997><n>.png`，同一进程内同时跑旧实现
（测试里的逐字拷贝）和新实现）：

| | 35,803 条 | 120,000 条 |
|---|---:|---:|
| 实施前（分配式 `natural_cmp`，`sort_by`） | 29.4 ms | 147.5 ms |
| 实施后（折叠游标，`sort_by`） | **6.6 ms** | **34.0 ms** |
| | **4.5×** | **4.3×** |

这一段正好落在用户看到列表之后，所以体感是“打开后卡一下”。（表里的 34.0 ms
是单测里 `Vec<String>` 的比较器成本；app 排序的对象是 `StoredRow`，§9 的
复测中位 66.6 ms。4.5× / 4.3× 是同一进程内两种比较器的比值，不受影响。）
`rows_are_sorted`
用的是同一个比较器，顺序恰好一致时的那一遍守卫也按同一比例下降（单遍
n 次比较，**结构推断**，未单独计时）。

（审计阶段还用过一份独立转写程序做数量级判断，它报出 128 / 536 ms，比这里高
一个量级——转写语料更长、且当时机器上有并行的编译与代理任务。以本节这组同进程、
同语料、release 的数字为准。）

### 2.3 实施内容

1. `natural_cmp` 改成无分配实现：ASCII 字符直接单字符折叠（`to_ascii_lowercase`，
   与 `char::to_lowercase` 对 ASCII 等价），数字段用切片比较（去前导零、先比长度
   再比字典序），非 ASCII 走一个按 `char::to_lowercase` 展开的游标，展开缓冲固定
   3 个 `char`（Unicode 的 SpecialCasing 里多字符小写只有 U+0130 一条，长度为 2，
   测试里对全部 `char` 断言了上界）。
2. 数字段的比较沿用原语义：去前导零后长度优先，再逐位；数值相等时前导零少的在前
   （`file01` > `file1`），完全相等则继续向后比较。
3. 排序保持 `sort_by`（稳定）。原因：`natural_cmp` 只折叠大小写，`README` 与 `readme`
   会判等（二次比较也是同一个折叠比较器），换 `sort_unstable_by`
   会让这类并列行的顺序在每次重排后不确定——这属于可观察的变化，不值得为一份
   n/2 的临时缓冲去换。
4. 语义等价性用测试钉住：`tests` 里保留旧实现的一份逐字拷贝作参照实现，对
   大小写、数字段、前导零、CJK、重音、`İ`、`ẞ`、emoji 混合语料逐个断言
   `natural_cmp(a, b) == reference(a, b)`，并对整份语料的排序结果断言两套排序一致。

### 2.4 为什么不是“移出 GUI 线程 + `layoutChanged`”

原计划还有两刀，实施时按下面的理由停在这里，记录在案：

- **移出 GUI 线程**：排序需要 `Vec<StoredRow>` 的所有权。把它移交给 worker，模型在
  排序期间就没有行，`data()` 会返回空——列表在 66 ms 内可能被绘制成空表。要避免
  就得先克隆全部行（12 万条约 10 ms 的分配），或另建一套索引映射，两者都不比
  66 ms 的原地排序便宜。比较器修完之后，最坏情况（12 万条、名字带数字）单测是
  34.0 ms、app 内 §9 复测中位 66.6 ms（60 Hz 下约四帧），3.5 万条是 6.6 ms
  （一帧以内）。
- **`layoutChanged` 代替 reset**：需要在 `layoutAboutToBeChanged` 之后为所有持久索引
  逐个 `changePersistentIndexList`（cxx-qt 0.10 没有现成绑定，要自己声明
  `#[inherit]` 并用排列表重映射），才能保证 QML 的 delegate 不指错行。它省下的是
  236 个 delegate 的重建，原按对象创建速率推断为 3–5 ms（**结构估计**）；§9 复测
  reset 之后的 QML 顶层工作中位 87.8 ms（其中 `Creating` 28–40 ms），与排序同
  量级，不是“小一个量级”。代价仍是把“重排后行仍指对”这个正确性交给一条没有测试
  覆盖的路径。等 §2.2 的排序成本真的回到帧预算以内（或 delegate 重建被量出来
  超过 8 ms），再回来做这一刀。§9 的复测满足了后一个条件；但 §10 先做了
  “只有可见视图重建 + 排序移出线程”，单轮 GUI 阻塞已经 ≤22 ms，这一刀的收益
  不再明显，留在 §7。

## 3. `index_of_path`：不再为每一行拼路径（已实施）

`index_of_path` 以前用 `row.full_path(&self.base) == path` 线性扫描，
`full_path` 对基准目录下的行（正常情况）是 `format!("{base}{name}")`：
12 万条目录一次查找就是 12 万次分配。调用点恰好都在“一次交互的收尾”上——
`FileList.qml` 的滚动恢复、Shift 点击的锚点、重命名/新建后的定位。

改为借用比较：`path.strip_prefix(&base) == Some(row.name.as_str())`；只有越界行
（`path: Some(full)`，本来就是完整拼写）才比较已存的值。分配从 O(行数) 降到
每次查找一次（`QString` → `String`）。

## 4. watcher 变更：补丁而不是重读（已实施）

### 4.1 原来的形状

`changed_children` 已经把事件折算成“具体哪些子项变了”
（`crates/dae-core/src/fs/local/directory.rs:39-56`），事件里也带着
`DirectoryChanged { names }`，但 `ListingHost::emit` 只取了 `path`，
`reload_after_change` → `load()` 重走一遍
`open_streamed_listing`。一个文件改名，150 ms 防抖后重读 12 万条目录：一次枚举
（仓库 bench：≈55 ms）+ 行构建 + §2 的重排 + 整表 diff。构建/下载目录里的事件流
会反复触发。

补丁所需的零件本来就在：`stat_named_children`（`directory.rs:363`，测试
`a_patched_entry_is_spelled_exactly_like_the_listed_one`）按名字 stat，拼写与列表行
完全一致；`service::read_directory_changes`（`service.rs:69-86`）已经把它放到
`spawn_blocking` 上并明确拒绝非本地路径。缺的只是消费端。

### 4.2 现在的形状

- **名字穿过防抖。** `Change { path, names }`：`names` 为 `None`
  表示这一批里有一个事件说不清是哪个子项（core 的空列表语义），只能重读；同一批
  其余事件的名字并进去并去重（`Change::merge`）。跨导航的两批以最新路径为准。
  单个事件自己也去重——一次改名会同时报旧名和新名，而补丁是按名字规划行编辑的。
- **能补就补。** `can_patch`：本地、`finished`、没有在飞的读、
  没失败过、名字数 ≤ `PATCH_LIMIT`（64）、排序键是 `name`、过滤器是默认值。
  最后两条是正确性条件而不是性能条件——别的排序键下元数据变化会让行换位置；
  非默认过滤器在插入时就把行丢掉了，后补回来的行无法恢复。
- **stat 在阻塞池上。** `patch_rows` 把名字交给
  `read_directory_changes`，结果带着 generation 回到 Qt 线程。落点时
  （`apply_patch`）再验一遍：closed、generation、是否又开始了读、
  排序键或过滤器是否变了；任何一项不成立都退回重读。
- **落成最小的模型信号。** `plan_patch`（纯函数）把
  `(names, entries)` 解成 removals / replacements / insertions：不存在的行删除、
  元数据变了的行替换、没见过的子项按 `compare_rows` 的位置插入。两个例外退回重读，
  而不是猜：子项**换了类型**（文件↔目录，`folders_first` 的分组位置会变）、名字
  只差大小写（大小写不敏感文件系统上 watcher 的拼写可能和列表不同）。
  `apply_row_patch` 每个删除/插入各发一次 `begin/end_*Rows`，
  连续的被替换行合并成一次 `dataChanged`；**不发 reset**，滚动位置和 delegate 都
  不动。行数变化后更新 `loaded` 与状态栏文案，并触发一次 status-only 的 Git 读
  （`begin_git_load(false)`），让被改的文件的徽标跟着变。
- **可观测。** `dae::listing` 在 debug 级别记“补丁 N 个子项 / 结果”，或者
  “为什么退回重读”；`DAE_LOG=debug` 就能看到。

### 4.3 验证

- 单测：`plan_patch` 的替换/删除/插入、类型变化退回、仅大小写不同退回、删除后的
  下标换算（`model.rs` 的 `a_patch_*`、`a_child_that_changed_kind_needs_a_read`、
  `a_spelling_that_differs_only_by_case_needs_a_read`、
  `replacements_keep_their_index_past_the_removals_before_them`）。
- app 内冒烟：debug 构建，`DAE_LOG=debug`，打开 `K:\dae-patch-smoke`（`folder1`、
  `a.txt`、`b.txt`、`c.txt`），从外部创建 `d-new.txt`、把 `b.txt` 改名
  `b-renamed.txt`、删除 `c.txt`，再改 `a.txt`：

  ```text
  [DEBUG][dae::listing] watcher: patching 4 child(ren) of K:\dae-patch-smoke
  [DEBUG][dae::listing] watcher: 4 rows now (2 removed, 0 replaced, 2 inserted)
  [DEBUG][dae::listing] watcher: patching 1 child(ren) of K:\dae-patch-smoke
  [DEBUG][dae::listing] watcher: 4 rows now (0 removed, 1 replaced, 0 inserted)
  ```

  两次都没有出现 “re-reading”；前后截图里列表从 `a.txt / b.txt / c.txt` 变成
  `a.txt / b-renamed.txt / d-new.txt`，滚动位置与选中未动。
- 没有测时间：省掉的是一次整目录枚举（12 万条 ≈55 ms，仓库 bench）加 §2 的重排，
  这是结构上的结论，不是这一轮量出来的——§9 量的是“打开 12 万条目录”，不是
  watcher 刷新；后者的整段仍未单独计时，延后条件记在 §9.1。

### 4.4 仍然重读的情况

名字超过 64 个的批量变更、一批里有说不清子项的事件、排序键不是 `name`、
用了非默认过滤器、上一次读失败、远端后端（它们的 watcher 只说“变了”，core 也
明确拒绝非本地路径）。这些路径与改动前完全一样。

## 5. QML 的 O(n) 跨 FFI（已实施）

### 5.1 原来的形状

四个入口形状相同，都是逐行调 invokable，而每次调用在 Rust 侧是
`format!`（完整路径）+ `QString`，在 QML 侧是一次字符串分配：

| 入口 | 位置 |
|---|---|
| 全选 | `qml/FileList.qml:344-355` |
| Shift 范围 | `qml/FileList.qml:304-315` |
| 列表框选 | `qml/FileList.qml:1329-1357`（pointer move 与 16 ms 定时器都调） |
| 网格框选 | `qml/FileGrid.qml:162-193`（按矩形行列遍历，未 clamp 到 `loaded`） |
| 前缀输入 | `qml/FileList.qml:238-268`、`qml/ColumnView.qml:283-291` |

前缀输入是最容易踩到的：未命中时扫满 `loaded` 行，每行一次 invokable + 一次
`toLowerCase()` 分配。10 万条的目录里敲一个不匹配的字母就是 10 万次边界穿越。

### 5.2 现在的形状

- **模型侧一次给全。** 这一轮先加了 `pathsForRows(first, last) -> QStringList`
  （区间先裁到实际行数，再在 Rust 里拼好），边界只穿一次、没有每行的 invokable 与
  QML 侧字符串。§6 把选区搬进模型之后，调用方连路径都不再需要：
  这一组入口换成了 `selectRange` / `selectAll` / `marqueeCells`，
  `pathsForRows` 与 QML 侧的路径数组一并删除。它留下的 `clipped_rows`
  仍在 `selection.rs` 里，是 `selectRange` 的裁剪逻辑。
- **前缀在 Rust 里扫。** `indexOfNamePrefix(needle, start) -> i32`：needle 由 QML 折叠一次
  （`toLowerCase()`，与原来的比较语义相同），Rust 用折叠游标逐字符比较行名（`folded_starts_with`），从 `start` 绕一圈。
  没做二分：行序由用户选的键决定，名字在 `size`/`modified` 排序下不是有序的，
  而线性扫描已经是零分配、零穿越。`İstanbul` 与 JS 一样折叠成 `i` + U+0307，
  所以 `"i"` 命中、`"istan"` 不命中（单测钉住了这条）。
- **调用点。** 全选、Shift 范围（列表、网格、分栏）都改成一次调用；列表与网格的
  框选先算行/格区间（网格把它抽成了 `marqueeRange`），区间没变直接返回——一次拖动
  里的 pointer move 远多于行区间的变化。§6 之后这些调用全部落在选区的三个入口上
  （`selectRange`、`selectAll`、`marqueeCells`），见 §6.2。
- **清掉死代码。** `DirectoryListModel.nameAt` 与 `FileList`/`ColumnView` 里两个
  `nameAt` 包装不再有人调用，已删除。

### 5.3 验证

- 单测：`find_name_prefix`（起点、回绕、未命中、空表、`İ` 的折叠语义）——
  `model::tests::type_ahead_finds_*`、`a_prefix_match_ignores_case_*`；区间裁剪随
  §6 搬进 `selection::clipped_rows`，由 `selection::tests::a_range_is_clipped_*`
  钉住。
- app 内冒烟（debug 构建，按窗口标题找句柄，用物理坐标驱动；150% 缩放）：
  36 条目的目录 `Ctrl+A` → 状态栏 `36 items · 36 selected`；输入 `m` → 选中
  `mango.txt`；`Shift+点击` → 5 行区间；在列表行下方的空白处向上拖动 → 6 行全选；
  切到网格（`explorer-qt.ini` 的 `viewMode=grid`）后拖动 → 3 个 tile，计数一致。
- 没有计时：省掉的是每行一次 cxx-qt 调用 + 一次 `QString`/JS 字符串，量级按行数
  线性；这是结构结论，不是这一轮量出来的。

## 6. 选区表示（已实施）

### 6.1 原来的形状

选区是 tab 上的 `path -> true` map，三处代价都是这个形状直接导致的：

- `Tab.toggle` 先把整个 map 复制一份再翻一个键（旧 `qml/Tab.qml:192-205`）：
  全选后 Ctrl+点击是 O(N) 次哈希插入 + 分配，每次点击都做；
- `selectionCount` 在 `selectionVersion` 变化时遍历整个 map（旧 `qml/FileList.qml:97-108`），
  状态栏每问一次就走一遍；
- 每个 delegate 的 `selected` 是一次 JS 函数调用加一次字符串键查找（旧
  `FileList.qml:986`、`Tab.qml:215-218`），靠读 `selectionVersion` 制造失效；
  `cacheBuffer: 2600` 下每次变更约 236 次调用，而且这 236 次里绝大多数行的
  结果没有变化。

### 6.2 现在的形状

选区进模型，tab 只留“锚点”和“历史快照”：

- **行带自己的 flag。** `StoredRow` 与 `SearchRow` 加 `selected`，模型发布成
  `selected` 角色，delegate 声明 `required property bool selected`。`StoredRow` 的
  `PartialEq` 手写并**排除** `selected`：刷新时的行 diff 是“这一行代表同一份元数据”，
  点击不是一次目录变化。
- **只发表变化的行。** 每次改选区，`Selection` 计算前后两个有序行索引集合的差，
  把连续段合成 `dataChanged(top, bottom, [ROLE_SELECTED])`——Ctrl+点击一行只发 1 段，
  全选后单独取消一行发 2 段，其余 delegate 的绑定不重算。
- **计数是维护出来的。** `selectedCount` 是模型属性，随 flag 变化更新；状态栏读
  属性而不是遍历。
- **点击不扫全表。** 每条操作更新的是一份“已选行索引”有序表
  （`crates/dae-qt/src/selection.rs`）：行 flag 是真相，这份索引是缓存，任何
  插入/删除/重排都把它作废，下一次选区操作再从 flag 重建扫描一次（一次重排一次，
  不是一次点击一次）。选中一行时只碰那几行。
- **框选在模型里算。** `beginMarquee(keep)` 记下拖拽起点的基线，`marqueeCells(...)`
  把“基线 ∪ 矩形里的格子”写回 flag；列表的矩形退化成一行区间，网格传行列矩形和
  列数。QML 不再每次 pointer move 拼路径数组、也不再用 `Set` 合并。
- **历史按路径恢复。** 历史条目存路径数组；`setSelectedPaths(paths)` 把路径折算成
  行键（`base` 还没到时先存路径，等首个批次带着 `base` 到达再折算），行还没到
  （导航刚发起时常见）就先挂着，流式批次逐批匹配，流结束时把没匹配上的丢掉。
  所以“选中一批、进目录、返回”不会变成“返回后什么都没选中”。
- **搜索与分栏各有自己的模型。** `SearchModel` 用同一份 `Selection` 与同一个
  `selected` 角色（`FileList`/`FileGrid` 的 delegate 是共用的）。分栏视图每一列
  是一个模型，`ColumnView` 汇总各列的已选路径与计数，状态栏和拖拽仍是“整条
  trail 的选区”。
- **可行的最小面。** 其余不动：锚点仍只在 tab（`selectionAnchor`），模型不存
  “当前行”，拖拽/右键/预览仍由视图发起。

### 6.3 验证

- 单测（`selection` 模块）：只留一行、翻转、区间裁剪、清空的分段、按路径恢复
  （行未到 / 行已到）、框选基线与回缩、网格单元格、行编辑后索引重建、删除行
  离开计数。
- app 内冒烟（debug 构建，`DAE_LOG=debug`，按窗口标题找句柄、用物理坐标驱动；
  150% 缩放；30 条目的目录 + 6 条目的目录）：
  - 列表：点击一行 → `30 items · 1 selected`；Ctrl+点击第二行 → `2 selected`；
    `Ctrl+A` → `30 selected`；Ctrl+点击已选行 → `29 selected`；`Esc` → 清空；
    `Shift+点击` → 7 行区间；`F5` → 选区与滚动位置都保留；
    在行下方空白向上拖 → 6 行全选。
  - 网格（`viewMode=grid`）：点击 tile、Ctrl+点击第二个 tile；拖动框选只覆盖
    被框住的行（2 个 tile）；Ctrl+拖动 → 基线 ∪ 框内 = 6 个 tile。
  - 分栏（`viewMode=column`）：左列点 `alpha` 钻入 → 右列点 `f2.txt`，状态栏
    `2 selected`、预览显示两项：跨模型汇总成立。
  - 搜索：`a01` → 1 条结果正常绘制（`selected` 角色缺失的话 delegate 根本不会
    创建），点击结果 → 高亮 + 预览；`Esc` 清空结果选区。
  - 历史：选中 7 行 → 进子目录 → 返回，返回的是进入前那一刻的选区（与旧行为
    一致：推入历史条目的时机在点击之后）。

### 6.4 顺手修掉的一处坐标错位

网格框选原来用 `columns`（把可用宽度按最小格宽切分得到的列数）把“行列”换算成
行号，而 `GridView` 实际按 `cellWidth = tileWidth + gapX` 流式排布，二者在余量
小于一个间隙时差一列（这台机器上是 5 与 4）。旧代码同样按 `columns` 算下标，
所以框选一直会选到下一行去。`FileGrid.flowColumns` 现在按 `GridView` 自己的
排布算，框选与所见一致（§6.3 的网格冒烟就是在修完之后才对上）。

### 6.5 没变的 / 仍然 O(n) 的地方

- `TrashView.qml` 仍用同一套 `path -> true` map + `selectionVersion`：回收站的行是
  另一份模型（`trash_model.rs`），它的 delegate 也自带状态，本轮没有一起搬。
  §7 的清单里记着这件事，等回收站自己也变成“行带 flag”的时候再回收。
- `selectedPaths()`（拖拽、右键、预览）会扫一遍行：调用点是用户动作，不是帧；
  选中的行才拼路径。
- 历史条目存的是路径数组，大选区会随后退栈保留多份——是内存问题，不是帧时间。

## 7. 次级清单（均未做，按体感可能性排）

> 本节一律用符号名定位：§10 的实现让审计时的行号全部错位过一轮。

| 位置 | 机制 | 备注 |
|---|---|---|
| `FileList.qml` 的 `list` | grid/column 模式下 list 仍常驻，165 个 delegate 随 reset 重建（§10.1 只按需化了 grid/column） | 要等 `list.` 的引用面收窄（滚动恢复、框选、前缀直接读它）后再 lazy |
| `model.rs` `layoutChanged` 代替 reset | 单轮阻塞已 ≤22 ms，收益不再明显；需要 `changePersistentIndexList`（cxx-qt 0.10 无绑定）和持久索引测试 | §10.6 |
| `FileList.qml` 的 `rowIcon` | 行内系统图标 `Image` 没有 `asynchronous: true`（同文件缩略图、ColumnView、FileGrid 都有），provider 调用发生在 GUI 线程 | 只在 `iconStyle == "system"` 时可见；默认 `themed` 走 Glyph，不触发 |
| `model.rs` 的 `git_badge` | git 徽标精确查不到就线性回退，干净行（大多数）都走这条 | 成本 = O(行 × 脏条目)，只在有大量未提交改动的仓库里明显 |
| `ShellWindow.qml` 的 `onSelectionChanged` → `PreviewPanel.showPaths` | 选区一变就重开预览，无去重、无防抖；`showPaths` 只在恰好 1 项时 `open`（多选是 `clear`），单行框选会每次重开 | 结果应用在 GUI 线程，超时的任务靠 generation 丢弃但会跑完 |
| `Glyph.qml` 的 `canvas.onPaint` | 每行一个 `Canvas.Immediate`，悬停/选中改 tint → `requestPaint`，paint 里 `d.match(...)` 每次重新分词 | 仓库做过 Canvas→Rectangle 的创建期 A/B（`qml-aot.md` §7）无差异；重绘路径**未测**（§9.1），所以不排前 |
| `OverviewModel::refresh`、`SidebarModel::refresh` | 刷新在 GUI 线程同步读 `recents`/`favorites`/`hidden` 等 JSON，再 `replace_rows` 两次 | Overview/侧栏切换时；文档注释里“这三次读是便宜的”只在热缓存成立 |
| `DirectoryListModel::data` | 每次角色读取重新格式化 size/time/路径/图标/缩略图；delegate 声明 13 个 required role，创建时全部读取 | 每屏几十行、每行 2–4 次分配，量级小于排序，但在滚动路径上 |
| `Tab.qml` 选区路径数组存入 `historyEntry()` | 大选区会随后退栈保留多份 | 内存，不是帧时间（§6.5） |
| `TrashView.qml` 的 `selectionVersion`/`isSelected` | 回收站仍是 `path -> true` map + `selectionVersion`，delegate 每次变更跑一次 `isSelected` | 行在 `trash_model.rs` 里；要等那份模型也带 `selected` 角色时一起搬（§6.5） |

## 8. 已经做对，不要重做

流式批次（512 起步翻倍到 8192）、150 ms watcher 防抖、`row_edit` 差量刷新、
类型图标按扩展名共享 URL、`StoredRow.path` 只给越界行保存、
进度事件 60/200/250 ms 节流、PGO/AOT、`reuseItems` + `cacheBuffer`
（`engine-qt-quick.md` §5.1 已实测）。不要引入 `QSortFilterProxyModel`：
过滤在模型内（`keep_visible`），排序在模型内，代理只会多一层索引映射。

## 9. 复现与测量

- 枚举/流水线：`bench/listing-pipeline/`（`measure`/`allocs`），以及
  `DAE_LISTING_BENCH=<dir> cargo test -p dae-core --lib -- --ignored reports_where_a_large_listing_spends_its_time`。
- 排序比较器：`cargo test -p dae-qt --lib --release -- --ignored --nocapture
  reports_where_the_completion_sort_spends_its_time`。同一个进程里对同一份语料
  各排 5 次：新实现与测试里保留的旧实现（逐字拷贝），并断言两者顺序一致。
  语料是 35,803 / 120,000 条 `IMG_<n%997><n>.png` 形状的名字。
- watcher 补丁：`cargo build -p dae-app`，用 `DAE_LOG=debug` 启动
  `target\debug\dae.exe <一个目录>`，从外部创建/改名/删除文件，看
  `dae::listing` 的两行（“patching N child(ren)” 与 “N rows now (…)”）；
  出现 “re-reading” 就说明这一批退回了重读，行里会写明原因。
- app 内整段（**基线，2026-11-03**）：release + `DAE_QML_PROFILE=1`（`build.rs`
  只为这种构建打开 `QT_QML_DEBUG`，与 debug 构建同一个开关）、`QMAKE` 指到
  Qt 6.12 的 `bin`，同时开 `DAE_LOG=debug` 和

  ```text
  qmlprofiler --include javascript,binding,handlingsignal,creating,compiling,scenegraph,animations,painting,pixmapcache \
    -o <trace.qtd> -- target\release\dae.exe K:\dae-listing-bench\huge-natural
  ```

  等列表稳定后关闭窗口——trace 只在退出时落盘。release 是 GUI 子系统、没有
  stderr，`DAE_LOG=debug` 的行在 `%LOCALAPPDATA%\com.melaine.dae\logs\dae.log`；
  视图模式在 `explorer-qt.ini` 的 `viewMode` 里，量列表路径时要是 `list`。

  语料必须让 `rows_are_sorted` 失败：`bench/listing-pipeline prepare` 的定宽名字
  在 K:（ReFS）和 C:（NTFS）上都恰好已经是视图顺序，完成时没有 `re-sort` 行、
  trace 里也没有 `saveScroll`；这里用 `huge-natural`（120,000 个
  `IMG_<n%997><n>.png` 空文件，与单测同名形）。

  **两段 native 成本**（日志行 `re-sort: 120000 rows, sort … ms, reset … ms`，
  8 次运行 = 4 次普通 + 4 次 qmlprofiler，三视图常驻、同步排序）：

  | | 中位 | 范围 |
  |---|---:|---:|
  | `sort_rows`（`StoredRow`，GUI 线程） | 66.6 ms | 61.4–99.8 |
  | `begin/endResetModel`（含 `selection.invalidate`） | 27.9 ms | 18.7–46.8 |

  同一二进制的波动就有 ±40%，所以这里取中位并给出范围。这两段在同一个事件循环
  轮次里先后执行，单轮阻塞的中位数相加约 **94.5 ms**。

  同机复跑 §2.2 的单测（`Vec<String>`）是 45.4 ms / 120,000 条（两种比较器
  163.9 / 45.4 = 3.6×）。app 内多出的约 20 ms 落在元素本身（**结构推断**：
  比较器相同，差异只剩元素大小与后台线程）：`StoredRow` 约 96 B，`String` 24 B，
  稳定排序的搬移量与缓存足迹差一个量级；排序期间 git/缩略图线程也在抢核。
  **表里的 34.0 ms 与 4.3× 都是单测 `Vec<String>` 上的数；比值可以用，绝对成本
  不能直接搬到 app。** §10.2 之后，≥2048 行的完成排序走 worker，下面这些数仍是
  比较器与原地排序的成本，不再直接等于完成帧里的阻塞。

  **reset 之后**（事件区间会嵌套，只对极大区间求和；窗口 600 ms）：

  | | 中位 | 范围 |
  |---|---:|---:|
  | 顶层 QML 忙时 | 87.8 ms | 70.7–94.1 |
  | └ `Creating` | 37.5 ms | 28.1–40.0 |
  | └ `Binding` | 24.1 ms | 17.8–24.3 |
  | └ `HandlingSignal` | 24.8 ms | 21.1–31.1 |
  | └ `Javascript` | 0.9 ms | 0.8–2.4 |

  重建了 341 个 delegate：list 164、grid 100、column 77。`FileList` 里的三个视图
  （`FileList.qml` 的 `list`/`gridSurface`/`columnSurface`）都常驻，
  `visible` 只切显示、不切模型，所以一次 reset 三份都重建；顶层 `<component>`
  创建分别是 list 9.7–13.2 ms、grid 4.2–5.4 ms、
  column 3.5–5.3 ms，其余 `Creating` 是行内的 `Text`/`Glyph`/`Connections`。换成
  120,000 个 `.txt`（不触发缩略图）再跑一次，reset 之后是 63.6 ms（`Creating` 29.5、
  `Binding` 17.8、`HandlingSignal` 15.7），形状相同、数值略低——大头是 delegate
  本身，不是失败的图片加载。

  **方法更正：`SceneGraph:6` 的间隔不是帧时间。** Qt Quick 按需渲染，没有动画时
  两次渲染之间可以是空闲；早先记的“包含 reset 的那一帧 105–169 ms”就是这种间隔，
  把空闲算了进去。这一版改用两个不受渲染节奏影响的量：日志里的 native 块
  （sort+reset）与 trace 里的**连续 QML 忙时**（极大区间、间隔 ≤1 ms 才并成一段），
  后者近似“一个事件循环轮次里 GUI 实际做了多少”。

  3.58 万条的基线（一次运行）：sort 16.0 ms、reset 22.9 ms（reset 不随行数缩小：
  它主要是视图侧的失效工作），reset 后 QML 忙时 115.2 ms，delegate 同为 341 个
  ——可见 delegate 数量由视口和 `cacheBuffer` 决定，与目录行数无关。

  **适用面。** 排序与 reset 是同一份 Rust 代码、同一个 `QAbstractItemModel`，
  单次成本与文件系统无关；会变的是它**是否发生**，以及有没有跨批拼接：

  - 本地流式读（`open_streamed_listing`）：每批在 `take_rows` 里用 `name_order`
    排，批与批按枚举顺序拼接（`local/directory.rs:151-155`）。只要枚举序不是名字序，
    `rows_are_sorted` 就失败，完成时照排一遍。K:（ReFS）与 C:（NTFS）都实测了
    定宽语料（守卫通过）与自然序语料（守卫失败）；ext4 的 htree 哈希序、APFS、
    exFAT 的创建序是**按代码推断**会失败，没有机器复测。
  - 远端（SMB/SFTP/cloud）的 `read_dir` 一次返回整个 `DirectoryView`
    （`stream_id: None`，远端分支），后端已经用
    `entry_sort_key`（= `name_sort_key`）排过；没有跨批乱序，只有“后端序 ≠
    视图序”（数字段、大小写并列、symlink 分组）时才会在完成时再排一遍。
    单次成本相同，远端目录通常更小；延迟在 `read_dir`，不在排序。
  - 平台：这组数来自 Windows + D3D11。排序、置换与 reset 是 Rust/Qt 模型侧的工作，
    换平台应相近；`Creating`/`Binding` 的量级未在 macOS/Linux 上测。

### 9.1 明确延后的测量

这两处还没有数字，是决定而不是遗漏。它们不改变已经给出的排序/重置结论，
但触发条件变化时要回来补：

| 路径 | 已知的 | 为什么现在不测 | 回来测的条件 |
|---|---|---|---|
| watcher 补丁（§4.3） | 省掉一次整目录枚举（12 万条 ≈55 ms，仓库 bench）加 §2 的完成重排；`plan_patch` 的单测与 app 冒烟已覆盖行为 | 需要可重复的外部事件流（批量改名/下载）才能定时；这条路径的风险是正确性（退回重读）而不是单次成本，正确性已有测试 | 大目录的改名/下载仍卡顿；150 ms 防抖或 `PATCH_LIMIT` 改动；watcher 后端换实现 |
| `Glyph` 重绘（§7） | 创建期 Canvas→Rectangle 的 A/B 无差异（`qml-aot.md` §7）；悬停/选中改 tint 会 `requestPaint`，`onPaint` 里 `d.match(...)` 每次重新分词 | 只有 `themed` 图标的悬停/选中会触发，没有为它造过悬停/框选的计时场景；qmlprofiler 需要单独开 `painting` 场景 | 悬停或框选整屏时掉帧；准备把 Glyph 换成 Rectangle 或缓存绘制前要基线 |

> §5.3 的“没有计时”不在这张表里：它省的是逐行跨 FFI，量级按行数线性，
> 行为已由 app 内冒烟覆盖；要数字时按 §5.3 的入口补 qmlprofiler。

## 10. 复测后的四刀（已实施）

### 10.1 可见视图按需

`FileGrid` 与 `ColumnView` 原来和列表一起实例化，`visible` 只挡绘制；一次 reset
三份 delegate 都重建。现在两者各包一层 `LazySurface`（`FileList.qml` 的
`gridSurface`/`columnSurface`）：第一次切到该模式才创建，之后保留。列表仍常驻
（它的 `list.` 引用面太大，见 §7）。

### 10.2 完成排序移出 GUI 线程

`sort_at_completion` 对 ≥2048 行改为：

1. 在 Qt 线程上做一份只含比较器输入的 `SortEntry` 快照（`name` 克隆 + 三个 Copy
   字段），记 `generation`/行数/`ViewOrder`；
2. worker 排 `Vec<u32>` 下标（`compare_rows` 的索引版本 + 下标做稳定序的
   tie-break，因此可以用 `sort_unstable_by`）；
3. 结果用 `qt_thread().queue` 回到 Qt 线程，重新核对 generation、行数、排序键和
   置换合法性；换目录、改排序、成绩过期都直接丢弃；
4. 合法就按置换重排行、`aboutToReload` → reset → `sortApplied`，QML 在
   `onSortApplied` 里恢复滚动位置。≤2048 行仍走原来的原地 `resort_rows`。

语义等价由测试钉住：`the_completion_permutation_matches_the_stable_sort`
（含大小写并列与随机名字）、`a_permutation_that_does_not_cover_every_row_is_rejected`。

### 10.3 结果

12 万条（列表模式；基线为三视图 + 同步排序，10.1 为单次运行，10.1+10.2 为 3 次中位）：

| | 基线 | 10.1 后 | 10.1+10.2 后 |
|---|---:|---:|---:|
| 单轮 GUI 阻塞 | sort 66.6 + reset 27.9 ≈ **94.5 ms** | 60.1 + 10.7 ≈ **70.8 ms** | permute 2.3 + reset 10.7 ≈ **13.0 ms** |
| reset 后 QML 忙时 | 87.8 ms | 44.1 ms | 34.5–36.1 ms |
| reset 时重建的 delegate | 341（list 164 / grid 100 / column 77） | 165（list） | 165（list） |
| 最长连续 QML 忙时 | — | 16.7 ms | 8.5–16.9 ms |

3.58 万条：

| | 基线 | 10.1 后 | 10.1+10.2 后 |
|---|---:|---:|---:|
| 单轮 GUI 阻塞 | 16.0 + 22.9 ≈ **38.9 ms** | 16.8 + 8.3 ≈ **25.1 ms** | permute 0.6 + reset 10.2 ≈ **10.8 ms** |
| reset 后 QML 忙时 | 115.2 ms | 63.7 ms | 31.8–46.3 ms |
| 最长连续 QML 忙时 | — | 10.1 ms | 11.2–11.5 ms |

排序本身的成本没有消失，只是换了线程：worker 上 12 万条 69.8 ms（68.7–71.2）、
3.58 万条 18.6 ms（18.5–18.8），比同进程原地排慢 10–16%（下标访问快照，缓存
不如连续行）。快照 1.6 / 0.4 ms、应用置换 2.3 / 0.6 ms。sorted 视图比 `finished`
晚到约 60–80 ms（两种规模都在这个范围：worker 排完后还要等 GUI 线程空出来），
这段期间列表已经可读可点。

**结果：完成路径上再没有一个超过 ~22 ms 的 GUI 轮次**（60 Hz 一帧 16.7 ms；
12 万条的最长轮次是 reset 后那一段 16.9 ms，3.58 万条是 apply 轮次 21.8 ms），
计划里的 ≤33 ms 目标达成。

### 10.4 交互式改排序（已实施）

`set_order` 以前在 GUI 线程直接走 `resort_rows`，12 万条点列头就是 §10.3 基线里
同一段 60–100 ms 的阻塞。现在 ≥2048 行走 §10.2 的同一条 worker 路径：快照比较器
输入、worker 排下标、回到 Qt 线程复核后应用置换并 `sortApplied`；≤2048 行仍原地排。

交互式排序的 generation 和行数在点击时都不变，§10.2 的复核无法区分“同一份列表的
旧结果”。所以每次 worker 排序带一张 `sort_request` 票据：`load`、同步
`resort_rows`、`finish_refresh` 都递增它；应用端先比票据，再比 generation、行数、
排序键，只有最新一次能落地并清除 `sorting`。旧结果既不改行，也不会提前放行
watcher 补丁。

app 内实测（release，120 000 条，`DAE_LOG=debug`，列表模式）：

| | 快照（GUI 线程） | worker 排序 | permute + reset（同一轮 GUI） |
|---|---:|---:|---:|
| 打开目录的完成排序 | 1.6 ms | 35.1 ms | 2.4 + 10.6 ≈ **13.0 ms** |
| 点 `Modified` 列头 | 3.3 ms | 24.3 ms | 2.4 + 10.4 ≈ **12.8 ms** |

第一次点一个新列会先改 `sortKey` 再改 `sortOrder`，QML 因此连续调两次 `setOrder`，
启动两次 worker（debug 日志能看到两行 `sort:`）；票据让第一次的结果被丢弃，只有
第二次落地。这是两个偏好写入的既有形状，不是新增的 GUI 阻塞；要省掉那次多余快照，
可以在 QML 把两次写入合成一次调用。

排序本身仍在 worker 上跑 24–35 ms，列表在结果回来前保持旧顺序——收益是点击不再
冻结 GUI，而不是立即换序。

### 10.5 终端输出按帧合并（已实施）

终端以前把每个跨线程的 `updated` 都换成一次 `refresh`：Rust 侧只合并同一瞬间的
突发，GUI 处理完一帧就允许下一次进来。构建日志这种每秒几万行的输出会让 GUI 线程
持续拉帧、解码、重绘，没有帧率上限。

`TerminalItem::refresh` 现在把拉帧限制在每 16 ms 一次：间隔内的唤醒只置一个
`m_refreshQueued`，由单次定时器在剩余时间到达时补最后一帧。终端仍是拉取模型，
丢掉的只是中间帧，不是最终状态；输入回显、滚动和主题切换最多晚一帧。

app 内实测（release，100 000 行 `1..100000`，同样是 4 s 完成）：

| | 进程 CPU 累计 |
|---|---:|
| 合并前（每次唤醒一帧） | 3.09 s |
| 合并后（≤60 帧/秒） | 2.30 s |

省下约 26% 的 CPU，墙钟不变；剩余成本主要是 PTY 读取与网格解析，不在刷新路径上。

### 10.6 没做的

- **`layoutChanged` 代替 reset**：单轮阻塞已经 ≤22 ms，§2.4 的触发条件不再成立；
  它还需要 `changePersistentIndexList`（cxx-qt 0.10 没有绑定）和持久索引正确性的
  测试。留在 §7。
- **列表视图在 grid/column 模式下也常驻**：那两种模式下 list 的 165 个 delegate
  仍随 reset 重建。要等 `list.` 的引用面收窄（滚动恢复、框选、前缀都直接读它）
  才能同样 `LazySurface` 化。留在 §7。
- **watcher 刷新**：§4.3 的整段仍未计时（§9.1）；§10 量的是打开路径。
- **预览去抖**：`PreviewPanel.showPaths` 只在恰好 1 项时 `open`，多选是 `clear`；
  grid 方向键只移动光标、不改选区，连按 3 s 的 CPU 增量约 0.03 s。没有量到
  帧成本，不做。
