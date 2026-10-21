# 决策记录

这个目录存放**已经做过判断的技术决策**，以及支撑那些判断的实测证据。
每份文档都应当能回答：「当时为什么这么定」，以及「如果情况变了，凭什么重新考虑」。

| 文档 | 状态 | 一句话 |
|---|---|---|
| [`tab-drag-webview-reparent.md`](tab-drag-webview-reparent.md) | **已否决** | 标签拖动改成浏览器式 webview reparent —— 机制可行，但代价不值 |
| [`tab-tear-off-warm-pool.md`](tab-tear-off-warm-pool.md) | **已实施，默认开启** | 预热窗口池让撕出看起来即时 —— 撕出中位从 ~530ms 降到 **~65ms**，代价 +200MB |
| [`gtk-stack-pinned-by-upstream.md`](gtk-stack-pinned-by-upstream.md) | **已定案，等上游** | gtk/gio 被上游锁在 0.18 —— gtk 0.19 连依赖解析都过不去，实测证据在案 |
| [`completion-io.md`](completion-io.md) | **已实测否决，不改** | 文件 IO 换成 IOCP/io_uring —— 枚举无异步形态、小文件慢 12.6 倍、拷贝输给 `CopyFileExW`；harness 与原始数据在 `bench/io-backends/` |
| [`windows-directory-enumeration.md`](windows-directory-enumeration.md) | **已实测，已实施** | Windows 目录枚举补上 `FIND_FIRST_EX_LARGE_FETCH` —— 35,803 条目 24.2ms → **13.7ms**（-43%）、120,000 条目 -42%；首次绘制不变；harness 与原始数据在 `bench/listing-pipeline/` |
| [`sftp-host-key-tofu.md`](sftp-host-key-tofu.md) | **已实施，未对真实 sshd 验证** | SFTP 之前接受任何主机密钥 —— 改为按用户 `~/.ssh/known_hosts` 的 `accept-new`：首次记录、变更即拒、记不下来也拒 |

第三份（`gtk-stack-pinned-by-upstream.md`）与上面两份**无关**：它记录的是**依赖升级遇到的上游阻断**，
不是标签撕出的方案取舍 —— 下面各节只涉及前两份。
`completion-io.md` 与上面三份都无关：它是一次**被实测否决的 IO 机制迁移**，结论、方法与原始数据都自洽。
`windows-directory-enumeration.md` 与 `completion-io.md` 相邻但结论相反，并不矛盾：
后者问的是「换**派发机制**值不值」（不值），前者做的是「同一套机制下减少**系统调用次数**」
（值，-43%）—— 后者结尾自己给出的建议就是「把力气花在减少系统调用次数、批量读上」。

## 两份文档的关系

它们解决的是**同一个问题的两条路**：标签撕出要等约 530ms 才有内容。

- **reparent** 想从根上避免重建 UI —— 被否决，因为 wry 的 reparent 是 `unstable` 特性、
  每个标签一个 webview 内存过高，而现有原生拖动已是 OS 级拖动，交互上几乎没有收益。
- **预热池** 接受「要重建 UI」，改为**把重建的成本提前付掉** —— 实测证明除绘制外
  都能在窗口可见之前完成。该路线已实施：`tab_windows.rs` 的池与收养分支 + 设置里的
  `warm_tab_pool`（默认开），收养成本实测中位 ~65ms。冷路径保留为回退。

所以 reparent 的否决**加强**了预热池这条路的理由，而不是削弱它。

## 关联：`WebKitWebProcess` 泄漏

`tab-drag-webview-reparent.md` §4.7 记录的泄漏（关闭标签窗口后子进程不退出，
wry 的 GObject 引用环，6 行 patch 曾在本地验证）是**当前架构的独立 bug**，
和上面两条路都无关，目前**记账但未排期**。

预热池与它有相互作用：池化会**减少**窗口的创建/销毁次数（对泄漏中性偏好），
但如果收养后反复关闭池窗口，泄漏会累积 —— 两者应当一起评估。
池化落地后这条相互作用仍然成立，上限是「池常驻 1 个 = +200MB」。

## 归档的 PoC

`poc/webview-reparent/` 存放 reparent 路线的实测代码与脚本，**不参与构建**
（`lib.rs` 里没有 `mod reparent_poc`，`Cargo.toml` 里没有 `unstable` feature）。
其中包括 `webview-memory.py`（进程树 RSS + PSS 测量），
预热池那条 **+200MB** 的内存数字（`tab-tear-off-warm-pool.md` §3.5）应当用它重新测，
而不是沿用文档里的旧机器数字；延迟数字已是生产构建实测，不受此影响。
