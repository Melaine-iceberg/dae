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
6. Linux/macOS 只有编译验证，没有运行时回归。

以下 P0/P1 缺口已按 §3 分阶段落地，逐项状态见 §6；标「已补」的条目只代表代码与
Windows 本机构建通过，macOS/Linux 的运行时结论仍以 §4 手测为准。

## 1. 已经对齐，不再列为待办

| 能力 | 状态 |
| --- | --- |
| 三平台编译 | `qt-check` 在 ubuntu-26.04 + macos-latest 上 `cargo build -p dae-app` |
| 窗口移动 | Windows 原生 hit test；Linux/macOS `startSystemMove` |
| 背景材质 | Windows Mica/Acrylic；Wayland `ext-background-effect-v1`；macOS `NSVisualEffectView` |
| 文件操作 / 回收站 | `dae-core` 三平台后端；macOS `~/.Trash` + put-back 记录 |
| 图标 | Windows Shell、Linux xdg、macOS NSWorkspace |
| 缩略图 | Windows Shell handler、Linux `.thumbnailer`、macOS QuickLook（§2.7） |
| 终端 | `portable-pty` + `alacritty_terminal`；Unix 读 `$SHELL` |
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

#### 2.8 macOS 窗口 chrome 是自绘的

- 现状：红绿灯由 QML 画（`qml/TitleBar.qml`），绿色按钮走 `showMaximized`
  而不是全屏；没有悬停字形；双击标题栏由 QML 自己实现，不遵循系统偏好。
- 这是产品决定，不是缺陷；但需要在 macOS 上明确定义：
  - 绿色按钮：最大化（zoom）还是原生全屏；
  - 是否补 hover 字形；
  - 双击标题栏是否读取系统偏好。
- 验收：与 macOS 原生文件管理器的窗口操作习惯不冲突；状态栏/标签不被遮挡。

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
3. 记录 Linux/macOS 手测结果（Finder/深链/菜单/缩放/主题/卷/缩略图）。

## 4. Linux / macOS 手测清单

| 项目 | 操作 | 期望 |
| --- | --- | --- |
| Finder 打开 | 运行中 Finder → 右键文件夹 → 打开方式 → dae | 新标签到该目录 |
| 冷启动文件 | 退出 dae，`open -a dae /tmp` | 启动并到 `/tmp` |
| 深链 | 浏览器打开 `dae://open?path=/tmp` | 复用/新建标签，窗口到前台 |
| 默认管理器 | 设置 → 默认文件管理器 → 设为默认，双击 Finder 文件夹 | dae 打开并定位 |
| 菜单 | Cmd+Q / Cmd+H / Cmd+M / 窗口菜单 | 行为与原生一致 |
| 缩放 | 拖动八条边/四角 | 窗口连续缩放，不闪 |
| 主题 | 系统切深色/浅色 | 立即切换 |
| 强调色 | 设置选「系统强调色」，改系统强调色 | 立即跟随 |
| 卷 | 插拔 U 盘 | 侧栏出现/消失 |
| 缩略图 | 选中 PDF / mp4 / HEIC | 显示缩略图 |
| 输入法 | 拼音输入中点击别处 / Tab | 组合取消，不提交原始拼音 |
| 拖放 | 从 Finder 拖文件进列表；从列表拖到 Finder | 复制/移动语义正确 |

## 5. 非目标

- 不把 Windows 的 Mica 像素复制到 Linux/macOS；
- 不引入 Qt Quick Controls 重写现有组件；
- 不为 X11 造背景模糊；
- 不在本阶段解决 macOS 公证与 Linux 包管理器分发（需要证书/打包决策）。

## 6. 进度记录

| 阶段 | 状态 | 证据 |
| --- | --- | --- |
| 文档 | 已完成 | 本文件 |
| 阶段 1 · QFileOpenEvent | 代码完成，待 macOS 手测 | `cpp/activation.{h,cpp}` 安装 `QFileOpenEvent` 过滤器，统一走 `Activation::forwardExternal` |
| 阶段 1 · 原生菜单 | 代码完成，待 macOS 手测 | 新增 `cpp/macos_menu.{h,mm}`；claim 后安装 App/File/Edit/Window 菜单 |
| 阶段 1 · 缩放 fallback | 代码完成，待 macOS 手测 | `qml/ResizeHandles.qml` 在 `startSystemResize` 返回 false 时手工改窗口几何 |
| 阶段 1 · 激活到前台 | 代码完成，待 macOS 手测 | `cpp/macos_backdrop.mm` 新增 `dae_qt_macos_activate_app`；`dae_qt_focus_window` 调用 |
| 阶段 2 · 主题/强调色 | 代码完成，待 macOS/Linux 手测 | `cpp/platform_watch.cpp` 接 `colorSchemeChanged`；`platform.rs` 增加 macOS `NSColor` 与 Linux portal 读取；非 Windows 每 2 s 轮询 accent |
| 阶段 3 · 卷监听 | 代码完成，待 macOS/Linux 手测 | 非 Windows 每 2 s 比较 `dae_core::fs::sidebar::disk_signature()`；Windows 仍走 `WM_DEVICECHANGE` |
| 阶段 4 · macOS 缩略图 | 代码完成，待 macOS 手测 | 新增 `crates/dae-core/src/fs/thumbnails/macos.rs`：`qlmanage -t` 私有目录 + 5 s 超时 + PNG 校验 |
| 阶段 5 · Linux 桌面/发布 | 代码完成，待 Linux 手测 | `linux_desktop::ensure_registered` 写 `dae.desktop` 并注册 scheme；`release.yml` 断言 Wayland 平台插件 |
| 阶段 6 · 验证 | 本机验证完成，待目标平台 | Windows 构建/测试通过；引擎文档 §4.2 已同步；`qt-check` 增加 `dae-qt` 测试步骤；macOS/Linux 手测未执行 |

2026-10-30 阶段 1 本机验证：`cargo build -p dae-app` 通过（44 s）；
`cargo test -p dae-qt --lib` 133 passed；`qmllint qml/ResizeHandles.qml` 无告警。
macOS 专有的 Objective-C++ 与 `QFileOpenEvent` 路径不在 Windows 上编译，
其编译结论以 `qt-check` 为准，运行时结论以 §4 手测为准。

2026-10-30 阶段 2 本机验证：`cargo build -p dae-app` 通过；`cargo test -p dae-qt --lib`
135 passed（新增 portal palette 与 RGB 解码用例）。`cpp/platform_watch.cpp` 在
Windows 上参与编译和链接，确认 cxx 桥的命名空间与符号正确；macOS `NSColor` 与
Linux portal 的实际读取仍待目标平台手测。

2026-10-30 阶段 3–5 本机验证：`cargo build -p dae-app` 通过；
`cargo test -p dae-qt --lib` 138 passed（新增 Linux desktop entry 用例），
`cargo test -p dae-core --lib` 301 passed / 10 ignored。macOS QuickLook 与
Linux portal / xdg-mime 路径不在 Windows 上执行，仍需按 §4 手测。

阶段 6 同时把 `cargo test --package dae-qt --lib` 加进 `qt-check`：Windows 上看不到
的 `cfg(target_os)` 模块至少要在目标平台上编译并跑单元测试，但仍不覆盖窗口、
IME、菜单和真实 D-Bus/AppKit 交互。

2026-10-30 `qt-check` 首次在 Linux/macOS 上运行新代码（run 40991895908，两端全绿）：
Linux 构建通过并跑完 132 个单元测试；macOS 通过构建与链接。运行中修掉三处只在
目标平台暴露的问题：`macos_menu.h` 不再依赖 `<QtGlobal>`（独立 `cc` 调用没有 Qt
include path），`macos_platform.mm` 改用 `NSColor` 分量读取（`getRed:…` 在新 SDK
下被判为 `void`），菜单实现补上 `dae_qt::` 限定名；另有 `sidebar_model` 的卷名
用例原来只断言 Windows 盘符拼写，已改成跨平台断言。
