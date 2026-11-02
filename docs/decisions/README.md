# 决策记录

这个目录存放**已经做过判断的技术决策**，以及支撑那些判断的实测证据。
每份文档都应当能回答：「当时为什么这么定」，以及「如果情况变了，凭什么重新考虑」。

| 文档 | 状态 | 一句话 |
|---|---|---|
| [`gtk-stack-pinned-by-upstream.md`](gtk-stack-pinned-by-upstream.md) | **已定案，等上游** | gtk/gio 被上游锁在 0.18 —— gtk 0.19 连依赖解析都过不去，实测证据在案 |
| [`completion-io.md`](completion-io.md) | **已实测否决，不改** | 文件 IO 换成 IOCP/io_uring —— 枚举无异步形态、小文件慢 12.6 倍、拷贝输给 `CopyFileExW`；harness 与原始数据在 `bench/io-backends/` |
| [`windows-directory-enumeration.md`](windows-directory-enumeration.md) | **已实测，已实施** | Windows 目录枚举补上 `FIND_FIRST_EX_LARGE_FETCH` —— 35,803 条目 24.2ms → **13.7ms**（-43%）、120,000 条目 -42%；首次绘制不变；harness 与原始数据在 `bench/listing-pipeline/` |
| [`sftp-host-key-tofu.md`](sftp-host-key-tofu.md) | **已实施，未对真实 sshd 验证** | SFTP 之前接受任何主机密钥 —— 改为按用户 `~/.ssh/known_hosts` 的 `accept-new`：首次记录、变更即拒、记不下来也拒 |
| [`engine-qt-quick.md`](engine-qt-quick.md) | **方向已定，四组门槛全过（§5.1–§5.4）；Qt 壳已覆盖浏览/标签/终端/搜索/预览/压缩包/文件图标（§4.3–§4.7）；图标接入后 12 万行复测通过（§5.1.1）** | 引擎从系统 WebView 换成 Qt Quick（QML 视图 + cxx-qt 的 Rust 后端）—— 结构性理由：每 tab ~200 MB PSS、WebView2 占冷启动 63–65%；12 万行列表实测工作 p95 最差 5.68 ms、PSS 峰值 123.7 MB（全 CJK）；画真实图标后同会话 A/B 的 work p95 1.60–1.66 ms、PSS +3–4 MB；IME 214 项检查全过；无边框 + Mica + 自绘标题栏 63 项检查全过；第二个窗口撕出最差 76.82 ms、PSS 增量最差 13.74 MB（热态 5.1–6.2 MB）、关闭后私有提交不增长；Qt 壳已跑通真实配置/列举/标签撕出/终端/搜索/预览/压缩包/回收站/设置，图标与系统缩略图走 `dae-core`；连接管理、深链、更新链路与旧壳退役已在 §9.13–§9.17 记完（MSIX 版「设为默认文件管理器」已知不生效，§9.15）。**迁移收尾：`main` 即 Qt 壳，Tauri 壳留档在 `tauri` 分支（§9.18）** |

`gtk-stack-pinned-by-upstream.md` 与其余各份**无关**：它记录的是**依赖升级遇到的上游阻断**，
不是方案取舍 —— gtk/gio 0.18 今天仍是 `dae-core` 在 Linux 上取主题图标的依赖。
`completion-io.md` 是一次**被实测否决的 IO 机制迁移**，结论、方法与原始数据都自洽。
`windows-directory-enumeration.md` 与它相邻但结论相反，并不矛盾：
后者问的是「换**派发机制**值不值」（不值），前者做的是「同一套机制下减少**系统调用次数**」
（值，-43%）—— 后者结尾自己给出的建议就是「把力气花在减少系统调用次数、批量读上」。

## 已归档：WebView（Tauri）壳时期的记录

下列内容描述的是**已退役的 WebView 壳**：该壳于 2026-11-01 退役、`main` 转为 Qt 壳，
这批记录随后于 2026-11-02 从 `main` 移除，留档在 `tauri` 分支。

- `cold-start-js.md`、`boot-screen-prerender.md`、`first-mount-cost.md` —— 冷启动归因，
  以及预渲染与 React 首次挂载这两项被实测否决的优化
- `tab-drag-webview-reparent.md`、`tab-tear-off-warm-pool.md`、`poc/webview-reparent/` ——
  标签撕出的两条路线：被否决的 webview reparent 与已实施的预热窗口池
- `slice-8-core-extraction.md` —— 把文件系统搬进 `dae-core` 的施工单、验收记录与踩坑清单
- `performance-opportunities.md` —— WebView 前端的性能机会清单与复现命令
- `bench/cold-start/`、`bench/frontend-scans/` —— 上述两份文档依赖的 harness

`engine-qt-quick.md` 仍然引用其中的实测数字（每 webview +190–200 MB PSS、预热池
+200 MB、WebView2 占冷启动 63–65%），作为「为什么离开系统 WebView」的论据来源；
要复核这些数字，到 `tauri` 分支取对应的文档与 harness。
