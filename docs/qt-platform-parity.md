# Qt 壳的三平台对齐清单

> 日期：2026-10-30 · 分支：`refactor/qt-quick`
> 对象：`apps/dae` + `crates/dae-qt` + `qml/` 的 Qt Quick 壳。
> 目标不是把 Windows 的像素搬到 Linux/macOS，而是把「Windows 上已经量过的原生行为」
> 在另外两个平台补到同等可用，并把只在 Windows 生效的监听器补成三平台。

## 0. 结论

主流程已经跨平台：文件操作、回收站、图标、终端、搜索、预览、深链解析、单实例、
更新包和三种 zip 都已落地；`qt-check` 也证明 Linux/macOS 能编译。

但 Windows 之外目前仍是「能启动、主流程能跑」的状态，缺的是**应用级集成**和
**平台监听器**：

1. macOS 的 Finder / 默认文件管理器 / `dae://` 没有事件入口；
2. macOS 没有原生菜单栏，窗口边缘缩放没有可用的 Qt 路径；
3. 系统明暗、强调色、卷热插拔的 watcher 只在 Windows 实现；
4. macOS 没有桌面缩略图产出；
5. Linux 的 scheme 注册、桌面入口和发布包校验还不完整；
6. ~~Linux/macOS 只有编译验证，没有运行时回归~~ → Linux 已按 §4 跑完全部可执行项，
   并在 2026-11-08 按全表真机复验通过（§4、§9.14）；macOS 的运行时回归不再需要，
   因为它不发布（§9.12.4）。

以下 P0/P1 缺口已按 §3 分阶段落地，逐项状态见 §6。标「不发布」的 macOS 条目
仍保留代码、仍由 `qt-check` 编译，但不再有「待手测」这种中间状态 —— 没有设备
就没有能验收它的人。

## 1. 已经对齐，不再列为待办

| 能力 | 状态 |
| --- | --- |
| 三平台编译 | `qt-check` 在 ubuntu-26.04 + macos-latest 上 `cargo build -p dae-app` |
| 窗口移动 | Windows 原生 hit test；Linux/macOS `startSystemMove` |
| 背景材质 | Windows Mica/Acrylic；Wayland `ext-background-effect-v1`；macOS `NSVisualEffectView` |
| 文件操作 / 回收站 | `dae-core` 三平台后端；macOS `~/.Trash` + put-back 记录 |
| 图标 | Windows Shell、Linux xdg、macOS NSWorkspace |
| 缩略图 | Windows Shell handler、Linux `.thumbnailer`、macOS QuickLook（§2.7） |
| 终端 | `portable-pty` + libghostty-vt（Windows 官方 DLL，Unix vendored 静态库）；Unix 读 `$SHELL` |
| 深链解析 / 单实例 | `dae-core` 路径解析 + QLocalServer/QLockFile |
| 打开方式 / 默认文件管理器 | Windows 原生；macOS LaunchServices；Linux `xdg-mime` |
| 更新 | 三平台安装脚本 + zip + minisign 校验 |
| 打包 | Windows zip、Linux portable zip、macOS `.app` zip |

## 2. 缺口清单

### P0：目前会坏或缺入口

#### 2.1 macOS 收不到 Finder / 默认文件管理器 / `dae://` 的打开事件（已补）

- 现状：`packaging/qt/Info.plist` 已声明 `CFBundleURLTypes` 与 `public.folder`，
  但应用只处理 argv 和 QLocalSocket，没有处理 macOS 的 `QFileOpenEvent`。
- 后果：Finder「打开方式」、把 dae 设为默认文件管理器、浏览器 `dae://` 都不会导航。
- 落点：`crates/dae-qt/cpp/activation.cpp`、`crates/dae-qt/cpp/activation.h`。
- 验收：运行中从 Finder 打开文件夹出现新标签；未运行 `open -a dae /tmp` 冷启动到该目录；
  运行中点击 `dae://open?path=…` 复用或新建标签。

#### 2.2 macOS 没有原生菜单栏（已补）

- 现状：`apps/dae/src/main.rs` 只创建 `QGuiApplication` + QML 引擎，没有 `QMenuBar`。
- 后果：Cmd+Q、Cmd+H、Cmd+M、About、Services、Window 菜单缺失，
  应用没有和 macOS 的系统菜单栏集成。
- 落点：新增 `crates/dae-qt/cpp/macos_menu.mm` + `.h`，在 claim 后安装。
- 验收：菜单栏出现 dae 菜单；Cmd+Q 退出、Cmd+H 隐藏、窗口菜单最小化/缩放可用。

#### 2.3 macOS 无边框窗口的边缘缩放没有 Qt 实现路径（已补）

- 现状：`qml/ResizeHandles.qml` 只调用 `startSystemResize()`；Qt 6.12 的 Cocoa
  后端只实现 `startSystemMove`，`QPlatformWindow::startSystemResize` 默认返回 `false`，
  且 QML 没有检查返回值或手工回退。
- 落点：`qml/ResizeHandles.qml` 增加 manual resize fallback（仅在
  `startSystemResize` 返回 false 时启用）。
- 验收：macOS 上八条边/四角都能拖拽改变窗口大小，最小尺寸不越界，内容不抖动。

#### 2.4 Linux 的 `dae://` 没有运行时注册（已补）

- 现状：`packaging/qt/dae.desktop` 声明了 `x-scheme-handler/dae`，但 portable zip
  不会安装它；`default_manager` 只注册 `inode/directory`。
- 落点：启动时确保 `~/.local/share/applications/dae.desktop` 存在并
  `xdg-mime default dae.desktop x-scheme-handler/dae`；或提供安装脚本。
- 验收：解压运行 dae 后，浏览器点击 `dae://` 能唤起已运行实例。

### P1：Windows 生效，另外两个平台不生效

#### 2.5 系统明暗 / 强调色 watcher 只在 Windows（已补）

- 现状：`crates/dae-qt/src/system_watch.rs` 在非 Windows 是空实现；
  `crates/dae-qt/src/platform.rs` 的非 Windows `platform_accent()` 返回 `None`，
  最终固定为 `#2f6fdf`；`Workspace.systemDark` 只在启动时读一次。
- 旧壳已有实现：`src-tauri/src/system_accent/macos.rs`（`NSColor.controlAccentColor`）、
  `src-tauri/src/system_accent/linux.rs`（freedesktop portal）、
  `src-tauri/src/system_theme/linux.rs`（portal）。
- 落点：Qt 侧接 `QStyleHints::colorSchemeChanged`；macOS 读 `NSColor`，
  Linux 读 portal；结果仍通过 `Workspace.applySystemAccent/applySystemTheme` 入队。
- 验收：运行中切系统深色/浅色立即生效；选择「系统强调色」后颜色与系统一致；
  Linux portal 与 macOS NSColor 都刷新。

#### 2.6 卷热插拔不刷新侧栏（已补，轮询）

- 现状：`volumesChanged` 只由 Windows `WM_DEVICECHANGE` 触发；
  旧壳的 Linux GIO / macOS NSWorkspace 监听未迁移。
- 落点：非 Windows 以 2 s 间隔比较 `dae_core::fs::sidebar::disk_signature()`
  （挂载点、显示名、文件系统、总容量），变化才调用
  `Workspace.notify_volumes_changed`；Windows 保留事件通道。签名刻意忽略
  剩余空间，普通写入不会触发刷新。GIO/NSWorkspace 事件通道作为后续优化。
- 验收：插入/移除 U 盘、挂载/卸载网络卷后侧栏在两个轮询周期内刷新。

#### 2.7 macOS 没有桌面缩略图（已补）

- 现状：`crates/dae-core/src/fs/thumbnails.rs` 的 macOS 后端返回 `None`；
  PDF 首屏、视频封面、HEIC/AVIF 在 Windows/Linux 有系统产出，macOS 没有。
- 落点：新增 macOS 后端（QuickLook / `qlmanage -t` 或 AppKit thumbnail generator），
  仍走 `dae_core::fs::thumbnails::extract`。
- 验收：macOS 预览面板对 PDF、mp4、HEIC 显示缩略图；无产出时仍回落到字形。

#### 2.8 macOS 窗口 chrome 是自绘的（已定案；macOS 不发布，只保编译）

- 现状：红绿灯由 QML 画（`qml/TitleBar.qml`），绿色按钮走 `showMaximized`
  而不是全屏；没有悬停字形；双击标题栏由 QML 自己实现，不遵循系统偏好。
- 三条决定（2026-11-01，按 macOS 原生行为定，不按口味定）：
  1. **绿色按钮 = 原生全屏切换**。自 10.10 起这就是该按钮的含义；
     「铺满可视区域」由双击标题栏提供（见第 3 条）。实现上走
     `window.showFullScreen()` / `showNormal()`，Qt 的 Cocoa 后端对应
     `NSWindow toggleFullScreen:`，动画与菜单栏/Dock 的隐藏交给系统。
  2. **补 hover 字形**：指针进入三颗灯中的任意一颗，三颗同时显示字形
     （`×`、`−`、全屏进出箭头）。字形是 `qml/Glyph.qml` 里的绘制路径
     （`windowClose` / `windowMin` / `windowFullScreen` /
     `windowExitFullScreen`），不是字符——macOS 系统字体没有 Windows 那组
     码位，早先版本正是因此把它们渲染成空框。绿色那颗随全屏状态在
     「进入 / 退出」两个形状间切换。
  3. **双击标题栏读系统偏好**：`AppleActionOnDoubleClick`（系统设置 →
     桌面与程序坞 → 「双击窗口标题栏以」），取值 `Maximize` / `Minimize` /
     `None`，分别对应 zoom / 最小化 / 不动作；读不到或不认识时按 zoom
     处理（macOS 对鼠标的默认）。读取在 `cpp/macos_platform.mm`
     （`dae_qt_macos_titlebar_double_click`），经
     `platform.rs::titlebar_double_click` 暴露为
     `WindowController.titlebarDoubleAction()`；QML 在**每次双击时**调用，
     所以改完偏好不需要重启。
  - 未做：Option+双击强制 zoom。`TapHandler` 拿不到修饰键，要拿就得换成
    `MouseArea`，而它会和拖动用的 `DragHandler` 抢点击；收益不足以承担这个
    风险，写在限制里。
- 验收：与 macOS 原生文件管理器的窗口操作习惯不冲突；状态栏/标签不被遮挡；
  绿色按钮进/出全屏、hover 出字形、双击按偏好执行。
- **状态：代码完成，不能验收。** 没有 Mac 设备，产物已随 §9.12.4 撤项；这四条
  结论里只有「编译并链接」由 `qt-check` 的 `macos-latest` 腿保证，其余三条写在
  这里是为了恢复 macOS 发布时有人照它验收，不是「已经验过」。

#### 2.9 macOS 单实例激活不保证到前台（已补）

- 现状：`dae_qt_focus_window` 只做 `raise()` / `requestActivate()`；
  代码注释承认后台应用还需要 dock/menu activation，未实测。
- 落点：macOS 上补 `[NSApp activateIgnoringOtherApps:YES]`（或等价 API），
  并在无窗口时处理 reopen。
- 验收：另一个应用在前台时，通过 `open`/深链唤起 dae，窗口到前台。

### P2：发布与验证

#### 2.10 Linux/macOS 只有编译验证

- `qt-check` 只执行 `cargo build`；没有运行时、IME、窗口拖动/缩放、可访问性回归。
- 现状下 `.github/workflows/linux-check.yml`、`macos-check.yml` 针对旧 Tauri 壳。
- 落点：至少给 Qt 壳补一个无显示器的 QML 加载冒烟；macOS/Linux 手测清单见 §4。

#### 2.11 macOS 发布签名

- 现状：release 只做 ad-hoc 签名，没有 Developer ID 与公证；仅 Apple Silicon。
- 后果：从网络下载的 zip 会被 Gatekeeper 拦截；Intel Mac 不能运行。
- 落点：产品决定是否购买证书/公证；短期在 README 与发布说明注明右键打开步骤。

#### 2.12 Linux 发布包与运行依赖

- 现状：只有 portable zip；没有 deb/rpm/Flatpak/AppImage；桌面入口和 scheme 不安装；
  发布校验只断言 `libqxcb.so`，Wayland 平台插件是否打包未断言；图标只有 128px。
- QtMultimedia 在 Linux 依赖 GStreamer 插件，目标机缺插件时视频/音频失败。
- 落点：安装脚本或安装包；发布校验补 Wayland 插件；明确 GStreamer 运行依赖。

#### 2.13 X11 背景模糊

- 现状：仅 Wayland `ext-background-effect-v1`；X11 保持实色。
- 这是设计边界，不在本次修复范围；设置面板已有说明。

## 3. 执行顺序

每个阶段独立可构建、可提交；Windows 本机验证 `cargo build -p dae-app` 与
`cargo test -p dae-qt`，Linux/macOS 由 `qt-check` 编译，运行时按 §4 手测。

### 阶段 1：macOS 应用级入口（P0）

1. `QFileOpenEvent` → `Activation.forwardExternal()`；
2. 原生菜单栏（About/Services/Hide/Quit、Edit、Window、Close）；
3. `ResizeHandles` manual fallback。

验收：Finder 打开、`dae://`、默认文件管理器、菜单快捷键、八向缩放。

### 阶段 2：系统外观监听（P1）

1. `QStyleHints::colorSchemeChanged` → `applySystemTheme`；
2. macOS `NSColor.controlAccentColor`；
3. Linux freedesktop portal accent；
4. 保留 Windows 现有 watcher，避免重复。

验收：运行中切换系统主题/强调色立即生效。

### 阶段 3：卷监听（P1）

非 Windows 用挂载签名轮询（2 s）触发 `notify_volumes_changed`；Windows 继续用
`WM_DEVICECHANGE`。验收：热插拔/挂载卸载在两个轮询周期内刷新侧栏。

### 阶段 4：macOS 缩略图（P1）

新增 macOS thumbnail 后端；优先 QuickLook，失败回落 `None`。

验收：PDF/video/HEIC 预览。

### 阶段 5：Linux 桌面与发布（P0/P2）

1. 启动时安装并注册 `dae.desktop`（scheme + 图标）；
2. 发布校验补 Wayland 插件与运行依赖；
3. 文档写明 GStreamer 依赖与 portable 安装方式。

### 阶段 6：验证与收尾

1. Windows：`cargo build -p dae-app`、`cargo test --workspace`；
2. 更新 `docs/decisions/engine-qt-quick.md` §4.2 的平台边界表；
3. 记录 Linux/macOS 手测结果（Finder/深链/菜单/缩放/主题/卷/缩略图）——
   **已完成（2026-11-01）**：Linux 11 项全通过，macOS 两项不适用，见 §4 与
   决策文档 §9.14。

## 4. Linux / macOS 手测清单

Linux 已在真机上按本表跑完，并在 2026-11-08 按全表复验（记录见本节末）；macOS 没有可用设备，
产物也随决策文档 §9.12.4 撤项，所以 macOS 列的 `—` 是**不适用**，不是「待测」。
两项 macOS 专属项（原生菜单、红绿灯 chrome）在 Linux 上同样不适用。

| 项目 | 操作 | 期望 | Linux | macOS |
| --- | --- | --- | --- | --- |
| 文件管理器打开 | 运行中，文件管理器里右键文件夹 → 用 dae 打开（macOS：Finder → 打开方式） | 新标签到该目录 | ✅ | — |
| 冷启动文件 | 退出 dae，`dae /tmp`（macOS：`open -a dae /tmp`） | 启动并到 `/tmp` | ✅ | — |
| 深链 | `xdg-open 'dae://open?path=/tmp'`（macOS：浏览器打开同一 URL） | 复用/新建标签，窗口到前台 | ✅ | — |
| 默认管理器 | 设置 → 默认文件管理器 → 设为默认，之后双击文件夹 | dae 打开并定位 | ✅ | — |
| 菜单 | macOS：Cmd+Q / Cmd+H / Cmd+M 与窗口菜单 | 行为与原生一致 | n/a | — |
| 缩放 | 拖动八条边/四角 | 窗口连续缩放，不闪 | ✅ | — |
| 主题 | 系统切深色/浅色 | 立即切换 | ✅ | — |
| 强调色 | 设置选「系统强调色」，改系统强调色 | 立即跟随 | ✅ | — |
| 卷 | 插拔 U 盘 | 侧栏出现/消失 | ✅ | — |
| 缩略图 | 选中 PDF / mp4 / HEIC | 显示缩略图 | ✅ | — |
| 输入法 | 拼音输入中点击别处 / Tab | 组合取消，不提交原始拼音（§2.8 上方的守卫说明、`cpp/ime.h`） | ✅ | — |
| 窗口 chrome（macOS） | 悬停红绿灯 / 点绿灯 / 双击标题栏 | 三颗灯出字形；绿灯进全屏、再点退出；双击按系统偏好执行（zoom / 最小化 / 不动作） | n/a | — |
| 拖放 | 从文件管理器拖文件进列表；从列表拖到文件管理器 | 复制/移动语义正确 | ✅ | — |
| 标签写入 | 设置 → 标签新建；右键 → 标签；侧栏计数 | NTFS/ext4/APFS 写进文件；远程/压缩包回退索引并提示 | Windows ✅ / Linux ✅ | — |
| 标签搜索 | 点侧栏标签，或在搜索框输入 `tag:名称` | 跨本地卷列出带该标签的项 | Windows ✅ / Linux ✅ | — |
| 排序与分组 | 视图菜单改排序键、文件夹顺序、分组与日期粒度 | 列表分组头正确；框选/键盘游标不错行；网格只重排不画分组头 | Windows ✅ / Linux ✅ | — |
| 每目录布局 | 关闭「所有目录使用同一布局」，在两个目录设不同视图 | 切回目录恢复视图/排序/列宽/图标大小 | Windows ✅ / Linux ✅ | — |
| 详情列 | 拖表头分隔线、双击自适应、视图菜单勾选列 | 列宽持久化；窄窗按序隐藏；扩展名开关只影响显示 | Windows ✅ / Linux ✅ | — |
| 搜索范围 | 搜索栏切换当前/收藏/最近/所有卷；附加日期/大小/类型过滤 | 结果合并去重；非本地根被拒绝；历史可回填 | Windows ✅ / Linux ✅ | — |
| 信息面板 | 选中单个或多个文件 | 单个：时间/权限/哈希/标签编辑；多个：数量、合计大小、共同标签 | Windows ✅ / Linux ✅ | — |
| 标签导入/导出 | 设置 → 标签 → 导出…/导入… | 导出即 `tags.toml`；导入按 id 覆盖、未知 id 追加、重名被跳过并说明 | Windows ✅ / Linux ✅ | — |
| 标签索引重建 | 删除若干已打标文件（或改名）后 → 设置 → 标签 → 重建索引 | 失效条目被移除、分片清空即删除、损坏分片隔离为 `*.corrupt-*`；状态行给出四项计数 | Windows ✅ / Linux ✅ | — |
| 网络发现 | 局域网内有一台广播 SMB/SFTP/WebDAV 的机器（NAS 或手机的共享） | 侧栏「网络」段出现该实例；点击后连接对话框预填协议/主机/端口，账号密码为空 | Windows ✅ / Linux ✅ | — |
| 主页部件 | 概览页滚到底 | 「标签」列出定义与计数，点击进入 `tag:` 搜索；「驱动器」列出卷与可用/总容量，点击进入该卷 | Windows ✅ / Linux ✅ | — |
| 系统索引搜索 | 在「所有卷」范围搜索一个已知文件名 | 结果包含该文件；索引不可用时结果仍来自遍历，不报错 | Windows ✅（未打包）/ Linux ✅（遍历回退） | — |
| 新增动作快捷键 | 选中一项后按 `Ctrl+Shift+C`；设置 → 快捷键查看新动作 | 路径写入剪贴板；新动作（打开方式、压缩、计算大小等）可绑定且立即生效 | Windows ✅ / Linux ✅ | — |

**2026-11-01 Linux 手测。** 环境：用户自有机器，已装 `.rpm`（与 §9.12.3 同一个
包），**Wayland 与 X11 两种会话都跑过**，输入法 **fcitx5**。上表 Linux 可执行的
11 项全部符合期望，没有一项需要改代码；两项 macOS 专属项按不适用记录。结论、
边界（缩略图取决于桌面的 freedesktop thumbnailer）与剩余项见决策文档 §9.14。
macOS 列保持 `—`：无设备，且产物已撤（§9.12.4）。

**2026-11-07 Linux 预验（WSL2）。** 环境：WSL2 的 CachyOS，Qt 6.12.0，WSLg
（Wayland 与 XWayland 两种 QPA 都跑过），软件渲染
（`QT_QUICK_BACKEND=software`，WSLg 没有 DRM），未打包的 release 构建，隔离的
`XDG_*` 目录。11-01 之后落地的功能行按本表跑：标签写入（`user.dae.tags`）与
`tag:` 搜索、排序/分组、每目录布局、详情列显示与勾选、搜索范围、信息面板、
索引重建、网络发现（python-zeroconf 广告一个 SFTP 实例）、主页部件、系统索引
回退（Linux 无索引时走遍历，日志无错误）、动作快捷键都符合期望。本轮修掉两个
跨平台缺陷：

1. **详情列全开挤掉 Name**：列轨道封在 704 px，五列全开时 Name 宽为 0、表头
   互相覆盖。`qml/FileList.qml` 给 Name 留 100 px，装不下时从尾部按序丢列，
   和窄窗规则一致。
2. **启动路径不恢复每目录布局**：命令行/深链直接打开目录时，标签页带着路径
   创建，`onPathChanged` 不触发，`currentPath` 从未写入，启动后用的是全局/
   默认布局。`qml/ShellWindow.qml` 在启动标签页分配完后同步一次 `currentPath`。

当时未覆盖的拖放、列宽拖拽/双击自适应（合成输入在 WSL 上定位不到表头分隔线）、
标签导出/导入的文件对话框（WSLg 下不可驱动；桥接层有单测），已于 2026-11-08
真机补测通过。另记的两个小问题已于 2026-11-08 修复并在真机复验中确认：①标签
维护按钮与导出对话框文案缺英文翻译——三个 `.ts` 各补齐 42 条缺失 message
（1179 → 1221，含标签按钮、导入/导出对话框、状态行、新增动作标签与概览/侧栏
词条）；②单行目录切换分组后分组头延迟刷新——`apply_order` 在单行或行序不变的
早退路径上只重写 `group_label` 而不发信号，分组头要等标签加载完成的
`emit_tag_roles` 才刷新，现早退路径补 `dataChanged(ROLE_GROUP)`。另核对
Shift+方向键只移动游标、不扩选区：与 `listing-shell-costs.md` §10.6 记录的
网格行为一致，属既定行为。
macOS 列保持 `—`：无设备，且产物已撤（§9.12.4）。

**2026-11-08 Linux 真机复验。** 环境：用户的物理机，CachyOS（Arch 系）。
按本表全表重跑，全部符合期望，没有需要改代码的地方；11-07 WSL2 预验留下的
拖放、列宽拖拽/双击自适应、标签导入/导出文件对话框三项这次都覆盖到了。
11-07 修掉的两个跨平台缺陷（详情列全开挤掉 Name、启动路径不恢复每目录布局）
与 11-08 修掉的两处小问题（英文文案、单行目录分组头刷新）也一并确认。至此
§4 的 Linux 列没有开口项；macOS 列仍按不适用记录。

## 5. 非目标

- 不把 Windows 的 Mica 像素复制到 Linux/macOS；
- 不引入 Qt Quick Controls 重写现有组件；
- 不为 X11 造背景模糊；
- 不在本阶段解决 macOS 公证与 Linux 包管理器分发（需要证书/打包决策）。

## 6. 进度记录

| 阶段 | 状态 | 证据 |
| --- | --- | --- |
| 文档 | 已完成 | 本文件 |
| 阶段 1 · QFileOpenEvent | 代码完成，不发布 | `cpp/activation.{h,cpp}` 安装 `QFileOpenEvent` 过滤器，统一走 `Activation::forwardExternal`；macOS 专属，随 §9.12.4 撤项 |
| 阶段 1 · 原生菜单 | 代码完成，不发布 | 新增 `cpp/macos_menu.{h,mm}`；claim 后安装 App/File/Edit/Window 菜单。macOS 专属，同上 |
| 阶段 1 · 缩放 fallback | **Linux 已测通过**（2026-11-01） | `qml/ResizeHandles.qml` 在 `startSystemResize` 返回 false 时手工改窗口几何；Wayland 与 X11 两种会话下拖边都连续（§9.14） |
| 阶段 1 · 激活到前台 | 代码完成，不发布 | `cpp/macos_backdrop.mm` 新增 `dae_qt_macos_activate_app`；`dae_qt_focus_window` 调用。macOS 专属，同上 |
| 阶段 2 · 主题/强调色 | **Linux 已测通过**（2026-11-01） | `cpp/platform_watch.cpp` 接 `colorSchemeChanged`；`platform.rs` 增加 macOS `NSColor` 与 Linux portal 读取；非 Windows 每 2 s 轮询 accent |
| 阶段 3 · 卷监听 | **Linux 已测通过**（2026-11-01） | 非 Windows 每 2 s 比较 `dae_core::fs::sidebar::disk_signature()`；Windows 仍走 `WM_DEVICECHANGE` |
| 阶段 4 · macOS 缩略图 | 代码完成，不发布 | 新增 `crates/dae-core/src/fs/thumbnails/macos.rs`：`qlmanage -t` 私有目录 + 5 s 超时 + PNG 校验。macOS 专属，同上 |
| 阶段 5 · Linux 桌面/发布 | **Linux 已测通过**（2026-11-01） | `linux_desktop::ensure_registered` 写 `dae.desktop` 并注册 scheme；菜单启动 / 深链 / 默认管理器三处都到同一进程（§9.14） |
| 阶段 6 · 验证 | 完成（2026-11-01；2026-11-08 全表复验） | Windows 构建/测试通过；Linux 真机按 §4 跑完 11 项全通过，macOS 两项不适用；Linux 后续功能行由 WSL2 预验覆盖，2026-11-08 在 CachyOS 真机按全表复验全部通过；`qt-check` 含 `dae-qt` 单测与 `macos-latest` 编译腿 |

2026-10-30 阶段 1 本机验证：`cargo build -p dae-app` 通过（44 s）；
`cargo test -p dae-qt --lib` 133 passed；`qmllint qml/ResizeHandles.qml` 无告警。
macOS 专有的 Objective-C++ 与 `QFileOpenEvent` 路径不在 Windows 上编译，
其编译结论以 `qt-check` 为准，运行时结论以 §4 手测为准。（后记：这一条永远
不会有运行时结论了 —— macOS 产物已撤，§9.12.4。）

2026-10-30 阶段 2 本机验证：`cargo build -p dae-app` 通过；`cargo test -p dae-qt --lib`
135 passed（新增 portal palette 与 RGB 解码用例）。`cpp/platform_watch.cpp` 在
Windows 上参与编译和链接，确认 cxx 桥的命名空间与符号正确；macOS `NSColor` 与
Linux portal 的实际读取仍待目标平台手测。（后记：Linux 已于 2026-11-01 在真机上
验过，Wayland 与 X11 两种会话、fcitx5，见 §4 与决策文档 §9.14；macOS 的 `NSColor`
读取不再有不发布的产物可验。）

2026-10-30 阶段 3–5 本机验证：`cargo build -p dae-app` 通过；
`cargo test -p dae-qt --lib` 138 passed（新增 Linux desktop entry 用例），
`cargo test -p dae-core --lib` 301 passed / 10 ignored。macOS QuickLook 与
Linux portal / xdg-mime 路径不在 Windows 上执行，仍需按 §4 手测。（后记：Linux 的
`xdg-mime` 与桌面入口已在 2026-11-01 真机验过，§9.14；QuickLook 随 macOS 撤项。）

阶段 6 同时把 `cargo test --package dae-qt --lib` 加进 `qt-check`：Windows 上看不到
的 `cfg(target_os)` 模块至少要在目标平台上编译并跑单元测试，但仍不覆盖窗口、
IME、菜单和真实 D-Bus/AppKit 交互。

2026-10-30 `qt-check` 首次在 Linux/macOS 上运行新代码（run 40991895908，两端全绿）：
Linux 构建通过并跑完 132 个单元测试；macOS 通过构建与链接。运行中修掉三处只在
目标平台暴露的问题：`macos_menu.h` 不再依赖 `<QtGlobal>`（独立 `cc` 调用没有 Qt
include path），`macos_platform.mm` 改用 `NSColor` 分量读取（`getRed:…` 在新 SDK
下被判为 `void`），菜单实现补上 `dae_qt::` 限定名；另有 `sidebar_model` 的卷名
用例原来只断言 Windows 盘符拼写，已改成跨平台断言。

### 2026-11-01 §9.3-7 / §9.3-8：IME 失焦守卫三平台化，macOS chrome 定案

**IME 失焦守卫（§2.6 上方的清单一节、`crates/dae-qt/cpp/ime.h`）。**
守卫从「只在 Windows 安装」改成三平台统一，取消原语按平台选，规则写成一句：
*焦点离开时组合被取消，不被提交*。

- Windows 仍是 IMM `CPS_CANCEL`（门槛 2 的原始结论，未改）。
- macOS / Linux 改调 `QInputMethod::reset()`。两个平台的该实现都是「取消」，
  与 Windows 相反，这一点是按 Qt 源码核对的，不是猜的：
  - `qcocoainputcontext.mm`：`reset()` 做 `discardMarkedText` + `unmarkText`
    （后者给控件发一个空预编辑），`setFocusObject()` 同样丢弃 marked text，
    所以 macOS 本来就没有 Windows 那个缺陷；
  - `qibusplatforminputcontext.cpp`：`reset()` 做 `context->Reset()` 并清空
    `d->predit`；只有 `commit()` 会把预编辑当提交串发出去，而它只从
    `invokeAction(Click)` 进来，`setFocusObject()` 走的是 `FocusOut()`。
  因此在这两个平台上守卫是「双保险」：真正的价值是让壳的行为不依赖后端策略
  （XIM、经 ibus 的 fcitx5、以及将来 Qt 改回 commit 的情况）。
- 成本：新增 `focus_has_composition()` 判定——读焦点项自己的 `preeditText`
  属性，为空时整条守卫不触发，所以没有组合时不会多出一次 IMM 调用或一次
  D-Bus `Reset`。Esc 不在守卫里（那次按键归输入法）。

**macOS chrome（§2.8）。** 三条决定与实现见 §2.8，落点：
`qml/TitleBar.qml`（绿灯全屏、hover 字形、双击读偏好）、`qml/Glyph.qml`
（两个新字形）、`cpp/macos_platform.{h,mm}`（读 `AppleActionOnDoubleClick`）、
`crates/dae-qt/src/platform.rs`（三值 → `zoom|minimize|none`，含单测）、
`window.rs`（`titlebarDoubleAction()`）、`ShellWindow.qml`（把控制器传给标题栏）。

**本机验证（Windows 11，Qt 6.12.0）。**

| 项目 | 结果 |
|---|---|
| `cargo build -p dae-app` | 通过（45 s） |
| `cargo test -p dae-qt --lib` | **148 passed**（新增偏好映射用例；顺带把 `finds_a_packaged_entry_under_xdg_data_dirs` 标成 `cfg(unix)`，它的搜索路径按 `:` 切分，Windows 盘符里的冒号会把它切坏） |
| `qmllint qml/TitleBar.qml qml/Glyph.qml` | 无告警 |
| Windows IME 失焦 | 未重测：取消原语一行未改，只加了「有组合才取消」的前置判断；门槛 2 的 214 项检查仍是该路径的证据 |

**未覆盖，写在这里免得被当成已完成。** macOS 的 `macos_platform.mm` 路径不在这台
机器上执行，也没有第二台机器可验：编译结论以 `qt-check` 的 `macos-latest` 腿为准，
运行时结论**永远不会有** —— macOS 产物已按 §9.12.4 撤项，这条路径只为防代码腐烂
而保留。Linux 的 `reset()` 路径已随后续的 Linux 真机手测覆盖（fcitx5、Wayland 与
X11），见 §4 与决策文档 §9.14。

**2026-11-05 P0 手测（Windows）。** `docs/files-parity.md` §10 的六项 P0 在
Windows 上按上表跑过：标签写进 NTFS ADS 并镜像索引、列表标签列/侧栏计数/按类型
分组/创建时间列/多选信息面板均按预期显示；`DAE_QML_LINT=1 DAE_QML_TYPEC=1
cargo build -p dae-app` 通过。Linux 的 xattr 分支已由 Linux 真机手测覆盖
（标签行，§4）；macOS 的 `com.dae.tags` 分支只过了 `qt-check` 编译，且 macOS
不发布（§9.12.4）。
