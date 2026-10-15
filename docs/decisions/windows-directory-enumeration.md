# Windows 目录枚举改用 `FindFirstFileExW` + `FIND_FIRST_EX_LARGE_FETCH`：**已实测，已实施**

> **状态：已实施**（只动 Windows 本地后端；Unix/macOS 走原路径，一行未改）。
> harness 与原始数据在 `bench/listing-pipeline/`；应用侧测量是
> `file_system/local/directory.rs` 里的 `reports_where_a_large_listing_spends_its_time`
> （`#[ignore]`，四行：`std walk` / `std rows` / `streamed` / `listing`——A/B 看 `streamed`）。
> 被测机器：Ryzen 9 7940H（8C/16T）、39 GiB RAM、K: 卷为 ReFS on
> ZHITAI TiPlus7100 NVMe。
>
> **这批改动曾经整批退出工作区**（备份 `K:/dae-discarded-work-20261015-162141/`），
> 已经恢复。恢复后在同一台机器上复跑，两侧都对得上当时的中位：出货函数 35,803 条目
> **14.13 ms**（当初 13.75 / 14.87），镜像 harness `std+key` 18.83 ms / `win32+cmp`
> **13.08 ms**（当初 17.99 / 12.64）。

## 0. 结论先行

**资源管理器的关键路径——目录列举——在 Windows 上约 78% 的时间花在「把目录读出来」这一件事上，
而排序只占约 7%。**（§1，35,803 条目实测：列举 14.15 ms / 构造行 2.96 ms / 排序 1.18 ms。）

Rust 的 `std::fs::read_dir` 在 Windows 上已经用了 `FindFirstFileExW` + `FindExInfoBasic`，
但**故意不传** `FIND_FIRST_EX_LARGE_FETCH`。`library/std/src/sys/fs/windows.rs` 的注释直说：

```text
We can pass FIND_FIRST_EX_LARGE_FETCH to dwAdditionalFlags to speed up things
more, but as we don't know user's use profile of this function, lets be
conservative.
```

文件管理器打开一个 3 万条目的目录，正是这个 flag 存在的理由。实测：

| | 35,803 条目 | 120,000 条目 |
|---|---|---|
| 改动前（HEAD，走出货函数） | 24.17 ms | 89.35 ms |
| 改动后（走出货函数，两次独立测量） | **13.75 / 14.87 ms** | **51.84 / 54.04 ms** |
| | **−43% ~ −38%** | **−42% ~ −40%** |

两次复测差 ~8%，是机器负载与目录索引缓存状态的差异（§7）：比值稳定，绝对值要连着当时状态一起读。

镜像 harness（mimalloc，与出货二进制一致）上，把两个改动分开记：

| | 35,803 条目 | 120,000 条目 |
|---|---|---|
| 改动前流水线（`std+key`） | 17.99 ms | 69.65 ms |
| 只换枚举（`win32+key`） | 13.02 ms（−27.6%） | 52.17 ms（−25.1%） |
| 再换排序 key（`win32+cmp`） | 12.64 ms（−29.8%） | 50.71 ms（−27.2%） |

**没有收益的地方也写清楚：** 首次绘制基本不变——512 条目的头一批只省 29 µs（§3）；
250 条目的小目录差异在噪声里（§3）；Windows 之外的平台没有等价改动，因为 Unix 的
`readdir` 本身就是带缓冲的（§6）。

## 1. 先测量：时间花在哪

不改任何东西，先用 `bench/listing-pipeline` 把一条列举拆成三段（每个 batch 各记一段，
再求和）。35,803 条目、mimalloc、9 次取中位：

| 阶段 | 时间 | 占比 |
|---|---|---|
| 列举（`read_dir` / `FindNextFileW`，不建行） | 14.15 ms | 78.6% |
| 构造行（元数据 + 名字 `String` + `ListingRow`） | 2.96 ms | 16.5% |
| 排序（`name_sort_key` + `sort_by_cached_key`） | 1.18 ms | 6.5% |

一整批 35,803 条目的 CJK 目录、9 个配置全跑的同一张表给出同样的形状
（`results/cjk.tsv`）：列举 13.85 ms（77.5%）、构造行 2.71 ms（15.2%）、排序 1.31 ms（7.3%）。

也就是说：**先动排序是划算不到哪去的**；瓶颈在系统调用次数上。这一条把后续的候选筛掉了大半。

## 2. 三组数字

### 2.1 出货函数（最强的证据）

镜像 harness 再好也只是复刻，所以另外在 `directory.rs` 里留了一个 `#[ignore]` 测量测试
（现在的名字是 `reports_where_a_large_listing_spends_its_time`；它印四行，表里对照的是
`streamed` 那一行），直接走 `open_canonical_listing` + 游标循环（也就是
`open_streamed_listing` + `read_remaining_batches` 的形状）。同一个测试、同一台机器、
同一份数据，对照是 `git worktree` 里的 HEAD（只把测试本身抄过去）：

| 数据集 | HEAD | 改动后 | |
|---|---|---|---|
| 35,803 条目 | 24.168 ms（7 次中位） | 13.746 ms / 14.870 ms | **−43% / −38%** |
| 120,000 条目 | 89.354 ms | 51.838 ms / 54.038 ms | **−42% / −40%** |

（两组「改动后」是两次独立测量；第二组跑的时候机器更忙，同一份数据的 7 次里最高到 17.6 ms——
这就是 §7 那条提醒的由来。对照的 HEAD 只测了一次，但它的中位与最低值 23.94 ms 只差 1%。）

两个数字都比 harness 大，原因只有一个：**`cargo test --lib` 的二进制里没有
`main.rs` 的 `#[global_allocator]`，跑的是系统堆**，而这条流水线原本每条目要多做
2 次分配（排序 key）外加 `Arc<PathBuf>` 与 `OsString` 的中转。出货二进制装了 mimalloc，
所以对照 harness 那一栏更接近真实出货环境；两者结论方向一致，绝对值按堆分别记。

### 2.2 镜像 harness（mimalloc，与出货一致）

见 `bench/listing-pipeline/README.md` 与 `results/*.tsv`：

| 条目数 | `std+key`（改动前） | `win32+key` | `win32+cmp` |
|---|---|---|---|
| 35,803（混合名） | 17.99 ms | 13.02 ms（−27.6%） | 12.64 ms（−29.8%） |
| 120,000 | 69.65 ms | 52.17 ms（−25.1%） | 50.71 ms（−27.2%） |
| 35,803（全非 ASCII） | 17.86 ms | 12.93 ms（−27.6%） | 12.53 ms（−29.8%） |

排序 key 那一项还给出干净的解释：**4.03 → 2.03 次分配/条目**
（`results/allocs.tsv`，35,803 条目共 144,209 → 72,595 次分配），差额正好是
`name_sort_key` 每条目生成的两个 `String`。

### 2.3 单个 flag 的 A/B

`win32` 与 `win32-nofetch` 只差这一个 flag，其余（`FindExInfoBasic`、
`FindExSearchNameMatch`、`.`/`..` 过滤、行的构造）完全相同，所以这一对差异就是这个 flag 的：

| 条目数 | `std` | `win32-nofetch` | `win32`（带 flag） |
|---|---|---|---|
| 35,803 | 14.15 ms | 13.03 ms | 9.76 ms |
| 120,000 | 55.78 ms | 53.76 ms | 40.32 ms |

这个 A/B 反着跑过 6 次（每轮 9 次取中位、两种顺序各一遍），结果稳定：
no-fetch 12.7–13.4 ms，large-fetch 9.3–9.8 ms。也就是说「带 flag 更快」不是
缓存预热顺序造成的：后跑的那个也赢。

`std` 与 `win32-nofetch` 之间还有约 0.8 ms 的差，来自 `std` 每条目多做的事：
`DirEntry` 会克隆一次 `Arc<PathBuf>` 并整份拷贝 592 字节的 `WIN32_FIND_DATAW`，
而构造行时 `file_name().to_string_lossy()` 要走 wide → `OsString` → `String`。
新实现直接把行写在 `FindNextFileW` 填的那个缓冲上。

## 3. 小目录与首次绘制：没有收益，也没有退步

用户真正等的是**头一批 512 行**。所以 250 / 512 条目这两个常见尺寸各跑 25 次取中位
（只测列举本身）：

| 条目数 | `std` | `win32-nofetch` | `win32` |
|---|---|---|---|
| 250 | 0.081 ms | 0.070 ms | 0.070 ms |
| 512 | 0.135 ms | 0.119 ms | 0.106 ms |

512 条目省 29 µs，一帧是 16.7 ms——**首次绘制不受影响**。250 条目上带不带 flag 完全一样
（缓冲还没到瓶颈），差异在噪声里。

所以这个改动的正确说法是：**它买的是「列举跑完」，不是「面板打开」**——大目录滚动到底、
全选、在列表里搜索都更快，35,803 条目省 4.97 ms、120,000 条目省 17.5 ms（还没算排序那部分）。

## 4. 机制与实现

- 新模块 `file_system/local/windows_directory.rs`：`FindFirstFileExW(FindExInfoBasic, …,
  FIND_FIRST_EX_LARGE_FETCH)` + `FindNextFileW`，行直接由 `WIN32_FIND_DATAW` 构造。
  `windows` crate 本来就是依赖（`Win32_Storage_FileSystem` 已启用），**没有新增依赖**。
- `DirectoryListingCursor.iterator` 变成按平台选择的类型；Unix 分支一行未动。
  新迭代器 `unsafe impl Send`：游标会被交给列举的读线程，这与 `std` 对
  `FindNextFileHandle` 的声明同源。
- **路径用 verbatim（`\\?\`）拼法**：这正是 `std` 的 `get_long_path` 在
  `prefer_verbatim` 分支里做的事，而 `types::canonical_path` 本来就已经解析过一次。
  `dunce::simplified` 只在**不会改变指向**时才去掉前缀，而它保留前缀的门槛是 260 个
  UTF-16 单元、`std` 转换的门槛是 248——**248..260 这个夹缝里的目录，前缀被去掉了，
  但系统调用仍需要它**。所以这里无条件补前缀（比 `std` 更稳），并留了一个测试专门造一条
  250 单元长的路径来钉死这个边界。

三件容易出错的事都钉了测试（`cargo test --lib` 全绿）：

| 不变量 | 测试 |
|---|---|
| 两条枚举路径产出**逐字段相同**的行（含符号链接、reparse tag、只读位、非 ASCII 名） | `the_two_walks_build_the_same_rows` |
| `win32` 的路径拼法与 `std` 的 `get_long_path` 一致（drive / UNC / `\\.\` / 已是 verbatim / 相对） | `spells_the_search_pattern_the_way_std_does` |
| 248..260 单元的目录仍然可列举 | `lists_a_directory_whose_path_is_at_the_max_path_boundary` |

## 5. 顺带改掉的排序 key

`name_sort_key` 返回 `(rank, name.to_lowercase(), name.to_owned())`——**每条目两个 `String`**，
被 `sort_by_cached_key` 缓存整个排序过程。改成等价的全序比较器
`types::name_order`（ASCII 快路径按字节比小写；非 ASCII 走 `char::to_lowercase()` 流，
与 `to_lowercase()` 拼接结果同序），零分配：

- 排序 3.97 → 2.35 ms（120,000 条目），1.18 → 0.68 ms（35,803）
- 分配 4.03 → 2.03 次/条目
- 整条流水线再快 2–3%（`win32+key` → `win32+cmp`）

两条排序路径（网络后端的 `entry_sort_key` 仍用 key）必须给出同一个顺序，
`the_cached_key_and_the_comparator_agree_on_order` 用多字符小写展开（`İ`）、
大小写折同、CJK、重音等用例钉住这一点；harness 在真实目录上另外检查一遍
（`# parity identical`）。

**没有去掉后端排序**：`navigation.ts` 的 `isSameListing` 依赖「同一个未变化的目录
重读回来顺序相同」，`read_dir` 的顺序不保证稳定，所以这个全序是功能而不是优化。

## 6. 没做的、否决的候选

| 候选 | 结论 |
|---|---|
| 直接删掉后端的每批排序 | **否决**：`isSameListing` 依赖它是全序（见 §5）。收益（约 7%）也不值这个风险 |
| 并行排序 | 未采纳：批次最大 8,192、排序只占 7%，而 rayon 全局池是和 UI 抢的（`collect_rows` 的注释已经写过这一点） |
| Unix 侧做等价改动 | **无需**：`std` 在 Unix 走 `readdir`，glibc/BSD 的 `readdir` 自带缓冲，本来就一次取一大块；这个 flag 的收益来自「内核缓冲变大」，Unix 没有对应物 |
| 换 IO 机制（IOCP/io_uring） | 之前已实测否决，见 `completion-io.md`；本次结论与它一致：该动的是**系统调用次数**，不是派发机制 |

## 7. 复现

```bash
# 1) 准备数据（幂等）：一个 3.5 万条目的目录
cd bench/listing-pipeline && cargo build --release
./target/release/listing-pipeline prepare K:/dae-listing-bench/big 35803
./target/release/listing-pipeline prepare K:/dae-listing-bench/huge 120000

# 2) 镜像 harness：每个配置的列举/建行/排序分段中位数，并先校验顺序一致
./target/release/listing-pipeline measure K:/dae-listing-bench/big \
  --passes 9 --warmup 3 --variants std,std+rows,std+key,std+cmp,win32,win32-nofetch,win32+rows,win32+key,win32+cmp
./target/release/listing-pipeline allocs K:/dae-listing-bench/big

# 3) 出货函数（注意：lib 测试跑系统堆，绝对值比 harness 大 ~30%）
cd ../../src-tauri
DAE_LISTING_BENCH=K:/dae-listing-bench/big \
  cargo test --lib --release -- --ignored --nocapture reports_where_a_large_listing_spends_its_time
```

**量之前先看负载**：harness 是单线程，机器忙的时候每一行一起变慢。同一份数据同一配置，
我在负载高时量到过 19.6 ms、安静时 18.0 ms；120,000 条目更敏感（目录索引是否还在缓存里，
`std` 的列举本身就从 10.3 ms 变到 14.2 ms）。**比值可复现，绝对值要连着当时的状态一起读。**

## 8. 什么条件下重新评估

- **`std` 自己传上这个 flag**（或提供开关）：那就能删掉 `windows_directory.rs` 里
  除了「直接建行」之外的全部代码。也就是 `library/std/src/sys/fs/windows.rs` 里
  那句「lets be conservative」被改掉的那天。
- **Windows 之外的目录枚举出现真正的批量读接口**：那时 `win32` 的做法才有对应物。
- **本地列举不再是关键路径**（例如缓存层把重复打开目录全部挡掉）：那时该测的是缓存命中率，
  不是枚举。
