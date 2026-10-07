# 决策记录

这个目录存放**已经做过判断的技术决策**，以及支撑那些判断的实测证据。
每份文档都应当能回答：「当时为什么这么定」，以及「如果情况变了，凭什么重新考虑」。

| 文档 | 状态 | 一句话 |
|---|---|---|
| [`tab-drag-webview-reparent.md`](tab-drag-webview-reparent.md) | **已否决** | 标签拖动改成浏览器式 webview reparent —— 机制可行，但代价不值 |
| [`tab-tear-off-warm-pool.md`](tab-tear-off-warm-pool.md) | **草案，未实施** | 预热窗口池让撕出看起来即时 —— 能存下 ~78% 的等待，代价 +200MB |

## 两份文档的关系

它们解决的是**同一个问题的两条路**：标签撕出要等约 530ms 才有内容。

- **reparent** 想从根上避免重建 UI —— 被否决，因为 wry 的 reparent 是 `unstable` 特性、
  每个标签一个 webview 内存过高，而现有原生拖动已是 OS 级拖动，交互上几乎没有收益。
- **预热池** 接受「要重建 UI」，改为**把重建的成本提前付掉** —— 实测证明除绘制外
  都能在窗口可见之前完成。

所以 reparent 的否决**加强**了预热池这条路的理由，而不是削弱它。

## 关联：`WebKitWebProcess` 泄漏

`tab-drag-webview-reparent.md` §4.7 记录的泄漏（关闭标签窗口后子进程不退出，
wry 的 GObject 引用环，6 行 patch 曾在本地验证）是**当前架构的独立 bug**，
和上面两条路都无关，目前**记账但未排期**。

预热池与它有相互作用：池化会**减少**窗口的创建/销毁次数（对泄漏中性偏好），
但如果收养后反复关闭池窗口，泄漏会累积 —— 两者应当一起评估。

## 归档的 PoC

`poc/webview-reparent/` 存放 reparent 路线的实测代码与脚本，**不参与构建**
（`lib.rs` 里没有 `mod reparent_poc`，`Cargo.toml` 里没有 `unstable` feature）。
其中包括 `webview-memory.py`（进程树 RSS + PSS 测量），
预热池的内存预算应当用它重新测，而不是沿用文档里的旧数字。
