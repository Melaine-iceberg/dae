# Qt Bridge 迁移执行手册

> **状态：进行中。** 分支 `feat/qtbridge`，已有 3 个提交，4 个 QObject 已迁移并通过
> 构建、单元测试和整机启动验证；其余约 20 个 QObject、`main.rs` 和 `cxx-qt-build`
> 的移除尚未完成。本文是给下一次对话直接执行用的，不需要上一次对话的上下文。

- 目标：`crates/dae-qt` 从 CXX-Qt 0.10 全量迁到 [Qt Bridge for Rust](https://doc.qt.io/qtbridge-rust/)（beta，API 已按 0.4.1 固定）。
- 依赖版本：`qtbridge = "0.4.1"`，MSRV 1.88，Qt ≥ 6.10。本机 Qt 6.12.0：
  `K:/Qt/6.12.0/msvc2022_64`，`qmake.exe` 在同目录 `bin/`。
- 分支：`feat/qtbridge`（基于 `main` @ `913f219`）。`main` 未被改动。

| 提交 | 内容 |
|---|---|
| `9988d08` | 可行性闸门 `bench/qtbridge-spike`（三个 C++ 结合点 + 抽象模型 + 自建 build.rs） |
| `6a68e6d` | `Startup`、`Session`、`TagStore` → qtbridge |
| `ed5f4f8` | `AppConfig` → qtbridge（保留 C++ extern 的混合形态） |

## 0. 三条硬约束（先读，决定后面所有写法）

1. **qtbridge 不支持 per-type C++。** 它用运行时 `DynamicMetaObject`，没有 moc/生成
   C++ 的入口。任何需要 `Q_OBJECT` 子类、C++ 属性、C++ 槽的类型都只能留在 C++，
   或者改设计。
2. **QML 模块名 = 宏所在 crate 的 `CARGO_PKG_NAME`。** 本 crate 叫 `dae-qt`（不能当
   QML URI），所以**每个类型都手写 `QmlElement` impl** 把 URI 固定成 `"dae"`，
   元素名与版本也写死。收益：30 个 `.qml` 文件一行都不用改，AOT 的模块名也不变。
3. **`cxx-qt` 与 `qtbridge` 可以共存于同一个二进制。** 已验证：未迁移的类型继续由
   cxx-qt 生成的插件注册，qtbridge 类型在 `dae_qt::init()` 里显式注册。迁移因此可以
   按模块推进，每一步都能编译、能跑。

## 1. 现状

已迁移（QML 无改动，构建 / 217 个 lib 测试 / 整机启动全部通过）：

| 文件 | 类型 | 形状 |
|---|---|---|
| `startup.rs` | `Startup` | 单例，1 个 slot，无状态 |
| `session.rs` | `Session` | 单例，2 属性 + 5 slot；`String` 替换 `QString` |
| `tags.rs` | `TagStore` | 单例，2 属性 + 2 signal + 11 slot；`Vec<String>` 替换 `QStringList` |
| `config.rs` | `AppConfig` | 普通 element，6 个 **Constant** 属性；C++ 调用保留在文件内的 `#[cxx_qt::bridge]` extern 块 |

剩余（原始规模，供排序和估算）：

| 文件 | 类型 | props | slots | signals | inherit | 单例 | C++ 依赖 |
|---|---|---:|---:|---:|---:|---|---|
| `appearance.rs` | `Appearance` | 6 | 6 | 0 | 0 | 是 | translator.h |
| `network_discovery.rs` | `NetworkDiscovery` | 3 | 1 | 0 | 0 | 是 | — |
| `preferences.rs` | `ExplorerPrefs` | 36 | 11 | 0 | 0 | 是 | — |
| `shell.rs` | `Shell` | 0 | 5 | 0 | 0 | 是 | activation / platform_watch 等 |
| `updater.rs` | `Updater` | 12 | 2 | 1 | 0 | 是 | — |
| `shell_command_model.rs` | `ShellCommandModel` | 4 | 8 | 0 | 2 | 否 | — |
| `open_with_model.rs` | `OpenWithModel` | 9 | 4 | 0 | 2 | 否 | — |
| `sidebar_model.rs` | `SidebarModel` | 2 | 2 | 0 | 2 | 否 | — |
| `trash_model.rs` | `TrashModel` | 4 | 7 | 0 | 2 | 否 | — |
| `spaces_model.rs` | `Spaces` | 3 | 15 | 1 | 2 | 是 | — |
| `overview_model.rs` | `OverviewModel` | 4 | 12 | 0 | 2 | 否 | — |
| `sidebar_tree_model.rs` | `SidebarTreeModel` | 4 | 2 | 1 | 8 | 否 | — |
| `search_model.rs` | `SearchModel` / `ContentSearchModel` | 19 | 22 | 0 | 6 | 否 | — |
| `model.rs` | `DirectoryListModel` | 26 | 36 | 4 | 8 | 否 | — |
| `workspace.rs` | `Workspace` | 45 | 84 | 12 | 0 | 是 | drag / clipboard / C++ 调用 |
| `window.rs` | `WindowController` | 4 | 5 | 0 | 0 | 否 | window.h / ime.h |
| `terminal.rs` | `TerminalSession` | 4 | 8 | 2 | 0 | 否 | terminal_item.h |
| `preview.rs` | `PreviewController` | 26 | 3 | 0 | 0 | 否 | image_provider / syntect |
| `properties.rs` | `PropertiesController` | 48 | 8 | 2 | 0 | 否 | image_provider? |
| `i18n.rs`（非 QObject） | — | — | — | — | — | — | translator.h |

总计约 270 个属性、240 个 invokable、23 个 signal 待迁，28k 行里大部分是这些
QObject 的实现体。

## 2. 已由闸门验证的事实

`bench/qtbridge-spike` 是一个独立 workspace，`cargo run` 跑 2 秒后自退出，输出 6 个
mark。它证明（Windows 11 / Qt 6.12 / qtbridge 0.4.1）：

| 现有实现 | qtbridge 对应物 | 结论 |
|---|---|---|
| `#[qproperty]` / `#[qsignal]` / `#[qinvokable]` | `qproperty!` / `#[qsignal]` / `#[qslot]` | 通过；slot 字符串参数是**owned `String`** |
| `QAbstractListModel` + 26 roles + 手工 `begin/endInsertRows`、`dataChanged` | `QAbstractItemModel` 基类 + `QAbstractItemModelBase` | 通过；26 roles 无上限（15 个的上限只属于 `QListModel`） |
| `QQuickImageProvider` 注册 | `QApp::qml_engine()`（`#[doc(hidden)]`）→ 自己的 C++ `addImageProvider` | 通过 |
| C++ `QQuickItem` 注册为 QML 类型（终端画笔） | 原样保留，独立模块 `dae.terminal` + `qmlRegisterType` | 通过 |
| `cxx-qt-build` 的 Qt 发现 / moc / include / link | 自写 `build.rs`（`qmake -query` + `cc` + `moc`） | 通过，约 60 行 |

`Session`/`TagStore` 的迁移还额外验证了：单例、`String` 参数、`Vec<String>`
参数和信号、属性 notify、以及 `cargo test -p dae-qt --lib` 全绿。

## 3. 单模块迁移配方

以 `startup.rs` 为例，完整流程如下；其它模块机械重复。

### 3.1 替换 bridge 声明

CXX-Qt：

```rust
#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Startup = super::StartupRust;

        #[qinvokable]
        fn mark(self: Pin<&mut Startup>, stage: &QString);
    }
}
```

qtbridge：

```rust
use qtbridge::{QmlElement, qobject};

#[derive(Default)]
pub struct Startup;

#[qobject(NoQmlElement)]
impl Startup {
    #[qslot]
    fn mark(&mut self, stage: String) {
        crate::window::startup_mark_once(&stage);
    }
}

impl QmlElement for Startup {
    const URI: &str = "dae";
    const ELEMENT_NAME: &str = "Startup";
    const MINOR_VERSION: u8 = 0;
    const MAJOR_VERSION: u8 = 1;
    const IS_SINGLETON: bool = true;
}
```

要点：

- `NoQmlElement` 阻止宏按 crate 名（`dae_qt`）注册第二份类型。
- **`Singleton` 不能和 `NoQmlElement` 同时写**（宏直接报
  `NoQmlElement is not compatible with Singleton`）。单例就写
  `#[qobject(NoQmlElement)]`，在手动 impl 里 `IS_SINGLETON = true`。
- QML 名字来自 `ELEMENT_NAME` 和属性名，不来自 Rust 标识符。
- 属性：`qproperty!("tagsJson", Member = tags_json, Notify = tags_json_changed);`
  - 只读快照用 `Constant`（不能有 `Write`/`Notify`），如 `AppConfig`。
  - 可写成员属性的 `Notify` 信号必须自己声明并**手动 emit**；qtbridge 不会替你在
    字段写入时发信号。这就是 `self.as_mut().set_x(v)` 的替代。
- slot 名：QML 要 camelCase 时给 impl 加 `ConvertToCamelCase`（`save_window` →
  `saveWindow`），否则用 Rust 名原样暴露；属性名永远来自字符串字面量。

### 3.2 从 cxx-qt 列表里摘掉

`crates/dae-qt/build.rs` 的 `.files([...])` 删掉该文件。

**例外**：文件里还保留 `#[cxx_qt::bridge]` 的 `extern "C++"` 块（例如 `config.rs`
调用 `dae_qt_system_dark`），则文件继续留在列表里，只把 `RustQt` 半部删掉。已验证
cxx-qt-build 接受只有 C++ extern 的 bridge。

### 3.3 注册

`crates/dae-qt/src/lib.rs::init()` 追加 `Type::register();`（`use qtbridge::QmlElement as _;`
已在函数内）。必须在 `Main.qml` 被解析之前；`init()` 正是这个时机。
`main.rs` 里也有一处 `dae_qt::init()` 调用，不要另起注册点。

### 3.4 验证（每个模块都要跑）

```powershell
# 构建（PATH 必须是 Windows 形式，QMAKE 也是）
$env:PATH = "K:\Qt\6.12.0\msvc2022_64\bin;$env:PATH"
$env:QMAKE = "K:\Qt\6.12.0\msvc2022_64\bin\qmake.exe"
cargo build -p dae-app
```

```bat
:: 测试与运行都要先给子进程一个含 Qt DLL 的 PATH，否则 0xC0000135
@echo off
set PATH=K:\Qt\6.12.0\msvc2022_64\bin;%PATH%
set QMAKE=K:\Qt\6.12.0\msvc2022_64\bin\qmake.exe
cd /d K:\code\dae
cargo test -p dae-qt --lib
target\debug\dae.exe
```

启动检查：`set DAE_STARTUP_PERF=1` 后应看到
`qml-root-completed` / `qml-first-model` / `qml-loaded` / `first-frame`，且 stderr
**没有** `ReferenceError` / `TypeError` / `is not defined` / `Cannot assign`。
`qml-root-completed` 就是 `Startup.mark` 打出来的，等于顺带验证了 QML→Rust 通路。

### 3.5 提交

一个模块一个提交，按现有风格 `feat(qt): port <Type> to Qt Bridge`，正文写清
“为什么这个模块可以独立迁 / 验证了什么”。

## 4. 类型映射

| CXX-Qt | qtbridge | 备注 |
|---|---|---|
| `QString` | `String` | 属性、参数、返回值都换；`&QString` → `String` |
| `QStringList` | `Vec<String>` | 跨桥是 `QList<QString>` 或 `QStringList`，QML 侧都是 JS array |
| `QList<i32>` 等 | `Vec<T>` | 支持 bool/整数/浮点/`String` 的 `Vec` |
| `QByteArray` | **无** | 只按 `QList<u8>` 过桥；见 §7.4 |
| `Pin<&mut Self>` | `&mut self` | 同时删掉 `use std::pin::Pin` |
| `self.as_mut().set_x(v)` | `self.x = v; self.x_changed();` | 信号需自己声明 |
| `self.as_mut().foo()` | `self.foo()` | 基类方法调用同理 |
| `#[cxx_name = "camelCase"]` | `ConvertToCamelCase` 或属性字符串 | 同一类型里两种命名混用时，逐个用字符串/`qslot` 名控制 |
| `#[qml_singleton]` | 手动 `IS_SINGLETON = true` | 见 §3.1 |
| `#[qproperty(..., cxx_name = "x")]` | `qproperty!("x", Member = ...)` | |
| `QModelIndex`/`QVariant`/`QHash` | `qtbridge::qtbridge_type_lib::{QModelIndex, QVariant, QHash_i32_QByteArray, QByteArray}` | 与 cxx-qt-lib 是同一批类型（0.3 起重导出） |

## 5. 列表模型怎么迁（剩余最大的一块）

qtbridge 的 `QAbstractItemModel` 基类几乎逐项对应现有实现，26 个 role 也不受限。
`QListModel`（最多 15 role、要求 `Item: Clone + QModelItem`）**不要用**——它会把
120k 行的紧凑行改成逐项 clone，性能和语义都变。

```rust
#[qobject(Base = QAbstractItemModel, NoQmlElement)]
mod rows {
    use super::*;
    use qtbridge::{QAbstractItemModel, QAbstractItemModelBase};

    pub struct Model { /* 现有字段原样 */ }

    impl QAbstractItemModel for Model {
        fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        fn parent(&self, child: &QModelIndex) -> QModelIndex;
        fn row_count(&self, parent: &QModelIndex) -> i32;
        fn column_count(&self, parent: &QModelIndex) -> i32;
        fn data(&self, index: &QModelIndex, role: i32) -> QVariant;
        fn role_names(&self) -> QHash_i32_QByteArray;   // 逐条 insert，替换 roleNames()
        fn set_data(&mut self, index: &QModelIndex, value: &QVariant, role: i32) -> bool;
    }
}

impl QmlElement for Model { /* URI "dae"，ELEMENT_NAME，IS_SINGLETON = false */ }
```

- 平铺列表的 `index` 用 `self.create_index(row, column, ptr as usize)`，
  `data` 里用 `index.internal_pointer_mut()` 取回指针；`parent` 返回
  `QModelIndex::default()`。树形（`sidebar_tree_model` 是扁平化的，仍按平铺处理）
  才需要真的算 parent。
- 通知：`QAbstractItemModelBase` 提供 `begin_insert_rows/end_insert_rows`、
  `begin_remove_rows/end_remove_rows`、`data_changed`、`begin_reset_model/end_reset_model`。
  现有 `self.as_mut().begin_insert_rows(&QModelIndex::default(), a, b)` 这类调用一一对应。
- `roleNames()` 的 `QHash_i32_QByteArray` 构造方式不变（`QHash::default()` + `insert`）。
- `data()` 里 `QVariant::from(&QString::from(...))` 可直接沿用。
- 模型是 `#[qml_element]` 普通类型（`IS_SINGLETON = false`），QML 里 `Model {}` 实例化，
  QML 文件无需修改。

建议顺序：`sidebar_model` → `trash_model` → `spaces_model` → `shell_command_model` →
`open_with_model` → `overview_model` → `search_model` → `sidebar_tree_model` → `model`。

## 6. 建议的推进顺序

1. **叶子单例**：`appearance` → `network_discovery` → `preferences`（36 属性，但自成一体）。
2. **模型**：见 §5。每迁一个都跑 `cargo test -p dae-qt --lib`（模型测试覆盖最全）。
3. **中量对象**：`updater`、`shell`、`window`、`terminal`、`preview`、`properties`。
4. **`workspace`**：45 属性 + 84 slot + 12 signal，且是跨线程重灾区（见 §7.2）。放最后，
   因为它的实现体最大，改动风险最高。
5. **`main.rs`**：换成 `QApp`（见 §7.3），删除 `cxx-qt-build`、moc 产物、`cxx-qt` /
   `cxx-qt-lib` 依赖，`build.rs` 缩成纯 C++ + `qtbridge` 需要的部分。

## 7. 已知缺口与处理方案

### 7.1 QML AOT / qmltypes

`apps/dae/build.rs` 用 `qmlcachegen` 预编译 53 个 QML 文件，输入是 cxx-qt 导出的
`plugin.qmltypes` + `qmldir`。qtbridge 运行时注册，不产出 qmltypes，所以已迁移类型
（`Startup` 等）会从类型表里消失，release 构建的 AOT 步骤会**降级**（不是失败），
冷启动退回 1120 ms（+265 ms）。

方案（推荐）：模块名仍是 `dae`，`apps/dae/build.rs` 的 `prepare_module` 已经会生成
一份给 `qmltc` 用的 `qmldir` + 净化后的 qmltypes。把已迁移类型按**不透明类型**补进去即可：

```
Component {
    name: "Startup"
    exports: ["dae/Startup 1.0"]
    isSingleton: true
    isCreatable: false
    isTypeOpaque: true
}
```

需要为每个已迁移类型维护一条（名字、是否单例、属性/方法可省略）。等上游提供
qmltypes 导出后可删。验收：`DAE_QML_AOT=1 cargo build --release` 不降级，且
`%LOCALAPPDATA%\dae\cache\qmlcache` 不再写 `.qmlc`。

### 7.2 线程

`cxx_qt::Threading` + `qt_thread()`（55 处调用、15 个 impl）在 qtbridge 里换成
`QThreadHandle`：

- 在 Qt 线程上从对象拿 handle：`obj.borrow().get_thread_handle()`（示例见
  `qtbridge-runtime/src/qthreadhandle.rs` 文档）。
- 迁到 worker：`handle.invoke_mut(|o| { o.x = v; o.x_changed(); })`。
- 无目标投递：`QThreadHandle::default().invoke(|| …)`。
- 目标已销毁时闭包被丢弃，不会 use-after-free；但如果 Qt 线程上该对象正被可变借用，
  会 abort（嵌套事件循环场景要检查）。
- `workspace.rs` / `model.rs` 的 `spawn_blocking` 回投路径逐条对照迁移。

### 7.3 `main.rs` 与引擎

- `QApp::new()` 内部创建 `QGuiApplication` + `QQmlApplicationEngine`。
- `QApp::qml_engine()` 是 `#[doc(hidden)]`，只给 `QQmlEngine*`：
  - 翻译器可经它 attach（`i18n::attach_engine` 收 `usize`，传指针即可）；
  - 图像 provider 用它（已验证）；
  - 占位窗 / `objectCreated` / 延迟 `engine.load` 需要 `QQmlApplicationEngine`：
    在 C++ 侧用 `dynamic_cast<QQmlApplicationEngine*>(qmlEnginePtr)` 拿回子类指针
    （对象确实是 `QQmlApplicationEngine`），不要 `reinterpret_cast`。
- 现有 `main` 的顺序必须保留：elevated 分支 → `startup_begin` → `dae_qt::init()`
  （alpha/diagnostics/fonts/settings/terminal 注册）→ `claim` → 占位窗 → AOT cache
  init → 引擎 → i18n → provider → `watch_first_frame` → load。
- `QApp` 只暴露 `new()` 和 `run()`，**没有**“app 已建、engine 未建”的插入点。可用的
  顺序是：`QApp::new()` → `dae_qt::init()` → provider/translator → `load_qml*`。
  这在语义上仍然正确，因为 `QApp::new()` 创建 engine 但**不创建窗口、不解析 QML**：
  `enable_alpha()` 改的是 `QQuickWindow` 的静态项（要求在第一扇窗之前），
  `apply_fonts()` / `QFontDatabase` 只要求 `QGuiApplication` 存在。不要把 `init()`
  提前到 `QApp::new()` 之前（那里还没有 `QGuiApplication`）。
- `QApp::load_qml_from_file("qrc:/qt/qml/dae/qml/Main.qml")` 可替代 `engine.load`；
  但 shell-first 的延迟加载仍要走 C++ 的 `load_after_shell`。
- 退出码/`run()`：现有 `application.exec()` 换成 `app.run()`。

### 7.4 `QByteArray`（终端，必须先处理）

qtbridge 只按 `Vec<u8>`（跨桥 `QList<uchar>`）过二进制，没有 `QByteArray`。终端的
C++ 画笔已经用 `QMetaObject::invokeMethod` 按 **`QByteArray`** 取帧和写键：

- `crates/dae-qt/cpp/terminal_item.cpp:258` —
  `Q_RETURN_ARG(QByteArray, frame)` 调用 `snapshot`；
- `crates/dae-qt/cpp/terminal_item.cpp:711` —
  `Q_ARG(QByteArray, bytes)` 调用 `writeBytes`。

槽改成 `Vec<u8>` 后这两处元调用会失败（签名不匹配，`invokeMethod` 返回 false，终端
空屏）。做法是改 C++ 侧，而不是改 QML：

1. `snapshot` 的取帧改成 `QList<quint8>`（`Q_RETURN_ARG(QList<quint8>, list)`），
   再 `QByteArray frame(reinterpret_cast<const char*>(list.constData()), list.size())`
   交给现有的 `decodeFrame`；
2. `writeBytes` 改成传 `QList<quint8>`（`QByteArray::toList()` 没有直接 API，
   用 `QList<quint8>(bytes.begin(), bytes.end())` 或 `std::vector` 转换）；
3. 若 `invokeMethod` 报未知类型，补 `qRegisterMetaType<QList<quint8>>()`。

`TerminalSession` 本身在 QML 里是强类型属性
（`qml/Tab.qml:45` 的 `property TerminalSession terminal`），qtbridge 类型可以直接用，
不需要改；`TerminalItem` 的 `Q_PROPERTY(QObject* session)` 也照旧接收它。

### 7.5 C++ QML 类型（终端画笔）

`TerminalItem` 是 `QQuickPaintedItem`，qtbridge 永远无法在 Rust 里表达。保留现状：

- `qmlRegisterType<TerminalItem>("dae.terminal", 1, 0, "TerminalItem")` 在 C++ 里；
- QML `import dae.terminal 1.0` 不变；
- 它的 `Q_PROPERTY(QObject* session)` 接收的是 qtbridge 暴露的 `TerminalSession`
  包装对象（QML 传对象引用即可，QML 不关心来源）。
- 该 C++ 文件用自写 `build.rs` 的 `cc` 编译（见闸门 `bench/qtbridge-spike/build.rs`）。

### 7.6 图像 provider / 平台 C++

`icons.rs`、`preview.rs`、`window.rs`、`activation.rs`、`ime.rs`、`shell.rs` 里的
`extern "C++"` 都保留为 `#[cxx_qt::bridge]`（或后续改成 `#[cxx::bridge]`），只迁
`RustQt` 半部。它们与 qtbridge 类型没有类型层面的耦合，只需要引擎指针，§7.3 已覆盖。

## 8. 陷阱清单（踩过的）

- **不要跑 `cargo fmt -p dae-qt`**：仓库当前不是 rustfmt-clean，它会顺手重排
  `model.rs`/`workspace.rs` 等无关文件的 import（本次已产生过一次 400+ 行噪声并撤回）。
  只格式化自己改的文件，或改完手工检查 `git diff`。
- 本仓库工作区是 CRLF，`core.autocrlf=true`。写入 LF 没问题，但用脚本做字符串替换时
  不要把 `\n` 和 `\r\n` 混着匹配（已踩过）。
- `#[qobject(NoQmlElement)]` 下 `Singleton` 参数会被宏拒绝，见 §3.1。
- 属性/元素的 QML 名字全靠手动 impl 和字符串字面量，写错不会编译报错，只会在运行时
  出现 `ReferenceError`。每次迁完必须跑一次真实启动。
- 重复注册（同一 URI/名字注册两次）Qt 会打 warning 并保留第一份；看到
  `Cannot install element ... already registered` 说明某个类型两边都还在注册。
- `qproperty!` 的 `Notify` 必须是**同一个 impl 块内**的 `#[qsignal]`；字段改了不发信号
  QML 不会更新，编译器不会提醒。
- `qtbridge` 的 slot 参数是 owned `String`，不是 `&str`/`&QString`；返回 `String`。
- `Constant` 属性不能有 `Notify`/`Write`，适合 `AppConfig` 这种一次性快照。
- 测试/运行的可执行文件直接跑会 `0xC0000135`（缺 Qt DLL），必须经带 Windows PATH 的
  `cmd` 批处理启动（§3.4）。
- `apps/dae/Cargo.toml` 里 `dae-qt` 作为 build-dependency 的 “unused dependency” warning
  是既有现象（用于 build script 排序），不是迁移引入的。

## 9. 完成定义

- `crates/dae-qt` 内不再有 `#[cxx_qt::bridge]` 的 `RustQt` 块，只有（必要时）
  `extern "C++"` 声明。
- `Cargo.toml` 删除 `cxx-qt` / `cxx-qt-lib` / `cxx-qt-build`；`build.rs` 不再生成
  QML 模块/moc 类型，只编译 C++ 并链接 Qt。
- `main.rs` 使用 `QApp`，启动时间线、占位窗、AOT、elevated 分支、单实例行为不变。
- `cargo test -p dae-qt --lib` 全绿；真实启动无 QML 警告；`DAE_QML_AOT=1` 的 release
  构建不降级（§7.1）。
- 一份 `docs/decisions/engine-qt-quick.md` 的后续决策记录，说明为什么换桥、换来了什么、
  丢了什么（AOT 描述、C++ 类型归属、线程模型）。

## 10. 新对话可直接粘贴的开场提示

```text
在 K:/code/dae 的 feat/qtbridge 分支上继续 Qt Bridge 迁移。先读
docs/decisions/qtbridge-migration.md（执行手册）和 bench/qtbridge-spike/README.md
（已通过的可行性闸门），再读 git log origin/main..HEAD 的三个提交。

从“叶子单例”继续：appearance.rs → network_discovery.rs → preferences.rs。
每个模块严格按手册 §3 的配方改，迁完立刻跑：
  cargo build -p dae-app
  cargo test -p dae-qt --lib
  DAE_STARTUP_PERF=1 启动 target/debug/dae.exe，确认时间线完整且无 QML 报错
每模块一个提交。不要跑 cargo fmt -p dae-qt（会重排无关文件）。
Qt 路径：K:/Qt/6.12.0/msvc2022_64，QMAKE 指向其 bin/qmake.exe。
```

## 11. 参考

- Qt Bridge 文档：<https://doc.qt.io/qtbridge-rust/>（`QApp`、`qobject`、`qproperty!`、
  `QAbstractItemModel`、`QThreadHandle`）
- 上游源码（本地克隆过，可重新 clone）：<https://github.com/qt/qtbridge-rust>
- 现有决策文档：`docs/decisions/engine-qt-quick.md`（为什么当初选 CXX-Qt）
- AOT 细节：`docs/qml-aot.md`
- 闸门与实测输出：`bench/qtbridge-spike/README.md`
