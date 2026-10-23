# PGO 与 BOLT：编译期和后链接的两种优化

> **状态：已接入，未测量收益。** 本地流程（Windows / Linux）与 `release.yml` 的三条腿
> （Windows / Linux / macOS）都已接上，Linux 额外带 LLVM BOLT；每一步都有降级路径；但**还没有一次干净的 A/B 数字**说明它把启动或列目录加快了多少。
> 按 `docs/decisions/` 的习惯，这里只写「怎么跑、为什么这么接、失败时退到哪」，
> 收益等有数了再回填——没有数字之前，不要在任何地方宣称它更快。

改动只有两处是产品代码/配置，其余都是脚本：

| 文件 | 作用 |
|---|---|
| `scripts/opt/lib.mjs` | rustflags 合并、LLVM 工具查找、训练负载 |
| `scripts/opt/pgo.mjs` | 分阶段 PGO（instrument / merge / build） |
| `scripts/opt/bolt.mjs` | BOLT（instrument / collect / merge / optimize） |
| `scripts/opt/post-link.mjs` | `beforeBundleCommand` 钩子，BOLT 的接线点 |
| `src-tauri/tauri.perf.conf.json` | 只被 CI 的 Linux 腿 `--config` 加进去 |
| `src-tauri/src/lib.rs` | `spawn_training_quit`：唯一一处产品代码改动（见 §3） |
| `.github/workflows/release.yml` | Linux 腿的新步骤：装 xvfb 与 `llvm-bolt`、训练、导出 flag，以及把 perf overlay 加进 `--config` |

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
`bun run perf:pgo:status` 会列出版本、产物大小、以及目标二进制能不能被 BOLT 处理。

## 3. 本地：分阶段手动跑

PGO 的效果**完全取决于训练期间你怎么用这个 app**。所以本地流程刻意拆成三步，
中间那步由人来跑（CI 里没有人，才退化成脚本负载，见 §5）。

```bash
bun run perf:pgo:instrument      # ① 插桩构建，输出 .perf/pgo/raw/ 的空目录
                                 #    自己去用那个 exe：打开真正慢的目录、搜索、开终端、切视图
bun run perf:pgo:merge           # ② llvm-profdata merge → .perf/pgo/dae.profdata
bun run perf:pgo:build           # ③ -Cprofile-use 重新编译
```

脚本构建时**默认带上 `--config src-tauri/tauri.release.conf.json`**（可用
`DAE_TAURI_BUILD_ARGS` 改）。这不是为了省事：release 构建如果没合并那份 overlay，
`plugins.updater` 是 `null`，updater 插件在初始化时就 panic——
`PluginInitialization("updater", "invalid type: null, expected struct Config")`，
app 根本起不来，也就采不到任何 profile。本地已经因为这个原因跑空过一次。

训练跑出来的 stderr 写在 `.perf/training.log`：`windows_subsystem = "windows"` 的构建
本身没有控制台，进程提前退出时，那段日志是唯一能看出为什么的地方。

要接着上 BOLT，第 ③ 步加 `--bolt`，它会额外做两件事：链接时加 `-Wl,--emit-relocs`、
用 `CARGO_PROFILE_RELEASE_STRIP=false` 保留符号表——这两样正是 BOLT 的输入要求，
而 `[profile.release]` 的 `strip = true` 恰好会把它们删掉：

```bash
bun run perf:pgo:build -- --bolt   # 或者 bun scripts/opt/pgo.mjs build --bolt
bun run perf:bolt                  # instrument → 跑训练负载 → merge-fdata → 重写 → strip
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

所以唯一可行的是**进程从 CRT 的 exit 出去**。唯一一处产品代码改动就是为此：
`src-tauri/src/lib.rs` 的 `spawn_training_quit`——`DAE_PERF_TRAIN_QUIT_MS` 设置时，
后台线程睡 N 毫秒后调 `exit(0)`（CRT 的那个，不是 `std::process::exit`）。
没设置时（即所有正常启动）它什么都不做。这个差别不是理论：同一个 instrumented
小程序，用 CRT `exit` 退出会写出 7.8 KB 的 profraw，用 `std::process::exit` 退出什么都没有。

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

**三条腿都跑标准两阶段 PGO**，Linux 额外多一层 BOLT：

1. `bun scripts/opt/pgo.mjs train --seconds 45 --target <triple>`：插桩构建 → 启动 app、
   打开一个生成的语料目录、跑 45 秒（LINUX 无 DISPLAY 时套 `xvfb-run`）→ merge。
   这一步 `continue-on-error`。`--target` 在 Windows/Linux 是空串，在 macOS 是
   `aarch64-apple-darwin`：它既传给 `tauri build`，也决定去 `target/<triple>/release/` 找 exe。
2. `bun scripts/opt/pgo.mjs flags <matrix.perf_flags> >> $GITHUB_ENV`：把
   `CARGO_ENCODED_RUSTFLAGS`（含 `-Cprofile-use`）交给后面的构建；只有 Linux 的
   `perf_flags` 是 `--bolt`，会额外带 `-C link-arg=-Wl,--emit-relocs` 和
   `CARGO_PROFILE_RELEASE_STRIP=false`——这两样是 GNU ld / ELF 专属，不能上另外两条腿。
3. `tauri-action` 于是用**这套 flag** 编出真正的产物（Windows/MSIX 与 macOS 的 dmg/app
   都在其中）。Linux 腿的 `--config src-tauri/tauri.perf.conf.json` 让 `beforeBundleCommand`
   在**链接之后、打包之前**跑 `post-link.mjs`，对 `target/release/dae` 执行 BOLT 并 strip。
   打包（deb/rpm/AppImage）与 updater 签名都在其后，所以安装包里的、`latest.json` 签名
   指向的，都是被 BOLT 重写过的那个二进制。
4. `beforeBundleCommand` 是唯一能插进「已经链接、还没打包」这个窗口的钩子；`beforeBuildCommand`
   不行，它在编译之前。

代价：每条腿多一次全量插桩构建（不论平台）+ 45 秒训练。实测本机 fat LTO 单次链接约 8 分钟，
所以单腿 CI 时间大致 +40~60%；`llvm-tools` 是 `dtolnay/rust-toolchain` 装的，不靠源。

**降级语义**（重要，别把它当成失败）：

| 情况 | 结果 |
|---|---|
| 训练跑不起来（runner 没有可用的 GUI 会话 / 没有 WebKit） | 没有 profdata → `flags` 不导出 `-Cprofile-use`，产物是普通 release 构建 |
| 训练失败（macOS/Windows 都算） | 同上；Linux 还会保留 BOLT 的链接 flag（只是没有 PGO） |
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
- **`DAE_PERF_TRAIN_QUIT_MS` 是唯一的产品代码改动**（`lib.rs::spawn_training_quit`，~25 行含注释）。
  不设就走原路径，不注册任何东西；如果不想要它，删掉那个函数和调用点即可，
  代价是 CI 里的训练步骤以后采不到 profile（本地手动训练也采不到，因为人也没法把它“干净”关掉）。

## 7. 怎么确认它真的生效了

- `perf:pgo:merge` 结尾会打印 `llvm-profdata show` 的摘要：`Total functions` 应该在几万这个量级、
  `Total count` 不为 0。为 0 就说明训练没跑出东西（本地实测：一次 24 秒的启动+列目录 =
  63,858 个函数、12,547,845 次计数、20 MiB profdata）。
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
