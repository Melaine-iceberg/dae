# P1 集成项的取舍：系统工具、argv 与进程内记录

> 日期：2026-11-05 · 状态：已实施（P1）
> 相关：`docs/files-parity.md` §5.7–§5.18、§11、`crates/dae-core/src/fs/{drives,actions,archive}.rs`、
> `crates/dae-qt/src/{session,workspace,preferences}.rs`、`qml/{StatusCenter,QuickPreview}.qml`

P1 的十二项里，真正需要判断的不是界面，而是四件事：**破坏性操作交给谁、外部程序怎么启动、
历史记在哪里、网络后端缺什么能力时怎么做**。下面逐条记录结论与理由。

## 1. 驱动器：动作自己做，格式化交给系统

弹出、卸载、挂载是**可逆、只影响一个卷**的操作，dae 直接调用平台工具：

| 动作 | Windows | Linux | macOS |
| --- | --- | --- | --- |
| 弹出 | `DeviceIoControl(IOCTL_STORAGE_EJECT_MEDIA)`（先锁卷、dismount，失败不阻断） | `udisksctl unmount` 后 `eject`/`gio mount -e` | `diskutil eject` |
| 卸载 / 挂载 | 不做：Windows 自己管 | `udisksctl unmount/mount -b <dev>`，设备来自 `/proc/self/mounts` | `diskutil unmount/mount <dev>`，设备来自 `df -P` |
| 格式化 | `SHFormatDrive` 系统对话框 | 打开 `gnome-disks` | 打开「磁盘工具」 |
| 清理 | `ms-settings:storagesense` | 隐藏 | 隐藏 |

不内置格式化的理由：它需要提权、会销毁数据、每个文件系统的参数都不同，而三个平台都已经有
带二次确认的官方界面。dae 只负责把用户送到正确的入口——这比在各平台写一套 `mkfs` 包装
风险低得多，也不会留下「dae 自己把盘格了」的维护负担。

能力按平台在 `drives::capabilities()` 里声明，菜单据此隐藏而不显示灰项：
Windows 没有「卸载」这一用户动作，Linux/macOS 没有「清理」，就是隐藏的原因。

**未做**：Windows 的 `CM_Request_Device_Eject` 回退。卷句柄的 `IOCTL_STORAGE_EJECT_MEDIA`
在正常读卡器上成功；失败的场景（设备被独占）`CM_Request_Device_Eject` 也常被同一进程挡住，
留着它只会把错误信息换成另一条。真遇到再补。

## 2. 自定义动作：argv，不是命令行字符串

`[[actions]]` 的命令与参数**分别**交给 `Command::new(program).args([...])`：

- 占位符（`%f`/`%F`/`%d`/`%u`）按**参数**展开，`%F` 是唯一把一个参数变成多个的；
- 不拼 shell 字符串，因此没有引用、转义、注入的问题；一个叫 `a & rm -rf ~` 的文件名
  永远是一个文件名的参数；
- 参数数组存在 `settings.toml`，设置页每行一个参数——换行是唯一的分隔符，所以带空格的
  路径可以原样粘进去。

程序不在 `PATH` 上时**不建行**（`command_available`：绝对路径看存在性，裸名扫 `PATH`，
Windows 额外试 `exe/cmd/bat/com`）。内置的「在 IDE 中打开」也遵守这条：没有 `code` 的机器
不会看到一个点了就报错的菜单项。

快捷键从 22 个扩到 35 个，新增的都是此前写死在 QML 里的组合键（新建/关闭/重开标签页、
切换侧栏、刷新、属性、显示选项、过滤、分屏）。写死的组合键与 `settings.toml` 是两份真相，
用户改了其中一份就会被另一份覆盖；现在只有一份。

## 3. 状态中心：进程内环形缓冲，持久化 50 条

操作记录在 `Workspace` 里，`begin_operation` 记开始时间与标签，`finish_operation` 落一条
`{id, kind, label, started, finished, status, detail, retryable, retry}`：

- 内存最多 200 条，持久化最近 50 条到 `<config>/operations-qt.json`（原子写）。50 是因为
  重启后要回答的是「刚才那个失败是什么」，不是完整审计；
- 只把**复制/移动/解压**标为可重试，重试参数（源、目标、是否移动）随记录存 JSON；删除、
  覆盖不提供重试——文件已经没了，按钮只会制造第二次失败；
- `failureCount` 是未读失败数，打开面板清零，行本身保留到「清除记录」；
- 不接系统通知：三平台 API 差异大，v1 的可见性由标题栏按钮 + 徽章承担
  （`docs/files-parity.md` §10 已把系统通知列为非目标）。

## 4. 会话恢复：存路径与几何，不存选择

`<config>/session-qt.json` 每窗口一条记录：几何、`activeIndex`、分屏开关、每个标签的
`surface`/`path`/`spaceId`/`pinned` 以及前进后退的**路径**。

- 不存选区：历史条目里的路径可能已被改名或删除，恢复一个指向不存在行的选区比空选区更糟；
- `clean` 标记：启动时写 `false`，最后一个窗口正常关闭写 `true`；下次启动看到 `false`
  就提示「上次未正常退出，已恢复」，而不是假装什么都没发生；
- 恢复时把旧 token 的记录删掉、用新 token 保存，否则同一会话会在下一次启动里变成两条；
- Wayland 可能忽略窗口位置：恢复失败不报错，尺寸仍然生效。

## 5. 压缩：RAR/ISO 只走外部工具

- 内置保持 zip/tar/tar.gz/7z；RAR 解码库的许可与本项目不兼容，不链接；
- `rar`/`iso` 依赖 `7z`/`7zz`/`7za`→`unrar`→`bsdtar` 任一个，缺失时解压入口不显示，
  真的调用到就返回带格式名的「缺少工具」错误；
- 智能解压：条目全部位于同一个顶层目录时解到压缩包所在目录（得到 `foo/` 而不是
  `foo/foo`），否则按旧规则新建以包名命名的目录；
- 「解压到新文件夹…」复用「移动到」的目录选择器（`extractSource` 模式），不再新写一个选择器。

## 6. FTP/FTPS：连接池而不是一条连接

`FileSystemBackend` 是 `Send + Sync`，传输引擎用 `par_iter` 并发复制同目录的兄弟项，
并把 `Box<dyn Write>` 移到各自的线程。FTP 的一条控制连接同一时刻只能有一个数据连接，
所以「一个 `Mutex<FtpStream>`」会在一台服务器内部的复制里自锁。后端改为**每服务器最多
8 条控制连接**的池（一次复制用 4 读 + 4 写），等待上限 60 秒；连接错误时丢弃重开，
只重放幂等的 `CWD`，不重放会改状态的命令。

上传与下载都走流式（`retr_as_stream` / `put_with_stream`），不把文件读进内存。代价是
数据连接最后的 `226` 只能在 `Drop` 里读走，写失败的细节只能进日志——传输引擎已经
flush 过，`Write::flush` 没有第二次机会。

TLS：21 明文、990 隐式、`ftps://` 显式；证书只按系统信任库验证，不做按主机 TOFU
（SFTP 的 `known_hosts` 是先例，但 FTP 的协议层没有等价物）。`DAE_TLS_CA_FILE`
可以额外加载一个 PEM 作为补充信任锚（私有 CA / 测试服务器），它只增加信任，不关闭
校验，也不替换系统根证书。

## 7. WebDAV：流式 PUT，而不是整文件缓冲

WebDAV 后端用 `reqwest` 实现 PROPFIND/MKCOL/DELETE/MOVE/GET/PUT。上传的第一版
`open_write` 返回 `Unsupported`，原因是写接口只有 `Write`、没有 `finish()`，而流式
PUT 需要一个「读到多少发多少」的请求体。后来按当时记下的补法实现：

- 给 `reqwest` 开 `blocking`，用 `Body::new(reader)` 接收一个 `Read`；
- `open_write` 返回写适配器，把字节按 64 KiB 分块推进深度 4 的有界通道；队列满时
  `write` 阻塞，这是传输引擎本来就依赖的背压，不会把大文件读进内存；
- 专用线程把通道包装成 `ChannelReader` 发 PUT；`flush()` 关闭通道并等待最终响应，
  所以「flush 成功」等价于「服务器已经收下文件」，而不是「字节已交给 socket」；
- 非 2xx 保留服务器正文并映射为既有错误类型；`Content-Length` 未知时用分块传输，
  服务器不支持分块 PUT 时会在状态里说明。

单测用进程内 HTTP/1.1 服务器验证分块 PUT 的字节与 403 的错误传递；真实 WsgiDAV
（匿名与 Basic）跑通了浏览、上传、同后端复制、改名、删除。

## 8. 平台安全：只读，且失败要说话

签名与 ACL 都只读。签名用 `WinVerifyTrust`；内嵌签名缺失时再按文件哈希查系统
catalog，因为 System32 的多数二进制是 catalog 签名，只查内嵌会把 `cmd.exe` 误报成
未签名。ACL 用 `GetNamedSecurityInfoW` 读 owner/group/DACL，ACE 渲染成只读文本；
写权限仍只有 POSIX/DOS 属性。两者都在 `spawn_blocking` 里与元数据并行，慢的证书链
不会拖住对话框。macOS 的 `SecStaticCodeCheckValidity` 与 NFSv4 ACL 没有实现，入口
按平台隐藏；Linux 没有文件签名概念，同样隐藏。

## 9. 系统共享：WinRT 的 `ForWindow` 路径

桌面进程没有 `CoreWindow`，所以 `DataTransferManager` 的 `ShowShareUI` 不可用，
必须走 `IDataTransferManagerInterop::ShowShareUIForWindow`。文件先在 MTA 线程解析成
`StorageFile`（Win32 路径；列表里的正斜杠先换成反斜杠，否则
`GetFileFromPathAsync` 返回 `ERROR_BAD_PATHNAME`），用 `AgileReference` 交回 UI
线程，再由自定义 `IIterable<IStorageItem>` 交给 `DataPackage`。事件处理器注册在
窗口的 `DataTransferManager` 上，最后一次注册保存在线程局部变量里，下一次共享替换。
macOS 的 `NSSharingServicePicker` 与 Linux 的统一 portal 没有实现，入口隐藏。

## 何时重新考虑

- Windows 弹出在真实读卡器上失败且 `CM_Request_Device_Eject` 能成功；
- 状态中心需要跨进程/跨用户审计，或要接系统通知；
- WebDAV 服务器拒绝分块 PUT，需要改成已知长度的 `Content-Length` 请求体；
- 用户自定义动作需要环境变量、管道或 shell 语义（那会推翻「argv 而非字符串」这一条，
  必须在设置里明确标注）。
