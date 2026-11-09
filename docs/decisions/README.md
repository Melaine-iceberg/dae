# 决策记录

这个目录存放**已经做过判断的技术决策**，以及支撑那些判断的实测证据。
每份文档都应当能回答：「当时为什么这么定」，以及「如果情况变了，凭什么重新考虑」。

| 文档 | 状态 | 一句话 |
|---|---|---|
| [`gtk-stack-pinned-by-upstream.md`](gtk-stack-pinned-by-upstream.md) | **已定案，等上游** | gtk/gio 被上游锁在 0.18 —— gtk 0.19 连依赖解析都过不去，实测证据在案 |
| [`completion-io.md`](completion-io.md) | **已实测否决，不改** | 文件 IO 换成 IOCP/io_uring —— 枚举无异步形态、小文件慢 12.6 倍、拷贝输给 `CopyFileExW`；harness 与原始数据在 `bench/io-backends/` |
| [`windows-directory-enumeration.md`](windows-directory-enumeration.md) | **已实测，已实施** | Windows 目录枚举补上 `FIND_FIRST_EX_LARGE_FETCH` —— 35,803 条目 24.2ms → **13.7ms**（-43%）、120,000 条目 -42%；首次绘制不变；harness 与原始数据在 `bench/listing-pipeline/` |
| [`listing-shell-costs.md`](listing-shell-costs.md) | **审计完成；第 1 项、§4 watcher 补丁、§5 批量选择/前缀、§6 选区表示、§10 可见视图按需、完成/交互式排序移出线程、终端输出按帧合并已实施，§7 次级清单未动** | 列表外壳的剩余开销 —— 枚举已到位（35,803 条 12.64 ms / 120,000 条 50.71 ms）；已修：完成时的整段重排（120,000 条 GUI 线程 **147.5 → 34.0 ms**，release 单测）、逐行拼路径的 `index_of_path`、watcher 变更按名字补丁（app 内冒烟）、全选/Shift/框选/前缀输入的逐行跨 FFI（app 内冒烟）、选区进模型（delegate 读 `selected` 角色、`dataChanged` 只发变化的行、计数是属性、框选与历史恢复在模型里做；app 内冒烟）；§9 复测后：隐藏视图不再随 reset 重建（341 → 165 个 delegate），≥2048 行的完成排序移到 worker（120,000 条单轮 GUI 阻塞 94.5 → 13.0 ms），交互式改排序同样移到 worker（12 万条 60–100 → 单轮 12.8 ms），终端输出按帧合并（10 万行 CPU 3.09 → 2.30 s）；未做：§7 的次级清单 |
| [`p1-integrations.md`](p1-integrations.md) | **已实施（P1）** | 驱动器动作自己做、格式化交系统；自定义动作走 argv 不走 shell；状态中心进程内记录 200/持久化 50；会话存路径与几何不存选区；RAR/ISO 只走外部工具；FTP/FTPS 用 8 连接池避免同服务器复制自锁；WebDAV 可读可改但上传缺 `finish()` 钩子 |
| [`file-tags-storage.md`](file-tags-storage.md) | **已实施（P0）** | 标签两层存储：文件流（ADS/xattr）负责随文件移动，分片 JSON 索引负责 `tag:` 搜索/计数/排序；卷能力探测一次，失败即索引模式并告知用户 |
| [`tag-index-integrity.md`](tag-index-integrity.md) | **已实施（2026-11-07）** | 索引条目记文件身份（Windows 卷序列号+文件索引，Unix `dev:ino`）：dae 内改名直接迁移，dae 外改名在读取时找回，路径复用不继承旧标签；重建索引只遍历索引、损坏分片隔离；定义导入按 id 合并 |
| [`search-index.md`](search-index.md) | **已实施（2026-11-07）；MSIX 形态未验** | 平台索引只做候选源：名字与过滤器仍由遍历那套规则裁；Windows 走 ISearchQueryHelper + CollatorDSO（列是 VARIANT，取 ItemUrl），Linux 走 tracker3/baloo；失败即退回遍历 |
| [`network-discovery.md`](network-discovery.md) | **已实施（2026-11-07）** | 侧栏网络段用 1.5 s 一次性 mDNS 探测而不是常驻守护；只取实例名/主机/端口，不读 TXT（广告方写的东西不进对话框）；按协议+主机+端口去重，失败即空表 |
| [`sftp-host-key-tofu.md`](sftp-host-key-tofu.md) | **已实施，未对真实 sshd 验证** | SFTP 之前接受任何主机密钥 —— 改为按用户 `~/.ssh/known_hosts` 的 `accept-new`：首次记录、变更即拒、记不下来也拒 |
| [`transfer-control.md`](transfer-control.md) | **已实施（2026-11-09）** | 暂停/取消/限速挂在操作上而不是窗口上；本地 `fs::copy` 不可中断，故暂停在条目边界生效，只有设了限速才走 256 KiB 分块；续传判据是「长度相同且目标不旧于源」，单文件字节续传留给下一步 |
| [`permission-failures.md`](permission-failures.md) | **已实施（2026-11-09），macOS 未真机验证** | 提权的是**一次操作**不是应用（job 文件 + `--elevated-job` 子进程，job 限定在用户配置目录内）；「拒绝访问」的另一半答案是占用者查询，Windows 走 Restart Manager、Linux 扫 `/proc/*/fd`、macOS 用 `lsof`，只接到错误文案上 |
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
