# 对照 Files 的功能缺口与跨平台实施计划

> 日期：2026-11-04 · 状态：P0、P1、R-1…R-6 均已落地（2026-11-05/07）；剩余项与未验证项见 §12
> 对象：`apps/dae` + `crates/dae-core` + `crates/dae-qt` + `qml/`
> 对照：Files v4（Windows）的功能面与命令面板
> 前提：Windows 发布 MSIX/zip，Linux 发布 deb/rpm/zip；macOS 只保留可编译代码、不发布，
> 且没有可用设备，**运行时永远不验证**（`README.md`「Install」；`.github/workflows/qt-check.yml`）

本文只回答三件事：**dae 现在缺什么、每项在三个平台怎么落、先做哪一项**。
所有「现状」都指向本仓库的文件；所有「Files 有」都来自其文档或命令面板，不是印象。
本文是规划稿，不是决策记录：落地时真正需要取舍、测量或复盘的判断，按
`docs/decisions/` 的格式另立文档。

---

## 0. 结论

dae 在「浏览、文件操作、多后端、终端、预览」上已经追平甚至超过 Files；
缺口集中在四类：

1. **元数据体系**：文件标签、按标签排序/分组/搜索、信息面板。
2. **视图组织**：每目录布局、分组、可调列宽、扩展名开关、图标大小。
3. **系统级集成**：驱动器弹出/格式化、FTP/WebDAV/NFS、共享、签名、ACL。
4. **可扩展动作**：用户自定义命令、全量快捷键、状态中心。

P0、P1 与 §12.1 的 R-1…R-6 落地后，上表四类只剩验证尾巴：平台索引只验过未打包
形态，动作面按需可继续加。**当前剩余项以 §12.3 为准**；
本节其余内容保留为立项时的判断依据。

跨平台原则先于功能：**任何一项都必须在 Windows、Linux 上要么可用，要么明确降级并说明原因**；
不允许「只在 Windows 生效且界面不提示」。macOS 不发布、无设备，只要求由 `qt-check`
每次 push 编译通过，新增代码保持可编译、入口按平台隐藏，运行时不验收（§7、§12.2）。

---

## 1. 判据

| 判据 | 含义 |
| --- | --- |
| 跨平台优先 | 能 Windows、Linux 都做的先做；平台独占项必须 `cfg` 隔离并隐藏入口；macOS 只保证编译 |
| 能力探测 | 启动或首次使用时探测 xattr、索引、外部工具；失败给原因，不静默 |
| 热路径不变 | 列表枚举不加逐条系统调用；标签/元数据按需加载（`docs/decisions/listing-shell-costs.md`） |
| 可回退 | 外部工具缺失时保留内置实现或隐藏入口，不把功能做成硬依赖 |
| 可测量 | 涉及列表、搜索、标签索引的改动先有基准，再谈优化 |

---

## 2. 现状盘点

| 能力域 | 已有 | 证据 |
| --- | --- | --- |
| 视图 | 列表、网格、分栏、分屏、密度、文件夹优先 | `qml/FileList.qml`、`FileGrid.qml`、`ColumnView.qml`、`ExplorerPrefs` |
| 标签页 | 新建/关闭/拖动/跨窗口撕出 | `qml/TabStrip.qml`、`qml/Tab.qml` |
| 文件操作 | 复制/剪切/粘贴、撤销/重做、回收站、永久删除、重命名（含正则批量）、副本、快捷方式、压缩/解压、移动到 | `crates/dae-core/src/fs/{undo,transfer,archive,shortcuts}.rs`、`workspace.rs` |
| 后端 | 本地、SFTP、SMB、压缩包、WSL、回收站、云盘（Google Drive/OneDrive/Dropbox） | `crates/dae-core/src/fs/`、`fs/cloud/` |
| 搜索 | 当前目录递归名称搜索、ripgrep 内容搜索、类型过滤 | `fs/search.rs`、`qml/SearchFilterBar.qml` |
| 预览 | 文本/Markdown/图片/PDF/音视频、缩略图 | `crates/dae-qt/src/preview.rs`、`qml/PreviewPanel.qml` |
| 属性 | 常规、POSIX 权限、DOS 属性、MD5/SHA1/SHA256 | `crates/dae-qt/src/properties.rs`、`qml/PropertiesDialog.qml` |
| Git | 状态、分支、fetch/pull/push/sync | `fs/git.rs`、`qml/GitPanel.qml` |
| 终端 | 内嵌 PTY 面板 | `crates/dae-qt/src/terminal.rs`、`qml/TerminalPanel.qml` |
| 系统集成 | 默认文件管理器、深链、更新器、Windows Shell 扩展命令 | `default_manager.rs`、`activation.rs`、`updater.rs`、`fs/shell_commands/windows.rs` |
| 外观 | 明暗、强调色、Mica/Acrylic/Wayland 模糊、减少动效、en/zh-CN | `settings.rs`、`qml/SettingsPanel.qml` |

---

## 3. 缺口矩阵

工作量按「一个熟悉该模块的人」估算：S ≤ 1 天，M ≈ 2–5 天，L > 1 周。

### P0：每天会碰，Files 有、dae 没有

| # | 缺口 | Files 的做法 | dae 现状 | 平台影响 | 量 |
| --- | --- | --- | --- | --- | --- |
| P0-1 | 文件标签：多标签、颜色、侧栏标签区、标签列、`tag:` 搜索、标签管理器、拖到标签上打标 | 标签存 NTFS ADS，可过滤/排序/搜索 | 无 | 三平台都要有；存储层按平台分支 | L |
| P0-2 | 排序/分组：按创建时间、标签、路径；按名称/日期/大小/类型/标签分组，日期可选年/月/日；文件与文件夹可混排 | Layout → Sorting and grouping | 仅 `name/modified/type/size` + `foldersFirst`（`preferences.rs:21-24`） | 创建时间在 Linux 需 `statx` 回退 | M |
| P0-3 | 每目录布局 + 跨目录同步开关 + 自适应布局 | 每个文件夹记住视图/排序 | `ExplorerPrefs` 只有全局一份（`preferences.rs:48-50`） | 无平台差异 | M |
| P0-4 | 详情列可拖宽/可勾选、自动适配；显示扩展名开关；图标大小滑杆 | Details 布局 | 列随窗口宽度自动隐藏（`FileList.qml:56-58`）；网格固定 5 列 38px（`FileGrid.qml:41`）；扩展名始终显示 | 无平台差异 | M |
| P0-5 | 搜索体系：跨位置搜索、日期/大小/类型过滤、搜索历史、`tag:` 语法 | Windows 搜索索引 + Filter 头 | 只搜当前目录（`fs/search.rs:41/66`）；过滤器只有类型 | 索引按平台探测 | L |
| P0-6 | 信息/详情面板：标签编辑、元数据、权限摘要 | Info Pane 的 Details/Preview 两页 | 只有预览面板；完整信息在属性对话框 | 无平台差异 | M |

### P1：系统集成与效率

| # | 缺口 | Files | dae 现状 | 平台影响 | 量 |
| --- | --- | --- | --- | --- | --- |
| P1-1 | 驱动器工具：弹出、格式化、清理、挂载/卸载 | 驱动器菜单 + 命令面板 | 全无；`grep eject` 只命中无关词 | 三平台 API 完全不同，见 §5.7 | M |
| P1-2 | FTP/FTPS/WebDAV/NFS、网络位置发现 | FTP/FTPS；侧栏 Network 段 | `Protocol::Ftp/WebDav` 可解析，但 `vfs.rs:79-80` 直接 `unsupported_scheme`；连接对话框只有 SMB/SFTP | 后端代码可复用；发现走 mDNS | L |
| P1-3 | 自定义动作 + 全量快捷键 + Open in IDE | 150+ actions，Actions 设置页 | 22 个可重绑定动作（`settings.rs` 的 `default_shortcuts`）；Windows 靠 `IExplorerCommand` 补一部分 | Linux/macOS 没有 Shell 扩展，需自带动作 | M |
| P1-4 | 状态中心：操作历史、失败项、重试、通知徽章 | Status Center | 只有 `qml/TransferQueue.qml` 的当前队列 | 三平台一致（应用内） | M |
| P1-5 | 属性增强：哈希比对、EXIF/媒体元数据、文件签名、Windows ACL、自定义图标 | Hashes/Signatures/Details | 只算不比对；`media_meta.rs` 只判断扩展名；权限只有 POSIX + DOS 属性 | 签名/ACL 平台独占；EXIF 跨平台 | S–L |
| P1-6 | 共享 / 上一版本 / BitLocker | Windows 集成 | 无 | Windows/macOS 有入口，Linux 无 | M |
| P1-7 | 标签页管理 + 会话恢复 | 重开、复制、关闭左/右/其他、固定；启动恢复 | 只有关闭/拖动/撕出 | 三平台一致 | S–M |
| P1-8 | 快速预览（Space 弹层） | 集成 QuickLook/SeerPro | `Space` 只切换预览面板 | 先做应用内弹层 | M |
| P1-9 | 工具栏自定义 / 主页小部件 | 可拖工具栏；Home 小部件 | 工具栏固定；`OverviewView` 只有项目/收藏/最近/空间 | 三平台一致 | M |
| P1-10 | 文件大小单位、按需计算文件夹大小 | KB/MB 与 KiB/MiB 可切 | `format.rs` 固定 1024 进制却标 KB/MB；文件夹大小只在属性里算 | 三平台一致 | S |
| P1-11 | 压缩格式：RAR/ISO、智能解压、解压到新文件夹 | 7-Zip 依赖 | 只有 zip/tar/tar.gz/7z；只有「解压到此处」 | 外部工具按平台探测 | M–L |
| P1-12 | 多语言 / 主题背景 / RTL | 30+ 语言、背景图、RTL | en/zh-CN；只有明暗 + 强调色 | RTL 需 QML 镜像审计 | L（持续） |

### P2：可选，或平台绑定太强

| # | 缺口 | 说明 |
| --- | --- | --- |
| P2-1 | 紧凑覆盖窗口（compact overlay） | 与 dae「内容优先」的外壳冲突，优先级低 |
| P2-2 | MTP/便携设备 | Files 文档未列为受支持能力；不作为本轮对齐目标 |
| P2-3 | 标签的 Explorer 互操作 | Windows 没有通用文件标签；macOS 可镜像 Finder 标签，Linux 无标准 |
| P2-4 | 系统通知 | 三平台 API 差异大；v1 只做应用内状态中心 |

---

## 4. 跨平台能力矩阵

这张表是每项设计的约束来源。实现时先查表，再决定入口是「可用」「降级」还是「隐藏」。
macOS 列只是语义参考：该平台不发布、无设备，实现时只需按此列 `cfg` 隔离入口并保证编译
（§12.2），不要求行为一致。

| 能力 | Windows | Linux | macOS | 降级 |
| --- | --- | --- | --- | --- |
| 扩展属性 | NTFS ADS（`path:dae.tags`） | `user.*` xattr（ext4/xfs/btrfs） | `com.dae.tags`（xattr） | 配置目录索引 |
| 创建时间 | `Metadata::created()` | `statx(STATX_BTIME)`，缺失回退 `ctime` 并标注 | `Metadata::created()`（birthtime） | 回退值加标注 |
| 弹出介质 | `DeviceIoControl(IOCTL_STORAGE_EJECT_MEDIA)` 或 `CM_Request_Device_Eject` | `udisksctl unmount` + `eject`/`gio mount -e` | `diskutil eject` | 隐藏入口并说明 |
| 格式化 | `SHFormatDrive` 系统对话框 | 打开 `gnome-disks`/`udisksctl`，不做内置格式化 | 打开「磁盘工具」，不做内置格式化 | 只提供系统工具入口 |
| 挂载/卸载 | 系统自动 | `udisksctl mount/unmount` | `diskutil mount/unmount` | 隐藏 |
| 系统共享 | WinRT `DataTransferManager` | 无统一入口 | `NSSharingServicePicker` | Linux 隐藏 |
| 文件签名 | `WinVerifyTrust` | 无系统级等价物 | `SecStaticCodeCheckValidity` | Linux 隐藏 |
| ACL | 安全描述符 | POSIX ACL | NFSv4 ACL | 先保留现有 POSIX 模式编辑 |
| 搜索索引 | Windows Search（`ISearchQueryHelper`，MSIX 形态需验证） | `tracker3`/`baloo` 可选 | Spotlight `mdfind` | 并行遍历 |
| 快速预览 | 应用内弹层 | 应用内弹层 | 应用内弹层；可选 `QLPreviewPanel` | 应用内 |
| 文件标签 | ADS + 索引 | xattr + 索引 | xattr + 索引；可选 Finder 标签镜像 | 索引 |
| 系统通知 | WinRT toast | D-Bus Notifications | `UNUserNotificationCenter` | 应用内状态中心 |

---

## 5. 逐项设计

### 5.1 文件标签（P0-1）

**数据模型**

```text
TagDefinition { id: String, name: String, color: String }
EntryTags     { path, tag_ids: Vec<String>, source: Xattr | Ads | Index }
```

标签定义存配置目录的 `tags.toml`（与 `settings.toml` 分开，便于导出/导入和手改）；
文件与标签的关联存文件本身或索引，不进 `settings.toml`。

**存储三层**

1. **文件系统扩展属性（本地可写卷的首选）**
   - Windows：NTFS ADS `path:dae.tags`，值为 UTF-8 JSON 数组（标签 id）。
   - Linux：`user.dae.tags`，同样 JSON；ext4/xfs/btrfs 可用，FAT/exFAT 不可用。
   - macOS：`com.dae.tags`；可选同时写 `com.apple.metadata:_kMDItemUserTags`
     （binary plist 字符串数组），让 Finder/Spotlight 也能看到。该镜像默认关闭，
     因为它是 macOS 专有格式，写失败不能影响主存储。
   - 写入失败（只读卷、网络卷、权限不足）不得静默：回退索引，并在标签编辑器里说明
     「此位置不支持随文件保存，标签只在本机生效」。

2. **配置目录索引（回退，也是远程/压缩包/云盘的唯一存储）**
   - 位置：`<config>/tags-index/`，按卷/后端分片，避免单文件过大。
   - 键：规范路径；本地卷带文件标识（Unix `dev+ino`，Windows 卷序列号 + 文件索引）
     以抵抗重命名。**已落地（2026-11-07）**：dae 内重命名直接迁移条目，dae 外重命名
     在读取新路径时按标识找回；细节与边界见 `docs/decisions/tag-index-integrity.md`。
   - 值：标签 id 列表 + 最后写入时间。
   - 远程（SFTP/SMB/云盘）与压缩包只走这一层；标签不随文件离开 dae，界面要讲清楚。
   - 是否需要引入嵌入式数据库（如 `rusqlite`）先做决策记录与基准；v1 可以用分片 JSON +
     原子写，量级到 10 万条再评估。

3. **读取路径（不碰列表热路径）**
   - 目录枚举不逐条读 xattr（会破坏 `docs/decisions/listing-shell-costs.md` 的预算）。
   - 列表完成后，按可见行/选中项批量读取，结果进缓存，用 `dataChanged` 更新 `tags` 角色。
   - 侧栏标签计数、`tag:` 搜索只查索引，不遍历文件。
   - 基准：12 万条目列表 p95 相对现状回退 ≤ 5%；在 `bench/listing-pipeline` 留一组对照。

**UI**

- 右键「标签」子菜单（多选时批量切换）；拖到侧栏标签上打标。
- 列表新增「标签」列（默认隐藏，可在列配置里打开）。
- 侧栏新增「标签」段，显示颜色与计数；点击进入标签搜索结果。
- 设置页新增「标签」：新建/改名/改色/删除、导入/导出定义。
- 命令面板：`tag:<name>`、清除标签、按标签过滤。
- 搜索语法：`tag:foo`、`tag:foo,bar`；与现有名称/内容搜索共用查询入口。

**平台注意**

- 能力探测按卷缓存：在目标目录建临时文件写一次 xattr/ADS，成功才用第一层。
- Linux 没有通用标签 xattr 约定；`user.dae.tags` 是 dae 自己的命名，不假设桌面互操作。
- macOS 的 Finder 标签镜像需要 binary plist 编码；失败只记日志，不影响主存储。
- 压缩包内条目标签只能存索引，且解压后不自动继承（除非显式复制）。

### 5.2 排序与分组（P0-2）

- `SortKey` 扩展：`created`、`path`、`tag`；`foldersFirst` 改为枚举
  `FoldersFirst | FilesFirst | Mixed`，旧配置迁移。
- 创建时间：
  - Windows/macOS 用 `Metadata::created()`。
  - Linux 用 `statx` 的 `STATX_BTIME`；文件系统不支持时回退 `ctime`，
    UI 标注「创建时间（回退）」而不是假装准确。
- 分组键：名称首字母/名称桶、修改时间、创建时间、大小桶、类型、标签；
  日期分组可选年/月/日；组内继续按当前排序。
- 实现：分组在 Rust 侧完成（12 万条不能在 QML 里分组），模型增加 `group` 角色，
  QML 用 `ListView.section` 或分组头行；分组后的选择/框选/键盘导航必须回归。
- 性能：≥2048 行沿用现有「排序移到 worker」的做法（`listing-shell-costs.md` §9），
  分组不能把整段重排拉回 GUI 线程。

### 5.3 每目录布局与自适应（P0-3）

- `prefs.toml` 增加 `[views]`：`"<canonical-path>" = { viewMode, density, sortKey, sortOrder, ... }`。
- 路径规范化：Windows/macOS 大小写折叠；远程用 `scheme://host/path`；LRU 上限 512，
  超出淘汰最久未用。
- 「跨目录同步布局」开关：开时只读写全局默认，关时读写当前目录。
- 自适应布局规则（先做简单可解释的版本）：图片占比高 → 网格；层级深/列多 → 分栏；
  其余 → 列表。规则写在 Rust 侧，便于测试。
- 迁移：现有全局偏好作为默认值；首次打开旧目录不写新键。

### 5.4 详情列、扩展名、图标大小（P0-4）

- 列表表头支持拖拽改列宽、双击自动适配；列可见性存进每目录视图配置。
- 「显示文件扩展名」只影响显示，不影响重命名、排序和搜索。
- 图标大小：网格 24–96 px 滑杆，列表缩略图同值；持久化到偏好。
- 全部是 QML + `ExplorerPrefs`，无平台分支；`qmllint` 与三平台编译是唯一门槛。

### 5.5 搜索体系（P0-5）

分两阶段，避免一上来做跨平台索引：

**阶段 1：范围搜索 + 过滤 + 历史**

- 范围：当前目录（现有）、收藏、最近、所有本地卷。
- 并行遍历，限制并发与结果上限；沿用现有取消令牌。
- 过滤器：类型、修改时间、大小（`FilterMenu` 已有，移到搜索栏复用）。
- 搜索历史存 `prefs.toml`，上限 20 条。
- **已落地（2026-11-09）**：命名的保存搜索（查询词 + 模式 + 范围 + 过滤器），见 §13.2。
- `tag:` 走标签索引，不走遍历。

**阶段 2：平台索引（能力探测，可选）**

- **已落地（2026-11-07）**：索引只作为候选源，名字与过滤规则仍由 `fs::search`
  统一执行；Windows 走 `ISearchQueryHelper` + `Search.CollatorDSO`（OLE DB，
  `System.ItemUrl` 列绑定为 VARIANT），Linux 走 `tracker3`/`baloo` 的 CLI。
  语义、失败回退与边界见 `docs/decisions/search-index.md`。
- Windows：`ISearchQueryHelper`；MSIX 打包形态要先验证权限与索引可见性。
- macOS：`mdfind`（Spotlight），实现成本最低。
- Linux：`tracker3` 或 `baloo` 存在时调用，否则隐藏「使用系统索引」开关。
- 索引结果与遍历结果合并去重；索引不可用时自动回退，不阻塞搜索。

### 5.6 信息/详情面板（P0-6）

- `PreviewPanel` 增加 Details 页签：标签编辑、类型/大小/时间、权限摘要、哈希入口。
- 复用 `crates/dae-qt/src/properties.rs` 的读取逻辑，不新增后端。
- 多选时显示聚合信息（数量、总大小、共同标签），与 Files 的 Info Pane 对齐。
- 平台一致；远程条目隐藏不可读字段并说明原因。

### 5.7 驱动器工具（P1-1）

驱动器菜单对 `DiskVolume.is_removable` 显示弹出/卸载/格式化；「清理」是系统级入口，
不绑定具体卷。破坏性动作永远二次确认，系统卷不提供格式化入口；每个动作失败时
给平台原因，而不是静默关闭菜单。

| 动作 | Windows | Linux | macOS |
| --- | --- | --- | --- |
| 弹出 | `DeviceIoControl(IOCTL_STORAGE_EJECT_MEDIA)`；失败回退 `CM_Request_Device_Eject` | `udisksctl unmount -b <dev>` 后 `eject <dev>` 或 `gio mount -e` | `diskutil eject <disk>` |
| 卸载/挂载 | 系统自动，隐藏 | `udisksctl unmount/mount -b <dev>` | `diskutil unmount/mount` |
| 格式化 | `SHFormatDrive` 系统对话框 | 打开 `gnome-disks`；不内置 | 打开「磁盘工具」；不内置 |
| 清理 | 打开 `ms-settings:storagesense` | 隐藏 | 隐藏 |

v1 不实现应用内格式化：破坏性操作交给系统工具，dae 只负责把用户送到正确入口。
这比在三个平台各写一套 `mkfs`/`format` 更安全，也避免提权问题。

### 5.8 网络后端（P1-2）

- **FTP/FTPS**：新增 `fs/ftp.rs`，走阻塞池；显式/隐式 TLS 用现有 `rustls`；
  被动模式；凭据沿用 `connections.rs` + `keyring`；路径解析接到 `Scheme::Ftp`。
- **WebDAV**：`reqwest` 实现 `PROPFIND`（Depth 1）、`MKCOL`、`DELETE`、`MOVE`、`COPY`、
  `GET`、`PUT`；先 Basic 认证，Digest 另立任务。
- **NFS**：不做用户态后端；系统挂载后作为普通卷出现（Windows NFS 客户端、
  Linux `mount -t nfs`、macOS `mount_nfs`）。
- **发现**：`mdns-sd` 浏览 `_smb._tcp`、`_sftp-ssh._tcp`、`_webdav._tcp`；
  Windows 可选 WS-Discovery。发现结果只填主机/端口，不自动带凭据。
  **已落地（2026-11-07）**：一次性、有时限的探测（1.5 s），侧栏「网络」段列出结果，
  点击只预填连接对话框；TXT 记录一律不信，WS-Discovery 仍未做
  （`docs/decisions/network-discovery.md`）。
- 在 `vfs.rs:79-80` 的 `unsupported_scheme` 移除之前，连接对话框不要放出 FTP/WebDAV 入口，
  避免「能填表但不能打开」。
- macOS 不发布，但代码必须编译；SMB 走现有实现。

### 5.9 自定义动作与快捷键（P1-3）

`settings.toml` 增加：

```toml
[[actions]]
id = "open-in-ide"
name = "在 IDE 中打开"
command = "code"
args = ["%d"]
platforms = ["windows", "linux", "macos"]
selection = "directory"   # any | file | directory | multiple
extensions = []           # 空表示不过滤
```

- 占位符：`%f` 单个文件、`%F` 全部文件、`%d` 当前目录、`%u` URL；参数按数组传，
  不拼 shell 字符串，避免注入与空格问题。
- 内置动作与自定义动作共用同一个动作注册表；快捷键从 22 个扩到全部动作，
  冲突检测沿用现有逻辑。
- 「Open in IDE」作为内置动作，命令可在设置里改。
- Windows 的 `IExplorerCommand` 继续作为补充；Linux/macOS 靠这套自定义动作补平。
- 动作执行复用现有 spawn/日志/错误上报，不新开进程模型。

### 5.10 状态中心（P1-4）

- 把 `TransferQueue` 扩成操作记录：`{id, kind, started, finished, status, error, retryable}`。
- 内存环形缓冲 200 条，持久化最近 50 条到配置目录（原子写）。
- 失败项可重试的只有传输/解压；删除、覆盖不可重试。
- 标题栏徽章显示进行中/失败数；打开面板即清零未读。
- v1 只做应用内，不接系统通知（三平台 API 差异大，收益低）。

### 5.11 属性增强（P1-5）

按投入从小到大：

1. **哈希比对**（S，三平台）：输入已知值或选另一个文件，比较 MD5/SHA1/SHA256。
2. **EXIF/媒体元数据**（M，三平台）：图片用 `kamadak-exif`，音频用 `lofty`；
   视频先探测 `ffprobe`，没有就隐藏。
3. **文件签名**（M，Windows/macOS）：Windows `WinVerifyTrust`，macOS
   `SecStaticCodeCheckValidity`；Linux 隐藏。
4. **ACL**（L，三平台）：Windows 安全描述符、Linux POSIX ACL、macOS NFSv4 ACL；
   先保留现有 POSIX 模式编辑，ACL 单独立项。
5. **自定义图标**（暂缓）：Windows `desktop.ini`、Linux `.directory`、macOS 资源叉/xattr，
   三套语义不一致，收益低。

### 5.12 标签页与会话恢复（P1-7）

- 已关闭标签栈（每窗口 16 个）、复制标签、关闭左/右/其他、固定标签、`Ctrl+Tab` 切换。
- 会话恢复：窗口几何、标签列表（路径 + 后端 + 历史）、分屏状态、每窗格当前项；
  启动时按设置恢复，崩溃后下次启动提示恢复。
- 原子写配置目录；多窗口按创建顺序恢复，避免抢焦点。
- 三平台一致；Wayland 下窗口位置可能由合成器决定，恢复失败不报错。

### 5.13 快速预览（P1-8）

- 应用内弹层：复用 `PreviewController`，`Space` 在预览面板关闭时打开弹层，
  `Esc`/点击外部关闭；焦点陷阱与键盘滚动。
- **已落地（2026-11-09）**：Office（`docx`/`xlsx`/`pptx`）的文本预览，见 §13.1。
- 不依赖 QuickLook/SeerPro；macOS 以后可选 `QLPreviewPanel`，但不作为 v1 条件。
- 三平台一致；远程条目沿用预览面板的「文本可预览、缩略图仅本地」规则。

### 5.14 工具栏与主页（P1-9）

- 工具栏项模型 + 拖拽排序 + 恢复默认；存 `settings.toml`。
- 主页小部件：标签、驱动器、最近、收藏；每项可开关。
  **已落地（2026-11-07）**：概览页新增「标签」「驱动器」两段，设置页的「主页小部件」
  随之多两个开关；标签行点击进入 `tag:` 搜索（从概览先回到文件夹），驱动器行跳到挂载点。
- 纯 QML/配置，无平台分支。

### 5.15 大小单位与文件夹大小（P1-10）

- `SizeUnits { Binary, Decimal }`；`format.rs` 输出 `KiB/MiB` 或 `KB/MB`，设置可切。
- 列表「计算大小」按钮按需调用现有 `directory_size`，带进度与取消；
  不在枚举时自动算。
- 三平台一致。

### 5.16 压缩格式（P1-11）

- 内置保持 zip/tar/tar.gz/7z。
- RAR：UnRAR 库的许可带额外限制，不适合直接链接进 GPL 项目；优先探测系统
  `unrar`/`7z`/`bsdtar`，没有就隐藏「打开 RAR」。
- ISO：`iso9660` crate 只读浏览；写入不做。
- 智能解压：单根目录时直接解到当前目录，否则新建同名目录；「解压到新文件夹」补齐。
- 外部工具按平台探测，路径可配置；失败信息要具体到缺哪个工具。

### 5.17 多语言、主题背景、RTL（P1-12）

- 翻译走现有 `.ts` 流程；新增语言是持续工作，不是一次性功能。
- 非源语言按链安装：先英文、再目标语言，缺词回落英文；简体中文是源语言，
  不装 translator。
- RTL：QML 全部加 `LayoutMirroring` 审计，重点检查侧栏、分栏、拖拽方向、图标方向。
- 背景图：QML 绘制，加对比度保护；不引入平台 API。
- 三平台一致。

### 5.18 共享、上一版本、BitLocker（P1-6）

这三项都是平台能力，不是文件管理器的核心路径；入口按平台显示，做不到就隐藏。

| 能力 | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 共享 | WinRT `DataTransferManager`（桌面需 `IDataTransferManagerInterop`；MSIX 形态需验证） | `NSSharingServicePicker`（需给 `objc2-app-kit` 开 `NSSharingService`） | 无统一 portal 入口，隐藏 |
| 上一版本 | VSS/资源管理器属性页；v1 只打开系统入口 | APFS 本地快照（`tmutil`）只读展示 | 无 |
| 磁盘加密 | BitLocker（`Win32_EncryptableVolume` / `manage-bde`）只读状态 | FileVault（`fdesetup`）只读状态 | LUKS（`cryptsetup`）只读状态 |

原则：v1 只做「显示状态 + 打开系统设置」，不在应用内解锁、加密或回滚快照。
这些操作涉及提权和数据安全，接管它们的收益远小于风险。

---

## 6. 里程碑

| 里程碑 | 范围 | 验收 |
| --- | --- | --- |
| M1 标签 | 5.1 全部；侧栏/列/搜索/设置 | 三平台本地卷可打标；exFAT/远程回退索引；12 万条列表 p95 回退 ≤ 5%；`tag:` 搜索正确 |
| M2 视图 | 5.2–5.4；每目录、分组、列宽、扩展名、图标大小（标签排序/分组随 M1 后置） | 切换目录记住布局；分组选择/框选/键盘回归；`qmllint` 通过 |
| M3 导航 | 5.12–5.14；会话恢复、标签页管理、快速预览、工具栏/主页 | 重启恢复标签与分屏；Space 弹层；关闭左/右/其他 |
| M4 集成 | 5.7–5.10、5.18；驱动器、FTP/WebDAV、自定义动作、状态中心、平台共享入口 | 弹出在三平台可用或明确隐藏；FTP/WebDAV 真实服务器冒烟；动作不经过 shell 拼接；共享/加密入口只在支持的平台出现 |
| M5 属性 | 5.11；哈希比对、EXIF、签名/ACL | 哈希比对三平台；签名仅 Windows/macOS；ACL 单独立项 |

依赖关系：M1 的标签索引是 P0-2 的「按标签排序/分组」和 P0-5 的 `tag:` 前提；
M2 不依赖 M1，可以并行；M4 的 FTP/WebDAV 与驱动器互不依赖。

---

## 7. 验证

每次合并前：

```bash
cargo test -p dae-core --lib
cargo test -p dae-qt --lib
bun run lint:qml
```

平台门槛：

- Windows：`cargo build -p dae-app`；MSIX 形态单独冒烟（打包后 xattr/ADS、驱动器、共享）。
- Linux：`qt-check` 的 ubuntu-26.04 构建；X11 与 Wayland 各跑一遍侧栏拖拽、标签、弹出。
- macOS：`qt-check` 的 macos-latest 构建（不发布，但必须编译）；xattr 与 `diskutil` 分支至少编译通过。
  没有可用设备，运行时不在验收范围内；新增 macOS 代码以「编译过」为唯一门槛。
- 手测按 `docs/qt-platform-parity.md` §4 的清单增补对应行；平台专属项标注「不可用/隐藏」而不是空白。

性能：

- 标签读取不进入枚举热路径；用 `bench/listing-pipeline` 对照 12 万条目。
- 分组/排序沿用 worker 方案；GUI 线程单轮阻塞不得超过现有预算。

---

## 8. 风险与非目标

**风险**

- 标签在 exFAT/FAT32、SMB、SFTP、云盘、压缩包上不可随文件保存；索引是唯一真相，
  必须让用户看见这个边界。
- 索引与文件系统可能不一致（外部改名、双系统、同步工具）。**已缓解（2026-11-07）**：
  文件身份让读取能找回外部改名，删除与路径复用不会继承旧标签，设置页有「重建索引」
  （只遍历索引，不扫盘）。远程路径的外部改名仍在边界内
  （`docs/decisions/tag-index-integrity.md` §6）。
- Windows Search 在 MSIX 打包形态下可能受限；先验证再承诺。
- 应用内格式化风险高；v1 明确交给系统工具。
- RAR 解码涉及非自由许可；只做外部工具探测。
- macOS 不发布，但新增 `objc2` 调用仍要过 `qt-check`，不能只在 Windows 上验证。

**非目标（本轮不做）**

- MTP/便携设备（Files 文档未列为受支持能力）。
- BitLocker/VSS 的应用内实现（只保留状态展示与系统入口，见 §5.18）。
- 紧凑覆盖窗口。
- 仅 ADS 的标签存储（Files 的已知限制，不复制）。
- 应用内格式化。
- 跨平台统一 ACL 编辑器（先保留 POSIX 模式 + DOS 属性）。

---

## 9. 参考

Files 侧：

- 命令面板（150+ actions）：<https://files.community/docs/features/command-palette>
- 布局（视图/排序/分组/列）：<https://files.community/docs/features/layout>
- 标签：<https://files.community/docs/features/tags>
- 云盘：<https://files.community/docs/features/cloud-drives>
- 侧栏：<https://files.community/docs/features/sidebar>
- FTP/FTPS：<https://files.community/docs/features/ftp>
- v4.0 发布说明（Omnibar、双窗格、过滤/搜索、Git、哈希比对、签名、状态中心）：<https://files.community/blog/posts/v4-0-0>

本仓库：

- `docs/decisions/listing-shell-costs.md` — 列表热路径预算与已实施的优化
- `docs/decisions/engine-qt-quick.md` — 外壳与引擎的边界
- `docs/qt-platform-parity.md` — 三平台对齐清单与手测表
- `docs/packaging.md` — MSIX/打包形态的限制
- `README.md` — 发布矩阵与 macOS 只编译不发布的说明

---

## 10. P0 落地记录（2026-11-05）

六项 P0 均已实现，跨 `dae-core`、`dae-qt` 与 `qml/`。这里只记录实际落地时
做出的取舍，以及仍然存在的边界；没有在本文改动设计目标。

### 10.1 文件标签（P0-1）

- 定义存 `<config>/tags.toml`；文件关联先写文件本身（NTFS ADS `dae.tags`、
  Linux `user.dae.tags`、macOS `com.dae.tags`），**同时**镜像进
  `<config>/tags-index/<volume>-<hash>.json`。索引是 `tag:` 搜索、侧栏计数和
  标签排序的真相；文件流是标签随文件移动的载体。
- 卷能力按卷探测一次并缓存（临时文件写读一次流），失败即索引模式，界面用
  `TagStore.note` 说明「此位置标签只在本机生效」。远程、压缩包直接索引模式。
- 索引键是折叠路径（Windows/macOS 大小写折叠），条目另存文件身份
  （Unix `dev:ino`，Windows 卷序列号 + 文件索引）。**2026-11-07 起**：外部改名在
  读取新路径时按身份找回并迁移，dae 内改名直接迁移；此前那条「外部改名不会关联
  回来」的边界已由 `docs/decisions/tag-index-integrity.md` 取代。
- macOS 的 Finder 标签镜像（`kMDItemUserTags`）按原计划默认关闭，因此没有
  写入代码；需要时另立任务，失败不得影响主存储。
- 读标签只在列表完成后按可见行批量请求（`DirectoryListModel.requestTags`），
  枚举热路径不读 xattr/ADS。按标签排序/分组时才读全量，仍在工作线程。
- 删除定义会从索引里剥掉该 id，但不会回写每个文件的流；没有定义指向的 id
  在读取时被忽略。

### 10.2 排序与分组（P0-2）

- `SortKey` 增加 `created`、`path`、`tag`；`foldersFirst` 迁移为
  `folderOrder = foldersFirst | filesFirst | mixed`，旧的 `foldersFirst=false`
  在首次读取时迁移为 `mixed`。
- 分组键：名称首字母（数字归入 `#`）、修改时间、创建时间、大小桶、类型、标签；
  日期桶可选年/月/日。标签分组按定义顺序排列名称，未打标签的一组排最后。
- 分组在 Rust 侧把 `group_rank`/`group_label` 预计算到行上，worker 排序只读
  两个值；QML 用 `ListView.section` 画分组头。**网格视图不画分组头**：
  `GridView` 没有 `section`，排序仍按组进行，这是与 Files 的已知差异。
- 列表的框选改为 `indexAt` 求首尾行，分组头的高度因此不会让选区错行。
- 创建时间：Windows/macOS 取 birth time；Linux 取 `Metadata::created()`，
  文件系统没有 `STATX_BTIME` 时回退 `ctime` 并在行上标记
  `createdFallback`，列里显示「（回退）」。

### 10.3 每目录布局（P0-3）

- 每目录布局存 `<config>/views-qt.toml`（`[views."<折叠路径>"]`，LRU 512），
  全局默认仍在 `explorer-qt.ini`。`syncLayout` 打开时只读写全局文档。
- `currentPath` 写入即恢复该目录布局；没有记录时回到全局默认。
- 自适应规则（Rust，可单测）：文件 ≥ 8 且图片占比 ≥ 60% → 网格；路径深度
  ≥ 3 → 分栏；否则列表。只在「自适应开 + 每目录布局开 + 该目录没有记录」时
  生效，改一次菜单即可覆盖。

### 10.4 详情列、扩展名、图标大小（P0-4）

- 列宽与可见性存进每目录布局（`columns`、`columnWidths`）；表头可拖动分隔线
  改宽，双击按标题/样例自适应。窄窗按类型 > 大小 > 修改 > 创建 > 标签的顺序
  依次隐藏。
- 「显示扩展名」只影响绘制；排序、搜索、重命名仍用完整名称。
- 图标大小 24–96 dp，列表缩略图与网格瓦片共用；视图菜单底部是滑杆，网格的
  瓦片高度/最小宽度随它推导。
- 「标签」列默认隐藏（默认 `type,size,modified`），可从视图菜单或命令面板打开。

### 10.5 搜索（P0-5 阶段 1）

- 范围：当前目录、收藏、最近、所有本地卷；多根在 Rust 侧合并、统一排序并
  截断到 200。任何非本地根按原契约拒绝（`fs.search_local_only`）。
- 过滤器：类型/修改时间/大小，与列表过滤器共用词表；修改时间在 Qt 侧换算成
  本地午夜/7 天/30 天的毫秒边界，避免时区漂移。
- `tag:` 语法（半角/全角冒号均可，逗号分隔多个）在 core 解析并解析成定义 id；
  指向未定义标签时返回空结果而不是报错。`tags` 参数另供侧栏点击使用。
- 搜索历史 20 条存 `explorer-qt.ini`（单行 JSON），搜索栏的「历史」chip 打开
  菜单；阶段 2 的平台索引（Windows Search / Spotlight / tracker3）未做，仍是
  并行遍历回退。

### 10.6 信息面板（P0-6）

- `PreviewPanel` 的详情复用 `PropertiesController`：类型、大小、修改、创建、
  权限摘要、所有者、按需哈希（MD5/SHA-1/SHA-256）。
- 标签编辑：面板列出全部定义，选中的标签为实心；点击对**整个选择**切换。
  多选时显示数量、可测文件合计大小和共同标签；没有共同标签时标签仍以未选中
  状态提供批量打标。
- 远程条目沿用属性控制器的降级规则（不可读字段留空/不显示），不新增后端。

### 10.7 验证

- `cargo test -p dae-core --lib`：325 passed / 10 ignored（新增 tags、搜索过滤、
  标签解析、创建时间编解码用例）。
- `cargo test -p dae-qt --lib`：189 passed / 1 ignored（新增排序、分组、每目录
  布局迁移、自适应规则用例）。
- `DAE_QML_LINT=1 DAE_QML_TYPEC=1 cargo build -p dae-app`：通过（qmllint 零告警、
  qmltc 通过）。
- Windows 手测：标签写入 NTFS ADS + 索引；列表标签列、侧栏计数、按类型分组、
  创建时间列、多选信息面板（数量/合计大小/标签编辑）均按上述行为显示。
  Linux/macOS 运行时未在本轮验证；`qt-check` 的三平台编译仍是门槛。

---

## 11. P1 落地记录（2026-11-05）

P1 的十二项按里程碑 M3–M5 推进，取舍记在 `docs/decisions/p1-integrations.md`。
下表是实际状态：**完成**指界面入口、后端与降级规则都在；**部分**指能用的部分已落地、
边界写在后面。括号里是完成度之外的限制（未覆盖的形态、比计划小的动作面），
剩余待办统一在 §12。

| # | 项 | 状态 | 边界 |
| --- | --- | --- | --- |
| P1-1 | 驱动器工具 | 完成 | 弹出/卸载/挂载/格式化入口/存储清理/加密状态/上一版本按平台能力显示；Windows 弹出在真实 USB 卷上返回 ok。`CM_Request_Device_Eject` 回退按 `docs/decisions/p1-integrations.md` §1 不做 |
| P1-2 | FTP/WebDAV | 完成（隐式 FTPS 未测） | FTP/FTPS 完整（浏览、下载、上传、改名、删除、建目录）；WebDAV 同样完整，上传走流式 PUT。对本地 FTP、显式 FTPS、WsgiDAV（匿名/Basic）冒烟通过；隐式 FTPS（990）与真实第三方服务器未覆盖 |
| P1-3 | 自定义动作 + 快捷键 | 完成（动作面小于 Files） | `[[actions]]`、argv 传参、PATH 探测、设置页增删改、内置「在 IDE 中打开」；可重绑定快捷键 22 → 35，全部有分发点。未绑定的菜单命令仍多，没有补齐到 Files 的 150+ 动作面 |
| P1-4 | 状态中心 | 完成 | 内存 200 条 / 持久化 50 条、失败徽章、传输与解压可重试、可清空 |
| P1-5 | 属性增强 | 部分 | 哈希比对、图片 EXIF、音频/视频容器元数据、Windows 签名（含 catalog 回退）均已落地；ACL 在 Windows（DACL）与 Linux（POSIX ACL）可读可编辑，见 §11.4；macOS 签名、macOS ACL、自定义图标未做 |
| P1-6 | 共享 / 上一版本 / BitLocker | 部分 | 磁盘加密状态（`manage-bde`/`fdesetup`/`cryptsetup`）与上一版本系统入口在驱动器菜单；Windows 系统共享已实现并冒烟；macOS 共享、Linux 共享按平台隐藏 |
| P1-7 | 标签页 + 会话恢复 | 完成 | 关闭栈 16、重开/复制/固定/关闭其他与左右；几何+标签+历史路径+分屏恢复，崩溃标记提示 |
| P1-8 | 快速预览 | 完成 | 停靠面板关闭或窗口过窄时 Space 打开弹层；Esc/外点关闭，方向键与翻页滚动 |
| P1-9 | 工具栏 / 主页小部件 | 完成（组件集与计划不同） | 主页四组（项目/收藏/最近/空间）可开关并持久化；工具栏模型 + 拖拽排序 + 恢复默认。计划里的「标签」「驱动器」部件未做 |
| P1-10 | 大小单位 + 按需文件夹大小 | 完成 | 二进制 KiB/MiB 与十进制 KB/MB 可切（修正了旧的 1024 进制写 KB 的错标）；列表菜单按需计算目录大小 |
| P1-11 | 压缩格式 | 完成 | 智能解压（单根目录不套娃）、解压到新文件夹；RAR/ISO 经 7z/unrar/bsdtar，缺工具时入口隐藏并给出具体错误 |
| P1-12 | 多语言 / RTL / 背景 | 部分 | `LayoutMirroring` 在窗口根开启并继承，未做逐组件镜像审计；语言为简中（源）、英文、繁体中文；背景图与四档遮罩已落地。语言数是持续工作，远未到 Files 的量 |

### 11.1 驱动器工具（P1-1）

- `fs/drives.rs`：`capabilities()` 按平台声明能力；Windows 弹出走
  `DeviceIoControl(IOCTL_STORAGE_EJECT_MEDIA)`（先 `FSCTL_LOCK_VOLUME`、
  `FSCTL_DISMOUNT_VOLUME`，失败不阻断），格式化走 `SHFormatDrive`，清理打开
  `ms-settings:storagesense`；Linux 用 `udisksctl` + `eject`/`gio`，格式化打开
  `gnome-disks`；macOS 用 `diskutil`，格式化打开「磁盘工具」。
- 加密状态只读：Windows `manage-bde -summary` 取三行，macOS `fdesetup status`，
  Linux `cryptsetup status <dev>` 取 `type:`。上一版本只打开系统入口。
- 侧栏驱动器行新增 `removable` 角色；右键菜单按 `Workspace.driveCapabilities`
  过滤，Windows 无「卸载」、Linux/macOS 无「清理」即不建行。破坏性动作（格式化）
  二次确认并由系统对话框执行。

### 11.2 自定义动作与快捷键（P1-3）

- `AppSettings.actions: Vec<CustomAction>`；`normalize_actions` 丢掉空 id/命令、
  去重、收敛 selection 与扩展名。
- `fs/actions.rs`：`command_available` 扫 `PATH`（Windows 试 `exe/cmd/bat/com`）；
  参数按 argv 展开，`%f/%F/%d/%u` 不经过 shell；`selection_allows` 判
  any/file/directory/multiple 与扩展名。
- 菜单：行菜单第二页「自定义动作」子菜单，按当前选择过滤；执行失败走既有
  `operationFailed`，进状态中心。
- 设置页新增「动作」：逐条改名称/命令/参数/适用选择，支持新增、删除、恢复内置。
- 快捷键：默认表 22 → 35 条，新增 `app.newWindow/newTab/closeTab/reopenTab/
  nextTab/previousTab/toggleSidebar`、`explorer.newFolder/properties/refresh/
  toggleSplit/displayOptions/filters`；QML 里原本写死的组合键改为
  `shortcutSequence(<id>)`，设置页分组同步补齐。

### 11.3 状态中心（P1-4）

- `Workspace` 维护 `OperationRecord` 环形缓冲（200 条），`begin_operation` 压开始时间，
  `finish_operation` 落记录并写 `<config>/operations-qt.json`（最近 50 条，原子写）。
- `operationsJson` 按完成时间倒序；`failureCount` 为未读失败数，打开面板清零。
- 重试只对 copy/move/extract 开放，参数随记录存 JSON；`retryOperation(id)` 重新发起。
- `qml/StatusCenter.qml`：状态栏右侧时钟按钮 + 失败徽章；面板列出状态点、标签、
  失败原因或时间、重试与「清除记录」。

### 11.4 哈希比对与元数据（P1-5）

- 列表菜单「比较 SHA-256」：恰好两个本地文件时可用，`hashing::digests` 各读一遍，
  结果以 toast 给出。
- 属性对话框校验页新增「粘贴已知的 MD5/SHA-1/SHA-256」：三种值都算完后按算法给
  「与 X 校验值相同 / 都不匹配」，颜色跟随结果。
- **EXIF**：`fs/media_meta.rs` 用 `kamadak-exif` 读图片头，预览面板的元数据块在
  「分辨率」之外增加相机、镜头、拍摄时间、曝光、光圈、ISO、焦距与 GPS；
  `qml/Localize.js` 负责这些标签的中英切换。没有 EXIF 的 JPEG 仍正常预览。
- **文件签名**：`fs/security.rs` 调 `WinVerifyTrust`，内嵌签名返回 `TRUST_E_NOSIGNATURE`
  时再按 SHA-256 哈希查系统 catalog（System32 的多数二进制是 catalog 签名），
  状态收敛为 `valid/unsigned/untrusted/invalid`；属性对话框新增「签名」页显示
  状态、签名者、颁发者与平台原因。Windows 独占，Linux/macOS 隐藏该页。
- **ACL**：`fs/security.rs` 在 Windows 用 `GetNamedSecurityInfoW` 读 owner/group/DACL，
  Linux 读 `system.posix_acl_access` xattr（缺失时回退 mode）。属性对话框有独立
  「安全」页：条目行（允许/拒绝、权限预设、继承标记）、继承开关（Windows）与
  添加/移除表单；继承条目只读。写回走 `SetNamedSecurityInfoW`（deny 先于 allow；
  关闭继承时先把继承 ACE 转成显式）与 `setxattr`（named 列表清空时 `removexattr`
  回到纯 mode）。`rights == "custom"` 表示「保留该主体当前的掩码」，未改动的条目
  因此无损往返。macOS 仍无入口。
- `security::signature_supported()` / `acl_supported()` 决定入口是否出现；
  平台读取在 `spawn_blocking` 里与元数据并行，慢的证书链不会拖住对话框。
- 单测：`fs::security` 对 `C:\Windows\System32\notepad.exe` 断言 `valid` 且有签名者，
  对临时文件断言 `unsigned`；Windows ACL 用例在临时文件上写显式 ACE、切换继承、
  断言 custom 保留原掩码与未知主体报错；Linux 用例往返 named user/group 与 mask、
  断言空列表移除 xattr（不支持 POSIX ACL 的文件系统上跳过）。

### 11.5 标签页与会话恢复（P1-7）

- 每个窗口 16 条已关闭标签（surface/path/spaceId/pinned/历史路径）；右键标签页有
  复制、固定、关闭、关闭其他/左侧/右侧、重新打开；`Ctrl+Shift+T` 也在动作表内。
- `Tab.pinned` 参与「关闭其他/左右」的保留规则；标签条用信号点标记。
- `crates/dae-qt/src/session.rs`：`<config>/session-qt.json`，每窗口一条记录，
  `clean` 标记区分正常退出与崩溃；启动时第一个窗口消费一次并恢复几何、标签、
  历史路径与分屏开关；恢复后删掉旧 token 的记录，避免下一次启动出现重复窗口。

### 11.6 快速预览（P1-8）

- `qml/QuickPreview.qml`：遮罩 + 760×680 面板复用 `PreviewPanel`；Esc、左/右方向键、
  空格或点击遮罩关闭；上/下/PageUp/PageDown 调 `PreviewPanel.scrollBy`。
- `togglePreview(path)`：停靠面板可见且是同一路径时关闭它；否则当面板关闭、窗口
  窄于 840 dp 或当前不是文件夹视图时打开弹层。`Space` 的语义因此不再改变布局。

### 11.7 主页小部件与工具栏（P1-9）

- `OverviewModel.setHiddenSections(csv)` 在重建时过滤分组；`ExplorerPrefs.hiddenHomeSections`
  持久化到 `explorer-qt.ini`，设置页「主页小部件」四个开关即时生效。
- 工具栏是模型：`toolbarLeading`（默认 `back,forward,up`）与 `toolbarTrailing`
  （默认 `search,trash,view,more`）存进 `explorer-qt.ini`，路径栏用 `Repeater`
  + `Loader` 按 id 实例化组件。存储值在 QML 里按本构建的 id 集合归一化：
  未知 id 丢弃、缺失 id 追加，所以旧配置不会留下空位，新动作会自动出现。
- 拖拽：每个 delegate 上的 `DragHandler` 在释放时按指针位置算插入点，调用
  `moveToolbarItem` 重排并立即持久化；设置页「恢复默认」写回两组默认顺序。
- 组件在路径栏末尾声明，Loader 的 `sourceComponent` 通过 `Qt.callLater` 延迟到
  整棵树创建完成后再取，避免创建期读到尚未存在的 Component。

### 11.8 大小单位与按需文件夹大小（P1-10）

- `format::SizeUnits { Binary, Decimal }`：二进制写 `KiB/MiB`（修正旧标签），十进制按
  1000 写 `KB/MB`。`dae_qt::init` 在第一次枚举前从 `explorer-qt.ini` 应用；
  `setString("sizeUnits", …)` 立即切换并影响所有后续行。
- 列表面板的文件夹大小仍是「—」，菜单「计算大小」调用 `Workspace.measureFolders`，
  后台并行走目录、结果经 `folderSizesMeasured` 写回模型并按行 `dataChanged`；
  再次调用会取消上一轮。取消入口 `cancelMeasureFolders`。

### 11.9 压缩格式（P1-11）

- `ArchiveFormat` 增加 `Rar`/`Iso`；识别只用于解压，创建返回明确错误。
- 加密创建：菜单「ZIP 压缩包（加密）」与「7Z 压缩包（加密）」先弹密码对话框
  （双输入校验），ZIP 走 WinZip AES-256（method 99 + AE-2 extra），7z 走头部加密；
  解压侧两者原本就支持，密码错误给具体报错。
- `top_level_root` 判断「全部条目在同一顶层目录」，命中时解到压缩包所在目录；
  「解压到新文件夹…」复用 `MoveToDialog` 的 `extractSource` 模式。
- 外部工具顺序：`7z/7zz/7za` → `unrar`（仅 rar）→ `bsdtar`；`external_formats()`
  把可用格式交给 QML，入口据此显示；缺失时报「缺哪个格式的工具」。

### 11.10 平台入口（P1-6）

- 驱动器菜单：加密状态（只读文本）、以前的版本（Windows 打开属性页、macOS 打开
  Time Machine）。
- **系统共享（Windows）**：`crates/dae-qt/src/share.rs` 走
  `IDataTransferManagerInterop`——桌面进程没有 `CoreWindow`，所以用
  `GetForWindow` / `ShowShareUIForWindow`。选中的文件先在 MTA 线程解析成
  `StorageFile`（`StorageFile` 只收 Win32 反斜杠路径；列表里的正斜杠路径先归一化，
  否则 `GetFileFromPathAsync` 报 `ERROR_BAD_PATHNAME`），以 `AgileReference` 交回
  UI 线程，再由自定义 `IIterable<IStorageItem>` 交给 `DataPackage`。
  `TypedEventHandler` 注册在窗口的 `DataTransferManager` 上，最后一次注册保存在
  线程局部变量里，下一次共享替换它。
- 右键「更多…」里的「共享…」只在 `Workspace.shareSupported`（Windows）且为本地
  路径时出现；失败时 toast 报「无法打开系统共享面板」。macOS/Linux 隐藏。
- 冒烟：在 Windows 上对 `README.md` 调 `Workspace.sharePaths` 返回 true，系统共享
  面板实际弹出并显示文件与共享目标（就近共享、微信、WorkBuddy、记事本、Outlook）。

### 11.11 RTL 与背景图（P1-12）

- `ShellWindow` 根开启 `LayoutMirroring.enabled: Qt.application.layoutDirection ===
  Qt.RightToLeft` 与 `childrenInherit`。侧栏、分屏、菜单等按 anchors 镜像；
  方向性图标（chevron/返回箭头）不翻转，含义随布局。
- **背景图**：`ExplorerPrefs.backgroundImage`（本地路径）与 `backgroundDim`（0–90）
  存进 `explorer-qt.ini`。`ShellWindow` 在画布与内容之间铺一张
  `Image { fillMode: PreserveAspectCrop }`；`Theme.content` / `Theme.contentAlt`
  在设置图片后变成带 alpha 的 canvas/canvasAlt，列表、网格、分栏、概览、空间、
  回收站与内容搜索结果都改用它，图片才会从内容后面透出来。chrome、侧栏与对话框
  保持实色，这是对比度保护：文字始终落在实色或足够厚的半透明层上。
- 设置页「背景图」有路径输入、`FileDialog` 选择、清除，以及「无遮罩/轻/中/强」
  四档遮罩；`QtQuick.Dialogs` 是新增的 QML 依赖，发布时由
  `windeployqt --qmldir qml` 一并部署。
- 冒烟：用纯红测试图确认内容区、侧栏与路径栏都透出背景；清除后回到实色。

### 11.12 FTP/FTPS 与 WebDAV（P1-2）

**FTP/FTPS**（`fs/ftp.rs`）

- 连接池：每服务器最多 8 条控制连接（一次同服务器复制并发 4 + 4），`Condvar`
  等待上限 60 s；连接错误丢弃重开，只重放幂等的 `CWD`。单连接 + `Mutex` 会在
  `par_iter` 的并发复制里互相等待。
- 列表：`FEAT` 声明 `MLSD` 时用 MLSD，否则退回 `LIST`；两种格式都手写解析
  （MLSX facts、`ls -l` 的名称含空格/符号链接/无组行、无年份时间戳的「未来即去年」规则）。
- 传输：`retr_as_stream` / `put_with_stream` 包装成 `Read`/`Write`，不整文件缓冲。
  数据连接最终的 `226` 在 drop 时读走：读失败记 `debug`，上传失败记 `warn`
  （引擎先 flush 再 drop，`Write::flush` 不能既关数据连接又保持可写）。
- 加密：21 明文、990 隐式 FTPS、`ftps://` 显式 `AUTH TLS`；信任锚用系统证书库
  （`rustls-native-certs`），不做按主机 TOFU。
- `stat` 依次试 `MLST` → `SIZE`+`MDTM` → `CWD` 目录探测；`remove` 对目录用
  `RMDIR`、文件用 `DELE`；`rename_to` 是 `RNFR`/`RNTO` 并校验同 authority。

**WebDAV**（`fs/webdav.rs`）

- PROPFIND Depth 1/0、MKCOL、DELETE、MOVE、GET；quick-xml 按 local name 解析
  （`d:`/`D:`/`ns0:`/无前缀都接受），href 解码一次，RFC 1123 手写解析。
- **上传**：`open_write` 返回一个写适配器，把字节按 64 KiB 分块推进 4 深度的
  有界通道；专用线程用 `reqwest::blocking::Body::new(ChannelReader)` 发流式 PUT，
  `flush()` 关闭通道并等待服务器最终状态。队列满时 `write` 阻塞，所以大文件不会
  被读进内存；`flush` 成功即代表服务器已收下文件。响应非 2xx 时保留服务器正文
  并映射为既有错误类型。
- 单测：进程内 HTTP/1.1 服务器验证 200 KiB 分块 PUT 的字节与顺序，以及 403 时
  `flush` 把服务器原因带回。
- 真实服务器冒烟：本地 WsgiDAV（匿名与 Basic 认证）跑通浏览、上传、同后端复制、
  改名、删除；`DAE_TLS_CA_FILE` 是 FTP/FTPS 的私有 CA 补充信任锚。

**接线**

- `vfs::resolve` 接上 `Scheme::Ftp` 与 `Scheme::WebDav`；`split_scheme` 增加 `ftps`。
- 连接对话框新增 WebDAV（默认端口 80，443 视为 `webdavs`）与 FTP/FTPS（默认 21，
  990 隐式 TLS）；`smb::test_connection` 增加两个分支，用表单里的凭据在保存前探测。
- `smb::split_authority` 修正带端口的 IPv6 authority（`[::1]:2121/pub` 之前把
  `:2121/pub` 当成路径，端口丢失）。

**网络发现**不在 P1-2 的落地范围内（当时侧栏只有已保存的连接）。它作为 R-2 于
2026-11-07 补齐：`fs/discovery.rs` 的一次性 mDNS 探测 + 侧栏「网络」段，细节见 §12.7。

### 11.13 验证

- `cargo test -p dae-core --lib`：356 passed / 10 ignored（新增 WebDAV 分块上传与
  403 传递、Windows 签名/catalog 回退、ACL 读取；FTP/WebDAV 解析与连接池用例仍在）。
- `cargo test -p dae-qt --lib`：193 passed / 1 ignored。
- `DAE_QML_LINT=1 DAE_QML_TYPEC=1 cargo build -p dae-app`：通过（qmllint 与 qmltc
  均通过；`Qt.application.layoutDirection` 的 qmllint 误报用行级 directive 抑制）。
- Windows 运行时冒烟：
  - 预置 `toolbarLeading=up,back,forward`、`toolbarTrailing=more,view,search,trash`，
    路径栏按该顺序加载；调用 `finishToolbarDrag("trailing", 0, …)` 后顺序变为
    `view,search,trash,more` 并写回 `explorer-qt.ini`。
  - 预置红色测试背景图后，列表、侧栏与路径栏都透出图片；清除后回到实色。
  - 对 `README.md` 调 `Workspace.sharePaths` 返回 true，系统共享面板实际弹出并显示
    文件与共享目标。
  - `drives::eject("E:")` 对真实 USB 卷返回 ok，`E:` 消失；格式化仍只打开系统对话框，
    没有在应用内执行。
- 真实服务器冒烟（本机临时服务）：FTP 明文、显式 FTPS、WsgiDAV 匿名与 Basic 认证
  均跑通「浏览 → 写文件 → 预览 → 同后端复制 → 改名 → 删除」。隐式 FTPS（990）与
  系统 catalog 之外的签名场景未覆盖。
- 未验证：格式化对话框的人工确认；macOS 无设备，按 §7 只验编译。拖放、列宽
  拖拽/双击自适应与标签导入/导出对话框已在 2026-11-08 的 Linux 真机复验中通过
  （§12.3）。

---

## 12. 剩余工作（2026-11-07）

**交付平台收敛为 Windows 与 Linux。** macOS 无设备、不发布，代码保持可编译即可，
运行时不进入验收（§7）。因此 §5 里所有「macOS 独占实现」从功能面移到 §12.2，
不再是交付阻塞项。

### 12.1 待办：Windows / Linux 交付面

| # | 缺口 | 依据 | 落点 | 量 | 状态 |
| --- | --- | --- | --- | --- | --- |
| R-1 | 平台索引搜索 | §5.5 阶段 2 | Windows：`ISearchQueryHelper` + 索引可见性探测（MSIX 形态单独验）；Linux：`tracker3`/`baloo` 存在时调用，否则隐藏开关。结果与遍历结果合并去重，沿用现有取消令牌与 200 条上限 | L | **完成**（2026-11-07，`docs/decisions/search-index.md`；MSIX 形态未验） |
| R-2 | 网络位置发现 | §5.8 | `mdns-sd` 浏览 `_smb._tcp`/`_sftp-ssh._tcp`/`_webdav._tcp`，结果只填主机与端口，不自动带凭据；侧栏新增「网络」段。Windows 的 WS-Discovery 后置 | S–M | **完成**（2026-11-07，`docs/decisions/network-discovery.md`） |
| R-3 | 标签定义导入/导出 | §5.1 UI | `TagStore` 增加导出/导入（格式即 `tags.toml` 文本），设置页两个按钮 + 文件对话框。导入策略要在实现时定一条并写进决策文档：同 id 覆盖名称/颜色，未知 id 追加 | S | **完成**（2026-11-07，`docs/decisions/tag-index-integrity.md`） |
| R-4 | 标签索引重建与外部改名修补 | §8 风险 | `tags.rs` 无 rebuild/repair API。重建入口放标签设置页，按卷分片重读；watcher 事件带 `dev:ino` 时把索引键从旧路径迁到新路径，补上「外部改名不关联」这条已知边界 | S–M | **完成**（2026-11-07，同上。实现改为按文件身份在读取时恢复，不依赖 watcher 的 rename 对） |
| R-5 | 主页部件「标签」「驱动器」 | §5.14 | `OverviewModel` 当前只有项目/收藏/最近/空间；补两段并接入 `hiddenHomeSections`，计数分别来自 `TagStore.counts()` 与侧栏卷列表 | S | **完成**（2026-11-07） |
| R-6 | 动作面补齐 | §3 P1-3 | 可重绑定快捷键 35 条，菜单命令仍多（标签、大小单位、测量大小、压缩、Git、打开方式、复制路径等）。按动作注册表逐项补 id 与分发点，冲突检测沿用现有逻辑 | M | **完成**（2026-11-07，13 条新动作，35 → 48；默认只给「复制路径」绑定 Ctrl+Shift+C，其余留空待用户绑定） |
| R-7 | 自定义图标 | §5.11-5 | 三平台语义不一致，维持「暂缓」，不进待办 | — | 暂缓 |

### 12.2 仅代码、不验收：macOS

以下四项只在出现 macOS 发布计划时再做；做了也只要求 `qt-check` 的 `macos-latest`
编译通过与入口按平台隐藏，**不承诺行为**：

- `SecStaticCodeCheckValidity` 文件签名（属性对话框「签名」页）；
- `NSSharingServicePicker` 系统共享（右键「共享…」）；
- `kMDItemUserTags` Finder 标签镜像（binary plist，失败不得影响主存储）；
- `QLPreviewPanel` 快速预览（`QuickPreview.qml` 的替代后端）。

### 12.3 验证与测量尾巴

- **Linux 运行时（WSL2 预验，2026-11-07）**：`docs/qt-platform-parity.md` §4 里
  11-01 之后落地的功能行在 WSL2（CachyOS、Qt 6.12、WSLg 的 Wayland 与 XWayland
  两种 QPA、软件渲染）上跑过：标签写入（`user.dae.tags`）与 `tag:` 搜索、排序与
  分组、每目录布局、详情列显示与勾选、搜索范围、信息面板、索引重建、网络发现
  （mDNS 广告）、主页部件、系统索引遍历回退、动作快捷键都符合期望。本轮修掉
  两个跨平台缺陷，细节与已知小问题见 parity §4：
  1. 详情列全开时 Name 被挤到 0 宽 —— `qml/FileList.qml` 给 Name 留 100 px，
     装不下时按序丢尾部列；
  2. 命令行/深链直接打开目录时不恢复该目录已存的布局 ——
     `qml/ShellWindow.qml` 启动标签页分配完后同步一次 `currentPath`。
  当时留下的拖放、列宽拖拽/双击自适应、标签导入/导出文件对话框，已在
  2026-11-08 的 Linux 真机复验中通过。
- **Linux 真机复验（2026-11-08）**：用户在物理机（CachyOS，Arch 系）上按 parity
  §4 全表重跑，全部符合期望。11-07 的两个跨平台修复（Name 列、启动布局恢复）
  与 11-08 的两处修正（缺失英文文案、单行目录切换分组后分组头刷新）一并确认；
  Linux 运行时回归至此没有开口项。
- **M1 验收指标已测（2026-11-07）**：12 万条目列表 p95 相对同管线（不含标签
  工作）为 **+3.52%**，预算 +5%，达标。测量是 `crates/dae-qt/src/model.rs` 的
  ignored 测试 `reports_the_tag_group_listing_regression`，跑真实路径
  （`open_canonical_listing` 批次、`from_listing`、`decorate_tags`、工作线程排序
  三件套）；跑法与数字见 `bench/listing-pipeline/README.md`「M1 acceptance」。
  基准先量出一处真回退：刷新时缓存已有首屏标签，`decorate_rows` 仍逐行重建
  `full_path`（+10.9%），已改为按名/按 path 借用查找（`decorate_tags`）。分组是
  可选视图（默认 `GroupKey::None`），成本单列：名字分组 median 115 ms、月份分组
  148 ms，不吃列表预算；512 可见行的标签读取发生在列表之后（median 25 ms），
  不在等待里。测量过程中修掉 `crates/dae-qt/build.rs` 重复加入 `src/tags.rs`
  导致同一 `.o` 被并行编译两次的间歇 C1083。
- **MSIX 形态冒烟已记录（2026-11-07）**：§7 的 ADS / 驱动器 / 共享三项与系统索引
  可见性全过，0.5.1.6 重打包后复验；步骤与输出见 `docs/packaging.md` §3.3。冒烟
  修掉 `manage-bde -summary` 的非法开关（24H2 报 Invalid Syntax）。
- **加密压缩与 ACL 编辑器（2026-11-12）**：补齐两个缺口。加密创建新增 ZIP
  （WinZip AES-256，此前只有 7z 能加密创建）；ACL 编辑器覆盖 Windows DACL 与
  Linux POSIX ACL，同一「安全」页（§11.4）。验证：
  - `cargo test -p dae-core --lib`：405 passed / 10 ignored（含加密 ZIP 创建与
    Windows/Linux ACL 写入往返新用例）；`cargo test -p dae-qt --lib`：
    208 passed / 2 ignored。
  - Linux 侧（WSL CachyOS）`cargo check -p dae-core` 与 POSIX ACL 往返用例通过；
    Linux 的 Qt 编译留给 qt-check CI。
  - `DAE_QML_LINT=1 DAE_QML_TYPEC=1 cargo build -p dae-app` 通过。顺带修了两处
    既有 qmllint 报错：`Accessible.ignored` 挂在 `TapHandler` 上
    （`qml/TitleBar.qml`、`qml/TrashView.qml`）。
  - Windows 运行时冒烟：临时文件经「安全」页添加 Everyone 读取的显式 ACE 保存，
    `Get-Acl` 独立复核命中；再经页面移除保存，继承条目全程不受影响。菜单
    「ZIP 压缩包（加密）」→ 密码对话框 → 压缩产出 test.zip，局部头加密位、
    method 99 与 AE-2 strength 3（AES-256）均正确。
- **隐式 FTPS（990）** 与格式化对话框的人工确认未覆盖（§11.13）。
- **macOS 运行时**：不做，不是待办。

### 12.4 维持不做

紧凑覆盖窗、MTP/便携设备、Explorer 标签互操作、系统通知（原 P2）；应用内格式化；
macOS ACL 编辑器（Windows/Linux 已于 2026-11-12 落地，见 §11.4）；仅 ADS 的标签存储。

### 12.5 剩余顺序

§12.1 的功能项已全部落地，剩下的是验证与测量：

1. ~~Windows MSIX 形态冒烟~~（2026-11-07 完成，见 §12.3）；
2. ~~M1 的 12 万条 p95 基准~~（2026-11-07 测得 +3.52%，见 §12.3）；
3. ~~Linux WSL2 预验~~（2026-11-07 完成，见 §12.3）；~~Linux 真机手测~~（2026-11-08
   完成，CachyOS；含拖放、列宽拖拽/双击自适应、标签导入/导出，见 §12.3）。

### 12.6 R-3 / R-4 落地记录（2026-11-07）

- **core**（`crates/dae-core/src/fs/tags.rs`）：`export_document`/`export_to`、
  `import_document`/`import_from`、`repair_index`、`migrate`、`tags::note_rename`；
  `TagDefinition.color` 加 `serde(default)`，手改的 `tags.toml` 少一个颜色字段不会
  让整份定义读成空。
- **文件身份**：索引条目新增 `file_id`——Windows 用
  `GetFileInformationByHandle`（卷序列号 + 文件索引，目录带
  `FILE_FLAG_BACKUP_SEMANTICS`），Unix 用 `dev:ino`。读取时用它做两件事：命中但
  身份不同 → 视为未命中（路径被新文件复用）；未命中且身份匹配别的键 → 迁到新键
  （dae 外改名）。
- **重命名挂钩**：`service::rename_entry`、`undo::apply_rename_pairs`（含批量暂存
  改名）、`undo_rename`/`redo_rename` 成功后迁移条目；目录按前缀迁移子项。
- **重建索引**只遍历索引，不扫盘：丢弃失效路径、采用文件流里的 id、按身份去重、
  损坏分片隔离为 `*.corrupt-<millis>`、按当前卷重新分片。
- **bridge**（`crates/dae-qt/src/tags.rs`）：`exportDefinitions`、
  `importDefinitions`、`rebuildIndex`，统一 `{"ok":…}` JSON 回复；回复构造是自由
  函数，单测不需要 QML 引擎。
- **UI**：设置 → 标签底部「导出… / 导入… / 重建索引」+ 一行结果说明（跳过项按
  前三条列名展开）。
- **验证**：`cargo test -p dae-core --lib fs::tags` 17 passed；
  `cargo test -p dae-qt --lib tags` 3 passed；
  `DAE_QML_LINT=1 DAE_QML_TYPEC=1 cargo build -p dae-app` 通过。
- **Windows 运行时冒烟**（隔离 `APPDATA`）：预置一个指向已删除文件的索引分片 →
  Ctrl+, 开设置 → UIA 选中「标签」页 → 按「重建索引」→ 状态行显示
  「索引已重建：0 条记录，移除 1 条失效、0 条重复、0 个损坏分片」，该分片文件被删除。
  导入/导出的文件对话框未自动点选，其桥接与合并语义由上面的单测覆盖。
- 取舍与边界见 `docs/decisions/tag-index-integrity.md`（远程路径的外部改名不在
  本轮范围内）。

### 12.7 R-2 落地记录（2026-11-07）

- **core**（`crates/dae-core/src/fs/discovery.rs`）：`discover(timeout)` 用
  `mdns_sd::ServiceDaemon` 浏览三种服务类型，一次探测、到点关守护；结果按
  协议 + 主机 + 端口去重，地址合并排序；主机名去掉尾点，没有主机名时回退到
  第一个地址。守护起不来只记 debug 日志，返回空表——发现是附加项，不是门槛。
- **不回退凭据**：TXT 记录不读，因此广告方能写的东西（用户名、路径、口令）不会
  进入对话框；这也是「结果只填主机/端口」的实现含义。
- **bridge**（`crates/dae-qt/src/network_discovery.rs`）：`NetworkDiscovery` 单例，
  `servicesJson` / `busy` / `scanned`，`refresh()` 在共享 runtime 的阻塞池上跑。
  `scanned` 让第二个窗口不再重复探测。
- **UI**：侧栏在「标签」之上新增「网络」段（`qml/Sidebar.qml`），行显示实例名与
  协议，右上角刷新；点击通过 `discoveredActivated(protocol, host, port)` 交给
  `ShellWindow`，`ConnectDialog.openWith` 预填协议/主机/端口，账号与密码留空。
  同时删掉了连接对话框里「FTP / WebDAV 即将支持」的过期说明。
- **验证**：`cargo test -p dae-core --lib` 369 passed（新增 5 个发现用例：
  去重合并、同主机不同协议、主机名缺失回退地址、丢弃空主机/零端口、实例名截断）；
  `cargo test -p dae-qt --lib` 196 passed（新增单例默认态）；
  `DAE_QML_LINT=1 DAE_QML_TYPEC=1 cargo build -p dae-app` 通过。
- **Windows 运行时冒烟**（隔离 `APPDATA`，本机临时 mDNS 广告器注册
  `Dae Smoke NAS._smb._tcp`）：侧栏出现「网络 / Dae Smoke NAS SMB」；点击后连接
  对话框以 SMB、`dae-smoke.local`、`445` 打开，账号与密码为空（UIA 读到的
  Edit 值为证）。冒烟同时修掉两处 qmltc 抓不到的运行时问题：重复的
  `Component.onCompleted`（`Sidebar.qml`）与 `LazySurface` 缺 `openWith` 代理。
- **边界**：只监听局域网组播，跨网段/防火墙拦截时无结果（与系统文件管理器一致）；
  Windows 的 WS-Discovery 未做；结果不持久化，重启后再探一次。

### 12.8 R-1 / R-5 / R-6 落地记录（2026-11-07）

- **R-1 平台索引**（`crates/dae-core/src/fs/search_index.rs`）：Windows 用
  `ISearchQueryHelper` 生成 SQL、`Search.CollatorDSO` OLE DB 执行；列绑定是
  VARIANT（0xc），取 `System.ItemUrl`（`ItemPathDisplay` 会本地化目录名），
  行句柄数组由提供方分配、`CoTaskMemFree` 归还。Linux 用 `tracker3`/`baloo`
  的 CLI。索引只贡献候选，`fs::search` 仍按名字、根与过滤器统一筛，并与遍历结果
  按折叠路径去重。详细取舍与实机调试结论见 `docs/decisions/search-index.md`。
- **R-5 主页部件**（`overview_model.rs`）：新增 `Tags`/`Drives` 两段，行构造抽成
  `tag_rows`/`drive_rows` 以便单测；设置页「主页小部件」多两个开关。标签行点击
  先回到文件夹再 `tag:` 搜索（`searchTag` 现在自己处理「不在文件夹」的情况），
  驱动器行跳到挂载点。
- **R-6 动作面**（`settings.rs`、`SettingsPanel.qml`、`ShellWindow.qml`）：可重绑定
  动作 35 → 48，新增复制路径/复制名称/打开方式/压缩 ZIP/解压到此处/计算大小/
  比较 SHA-256/创建副本/创建快捷方式/移动到/批量重命名/切换视图/切换大小单位。
  默认只绑定 `Mod+Shift+C`（复制路径），其余留空由用户在设置里分配；快捷键作用于
  **当前焦点窗格的选择**，不复用右键菜单的上下文捕获。
- **验证**：`cargo test -p dae-core --lib` 374 passed（新增索引解析/URL 换算用例）；
  `cargo test -p dae-qt --lib` 199 passed（新增标签/驱动器行映射用例）；
  `DAE_QML_LINT=1 DAE_QML_TYPEC=1 cargo build -p dae-app` 通过。
- **Windows 运行时冒烟**（隔离 `APPDATA`）：概览页显示「标签」（Work/Home）与
  「驱动器」（C:\ / Develop，含 NTFS/ReFS 与可用/总容量）；点击标签行进入
  `tag:Work` 的「所有卷」搜索；`Ctrl+Shift+C` 把当前目录写入剪贴板（读回一致，
  且界面弹出「已复制路径」）。
- **未验证**：Linux 运行时（tracker3/baloo 的实际输出）、MSIX 形态的索引可见性。

## 13. Office 预览、保存搜索与可访问性审计（2026-11-09）

### 13.1 Office 预览（扩展 §5.13 快速预览）

- **core**（`crates/dae-core/src/fs/office.rs`）：不引入 Office 引擎，也不起外部进程，
  直接从 OOXML 的 ZIP 容器里读文本。
  - `docx`：`word/document.xml` 的 `w:p` 与 `w:tbl`，表格行输出成 `单元格 | 单元格`。
  - `xlsx`：`xl/workbook.xml` 的 `sheet` 顺序配 `xl/_rels/workbook.xml.rels` 定位到各
    `worksheets/*.xml`；`t="s"` 的单元格走 `sharedStrings.xml` 索引；一行一条。
  - `pptx`：页序取自 `ppt/presentation.xml` 的 `sldIdLst`（不是文件名序），`r:id` 经
    rels 落到 `slides/*.xml`。`sldId` 上同时有 `id` 和 `r:id`，取属性时优先带前缀者。
  - **有界**：容器 48 MiB、单部件 8 MiB、220 行、表格 40 行 × 10 列、6 张表、12 页、
    单行 400 字符。全部用 `Read::take` 截断读，超限只置 `truncated`，不把整份解压进内存。
  - **错误码**：`fs.office_unsupported`、`fs.office_unreadable`、`fs.office_empty`、
    `fs.office_too_large`；`.doc/.xls/.ppt`（旧二进制）与 `odt/ods/odp/rtf/pages`
    （别的容器）各有一条独立文案——对用户来说这是三件不同的事。
- **bridge**：`crates/dae-qt/src/office_markup.rs` 把 `OfficePreview` 渲染成
  `QTextDocument` 能吃的 HTML 子集（标题 13px/600、正文 11px、表格行等宽并保留
  ` | ` 对齐），复用 `preview_markup` 的调色板与转义；`preview.rs` 缓存
  `source_office`，切主题时按纯函数重渲染——与 Markdown、代码预览同一条契约，不重读盘。
- **QML**：`PreviewPanel.qml` 的 `officeCard`，`officeTruncated` 时右下角提示
  「内容过长，仅显示开头部分」。
- **不做**：图表/SmartArt/图片、列宽与公式求值、`xlsx` 日期序列到日期的换算
  （原样显示序列号）、分页符与页眉页脚。

### 13.2 保存搜索（§5.5 阶段 1 的延伸）

- **偏好**（`crates/dae-qt/src/preferences.rs`）：`SavedSearch { id, name, query, mode,
  scope, typeId, kind, modified, size, regex, caseSensitive, filter }`，落在既有的
  `explorer-qt.ini` 单行 JSON 里（`savedSearches`），与 `searchHistory` 共用一套读写；
  上限 40 条。
- **API**：`saveSearch(name, spec)` 同名覆盖并保留 id（侧栏的行不跳位），否则插到队首；
  另有 `deleteSavedSearch(id)` 与 `renameSavedSearch(id, name)`。
- **QML**：搜索栏的「保存」chip 只在有查询词时出现；侧栏「保存的搜索」段最多显示 4 行，
  hover 出删除叉。恢复时一次还原 mode / scope / typeId / kind / modified / size / regex /
  caseSensitive / filter，再走与手打搜索相同的 `acceptSearchText`，所以「保存的」和
  「刚打的」是一条执行路径。
- **不做**：拖拽排序、导入导出、把保存的搜索固定成侧栏条目。

### 13.3 可访问性审计（`scripts/a11y-audit.mjs`）

- **为什么静态查**：`qmllint` 不检查这一项，截图也看不出来——这里的按钮是 `Rectangle`
  套 `MouseArea` 套 `Text`，Qt 不会从画出来的字形推断名字，屏幕阅读器只会读出一串「按钮」。
- **规则**：`MouseArea` / `TextInput` / `TextEdit` / `TapHandler` 必须自身或 5 层以内
  祖先声明 `Accessible.name`；`acceptedButtons: Qt.NoButton` 的滚轮拦截层跳过；
  `Accessible.ignored: true`（字面量 `true`）视为「有意移出无障碍树」；`// a11y:decorative`
  是同一件事的注释形式。条件式的 `Accessible.ignored`（如 `entry.separator`）不算，
  因为它在另一态仍然会被朗读。
- **用法**：`bun run a11y:audit`（等于 `--baseline`，只有新缺口才失败）；`--write-baseline`
  记录当前结果；`--json` 给外部工具；`--no-fail` 只报告。基线键是
  `${file}:${element}:${parent}#${序号}`，与行号无关，所以在上面加一行不会把老缺口
  变成新缺口。
- **本次清掉 87 条存量债务**：78 条补 `Accessible.role` + `Accessible.name`（多数是
  `Rectangle`+`MouseArea` 拼的按钮、列表行与输入框，行/页签另带
  `checkable`/`checked`/`selected`），9 条按 `Accessible.ignored: true` 归为**指针专属**
  （列宽拖拽、橡皮筋框选、分栏拖拽、窗口缩放手柄、对话框遮罩、标题栏双击最大化、
  只搬焦点的 `TapHandler`）。基线现在是空的，「只缩不涨」从第一天就是 0。
- **还没做**：新补的 `MouseArea` 按钮只声明了角色与名字，没有接
  `Accessible.onPressAction`（既有控件是接了的）——项目里的写法是把点击体复制一遍，
  机械铺到四十处只会和原体漂移，所以留给能跑起界面的那一轮；屏幕阅读器能读到名字，
  激活仍走指针。真机 NVDA/Orca 的朗读顺序也还没过。
- **验证**：`cargo test -p dae-core --lib` 通过（含 9 条 office 用例），
  `node scripts/a11y-audit.mjs` 零缺口。`cargo build -p dae-app` 与 `bun run lint:qml`
  在本轮机器上没跑（没有 Qt/qmake），这是接手后要补的第一件事。


## 14. 文本预览按内容判定（2026-11-13）

- **问题**：`fs::preview::is_text_preview_path()` 是一张扩展名白名单。名单之外的
  源码（`.zig`、`.sol`、`.kt`、没有后缀的 `Justfile`/`Caddyfile`）一律没有文本
  预览，而名单本身只增不减，得靠人记得去加。`docs/decisions/engine-qt-quick.md`
  写的是「普通文本是可选择的 `PlainText`」，QML 里实际是 `Text`（不可选）。
- **规则**：`may_have_text_preview()` 只做快速拒绝——图片、音视频、压缩包、可执行
  文件、Office、字体、数据库这些**已知容器**不读；其余一律按字节判定。
  `read_text_preview()` 返回 `Result<Option<TextPreview>>`，`None` 表示内容不是文本。
- **判定顺序**（`decode_text`，全部在 64 KiB 的有界读之上）：BOM 命名编码（UTF-8 /
  UTF-16LE / UTF-16BE）→ 无 BOM 的 UTF-16 按「一条 lane 几乎全是 0」识别（Windows
  工具常这么写）→ 有 NUL 或控制字节超过 5% 判为容器 → 合法 UTF-8 直接显示 →
  其余交给 `chardetng` 猜 GBK/Shift-JIS/CP1252，猜错也只是乱码，不会拒绝预览。
  被读取上限切断的多字节字符按前缀截断处理，不再变成替换字符。
- **代价**：未知后缀的文件会多读一次 64 KiB（本来选中项也要读）；已知容器仍然
  `stat` 之后直接跳过。
- **验证**：`cargo test -p dae-core --lib fs::preview` 6 passed（含无后缀文本、
  ELF 容器、UTF-16LE、GBK 与切断的多字节用例）；`cargo check -p dae-qt` 通过。
- **还没做**：纯文本卡仍是 `Text`，不可选中；代码卡的 64 KiB 截断提示只覆盖读取
  上限，不覆盖渲染量。见 §15。

## 15. 代码高亮：语法集与主题（2026-11-13）

- **问题**：`syntect` 的 `default-fancy` 只有 53 个 Sublime 旧语法，`language_token`
  映射的约 60 个 token 里有 18 个落到 `find_syntax_plain_text()`——`Cargo.toml`
  自己都没高亮。主题是 `base16-ocean.dark` / `InspiredGitHub`，注释对比度
  2.71:1 / 2.91:1（AA 正文 4.5:1），且代码块自带蓝灰底，与 instrument 调色板冲突。
- **语法集**：换 `two-face`（bat 0.26 的语法包，`extra_newlines()`，纯 Rust
  `regex-fancy`）。TS/TSX、Kotlin、Swift、Dart、TOML、INI、Dockerfile、Git
  ignore、GraphQL、Protocol Buffers、Nix 都有语法了；PowerShell 不在包里，
  `ps1` 仍是纯文本。回归测试 `every_mapped_language_resolves_to_a_grammar()`
  按 token 表断言，新增映射没有语法会直接失败。
- **主题**：`crates/dae-qt/assets/preview-{dark,light}.tmTheme`，按 `Theme.qml` 的
  纸张调色板写 scope（keyword 朱红、string 绿、number 赭、type 蓝、comment 灰斜），
  深色注释 4.89:1、浅色 4.94:1，其余 ≥5.2:1。主题文件用 `include_str!` 嵌入，
  `theme()` 由 `LazyLock` 各加载一次；`the_preview_themes_keep_text_readable()`
  逐条校验对比度，改色掉到 4.5 以下会失败。
- **排版**：`<pre>` 从 11px/150%/`pre-wrap` 改成 12px/`line-height:18px`/
  `white-space:pre`；`\t` 在 tokenize 前按 4 列展开（`QTextDocument` 的 tab stop
  是 80 device units，与等宽网格无关）。`padding:8px` 删掉——Qt 富文本只对表格
  单元格应用 padding，它从未生效。
- **横向滚动**：`PreviewPanel.qml` 的文本卡与代码卡都改成双向 Flickable，
  `contentWidth` 取内容宽度；纯文本行不再 `elide`，代码行不再折行。纯文本卡
  现在多显示一行提示「仅显示前 500 行」，此前 500 行截断是静默的。代码卡里的
  拖动被 `TextEdit` 拿去选中文字，所以鼠标靠 Shift+滚轮或触摸板横向滑动平移。
- **验证**：`cargo test -p dae-qt preview_markup` 13 passed；`qmllint
  qml/PreviewPanel.qml` 无新增警告；用 `qml.exe` 离屏渲染同一份 HTML（340×500
  双卡）确认颜色、斜体注释、tab 缩进与不折行，`contentWidth 421.875 >
  viewport 318` 时无绑定循环。临时 harness 与 dump 测试已删除。
- **还没做**：代码卡没有行号——行号需要与 `QTextDocument` 的行高逐行对齐，
  而 `TextEdit` 不暴露 `lineHeight`/`documentMargin`，硬拼会在字号或缩放变化时
  漂移；这一步留给「预览当阅读器」时的 `QSyntaxHighlighter` 方案。
