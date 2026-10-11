# 存档：GUI 栈被上游锁在 gtk 0.18（**gtk / gio 本轮都不升**）

> ## ⛔ 决定（2026-10-11，最终）：**不升级**，`gtk` 0.18 / `gio` 0.18 保持不动
>
> 1. **`gtk` 0.19 升不上去**：不是类型不匹配，而是 Cargo 在**依赖解析阶段**就拒绝 ——
>    `gdk-sys` 的 `links = "gdk-3"` 与 vendored `drag` 依赖的 `gdk 0.18` 冲突（§1.1 逐字报错，exit 101）。
>    这个规则没有 workaround：「同一张依赖图里只能有一个包声明相同 `links` 值」。
> 2. **上游没得等**：`tao` / `wry` / `tauri` / `muda` **全部**仍要求 `gtk ^0.18`，
>    且四个都已是各自最新稳定版（§1.2）。就算把 vendored `drag` 也改到 gdk 0.19，
>    冲突只会转移到 tao/wry 的 gdk 0.18 上。
> 3. **代码层还要求版本一致**：`webview.inner().upcast::<gtk::Widget>()` 把 wry/tao 的 widget
>    转成我们的 `gtk::Widget`，两边必须是同一个 gtk 版本（§1.3）。
> 4. **`gio` 0.22 技术上可单独升**（实测编译 + 296 单测全过，§2.1），
>    但会把 glib/gio 拆成**两份 FFI 绑定**，且我们只用到 0.18 就有的基础 API ——
>    收益为零、代价实打实，故一并放弃（§2.3）。
>
> **因此**：等 `tao`/`wry` 转到 gtk 0.19 后，**`gio` 与 `gtk` 一起**再升（§3）。
> 本文档的所有结论均来自本机实测（Arch / rustc 1.99.0 / 系统 GTK 3.24.52），
> 复现命令见 §4。**再有人想试 gtk 0.19，先读 §1.1，不用重跑。**

## 0. 结论先行

| 目标 | 结论 | 依据 |
|---|---|---|
| `gtk` 0.18.2 → 0.19.0 | ❌ **不可能**（解析期硬失败） | §1.1 |
| 上游是否已支持 gtk 0.19 | ❌ 否，且都是最新版 | §1.2 |
| `gio` 0.18.4 → 0.22.10 | ⚠️ 可行，但**决定不做** | §2 |
| 本轮改动 | **无**（Cargo.toml / Cargo.lock 均未变） | — |

一句话：**不是不想升，是依赖图不允许**；`gio` 虽说能绕过，但要为一个用不到的新 API
付双份 FFI 的代价，不值得。

## 1. `gtk` 0.19：Cargo 在解析阶段就拒绝

### 1.1 逐字报错（`cargo check --features export-bindings --all-targets`，exit 101）

把 `gtk` 改到 `"0.19"`、`gio` 改到 `"0.22"` 后：

```
error: failed to select a version for `gdk-sys`.
    ... required by package `gdk v0.18.0`
    ... which satisfies dependency `gdk = "^0.18.0"` of package `drag v2.1.1 (/home/melaine/code/dae/src-tauri/vendor/drag)`
    ... which satisfies dependency `drag = "^2.1.1"` (locked to 2.1.1) of package `dae v0.4.0 (/home/melaine/code/dae/src-tauri)`
versions that meet the requirements `^0.18` are: 0.18.2, 0.18.0

package `gdk-sys` links to the native library `gdk-3`, but it conflicts with a previous package which links to `gdk-3` as well:
package `gdk-sys v0.19.0`
    ... which satisfies dependency `gdk-sys = "^0.19.0"` of package `gdk v0.19.0`
    ... which satisfies dependency `gdk = "^0.19.0"` of package `gtk v0.19.0`
    ... which satisfies dependency `gtk = "^0.19"` of package `dae v0.4.0 (/home/melaine/code/dae/src-tauri)`
note: only one package in the dependency graph may specify the same links value to ensure that only one copy of a native library is linked in the final binary
help: try to adjust your dependencies so that only one package uses the `links = "gdk-3"` value
```

**读法**：这与「API 变了导致编译报错」完全不同 —— 它发生在**解析期**，
一个 `.rs` 文件都还没编译。`links` 唯一性规则的作用是保证最终二进制里
只链接一份原生库；`gdk-sys` 声明了 `links = "gdk-3"`，因此 gdk 0.18 与 0.19 **不能共存**。

### 1.2 为什么绕不开：两侧夹击

**左侧 —— vendored `drag`**（`vendor/drag/Cargo.toml:31-33`，经 `Cargo.toml:203`
以 path 依赖引入）：

```toml
gdk = "0.18.0"
gdkx11 = "0.18.0"
gtk = "0.18.1"
```

**右侧 —— Tauri Linux 栈**（全部要求 gtk 0.18，且全部已是最新）：

| crate | 当前解析版本 | 是否最新 | 对 gtk 的要求 |
|---|---|---|---|
| `tao` | 0.37.1 | ✅ | `gtk ^0.18` |
| `wry` | 0.57.0 | ✅ | `gtk ^0.18` |
| `muda` | 0.20.0 | — | `gtk ^0.18` |
| `tauri` | 2.12.3 | — | `gtk ^0.18` |

`tao` 与 `wry` 的 `max_stable_version` 分别就是 0.37.1 / 0.57.0，
**上游没有用 gtk 0.19 的版本可等**。所以把 `drag` 改到 gdk 0.19 也无效：
冲突只会从 `drag ↔ gtk 0.19` 转移到 `tao/wry (gdk 0.18) ↔ gtk 0.19`。

### 1.3 代码层耦合：两处 `upcast` 要求版本一致

```rust
// src/tab_windows.rs:191
let widget: gtk::Widget = webview.inner().upcast();
// src/tab_windows.rs:890
let widget: gtk::Widget = webview.inner().upcast();
```

`webview.inner()` 来自 wry/tao，类型是它们的 `gtk` 版本下的 widget；
`.upcast::<gtk::Widget>()` 用的是**我们的** `gtk`。两者必须同版本才能编译。
这条独立于 §1.1 的 `links` 冲突 —— 即便某天 `links` 问题被解决，
这两行仍会拦住混版方案。

## 2. `gio`：技术可行，但决定不升

### 2.1 实测证据（`gtk` 留 0.18、只把 `gio` 改到 0.22）

| 检查 | 结果 |
|---|---|
| `cargo check --features export-bindings --all-targets` | ✅ exit 0 |
| `cargo test --features export-bindings` | ✅ exit 0，**296 passed / 0 failed / 2 ignored** |
| `file_system::open_with::linux::tests` 直接覆盖 gio 路径 | ✅ 3 个测试 |
| 结果 | `gio 0.18.4 → 0.22.10` 真的能跑 |

代价是树上出现**两份 FFI 绑定**（实测 lock 内容）：

```
gdk-sys   0.18.2
gtk-sys   0.18.2
gio-sys   0.18.1  |  gio-sys 0.22.9    ← 两份
glib-sys  0.18.1  |  glib-sys 0.22.9   ← 两份
```

### 2.2 为什么只有 `gio` 能双版本共存

关键在下游 sys crate 的 `links` 属性，实测：

| crate | `links` | 能否双版本共存 |
|---|---|---|
| `gdk-sys` | `links = "gdk-3"` | ❌ 被 §1.1 规则拦住 |
| `glib-sys` | **无** | ✅ |
| `gio-sys` | **无** | ✅ |

所以「gtk 卡死」和「gio 能过」不是矛盾，而是同一个机制在不同 crate 上的不同后果。

### 2.3 决定不升的理由

1. **收益为零**。我们只用 `gio` 的基础 API，全部在 0.18 就有 ——
   `open_with.rs` 用 `gio::File` / `ThemedIcon` / `Icon` / `AppInfo` /
   `AppLaunchContext` / `Cancellable` / `FileQueryInfoFlags`；
   `volume_monitor/linux.rs` 用 `gio::VolumeMonitor` / `Volume` / `Mount` / `Drive`。
   0.22 带不来任何功能。
2. **代价是确定的**：双份 glib/gio `-sys` → 更大的二进制、更慢的编译。
3. **留下一个隐患**：树上同时有 glib/gio 0.18 与 0.22，一旦以后把 `gio` 类型
   在 gio 0.22 代码与 gtk 0.18 代码之间传递，会报出难以理解的类型错误。
   而现在两者版本一致，不会遇到。
4. **拆分升级本来就多余**：§3 的结论是 gio 与 gtk 应当**一起**升，
   单独升 gio 只是把一个暂态固化下来。

**反悔成本很低**：真要单独升 gio，重跑 §4 第 2 条即可，实测已通过。

### 2.4 CI 风险已排查（供将来升级参考）

`gio-sys` 0.22 仅要求系统 `gio-2.0 >= 2.56`，
CI 的 Ubuntu + `libgtk-3-dev` 远高于此，**不构成升级障碍**。

## 3. 什么条件下重新考虑

**触发条件：`tao` 或 `wry` 出现要求 `gtk ^0.19` 的版本。**

届时按此顺序做，**`gio` 与 `gtk` 一起升**：

1. 先核对 `tao` / `wry` 的新版本对 gtk / gdk / glib 的要求（`cargo tree -i gtk`）。
2. 同步升 `gio`（`gtk 0.19` 要求 `gio ^0.22` + `glib ^0.22`，见下）。
3. **检查 vendored `drag`**：它是 `links` 冲突的另一端，
   若它还停在 gtk 0.18，需要一并处理，否则第 1 步就会失败。
4. `cargo check --features export-bindings --all-targets` + `cargo test --features export-bindings`。

**gio 与 gtk 必须同升，不能拆开** —— 实测 `gtk 0.19.0` 声明：

```
gtk 0.19.0 → gio ^0.22, glib ^0.22
gtk 0.18.x → gio ^0.18, glib ^0.18
```

所以不存在「gtk 0.18 配 gio 0.22」的长期稳定组合，那只是 §2.1 的暂态。

## 4. 复现命令

两条实验都很轻量，任一条都能在本机重现本文档结论。

**1) 复现 gtk 0.19 的解析期失败**（约数秒，会在解析阶段就退出）：

```bash
cd src-tauri
sed -i 's|^gtk = "0.18"|gtk = "0.19"|; s|^gio = "0.18"|gio = "0.22"|' Cargo.toml
cargo check --features export-bindings --all-targets   # exit 101，报错见 §1.1
git checkout Cargo.toml Cargo.lock
```

**2) 复现 gio 0.22 单独升级可用**（编译约 20s）：

```bash
cd src-tauri
sed -i 's|^gio = "0.18"|gio = "0.22"|' Cargo.toml
cargo check --features export-bindings --all-targets   # exit 0
cargo test  --features export-bindings                 # 296 passed
rg -A1 'name = "(gio-sys|glib-sys)"' Cargo.lock | rg version   # 看到两份
git checkout Cargo.toml Cargo.lock
```

**3) 查系统 GTK 版本**（判断 gtk-sys 能否构建）：

```bash
pkg-config --modversion gtk+-3.0    # 本文档实测环境：3.24.52
pkg-config --modversion glib-2.0    # 本文档实测环境：2.90.1
```

## 5. 附录：同批依赖升级（2026-10-11）的其余遗留项

本节只做**索引**，细节在各 commit message 里。列出它们是为了避免「以为都验证过了」。

| 项 | 性质 | 状态 |
|---|---|---|
| `@xterm/addon-canvas` 0.7.0 ↔ `@xterm/xterm` 6.0.0 | peer 声明 `^5.0.0`，**上游无适配版**（0.7.0 已是最新） | ⚠️ 未验证 |
| canvas 降级渲染路径 | 依赖仅调的私有 API `_core._createRenderer` 在 xterm 6 中仍存在 | ✅ 静态确认<br>❌ 未运行时验证 |
| `smb2` 0.27.2 强制 `aes >=0.9.1, <0.9.3` | 导致共享 `aes` 由 0.9.3 **降级**到 0.9.2，波及 zip / russh / sevenz-rust2 | ✅ 已查 OSV：两版**均无**漏洞公告，非安全回退 |
| `smb2` / `russh` 真实会话路径 | 单测只覆盖 URL 解析、连接池、路径解析等纯逻辑 | ❌ 仅编译验证，**未开真实连接** |

**russh 升级的额外结论**（曾以为是连锁升级，实测不是）：
`russh-sftp` 对 `russh` 的依赖是 **dev-only**，且代码传的是 `channel.into_stream()`（泛型流），
两者运行时解耦 —— 所以 `russh` 可独立升级，`russh-sftp` 也已是最新的 3.0.1。
