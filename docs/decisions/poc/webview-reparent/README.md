# 存档：tab 拖拽 reparent PoC（已否决路线）

这份目录是 `try/webview-reparent` 分支上 PoC 的只读快照，**不属于构建**。
目的：以后有人想重提「浏览器式标签拖拽」时，能直接看到代码与实测结论，
不必去翻一个没有 upstream 的本地分支。

结论与完整分析见 `docs/decisions/tab-drag-webview-reparent.md`（路线**已否决**）。

## 文件

| 文件 | 原路径 | 说明 |
| --- | --- | --- |
| `reparent_poc.rs` | `src-tauri/src/reparent_poc.rs` | 探针主体（场景 1–4） |
| `reparent-poc.html` | `public/reparent-poc.html` | 静态探针页，用 `window.name` / JS 计数器证明上下文存活 |
| `webview-memory.py` | `scripts/dev/webview-memory.py` | 进程树 RSS + PSS 测量 |

这些文件在 `main` 上**不会被编译**：放在 `docs/` 下纯粹是文本归档。
要真正跑起来，需要按下面的 patch 手工接回 `src-tauri/`。

## 接回所需的改动（两个文件）

`src-tauri/src/lib.rs`：

```diff
+// PoC-only: probes whether a live webview can be re-parented between windows.
+// Remove together with the `unstable` feature in Cargo.toml.
+mod reparent_poc;
@@ pub fn run() {  // invoke handler 链里
+            } else if command == "poc_report" {
+                reparent_poc::handle_invoke(invoke)
@@ // setup 里，window_material::init 之后
+            if reparent_poc::enabled() {
+                reparent_poc::spawn(app.handle().clone());
+            }
+
+            if reparent_poc::real_tab_enabled() {
+                reparent_poc::spawn_real_tab(app.handle().clone());
+            }
```

`src-tauri/Cargo.toml`：

```diff
-tauri = { version = "2", features = ["macos-private-api"] }
+# `unstable` 是 `Window::add_child` / `Webview::reparent` 的前置条件。
+# PoC-only：合并前必须连同上面的模块一起去掉。
+tauri = { version = "2", features = ["macos-private-api", "unstable"] }
```

## 复现命令

```bash
# 场景 1–3：静态探针页
DAE_POC_REPARENT=1 bun tauri dev
python3 -c "import json;[print(json.loads(l)) for l in open('/tmp/dae-reparent-poc.jsonl')]"

# 场景 4：两个真 app 窗口 + 程序化 reparent（等 6s 让 React 起来）
# 探针事件名以 rt- 开头（rt-before / rt-post / rt-after / rt-close-empty-shell …）
DAE_POC_REALTAB=1 bun tauri dev

# 内存（进程树 RSS + PSS）
python3 docs/decisions/poc/webview-reparent/webview-memory.py $(pgrep -f '[t]arget/debug/dae' | head -1)
```

坑（当时踩过，原文见决策文档 §7）：别用 dev-invoke 调碰裸 GTK 的命令
（`tear_off_tab` 会 panic `GTK may only be used from the main thread.`）；
`pkill -f "bun tauri dev"` 会匹配到自己的 shell，要用 `pgrep -f '[t]arget/debug/dae'` 写法。

## 实测结论（Linux / WebKitGTK 2.54.1 / Wayland / niri）

- **机制可行**：真 app bundle 的 webview 被 reparent 后 **JS 上下文存活**，无 reload，IPC 通。
- **但性价比不成立**：`unstable` 特性、每 tab 一个 webview（约 200 MB PSS 量级）、
  拖动逻辑要重写，而现有原生拖动已经不慢。
- 顺带发现一个**与标签页方案无关、当前架构仍然存在**的 bug：
  关闭标签窗口后 `WebKitWebProcess` 不退出（wry GObject 引用环）。已记账、未排期，
  见决策文档 §4.7。
