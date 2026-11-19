# 对照主流文件管理器的剩余缺口与实施计划

> 日期：2026-11-19 · 状态：规划稿；落地记录随实施追加
> 对象：`apps/dae` + `crates/dae-core` + `crates/dae-qt` + `qml/`
> 对照：Windows 11 Explorer、macOS Finder、KDE Dolphin、Total Commander、TeraCopy、yazi
> 前提：与 `docs/files-parity.md` 相同——Windows / Linux 交付，macOS 只保证编译

`files-parity.md` 对照单一应用（Files v4），§12.1 功能项已清空。本文是下一轮缺口的来源：
以 2026-11-19 的**代码**为准重新盘点（文档旧描述不作依据），对照多家文件管理器找出还值得借鉴的项。
本文是规划稿，不是决策记录；落地时需要取舍的判断按 `docs/decisions/` 格式另立。

---

## 0. 结论

盘点结论先说：dae 在视图（列/网格/分栏/双窗格/每目录布局）、预览（Quick Look、Office、
媒体、文本按内容判定）、传输（并发槽位、暂停/恢复、全局限速、冲突矩阵、撤销重做）、
搜索（USN 索引、内容搜索、保存搜索）、终端（内嵌 PTY + OSC 7 cwd 上报）上已经对齐或
超过对照对象。剩余缺口集中在**列表操作效率**与**远端/容器一致性**两类，按投入产出排序：

| 档 | # | 缺口 | 量 | 借鉴对象 |
| --- | --- | --- | --- | --- |
| P1 | G-1 | 当前目录即时名称过滤（无 `filterText`，只有属性芯片） | S | Explorer 11 列头搜索框 |
| P1 | G-2 | 键盘范围选择：方向键无显式处理，Shift+方向不存在（Shift+点击已有） | S | Finder / Total Commander |
| P1 | G-3 | 传输无 ETA（只有瞬时速率） | S | TeraCopy |
| P2 | G-4 | 压缩包不能当目录浏览（`Scheme` 无 archive、`vfs.rs` 无 archive 后端） | M | Dolphin KIO `archive:/` |
| P2 | G-5 | 远程路径缩略图禁用（`thumbnail_url(remote)` 返回空）、远程搜索拒绝 | M | Dolphin（远程按需取） |
| P2 | G-6 | 续传是条目级截断写，无字节区间续传 | M | TeraCopy |
| P3 | G-7 | 地址栏无最近路径下拉、无 `~`/环境变量展开 | S | Dolphin |
| P3 | G-8 | 预览无十六进制视图 | S | Finder Quick Look 的类别分发 |
| P3 | G-9 | 终端无「跳到当前目录」显式动作 | S | Dolphin F4（用户主动，非自动跟随） |

维持不做（不因本文重开）：MTP/便携设备、紧凑覆盖窗、多窗格 >2、系统通知——
依据 `files-parity.md` §12.4。Explorer 标签互操作同样维持不做。

---

## 1. 判据

沿用 `files-parity.md` §1：跨平台优先（Windows/Linux 都可用或明确降级）、能力探测、
热路径不变（列表枚举不加逐条系统调用，`docs/decisions/listing-shell-costs.md`）、
可回退、可测量。另加一条本文特有的：

- **不双轨**：新过滤、新键盘行为必须走既有筛选管线与既有选择管线，不加第二套
  状态；Finder/Dolphin 的差异是交互层差异，不是数据层差异。

---

## 2. 现状证据（2026-11-19 盘点）

只列与缺口直接相关的证据，全量盘点不重复。

| 域 | 现状 | 证据 |
| --- | --- | --- |
| 过滤 | 隐藏/类型/修改时间/大小四类芯片，无名称过滤输入 | `qml/FilterMenu.qml:25-50`、`crates/dae-qt/src/model.rs:288,3117` `set_filters` |
| 键盘 | 52 个可重绑定动作、type-ahead 定位；列表/网格无 `Qt.Key_Up/Down` 分支，方向键只靠 `ListView/GridView` 默认移动 `currentIndex`，选择不随动；Shift+点击范围选择已有 | `qml/FileList.qml:833-875`（Shift+点击）、`FileGrid.qml:283-310`；两文件全文无方向键处理 |
| 传输 | 每行 `percent` + 瞬时 `speed`、整体合并速率；暂停/恢复/取消、全局限速、冲突 Replace/Skip/KeepBoth 均有；无 ETA | `crates/dae-qt/src/workspace.rs:2433-2450` `publish_transfers`、`:4409` `format_speed` |
| 压缩 | 创建/解压 ZIP/TAR/TAR.GZ/7Z + 密码，但不能像目录一样打开 | `crates/dae-core/src/fs/archive.rs`、`url.rs` `Scheme` 无 archive |
| 远程 | SMB/SFTP/FTP/WebDAV/云走统一 `FileSystemBackend`，但缩略图、搜索、终端、解压目标在远程全部禁用 | `model.rs:4672-4675` `thumbnail_url`、`search.rs:19-70`、`TerminalPanel.qml`（`!remoteLocation`） |
| 续传 | 取消重试按条目合并（跳过已落地项），写侧为截断写 | `jobs.rs:48-49,231-237`、`vfs.rs` `open_write` 注释「Creates (or truncates)」 |
| 地址栏 | 面包屑/编辑双态、子目录补全；无最近路径、无 `~` 展开 | `qml/Locator.qml:84-110,587-653` |
| 预览 | 文本/MD/Office/媒体/缩略图（PDF 首页帧）；无十六进制 | `crates/dae-qt/src/preview.rs` |
| 终端 | OSC 7/9/1337 上报 cwd；策略明确「绝不在运行中的 shell 注入 cd」 | `TerminalPanel.qml:163-167`、`pty.rs:126,154` |

---

## 3. P1：三项小改动（先做）

### 3.1 G-1 当前目录即时名称过滤

- **借鉴**：Explorer 11 在列头/命令栏放搜索框，输入即过滤当前视图；Nautilus 的
  过滤弹出同理。共同语义：**作用于当前目录的已枚举行**，不触发遍历，不改排序分组。
- **落点**：
  - 后端：`crates/dae-qt/src/model.rs` 的 `set_filters` 旁边加名称过滤参数
    （大小写不敏感子串，空串即清空），过滤在既有筛选管线里做，不新增第二套行集。
  - 前端：`qml/FilterMenu.qml` 顶部加输入框；激活芯片栏显示过滤词 chip 可一键清空。
  - 快捷键：不新增可重绑定动作前先用 `Mod+F` 之外的方案——**结论在实现时定**：
    若占用全局搜索焦点键则不可接受，倾向进入 FilterMenu 后即聚焦输入框。
- **边界（不做）**：不做正则、不做通配符、不做跨目录（跨目录走既有搜索，
  两者语义不同，不合并 UI）。
- **验收**：10 万条目目录输入过滤词，行数即时收缩且首屏无卡顿（复用
  `bench/listing-pipeline` 的既有口径）；清空过滤恢复全部行；与分组节头共存正确。

### 3.2 G-2 键盘范围选择

- **借鉴**：Finder / Total Commander：方向键移动光标并单选；Shift+方向从锚点扩展；
  Ctrl+方向移动光标不动选择。锚点语义与既有 Shift+点击一致（`handleClick`）。
- **落点**：
  - `qml/FileList.qml`、`qml/FileGrid.qml` 各自的键处理处补 Up/Down/PageUp/PageDown/
    Home/End × {无修饰、Shift、Ctrl} 九种组合；锚点复用 Shift+点击的锚点变量，
    不引入第二套锚点状态。
  - Ctrl+方向只动 `currentIndex`（type-ahead 之外的纯光标移动）。
  - 分栏视图 `ColumnView.qml` 的方向键进入/回退语义保留，本轮不改。
- **边界（不做）**：不做 Ctrl+方向「跳词」（文件列表无词概念）；Shift+点击与
  Shift+方向混用时的锚点以最后一次操作为准。
- **验收**：列表与网格里 Shift+Down/Up 扩展/收缩、Ctrl+Down 不改选择、
  Shift+PageDown 跨屏扩展；操作后 Ctrl+C 复制的内容与视觉选择一致。

### 3.3 G-3 传输 ETA

- **借鉴**：TeraCopy 对运行中任务显示剩余时间，估算基于**平滑速率**而非瞬时速率。
- **落点**：
  - `crates/dae-qt/src/workspace.rs` `publish_transfers`：速率从瞬时差值改 EMA
    （或新增 EMA 字段保留瞬时值），`eta = 剩余字节 / EMA 速率`，低速阈值下
    （< 1 MiB/s）显示「—」避免倒计时抖动。
  - `qml/TransferQueue.qml` 行内在速率旁显示「剩余 mm:ss / h m」。
  - 三语文案（`en` / `zh-CN` / 第三语与现有 i18n 一致）。
- **边界（不做）**：不做「基于历史目录大小」的预测；排队任务不显示 ETA。
- **验收**：单测覆盖速率采样→EMA→ETA 换算（含低速阈值与除零）；运行冒烟里
  大文件传输行上 ETA 单调递减、暂停时冻结。

---

## 4. P2：两项一致性（P1 完成后排期）

### 4.1 G-4 压缩包当目录浏览

- **借鉴**：Dolphin KIO 的 `archive:/`——压缩包在 VFS 层就位，浏览、打开、拖出
  都与目录同一条代码路径。dae 的 `FileSystemBackend` trait（`vfs.rs:59-160`）已经
  把 `read_dir/stat/open_read` 抽象好，缺的只是 archive scheme 与后端实现。
- **步骤**：`url.rs` 增 `Archive` scheme 与 `archive:<压缩包路径>!/<内部路径>` 形态
  → 实现 `FileSystemBackend`（ZIP/TAR 原生，7z/RAR 沿用既有外部工具边界）→
  侧栏/面包屑显示压缩包层级 → 只读（写回、增量修改维持不做，与 KIO 侧重浏览一致）。
- **验收**：打开 `.zip` 双击进入，内层文本可 Quick Look、可直接拖出复制到本地；
  加密包沿用 `ArchivePasswordDialog`；`archive:` 路径可作标签页路径、重开后恢复。
- **决策点（实现时另立决策文档）**：scheme 语法用 `!`（Dolphin 同款）还是查询参数；
  大包是否列目录摘要而非逐条解压索引。

### 4.2 G-5 远程缩略图

- **借鉴**：Dolphin 对远程目录按需拉取缩略（小文件直接下、大文件先占位）。
  dae 现状是 `thumbnail_url` 对远程一律返回空——能力上的省略，不是性能取舍。
- **步骤**：`crates/dae-qt` 缩略图提供器对远程路径走 `open_read` 有界读取
  （图像上限沿用本地策略）→ 结果缓存走既有缩略图缓存，键含后端与凭据域
  → 失败静默降级为图标（与本地缩略失败一致）→ SFTP/WebDAV 先行，SMB 按带宽
  排后。
- **边界（不做）**：远程视频帧、远程内容搜索（后者维持 `search.rs` 的拒绝语义，
  理由是远程遍历成本，单独立项再谈）。
- **验收**：SFTP 目录中图片行显示缩略图；断网/鉴权失败回退图标不阻塞列表。

### 4.3 G-6 字节级续传

- **借鉴**：TeraCopy 的断点续传是**字节区间**的：写侧记录已落盘 offset，重试从
  offset 继续。dae 现状是条目级（跳过已完成条目），单个大文件中断即整文件重来。
- **步骤**：`transfer.rs` 目标侧写前 `stat` 取已存在大小 → 与源侧按块对齐续写
  （append 而非 truncate）→ 冲突判定：目标大小 > 源大小时视为损坏，回退截断写
  → 状态中心的重试入口沿用。
- **决策点**：只对 `resume` 标记的任务生效，正常覆盖复制仍截断写；一致性校验
  （hash 对比）维持不做。
- **验收**：中断大文件复制→重试→目标文件字节完整且耗时 ≈ 剩余部分；中途断电
  场景由单测模拟（半写文件 + 重试路径）。

---

## 5. P3：小项（按需）

- **G-7 地址栏**：最近路径下拉（`Locator.qml` 编辑态补全源加历史，去重、上限 8）
  与 `~`/`%VAR%`/`$VAR` 展开（展开失败按字面处理并保留输入）。落点
  `Locator.qml:587-653`、`settings.rs` 的历史读取。
- **G-8 十六进制预览**：`preview.rs` 按类别分发加一档：既非文本、又命中
  `may_have_text_preview` 快速拒绝的文件显示前 4 KiB hex dump（地址列 + ASCII 列）。
  不做分页与编辑。
- **G-9 终端跳转**：一次性显式动作（菜单/快捷键），向当前会话**作为键盘输入**
  发送 `cd -- <quoted>` 行，不自动跟随、不改 OSC 上报逻辑。与
  `TerminalPanel.qml:163-167` 的「不注入 cd」不冲突：那是自动跟随，这是用户
  主动请求。快捷键进可重绑定动作表（R-6 的注册表），默认绑定 F4 之外的空位。

---

## 6. 实施顺序

1. P1 三项（G-1/G-2/G-3）：纯增量，互不依赖，可并行；每项独立验收。
2. G-4 archive：VFS 后端，动 `url.rs`/`vfs.rs`，需要决策文档先行。
3. G-5 远程缩略图、G-6 字节续传：互不依赖，按带宽风险 G-5 先行。
4. P3 按需排入。

## 7. 落地记录

### 7.1 P1 三项（2026-11-19）

**G-1 当前目录即时名称过滤**

- **偏好**（`crates/dae-qt/src/preferences.rs`）：`nameFilter` 自由文本字段，
  与 `kindFilter` 等同语义——`setString("nameFilter", …)` 写入、`resetFilters()`
  一并清除、`explorer-qt.ini` 持久化；入口清洗 trim + 128 字符截断
  （`sanitize_name_filter`）。
- **模型**（`crates/dae-qt/src/model.rs`）：`RowFilter` 增 `name: Option<String>`
  （空串折叠为 `None`，故去 `Copy`）；`keeps()` 走 `ascii_fold_contains`——
  逐字节窗口扫描，ASCII 折叠、非 ASCII 原样（Windows 名称匹配规则，CJK 无大小写），
  不在枚举热路径上分配小写副本。`setFilters` 桥签名加第 5 参；QML 两个调用点
  （`FileList.qml`、`ColumnView.qml`）与 `onNameFilterChanged` 监听同步补齐。
- **UI**（`qml/Menu.qml`、`qml/FilterMenu.qml`）：`Menu` 增加可选 `header` 带
  （与 `footer` 镜像：Loader 顶贴、MenuList 让位、面板高度计入）。
  筛选菜单顶部是名称输入带：打开即自动抢焦点（`show()` 已排队 `list.takeFocus`，
  输入带用二次 `Qt.callLater` 反超）；输入去抖 300 ms 后写偏好（一个词不触发
  四次重载）；Enter 立即应用并关菜单；Esc 只关菜单（过滤保持，状态芯片亮起，
  菜单重开时回显）；「×」清空；Down 把键盘还给菜单行（`Menu.focusRows()`）。
  **快捷键结论**：不占全局键——菜单经状态栏「筛选」芯片与「筛选…」菜单项可达。
- **验收**：单测 `a_name_filter_matches_ignoring_ascii_case`（大小写、空串、
  CJK、非匹配）；Windows 冒烟（隔离 APPDATA/LOCALAPPDATA，UIA 驱动）：输入 `re`
  后文件行精确收敛为 {README.md, Report Q3.docx, REPORT-final.csv}，
  Esc 后保持。

**G-2 键盘范围选择**

- `qml/FileList.qml`：`handleCursorKey`（Up/Down/PageUp/PageDown/Home/End ×
  plain/Shift/Ctrl）。plain=移动即单选并更新锚点；Shift=从点击锚点
  `selectRange` 扩展（无锚点时以当前行为锚）；Ctrl=只移动光标不动选择；
  分栏视图显式排除（其 ←/→ 进入/回退语义保留）。
- `qml/FileGrid.qml`：`cursorKeyTarget`/`visibleRows`，上下跨 `flowColumns`，
  左右跨行首行尾（Explorer 网格语义）；Page 步长按可见行数整行跳。
- **行为变化**：方向键原先只走 `ListView` 默认移动（光标不动选择），
  现在 plain 方向键会改选择——这是本项的目的语义，不是回归。
- **验收**：UIA 冒烟读 `SelectionItemPattern.IsSelected`：点击
  {alpha-notes-2} → Down {alpha-notes-3} → Shift+Down {3,4} →
  Ctrl+Down 保持 {3,4}。

**G-3 传输 ETA**

- `crates/dae-qt/src/workspace.rs`：`transfer_eta`（速率未采样/总量未知/无剩余
  → `None`）+ `format_eta`（`m:ss` / `h:mm:ss` 时钟，99:59:59 饱和，
  NaN/负数给空串）；`publish_transfers` 的 JSON 增 `eta` 字段，仅运行中行。
  速率沿用既有 `sample_speed` 的 0.6/0.4 EMA，**未加 §3.3 预议的 <1 MiB/s 阈值**：
  平滑已经在做抖动抑制，慢链路恰恰是最需要 ETA 的场景，隐藏是错误取舍；
  规划稿该条按此修正。
- `qml/TransferQueue.qml`：行状态 `percent% · 速度 · 剩余 mm:ss`（`qsTr("剩余 %1")`，
  en「%1 left」/zh_TW「剩餘 %1」三语补齐）。
- **验收**：单测 2 条（采样门槛与时钟格式化）；Windows 冒烟：预写
  `[transfer] speedLimitKib = 5120` 到隔离 settings.toml，复制 30 GB 文件，
  队列面板行逐字显示 `0% · 5.0 MB/s · 剩余 1:35:38`（与 30GB ÷ 5 MiB/s ≈
  1h39m 一致）。

### 7.2 验证与已知边界（2026-11-19）

- `cargo test -p dae-qt --lib` 231 passed / 3 ignored；`cargo test -p dae-core --lib`
  499 passed / 10 ignored；`DAE_QML_LINT=1 cargo build -p dae-app` 通过；
  `node scripts/a11y-audit.mjs` 零新缺口（新增输入带/清除按钮均带角色与名字）。
- `DAE_QML_TYPEC=1` 在 HEAD 上即失败于 `TerminalPanel.qml`
  （`copyOnSelect` 缺属性、旧式 Connections——非本轮改动，属终端重设计提交），
  本轮六个 QML 文件的 qmltc 状态因此未验证，待终端面板修复后补跑。
- 冒烟发现的边角：名称输入带抢焦点后，物理键盘输入受系统 IME 截获（拼写出候选词）
  真人击键不受影响。

### 7.3 G-4 压缩包当目录浏览（2026-11-19）

取舍、路径形态与索引设计见 `docs/decisions/archive-browse.md`（决策文档先行，
本文 §4.1 只补结果）。

- **core**：`url.rs` 增 `Scheme::Archive` 与 `Location::Archive`（`archive:<目标>!/<内层>`，
  内层段经 `normalize_segments` 钳制，`..` 永解析不出包）；新模块
  `fs/archive_view.rs` 实现 `FileSystemBackend`——ZIP 走 central directory
  （`by_index_raw` 只读元数据、加密包整体拒绝）、`open_read` 用 `data_start()`
  字节区间 + `flate2::DeflateDecoder`，TAR 一次扫描建索引
  （`LazyLock` 缓存，键含 mtime/size，上限 32 项），`open_read` 按数据偏移直读；
  六个写方法一律 `Unsupported`；`vfs::resolve` 加一臂。
- **Qt/QML**：`Scheme::Archive` 自动落入 model 的远程分支（一次 `read_dir`、
  `stream_id: None`），面包屑由后端给（本地面包屑 + 包名 + 内层段）；
  `errors.rs`/`Localize.js`/三个 `.ts` 补六条错误文案；
  `ShellWindow.isRemotePath` 补 `archive`、`parentOf` 学会沿内层上溯并从包根
  回到持有目录；双击本地 `.zip/.tar` 经 `enterArchivePath` 进入压缩包视图
  （`FileList.enterArchive` 钩子覆盖列表/网格/分栏三个面）。
- **验证**：`cargo test -p dae-core --lib` 504 passed（新增 5 条）；
  `DAE_QML_LINT=1 cargo build -p dae-app` 通过；Windows 冒烟（隔离 APPDATA，
  UIA）：zip 根 {docs, top.txt}、内层 {nested, data.bin, readme.txt}、
  tar 根 {docs, plain.bin}；面包屑 `K: > … > notes.zip > docs` 可点；
  复制 `readme.txt` 出包 43 B 逐字节一致；Quick Look 预览包内文本无错误。
- **维持不做**（决策文档 §5）：远程底层、tar.gz/7z/rar/iso、加密 zip、
  写回、包内剪切（`rename_to` 跨后端回退 copy+delete 会命中只读错误，
  文案「压缩包为只读」）。

### 7.4 G-5 远程缩略图（2026-11-19）

Dolphin 的取舍照搬：远程行值得拉的只有**视图自己会解码的格式**（png/jpg/
gif/bmp/webp/svg/tiff），桌面代工集（PDF/视频/HEIC）维持本地——生产者要读
本地文件，远程拉下来也没人能解。

- **core**（`crates/dae-core/src/fs/thumbnails.rs`）：`fetch_remote(path, cap)`
  ——`is_thumbnail_path` 先挡（不值得的字节不上网），`vfs::resolve` →
  `stat`（目录/空/超 64 MiB 上限拒）→ `open_read` 有界读。任何一步失败都
  是安静的 `None`，行回落类型图标。
- **桥**（`crates/dae-qt/src/icons.rs`）：`dae_qt_is_local_path` /
  `dae_qt_fetch_remote_thumbnail` 两个 cxx 桥。
- **C++**（`cpp/image_provider.cpp/.h`）：`requestImage` 按 `is_local_path`
  分流；远程半场 `remoteImage` 从内存 `QBuffer` 解码（缩放逻辑与本地同款），
  缓存键用 URL 里的 `v=mtime-size` 戳——`QFileInfo` 对远程无从取时间。
- **模型**（`crates/dae-qt/src/model.rs`）：`thumbnail_url` 的 `if remote
  return ""` 收窄为「远程且非可解码格式」；**预览面板**
  （`preview.rs has_thumbnail`）同样放行远程图片格式。
- **验证**：`cargo test -p dae-core --lib` 505 passed（新增：archive 后端
  充当非本地路径——真实 zip 内 png 逐字节取回、pdf 拒、超限拒、目录拒、
  未知拒）；`cargo test -p dae-qt --lib` 233 passed（远程 URL 生成与
  `has_thumbnail` 门控各增用例，旧契约 `network_entries_have_no_desktop_thumbnail`
  按新行为改写）。
- **Windows 冒烟**（用户授权自建本机服务器，测试后全部删除）：wsgidav 起于
  `127.0.0.1:8765`（匿名，`pip` 装的 wsgidav/cheroot 用毕卸载）；隔离
  APPDATA 预置一条 `connections.json` 保存连接（与 `remote_smoke.rs` 同一
  机制），点侧栏服务器行进入。结果：**网格视图四个 png 显示真图**
  （纯红/纯绿/蓝渐变/条纹），`notes.txt` 与 `report.pdf` 正确保持类型图标；
  Quick Look（检查器）预览 `webdav://…/red-photo.png` 渲染红色图无错误；
  另跑仓库自带 `remote_smoke webdav -- --ignored` 对同一服务器全绿
  （connect/browse/preview/copy/rename/delete）。
- **维持不做**：远程视频帧与 PDF 首页（桌面生产者需要本地文件）、远程
  `search`（既有拒绝语义，`vfs.rs` 默认）、SFTP 真机冒烟——sshd 在跑但
  未动用户的凭据配置，管线与 WebDAV 完全同层（同一 `fetch_remote` 与
  同一 provider），由单测覆盖。

### 7.5 G-6 字节级续传（2026-11-19）

TeraCopy 语义两半：**取消保留半文件**（保留是续传能接手的前提），**继续从
半文件自己的结尾接着写**。取消契约的改写（旧规则「正在写的文件被删除」的
理由——续传与冲突判断失真——在字节级续传面前不再成立）已同步进
`docs/decisions/transfer-control.md` §0/§4。

- **vfs 原语**（`vfs.rs`）：`open_write_at(path, offset)`，默认 `Unsupported`；
  本地后端 `OpenOptions + seek` 落位。其余后端继承默认 → 引擎回退整文件重拷。
- **通用引擎**（`transfer.rs`）：`resume_offset`（目标更小、非空、不大于源、
  不旧于源 → 其长度即偏移；源比半文件新 = 被改过，回退整拷）+
  `skip_bytes`（读原语无 seek，前缀流式跳过）+ `copy_node` Merge 分支接
  `open_write_at`，前缀计入进度（进度条描述整个文件）；**取消不再删半文件**
  ——`Replace` 本就先删目标，普通复制压同名走冲突对话框，都是对可见文件的
  明确回答。
- **本地快速路径**（`local/operations.rs`）：冒烟暴露的真问题——本地→本地
  走 `fs::copy` 快路径，不经通用引擎；限速分支（唯一可中断的本地路径）
  `copy_file_throttled` 同样支持偏移续写、取消保留半文件。`fs::copy` 原子且
  总是整拷，无需处理。
- **验证**：`cargo test -p dae-core --lib` 508 passed（新增 3 条：毒化
  `open_write` 证明走的是偏移路径、更大/更旧半文件回退整拷、
  无偏移写后端回退整拷）。Windows 冒烟（隔离 APPDATA，限速 5 MiB/s）：
  300 MB 复制 15 s 后取消 → **半文件 92 MB 留盘**；状态中心「继续」→
  300 MB 补齐、SHA256 与源一致；续传耗时 **43.0 s ≈ 剩余 222 MB ÷ 5 MiB/s**
  （整文件重拷需 60 s）——「耗时 ≈ 剩余部分」的验收逐字成立。
- **维持不做**：远程目标续写（`open_write_at` 默认 Unsupported 即此语义）、
  续传后哈希校验（规划既定）、`undo` 对半文件的处理（journal 只记完整落地
  条目，半文件不受撤销影响；`Replace` 目标在操作前已被 journal 记录，撤销
  会连半成品一起清理——既有 §4.2 规则不变）。

### 7.6 P3 三项（2026-11-19）

**G-7 地址栏历史与展开**

- **桥**（`workspace.rs`）：`recentDirectories()`（recents 里的目录，按访问时间
  倒序、去重、上限 32）与 `expandTypedPath(text)`（`~`/`%VAR%`/`$VAR`/`${VAR}`，
  逐 char 扫描保多字节文本；解析不到的环境变量原样保留——导航报错时会念出
  用户输入的原话）。
- **QML**（`Locator.qml`）：地址编辑态打开时拉一次 recents；`updateCompletions`
  在「还没有分隔符」时改用历史做补全源（前缀匹配、上限 8），有分隔符时仍走
  既有子目录补全；回车路径先剥引号、再展开、再导航。
- **验证**：单测覆盖四种展开、不可解析保留、`~notes`（非路径）不动、
  `~`/`~\sub` 到家目录；冒烟：地址栏输入 `~` 回车 → 面包屑
  `此电脑 > … > MR342`，真机到家目录；补全弹层正常出现（本次冒烟地址栏预填
  带分隔符的路径，历史分支的入口由单测与代码路径覆盖）。

**G-8 十六进制预览**

- **桥**（`dae-qt/src/preview.rs`）：`snapshot_body` 在「文本未解出、且图片/
  媒体/Office 都没有认领、本地文件」时读前 4 KiB 生成 hex 列表
  （`format_hex`：8 位地址列 + 16 字节列 + 可打印 ASCII 列，控制字符记 `.`）；
  `snapshot_from` 增 `forced_kind`，hex 内容强制走代码卡（等宽、无行号、
  横向滚动，未知语言回落 plain `<pre>`）。`truncated` 按文件大小判定，
  底部既有「内容过长」提示复用。
- **验证**：单测 2 条（格式：偏移/字节列/对齐到列 57；读取窗口：4 KiB 截断与
  行数）；冒烟：选中 `unknown-format.bin`（0..255 循环）按空格 → 检查器逐字
  读出 `00000000  00 01 … 0f` 与 `00000010  10 11 … 1f` + 点状 ASCII 列，
  无报错。

**G-9 终端跳转**

- **动作**（`settings.rs`）：`explorer.terminalJumpFolder` 进可重绑定注册表
  （52 → 53，默认留空）；设置面板「视图与导航」组增「终端跳转当前目录」行。
  `FileList.handleKey` 命中后发 `terminalJumpRequested`，`ShellWindow` 转给
  `terminalPanel.jumpToFolder()`（两个窗格实例都接线）。
- **行为**（`TerminalPanel.qml`）：`jumpToFolder` 取当前标签的**本地**目录，
  按会话 shell 生成 `cd -- "…"`（PowerShell 转义反引号与 `$`；cmd 用
  `cd /d "…"`；POSIX 用单引号并处理内嵌单引号），作为键盘输入写入 PTY。
  与既有的「不自动跟随」注释并列写清：跟随不做，用户显式请求做。
- **验证**：冒烟：`显示终端` 开面板（会话生于 `…\smoke7\in`）→ 列表导航到
  `K:\code\dae\target` → 点面板「跳转」→ 提示符从 in 变为
  `PS K:\code\dae\target>`，无报错。
- **维持不做**：面板按钮之外的自动跟随（既有决策）、远程标签的跳转（本地 PTY
  没有可命名的远程目录，函数内直接返回）。

### 7.7 修复 DAE_QML_TYPEC 门（2026-11-20）

根因：`qml/dae/terminal/terminal.qmltypes` 是手写的工具元数据（头注释要求
「Keep it in sync with the Q_PROPERTY / Q_INVOKABLE / Q_SIGNAL block」），
终端重设计（HEAD `d80484f`）给 `cpp/terminal_item.h` 加了
`copyOnSelect`、只读 `columns`/`rows`、`searchRequested`/`gridSizeChanged`/
`linkActivated(target,line,column)` 等成员，这份文件没有同步——于是 qmltc 在
`TerminalPanel.qml` 上三连失败：
- `onSearchRequested` / `onLinkActivated` 找不到信号（unqualified warning）；
- `copyOnSelect` 找不到属性（missing-property error）；
- 连带报出通用的「custom parsers / old forms of Connections」compiler 错误
  （qmltc 在元数据缺口后的兜底话术；文件里并不存在 ListModel 或旧式
  Connections，`Connections` 是新式 `function onExited`）。

修复：按头文件逐项重写 `terminal.qmltypes`（8 个属性含索引与 notify、
7 个信号含 `linkActivated` 的三个参数、3 个方法）；`lineHeight` 的类型从
`qreal` 改为 QML 类型名 `double`（`qreal` 会让 qmltc 报
"No type found for property"）。运行时不受影响：类型注册走
`qmlRegisterType`，qmltypes 只服务 qmllint/qmlcachegen/qmltc。

验证：`DAE_QML_TYPEC=1 cargo build -p dae-app` 先 touch
`TerminalPanel.qml` 强制重跑，干净 `Finished`、无 panic（此前在 HEAD 上必挂）；
`DAE_QML_LINT=1` 门通过；`node scripts/a11y-audit.mjs` 零新缺口；
`cargo test -p dae-core --lib` 508 passed（本修复不触 Rust）。
终端运行时的行为核对未另跑：qmltypes 无运行时效应，且同一二进制在 G-9
冒烟里刚验证过终端面板与 PTY。
