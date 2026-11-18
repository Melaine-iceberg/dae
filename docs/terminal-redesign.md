# 终端重设计（对齐 Zed）

依据：Zed `crates/terminal` / `terminal_view` 的公开源码与官方文档，对照 dae 现有实现
（`crates/dae-core/src/pty.rs`、`crates/dae-core/src/ghostty_vt.rs`、
`crates/dae-qt/src/terminal.rs`、`crates/dae-qt/cpp/terminal_item.*`、
`qml/TerminalPanel.qml`、`qml/Tab.qml`）。

## 0. 结论

dae 的 VT 内核（vendored libghostty-vt）能力完整，问题不在内核而在三处：

1. **渲染管线全量**：`TerminalSession::snapshot` 每次输出把整个 viewport 拷成
   `Vec<CellSnapshot>`，跨 cxx 桥编码成全屏 `DAET` 帧，`TerminalItem::paint`
   全屏重绘。
2. **会话模型是孤岛**：一个 tab 绑死一个 shell，面板无多会话 tab、无搜索、无路径链接。
3. **CWD 靠猜和注入**：面板标题显示的是「我们把它 spawn 在哪」，运行中跟随目录用
   文本 `cd` 注入（污染 shell 历史，是 Zed 明确不用的反模式）。内核原生支持 OSC 7/9/1337。

Zed 的教训对应成五步，P1→P5，每步独立可验收。

## 1. Zed 的设计要点（抄什么、不抄什么）

| Zed 设计 | 决定 |
|---|---|
| 内核不自研（alacritty_terminal + vte，`alacritty.rs` 封装层隔离第三方类型） | dae 已同理（ghostty-vt + `ghostty_vt.rs`），保留 |
| `mappings/{keys,mouse,colors}.rs` 三文件隔离事件世界与协议字节 | dae 的键盘/鼠标编码散在 `terminal_item.cpp`，P5 归拢 |
| 增量 `TerminalContent` 快照 | P1 |
| 面板内多终端 tab（`Ctrl+N`）+ 分屏 | P2 取多 tab，**分屏不做**（dae 面板是单 dock 滚动布局，收益低） |
| `working_directory` 策略档位；从不在运行中 shell 里 `cd` | P3 |
| OSC 7 + shell 集成上报 CWD（`cwd_history`） | P3 |
| `path_hyperlink_regexes` + 匹配时间预算 | P4 只支持 `path:line[:col]` 一档正则，点击时匹配（零常态开销，等效于无预算问题） |
| 搜索复用编辑器搜索条，i32 行号可指历史 | P4 搜含滚回历史，UI 用面板内自绘条（dae 无可复用的编辑器搜索条） |
| 任务系统寄生终端（`TaskState`/`completion_tx`） | **不做**，dae 无任务系统 |
| 设置两级合并（执行环境可项目级、外观仅用户级） | dae 无项目级设置，不做 |
| bracketed paste、alternate scroll、copy_on_select | P5 |
| vi 模式 | 不做（超范围） |

## 2. P1 增量帧（性能主因）— 已实施

**现状成本**：80×24 一帧 = 24 行全拷贝 + ~25 KB 编码 + 全屏 QPainter 重绘，每读一批字节
就发生一次。

**实测**（`crates/dae-core/examples/dirty_probe.rs`，对 vendored libghostty-vt 1.0.1）：
渲染状态的 dirty 档位**不可用**——哪怕只移动光标、甚至无写入的第二次 update，都报
`DIRTY_FULL` 且全部行报脏。可用的增量信号是 **RowId**：viewport 上滚 k 行时，
新 row i 的 id 恰是旧 row i+k 的 id（探针：30 行输出后 shift=1 精确检出）。

**改动**：

- `ghostty_vt.rs`：声明 `GhosttyRowId`（`ROW_DATA_ID`）与 `Row::id()`；同时保留
  dirty 档位绑定，探针示例靠它监控库升级后行为是否变化。
- `pty.rs`：`SnapshotKind = Full | Shift(k)`。`snapshot()` 每次读全网格并记录各行 id；
  若新帧的每一行都是上一帧某行整体上移 k 的结果（且主题未翻转），只把**底部 k 行新
  cells** 放进快照。主题翻转、resize（`last_ids` 清空）、非纯滚动一律 Full。
- `dae-qt/src/terminal.rs`：`DAET` 帧版本 2→3。头部 25 字节，追加 `kind`（0 全量 /
  1 位移）与 `count`；位移帧体 = k 行 cells。
- `terminal_item.cpp`：位移帧 = shadow grid `memmove` 上移 + 底部 k 行覆盖 +
  全项重绘（行都动了，损坏区域就是全屏；省的是跨桥字节与编码/解码，不是 paint 区域）。
  全量帧路径不变。

**验收（已跑）**：`terminal_smoke.rs` 的流式测试断言 `Full` 与 `Shift` 两态都出现、
`Shift` 帧 cells 数恰为 `k × cols`；`cargo test -p dae-core -p dae-qt` 全绿；
`cargo build -p dae-qt`（C++）通过。GUI 层的滚动连续性未在本环境驱动，见 §7。

## 3. P2 多会话 tab — 已实施

**现状**：`qml/Tab.qml` 的 `property TerminalSession terminal` —— 会话生命周期挂在 tab 上。

**设计**：会话列表升到面板层，对齐 Zed「面板是终端的家，tab 只是给新终端一个起点」。

- `TerminalPanel.qml`：面板持有 `property list<TerminalSession> sessions` +
  `activeIndex`；面板头右侧加会话 tab 条（序号 + shell 名，点选切换，`×` 关闭），
  `＋` 新建。
- 新建目录策略：**当前活动 tab 的目录**（Zed 的 `current_project_directory` 档语义）。
- `Tab.qml`：删除 `terminal` 属性与 `startTerminal()`；面板直接
  `TerminalSession.start(path, cols, rows, shell)`。首次打开面板时若无会话则自动建一个
  （保持「打开即用」）。
- 会话退出：该会话的 tab 显示「已退出 %1 + 重新启动」（重启沿用其 spawn cwd）。
- 生命周期：面板销毁（窗口关）时 `kill()` 全部会话。

**验收标准**：开两个终端跑不同目录互不干扰；切 tab、关 tab 不杀会话；qmllint 通过。
**已自动化的部分**：`DAE_QML_LINT=1` 构建零告警；GUI 交互见 §7 验证限制。

## 4. P3 CWD：OSC 上报替代 cd 注入 — 已实施

**内核能力**（`ghostty/vt/terminal.h`）：`GHOSTTY_TERMINAL_OPT_PWD_CHANGED`（回调）
+ `GHOSTTY_TERMINAL_DATA_PWD`（OSC 7/9/1337 均已解析）；`OPT_TITLE_CHANGED` +
`DATA_TITLE` 同理。

**改动**：

- `ghostty_vt.rs`：声明上述 option/data；`Vt::new` 注册 `pwd_changed`/`title_changed`
  回调（userdata 扩展为 `{ writer, pwd, title }` 共享槽，回调里读取存入）。OSC 7 的
  `file://` URI 做一次 percent-decode + 去前缀。
- `TerminalSession`：暴露 `working_directory()` / `title()`；快照头部带上两者。
- `TerminalPanel.qml`：
  - **删**「跟随目录」开关、`syncDirectory()`、`changeDirectoryCommand()`、
    `shellDirectory` 注入逻辑；
  - 标题改为 `session.title ?? shellName`；工作目录列显示 OSC 上报值，
    未上报时回退 spawn 目录并淡化（tooltip 说明等待 shell 集成）。
- Shell 集成（对齐 Zed：env 标记 + 可选脚本，不改用户 rc）：
  - `spawn` 注入 `DAE_TERM=1`（Zed 的 `ZED_TERM` 同义）；
  - 仓库新增 `Assets/terminal-integration/`：`bash-zsh.sh`（`PROMPT_COMMAND`/precmd
    发 OSC 7）、`powershell.ps1`（`prompt` 函数包装发 OSC 9;9），README 说明
    source/调用方式；设置面板 terminalShell 旁提示一次。
  - **不做**：强制 `--rcfile`/`-Command` 包装（破坏用户 shell 配置，Zed 也不做）。

**验收标准**：source 集成片段后 `cd` 面板标题即时跟随；PowerShell 集成脚本同理；
不装集成时回退路径显示正确、无任何 cd 注入行为。
**已自动化的部分**：`cargo test -p dae-core` 的 OSC 7 URI 解码单测（Windows 盘符、
POSIX 前导斜杠、percent-escape、OSC 9/1337 裸路径）全绿；`DAE_QML_LINT=1` 构建零告警。

## 5. P4 搜索 + 路径超链接 — 已实施

**内核能力**：`ghostty/vt/search.h`（`ghostty_search_*`，含滚回历史）；
`grid_ref.h` 的 `hyperlink_uri`（OSC 8，shell 少发，不作主路径）。

**搜索**：
- `ghostty_vt.rs` 包 `Search` 句柄：`new(vt, pattern)` / `next/prev` / 返回
  （viewport 坐标或历史行号, 列区间）；历史行命中先 `scroll_to_row` 带入视口。
- `TerminalPanel.qml`：`Ctrl+F` 打开面板内搜索条（输入 + 上/下一个 + 计数 `n/m`），
  样式同现有 header 控件；painter 收到高亮区间后以 accent 半透明底描出。
- 滚回行高亮：`display_offset` 换算成历史行号后由 painter 覆盖绘制。

**路径超链接**（Zed 的 regex 方案，点击时才匹配 → 无常态成本，等效满足
`path_hyperlink_timeout` 的意图）：
- `terminal_item.cpp`：`Ctrl+Click`（鼠标上报模式下也拦截，Zed 的
  `open_links_in_mouse_mode` 默认行为）时，从 cell 反查该行文本，跑一次
  `path:line(:col)?` 正则（相对路径按会话 cwd 解析），命中即调用现有
  `open_with`/定位服务打开并跳行；行内 URL（http/https）一并支持，交给系统打开。
- hover 时对命中区域画下划线 + pointer 光标（hover 才做单行正则，量级可控）。

**验收标准**：滚回历史搜 "error" 逐个跳；编译输出里的
`crates/dae-qt/src/terminal.rs:250` Ctrl+Click 打开该文件跳到 250 行。
**已自动化的部分**：`terminal_smoke.rs::search_finds_matches_and_steps_through_them`
（真实 PTY：echo 三行 → 计数 ≥3、步进一致、viewport 高亮矩形非空、清空 needle 归零）；
`DAE_QML_LINT=1` 构建零告警（find bar / 链接信号经 qmllint 门禁）。

## 6. P5 细节包 — 已实施

- **bracketed paste**：`pty.rs` 新增 `paste(&self, text)`——经 `ghostty_paste.h`
  编码器（声明 binding）按当前 `MODE_BRACKETED_PASTE` 包裹；
  `terminal_item.cpp` 的 `paste()` 改走它。vim/tmux 多行粘贴不再被执行。
- **alternate scroll**：wheel 时若备用屏激活 + 程序开鼠标上报 → 经 `mouse.h`
  编码器发方向键序列（Zed `alternate_scroll`）。
- **copy_on_select**：`ExplorerPrefs` 加布尔设置（默认 false），选中即写系统剪贴板。
- 编码归拢（**未做**，如实记录）：`terminal_item.cpp` 的按键→ESC、鼠标→协议序列表
  仍留在 C++。本项是纯搬家、收益只在可维护性，留待后续；实现的是备用屏滚轮走
  Rust 侧 `wheel_scroll`（查 `ACTIVE_SCREEN` 后发方向键），未用 `mouse.h` 编码器。

**已自动化的部分**：`cargo test -p dae-core -p dae-qt` 全绿；
`DAE_QML_LINT=1 cargo build -p dae-app` 零告警（含 `copyOnSelect` 设置项与面板绑定）。
bracketed paste / 备用屏滚轮为运行时行为，GUI 交互未在本环境驱动，见 §7。

## 7. 顺序与依赖

```
P1 增量帧 ──► P3 CWD ──► P4 搜索/链接（P4 依赖 P1 的帧语义与 P3 的 cwd 解析）
P2 多会话（与 P1 无依赖，可并行）
P5 最后（依赖 P1 的 painter 结构定型）
```

每步独立验收：`cargo test -p dae-core -p dae-qt`（帧/搜索/粘贴单测与真实 PTY 冒烟）
+ `qmllint` + 手工冒烟（大输出滚动、vim 备用屏、双终端目录隔离）。

**验证限制**：本环境无 GUI 交互通道，QML 层行为以 qmllint + 代码评审为准；
PTY → 帧 → 解码全链路由 `terminal_smoke.rs`（真实 shell）与帧单测覆盖。
