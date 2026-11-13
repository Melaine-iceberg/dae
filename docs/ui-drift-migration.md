# dae 纸张界面 · drift 迁移

> 日期：2026-11-13 · 分支：当前工作树
> 第一阶段的执行记录，不是讨论稿。参考物是 Figma Make 原型「跨平台桌面文件管理器」
> 导出的 5 个 React 文件（`App.tsx` / `FileIcon.tsx` / `data.ts` / `index.css` / `main.tsx`），
> 原型本身不在本仓库里；下文把它的设计变量写全，读这份文档不需要再打开 Figma。

原型是 React + Tailwind，本仓库是 QML，所以这次迁移不是翻译代码，而是**把原型的设计
变量和三个招牌构件搬进 `qml/`，接到真实的模型和手势上**。行为层不重写：标签、历史、
选中、拖放、排序、回收站、预览全部沿用 Qt 壳已有的实现。

## 1. 设计变量（已进 `qml/Theme.qml`）

| 角色 | 亮色 | 暗色 | 原值 |
| --- | --- | --- | --- |
| `canvas` / `chrome`（纸） | `#ece6d8` | `#131210` | `#ffffff` / `#ecece9` |
| `canvasAlt` / `raised` / `field`（卡） | `#f6f2e8` | `#1d1b17` | `#f4f4f2` / `#ffffff` |
| `ink` | `#16140f` | `#ece6d8` | `#121316` / `#eaebed` |
| `inkMuted` / `inkFaint` | `#4a453b` / `#625c50` | `#b9b3a5` / `#948e80` | 中性灰 |
| `signal` / `brand`（朱红） | `#ff4d1f` | `#ff6a3d` | 仪器橙 `#c2410c` / `#f0925e` |
| `hairline`（表内线） | ink 20 % | ink 20 % | 8 % 黑 |
| `hairlineStrong`（结构线） | ink 80 % | ink 80 % | 24 % 黑 |
| `rowRule` | ink 10 % | ink 10 % | 6 % 黑 |
| `radiusXs…Lg` | 0 | 0 | 4 / 6 / 8 / 12 |
| `edgeWidth` | 1 | 同 | 新增 |
| `folderTab` / `folderBody` | `#f0b838` / `#d99a1c` | `#c08c1e` / `#a4761a` | 新增 |
| `serifFamily` | Windows `SimSun` · macOS `Songti SC` · Linux `Noto Serif CJK SC` | | 新增 |

规则和原型一致：**结构与状态都用线，不用圆角和阴影渐变**。一处亮色只出现在信号色
（选中、焦点、危险操作）。`backdropTint()` 保留 Mica/Acrylic 能力，但遮罩改为纸色，
材质透出来时是"同一张纸举起来"，不是第二个灰蓝色的窗口。

## 2. 构件

- **`qml/DriftSheet.qml`（新）** — 纸张文件卡：90×120 的卡片（折角、扩展名墨标、
  内容草图：文档横线、表格网格、代码暗底、音频波形、视频首帧、压缩包齿、字体 Aa），
  和 120×96 的马尼拉文件夹。Canvas 按原型自己的坐标系绘制后缩放，所以 18 dp 的行内
  小图标和 104 dp 的预览板是同一张图，不是两套比例。`detail: false` 画单色剪影。
  配色与波形用的种子取自**整个文件名**的哈希：早先只哈希长度和首字符，同一目录里四个
  `Windows-FY27-….jpg` 拿到了同一个种子，四张卡同一个红。
  **它现在是全应用唯一的文件图标**：网格、详细列表、分栏视图、冲突对话框两侧、面包屑、
  媒体预览的空封面都换成了它。旧的 `qml/ThemedIcon.qml`（按 category 查线条图标）已无
  调用点，随本次删除；`Glyph` 仍在，负责的不是文件——展开箭头、关闭、回收站、菜单箭头。
- **`ColumnView.qml`** — 行图标换 `DriftSheet`（18 dp、`detail: false`）；选中从"信号色
  淡底 + 3 dp 左轨"改成与详细列表一致的墨色横幅反白，名称与展开箭头随之转纸色。
- **`PathBreadcrumbs.qml`** — 每段路径的图标由线条字形换成 14 dp 的马尼拉剪影，非当前段
  压到 55 %。
- **`ConflictSide.qml`** / **`MediaPreview.qml`** — 冲突对话框侧（16 dp）与音频空封面
  （44 dp，`detail: true`，保留波形）。
- **`FileGrid.qml`** — 图块改为"卡片 + 名字 + 等宽字号"：选中是名字下方的实心墨块
  （纸色字反白），不是给整格染色；悬停时卡片上浮 3 dp；拖放目标才有信号色边框。
  图块高度固定，名字占两行、字号行紧跟其下，短名与长名不会把下一行推下去。
- **`FileList.qml`** — 行选中改为墨色横幅反白，去掉 3 dp 左轨；图标换成 `DriftSheet`
  的剪影；等宽列（大小、日期）沿用模型给的字符串。行首加等宽序号 `01/02/03`（名称列、
  表头、分组标签一起右移 28 dp）；序号报的是**当前顺序**，所以改排序就会重排——这正是它
  要说明的事。
- **`Sidebar.qml`** — 位置行用等宽序号 `01…07` 代替图标（图标只说"某处"，序号说
  "第三个目的地"）；当前行墨色反白、序号转信号色；分组标题用等宽字。卷、网络存储、
  云账户仍保留自己的字形和展开箭头。顶部是 `dae.` 字标（serif 斜体，句点是朱红，
  只对读屏隐藏，因为它不是控件）；底部是分段容量条（24 格、每格约 4 %、>90 % 转 `warn`，
  下面一行 `已用 / 总量` 等宽）。
- **`qml/HardShadow.qml`（新）** — 浮动面的硬阴影：目标矩形 +5,+5 的实心墨色副本，无模糊。
  它是目标的**兄弟**而不是子项（子项会盖在父项之上，负 z 也仍在父项自己的绘制之后），
  所以每个宿主里声明在面板之前。`Theme.shadowOffset = 5` 随之回填。
- **浮动面** — 视图菜单、筛选菜单、右键菜单与子菜单、命令面板、状态中心、传输队列、
  `ConfirmDialog` / `NamePrompt` / `PropertiesDialog` / `BulkRenameDialog` / `MoveToDialog` /
  `OpenWithDialog` / `ConnectDialog` / `CloudAccountDialog` / `ArchivePasswordDialog` /
  `UpdateDialog` / `TransferConflictDialog` 各获得一条阴影。
- **`Toast.qml`** — 改成墨色板 + 纸色字 + **朱红**硬阴影（4 dp）：drift 里唯一不是墨色阴影的
  面，用来区分"刚刚发生了什么"和"有什么打开了"。撤销/关闭的悬停改纸色 14 %，因为在墨底上
  原来的 12 % 墨wash 看不见。
- **`crates/dae-qt/src/sidebar_model.rs`** — 新增 `diskFill` / `diskUsed` / `diskTotal`：
  容量条只描述**一块**盘（第一块报告容量的固定盘，全是可移动介质时退到第一块有数字的），
  选择规则抽成 `footer_capacity()` 并配 4 个测试（固定盘优先、只有可移动介质、总容量为 0、
  没有卷）。探测开始先把三个值清空，避免换盘后显示上一台机器的数字。
- **`FileList.qml` 标题带** — 等宽小标题（`文件夹` / `搜索结果`）+ serif 56 dp 的目录名
  + 信号色上标计数 + 等宽元信息行（`2 个文件夹 · 60 个文件 · 更新于 2026-11-13 01:49`），
  高 118 dp，列表与网格共用；分栏视图不画（它的列从顶边开始）。标题是静态文本：无
  MouseArea、无 tab 停靠点，读屏拿到一句话。搜索态没有"文件夹那一半"可数，就整行不画。
- **`qml/Grain.qml`（新）+ `Assets/grain-{ink,paper}.png`** — 纸纹：128 dp 的逐像素噪点平铺，
  纸色主题用墨点、暗色主题用纸点，两张贴图各 ~23 KB，位于仓库根的 `Assets/`（打包用的位图
  都在那里，qrc 路径仍是 `qml/assets/…`，所以它们不会出现在 `qml/` 的目录列表里）。
  QML 的 `Image` 没有混合模式，`ShaderEffectSource` 又要每帧重抓整窗；两张极性的噪点贴图
  在低透明度下做同一件事，代价是一个带 repeat 的纹理四边形。覆盖整窗（含菜单与对话框），
  不吃输入。
  **强度**：原型是 7 %（`multiply` / 暗色 `screen`），本机实测太强——满窗的细颗粒读起来是
  一层滤镜而不是纸。现为**纸色 3 % / 暗色 2 %**，暗色低一档是因为浅色颗粒在近黑底上更容易
  读成"屏幕脏了"。
- **`crates/dae-qt/src/model.rs`** — 目录模型补 `folders` 与 `newestModifiedText`，由
  `publish_counts()` 那次唯一的全表遍历一起算出（`summarize()`，纯函数，带测试：
  种类各计一次、符号链接既不算文件夹也不算文件但它的时间仍算"最近更新"、空目录全零）。
  新增一次遍历只为一行文字，对 12 万行的目录是不划算的。
- **`qml/DotPlate.qml`（新）** — 预览面板的点阵底：12 dp 一格、ink 28 %、Canvas 现画，
  不塞位图（位图在每个缩放比下都会糊）。`DriftSheet.shadowed` 补上无模糊的墨色偏移
  阴影（+5,+5 的同一形状），预览板上那张 −3° 的卡片就是纸卡 + 硬阴影。
- **`PreviewPanel.qml`** — 空状态换成"点阵底 + 倾斜纸卡 + serif 标题"；多选换成
  "点阵底 + serif 96 dp 的朱红计数"（数字自己说话，不再挂单位字）；单文件顶部加等宽
  朱红种类小标 + serif 30 dp 名字；元信息行改成虚线规则、键值等宽；标签 `预览` 转等宽。
- **`ShellWindow.qml` 状态条** — 状态文字改等宽；右端加 drift 的方块光标（`Theme.motion`
  关闭时不动，不是变快）；传输 chip、状态中心、分支、面板开关全部保留。左侧多一段剪贴板
  指示（`剪贴板 N · 复制/剪切`，读 `Workspace.clipboardMode`，900 dp 以下隐藏）。

  原型的页脚右端还有一串快捷键提示（`⌘C 复制 · ⌘X 剪切 · …`），照做后**按评审意见去掉**：
  这些绑定在右键菜单、命令面板和设置面板里都已经印着，页脚再念一遍是占位而不是信息。
  `statusHints()` 一并删除，没有留下死代码。

## 3. 未落地（下一阶段）

| 项 | 位置 | 说明 |
| --- | --- | --- |
| 面板内部的弹层 | `TransferQueue.qml` 的限速菜单等 | 只给了顶层浮动面；嵌在面板里的小弹层仍是平贴的边 |
| 整窗面板 | `SettingsPanel` / `GitPanel` | 有意不加阴影：它们是版面的一部分，不是浮在纸上的纸 |

## 4. 验证

纸纹开销：`QSG_RENDER_TIMING=1` + 15 次 PageDown（每次整窗重绘），解析 `frame rendered in Nms`：

| | 帧数 | 平均 | 最大 |
| --- | --- | --- | --- |
| 开 | 57 | 1.51 ms | 33 ms |
| 关 | 55 | 1.71 ms | 31 ms |

差值落在噪声里：覆盖层的成本低于该计时器 1 ms 的分辨率。最大值那 30 余毫秒来自滚动本身
（列表重排），与纸纹无关。

```bash
export QMAKE="K:/Qt/6.12.0/msvc2022_64/bin/qmake.exe"
export PATH="K:/Qt/6.12.0/msvc2022_64/bin:$PATH"
cargo build -p dae-app
/k/Qt/6.12.0/msvc2022_64/bin/qmllint.exe qml        # 只剩既有的 unqualified-access 提示
node scripts/a11y-audit.mjs --baseline              # 6 项均为本次改动之外的既有缺口
```

构建后把 `target/debug/dae.exe` 放进 `dist/`（那里有 windeployqt 的 DLL 与插件），
`dist/dae.exe <目录>` 运行。截图在 `docs/prototypes/`：`baseline-before.png`（改前）、
`after-hero-grid5.png`（浅色网格）、`after-hero-list2.png`（浅色列表选中）、
`after-hero-dark.png`（暗色列表选中）、`after-preview-empty2.png`（预览空状态）、
`after-preview-file.png`（单文件详情）、`after-preview-multi3.png`（多选计数）、
`after-sidebar2.png`（字标 + 容量条）、`after-sidebar-dark.png`（暗色）、
`after-rows-footer2.png`（行号 + 状态条）、`after-meta.png`（标题带元信息行）、`after-grain-soft2.png`（纸纹 3 %）、
`after-grain-dark.png`（暗色 2 %）、`after-icons-column.png`（分栏视图 + 面包屑图标）、`after-footer-clean.png`（去掉提示行的页脚）、
`after-sheet-colours.png`（不同文件不同色）、
`after-menu-shadow2.png`（视图菜单）、`after-dialog-shadow.png`（新建文件夹提示）、
`after-toast2.png`（墨色 Toast + 朱红阴影）。

想复现浅色 + 网格：设置里主题选浅色，显示菜单切网格（等价于
`%APPDATA%\com.melaine.dae\settings.toml` 的 `theme = "light"` 与
`explorer-qt.ini` 的 `viewMode=grid`）。

## 5. 已知边界

- 暗色下的纸色是 `#131210`，与原来的 `#0f1012` 只差色温；对比度已按 §3.1 的门槛复核
  （`inkMuted` 两套都 > 7:1，`inkFaint` > 5:1）。
- 原型里的 `drift.` 字标不搬：产品名是 dae，字标本应写成 `dae.`；它连同侧栏底部的
  分段磁盘条留到下一阶段。
- **实测边界**：行首序号、`F2` 大写（`Shortcut.js` 的显示修正，菜单与命令面板一起受益）在截图里验证过；剪贴板指示的**状态**没能在
  这台机器上用合成输入驱动出来（`SendKeys` 的 `Ctrl+C` 三次都没到列表的按键处理，加在
  `Workspace.clipboardMode` 上的变更日志始终为空），只验证了它的渲染与布局。它读的是
  每个菜单的「粘贴」项已经在用的同一个属性，所以状态来源本身是既有行为。
- 标题带的 56 / 30 dp 是字面量：只出现一次，等第二处需要同一档时再收进 `Theme`。
