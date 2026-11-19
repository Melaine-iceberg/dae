# 压缩包当目录浏览：archive: 只读后端

> **状态：已实施（2026-11-19）。** 范围：`crates/dae-core/src/fs/url.rs`（`Scheme::Archive`、
> `Location::Archive`）、`crates/dae-core/src/fs/archive_view.rs`（新）、`vfs.rs`（resolve）、
> `qml/ShellWindow.qml`（进入/远端判定）。
> 相关：`docs/managers-parity.md` §4.1 的 G-4；借 Dolphin KIO 的「压缩包在 VFS 层就位」。

## 0. 结论

压缩包浏览不是解压的变体，是**后端的只读视图**：`FileSystemBackend` 已把
`read_dir/stat/open_read` 抽象好，archive 实现这三个，其余九个方法返回
`Unsupported`。拖出复制、Quick Look、面包屑、标签页全部走既有管线，
不加第二套状态。

四条边界，全部是能力事实而不是妥协：

1. **`archive:` 底层只接受本地文件。** `open_read` 给远程后端的原语是
   `Box<dyn Read + Send>`，没有 `Seek`，而 ZIP 的随机访问（central
   directory + `data_start`）必须有可寻址的文件；远程 zip 得整包下载，
   那是下载对话框的事，不是浏览。`archive:smb://...` 返回
   `fs.archive_local_only`。
2. **浏览 ZIP 与未压缩 TAR。** ZIP 有 central directory，`data_start()` 是
   公开 API，`Stored` 直接区间读、`Deflated` 套 `flate2::read::DeflateDecoder`
   （flate2 已是依赖）流式解压——两者都能返回无 Seek 的 `Read`。TAR 没有
   索引，**一次顺序扫描建内存索引**，按 `(路径, mtime, size)` 缓存命中，
   `open_read` 按索引 offset 直读。`tar.gz` 每次读都要整包解压，7z 是
   solid 块无随机访问，RAR/ISO 本来就走外部进程——这四类维持解压路径。
3. **加密 ZIP 不浏览，直接报错**（`fs.archive_encrypted`）。central
   directory 可读意味着名字会先泄漏、内容再要密码，这比「拒绝整个包」
   更糟；解压路径已有 `ArchivePasswordDialog` 与 `WrongPassword` 重试，不重复。
4. **只读。** `create_entry/mkdir/open_write/remove/rename_*` 一律
   `Unsupported`；KIO 也是「侧重浏览」，写回是另一件事。

## 1. 路径形态

`archive:<底层 EntryUrl 的 Display>!/<内部/相对/路径>`：

- `archive:/K:/code/notes.zip!/docs/2026/report.md`
- 内层为空 = 压缩包根：`archive:/K:/code/notes.zip!/`

`!` 而不是查询参数，因为内部路径自己可以携带 `?&`；也不学 KIO 的
`tar:/file.tar/dir` 把内层混进同一路径——包名与内层之间必须有不可冒充
的分隔符。`EntryUrl::parse` 必须在「没有 `://` 就当本地路径」回退**之前**
识别 `archive:`，且把 `!` 之后的部分按 `/` 分段。`Display` 输出规范形
（内层段过滤空段，结尾仅在非根时保留文件名）。

内层段做净化：`./` 折叠、绝对段丢弃、`..` 钳制在包根（`merge` 不拒绝，
浏览时能看到可疑名字本身就是信息；真正的防线是索引按净化后的段寻址，
`..` 永远解析不到包外）。

## 2. 索引与随机访问

**ZIP**：`read_dir`/`stat` 每次 `ZipArchive::new(File)`（central directory
毫秒级，不缓存句柄）；条目键 = 净化后的段路径。`open_read`：
`index_for_name` → `by_index` 拿 `data_start()`、`compressed_size()`、
`compression()` → 丢弃借用，另开 `File`，`seek(data_start)` +
`take(compressed)`；`Stored` 原样返回，`Deflated` 套
`flate2::read::DeflateDecoder`，其余方法 `fs.archive_method_unsupported`。
不构造自引用结构，不整条读进内存——大文件拖出不受限。

**TAR**：`read_dir` 扫一遍 `entries()` 建
`Vec<(段路径, offset, size, is_dir)>`，进程级 `Mutex<HashMap>` 缓存，键含
`(底层路径, mtime, size)`，超 32 项整表清空（防打一排大 tar 把内存吃满）。
`open_read` 按索引 `seek(offset + 0)`——数据区紧跟 512 字节 header，
`take(size)`；长名/扩展 header 的 offset 以 `header.offset()` 记数据起点
为准。

**stat**：索引行给出 `File`/`Directory`、字节数、修改时间（zip 处
`last_modified()`，tar 处 header mtime）；symlink 一律按 `File`（浏览
层不解析目标）。

## 3. 面包屑与门控

`read_dir` 的 `DirectoryView.breadcrumbs` = 底层本地面包屑 + 包名（路径为
`archive:…!`）+ 内层逐段（路径逐级 `archive:…!/a/b`）。点击任意一段都是
普通 `navigate(path)`。

`Scheme::Archive != Scheme::Local`，于是 Qt model 的 `remote` 自动为真：
终端、解压目标、本地搜索、缩略图、写操作全部按既有远程门控关闭，无需
新分支。QML 只改两处：

- `ShellWindow.isRemotePath` 正则补 `archive`（否则标签页级判断漏判）；
- 双击行为：`activatePath` 里「本地 + `isArchivePath` + 非目录」→
  `navigate(archive 形路径)`。Windows 11 双击 zip 也是进资源管理器，
  用别的程序打开走「打开方式」。

## 4. 验证

- `cargo test -p dae-core --lib`：
  - URL：`archive:` 本地/嵌套解析、往返、`join`/`pop`/`segments`、
    恶意内层段钳制；
  - 后端：临时目录构造真实 zip（`ZipWriter`）与 tar，走
    `vfs::resolve` → `read_dir`（含子目录树、面包屑）、`stat`、
    `open_read`（Deflate 与 Stored 各一条、内容逐字节比对）、
    写方法 `Unsupported`、加密 zip 拒绝、非 zip/tar 后缀拒绝；
  - 索引缓存：改 mtime 后 `read_dir` 重扫。
- Windows 冒烟：双击进入 zip → 面包屑 → 内层文本空格预览 → 拖出/复制
  到本地 → 字节一致；tar 同。

结果（2026-11-19，隔离 APPDATA，UIA 驱动）：

- `cargo test -p dae-core --lib` 504 passed / 10 ignored（新增 5 条：
  zip 树浏览与面包屑、Stored/Deflate 字节区间读取、tar 缓存与失效、
  守卫集、URL 解析与钳制）；`cargo test -p dae-qt --lib` 231 passed；
  `DAE_QML_LINT=1 cargo build -p dae-app` 通过。
- zip 根 {docs, top.txt}、内层 {nested, data.bin, readme.txt}、
  tar 根 {docs, plain.bin}（UIA 读行名）；面包屑渲染
  `K: > code > … > notes.zip > docs`。
- 复制 `docs/readme.txt` 出包：43 B 与 zip 内逐字节一致。
- Quick Look 预览 `docs/readme.txt` 显示正文 `The quick brown fox jumps
  over the lazy dog`，无错误。

## 5. 仍然存在的边界

- **远程底层、tar.gz、7z、rar、iso**：维持解压路径，错误文案点名格式。
- **加密 zip**：整个拒绝；后续若做「按条目输密码」须复用解压的
  `WrongPassword` 通道，不另起 UI。
- **写回**：`Unsupported`；改包内的重命名/删除是另一轮的事。
- **大 tar 首次进入**：建索引要顺序读全包，1 GiB 的 tar 在 NVMe 上秒级；
  流式列表（`stream_id`）仍不适用——archive 目录走非本地的一次性
  `read_dir` 分支，与远程目录同一体验。
- **zip 条目的非常规方法**（bzip2/zstd/lzma）：`zip` crate 未启用这些
  feature，读时报 `fs.archive_method_unsupported`，名字仍可见。
