# QML 预编译：把 53 个文件的首启编译挪到构建期

> **状态：已接入。release 默认开启，debug 默认跳过；首启收益已实测，热启收益未测。**
> 接法在 `apps/dae/build.rs` 的 `mod aot`，改动只有那一个文件加两个 workflow 的
> 环境变量。测出来的数字只有一组（Windows，`dist\dae.exe`），换平台前别当成结论。

## 1. 为什么

`rcc` 只把 `.qml` 源码装进二进制。于是每次「冷」启动——装完第一次、升级后第一次、
或者用户清过缓存——`QQmlApplicationEngine` 都要把整棵树解析、编译一遍，再写 53 个
`.qmlc`（2.0 MB）进 `%LOCALAPPDATA%\dae\cache\qmlcache`。

5 轮交替测量，进程 spawn → 首个窗口句柄：

| 状态 | 各轮 ms | 中位 |
|---|---|---|
| 改造前 · 缓存热 | 863, 874, 851, 849, 851 | 851 |
| 改造前 · 缓存冷 | 1137, 1120, 1118, 1122, 1115 | 1120 |
| 改造后 · 缓存热 | 828, 856, 845, 877, 831 | 845 |
| 改造后 · 缓存冷 | 848, 860, 863, 822, 873 | 860 |

**冷的那 265 ms（首启的 23%）就是解析 + 编译 + 写缓存**，预编译之后不再发生：冷启动
1120 → 860 ms，且与热启动持平（845，差异在噪声里）；启动后
`%LOCALAPPDATA%\dae\cache\qmlcache` **一个 `.qmlc` 都不会再写**。热启动没有可宣称的收益。

## 2. 是怎么接的

CMake 的 `qt_add_qml_module` 会自动做两件事，cxx-qt-build 没有对应实现，所以这里手工复刻：

1. **每个文件一个编译单元**：
   `qmlcachegen --resource-path /qt/qml/dae/qml/<f>.qml -I <qml_modules> -i <qmldir> -I qml -o <out>.cpp <f>`
2. **一个 loader**：`qmlcachegen --resource-name dae_qmlcache -o …/dae_qmlcache_loader.cpp <各文件的资源路径…>`
   —— 输出文件名必须以 `qmlcache_loader.cpp` 结尾、并给 `--resource-name`，
   `qmlcachegen` 才进 loader 模式（见 Qt 的 `tools/qmlcachegen/qmlcachegen.cpp`：
   `outputFileName.endsWith("qmlcache_loader.cpp")`）。

loader 生成的是一个 `QQmlPrivate::RegisterQmlUnitCacheHook`，把
`qrc:` 路径映射到各单元导出的 `qmlData` / `aotBuiltFunctions`。引擎
`load("qrc:/qt/qml/dae/qml/Main.qml")` 时先查这张表，命中就用 native unit。

`.qml` 仍然留在 qrc 里：单元是**缓存**，不是替代品。查表按 URL 命中；某个单元的
lookup 签名对不上（Rust 侧属性变了而单元没重编）时，引擎回退到编译源码，不会失败。

### 2.1 `cpp/qml_cache_init.cpp` 是必须的，不是样板

`cxx-qt-build` 把所有 C++ 编进**一个静态归档**，而静态归档只在有符号被引用时才交出
对应的 object。loader 靠 CRT 构造函数自注册，没有任何外部引用它——于是链接器把它
（以及它引用的每一个单元的 `qmlData`）全丢掉，引擎照旧在运行时编译 QML。

这个坑是实测出来的，不是推演：只生成单元、不加 anchor 的构建**启动照旧、日志照旧**，
唯一的信号是 `%LOCALAPPDATA%\dae\cache\qmlcache` 又被写进了 53 个 `.qmlc`。

所以 `main.rs` 里那次 `dae_qml_cache_init()` 调用只有一个作用：制造那条引用链
（Rust → anchor object → loader → 各单元）。它由 `DAE_QML_AOT` 宏守着，
没有生成单元的构建里它是个空函数。

**Rust 类型要可见，否则编译成果会打折。** `crates/dae-qt` 的 build script 已经把
`qmldir` 与 `plugin.qmltypes` 导出到 `target/cxxqt/qml_modules/dae/`，这里通过
`-I <qml_modules> -i <qmldir>` 交给 `qmlcachegen`，`import dae 1.0` 的
`#[qobject]` 类型才解析得到。实测 `qml/FileList.qml` 编译出 **210 个 AOT 函数、
0 个 fallback**（`aotBuiltFunctions[]` 里只有结尾的 `{0,0,nullptr,nullptr}`）。

## 3. 开关

| `DAE_QML_AOT` | 行为 |
|---|---|
| 未设置 | `PROFILE=release` 时开，失败降级为警告；debug 跳过 |
| `1` | 任何 profile 都开，**失败即构建失败**（CI 用这个） |
| `0` | 关 |

定位工具：`DAE_QMLCACHEGEN` → `QMAKE -query QT_INSTALL_BINS`（与
`crates/dae-qt/build.rs` 用的是同一个 Qt）→ `PATH`。找不到就降级，不拦构建。

`qt-check.yml` 与 `release.yml` 的 Qt 腿都设了 `DAE_QML_AOT=1`：前者保证
Linux / macOS 在合入前就看到预编译的错误（这个 job 存在的理由就是不让 release
标签成为那两个平台的第一个编译器），后者保证发出去的包真的带单元。

## 4. 成本

- 生成 C++ 约 **18 MB / 51 个单元**，`qmlcachegen` 自身约 5.5 s。
- 这些 `.cpp` 要跟着编一遍：debug 下有单元时 `cargo build -p dae-app` 实测 26–29 s；
  release 只重编 `dae-app` 并做 fat LTO 链接时 2m02s（链接本来就要这么久）。
- QML 改动会让 build script 重跑；只要它重跑，cxx-qt 那整个 C++ 归档都会重编
  （cc-rs 的陈旧判断在这一层不是按文件生效的），debug 下实测 +25 s。所以编辑 QML 的
  迭代成本在 debug 上是正常的，在 `DAE_QML_AOT=1` 的检查/发布构建上是 +25 s 起。
- debug 因此默认跳过——那不划算。

## 5. 怎么确认它真的生效

**缓存目录不是信号。** 旧版这一节让人删掉
`%LOCALAPPDATA%\dae\cache\qmlcache` 再启动，看有没有重新写进 53 个 `.qmlc`。在 Qt
6.12 上这个检查**证明不了任何事**：qrc 里的 QML 无论是否 AOT 都不写这个目录——
用 `DAE_QML_AOT=0` 编的二进制连跑几次，目录同样不存在。

现在的检查是**计时**：同一份代码编两次（`DAE_QML_AOT=0` 与 `=1`），交替跑，比
`qml-loaded`：

| 构建 | `qml-loaded` 中位（8 组交替） |
|---|---:|
| `DAE_QML_AOT=0`（运行时编译） | 921.6 ms |
| `DAE_QML_AOT=1`（预编译单元） | 905.6 ms |

差值只有 **~16 ms**。AOT 仍然让构建失败可见（`DAE_QML_AOT=1` 是 fatal），但别再引用
§1 的 265 ms——那是另一个指标（spawn → 窗口句柄）、非交替测量，而且包含冷缓存下写
2 MB `.qmlc` 的磁盘成本，不是编译本身。

`qmlcachegen --dump-aot-stats` 仍可用来确认某个文件确实编成了 C++（`aotBuiltFunctions[]`
里的 `nullptr` 项是回退到字节码的函数）。

## 6. 首启 `engine.load` 的 924 ms 拆开了（2026-10-31）

**前提是 AOT 真的生效。** 按 §5 的交替计时确认（差值 16 ms 量级，不是缓存目录）。
同一批运行里 `qml-loaded` 中位 954.6 ms，与 §1 改造后的冷启动 860 ms 同一量级，
和改造前的 1115–1137 ms 已经分开。

`DAE_STARTUP_PERF=1` 现在多三个点。它们由 QML 与模型各自标记
（`crates/dae-qt/src/startup.rs`），同名阶段只打印第一次：

| 点 | 位置 | 含义 |
|---|---|---|
| `qml-root-completed` | `ShellWindow.qml` 的 `Component.onCompleted` 第一条 | 对象树建完 |
| `qml-first-model` | `model.rs` / `overview_model.rs` 发布行之前 | 首屏数据 |
| `qml-first-list` | 两个 `ListView` 的 `onContentHeightChanged` | 首屏布局 |

`qml-first-model` 标在模型里、不是 QML 里：QML 的 `onModelReset` / `onCountChanged`
在视图自己的连接**之后**才跑，会把布局算成数据。第一版就是 QML 标记，实测
`qml-first-list` 比 `qml-first-model` 早 5–6 ms，顺序倒挂。

7 次 release + AOT（Windows，`target\release\dae.exe`，中位）：

| 分段 | 中位 | 占 `engine.load` |
|---|---:|---:|
| `qml-engine` → `qml-root-completed`（实例化） | **745.4 ms** | **80.6%** |
| `qml-root-completed` → `qml-first-model`（建 tab + 概览扫描 + 模型） | 161.7 ms | 17.5% |
| `qml-first-model` → `qml-loaded`（`onCompleted` 余下 + load 返回） | 17.2 ms | 1.9% |
| `qml-loaded` → `qml-first-list`（列表布局） | 3.8 ms | 0.4% |
| **`qml-engine` → `qml-loaded`** | **924.2 ms** | 100% |
| `qml-loaded` → `first-frame`（首次绘制） | 278.0 ms | — |

**结论：AOT 之后 `engine.load` 的大头是实例化，不是编译，也不是数据。** 977 ms 里
745 ms 花在把 `ShellWindow` 那棵树的对象建出来（`Component.onCompleted` 之前的
`engine.load` 内部），首屏数据 162 ms，列表布局只有 4 ms。首批行到达时 `engine.load`
还没返回，布局发生在它返回之后、下一轮事件循环里；随后到首帧的 278 ms 才是绘制。

**数字的稳定性。** 同一二进制、同一天，不同批次之间绝对值会漂 ±10–15%
（实例化在 622–827 ms 之间出现过，批次内很稳，批次之间不稳），所以表里只承诺比例：
实例化约 80%、数据约 17%、布局 <1%。换机器或换 Qt 版本要重测，别引用绝对值。

**下一步该看哪里。** 实例化 745 ms 是新的最大项，但它不是「编译 QML」——§7 用探针
把对象数量、绑定、单元加载、Rust 对象和大组件逐个排除掉之后，指向的是每个组件文件
第一次被实例化时的引擎侧准备成本。首帧前的 278 ms 是第二项，和首屏数据同量级，
比布局本身大两个数量级。

## 7. 首启实例化的 ~700 ms 排除了什么（2026-10-31）

§6 把 `engine.load` 拆成「实例化 745 ms / 数据 162 ms / 布局 4 ms」之后，下一步是找出
那 745 ms 花在哪。结论先说：**它不是对象数量、绑定、单元加载、Rust 对象或某一个大组件，
而是每个组件文件第一次被实例化时的引擎侧准备成本。**

用 `DAE_STARTUP_PERF=1` 与几个临时探针（均已移除）量到的边界：

| 假设 | 测法 | 结果 |
|---|---|---|
| 单元加载（AOT 反序列化） | `QQmlComponent` 只加载不创建 | 45 ms（Main.qml）；再预载 10 个文件各 <0.1 ms |
| 对象数量 | `Component { Item {} }` 创建 1000 个 | **0–3 ms** |
| 绑定求值 | 1000 个带 anchors + `Theme.*` 绑定的 Rectangle | **8–11 ms** |
| Rust 对象构造 | 单独 `createObject` AppConfig / TrashModel / OverviewModel / ShellCommandModel | 每个 **0 ms** |
| Canvas（542 个 Glyph） | 把 Glyph 的 Canvas 换成 Rectangle 做交替 A/B | 无差异 |
| 终端项（profiler 报 638 ms） | 在 `TerminalItem` 构造函数两端加标记 | **2 ms** |
| 子组件 `onCompleted` | Sidebar 的 `Spaces.refresh` + `sidebarModel.refresh` 两端加标记 | 各 **0.2 ms**，且都在根 `onCompleted` **之后**才跑 |
| 同一棵树第二次实例化 | 运行时再建一个 `ShellWindow` | **~206 ms**（含它自己的启动工作），说明类型热了以后树本身很便宜 |
| 未在首启路径上的组件类型 | 首次 vs 第二次 `createObject`（SettingsPanel / CommandPalette / OpenWithDialog / BulkRenameDialog） | 首次 0–21 ms，第二次 0–13 ms |

把树拆开量之后，剩下的解释只有「每个组件文件第一次被用到时，引擎要为它建类型、准备
绑定表、准备单元数据」，按每个文件 5–15 ms、首启路径约 50 个文件算，正好是 700 ms
的量级。第二次 `ShellWindow` 只花 ~50 ms 创建，因为所有类型都已经热了。

**所以 AOT 之后能砍的不是「编译」，而是「首启路径上碰到的组件类型数量」。** 已经
LazySurface 掉的 14 个面板不再计入。菜单（DisplayMenu / FilterMenu / ContextMenu /
MenuList）也量过：profiler 的计数里 `MenuList.qml` 一共创建了 **1152 个对象**
（全树 6113 的 19%），按每对象 ~8 µs 算约 **10 ms**；把它们改成按需创建要先动 19 处
`menu.*` 引用，收益远小于风险，不做。

**工具注意事项。** `qmlprofiler` 能连上，但代价很大：要 `QT_QML_DEBUG`
（`DAE_QML_PROFILE=1` 可给 release 打开，见 `crates/dae-qt/build.rs`），而 QML 调试
服务会**绕过 AOT 单元**，并且每个事件都有开销。用它得到的 `TerminalItem` 638 ms
与实测 2 ms 差了三个数量级——**不要用 profiler 的绝对时间做结论**，只用它的事件计数
（哪些组件被创建了多少个）。

## 8. 首帧前的 ~290 ms 是 GPU 初始化，不是 QML（2026-10-31）

§6 的 `qml-loaded` → `first-frame` 这一段（中位 278 ms）一直没拆开。换渲染后端就
能看出来：同一二进制、同一棵树，只改环境变量：

| 后端 | `qml-loaded` | `qml-first-list` | `first-frame` | load → frame |
|---|---:|---:|---:|---:|
| D3D11（默认） | 958.2 ms | 974.7 ms | 1247.0 ms | **289 ms** |
| `QT_QUICK_BACKEND=software` | 987.1 ms | 991.0 ms | 1044.9 ms | **58 ms** |

QML 侧的布局只有 ~4 ms，软件后端画完整棵树到第一帧也只要 ~58 ms；换成 D3D11 多出来的
**~230 ms 是设备/交换链创建和第一帧上传**，不是布局、不是文本整形、也不是 Canvas
（§7 已把 Canvas 排除）。

**提前显示窗口不能把它重叠掉。** 把 `ShellWindow` 的 `visible: false` 改成 `true`，
让窗口在 `engine.load` 期间就暴露、让渲染线程先建设备，交替 A/B 反而更差：

| | `qml-first-list` 中位 | `first-frame` 中位 |
|---|---:|---:|
| 加载完成后才显示（现状） | 991.6 ms | 1206.6 ms |
| 一开始就显示 | 1228.4 ms | 1243.1 ms |

渲染线程和 GUI 线程抢 CPU，布局被推迟 230 ms，首帧没有提前。已回退。

**结论**：这一段要么接受（D3D11 初始化是平台成本），要么走 Qt 内部（提前建 RHI
设备、或换成 `QQuickRenderControl` 预热），都不是 QML 侧能改的。真正还能动的是
§7 的组件类型数量——那需要 `qmltc` 级别的改造，不在当前计划里。

## 9. 还没做

- **热启动的 A/B。** 热缓存那条 851 ms 里仍然要读 `.qmlc`、跑字节码；AOT 之后这两样
  都没了，但没测，别引用。
- **`qmlsc` 的严格面。** `--warnings-are-errors`、`qmllint` 的 `unqualified` 检查没接，
  所以「编译不了只能回退」的绑定不会让构建变红。
- **`.js` 单个文件的收益**：`DialogFocus.js` / `Shell.js` / `Shortcut.js` 一起编了，
  没单独量。
