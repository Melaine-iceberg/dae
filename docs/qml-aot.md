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
§7 的组件类型数量——那需要 `qmltc` 级别的改造；这类改造只保留为构建期校验，见 §10。

## 9. 还没做

- **qmlsc 的严格面。** `qmllint` 已接（§12），但 `qmlcachegen --warnings-are-errors`
  没有接：一份能解析同目录组件的模块环境里仍有约 1800 条 `[compiler]` 提示（未标注
  类型的 JS 函数/调用、数组写入非序列类型等），它们是「这段绑定编不成 C++、运行时
  回退」的性能提示，不是错误。清零是另一个量级的改造。
- **AOT 对同目录组件的解析（已试，收益为零）。** `qmlcachegen` 不做隐式目录导入，
  `Glyph`、`DialogHeader` 这类同目录类型在单元里未知，实例化回退到运行时查找。
  补模块 qmldir 并显式 `import dae 1.0` 能把回退从 2464 降到 1010，但热启动不变，
  已回退，见 §13。
- **`.js` 单个文件的收益**：三个 `.js` 单元的 `aotBuiltFunctions[]` 都是 0 项
  （JS 只预编成字节码，没有 AOT 函数），所以不存在「AOT 函数」层面的收益；单独再量
  它们省下的解析时间会低于启动抖动，不做。

## 10. `qmltc` 只做构建期校验（2026-10-31）

`apps/dae/build.rs` 的 `mod typec` 在 `DAE_QML_TYPEC=1` 时对每个 `.qml` 文档跑一次
`qmltc`，失败即构建失败（`qt-check.yml` 的 Qt 腿开这个开关）。**它不产出任何运行时
代码**：49 个文档里只有 `qml/MediaPreview.qml` 被跳过——Qt 6.12 QtMultimedia 自带的
qmltypes 把 `QAudioBufferOutput` 标成 `isTypeOpaque: true`，qmltc 拒绝为 opaque 属性
生成 setter；错误在 Qt 的元数据，不在本树，类型导出后删掉 `SKIPPED` 条目即可。

要让 qmltc 跑通，下面几件事缺一不可：

- **`--resource` 映射。** qmltc 拒绝不在 qrc 里的文件（"No resource paths for file"）。
  build.rs 按 `QML_FILES` 生成 `files.qrc`，与 rcc 的资源别名同源，避免「按一个路径
  编译、按另一个路径挂载」。
- **剥掉 cxx-qt 的 opaque 占位。** 生成的 `plugin.qmltypes` 把只在属性里出现的
  `QString`/`bool`/`double` 等内建类型写成 `isTypeOpaque: true` 的 `Component`
  占位；qmllint 接受，qmltc 报每个 `dae` 类型的每个属性 "Type of property 'x' is
  unknown"。校验前复制一份并删掉这些块，生成文件本身不动。
- **`depends QtQuick`。** 列表模型的 prototype 是 `QAbstractListModel`，cxx-qt 不会
  从 prototype 名字推断依赖，所以在 `crates/dae-qt/build.rs` 里显式
  `QmlModule::new("dae").depend("QtQuick")`；校验用的 qmldir 副本也带这一行。
- **`Shortcut.js` 的导入别名。** `import "Shortcut.js" as Shortcut` 会遮蔽 QML 的
  `Shortcut` 类型，qmltc 报 "Shortcut was not found. Did you mean Shortcut?"。四个
  文件统一改成 `as ShortcutJS`。
- **两个手写模块的元数据。** `qml/dae/terminal/` 与 `qml/dae/activation/` 补了
  `qmldir` + `qmltypes`，描述运行时注册的 `TerminalItem` 与 `Activation` 单例。
  它们只给工具链（qmltc/qmllint）看，不进 qrc。

### 运行时接线的尝试与回退

运行时路径（生成的构造函数替换 `engine.load`）试过，**已放弃**，原因不只是「省不到
100 ms」：

- qmltc 把生成的类放在匿名命名空间里。`Main.qml` 继承 `ShellWindow`，基类定义和它
  的 `staticMetaObject` 必须与子类在同一个 TU，所以只能用「全部 49 个单元 + 各自的
  moc + 入口」拼一个大 TU；还要给生成头补 `qqmlintegration.h`、补直接依赖头，并把
  同一头里的基类定义排到子类前（`PropertiesDialog.qml` 先声明 `PermissionRow`，后
  声明它内联继承的 `Toggle`）。
- 这个 TU 依赖私有 API（`private/qqmltcobjectcreationhelper_p.h` 等）与
  `QtQmlModels`/`QtQmlMeta`；在 Windows 上还卡在 `QQuickCanvasItem` 没有
  `Q_QUICK_EXPORT`（`Glyph.qml` 的 Canvas 用到），链接直接失败。
- 勉强链上之后，窗口在 25 s 内**没有完成构造**：`ExplorerPrefs is not defined`、
  `Binding loop detected for property "tabs"`、`QQuickColorValueType` 构造错误等。
  同一份构建的 AOT 基线是 `qml-loaded` 809.56 ms、`first-frame` 1102.47 ms，qmltc
  侧拿不到可比的数字。

§11 实测热启动下 AOT 只省 ~50 ms，运行时 qmltc 即便成功也基本没有空间，
却要背上私有 API、单 TU 与平台导出限制。所以最终形态是本节开头的「只做校验」。


## 11. 热启动的 A/B（2026-10-31）

§5 的 16 ms 是在更早的一组交替测量里得到的。热启动这一条一直没测：同一份 release
代码编两次（`DAE_QML_AOT=0` / `=1`），Windows，`DAE_STARTUP_PERF=1`，每次
`timeout 5` 启动，8 组交替（奇偶轮换先后顺序），取中位：

| 构建 | `qml-loaded` 中位 | `first-frame` 中位 |
|---|---:|---:|
| `DAE_QML_AOT=0` | 877.7 ms | 1090.8 ms |
| `DAE_QML_AOT=1` | 823.7 ms | 1038.4 ms |
| 差值 | **54.0 ms** | **52.4 ms** |

第一次启动（DLL、字体缓存都冷）那一对是 1429.5 → 968.4 ms，差距更大，但只有一对，
不能当结论；其余七对都在 ±60 ms 内。

Qt 6.12 的 qrc QML 不写 `.qmlc` 目录（§5），所以这里省下的不是磁盘缓存，而是每次
启动都要做的源码解析与字节码编译。热启动下 AOT 仍然值 ~50 ms，但量级不大——这和
§6 的结论一致：AOT 之后的大头是实例化，不是编译。

## 12. `qmllint` 接入与 `qmlsc` 的严格面（2026-10-31）

`apps/dae/build.rs` 的 `mod lint` 在 `DAE_QML_LINT=1` 时对每个文档跑 `qmllint`
（同 `mod typec` 的 `-I target/cxxqt/qml_modules -I qml -i qml/qmldir`），输出里出现
任何 `Warning:`/`Error:` 行即构建失败；`qt-check.yml` 已开这个开关。它补的是
qmlcachegen 不报的那一层：绑定不再解析时，AOT 单元只会退化成运行时查找，构建照样
成功。

接入前清了 149 条告警，全树现在为零：

| 类别 | 数量 | 处理 |
|---|---:|---|
| `unqualified` | 125 | 两个文件补 `pragma ComponentBehavior: Bound`；嵌套组件里对 delegate/父级属性的访问改成 `id.属性` |
| `missing-property` | 19 | `Loader.item` 用 `as MediaPreview` / `as SidebarTree` 转换；真正动态的 4 处和 `QStringList.length`（qmllint 类型模型没有这个成员）用 `// qmllint disable missing-property` 精确豁免 |
| `id-shadows-member` | 4 | `header`、`overviewModel` 两个 id 与属性同名，改名 |
| `import` | 1 | `qmllint` 显式 `-i qml/qmldir`，否则 `Theme` 的单例声明看不到 |

**`--max-warnings 0` 在 Qt 6.12 上不能用。** 对 `ShellWindow.qml` 它会让 qmllint 以
127 退出且不打印任何东西（`1`–`10000` 都正常）。所以 `mod lint` 不依赖该选项，改为
解析报告里的 `Warning:` / `Error:` 行。升级 Qt 后可以再试。

**`qmlcachegen --warnings-are-errors` 没有接。** 在给全树补上显式 `import dae 1.0`
并生成完整模块 qmldir 的原型环境里，仍剩约 1800 条 `[compiler]` 提示：409 条未类型化
的 JS 调用、272 条未标注类型的函数、117 条数组写入非序列类型，以及 `QJSValue` 查找、
`QtObject::platform.os` 遮蔽等。它们是 qmlsc「编不成 C++、运行时回退」的提示，不是
错误，清零等于重写大部分 JS 绑定，暂不接入。

**顺带发现的 AOT 盲点。** `qmlcachegen` 不做隐式目录导入：`qml/Toast.qml` 引用同目录
的 `Glyph` 时，AOT 单元里 `Glyph` 是未知类型（`Toast.qml` 仍有 93 个 AOT 函数，但
每次实例化 `Glyph` 都回退到运行时查找）。补模块 qmldir 并显式 `import dae 1.0` 后，
同一文件的告警从 33 条降到 1 条。覆盖率确实提高了，但启动没有收益，见 §13。

## 13. AOT 同目录组件：覆盖率上去了，启动没变（2026-10-31）

§12 末尾那条盲点做完了，结论是**不值得**，改动已全部回退。全树 AOT 回退从 2464 条
降到 1010 条（编译成功 4037 → 5491，总条目都是 6501），但热启动与实例化都在噪声内
不动。

### 怎么做的

构建期生成一个 `dae` 模块 `qmldir`：`typeinfo`（cxx-qt 的 `plugin.qmltypes`）+
全部组件条目；编译时把 `.qml`/`.js` 复制到同目录，`qmlcachegen -I <模块根>` 编译
副本。模块随树挂到私有根 `qrc:/qt/qml/dae_aot/dae/`，24 个缺省的文件补
`import dae 1.0`。编译产物里同目录类型因此从“未知、运行时查找”变成限定的
`dae/<Type>`。

### 三个实现上的硬约束

1. **`qmlcachegen` 不读隐式目录。** 它用 `--resource-path`（qrc URL）而不是本地源
   文件路径去算隐式导入目录；没有 `--resource` 映射时这个目录读不出来。把文件复制
   到临时目录并在旁边放一个无 `module` 行的 qmldir 不会让兄弟类型可见。只有模块
   qmldir 能把它们变成 `dae/<Type>`。
2. **运行时签名校验不认 `prefer`。** Qt 的 `Validator::getTypeFromModule`
   （`qqml.cpp`）先用 `urlsForModule(uri)` 取第一个 qrc 模块目录，再直接拼
   `<目录>/<qmldir 里的文件名>`；它不解析 `prefer`。所以“编译期模块 + prefer 指回
   `:/qt/qml/dae/qml/`”的单元会在运行时被整体拒绝（`QV4_FAIL_ON_INVALID_AOT=1`
   会直接退出）。要过校验，qmldir 必须和 QML 文件同目录——这也是最终把挂载点移到
   私有根 `qrc:/qt/qml/dae_aot/dae/` 的原因。
3. **手写 qmltypes 的 `name` 必须是 C++ 全名。** 非复合类型校验走
   `QMetaType::fromName(name + "*")`；`TerminalItem` 注册成 `dae_qt::TerminalItem`，
   而 `terminal.qmltypes` 原来写的是不带命名空间的 `TerminalItem`，`TerminalPanel.qml`
   的单元一直校验失败。改成 `dae_qt::TerminalItem` 后全部 50 个文档通过严格校验。
   这条与布局无关，将来若单独修，只需改那一行。

### 测量

Windows，release，`DAE_STARTUP_PERF=1`，每组 8 轮交替（奇偶轮换先后），取中位；
实例化 = `qml-root-completed` − `qml-engine`：

| 构建 | 实例化 | `qml-loaded` | `first-frame` |
|---|---:|---:|---:|
| 旧 AOT（基线） | 613.3 ms | 778.3 ms | 988.8 ms |
| 新 AOT（同目录模块） | 616.6 ms | 783.5 ms | 990.4 ms |
| 新 AOT | 614.9 ms | 778.4 ms | 981.6 ms |
| 新构建 · `DAE_QML_AOT=0` | 621.3 ms | 788.0 ms | 992.8 ms |

旧/新 AOT 的差在噪声内；新树里 AOT 相对无 AOT 只剩 **~10 ms**（§11 量到的是 ~54 ms，
但那次的无 AOT 基线是 877.7 ms，机器状态不可直接比）。**实例化没有变化**，与 §7 的
结论一致：AOT 之后的大头是引擎为每个组件文件做的首次准备，不是绑定/函数能不能编成
C++。覆盖率从 62% 提到 84% 也没有动这一项。

所以止损在这一步：不保留模块目录、全树导入和私有挂载点，`docs` 只留这条负结果。
再往上就不是 QML 侧能改的了。
