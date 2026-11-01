# 发布物的两个安装形态：怎么验，验到什么程度

> 对应 [`decisions/engine-qt-quick.md`](decisions/engine-qt-quick.md) §9.2 的三条
> P0（其中 macOS 一条已撤，见 §1）。判据只有两条：一个**能执行的验收动作**，和一处
> **指得到文件与行的证据**。这里的每一步都在某台真机上跑过或被明确标为「未跑」。

产物全部来自 Qt 壳（`apps/dae` + `qml/`），由 `.github/workflows/release.yml`
的 `qt` 矩阵产出。旧 Tauri 壳的产物不在这里。

| 平台 | 文件 | 更新方式 |
|---|---|---|
| Windows | `dae_<version>_x64.msix`、`dae-<version>-windows-x86_64.zip` | MSIX 由系统安装与升级；zip 走 `latest-qt.json` 自更新 |
| Linux | `dae_<version>_amd64.deb`、`dae-<version>-1.x86_64.rpm`、`dae-<version>-linux-x86_64.zip` | deb/rpm 由 apt/dnf/zypper 替换；zip 自更新 |
| macOS | 无 | 2026-11-01 起不再发布，见 §1 |

---

## 1. macOS：暂不打包（2026-11-01 撤销）

### 1.1 为什么撤

Mac 版的链路本身是通的：Runner 上 `macdeployqt`、Developer ID 签名、`notarytool`
公证、`stapler` 装订、`spctl --assess` 逐条都在，任何一条不过就直接 `::error::`。
问题不在能不能**做出**这个包，而在能不能**证明它能用**。

- 这台开发机是 Windows，没有 Mac 设备。撤项前那一套验收——`codesign -dv` 看
  Authority、`spctl` 看 Notarized Developer ID、双击看窗口起不起来——一条都跑不了。
  CI 里的 `spctl --assess` 只证明 Gatekeeper 的静态判断，不证明它在这台机器上
  能启动、能贴边、能收到 Finder 的打开事件。
- Apple 证书那六个 secret 一个都没有配置，而缺任何一个该腿就 `::error::` 退出
  （见旧版 §1.2）。也就是说这条腿本来就跑不过，只是把一个必然失败的环节挂在每次
  发版前面。

**结论：一个没人能验收的平台不发。** 发一个装不上或装上了打不开的包，比不发更糟。

### 1.2 撤掉了什么

| 位置 | 改动 |
|---|---|
| `release.yml` 的 `build` 矩阵 | 删掉 `macos-latest` / `aarch64-apple-darwin` 一条腿 |
| `release.yml` 的 `plan` 作业 | 腿清单少一条，`qt` 矩阵随之只剩 Windows 与 Linux |
| `release.yml` 的 `Deploy Qt runtime and package (macOS)` | 整段删除（114 行，含一次性 keychain、公证、`spctl` 与 zip 断言） |
| `workflow_dispatch.platform_key` | 选项只剩 `linux-x86_64` / `windows-x86_64` |
| `README.md` 的 Install 表 | 删掉 macOS 行，换成一段「没有 macOS 产物，以及为什么」 |

`scripts/qt-update-feed.mjs` **没有改**：它按目录里实际存在的产物生成
`platforms`，只有 `dae-<version>-darwin-*.zip` 和 `.sig` 同时出现才会写
`darwin-*` 键。少了这条腿，`latest-qt.json` 自然只有 `windows-x86_64` 与
`linux-x86_64`，已安装的 Mac 版不会因为「feed 里少了一个键」而误判更新。

### 1.3 留下了什么

macOS 专属的 Rust 代码一行没删，仍然被编译：

- `macos-check.yml`：push 到 `main` 时编译 `dae` crate，专抓
  `#[cfg(target_os = "macos")]` 模块在别的平台上不可见的那类错误；
- `qt-check.yml` 的 `macos-latest` 腿：编译 `dae-app` 并跑 `dae-qt` 的库测试。

两者都只证明**类型与链接**，不证明行为——这正是 §1.1 的问题所在。`packaging/qt/`
下的 `Info.plist` 也保留着（`crates/dae-qt/src/default_manager.rs` 的注释仍指向它）。

### 1.4 想恢复时的清单

1. `build` 与 `plan` 的腿清单各加回一条 `macos-latest`；
2. 把 `Deploy Qt runtime and package (macOS)` 整段加回来（历史版本：本文件 2026-11-01
   之前的 revision，或 `git log -p .github/workflows/release.yml`）；
3. 配置 `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、
   `APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_TEAM_ID`、
   `APPLE_APP_SPECIFIC_PASSWORD` 六个 secret；
4. **在这之前先弄到一台能装 Mac 版并点开的机器**：没有这一步，恢复出来的仍然是
   一条没人验收的链路。

原来的签名与公证设计（为什么不能只做 ad-hoc、`macdeployqt -sign-for-notarization`
的作用、entitlements 里的 `allow-jit` 与 `disable-library-validation`、公证用
`ditto` 打 zip 的顺序）记在
[`decisions/engine-qt-quick.md`](decisions/engine-qt-quick.md) §9.9，恢复时按那份
记录照做即可。

---

## 2. Linux：`.deb` / `.rpm` + GStreamer 运行时

### 2.1 装什么

```bash
sudo apt install ./dae_<version>_amd64.deb        # Debian / Ubuntu
sudo dnf install ./dae-<version>-1.x86_64.rpm     # Fedora / RHEL
sudo zypper install ./dae-<version>-1.x86_64.rpm  # openSUSE
dae                                               # /usr/bin/dae → 启动器，exec /usr/lib/dae/dae
```

两个包的内容完全相同，只有元数据不同（见 §2.2）。包内布局：

```
/usr/bin/dae                                    启动器（exec /usr/lib/dae/dae "$@"）
/usr/lib/dae/                                   Qt 运行时（dae、lib/、platforms/、qml/、qt.conf…）
/usr/share/applications/dae.desktop             菜单项、`inode/directory` 候选、`dae://` 处理者
/usr/share/icons/hicolor/{128x128,256x256}/apps/dae.png
```

`/usr/bin/dae` 是启动器（[`packaging/qt/dae-launcher`](../packaging/qt/dae-launcher)），
**不是**指向 `../lib/dae/dae` 的软链接。曾经是软链接，理由写的是「Qt 用
`/proc/self/exe` 定位 `qt.conf`」——这条是错的：Qt 的 application directory 取的是
**进程启动时那个路径**所在的目录，`/proc/self/exe` 只用来取文件名。于是 0.5.0 的
rpm 装完点图标没有任何窗口，`QT_DEBUG_PLUGINS=1` 显示：

```
qt.core.plugin.factoryloader: checking directory path "/usr/bin/platforms" ...
qt.qpa.plugin: Could not find the Qt platform plugin "xcb" in ""
```

`qt.conf` 明明在 `/usr/lib/dae/`，但 QLibraryInfo 根本不去那儿找。把 `qt.conf` 放进
`/usr/bin` 能让报错消失，代价是系统上**所有**从 `/usr/bin` 启动的 Qt 程序都会读到它并
继承这包的插件路径——所以入口改成启动器：`exec` 之后 app directory 回到
`/usr/lib/dae`，`/usr/lib/dae/qt.conf` 是唯一生效的配置，rpath 仍是 `$ORIGIN/lib`。

两个包都断言这件事：`.deb` 用 `dpkg-deb --fsys-tarfile` 把入口读回来比对 `exec` 行，
`.rpm` 检查 mode 必须是 `0100755`（软链接是 `0120777`），且两者都要求 `qt.conf` 在载荷里。

`/usr/share/applications/dae.desktop` 来自
[`packaging/qt/dae-installed.desktop`](../packaging/qt/dae-installed.desktop)，与
便携 zip 里的 `dae.desktop` 不是同一个文件：便携版只承载 `dae://`，安装版还声明
`MimeType=inode/directory`，让 dae 出现在「打开方式」里，但**不会**因此变成默认
文件管理器 —— 那是设置页里的显式动作（`default_manager`）。

因为系统里已经有这份条目，`linux_desktop::ensure_registered` 在安装版上**不再写**
`~/.local/share/applications/dae.desktop`（只在 `$XDG_DATA_DIRS` 里找不到时才写），
否则用户级副本会按 XDG 优先级挡住系统那份，丢掉 `inode/directory`，并在
`dnf remove` 之后残留。`dae://` 的默认关联照旧修，只是不再写文件。

### 2.1.1 Wayland 的客户端 buffer 插件

Qt Quick 在 Wayland 下走哪条图形路径，由
`plugins/wayland-graphics-integration-client/` 里的插件决定（EGL、shm 两个后端）。
这些插件由 `dlopen` 加载，可执行文件的符号表里没有任何引用，
`linuxdeploy-plugin-qt` 也只部署 Wayland 的**服务端**（`waylandcompositor`），
于是 0.5.0 的包装完是这样：

```
qt: Failed to load client buffer integration: "wayland-egl"
qt: Available client buffer integrations: QList()
qt: Failed to initialize graphics backend for OpenGL.
```

**不能拿发行版的 Qt 补这个目录**：同名的插件在发行版里是针对它自己的 Qt 6.x 编的，
Qt 的插件加载器遇到版本不一致直接拒绝，不会降级。这与整棵运行时都随包带上是同一个
理由，所以 `release.yml` 从 `$QT_ROOT_DIR/plugins/wayland-graphics-integration-client`
整目录复制（不写死文件名，Qt 6 各版本命名不同），并在 zip / `.deb` / `.rpm` 三处
断言目录非空。

插件本身 `dlopen` 的是系统上的 EGL/GLES 运行时（mesa、glvnd）；这部分属于桌面环境
自带，不随包走。WSLg 这类拿不到可用 DRM 驱动的环境里 EGL 初始化会失败
（`MESA-EGL: warning: egl: failed to create dri2 screen`），补插件前后都起不来——补之前
是加载不到后端、补之后是崩在 EGL 初始化，两者都不是「打不开窗口」而是直接退出。
这类环境用 `QT_QUICK_BACKEND=software` 起。

### 2.2 依赖是推导出来的，不是手写的

`release.yml` 对整棵树（`dae` + `lib/*` + 各 plugin 目录 + `qml/**`）跑 `ldd`，
把解析到 `$staging` 之外的库挑出来，再用 `dpkg -S` 反查提供它的包。理由很直接：
Qt 之外还依赖 xcb、wayland、fontconfig、dbus 等一批系统库，手写清单在下一个
发行版就会漏。

链接依赖（`Depends`）与解码插件（`Recommends`）分开：

```
Depends: libc6, libstdc++6, libx11-6, libxkbcommon0, …（由 ldd 得出）
Recommends: gstreamer1.0-plugins-base, gstreamer1.0-plugins-good,
            gstreamer1.0-plugins-bad, gstreamer1.0-libav
```

`Recommends` 是 `apt` 默认安装的，所以普通用户装完即可播；把它写成 `Depends` 会
在无桌面/服务器镜像上强行拉进整套 GStreamer，而文件管理器本身不需要解码器。

**`.rpm` 走的是另一条路，因为它本来就更强**：`rpmbuild` 的 ELF 生成器直接问
soname（`libxcb.so.1()(64bit)`），与发行版的包名无关，所以 Fedora / RHEL /
openSUSE 吃同一个包，不需要第二份手写清单。要注意的一点：Qt 是随包带上的，生成器
会把 `libQt6Core.so.6()(64bit)` 同时写进 `Provides` 和 `Requires` —— 包自己满足
自己的 require，这是「自带运行时」的正确形状。`release.yml` 因此断言的是
**每个 `libQt6*` 的 require 都必须在 Provides 里**，而不是「不能出现 Qt require」：
后者在真的漏包时会静默放过，前者一旦有库没进 `%files` 就立刻失败。

rpm 里也关掉了 `%__os_install_post` 与 debuginfo：二进制是 cargo 产出的，PGO 与
BOLT 已经跑过，发行版的 strip 钩子只会把已经处理好的东西再改一遍。

GStreamer 在两个包里的名字不同（Fedora/RHEL：`gstreamer1-plugins-base`、
`gstreamer1-plugins-good`、`gstreamer1-plugin-libav`；openSUSE：
`gstreamer-plugins-*`），spec 里按 `%{?suse_version}` 分支，其余走 Fedora 命名。

### 2.3 缺少 GStreamer 时的表现

Qt Multimedia 在 Linux 上的后端是 GStreamer。后端整套缺失（或编解码插件缺失）
时，Qt 的 `MediaPlayer` 只在 `error` 上报 `ResourceError`/`FormatError`，什么也
不做；更糟的情况是**连后端都没加载**，`play()` 之后状态停在 `NoMedia`、`error`
仍是 `NoError` —— 这就是「静默失败」。

`qml/MediaPreview.qml` 因此有两道提示：

1. `failed`（`player.error !== NoError`）：显示 Qt 自己的 `errorString`；
2. `backendUnavailable`：点了播放 2 秒后仍停在 `NoMedia`、没有错误，判定没有后端
   应答。Linux 上文案直接点名要装的包
   （`gstreamer1.0-plugins-base / -good / -libav`）。

两个条件合成 `unplayable`，播放按钮同时变灰且不可点，避免用户反复点一个永远不会
开始的播放。

### 2.4 验收

```bash
# Debian / Ubuntu
dpkg-deb --info dae_<version>_amd64.deb          # Depends / Recommends 如上
sudo apt install ./dae_<version>_amd64.deb
dae &                                            # 窗口出现，菜单里有 dae
xdg-mime query default x-scheme-handler/dae      # dae.desktop
xdg-open 'dae://?path=%2Ftmp'                    # 复用已运行实例
sudo apt remove dae                              # /usr/lib/dae 整棵树消失，无残留

# Fedora / RHEL
rpm -qip dae-<version>-1.x86_64.rpm              # Requires 全是 soname，无 Qt
rpm -qp --recommends dae-<version>-1.x86_64.rpm  # gstreamer1-*
sudo dnf install ./dae-<version>-1.x86_64.rpm
QT_DEBUG_PLUGINS=1 dae 2>&1 | head -3            # checking directory path "/usr/lib/dae/platforms"
rpm -V dae                                       # 装完文件校验通过
sudo dnf remove dae
```

两个包**都已在 WSL 里用真实 ELF 载荷验过**：`dpkg-deb --build` 通过、`control`
字段正确、启动器与平台插件都在包里；`rpmbuild` 出的包 `Provides` 里有
`libQt6Core.so.6()(64bit)`、`Requires` 只有 libc/libm/rpmlib，`Recommends` 是
`gstreamer1-*`，解开后 `/usr/bin/dae` 是 mode `0100755` 的启动器，二进制能直接运行
（见 [`decisions/engine-qt-quick.md`](decisions/engine-qt-quick.md) §9.10）。

安装形态另有一次真机复验（Fedora 44 / WSLg）：把 rpm 解到 `/usr/lib/dae`、启动器装成
`/usr/bin/dae`，`QT_DEBUG_PLUGINS=1` 下所有插件目录都落在 `/usr/lib/dae/`（`platforms`、
`imageformats`、`theme`…），`QT_QUICK_BACKEND=software` 跑满 20 秒无崩溃。0.5.0 之前的
软链接形态在同一台机器上复现出上面的 `Could not find the Qt platform plugin`，两次对比
是这次把入口从软链接改成启动器的依据。

**已由用户在真机上装包验证（2026-11-01）：** 覆盖安装后从菜单点图标启动正常，§2.1 的
`Could not find the Qt platform plugin` 与 §2.1.1 的 `Failed to load client buffer
integration` 都不再出现。也就是说三件事同时被证实：入口是启动器（不是软链接）、
`qt.conf` 从 `/usr/lib/dae` 生效、`wayland-graphics-integration-client` 里的 EGL
后端能被加载。

还剩两处没有独立验过，都属环境而非包：`dae://` 关联的端到端（点链接复用已运行实例）
与 `apt`/`dnf remove` 后的零残留；依赖推导用的是桩 `dpkg`/真实 `ldd`，真实输入只有
CI 的 `qt` 作业能给。

---

## 3. Windows：MSIX 的包身份、`dae://` 与升级路径

### 3.1 包身份带来的三件事

装成 MSIX 之后，进程有 **package identity**（`dae-core::package::is_packaged()`，
用 `GetCurrentPackageFullName` 做零长度探测，按进程问，不按路径猜）：

1. **自更新关闭。** 安装目录在 `C:\Program Files\WindowsApps\<name>_<version>_
   <arch>__<publisher>`，只读，而且每次更新换名字。`updates_enabled()` 因此返回
   false。UI 不再假装能更新：菜单项改成「由安装包更新」，打开更新对话框会说明
   「此安装来自 Windows 应用包」。
2. **`dae://` 由清单声明。** `activation::claim()` 在打包运行时跳过
   `HKCU\Software\Classes\dae` 写入 —— 再写一条按用户的命令只会把 scheme 钉在
   下一次更新就会消失的路径上。
3. **默认文件管理器：路径选对了，但写不出去。** 清单声明了
   `windows.appExecutionAlias`（`Alias="dae.exe"`），Windows 把它暴露为
   `%LOCALAPPDATA%\Microsoft\WindowsApps\dae.exe`，跨版本稳定。
   `default_manager` 在打包运行时确实把 `dae.folder` 指向这条别名而不是版本目录，
   `claim()` 每次启动也只重写「目标已不存在」的注册 —— 一个仍能解析的注册
   （别人在用的、或便携版被挪走前的旧路径还在时）不会被旁边的开发构建抢走。

   **但打包应用的 `HKCU\Software\Classes` 写入会被 Windows 重定向到每应用注册表**
   （`%LOCALAPPDATA%\Packages\<PFN>\SystemAppData\Helium\UserClasses.dat`），
   真实注册表不变，Explorer 看不到。2026-11-01 的真机证据见
   [`decisions/engine-qt-quick.md`](decisions/engine-qt-quick.md) §9.15：日志报
   「re-pointing it at …\WindowsApps\dae.exe」且无错误，真实 `HKCU` 却仍是旧值，
   而 `UserClasses.dat` 里能找到 `dae.folder`、`DefaultIcon`、`command`、
   `OpenWithProgids` 这些键名。

   结论：**MSIX 版里「设为默认文件管理器」当前不生效**，需要清单声明或改用系统
   设置的引导，属于未决的产品项 —— 候选方案列在 §9.15 末尾。

### 3.2 验收脚本

```powershell
# 只读：从 .msix 里读 AppxManifest.xml，不改动机器
pwsh scripts/msix-verify.ps1 -InspectOnly

# 升级路径：先装旧包再装新包，检查完一起卸载
pwsh scripts/msix-verify.ps1 -Previous .\dae_0.5.0.1_x64.msix -TrustCertificate -Uninstall
```

**证书要先信任一次，而且需要管理员。** 部署按机器级存储验签，用户级
`TrustedPeople` 不算数，缺了会得到 `0x800B0109`：

```powershell
winapp cert install devcert.pfx --password password
```

**旧包从哪儿来。** 历史 tag 里没有 `pack-msix.ps1`（MSIX 是 Qt 迁移期间才加的），
所以不能「checkout 上一个 tag 再打包」。连着跑两次 `pack-msix.ps1` 就行：
`sync-msix-version.mjs` 会把第四段 revision 自增（同一版本 Windows 拒绝安装），
于是手上有一个 `0.5.0.1` 和一个 `0.5.0.2`，前者就是 `-Previous` 要的旧包。

`-Uninstall` 是「走完整个流程后卸载」，不是纯卸载 —— 它仍会先安装一遍。只想清掉
已装的包：

```powershell
Get-AppxPackage -Name dae | Remove-AppxPackage
```

脚本检查的东西，以及每一条为什么值得检查：

| 检查 | 失败意味着 |
|---|---|
| `Identity Version` 四段、与包内一致 | Windows 拒绝升级（版本不变）或拒绝安装 |
| 清单里有 `windows.protocol` = `dae` | 点 `dae://` 链接会开一个「没有关联」的提示框 |
| 清单里有 `appExecutionAlias` = `dae.exe` | 注册表里的启动命令会指向升级前的目录 |
| 安装后 `platforms\qwindows.dll`、`qml\QtQuick`、`multimedia` 在 | 旧包只有 `dae.exe` 也能「装成功」，然后打不开窗 |
| 别名路径存在 | 默认文件管理器注册不可用 |
| 升级后旧安装目录消失、只注册了一个版本 | Windows 变成了并存安装，两次打开会各开一个窗口 |
| 进程真的起来了、日志写进 `%LOCALAPPDATA%\com.melaine.dae\logs\dae.log` | 全信任包不被重定向，日志必须与便携版同一处 |
| 深链后进程数仍为 1 | 转发失败，成了两个实例抢一个窗口 |
| `HKCU\…\dae.folder\shell\open\command` 指向 `WindowsApps\dae.exe` | **只记录，不判失败**：打包应用的写入被重定向到每应用注册表（§3.1 第 3 条），所以这条在 MSIX 下永远拿不到别名；输出 `NOTE` 而不是 `FAIL`，指向便携版的旧值则是安装前遗留 |

### 3.3 状态

- **清单、打包链路已验证**：`winapp pack` 通过，包内 `AppxManifest.xml` 含
  `windows.protocol` 与 `appExecutionAlias`；`scripts/msix-verify.ps1 -InspectOnly`
  对同一包给出 PASS（改动前对旧包给出 FAIL，即该检查确实在测东西）。
- **真机安装与升级已跑通（2026-11-01）**：`Add-AppxPackage` 走完
  `0.5.0.1 → 0.5.0.2`，除 `dae.folder` 那条 `NOTE` 外全绿。要点——
  替换而非并存（旧安装目录被删、只注册一个版本）；安装树完整（`qwindows.dll`、
  `qml\QtQuick`、`qml\QtMultimedia`、`multimedia`）；打包实例能启动且日志落在
  `%LOCALAPPDATA%\com.melaine.dae\logs\dae.log`（full-trust 不被重定向）；深链在
  两种情形下都只有一个进程（有便携版注册时转发给同一实例，注册被删后由包清单独立
  拉起）；卸载后包注册、别名、安装目录、包数据目录全部消失。
- **第一次跑就修掉了脚本的三个缺陷**：`Import-Certificate` 不接受 `.pfx`；
  PowerShell 7 的该 cmdlet 没有 `-Cert` 参数（`-Cert` 会前缀匹配到
  `-CertStoreLocation`）；信任写错了存储层级（用户级 → 机器级，并在非提权时给出
  可执行的提示而不是让用户对着 `0x800B0109` 猜。
- **未决**：`dae.folder` 在打包形态下写不进真实注册表（§3.1 第 3 条），
  即 MSIX 版「设为默认文件管理器」不生效。细节与候选方案见
  [`decisions/engine-qt-quick.md`](decisions/engine-qt-quick.md) §9.15。

---

## 4. 仍然不在这两个形态里的事

- **macOS 的一切运行时手测** —— §1 撤掉发布之后，这类问题连入口都没有了；macOS 窗口
  chrome 的三条待定项（红绿灯 hover 字形、绿灯语义、双击标题栏读系统偏好，§9.3-8）
  随之搁置，代码保留但不验收；
- Linux 侧的 12 项运行时手测 —— §9.3-5，回填
  [`qt-platform-parity.md`](qt-platform-parity.md) §6；
- MSIX 走系统部署 API 做应用内「检查更新」—— 需要 `packageManagement` 能力与
  Store 之外的签名信任，属独立决定（§9.7.3-3 的后半段）。
