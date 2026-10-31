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

删掉缓存目录，启动一次，**再看一眼**：

```powershell
Remove-Item "$env:LOCALAPPDATA\dae\cache\qmlcache" -Recurse -Force -ErrorAction SilentlyContinue
# 启动 dae.exe，等窗口出来，关掉
Get-ChildItem "$env:LOCALAPPDATA\dae\cache\qmlcache" -ErrorAction SilentlyContinue
```

目录仍然是空的（或不含本树里的文件名）才说明单元被用上了。又被写进 53 个 `.qmlc`
= loader 没被链接进来，先看 §2.1 那个 anchor，而不是去怀疑 `qmlcachegen`。

另一个廉价信号：`qmlcachegen --dump-aot-stats --module-id …` 的输出，以及
`aotBuiltFunctions[]` 里 `nullptr` 项的数量——那才是「编译成 C++」的函数，
其余是回退到字节码的。

## 6. 还没做

- **热启动的 A/B。** 热缓存那条 851 ms 里仍然要读 `.qmlc`、跑字节码；AOT 之后这两样
  都没了，但没测，别引用。
- **`qmlsc` 的严格面。** `--warnings-are-errors`、`qmllint` 的 `unqualified` 检查没接，
  所以「编译不了只能回退」的绑定不会让构建变红。
- **`.js` 单个文件的收益**：`DialogFocus.js` / `Shell.js` / `Shortcut.js` 一起编了，
  没单独量。
