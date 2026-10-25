# 切片八执行手册：把 `file_system` 搬进 `dae-core`

> **这不是决策记录，是施工单。** 决策与来龙去脉在
> [`docs/decisions/engine-qt-quick.md`](decisions/engine-qt-quick.md)；
> 这份文档假定读者（或新会话里的 agent）**没有**参与前面七刀，
> 所以每一步都写了"为什么"和"怎么验证"。
>
> 起点：分支 `refactor/qt-quick`，切片 1–7b 已合并（最新提交见 `git log`）。
> 终点：`dae-core` 是一个能独立 `cargo test` 的文件系统库，
> `src-tauri` 只剩下真正依赖窗口/webview 的壳。

---

## 0. 完成定义

同时满足这五条才算做完：

1. `crates/dae-core` 里包含**全部**文件系统领域逻辑，且
   `crates/dae-core/Cargo.toml` 的依赖里**没有** `tauri` / `tauri-specta` /
   `winit` / `qt*` 这类"知道 UI 存在"的 crate。
   （`notify`、`git2`、`russh` 这些是领域依赖，本来就该在 core。）
2. `src-tauri/src/file_system/` 只剩 §3 表里"留壳"的 7 个文件（外加从
   `sidebar.rs` 拆出来的 `places.rs`、从 `listing.rs` 拆出来的
   `listing_packets.rs`）。
3. 下面四条命令全绿，且**绑定的 diff 为零**：

   ```bash
   cargo check --manifest-path src-tauri/Cargo.toml
   cargo test  --manifest-path crates/dae-core/Cargo.toml
   src-tauri/target/bin/cargo-nextest.exe nextest run --manifest-path src-tauri/Cargo.toml
   bunx tsc --noEmit
   ```

4. `tauri::` 引用数从 **267** 降到 **≤ 180**（大头是 `commands.rs` 的 95 处
   IPC 包装，那部分本来就要留）。`src-tauri/src/file_system/` 下只剩
   `commands.rs` / `preview.rs` / `system_files.rs` / `places.rs` /
   `listing_packets.rs` / `volume_monitor*` 提到 `tauri::`。

   > **事后更正（切片 8 完成时实测）**：这条门槛的前提有误。被搬走的 51 个文件
   > 在切片 3–8 里已经全部接缝化，本来就不含 `tauri::`，所以搬移只能带走 2 处
   > （`prefetch` 的 `AppHandle`），全树停在 **265**。剩余引用里 95 处在
   > `commands.rs`、139 处在壳根模块（随 webview 一起退役）、31 处在
   > `preview` / `system_files` / `places` / `volume_monitor`。真正可验证的门槛是
   > “core 的 `Cargo.toml`/源码里不出现 `tauri`/`winit`/`qt` 依赖”与
   > “`file_system/` 只剩 7+3 个壳文件”，这两条已满足。
5. `docs/decisions/engine-qt-quick.md` 的切片列表追加第 9 条，写下实测数字。

---

## 1. 现状盘点（实测，51 个文件）

| | 数量 | 含义 |
|---|---|---|
| **A. 直接搬** | 36 | 零 `tauri::`、零 `tauri_specta::Event`，`git mv` 即可 |
| **B. 先拆再搬** | 8 | 逻辑引擎无关，但**文件里住着 wire 类型或 `AppHandle` 半边** |
| **C. 留壳** | 7 | 天生窗口/webview 相关 |

### A 组（36，原样搬）

```
archive.rs  cloud.rs  cloud/{accounts,drive,dropbox,oauth,onedrive,provider}.rs
connections.rs  git.rs  known_hosts.rs  local.rs
local/{content_search,directory,ignores,operations,properties,search,windows_directory}.rs
macos_trash.rs  media_meta.rs  open_with.rs  recents.rs  search.rs  service.rs
sftp.rs  smb.rs  spaces.rs  test_support.rs  transfer.rs
trash_view.rs  trash_view/recycle_bin.rs  vfs.rs
entry_codec.rs      ← 见 §4.9：搬过去是**临时**的
error.rs types.rs   ← 现在是 5 / 8 行的垫片，搬完后**删除**
```

### B 组（8，先拆）

| 文件 | 里面留了什么 | 拆法 |
|---|---|---|
| `listing.rs` | `BatchSink::Channel(tauri::ipc::Channel<…>)` | §3.2 |
| `watch.rs` | `DirectoryChanged` 的 `tauri_specta::Event` derive | §3.1 |
| `progress.rs` | `FileOperationProgress` 的 derive | §3.1 |
| `undo.rs` | `UndoRedoChanged` 的 derive | §3.1 |
| `directory_size.rs` | `DirectorySizeProgress` 的 derive | §3.1 |
| `hashing.rs` | `FileHashProgress` 的 derive | §3.1 |
| `sidebar.rs` | `get_system_places` 用 `Manager::path()` 解析七个目录 | §3.3 |
| `prefetch.rs` | `warm_startup_data(app)` 只是为了把 `app` 传下去 | §3.3 |

### C 组（7，留壳，动都不动）

```
commands.rs            73 个 #[tauri::command] 包装（IPC 壳本身）
preview.rs             dae:// 缩略图协议 + 渲染线程（Qt 侧换成 QQuickImageProvider）
system_files.rs        原生拖出（需要窗口 + 主线程）+ 剪贴板 + 取键态
volume_monitor.rs      卷监听线程（app 驱动）
volume_monitor/{linux,macos,windows}.rs
```

> **判据**：一个文件只要"删掉 Tauri 就没有意义"，就留壳。
> `preview.rs` 的 400 行解码/缩放逻辑其实引擎无关，但它只被 `dae://` 协议
> 处理器调用，Qt 侧会整条换掉——**不要为了好看去拆它**。

---

## 2. 目标结构

```text
crates/dae-core/src/
  lib.rs
  error.rs  hashing.rs  host.rs  paths.rs  progress.rs  types.rs   ← 已在
  fs/
    mod.rs                 ← 原 file_system.rs 的模块清单
    archive.rs  cloud.rs  cloud/…  connections.rs  git.rs  known_hosts.rs
    local.rs  local/…  macos_trash.rs  media_meta.rs  open_with.rs
    recents.rs  search.rs  service.rs  sftp.rs  smb.rs  spaces.rs
    transfer.rs  trash_view.rs  trash_view/…  vfs.rs
    entry_codec.rs         ← 临时，见 §4.9
    listing.rs  watch.rs  progress.rs  undo.rs  directory_size.rs  hashing.rs
    sidebar.rs  prefetch.rs
    test_support.rs

src-tauri/src/file_system/
  commands.rs              ← 不变
  preview.rs  system_files.rs  volume_monitor.rs  volume_monitor/…   ← 不变
  places.rs                ← 从 sidebar.rs 拆出（get_system_places）
  listing_packets.rs       ← 从 listing.rs 拆出（Channel 传输）
  ipc_events.rs            ← 新：6 个 wire 结构体（§3.1）
src-tauri/src/file_system.rs
  pub use dae_core::fs::*;      ← 垫片：所有 call site 先不用改
  pub mod commands;  pub mod preview;  pub mod system_files;
  pub mod places;  pub mod listing_packets;  pub mod volume_monitor;
  pub mod ipc_events;
```

**依赖方向**：`src-tauri` → `dae-core`，永不可逆。
`dae-core` 不得出现 `tauri`、`tauri-specta`、`winit`、`qt*`、任何 IPC 格式。

---

## 3. 执行顺序

每一步都以**可编译、测试全绿**结束，单独提交。不要在一步里混两种改动。

### 3.1 第一步：把 6 个 wire 事件结构体搬出 `file_system/`

**为什么**：它们带 `#[derive(tauri_specta::Event)]`。`dae-core` 不能依赖
`tauri-specta`，所以这些结构体必须留在壳里，而**事件名不能变**
（`src/bindings.ts` 里 15 个事件名是前端 API 的一部分）。

**做什么**：

1. 新建 `src-tauri/src/file_system/ipc_events.rs`，把下面六个结构体连同
   `#[tauri_specta(event_name = …)]` 与 `#[serde(rename_all = "camelCase")]`
   原样搬进去（字段一个都别改）：
   - `progress::FileOperationProgress` → `explorer-file-operation-progress`
   - `listing::DirectoryEntriesBatch` → `explorer-directory-entries`
   - `watch::DirectoryChanged` → `explorer-directory-changed`
   - `undo::UndoRedoChanged` → `explorer-undo-redo-changed`
   - `directory_size::DirectorySizeProgress` → `explorer-directory-size-progress`
   - `hashing::FileHashProgress` → `explorer-file-hash-progress`
2. 原文件里改成 `pub use crate::file_system::ipc_events::X;`（垫片，先不动 call site），
   或直接改引用点——二选一，**但同一提交里两种写法不要混用**。
3. 更新 `src-tauri/src/lib.rs` 的 `collect_events![…]` 路径。
4. 更新 `src-tauri/src/host.rs` 里 `TauriHost::emit` 的构造路径。
5. `file_system.rs` 里 `pub use watch::DirectoryChanged;` 指向新位置。

**坑**：`watch.rs` 的 `DirectoryChanged` 有 `names` 字段和一段长文档；
`undo.rs` 的 `UndoRedoChanged` 现在是 `dae_core::host::Event` 的载荷来源。
搬结构体时**只搬结构体**，不要顺手改字段顺序（specta 的 TS 输出会跟着变）。

**验证门**：

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cp src/bindings.ts /tmp/before.ts
cargo run --quiet --manifest-path src-tauri/Cargo.toml \
  --features export-bindings --bin export_bindings
diff /tmp/before.ts src/bindings.ts     # 必须是空
```

### 3.2 第二步：拆 `listing.rs` 的传输通道

**为什么**：`BatchSink` 一半是接缝（`Events` 走 `Host::emit`），一半是
`tauri::ipc::Channel`（列式 packet）。前者属于 core，后者属于壳。

**做什么**：

1. 在 `dae-core::fs::listing` 定义：

   ```rust
   /// 一批已就绪的列举数据，交给壳去送达。
   pub struct Batch {
       pub stream_id: String,
       pub path: String,
       pub rows: Vec<ListingRow>,
       pub done: bool,
   }

   /// 送达方式由壳决定：Tauri 走事件或 packet 通道，Qt 走模型插入。
   /// 返回 false 表示接收端已经走了，发射线程应当停下。
   pub trait BatchTarget: Send + Sync + 'static {
       fn deliver(&self, batch: Batch) -> bool;
   }
   ```

2. core 里实现 `EventsTarget { host: Arc<dyn Host> }`：
   `deliver` 把 `ListingRow` 用 `entry_codec::base_of(path)` 拼成
   `DirectoryEntry`，发 `dae_core::host::Event::DirectoryEntries { … }`，返回 `true`
   （原因见 `listing.rs` 现有注释：`Host::emit` 故意没有失败通道）。
3. `open_streamed_listing` / `spawn_listing_reader` / `spawn_batch_emitter`
   改为收 `Arc<dyn BatchTarget>`。
4. 壳里新建 `src-tauri/src/file_system/listing_packets.rs`：
   `ChannelTarget(Channel<InvokeResponseBody>)`，把 `Batch` 打包成
   `entry_codec::pack_with_header_flags` 的字节，保留 `HEADER_FLAG_FINAL` 语义。
5. `commands.rs` 里 `read_directory` 传 `EventsTarget`，
   `read_directory_packets` 传 `ChannelTarget`。

**验证门**：`nextest` 全绿（`listing::tests::an_events_sink_spells_the_rows_and_hands_the_batch_to_the_seam`
要改成对新 trait 断言，语义不变：**行必须在交给接缝前拼成完整路径**）。

### 3.3 第三步：拆 `sidebar` / `prefetch`

**为什么**：`sidebar::get_system_places` 用 `Manager::path()` 解析
Home/Desktop/Documents/Downloads/Pictures/Music/Videos 七个目录；
`prefetch::warm_startup_data(app)` 只是把 `app` 传给它。

**做什么**：

1. 把 `get_system_places` + 它用到的 `PlaceKind` 搬到
   `src-tauri/src/file_system/places.rs`，保持 `#[tauri::command]` 与
   `app: tauri::AppHandle`；`lib.rs` 里注册路径改为 `file_system::places::get_system_places`。
   **注意**：它内部仍走 `prefetch::state().take_system_places()`——那是 core 的
   单例，调用方向是对的（壳 → core）。
2. `prefetch::warm_startup_data` 改成收数据而不是收壳：

   ```rust
   pub fn warm_startup_data(places: Option<Vec<SystemPlace>>)
   ```
   壳里的调用点先取 `places`（调用 `places::get_system_places(app)` 的内部版本，
   别调命令本身），再 `warm_startup_data(places)`。
3. `sidebar::write_atomic` 被 `recents` 使用 → 搬进 `dae_core::fs::atomic`
   （或 `dae_core::paths`），两边都改成引用它。**这是 recents 能搬走的前提。**

**验证门**：`cargo check` + `nextest`；`lib.rs` 里 `file_system::sidebar::get_system_places`
必须已经不存在。

### 3.4 第四步：建立 `dae-core/src/fs/`，按"叶子 → 主干"分波搬

**为什么分波**：一次搬 36 个文件，编译错误会互相淹没。按依赖深度搬，
每波都能独立编译。

**波次**（每波一个提交）：

| 波 | 文件 | 备注 |
|---|---|---|
| 1 | `error.rs`(删垫片) `types.rs`(删垫片) `vfs.rs` `local.rs` `local/*` | 最底层；`fs/mod.rs` 里 `pub use crate::{error, types};` |
| 2 | `transfer.rs` `archive.rs` `search.rs` `known_hosts.rs` | |
| 3 | `sftp.rs` `smb.rs` `connections.rs` `cloud*` `open_with.rs` `media_meta.rs` | 网络/平台 |
| 4 | `git.rs` `trash_view*` `macos_trash.rs` `spaces.rs` `recents.rs` `connections.rs` | |
| 5 | `service.rs` `undo.rs` `progress.rs` `directory_size.rs` `hashing.rs` `listing.rs` `watch.rs` `sidebar.rs` `prefetch.rs` `test_support.rs` `entry_codec.rs` | B 组在这一波才真正落地 |

**每波的具体动作**：

```bash
git mv src-tauri/src/file_system/<x>.rs crates/dae-core/src/fs/<x>.rs
# 在 src-tauri/src/file_system.rs 加垫片：
#   pub use dae_core::fs::<x>;
```

`fs/mod.rs` 里保留原来的 `pub mod …;` 清单（`test_support` 用
`#[cfg(test)] pub(crate)`）。

**需要改的引用**（全树只有这几类）：

- `crate::file_system::X` → `crate::fs::X`（core 内部），或继续走
  `src-tauri/src/file_system.rs` 的 glob 垫片（壳内部，**先不改**）。
- `crate::file_icons` / `crate::desktop_thumbnails`：**只出现在 `preview.rs`**
  （留壳），所以 core 不需要它——这是这套切分最省事的地方。
- `crate::host::testing::RecordingHost`：出现在 `listing.rs` / `progress.rs` /
  `undo.rs` 的测试里。**把 `RecordingHost` 搬进 `dae_core::host`**
  （`#[cfg(test)]` 或公开均可），壳里 `crate::host::testing` 变成对它的再导出。
  不做这步，搬走的模块测试编译不过。
- `dae_core::paths::config_dir()` 已经就位，`settings.rs` / connections / cloud
  都已在用，无需再动。

**`dae-core/Cargo.toml` 需要加的依赖**（实测哪些模块在用，按需搬运，
`src-tauri` 里保留它自己的）：

```toml
serde  specta  async-trait  tokio  rayon  ignore
notify  sysinfo  dunce  mime_guess  image  base64  rand
sha2  md-5  sha1
zip  tar  flate2  sevenz-rust2
git2  smb2  keyring  trash
russh  russh-sftp
grep-regex  grep-searcher  grep-matcher
reqwest  rustls            # cloud/oauth
quick-xml                  # cloud 的 OOXML 读取（确认后再加）
[target.'cfg(windows)'.dependencies]
windows = { … }            # local/windows_directory, system_files 的一半留壳，按编译错误收敛
[target.'cfg(target_os="linux")'.dependencies]
gio = "0.18"               # open_with::linux
zbus = "5"                 # 如果 sidebar/recents 会用到（当前不用，别急着加）
[target.'cfg(target_os="macos")'.dependencies]
core-foundation  objc2*    # open_with::macos
```

> 建议：先把 `src-tauri/Cargo.toml` 的 `[dependencies]` 整段复制到
> `dae-core`，编译通过后用
> `cargo tree -p dae-core` + 逐个删除来收敛。不要凭空猜。

### 3.5 第五步：收尾

1. 删除 `crates/dae-core/src/fs/{error,types}.rs` 垫片，`fs/mod.rs` 改成
   `pub use crate::{error, types};`。
2. 删掉 `src-tauri/src/file_system.rs` 里所有已无 call site 的垫片
   （编译器会告诉你哪些没人用了）。
3. 跑 §5 的全套验证。
4. 更新 `docs/decisions/engine-qt-quick.md`（切片 9 条目 + 累计数字）。

---

## 4. 已知的坑（都是这几刀里真实踩过的）

1. **`tauri::` 计数会漏 `tauri_specta::Event`。**
   我用 `grep -c 'tauri::'` 当指标，wire 结构体的 derive 不在里面。
   搬文件前必须单独查一遍：`grep -rn 'tauri_specta' src-tauri/src/file_system`。
2. **`cfg(test)` 在 Windows 上也会编译 Linux 模块。**
   `#[cfg(any(target_os = "linux", test))] mod linux;` 是现有写法。
   给这类模块加 `use` 时，要跟着函数一起 `#[cfg(target_os = "linux")]`，
   否则测试构建会报 `unused import`（切片 4 踩过）。
3. **内层模块看不见文件级 `use`。**
   `trash_view::{browse, macos_view}`、`system_files::platform`、
   `volume_monitor/*` 都是内层模块；在文件顶部加 `use` 不会渗进去。
   要么把 `use` 放进内层模块，要么用绝对路径（切片 3/7 踩过）。
4. **同名命令在两个 cfg 模块里各有一份。**
   `trash_view` 的 `list_trash` 等四个命令在两个平台模块里各定义一次，
   包装只能写一份，靠 `pub use` 在对应平台解析（切片 7a 踩过）。
5. **去重脚本会留下孤儿文档注释。**
   包装去重时如果只从 `#[tauri::command]` 开始删，前面的 `///` 会留下，
   specta 会把它们粘到**下一个**命令上（切片 7b 真的发生了，
   `list_cloud_accounts` 一度带着 trash 的文档）。
   **每次动命令或事件，重新生成 `src/bindings.ts` 并 diff，两次生成要幂等。**
6. **`crates/dae-core` 单独 `cargo test` 有自己的 lock。**
   它是 `src-tauri` 的 path 依赖，但自己跑测试时解析自己的依赖版本
   （specta 的 rc 版本会漂，`Type` derive 需要 `features = ["derive"]`）。
   workspace 目前**故意没建**：根 workspace 会挪走 `target/`（73 GB，全量重编译），
   并让 `src-tauri/.cargo/config.toml` 里的 `/STACK:8388608` 失效——那条正是防
   OLE 拖放路径栈溢出的（tauri#9882）。等 `apps/dae` 落地时再一次性搬。
7. **`include_str!` 的相对路径写死了。**
   `crates/dae-core/src/paths.rs` 里用 `../../../src-tauri/tauri.conf.json`。
   如果移动 `paths.rs`，这个路径要重算。
8. **`lib.rs` 的 `collect_commands![…]` 是唯一的注册表。**
   命令从哪个模块移出，就要在这里改路径；漏一条就是运行期 `unknown command`，
   **编译期不报错**（除非是 `__cmd__` 宏不存在——一般在 `collect_commands!` 里
   会报出来，但别赌）。
9. **`entry_codec.rs` 是"一半核心、一半壳"，搬的时候要标注清楚。**
   它 1510 行，实测使用者是 `local/directory.rs`、`local/windows_directory.rs`
   （枚举器产出的行模型 `ListingRow` + `into_entry` + `base_of`）、`listing.rs`、
   `commands.rs`。所以它**不是**纯 JS 代码：`ListingRow` 就是枚举器的行模型。
   真正只服务 webview 的是打包函数（`pack_with_header_flags`、
   `HEADER_FLAG_FINAL`）。搬进 core 时在 `fs/mod.rs` 顶部写明：
   Qt 壳落地时把**打包那半**连同 `listing_packets.rs`、`BatchTarget` 的 Channel
   实现一起删掉，`ListingRow` / `into_entry` / `base_of` 留下。
10. **`crates/dae-core` 绝不能引入 `tauri`/`tauri-specta`/`winit`/`qt*`。**
    这不是洁癖：引入任一个，Qt 壳就得为 Tauri 付编译时间，迁移也就白做了。
    加依赖前问一句"这个 crate 知道 UI 存在吗"。

---

## 5. 验证清单（每步都跑，别攒到最后）

```bash
# 1. 壳能编译（增量约 3–10 s）
cargo check --manifest-path src-tauri/Cargo.toml

# 2. core 能独立测试
cargo test --manifest-path crates/dae-core/Cargo.toml

# 3. 全量 Rust 测试（342→347 个用例，约 30 s）
src-tauri/target/bin/cargo-nextest.exe nextest run --manifest-path src-tauri/Cargo.toml
#   ↑ 没装的话：cargo binstall -y cargo-nextest --install-path src-tauri/target/bin

# 4. 前端契约没变
cp src/bindings.ts /tmp/before.ts
cargo run --quiet --manifest-path src-tauri/Cargo.toml \
  --features export-bindings --bin export_bindings
diff /tmp/before.ts src/bindings.ts      # 必须为空
bunx tsc --noEmit                        # 必须 0

# 5. 跨平台（你们的 workflow 只在 push main / PR 时自动跑）
git push
gh workflow run linux-check.yml --ref <branch>
gh workflow run macos-check.yml --ref <branch>
gh run list --branch <branch> --limit 2
```

> 第 4 步是这套重构的**主要安全网**：45+ 个命令换了注册模块、27 个函数换了
> 签名，前端一行没改、类型仍然全过。任何"看起来没问题"的改动，都靠它抓出来。

---

## 6. 回退

每一步单独提交，且提交后都是可编译、可测试的状态。
出问题的代价是 `git revert` 一个提交，不是整分支回滚。

如果某一波搬完发现模块间循环依赖（不该发生——见下方断言），
把那一波的文件 `git mv` 回去、删掉垫片即可，其他波不受影响。

**结构断言（可以在搬完后写成测试或 CI 检查）**：

- `dae-core` 的 `Cargo.toml` 不含 `tauri*`/`winit`/`qt*`。
- `src-tauri` 里不存在 `use dae_core::*` 之外的反向依赖（即 core 不引用壳）。
- `src-tauri/src/file_system/` 下没有新增 `#[tauri::command]`
  （现有的 73 个都在 `commands.rs`）。

---

## 7. 做完之后解锁什么

1. **`dae-core` 可以脱离 Tauri 跑测试**——列举管线、SFTP、云、归档、
   undo/redo 全部可独立验证，CI 里可以单拉一个 job。
2. **Qt 的四个门槛 spike 可以吃真数据**（`docs/decisions/engine-qt-quick.md` §5）：
   - 12 万行 `QAbstractListModel` + 增量补丁 → 直接喂 `fs::listing` 的输出；
   - 中文 IME、Mica 无边框、单窗口内存增量 —— 与 core 并行，互不阻塞。
3. **`commands.rs` 变成唯一需要为 Tauri 维护的文件**，约 650 行包装。
   Qt 壳出现后，它随 Tauri 壳一起退役。
4. 剩下的 `tauri::` 应该集中在：`commands.rs`、`preview.rs`（`dae://`）、
   `system_files.rs`（拖出）、`places.rs`、`listing_packets.rs`、
   `volume_monitor*`、以及 `src-tauri/` 根下的壳模块
   （`tab_windows` / `window_material` / `deep_link` / `updater` / `terminal` /
   `settings` / `default_manager` / `shell_commands` / `system_*`）——
   这些随 webview 一起消失或重写。

5. **已知未接线的一处**：`volume_monitor` 的 `VolumesChanged` 仍然直接
   `tauri_specta::Event::emit(&VolumesChanged, &app)`，没有走 `dae_core::host`
   （它是唯一一个没进接缝的事件）。Qt 侧落地时补一个 `Event::VolumesChanged`
   变体即可，现在别动——卷监听的三个平台后端都还有 `app`。

---

## 8. 交接清单（新会话开工前读这三样）

1. 本文档；
2. `docs/decisions/engine-qt-quick.md`（为什么是 Qt、为什么是这套切分）；
3. `crates/dae-core/src/host.rs` 的模块文档（接缝的定义与界限）。

开工第一件事：

```bash
git switch refactor/qt-quick && git pull
cargo check --manifest-path src-tauri/Cargo.toml     # 确认起点是绿的
```

然后从 §3.1 开始，一步一提交。
