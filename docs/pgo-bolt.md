# PGO 与 BOLT：编译期和后链接的两种优化

> **状态：已接入，未测量收益。** 两个壳都接上了：`tauri`（旧 WebView 壳）与
> `qt`（`apps/dae`，cxx-qt 桥）。本地流程（Windows / Linux）与 `release.yml` 的三条腿
> （Windows / Linux / macOS）都覆盖，Linux 额外带 LLVM BOLT；每一步都有降级路径；但**还没有一次干净的 A/B 数字**说明它把启动或列目录加快了多少。
> 按 `docs/decisions/` 的习惯，这里只写「怎么跑、为什么这么接、失败时退到哪」，
> 收益等有数了再回填——没有数字之前，不要在任何地方宣称它更快。
>
> 选择哪个壳：`--qt`（或 `DAE_PERF_SHELL=qt`）。不带参数时仍是 Tauri，直到旧壳退役。

改动只有少数几处是产品代码/配置，其余都是脚本：

| 文件 | 作用 |
|---|---|
| `scripts/opt/lib.mjs` | 壳选择、rustflags 合并、LLVM 工具查找、训练负载 |
| `scripts/opt/pgo.mjs` | 分阶段 PGO（instrument / merge / build） |
| `scripts/opt/bolt.mjs` | BOLT（instrument / collect / merge / optimize） |
| `scripts/opt/post-link.mjs` | `beforeBundleCommand` 钩子，BOLT 的接线点 |
| `src-tauri/tauri.perf.conf.json` | 只被 CI 的 Tauri Linux 腿 `--config` 加进去 |
| `src-tauri/src/lib.rs` | Tauri 的训练退出钩子（见 §3） |
| `apps/dae/Cargo.toml`、`apps/dae/src/main.rs` | `perf-train` feature 与 Qt 的训练退出钩子（见 §3） |
| `apps/dae/build.rs`、`crates/dae-qt/build.rs` | cxx-qt C++ 半的 profile flag（见 §2.1） |
| `.github/workflows/release.yml` | 两条腿都训练/导出 flag；Linux 的 BOLT 与 `tauri.perf.conf.json` |
| `.github/workflows/perf-check.yml` | 手动检查 `--qt` 的训练链路 |

`[profile.release]` 本身没改：`strip = true` 是发货想要的，BOLT 构建通过
`CARGO_PROFILE_RELEASE_STRIP=false` 从环境覆盖它，因为 `tauri build` 只会编 `release`
这个 profile，没有 `--profile` 可选。

## 1. 为什么是这两种

- **PGO（IR 插桩）** 让编译器知道「哪个分支常走、哪个调用点是单态的」。rustc 的接口是
  `-Cprofile-generate` / `-Cprofile-use`，profile 是 LLVM 的 IR 级插桩数据，所以叫
  「LLVM IR PGO」，与 Clang 的 `-fprofile-generate` 是同一套 runtime（`compiler-rt`）。
- **BOLT** 在**链接之后**重写已经固定的机器码：按 profile 重排基本块、把冷函数拆到远处、
  做 ICF。它拿到的信息是 PGO 拿不到的——链接器把各 crate 的 `.text` 按什么顺序拼起来、
  fat LTO 之后内联成了什么样。两者不重复，叠加才合理。

### 顺带回答「CSPGO」

如果指的是 Clang 的 `-fcs-profile-generate`（context-sensitive PGO），**rustc 没有这个开关**，
不是「没接」，是上游没有暴露：rustc 只转译 `-Cprofile-generate/-Cprofile-use` 两个选项，
没有第二遍 CSPGO 的入口。要 CSPGO 只能等 rustc 支持，或者把热点 crate 改成 C/C++ 并用
Clang 编——那不属于这个项目。所以这里做的是「LLVM IR 插桩 PGO」，不是 CSPGO。

## 2. 前置条件

```bash
rustup component add llvm-tools-preview   # llvm-profdata：必须和 rustc 同版本 LLVM
```

`llvm-profdata` **优先从 rustc 的 sysroot 里取**（`$(rustc --print sysroot)/lib/rustlib/<host>/bin`），
而不是 PATH 上的随便一个 LLVM：profraw 是带版本的格式，版本不匹配的后果不是立刻报错，
而是在两小时后的 `-Cprofile-use` 阶段才发现 profile 读不了。找不到时会退到 PATH 与
`/usr/lib/llvm-*/bin`，并**警告版本不一致**而不是静默继续。

BOLT 不在 rustup 里，要单独装（ELF / Linux 才有）：

```bash
sudo apt install llvm-bolt        # Ubuntu 25.04+、Debian 13+、Fedora 42+
# 或者 LLVM 官方仓库 / 发行版的 bolt-NN 包，二进制在 /usr/lib/llvm-NN/bin/llvm-bolt
```

工具路径可以用 `DAE_LLVM_PROFDATA`、`DAE_LLVM_BOLT`、`DAE_MERGE_FDATA`、`DAE_LLVM_STRIP` 覆盖。
`bun run perf:pgo:status`（或 `perf:pgo:qt:status`）会列出版本、产物大小、以及目标二进制
能不能被 BOLT 处理。

### 2.1 C++ 半：cxx-qt 桥里能被优化的只有这一小块

Qt 本体是预编译 DLL/dylib，PGO 动不到；启动路径上属于本项目的 C++ 只有 cxx-qt 生成的
胶水与 `crates/dae-qt/cpp/` 的桥接文件。它们在 `apps/dae/build.rs` 与
`crates/dae-qt/build.rs` 里通过 `cc` 编译，所以 PGO 脚本把两个环境变量交给构建脚本：

| 变量 | 阶段 | flag |
|---|---|---|
| `DAE_PERF_CXX_PROFILE_GENERATE` | 插桩 | `-fprofile-instr-generate=<raw>/cxx-%m-%p.profraw` |
| `DAE_PERF_CXX_PROFILE_USE` | 使用 | `-fprofile-instr-use=<profdata>` |

**只有 clang 能参与**，而且它的 LLVM 主版本必须和 rustc 的一致——profraw 是带版本的，
MSVC / GCC 的 profile 数据 `llvm-profdata` 根本无法合并。构建脚本检测到不是 clang 或
版本不一致时会打一条 `cargo:warning` 并跳过 C++ 半，**Rust 半照常**。因此：

- macOS（Qt 用的就是 clang，且版本与 rustc 一致时）：C++ 桥也进 profile；
- Windows（MSVC）、Linux（GCC）：C++ 桥不进 profile，只有 Rust 侧被优化；
- 想覆盖 C++ 半，就在 Linux 上用与 rustc 同版本的 clang 构建（`CXX=clang++`）。

这不是「少接了一半」，而是「另一半的工具链不支持同一份 profile」。Qt 本体无论哪种
情况都不会被优化。

## 3. 本地：分阶段手动跑

PGO 的效果**完全取决于训练期间你怎么用这个 app**。所以本地流程刻意拆成三步，
中间那步由人来跑（CI 里没有人，才退化成脚本负载，见 §5）。

```bash
# 旧 WebView 壳（默认）
bun run perf:pgo:instrument      # ① 插桩构建，输出 .perf/pgo/raw/ 的空目录
                                 #    自己去用那个 exe：打开真正慢的目录、搜索、开终端、切视图
bun run perf:pgo:merge           # ② llvm-profdata merge → .perf/pgo/dae.profdata
bun run perf:pgo:build           # ③ -Cprofile-use 重新编译

# Qt 壳（apps/dae，产物在 target/release/dae）
bun run perf:pgo:qt:instrument
bun run perf:pgo:qt:merge
bun run perf:pgo:qt:build
```

`--qt` 走的是 `cargo build --release -p dae-app`，不经过 Tauri CLI，也没有 bundler；
`--target TRIPLE` 会原样交给 cargo，并从 `target/<triple>/release/` 找 exe。两个壳共用
`.perf/pgo/` 的产物——同时训练两个壳会互相覆盖，想并行就把 `DAE_PERF_DIR` 指到不同目录。

Qt 壳启动时还需要 Qt 的运行时 DLL：本机把 `K:/Qt/<ver>/msvc2022_64/bin` 放进 `PATH`
（CI 由 `install-qt-action` 做）。只设 `QMAKE` 够编译，不够运行；缺 DLL 时进程会在
`logging::init` 之前退出，`training.log` 是空的，训练脚本只会报「没有 profraw」。

Tauri 脚本构建时**默认带上 `--config src-tauri/tauri.release.conf.json`**（可用
`DAE_TAURI_BUILD_ARGS` 改）。这不是为了省事：release 构建如果没合并那份 overlay，
`plugins.updater` 是 `null`，updater 插件在初始化时就 panic——
`PluginInitialization("updater", "invalid type: null, expected struct Config")`，
app 根本起不来，也就采不到任何 profile。本地已经因为这个原因跑空过一次。Qt 壳没有这份
overlay，`--qt` 不需要它。

训练跑出来的 stderr 写在 `.perf/training.log`：两个壳的 release 构建在 Windows 上都是
`windows_subsystem = "windows"`，本身没有控制台，进程提前退出时，那段日志是唯一能看出
为什么的地方。

要接着上 BOLT，第 ③ 步加 `--bolt`，它会额外做两件事：链接时加 `-Wl,--emit-relocs`、
用 `CARGO_PROFILE_RELEASE_STRIP=false` 保留符号表——这两样正是 BOLT 的输入要求，
而 `[profile.release]` 的 `strip = true` 恰好会把它们删掉：

```bash
bun run perf:pgo:build -- --bolt   # 或者 bun scripts/opt/pgo.mjs build --bolt
bun run perf:bolt                  # instrument → 跑训练负载 → merge-fdata → 重写 → strip
bun run perf:pgo:qt:build:bolt     # Qt 壳；随后 post-link.mjs --qt（release.yml 的 Linux 腿）
bun run perf:bolt:qt
```

`perf:bolt` 默认把结果留在 `.perf/bolt/dae.bolt`（原 exe 不动），确认没问题后
`--in-place` 覆盖，或者手动拷过去。`perf:bolt status` 会先检查 `.symtab` 与 `.rela.text`
是否存在——缺任何一样都直接说明该用什么 flag 重编，而不是丢一个 BOLT 的内部报错。

### 训练负载是怎么被采集的

这一节是**实测出来的**，不是设计出来的。PGO 的计数器在 CRT 的 `atexit` /
`.fini_array` 钩子里写出，BOLT 的插桩 runtime 同理。所以“怎么让进程结束”就成了关键，
而下面每一条都试过、都不行：

| 结束方式 | 结果 |
|---|---|
| `kill` / `TerminateProcess` | 钩子一个不跑，无 profile |
| 关主窗口 | 进程根本不结束：预热标签池里的隐藏窗口还活着（`docs/decisions/tab-tear-off-warm-pool.md`） |
| `AppHandle::exit(0)`（也即 Tauri 事件循环结束时走的路） | Windows 上走到 `ExitProcess`，**跳过 CRT 的 atexit**，无 profile。已实测：exit code 0、干净退出、一个 profraw 都没有 |
| compiler-rt 的 continuous mode（`LLVM_PROFILE_FILE=…%c.profraw`） | 文件停在 20 MB 的 “truncated profile data”，`llvm-profdata merge` 直接拒收 |

所以唯一可行的是**进程从 CRT 的 exit 出去**。两个壳各有一个同构的钩子，都只在
`--features perf-train` 的构建里存在：`src-tauri/src/lib.rs` 的 `spawn_training_quit`
与 `apps/dae/src/main.rs` 的 `perf_train::arm`。`DAE_PERF_TRAIN_QUIT_MS` 设置时，
后台线程睡 N 毫秒后调 `exit(0)`（CRT 的那个，不是 `std::process::exit`）。
没设置时（即所有正常启动）它什么都不做。这个差别不是理论：同一个 instrumented
小程序，用 CRT `exit` 退出会写出 7.8 KB 的 profraw，用 `std::process::exit` 退出什么都没有。

Qt 壳比 Tauri 壳更进一步：计数器先由 `__llvm_profile_write_file` 落盘，然后
Unix 用 `_exit`、Windows 用 `TerminateProcess` 直接结束——都不跑 Qt / 平台插件的
teardown。原因是实测：Windows 上 `_exit` 仍会走到 `ExitProcess` 的 DLL detach，
Qt 的 D3D11 后端在那里多花 **约 20 s**（并打一条 `QDxgiVSyncService not destroyed in
time`），而 harness 的宽限期只有训练时长 +20 s；xvfb 下则可能永远不结束。C++ 半的计数器
也由同一个 compiler-rt runtime 写出（见 §2.1），所以 `perf-train` 只打开一处就覆盖两者。

**Tauri 壳最初的 Linux `exit` 不返回。** CI 里 Windows / macOS 都拿到了 profile（20 MiB /
14.5 MiB），Linux 每次都是：训练进程活过自己的定时器 → harness 在 45+20 秒后 SIGTERM →
`.perf/pgo/raw` 一个 profraw 都没有。`xvfb-run` 下的 `exit` 卡在退出路径上（GTK/WebKit 的
teardown 与 `atexit` 钩子），进程既没活着完成也没退出。v0.5.0 的 Linux 产物就是这样发出去的：
BOLT 有（它的 runtime 每 5 秒自己 dump，不依赖退出），PGO 没有。

现在的做法是不再依赖退出路径：

- `perf-train` 这个 feature 只给 `-Cprofile-generate` 的构建开。它让
  `spawn_training_quit` 直接调 compiler-rt 的 `__llvm_profile_write_file()`——也就是它自己
  `atexit` 钩子调的那个函数——把计数器先落盘，然后走 `_exit`（**不进入任何 exit 钩子**，
  因此绕开卡住的那段 teardown）。
- Windows / macOS 仍是 `exit(0)`：那条路本来就通，不动。
- 没开这个 feature 的构建根本引用不到那个符号，所以它不会误开——反过来，给非插桩构建
  开 `perf-train` 会**链接失败**，这正是想要的约束。
- 应用还会往 stderr 打两行：`[perf] training quit armed: …ms`（钩子挂上了）和
  `[perf] profile counters flushed (status N)`（计数器落盘了）。两行都没有，说明应用
  根本没走到 setup——那是另一类问题，不是退出路径的问题。harness 在失败时会把
  `.perf/training.log` 的尾巴打出来。

BOLT 那一侧还叠了一层保险：插桩时加 `-instrumentation-sleep-time=5`
`-instrumentation-no-counters-clear`，runtime 会 fork 一个 watcher 每 5 秒把累计计数器
dump 一次，所以即使训练进程被意外杀掉，也能拿到最后一份数据。PGO 没有等价机制，
只能靠干净退出。

## 4. BOLT 只做 Linux，这不是没接

BOLT 只认 **ELF**（x86-64 / AArch64）。Windows 是 PE/COFF、macOS 是 Mach-O，两者都没有
等价的「链接后重排」工具，所以：

- `bolt.mjs` 在非 ELF 上直接说明原因并退出；
- `release.yml` 只给 Linux 腿加 `src-tauri/tauri.perf.conf.json`，Windows 与 macOS 的产物
  不经过任何后链接步骤。

## 5. CI 是怎么接的

`release.yml` 的 **`build`（Tauri）与 `qt` 两条矩阵都跑标准两阶段 PGO**，Linux 额外多一层
BOLT：

1. `bun scripts/opt/pgo.mjs train --seconds 45 --target <triple> [--qt]`：插桩构建 → 启动 app、
   打开一个生成的语料目录、跑 45 秒（Linux 无 DISPLAY 时套 `xvfb-run`）→ merge。
   这一步 `continue-on-error`。`--target` 在 Windows/Linux 是空串，在 macOS 是
   `aarch64-apple-darwin`：它既传给构建命令，也决定去 `target/<triple>/release/` 找 exe。
   `--qt` 使这一步走 `cargo build -p dae-app`，并把 C++ bridge 的 profile 变量交给
   构建脚本（§2.1）。
2. `bun scripts/opt/pgo.mjs flags [--qt] <matrix.perf_flags> >> $GITHUB_ENV`：把
   `CARGO_ENCODED_RUSTFLAGS`（含 `-Cprofile-use`）交给后面的构建；`--qt` 还会导出
   `DAE_PERF_CXX_PROFILE_USE`。只有 Linux 的 `perf_flags` 是 `--bolt`，会额外带
   `-C link-arg=-Wl,--emit-relocs` 和 `CARGO_PROFILE_RELEASE_STRIP=false`——这两样是
   GNU ld / ELF 专属，不能上另外两条腿。
3. 真正的产物用**这套 flag** 编出来。Tauri 腿由 `tauri-action` 构建，Linux 的
   `--config src-tauri/tauri.perf.conf.json` 让 `beforeBundleCommand` 在**链接之后、打包之前**
   跑 `post-link.mjs`。Qt 腿则是 `cargo build --release -p dae-app` 之后直接跑
   `post-link.mjs --qt`。Linux 腿因此在打包（deb/rpm/AppImage、portable zip、MSIX、
   `latest-qt.json` 指向的 zip）之前完成 BOLT 与 strip；Windows/macOS 没有 BOLT，
   符号剥离由 release profile 自己完成。
4. `beforeBundleCommand` 是唯一能插进「已经链接、还没打包」这个窗口的 Tauri 钩子；
   Qt 没有这个钩子，所以 release 步骤自己按同样的顺序调用 `post-link.mjs`。

代价：每条腿多一次全量插桩构建（不论平台）+ 45 秒训练。实测本机 fat LTO 单次链接约 8 分钟，
所以单腿 CI 时间大致 +40~60%；`llvm-tools` 是 `dtolnay/rust-toolchain` 装的，不靠源。
Qt 腿另有 `windeployqt` / `macdeployqt` / `linuxdeploy` 的时间，那本来就在。

**降级语义**（重要，别把它当成失败）：

| 情况 | 结果 |
|---|---|
| 训练跑不起来（runner 没有可用的 GUI 会话 / 缺 WebKit 或 Qt 平台插件） | 没有 profdata → `flags` 不导出 `-Cprofile-use`，产物是普通 release 构建 |
| 训练失败（macOS/Windows 都算） | 同上；Linux 还会保留 BOLT 的链接 flag（只是没有 PGO） |
| Qt 腿的 C++ 编译器不是 clang，或 LLVM 主版本与 rustc 不同 | 构建脚本打 `cargo:warning`，C++ 半不进 profile；Rust 半照常 |
| `llvm-bolt` 没装 | `post-link` 打印原因、strip 之后继续打包 |
| 二进制没有 `.rela.text` | 同上：跳过 BOLT，不跳过打包 |
| BOLT 中途失败 | 打警告、还原原始二进制、继续打包（`DAE_PERF_STRICT=1` 可以让它硬失败） |
| `DAE_PERF_SKIP_BOLT=1` | 显式跳过 BOLT，只 strip |

一句话：**优化器坏了不能拦住发版**。真正要留意的信号是日志里的 `BOLT skipped:`、
`::warning title=BOLT unavailable::`、`PGO — instrument and train` 那一步是不是红的，
以及 `-dyno-stats` 那段数字。

## 6. 几个已经踩过/会踩的坑

- **`RUSTFLAGS` 会把 `.cargo/config.toml` 的 rustflags 整个换掉。**
  `src-tauri/.cargo/config.toml` 里给 Windows 加的那条 `/STACK:8388608` 就是为了躲
  `STATUS_STACK_OVERFLOW`，如果 PGO 构建直接 `export RUSTFLAGS=…`，它会被**悄悄丢掉**。
  所以脚本先把 config 里的数组读出来、追加自己的 flag，再以 `CARGO_ENCODED_RUSTFLAGS`
  交给 cargo（这是唯一不会在含空格的路径上被按空白切开的形态）。
- **`beforeBuildCommand` 会污染 profile。** 它先跑 `bindings:export`，也就是**再编一个带
  插桩的二进制并运行它**；那些 profraw 描述的是构建工具，不是 app。脚本在构建完成后
  清空一次 `raw/`，所以目录里留下的只可能是训练那次。（实测一次构建就留下 70 个。）
- **release 构建必须带 `tauri.release.conf.json`。** 不带时 `plugins.updater` 是 `null`，
  updater 插件在初始化时 panic，app 连窗口都出不来。脚本已默认带上这份 overlay。
- **`lto = "fat"` + `--emit-relocs`。** 一般可用；如果 `llvm-bolt` 报错说找不到重定位目标，
  改用 lld 链接（`-C link-arg=-fuse-ld=lld`，CI 需要装 `lld`），本仓库暂未这样做。
- **BOLT 版本不等于 rustc 的 LLVM 版本**（Ubuntu 26.04 上 `llvm-bolt` 是 21，rustc 是 23）。
  BOLT 读的是 ELF，不要求版本一致，只是新版本对个别段的处理更全；失败就降级。
- **`.perf/` 是产物，不要提交。** 已在 `.gitignore` 里。
- **CI 的训练负载只是「启动 + 列一个目录」。** 它一定不如人练得好。正式发布前如果在意收益，
  应该用本地训练出来的 `.profdata`（可以传进 CI 复用，但 BOLT 的 fdata 必须对应 CI 自己链接
  出来的那个二进制，不能跨机构复用）。
- **`DAE_PERF_TRAIN_QUIT_MS` 是唯一的产品代码改动**，两个壳各一份同构实现
  （`src-tauri/src/lib.rs::spawn_training_quit`、`apps/dae/src/main.rs::perf_train`，
  含注释各约 60 行）。不设就走原路径，不注册任何东西；如果不想要它，删掉那个函数和调用点、
  以及对应 `Cargo.toml` 里的 `perf-train` feature 即可，代价是 CI 里的训练步骤以后采不到
  profile（本地手动训练也采不到，因为人也没法把它“干净”关掉）。
- **Qt 的训练会经过单实例锁。** `activation::claim()` 发现已有 dae 在跑时会让本次进程直接
  退出，训练因此得到空 profile。CI 是干净会话所以不成问题；本地训练前先关掉正在运行的实例。
- **Qt 的 C++ 半没有独立的 `llvm-profdata merge`。** clang 的 profraw 与 rustc 的在同一目录，
  一次 `merge` 同时收下；C++ 若被跳过（§2.1），profile 只是少了那些函数，不影响 Rust。
- **`perf-train` 只能开在插桩构建上。** 它引用的 `__llvm_profile_write_file` 由
  `-Cprofile-generate` 带进来的 compiler-rt 提供；给别的构建开它会链接失败。`pgo.mjs` 只在
  `instrument` / `train` 里传，`build` 不传——那边是 `-Cprofile-use`，没有那个运行时。

## 7. 怎么确认它真的生效了

- **`.github/workflows/perf-check.yml`（手动触发，只跑 Linux）**：它会用 `--qt` 完整跑一遍
  插桩 → 训练 → merge，并把 `.perf/training.log` 和 `.perf/pgo/` 传成 artifact。
  release 腿那一步是 `continue-on-error`，空的 profile 与成功的发布长得一模一样；
  这个 workflow 是专门让它可以变红的地方。改训练链路前后都应跑一次。
- `perf:pgo:qt:status` 第一行会写明当前选的是哪个壳，`binary:` 则直接给出 Qt 的
  `target/release/dae`（Tauri 是 `src-tauri/target/release/dae`）。选错壳时先看这两行。
- `perf:pgo:merge` 结尾会打印 `llvm-profdata show` 的摘要：`Total functions` 应该在几万这个量级、
  `Total count` 不为 0。为 0 就说明训练没跑出东西（本地实测：一次 24 秒的启动+列目录 =
  63,858 个函数、12,547,845 次计数、20 MiB profdata）。
- `.perf/training.log` 里那两行 `[perf] training quit armed` / `[perf] profile counters
  flushed` 是训练链路的两个断点：前者说明进程起来了、定时器挂上了（带 epoch 毫秒，可以拿去
  对 job log 的时间戳），后者说明计数器落了盘。只有前者 → 要么退出路径没走完，要么
  `perf-train` 没编进去（`grep -a "profile counters flushed" target/release/dae` 一秒判定）；
  两者都没有 → 应用没启动起来（缺 WebKit/Qt 平台插件、没显示、配置不对）。
- **定时器从进程启动算起，不是从 `setup` 算起。** harness 的宽限期也是从 spawn 算的，
  两边必须是同一个起点。两个壳都在 `main` 的第一行挂定时器，而不是等窗口建好；xvfb 下
  （软件 GL、font cache 冷启动）建窗这一段可能就有几十秒，晚挂的计时点会落到 harness 的
  65 秒之后——这正是 Linux 那条腿之前一直"超时"的形态。
- **Qt 训练日志里没有 WebKit 的噪声，但可能多一行单实例锁。** 如果看到进程立刻以 0 退出，
  先按 §6 关掉正在运行的 dae，再重跑训练。
- `-Cprofile-use` 本身**不会**告诉你“匹配了多少”，除非加 `-Cllvm-args=-pgo-warn-missing-function`。
  默认不加：这个 crate 上它会吐 **77,886 行** "no profile data available for function …"（build script、
  没被训练覆盖的 crate 都在里面），把有用的信号埋掉。调试某份 profile 时用
  `DAE_PGO_WARN_MISSING=1` 再打开。
- BOLT 的输出里有 `BOLT-INFO: … reordered`、`BOLT-INFO: ICF folded N functions`，
  以及 `-dyno-stats` 前后两列动态指令数。
- 二进制大小与 `.text` 布局会变；`perf:bolt status` 里 `optimized` 一项不应为 missing。

## 8. 还没做

- **没有 A/B 数字。** 冷启动、目录列举、搜索各自能拿多少，目前未知；要量的话用
  `bench/` 里的 harness（`frontend-scans`、`listing-pipeline`）配合两个二进制对比。
- **没有把本地训练的 profdata 接进 CI。** 现在 CI 自己练，覆盖率低但流程闭环。
- **没有 macOS/Windows 的等价后链接步骤**（BOLT 只认 ELF，不存在等价物）；PGO 三平台都已开。
- **没有 sign 之后的验证**：`latest.json` 的签名是否始终指向 BOLT 后的产物，需要一次真实
  的 tag 发布来确认（这条是 CI 改动里唯一没被本地验证过的部分）。
