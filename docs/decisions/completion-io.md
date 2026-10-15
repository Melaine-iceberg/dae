# 文件 IO 换 IOCP / io_uring：**实测否决**（不改机制）

> **状态：已实测，结论是不换。** 实验分支 `exp/completion-io-bench`；
> harness 与原始数据在 `bench/io-backends/`（`results/` 为 Windows，`results-linux/`
> 为 io_uring）。应用代码**没有任何改动**——这一轮只做测量与判断。
> 被测机器：Ryzen 9 7940H（8C/16T）、39 GiB RAM、K: 卷为 ReFS on ZHITAI
> TiPlus7100 NVMe。复现命令见 §8。

## 0. 结论先行

**换机制拿不到收益，因为瓶颈不在派发机制上。**

- 资源管理器的关键路径——目录枚举——**在 Windows 上根本没有异步 API 可用**：
  `FindFirstFileW`/`FindNextFileW` 是同步的，compio 的 fs 层（0.12.1）**连一个
  `read_dir` 都没有导出**。实测三种机制跑同一段阻塞代码：36 000 条目
  16 线程下 **17.2M–17.8M entries/s**（差 <4%，同一段阻塞代码；§4.2）。
- 小文件读取（预览 / 缩略图 / 校验和的形状）**换过去慢 12.6 倍**：
  20 000 × 8 KiB 全读，`spawn_blocking`+rayon 3614 MiB/s，compio **286 MiB/s**。
  原因是 compio-fs 在 Windows 上把 `File::open`、`metadata`、`set_len`
  全部实现为**阻塞池回退**（`spawn_blocking_with`），只有真正的 read/write 走
  IOCP——也就是说换过去是「保留阻塞池 + 再加一层完成队列往返」（§2.2、§4.4）。
- 拷贝**别想赢过 `CopyFileExW`**：`fs::copy` 在池子里 4255 MiB/s，
  compio 在驱动上自己 read+write 只有 1449 MiB/s（§4.7）。
- 唯一确定的 IOCP 优势出现在**深队列冷读**：采 32 GiB 冷窗口顺序读时，
  compio 吞吐与池子同级（2845 vs 2756 MiB/s）但**进程 CPU 只用 1/6**
  （1.6s vs 9.2s）。原因是 buffered overlapped read 的 memcpy 由缓存管理器的
  系统工作线程完成，记在 System 进程而不是我们进程（§4.5）。
- 而「IOCP 该赢」的那些场景，本应用**没有那个形状**：4 KiB 随机块读在
  Windows 上池子本身就遇到句柄/缓存争用（共享句柄时 16 线程 **787 MiB/s < 1 线程
  1238 MiB/s**），补上「每次在途读各用一个句柄」之后池子 **1468 MiB/s > compio
  936 MiB/s**（§4.3）。真要说收益，先修句柄/线程数，而不是换运行时。
- io_uring 侧（WSL2，compio 报告 `IoUring`）结论相同方向：cached 4 KiB 读
  16 线程 pread **31.5 GB/s** vs io_uring **3.2 GB/s**；io_uring 的每个 op 要
  提交+回收各一次（compio 还是 thread-per-core，单线程吃满也只有
  841k op/s）。**但 WSL2 的跨线程唤醒被放大到 65 µs/次，表里的 tokio 基线不可信**
  （§5）。

一句话：这一轮该做的事是**保持线程池**，把力气花在减少系统调用次数、批量读、
复用句柄上；等真有「深度队列 + 每 op 昂贵」的负载，再回访 IOCP（§7）。

## 1. 现状：这个应用的文件 IO 到底跑在哪

不是「全部走 Tokio 线程池」，而是四条互不相同的路：

| 路径 | 实现 | 引用 |
|---|---|---|
| Tauri 命令（预览、属性、git、打开方式…） | `tauri::async_runtime::spawn_blocking`，**每个命令一次** | `file_system/commands.rs`（:98, :192, :321 …），`preview.rs:1762` |
| 目录列举（资源管理器关键路径） | `std::thread`：一个读迭代器、一个序列化 IPC，中间有界通道；每条目一次 `DirEntry::metadata()` | `file_system/listing.rs:258`、`local/directory.rs:226` |
| 本机树拷贝/移动 | 独立 rayon 池（`transfer_pool`，核数-1）+ `fs::copy` | `local/operations.rs:26, :396` |
| 跨后端传输（SMB/SFTP/云/归档） | 手写双线程流水线，环形缓冲复用 | `transfer.rs:790` `copy_stream` |

另外 `hashing.rs:113`、`directory_size.rs:97`、`prefetch.rs:63` 用的是裸
`std::thread`；缩略图渲染是自有 `RENDER_WORKERS` 线程 + 通道
（`preview.rs:379`）。也就是说：**这个应用的 IO 早已是「若干条线程各跑阻塞循环」**，
Tokio 阻塞池只承担「一次命令 = 一次 handoff」的部分。

## 2. 被测的到底是什么

`bench/io-backends`（独立 crate，不参与 Tauri 构建）用同一份 `Vec<Op>` 描述工作，
四个后端跑同一份 op 列表、同一批路径、同样的在途数，只有派发方式不同：

| 后端 | 含义 |
|---|---|
| `tokio` | 每个 op 一次 `spawn_blocking` —— 今天每个 Tauri 命令的形状 |
| `tokio-chunked` | 每个线程一次 `spawn_blocking` 后内部循环 —— `read_directory` / `stat_named_children` / 拷贝的形状 |
| `rayon` | `transfer_pool` 的形状 |
| `compio` | 完成式：Windows 上 IOCP + `OVERLAPPED`，Linux 上 io_uring |

### 2.1 compio 确实是真 IOCP

不是「线程池改了个名字」：`compio-driver` 的 Windows 后端
`CreateIoCompletionPort(INVALID_HANDLE_VALUE, …, 1)` + `GetQueuedCompletionStatusEx`
（`compio-driver-0.12.5/src/sys/driver/iocp/cp/mod.rs`），文件以
`FILE_FLAG_OVERLAPPED` 打开（`compio-fs-0.12.1/src/open_options/windows.rs`），
读走 `ReadAt` op → `ReadFile` + `OVERLAPPED`。harness 每次运行都会打印实际绑定的
驱动（`# compio driver: IOCP` / `IoUring`），避免「用的是 polling 兜底却当成
io_uring」这种自我欺骗。

### 2.2 但 compio-fs 在 Windows 上有一半是阻塞池

`compio-fs` 0.12.1 的 Windows 实现里，这些全是回退到阻塞池的：
`File::open`（`OpenOptions::open` → `spawn_blocking_with`）、`File::metadata`、
`set_len`、`metadata(path)`、所有目录操作。只有 read/write 真正进驱动。

它自带的那只池也不是免费的：`AsyncifyPool` 用 `bounded(0)` 会合通道，
`try_send` 失败且未达上限（**256**）时**现场 `std::thread::spawn` 一个新 worker**，
线程存活 60s（`compio-driver-0.12.5/src/asyncify.rs`）。这条路径就在
「每个文件一次 open」的必经之路上，§4.4 的 12.6 倍差距基本来自这里。

## 3. 方法（先把公平性钉住）

- **`threads` = 在途请求数**，不是线程数。窗口式后端「最老的完成就补一个」，
  rayon 靠工作窃取，chunked 直接切段；所以「在途数」被固定为同一个值，
  变化的只有机制——这正是完成式 IO 声称能提高、线程池不能提高的那个数。
- **句柄在计时外打开**（positional read 打在预热好的句柄上），否则
  `CreateFileW`（compio 是在阻塞池上做的）会淹没一次 4 KiB 读。
- **冷数据靠轮换窗口逼近**：Windows 没有用户态清 page cache 的手段，所以
  `*-big` 每次读 96 GiB 文件里的一段 32 GiB 窗口，窗口按 pass 整块轮换——
  同一个窗口要等另外两段（64 GiB）读完才回来，超过本机 39 GiB 内存，
  每个 pass 都从被驱逐的状态开始，后端之间才可比。
- **CPU 只在有意义时报告**：`GetProcessTimes` 以 15.625 ms 时钟滴答为单位，
  还可能把窗口之前开始的滴答算进来，所以短于「8 个滴答 × 线程数」的 pass 打印 `-`。
- **warmup 丢弃、median 取中间那个 pass 本身**（不是把不同 pass 的字段拼在一起）。
  冷负载不 warmup。
- **与产品同款分配器**：全局 `mimalloc`，与 `src-tauri/src/main.rs` 一致；仓库自己
  记录了「仅换分配器就让 35 821 条目目录从 48.3ms 降到 32.6ms」，测 IO 机制时
  不能同时换堆。
- 拷贝是唯一「后端做的系统调用不同」的负载：`fs::copy`（= `CopyFileExW`）对
  compio 没有等价物，所以 compio 有两个形状——`kernel`（同样调 `fs::copy`，
  只是放在它的阻塞池上，等于「务实移植会怎么写」）和 `readwrite`（在驱动上
  自己读进来写出去）。

## 4. Windows / ReFS / NVMe 实测（median，`bench/io-backends/results/`）

### 4.1 派发一个操作的成本（`dispatch`，100 000 个空 op）

| 在途 | tokio | tokio-chunked | rayon | compio |
|---|---|---|---|---|
| 1 | **220 k/s** (4.5 µs/次) | 156 M/s | 298 M/s | 12.9 M/s (0.08 µs/次) |
| 16 | 320 k/s | 286 M/s | 635 M/s | 17.7 M/s |

真实操作的成本（`perop`：每个 op 一次 1 字节 cached 读，20 000 个）：

| 在途 | tokio | tokio-chunked | rayon | compio |
|---|---|---|---|---|
| 1 | 127 k/s (7.9 µs/op) | 1491 k/s (0.67 µs) | 1486 k/s | 449 k/s (2.2 µs) |
| 16 | 197 k/s (5.1 µs) | 745 k/s (1.3 µs) | 736 k/s | **650 k/s (1.5 µs)** |

两件事同时成立：IOCP 的每次操作成本（1.5–2.2 µs）**低于** `spawn_blocking`
（5.1–7.9 µs），而且**随深度下降**（2.2 → 1.5），线程池则**随深度上升**
（7.9 → 5.1 也是它自己在抖，randread 里更明显）。但**最优解是「已经跑着的线程
里做掉」**（0.67 µs）——这正是应用现在的形状。

### 4.2 目录枚举：机制完全无关（`enumerate`，36 000 条目）

| 在途 | tokio | tokio-chunked | rayon | compio |
|---|---|---|---|---|
| 1 | 2.91 M entries/s | 3.10 M | 3.24 M | 3.29 M |
| 16 | 17.39 M | 17.80 M | 17.24 M | 17.42 M |

@1 差 ≤13%、@16 差 <4%（同一段代码，差异在噪声量级），且 compio 这一格跑的**就是同一段阻塞代码**（它没有异步目录迭代可用）。
这一格是「资源管理器的关键路径」——换机制在这里的收益是 0，不是「小」。

### 4.3 随机 4 KiB 读：共享句柄会自伤，修句柄比换机制有用

cached（8 GiB blob 全在 page cache 里，65 536 × 4 KiB）：

| 在途 | tokio | tokio-chunked | rayon | compio | 单句柄 |
|---|---|---|---|---|---|
| 1 | 339 | 1238 | 1240 | 799 | ← `randread` |
| 16 | 539 | **787** | **783** | **961** | 单句柄 |
| 64 | 523 | 773 | 754 | 968 | 单句柄 |
| 1 | 352 | 1234 | 1200 | 794 | ← `randread-multi` |
| 16 | 670 | **1468** | **1445** | 936 | 64 句柄 |
| 64 | 828 | 1429 | 1511 | 953 | 64 句柄 |

冷盘（96 GiB 文件，真打到设备）：

| 在途 | tokio | tokio-chunked | rayon | compio | |
|---|---|---|---|---|---|
| 1 | 25 | 1022 | 1036 | 630 | 单句柄 |
| 16 | 361 | 512 | 520 | **762** | 单句柄 |
| 16 | 755 | **1357** | **1399** | 734 | 64 句柄 |

两个读法都测是必要的：**单个共享句柄上多线程读，池子会越加越慢**
（1238 → 787），此时 IOCP 看起来赢了 1.5 倍——但那是自己给自己制造的瓶颈。
给每次在途读各自一个句柄之后，池子 1445–1468 反过来压过 compio 936。
（Tauri 命令本来就一个文件一个句柄，所以真实代码不受这条坑影响。）

### 4.4 小文件：换过去慢 12.6 倍（`smallfiles`，20 000 × 8 KiB 全读）

| 在途 | tokio | tokio-chunked | rayon | compio |
|---|---|---|---|---|
| 1 | 183 | 421 | 435 | 125 |
| 16 | 948 | **3602** | **3614** | **286** |

每文件：rayon 2.2 µs，compio 27.4 µs。差距不是「不幸运」，是 §2.2 的结构：
每次 open 一次阻塞池派发（还可能现场起线程）+ 一次 IOCP 往返。
**预览、缩略图、校验和、内容搜索全是这个形状。**

而 tokio 那一列的 948 MiB/s 里，**大头是派发本身**，不是 IO。把「一个阻塞调用携带
几个 op」做成参数（`--batch`，`run-batch-curve.sh`）就能把它量出来：

| 每次 `spawn_blocking` 携带 | wall（20 000 文件） | 每文件 | MiB/s |
|---|---|---|---|
| **1**（今天：一个文件一条命令） | 404 ms | **20.2 µs** | 386 |
| 2 | 206.5 | 10.3 | 757 |
| 4 | 107.9 | 5.4 | 1449 |
| 8 | 66.9 | 3.3 | 2338 |
| 16 | 46.3 | 2.3 | 3376 |
| **32** | 42.9 | **2.15** | 3639 |
| 64 | 45.1 | 2.25 | 3466 |
| 128 | 43.9 | 2.20 | 3561 |
| 256 | 44.2 | 2.20 | 3535 |
| 1250 = 每线程一次（`tokio-chunked`/rayon 的形状） | 42.8 | 2.14 | 3654 |

在途只有 1 时（前端一次只等一个预览）：batch=1 是 **54 µs/文件**，
batch=全部是 **14.5 µs/文件**（1081 ms → 291 ms）。

三条要点：

- **拐点在 batch 16–32，再大没有收益**：剩下的 2.1–2.2 µs/文件就是真正的
  `open` + 读 8 KiB + 分配缓冲。也就是说派发占掉了 20.2 µs 里的 ~18 µs。
- **倍数是 8×，但没有 IO 的功劳**：`batch=1` 今天在 8 个独立进程里稳定在
  358–460 ms（18–23 µs/文件）；同一配置在更早那轮 sweep 里是 165 ms
  （8.2 µs/文件）。**per-op 派发成本本身随机器状态浮动 2.5 倍**
  （C-state/调度/扫描类后台进程），而批量化的那一端在所有观测里都是
  42.8–46.3 ms（±5%）。所以按区间报：**3.6×–9×，稳定态约 8×**，
  顺带买到的是**可预测的延迟**。
- 这条曲线只给**派发**定价。句柄复用没有再单独量化（`perop` 的 1.88 µs/op 是
  「16 线程争用同一个句柄」的数字，不能与 `smallfiles` 直接相减）。

**收益正比于「每个文件真正的活有多少」**：文本预览这类每文件几 µs 的路径能吃满
8×；图片解码那种每文件几百 µs 的路径，省下的 18 µs 是 3% 的噪声——而缩略图
渲染本来就已经是「有界 worker 池 + 通道」（`preview.rs:379`），也就是批量形状。

### 4.5 顺序读：吞吐同级，冷读时 IOCP 的 CPU 便宜 6 倍

cached 8 GiB（4 MiB 块）：@1 5732 / 7344 / 7520 / **7683**；@16 6746 / 7161 / 7289 / 6552。

冷 32 GiB 窗口（`seqread-big`）：

| 在途 | tokio | tokio-chunked | rayon | compio |
|---|---|---|---|---|
| 1 | 2936 MiB/s (7.2s cpu) | 2779 (7.7s) | 2806 (7.1s) | 2002 (**1.6s**) |
| 16 | 2756 (9.2s) | 2055 (10.3s) | 2185 (10.2s) | **2845 (1.6s)** |

吞吐同一量级（设备带宽说了算），但 compio 的进程 CPU 只有 1.6s —— 因为
buffered overlapped read 的 memcpy 由缓存管理器的系统工作线程做，记在 System
进程上；同步 `ReadFile` 是在调用线程上下文里做，记在我们进程上。**这不是
「省掉了工作」，而是「工作换了个进程记账」**，吞吐也确实没掉。
对 GUI 应用这是个真实的、但很窄的好处：少抢核。

### 4.6 写入（4 GiB / 1024 个新文件，page cache 吸收）

| 在途 | tokio | tokio-chunked | rayon | compio |
|---|---|---|---|---|
| 1 | 3664 | 3930 | 3940 | 3417 |
| 16 | 8500 | 8946 | 7888 | 4427 |

同样是 compio-fs 的 open 走阻塞池（✕1024）。注意 4 GiB 这一档被 39 GiB
page cache 吸收，测的是提交+memcpy 成本；要测设备写带宽得加大每 pass 数据量。

### 4.7 拷贝：内核那一次调用赢过一切用户态循环

1 GiB / 4000 文件（median，MiB/s）：

| 形状 | 在途 | tokio | tokio-chunked | rayon | compio |
|---|---|---|---|---|---|
| `kernel`（`CopyFileExW`） | 1 | 1189 | 1394 | 1350 | 1041 |
| `kernel` | 16 | 1985 | **4266** | **4255** | 1984 |
| `readwrite`（驱动上自己读写） | 1 | 1282 | 1320 | 1256 | 614 |
| `readwrite` | 16 | 1989 | 4232 | 3905 | **1449** |

`fs::copy` 放在池子里最好（4266）；compio 不管哪个形状都更差——走自己的阻塞池
是 1984（每文件一次 Asyncify 派发），走驱动自己读写是 1449。**移植到完成式
运行时意味着放弃 `CopyFileExW`**，这是一个纯亏的决定。
（拷贝是全部负载里 pass 间波动最大的，pass 只有 ~0.5s。）

## 5. io_uring（WSL2 / ext4 on VHDX / virtio，compio 报告 `IoUring`）

| 负载 | 在途 | tokio-chunked | rayon | compio (io_uring) |
|---|---|---|---|---|
| `perop`（1 字节 cached 读） | 1 | 2282 k/s | 2148 k/s | 784 k/s |
| `perop` | 16 | 12246 k/s | 12287 k/s | 1605 k/s |
| `randread`（4 KiB cached，多句柄） | 16 | **31 519** | **33 694** | **3190** MiB/s |
| `randread` | 64 | 29 993 | 17 270 | 3286 |
| `smallfiles` | 16 | 10 422 | 12 438 | 1644 |
| `enumerate` | 16 | 4.21 M/s | 4.06 M/s | 4.24 M/s |
| `seqread`（4 MiB 块） | 1 | 18 155 | 18 112 | 17 886 |
| `copy`（readwrite） | 16 | 10 627 | 10 661 | 2127 |

- **cached 小 op 上 io_uring 明显更贵**：每个 op 要一次提交 + 一次回收，
  而且 compio 是 thread-per-core，单个运行时线程就是天花板（单线程 841k op/s ≈
  3.2 GiB/s），而 16 个线程的 pread 能吃满 `memcpy` 带宽（31–33 GiB/s）。
- **目录枚举同样无关**（4.0–4.2 M/s），顺序大块读同样同级。
- ⚠️ **表里的 tokio 基线在 WSL2 上不可信**：`dispatch`（空 op）只有
  14 k/s = **65 µs 一次 handoff**，而进程峰值线程数只有 6（不是每 op 起线程），
  裸 `pthread_create` 在这台 WSL2 上是 175 µs/次（`probe-linux-dispatch.sh`）。
  原生 Linux 上 `spawn_blocking` 的 handoff 是 1–3 µs 量级，这台机器把它放大了
  20 倍以上。所以 §5 只能读「机制之间」的相对关系，不能读「Linux 上 tokio 有多差」。
- WSL2 的 io_uring 语义与裸机并非逐项等价，本轮未核对。

## 6. 为什么 IOCP 帮不上这个应用（结构原因）

1. **枚举没有异步形态**：Windows 侧 API 就是同步的；Linux 侧 io_uring 有
   `IORING_OP_GETDENTS`/`statx`，但 compio-fs 没导出 `read_dir`，而且我们
   在 Windows 上跑——「关键路径」这一格连机会都没有。
2. **每 op 成本只在深队列里才被摊薄**：IOCP 每次操作 1.5–2.2 µs 并不低，
   它赢的是「不随深度变差」；而这个应用真正的 IO 是「一次命令、一个大循环」
   （`read_directory`、`copy_stream`、`transfer_pool`），在途数本来就低。
3. **阻塞池无法摆脱**：compio-fs 在 Windows 上把 open/metadata/dir 都交给阻塞池，
   而它那只池比应用现有池更贵（每文件一次派发、可能现场起线程、60s 线程存活）。
   于是「换成 IOCP」变成「保留阻塞池 + 加一层完成队列」。
4. **`CopyFileExW` 无可替代**：内核里一次调用、无用户态缓冲，任何异步循环都更慢。
5. **纯 CPU 的活不在这里**：仓库自己的测量显示，仅仅换分配器就有 -31% 的列举
   收益，列举行的成本大头在条目构造 + JSON/IPC，而不是 IO 机制。

## 7. 建议

**不做迁移。** 若之后要动 IO，按实测收益排序：

1. **小文件批量化**（收益最大的一格，也是唯一有量化曲线的）：把「N 个文件各自
   一次命令 + 一次 `spawn_blocking`」改成「一次阻塞调用携带 16–32 个文件」
   （§4.4：每文件 20.2 µs → 2.15 µs，**3.6×–9×**，拐点在 16–32，再大无收益；
   在途为 1 时 54 µs → 14.5 µs）。代价是首字节延迟变大，而应用已有的流式列举
   （`listing.rs` 的有界通道 + 512/8192 分批）就是这个形状，可以照搬。
   只对「每文件只有几 µs 活」的路径（文本预览、校验和、`stat_named_children`）
   有意义；图片解码那种每文件几百 µs 的路径不必动。句柄复用/LRU 这一条本轮
   没有单独量化，别按上面的数字推算。
2. **别在一个句柄上堆线程**（§4.3）：需要并发读同一文件时就每在途请求一个句柄；
   cached 场景下线程多反而更慢。
3. **拷贝继续用 `fs::copy` / 内核原语**（§4.7）。
4. **只在「CPU 与 UI 抢核」成为真问题时回访 IOCP**（§4.5 的 1.6s vs 9.2s），
   并且必须先确认吞吐不掉。

**什么条件下重新考虑这个结论：**

- 目标平台变成 Linux，且负载是元数据密集（io_uring 有真正的 async `statx`/
  `getdents`）——但需要在**裸机**上重跑本 harness，WSL2 的基线不可用。
- 出现「深度队列 + 每 op 昂贵」的真实负载（例如一次拖入 1000 个网络文件、
  或随机块级的增量哈希），且能接受把 `open` 也从阻塞池里拿出来。
- compio / 其他运行时的 Windows fs 层补齐了 open/metadata/dir 的异步实现
  （`compio-fs` 0.12.1 做不到，这与运行时无关，是 fs 层的事）。

## 8. 复现命令

```bash
cd bench/io-backends
cargo build --release

# 数据（约 105 GiB：8 GiB 常驻 blob + 96 GiB 冷 blob + 约 25 000 个小文件；幂等）
./target/release/io-backends prepare all --root K:/dae-io-bench

# 全量 sweep -> results/*.tsv（含 median 汇总表）
./run-all.sh K:/dae-io-bench
python summarize.py results

# 小文件批量化的曲线（§4.4）-> results/smallfiles-batch-*.tsv
./run-batch-curve.sh K:/dae-io-bench

# Linux / io_uring（WSL2 或裸机；把 crate 复制到家目录再编译）
bash run-linux.sh /mnt/k/code/dae/bench/io-backends

# WSL2 调度噪声的证据（每次 handoff 65 µs、pthread_create 175 µs）
bash probe-linux-dispatch.sh
```

`DAE_BENCH_COPY_MODE=kernel|readwrite` 切换 §4.7 的两种拷贝形状，
`--blob-mib` 可把 8 GiB 的常驻 blob 调小做 dry run。

## 9. 未验与风险

- 只在 **Windows/ReFS/NVMe** 与 **WSL2/ext4-virtio** 上测过，没有裸机 Linux。
- io_uring 只跑过 WSL2（内核 6.18.54.1-microsoft-standard-WSL2）。
- **per-op 派发成本随机器状态浮动**（§4.4）：同一份配置在本机测到过 8.2 µs/文件
  和 20.2 µs/文件两个稳定态，差 2.5 倍。讨论「每命令一次 `spawn_blocking` 要花
  多少」时必须带上这个区间；批量化的那一端没有这个问题（±5%）。
- 没有测：**io_uring 的 registered buffers / SQPOLL / 批量提交**——这几项才是
  io_uring 真正能翻盘的形态，需要另一套 harness；本轮测的是「逐 op 提交」，
  也就是把网络 IRQ 那套语义搬到文件上的默认用法。
- 没有测网络后端（SFTP/SMB/云）：它们本来就是异步 + 等待网络，本文结论不适用。
- 绝对数字依赖卷与缓存状态；跨机比较请重跑 `prepare` + `run-all.sh`。
- compio 0.19.2 / compio-fs 0.12.1 的具体行为可能随版本变化（尤其 §2.2）。

## 10. 相关文档

- [`README.md`](README.md) —— 决策记录索引
- `bench/io-backends/README.md` —— harness 的方法、负载表、注意事项
- `bench/io-backends/results/summary.md`、`results-linux/summary.md` —— 本页所有表格的原始产出
- `src-tauri/src/file_system/listing.rs`（流式列举）、`local/operations.rs`（`transfer_pool`）、
  `transfer.rs`（`copy_stream`）、`preview.rs`（渲染 worker）—— §1 的四条路径
