# 传输队列的多槽化：槽位归 Job，冲突流独立

> 日期：2026-11-18 · 状态：**已实施**
> 对象：`crates/dae-core/src/fs/jobs.rs`、`crates/dae-qt/src/workspace.rs`、
> `qml/TransferConflictDialog.qml`、`qml/TransferQueue.qml`

本文记录传输队列从「两个运行槽 + 一个全局预检位」改为「四个整 Job 槽位」的
边界：槽位从什么时候算占用、冲突对话框如何同时服务多个 Job，以及为什么
没有走 KIO Scheduler 的按目标序列化。

---

## 0. 结论

| 问题 | 决定 |
|---|---|
| 槽位归谁 | 一个槽 = 一个 Job 的全程：冲突检查、等回答、跑拷贝，直到回报。`MAX_SLOTS = 4` |
| 预检串行 | 取消。任何一个 Job 的慢检查或没人回答的对话框只占一个槽，不再挡住整个队列 |
| 冲突对话框 | 每 Job 独立：壳层把各 Job 的 prompt 排成 FIFO，屏幕上永远是队头那份；回答与关闭按 `job_id` 寻址 |
| 队列面板 | 预检中的行（「准备中」）的 ✕ 只 dismissal 那一个 Job；其它 Job 的问题不受影响 |

## 1. 动机

旧调度器的接受条件是「有空槽 **且** 所有已接受 Job 都在运行」。也就是说预检
阶段（冲突检查 → 冲突对话框）是全局唯一的：一个 SFTP 大树的冲突检查要跨网走
完，或者一个对话框没人理，后面所有本地拷贝就都堵在等待队列里——本地盘明明
闲着。这正是 Directory Opus 与 KIO Scheduler 都不存在的问题形态：它们眼里
并行单位是「一次操作」，不是一个阶段。

多槽并行的基础设施其实早已就绪：core 按操作 id 注册 `TransferControl`，限速
是进程级的令牌桶（`transfer-control.md` §5），进度按行渲染。缺的只有两件事：
壳层放开槽位（不再假设「一次只有一个待答 Job」），以及每个 Job 一条独立的
冲突流。

## 2. 槽位语义：一个 Job 一个槽

- `State::can_accept` 只看 `jobs.len() < MAX_SLOTS`。Job 从被接受到回报（跑完、
  失败、被 dismissal）都占着槽位。
- `answer(job_id, action, apply_to_all)` 与 `dismiss(job_id)` 按 id 寻址；对
  运行中或不存在的 Job 是 no-op，这消化掉「对话框落后队列一帧」的竞态。
- 旧的 `pending_id()`（找「唯一非运行 Job」）删除：它在新模型里没有意义，
  两个 Job 同时待答是常态。

`MAX_SLOTS = 4` 而不是原来的 2：动机场景是「一条慢 SFTP + 若干本地拷贝」。
2 的话本地拷贝只剩一条通道；4 让慢链路占一槽、三条本地并行，而真正的节流
由令牌桶做，不由槽位数做。不设为可配置：目前没有用户可表述的诉求，先不改
设置面。

## 3. 壳层：每 Job 一条冲突流

core 的 `ConflictPrompt` 本来就带 `job_id`；旧壳层把它渲染进一组全局
`conflict_*` 属性，等于假设同一时刻只有一个提问。新壳层：

- 每个 prompt 到达时把 payload（名字、两侧卡片文案、index/total）渲染一次，
  存进 `pending_conflicts`（FIFO，按 prompt 首次到达排序）；同一 Job 的下一
  个冲突原地覆盖。
- `show_conflict_head()` 永远从队头刷对话框；`conflictJobId` 告诉 QML 当前问
  的是谁，`conflictWaiting` 是排在后面的 Job 数，对话框据此印一行「回答后还有
  N 个传输等待确认」。
- 回答走 `resolveConflict(jobId, …)`，dismissal 走 `cancelConflict(jobId)`
  ——队列面板「准备中」行的 ✕ 传该行自己的 id，对话框的 Esc/遮罩传
  `conflictJobId`。core 对这两种来源不做区分：都是按 id 寻址。
- 「应用到全部」的勾在 `conflictJobId` 变化时清零：它是针对一个 Job 一批
  冲突的答案，不能跨 Job 带走。

对 Job 结束的三条路（`Started`、`CheckFailed`、dismissal）都会把它从 FIFO
里摘掉并刷新队头，所以对话框永远呈现下一个待答 Job，而不是消失后凭空再现。

## 4. 为什么没有学 KIO 的「按目标序列化」

KIO Scheduler 还有一层我们没做：同一目标的 Job 串行、不同目标并行。没做的
理由：目标字符串相同不代表物理设备相同（两个 `C:/…` 也可能争同一块盘），
反之亦然；做对它需要挂载/设备映射，而 FIFO + 令牌桶已经覆盖本改动的全部
动机（慢链路不再堵队列）。如果将来实测发现「同盘 4 并发互相踩踏」，正确的
下一步是按 `vfs` 的后端/主机分组限并发，而不是撤回多槽。

## 5. 明确没有做的

- **可配置的槽位数。** 没有用户表述得出来的诉求之前不加设置项；改起来只是
  把 `MAX_SLOTS` 换成读 `settings.toml`。
- **对话框并行。** 屏幕上仍是一次一个问题（FIFO 队头），Opus 也如此：两个
  冲突对话框叠在屏幕上不是并行，是噪音。真正并行的是检查与回答的**流水**。
- **取消正在检查的 Job 的回调撤销。** 检查回调回到 core 时 Job 已被
  dismissal，`adopt_conflicts` 找不到 Job 直接丢弃，无需撤销机制。
