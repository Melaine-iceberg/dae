# dae 仪器感界面 · QML 落地清单

> 日期：2026-10-28 · 分支：`refactor/qt-quick`
> 原型为准，行为不回退。本文是执行清单，不是讨论稿。

这份文档把会话里的高保真原型翻译成 QML 改造任务。当前 Qt 壳的行为层已经基本齐全——标签、历史栈、搜索、预览、终端、回收站、命令面板、传输队列都在；这次要做的是**把视觉、密度和外壳结构对齐原型**，而不是重写文件管理器。

三句话给执行者：

1. 先读 §2「不可回退」和 §10「文件地图」，再动代码。
2. 所有颜色、圆角、字号、行高只从 `qml/Theme.qml` 取；不要再往组件里写新的视觉常量。
3. 每个阶段结束都要能构建、能截图、能通过 §12 的对应验收；不要一次性铺开所有文件。

---

## 1. 参考物

| 用途 | 位置 |
| --- | --- |
| 视觉与交互原型 | [`docs/prototypes/dae-instrument-ui-20261028.html`](prototypes/dae-instrument-ui-20261028.html) |
| 旧版设计规范（会被本次替换的部分） | [`docs/ui-ux-2026.md`](ui-ux-2026.md) |
| 唯一色板 | [`qml/Theme.qml`](../qml/Theme.qml) |
| Qt 壳与命令策略 | [`qml/ShellWindow.qml`](../qml/ShellWindow.qml) |
| 引擎与原生边框决策 | [`docs/decisions/engine-qt-quick.md`](decisions/engine-qt-quick.md) |

原型是自包含 HTML 片段：用浏览器直接打开即可，支持 724 / 1280 宽自适应；底部「原型控制」条只用于演示主题、平台、密度、强调色，**不进入产品**。原型里的假数据、`dae://` 路径、只读终端也不要搬进 QML。

运行 QML 壳（Windows 需先把 Qt 的 `bin/` 放进 `PATH`，详见引擎决策文档）：

```bash
cargo run -p dae-app
cargo build -p dae-app        # 三平台 CI 使用的真实构建
```

---

## 2. 不可回退

改造期间以下能力必须保持可用。任何一项被视觉改动弄坏，都算阻塞：

- 每标签独立的前进/后退栈、`Tab.qml` 的路径历史。
- 标签拖拽排序、撕出为新窗口（`TabStrip.qml` / `TearOffWindow.qml`）。
- 列表 / 网格 / 分栏三视图的选中、框选、Shift 连选、type-ahead、右键菜单。
- 搜索：按名称 + 按内容两种模式、结果选中与预览同步。
- 空间（Spaces）、概览（Overview）、Git 分支面板、传输队列、只读终端。
- 回收站还原、永久删除、清空；属性、重命名、冲突确认、压缩包密码等全部对话框。
- 网络存储 / 云账户连接。
- 设置里的快捷键重绑：菜单和命令面板显示的必须是当前绑定，不是默认值。
- 无边框窗口的原生命中测试：标题栏拖动、标签点击、窗口按钮、缩放边框。
- `Theme.motion`（减少动效）开关：关闭时全部瞬时切换，不是缩短时长。
- 中文 / 英文 i18n；所有新增可见字符串进 `qml/i18n/`。

---

## 3. 设计变量

### 3.1 颜色

把 `qml/Theme.qml` 的角色值改成下表。命名保持不变；只改值，避免 41 个组件同时改属性名。

| 角色 | 亮色 | 暗色 | 用途 |
| --- | --- | --- | --- |
| `canvas` | `#fbfbf9` | `#151619` | 列表、预览正文、内容面 |
| `canvasAlt` | `#f4f4f1` | `#1b1c20` | 次级内容面、成组区域 |
| `chrome` | `#efefec` | `#101114` | 标题栏、导航条、侧栏、状态条 |
| `raised` | `#ffffff` | `#202125` | 菜单、对话框、命令面板 |
| `field` | `#e9e9e5` | `#17181b` | 搜索框、路径编辑、重命名 |
| `ink` | `#161719` | `#ecedef` | 文件名、正文 |
| `inkMuted` | `#5c6066` | `#9ba0a8` | 次级信息、列头、状态 |
| `inkFaint` | `#646970` | `#8b9098` | 分组标签、弱提示；两套都 ≥4.5:1 |
| `hairline` | `#14000000` | `#16ffffff` | 结构线、未聚焦控件边 |
| `hairlineStrong` | `#25000000` | `#25ffffff` | 输入框、浮层、抬起面的边 |
| `hover` | `#0c000000` | `#0dffffff` | 指针悬停 |
| `fill` | `#12000000` | `#14ffffff` | 激活 chip、当前标签、菜单悬停 |
| `rowRule` | `#09000000` | `#0bffffff` | 列表行分隔；比 hairline 更轻 |
| `signal` | `#b45309` | `#f0a24a` | 全窗口唯一信号色 |
| `signalSoft` | `#1ab45309` | `#1af0a24a` | 选中行、激活菜单项 |
| `signalSoft2` | `#2eb45309` | `#29f0a24a` | 拖放边、搜索框聚焦底 |
| `onInteractive` | `#ffffff` | `#1b1306` | 信号色上的字（即原型里的 `signalInk`） |
| `danger` | `#b42318` | `#f0918a` | 破坏性文字 |
| `dangerSolid` | `#c62c26` | `#c6403a` | 破坏性主按钮 |
| `warn` | `#9a6700` | `#e0a340` | 容量 >90%、冲突 |
| `success` | `#2f7d32` | `#8fd18f` | 完成态 |
| `link` | `#0969da` | `#6cb6ff` | 提交哈希、引用型信息 |
| `scrim` | `#1a181816` | `#61000000` | 模态遮罩 |

补充规则：

- **默认强调色改为 `brand`，brand 就是上表的信号橙。** 这样新装用户看到的是仪器感橙色；「系统强调色」仍保留在设置里。需要改 `crates/dae-core/src/settings.rs` 的默认值（当前是 `"system"`）和对应测试。`Theme.brand` 与 `Theme.signal` 取同一对值，`Theme.interactive` 仍按 `Appearance.accent` 在 brand / 系统色之间选择。
- 设置面板的「dae 蓝」改为「信号橙」。原型里的青/蓝只是演示色板，**不新增**到 Rust 设置枚举。
- `Theme.selection` 改为信号色 10%（亮）/16%（暗）；`focusRing` 保持 30%。
- 同步改 `Theme.qml` 里 `inkFaint` 的注释：旧注释写着「低于 4.5:1，只做图标」；新值两套主题都过 4.5:1，可以承载 11px 分组标签与元数据。
- 一屏内信号色最多 3 处：当前行左轨、焦点环、一个主按钮/激活 chip。不要给图标、卡片、分组标题上色。
- 左侧当前行是 `signalSoft` 底 + 2px `signal` 左轨；不要用整行饱和填充。
- 文件类型图标全部单色化；颜色只留给 git 状态小徽章（M 用 `warn`，U 用 `link`）、容量告警和危险操作。
- **材质**：设置里的「背景效果」默认开启。Windows 默认 Mica，可切换 Acrylic；系统关闭透明效果或版本不支持时自动回退为实色。Linux Wayland 通过 `ext-background-effect-v1` 模糊背景，合成器未提供该协议时保持实色；macOS 使用 `NSVisualEffectView` 的系统毛玻璃并跟随窗口深浅色。半透明 chrome 使用 `chromeTint`，实色回退仍用 `Theme.chrome`。

### 3.2 字号

系统无衬线字体继续沿用平台栈；等宽仍用 `Theme.monoFamily`。字号只允许以下 8 档（一屏同时出现不超过 4 档）：

| 角色 | px | 用途 |
| --- | --- | --- |
| micro | 11 | 列头、分组标签、快捷键、状态附件 |
| caption | 11.5 | 状态条、空状态提示、预览元数据标签 |
| small | 12 | 标签页、文件元数据、表格数字、菜单辅助 |
| body | 12.5 | 侧栏行、面包屑、菜单项、命令面板项 |
| name | 13 | 文件名、列表主行、对话框正文 |
| emphasis | 13.5 | 预览文件名、命令面板输入 |
| title | 14 | 预览/对话框标题 |
| display | 22 | 多选预览的计数；不要用于标题或空状态 |

- 全仓删除 `pixelSize: 10`（`ContextMenu.qml` 的组标题目前是 10）和更小值。
- 尺寸、日期、传输速率、容量一律 `font-variant-numeric: tabular-nums`（QML 用 `font.features: {"tnum": 1}` 或继续用现有等宽/`Text` 数字设置）。
- 字重只用 Regular / Medium（400/500）；标题不要 Bold。
- 一屏同时出现的字号不超过 4 档。

### 3.3 几何

| 项 | 目标值 | 备注 |
| --- | --- | --- |
| 标题栏 | 44 | 含标签；mac 左侧让位 78 |
| 导航条 | 44 | 当前 46，减 2 |
| 状态条 | 28 | 当前 26，加 2；总高仍 116 |
| 行高 | 28 / 32 / 40 | 紧凑 / 默认 / 宽松 |
| 表头 | 28 / 32 / 36 | 与行高成对 |
| 标签 | 32，上边距 6 | 激活下划线 2px，左右各缩进 8 |
| 新标签按钮 | 26 × 26 | 上边距 9 |
| 导航按钮 | 28 × 28 | 圆角 4 |
| 面包屑行高 | 28 | 文字 12.5 |
| 搜索框 | 210 × 28 | 圆角 4；聚焦 1px signal |
| 搜索筛选条 | 32 | chip 22，圆角 2 |
| 侧栏 | 216（180–280 可拖） | 6px 拖拽缝 |
| 预览 | 300（260–420 可拖） | 6px 拖拽缝；默认 300 |
| 菜单 | 212–286 宽 | 内边距 4，行高 32，圆角 8 |
| 命令面板 | 560 宽 | 顶部 64，圆角 8；输入区 44 |
| 简单对话框 | 440 宽 | 顶部 64，圆角 12；标题区 44 |
| 数据型对话框 | 现有功能宽度 | 属性/批量重命名等可保持 600，但圆角 12、标题区 44 |
| Toast | 34 高 | 底部 38，圆角 4 |
| 传输浮层 | 326 宽 | 底部 32，圆角 8 |
| 终端 | 132 高 | 内容区内嵌 |
| 圆角 | 2 / 4 / 8 / 12 | xs / sm / md / lg，全部进 Theme |

- 表格列宽（宽屏）：名称 `1fr`、类型 108、大小 92、修改时间 142；中屏砍类型，窄屏再砍大小。
- 侧栏容量条 2px，填充 `signal` 50% 不透明度；>90% 换 `warn`。
- 图标尺寸：行图标 16、侧栏图标 14、菜单/面板图标 14、空状态 28、网格字形 38。
- 所有 1px 线用 `hairline`；只有输入框、浮层、抬起面用 `hairlineStrong`。

建议在 `Theme.qml` 增补这份最小 token 集，组件只引用它们：

```qml
readonly property int radiusXs: 2
readonly property int radiusSm: 4
readonly property int radiusMd: 8
readonly property int radiusLg: 12
readonly property int rowCompact: 28
readonly property int rowDefault: 32
readonly property int rowSpacious: 40
readonly property int shellTitle: 44
readonly property int shellNav: 44
readonly property int shellStatus: 28
readonly property real typeMicro: 11
readonly property real typeCaption: 11.5
readonly property real typeSmall: 12
readonly property real typeBody: 12.5
readonly property real typeName: 13
readonly property real typeEmphasis: 13.5
readonly property real typeTitle: 14
readonly property real typeDisplay: 22
readonly property color rowRule: dark ? "#0bffffff" : "#09000000"
readonly property color terminalCanvas: "#0b0b0c"
```

### 3.4 动效

- 指针反馈 90–110ms；菜单/面板 160–180ms；面板尺寸变化 180ms。
- 只动 `opacity` 与 `transform`/位置；不要给行做错峰入场。
- `Theme.motion === false` 时全部瞬时完成；沿用现有 `Behavior { enabled: Theme.motion }` 写法。
- 菜单、Toast 的退出不做淡出；关闭即消失。

---

## 4. 外壳

### 4.1 标题栏 `qml/TitleBar.qml` + `qml/TabStrip.qml`

- 高度 44；mac 左侧 78 让位红绿灯；Windows/Linux 右侧窗口按钮 46×44 / 40×44。
- 应用标记 17×17，只在非 mac 显示；mac 不画应用图标。
- 标签：32 高、上边距 6、圆角 4（只圆上边），未激活 `inkMuted`，激活 `fill` 底 + `ink`。
- 激活标签底部 2px `signal` 短线，左右各缩进 8；关按钮只在悬停/激活时出现。
- Windows 关闭按钮悬停 `#c42b1c` 背景 + 白字；最小化/最大化悬停 `hover`。
- 标签拖动、撕出命中区域不要被新的圆角/内边距吃掉。
- **改标题栏几何必须同时核对原生命中测试**：`ShellWindow.qml` 里 `chrome.setCaptionClient(44 + clientLeft, 44 + clientRight)` 这一对值要跟新的标签带一致；否则点标签会变成拖窗口。
- `TabStrip.qml` 的 `clientLeft/clientRight` 是原生命中测试的输入，不是纯视觉属性。

### 4.2 导航条

目标结构（从左到右）：

`后退 前进 上一级 | 面包屑 … [右侧] 清空(仅回收站) 搜索 视图 更多`

- 高 44，左右内边距 8，按钮 28×28，圆角 4。
- **删除左侧汉堡菜单**。侧栏显隐放进「视图」菜单和命令面板；窄屏自动收起，不靠汉堡。
- 面包屑：12.5px，当前段 `ink` + 500，其余 `inkMuted`；每个 crumb 24 高、圆角 3、悬停 `hover`。
- 路径编辑：等宽 12，`field` 底，1px `signal` 边，行高 28。
- 搜索：默认只显示放大镜按钮；点开后右侧出现 210×28 输入框，`Esc` 先清空、再收起。
- 「视图」按钮显示当前视图图标，点开菜单（列表/网格/分栏 + 预览/终端开关）；「更多」保留刷新、隐藏文件、排序、设置等。
- 回收站视图在右侧多一个「清空」文字按钮，危险色只在悬停时出现。
- 导航条所有图标按钮必须有 `Accessible.name`。

### 4.3 状态条

高 28，三段：

- 左：`42 项`、`已选 3 项`、`12 KB`；用 `·` 分隔，`caption` 字号，数字等宽。
- 中：传输胶囊 `2 项传输 —— 68%`；点击打开 326px 浮层。
- 右：分支图标 + `main`（不要填充胶囊、不要「Git」前缀）、预览开关、终端开关。
- **删除**状态条里的启动路径、profile 文本、设置入口、更新入口；设置进「更多」菜单。
- 分支太长用省略号，保留 `main`/分支名首段；ahead/behind 数字留在分支面板里，状态条只显示分支名。

### 4.4 响应式

| 宽度 | 行为 |
| --- | --- |
| ≥1180 | 侧栏 + 列表 + 预览三栏 |
| <1180 | 隐藏预览（拖拽缝一起隐藏） |
| <920 | 侧栏 184；容量文字改成 `60%`；列表保留名称/大小/修改时间 |
| <760 | 隐藏「类型」列；状态条中间传输隐藏 |
| <640 | 隐藏侧栏；列表占满 |
| <560 | 隐藏大小/修改时间，只留名称 |

- 任何宽度下都不出现横向滚动；列表右侧不允许被裁切。
- 默认窗口 1180×780；最小 900×600。
- 预览宽度、侧栏宽度继续记忆在 `ExplorerPrefs`，不要写死。

---

## 5. 侧栏

`qml/Sidebar.qml`、`qml/SidebarTree.qml`、`qml/Glyph.qml`、`SidebarModel`。

### 5.1 分组与顺序

1. `位置`：主目录、桌面、下载、文稿、图片。
2. `磁盘` / `宗卷`：卷行，带容量。
3. `空间`：工作 / 个人 / 共享 / 归档 + 新建空间。
4. `网络存储`：WSL（仅 Windows）、连接网络存储。
5. `云存储`：添加网盘账户。
6. 底部固定 `回收站` / `废纸篓`，显示项目数。

- **卷组在空间组之前**；回收站固定到底部，不再放在顶部。
- 移除侧栏顶部的「概览 / 回收站」头区。概览保留在产品里，但入口改为命令面板的「概览」和「视图」菜单；不要为了对齐原型直接删掉功能。
- 音乐 / 视频 / 收藏等额外条目可以保留，但排在五个主位置之后；首屏不超过 10 行。
- 分组标签 20 高、11px、字距 0.08em、`inkFaint`。

### 5.2 行

- 行高 28，圆角 4，左右内边距 10，图标 14，标签 12.5，右侧信息 11 等宽。
- **去掉图标背后的 16×16 着色方块**；图标直接画在行内，`inkFaint`，悬停/激活时 `inkMuted`。
- 当前行：`fill` 底 + 左侧 2px `signal` 轨道；悬停 `hover`；拖放目标 `signalSoft` + 1px `hairlineStrong`。
- 卷行高 38：两行结构（名称行 + 2px 容量条），容量文字 `286 / 477 GB`；窄侧栏只显示 `60%`。
- 体积超过 90% 时容量条和百分比用 `warn`。
- 空间行的「空」是 11px `inkFaint`，不要用符号代替。
- 回收站计数 `2 项`；计数为 0 时留空。
- 侧栏宽度 216 默认，可拖 180–280，6px 拖拽缝；拖拽时显示 1px `signal`。

---

## 6. 文件列表

### 6.1 列表 `qml/FileList.qml`

- 表头 32，行 32（紧凑 28 / 宽松 40）；行底边 `rowRule` 1px。
- 名称 13 `ink`；类型/大小/修改时间 12 `inkMuted`；表头 11 `inkFaint`、字距 0.05em。
- 选中：`signalSoft` 底 + 2px 左轨；悬停 `hover`；键盘焦点 1px `signal` 内描边。
- 名称列图标 16 单色；文件夹不再用蓝色填充，文件不再用类型色块。
- 目录大小显示 `—`；大小/时间右对齐，数字等宽。
- git 徽章：`M` / `U` 9px 小方块，紧贴文件名；只在这里使用 warn/link 色。
- 排序列头显示 4px 上下三角；点击同一列切换升降序。
- 重命名输入框使用 `field` 底、1px `signal` 边、12.5px。
- 拖放目标行用 `signalSoft` + 1px `signal`；拖拽源降低透明度。
- 保留现有框选、Shift 连选、type-ahead、双击打开、右键菜单。

### 6.2 网格 `qml/FileGrid.qml`

- **去掉卡片**：无卡片底色、无 12px 圆角、无阴影。
- 网格 `auto-fill, minmax(100px, 1fr)`，间距 14×12，内边距 16。
- QML 的 `GridView` 没有 CSS `auto-fill`：按 `columns = max(1, floor((width - 32 + 12) / (100 + 12)))` 求列数，再把剩余宽度均分给 `cellWidth`；窗口尺寸变化时重算，不要写死 5 列。
- 字形 38；名称 12.5、居中、最多两行；名称与字形间距 7。
- 选中：`signalSoft` 底 + 1px `signal` 内描边，圆角 4；悬停只给 `hover`。
- 图片继续显示缩略图；文件夹用单色文件夹字形。
- 网格中也要显示键盘焦点环，且与选中可区分。

### 6.3 分栏 `qml/ColumnView.qml`

- 列宽 240–260，表头 32，行 32；列之间 1px `hairline`。
- 当前列选中行同样是 `signalSoft` + 2px 左轨；右侧显示 12px 箭头。
- 父列当前项保持 `fill` 底；分栏导航更新面包屑与预览。
- 三列以上时允许横向滚动，但不要出现整窗横向滚动。

### 6.4 图标 `qml/ThemedIcon.qml` + `qml/Glyph.qml`

- 重写 `ThemedIcon` 的绘制：保留 `category` / `isDir` / `size` API，内部改为单色线稿；删除类型色表和彩色填充。
- 扩展 `Glyph.qml`：`folder`、`file`、`image`、`code`、`archive`、`media`、`drive`、`network`、`cloud`、`trash`、`diamond`、`branch`、`terminal`、`panel`、`chevron`、`check`。全部 16 单位框、1.2 线宽、圆角线帽。
- 真实系统缩略图仍走现有后端；不要把图片换成图标。
- 侧栏、标签页、菜单、空状态统一从 `Glyph` 取图标，不再用 unicode 字符或字体图标。

### 6.5 空状态

- 居中：28px 单色图标 → 标题 13 `ink` → 提示 11.5 `inkFaint` → 文字按钮（`signal`）。
- 文案：
  - 搜索无结果：`没有匹配项` / `试试其他关键词，或检查大小写与类型筛选。` / `清除搜索`
  - 空文件夹：`这个文件夹是空的` / `拖入文件，或在这里新建文件夹。` / `新建文件夹`、`粘贴`
  - 空空间：`这个空间是空的` / `把文件拖进来，或在这里新建文件夹。` / `新建文件夹`
  - 空回收站：`回收站为空` / `删除的文件会先放在这里，可以随时还原。`
- 不要插画、不要大标题、不要骨架屏。

### 6.6 回收站与搜索结果

- `TrashView` / `ContentResults` 复用列表行样式；回收站第四列为「删除时间」。
- 回收站视图导航条显示「清空」，确认后永久删除；空状态居中。
- 搜索结果沿用同一套行高与选中；筛选条显示匹配数；选中项被筛掉时自动改选第一个可见项，不能出现预览显示已隐藏文件的情况。

---

## 7. 面板

### 7.1 预览 `qml/PreviewPanel.qml` / `MediaPreview.qml`

- 宽度默认 300，可拖 260–420，双击恢复 300；6px 拖拽缝，悬停 `signal`。
- 头部 32：`预览` 11.5 `inkFaint`，右侧 20×20 关闭。
- 文本预览：等宽 11、行号列 28、行高 1.6、超出省略；代码块底色 `canvasAlt`、圆角 4。
- 图片：内容宽 -24，最大高度 200，圆角 8，1px `hairline`。
- 文件夹：显示项目数与前几项文件名。
- 元数据：标签 11 `inkFaint`，值 12.5 `ink`，行高 24；底部「全部属性」用 `signal` 文字 + 箭头。
- 未选中：`未选择文件` / `单击文件查看详情`；回收站中改为 `选中项目查看原位置`。
- 关闭后主区扩展，状态条预览开关同步；窄屏自动隐藏。

### 7.2 终端 `qml/TerminalPanel.qml`

- 高 132，内容区内嵌，不覆盖列表。
- 头部 28：`终端 · zsh / bash / PowerShell`，右侧关闭 20×20。
- 背景 `#0b0b0c`（进 `Theme.terminalCanvas`），文字 11 等宽，提示符用 `signal`。
- mac 提示符 `%`，Linux `$`，Windows `PS …>`；shell 名按平台/后端显示。
- 保留真实终端焦点、复制粘贴、滚动；不要用演示文本替换。

### 7.3 传输 `qml/TransferQueue.qml`

- 326 宽浮层，底部 32，圆角 8，`raised` 底，`overlay` 阴影。
- 头部 26：`传输队列` 11 字距 + 关闭。
- 每行：文件名 11.5、右侧状态 11 等宽（`68% · 4 秒` / `完成`）、2px 进度条、目标路径 11 `inkFaint` 单行省略。
- 进度填充 `signal`；完成态可保持满条但不要变成绿色。
- 行悬停 `hover`；空队列显示一行提示，不留空白面板。

### 7.4 Toast `qml/Toast.qml`

- 34 高，底部 38 居中，圆角 4，`raised` + `hairlineStrong`，12px。
- 结构：图标 14 → 文本（最大 320，单行省略）→ 可选「撤销」12px `signal` → 关闭 20×20。
- 4.8 秒自动消失；撤销后立即关闭；退场瞬时。

---

## 8. 搜索、命令面板、菜单、对话框

### 8.1 搜索

- 输入框 210×28，圆角 4；`Esc` 清空再收起；空输入时筛选条显示「输入以筛选名称」。
- 筛选条 32，顺序：`文件名 / 内容`（模式 chip）→ `Aa` → 类型下拉 → 右侧「N 项匹配」。
- chip 22 高、11px、圆角 2；激活 `fill` + `ink`；不要用大药丸。
- 保留内容搜索模式；网络位置禁用内容搜索并在 chip 上降低透明度。
- 类型下拉沿用现有分类；不要新增筛选维度。
- 查询变化时同步选中与预览；结果为空时显示居中空状态。

### 8.2 命令面板 `qml/CommandPalette.qml`

- 宽 560，顶部 64，圆角 8，`raised`，overlay 阴影。
- 输入区 44：搜索图标 14 + 输入 13 + `Esc` 键帽 20×18。
- 分组标签 11 `inkFaint`，行 30；图标 14，标签 12.5，右侧 chord 11 `inkFaint`。
- 激活行 `signalSoft` + 左侧 2px `signal`；悬停 `hover`。
- 底部 32：`↑↓ 选择`、`↵ 执行`、`Esc 关闭`，右侧结果数。
- 保留渐进式文件名搜索与「在文件内容中搜索」兜底动作；没有结果时不要留白。
- 所有 chord 从 `Shortcut.js` 当前绑定取，mac 上显示 `⌘`、`⇧`、`↩`。

### 8.3 菜单 `qml/ContextMenu.qml` / `DisplayMenu.qml` / `FilterMenu.qml`

- 宽 212–286，内边距 4，圆角 8，行 32，图标 14，标签 12.5，chord 11。
- 危险项只在图标与文字上用 `danger`，不要整行红底。
- 分隔线 1px `hairline`，上下各 4；组标题 11 `inkFaint`（删除 10px）。
- 打开后焦点落在第一个可用项；`↑↓`、`Home/End`、`Enter`、`Esc` 全可用；超出窗口时向内夹取。
- 行菜单、空白菜单、侧栏菜单、回收站菜单的文案保持一致；mac 的 `⌘⌫`、`↩`、`⌘↓` 由 chord 函数生成。

### 8.4 对话框

- 简单确认/命名/连接：440 宽，顶部 64，圆角 12；头部 44，标题 14，正文 13，底部 44。
- 属性、批量重命名、设置、云账户等数据型对话框：沿用现有功能宽度（属性 600），只统一圆角 12、头部 44、按钮 28 高、标签/值字号。
- 遮罩 `scrim`；点击遮罩关闭（破坏性确认除外）。
- 焦点进入对话框，`Esc` 关闭；`Tab` 在对话框内循环；按钮顺序 `取消` → 主操作。
- 破坏性主按钮 `dangerSolid`，普通主按钮 `signal` + `signalInk`。
- 表单标签 11 `inkFaint`，输入 28 高、`field` 底、聚焦 1px `signal`。

---

## 9. 平台、快捷键、可访问性

### 9.1 平台

| 项 | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 窗口按钮 | 右 3×46 | 左红绿灯，让位 78 | 右 3×40 |
| 卷组名 | 磁盘 | 宗卷 | 磁盘 |
| 回收站文案 | 回收站 | 废纸篓 | 回收站 |
| 终端 shell | PowerShell | zsh | bash |
| 修饰键 | Ctrl | Cmd | Ctrl |
| 章 | `C:\` | `/` | `/` |

- 卷名、根路径、回收站名称来自模型/i18n，不在 QML 里写死 `C:` / `D:` / `WSL`。
- 终端 shell 名按平台/后端；Windows 不要显示 zsh。
- mac 隐藏应用标记，保留红绿灯；Windows/Linux 不画红绿灯。

### 9.2 键盘

| 动作 | Windows / Linux | macOS |
| --- | --- | --- |
| 打开 | Enter | Cmd+↓ |
| 重命名 | F2 | Return |
| 上一级 | Alt+↑ / Backspace | Cmd+↑ |
| 后退 / 前进 | Alt+← / Alt+→ | Cmd+[ / Cmd+] |
| 移到回收站 | Delete | Cmd+Delete |
| 显示隐藏文件 | Ctrl+H | Cmd+Shift+. |
| 搜索 | Ctrl+F | Cmd+F |
| 路径编辑 | Ctrl+L | Cmd+L |
| 命令面板 | Ctrl+K | Cmd+K |
| 新建文件夹 | Ctrl+Shift+N | Cmd+Shift+N |
| 列表/网格/分栏 | Ctrl+1/2/3 | Cmd+1/2/3 |
| 属性 | Alt+Enter | Cmd+I |
| 上下文菜单 | Shift+F10 | Shift+F10 / 右键 |

- 所有菜单/面板里显示的 chord 必须走 `Shortcut.display` 或等价的平台转换，不要写死 `F2`/`Ctrl+…`。
- mac 上 Enter 是重命名、Cmd+↓ 是打开；Windows 上 Enter 是打开、F2 是重命名。两者不能混。
- 原生标题栏命中测试、窗口缩放边框在改动后必须手测。

### 9.3 可访问性

- 每个图标按钮有 `Accessible.role` 与 `Accessible.name`；列表行有 `Accessible.selected`。
- 焦点顺序：标题栏标签 → 导航条 → 侧栏 → 列表 → 预览 → 状态条；对话框打开时焦点进入对话框。
- 焦点环 1px `signal`，始终可见；不要用 `focus: false` 隐藏。
- 对话框 `Esc` 可关；菜单 `Esc` 可关；没有键盘陷阱。
- 对比度：`ink` ≥12:1，`inkMuted` ≥4.5:1，`inkFaint` 在本方案中 ≥4.5:1（与旧规范不同，旧规范把 inkFaint 限制为图标；这次它承担 11px 标签，所以值被调深）。

---

## 10. 文件地图

| 原型区域 | QML 文件 | 主要动作 |
| --- | --- | --- |
| 标题栏 / 标签 | `TitleBar.qml`、`TabStrip.qml` | 44 高、32 标签、窗口按钮、命中测试 |
| 导航条 / 面包屑 | `ShellWindow.qml` 导航段、`PathBreadcrumbs.qml` | 44 高、删汉堡、面包屑、搜索/视图/更多 |
| 状态条 | `ShellWindow.qml` 状态段 | 28 高、左中右三段、去 profile/设置 |
| 侧栏 | `Sidebar.qml`、`SidebarTree.qml`、`Glyph.qml`、`SidebarModel` | 分组顺序、行样式、卷容量、回收站底部 |
| 列表 | `FileList.qml` | 行/表头、选中、图标单色、git 徽章 |
| 网格 | `FileGrid.qml` | 去卡片、格子尺寸、选中框 |
| 分栏 | `ColumnView.qml` | 列宽、选中、表头 |
| 搜索结果 | `ContentResults.qml` | 复用列表行样式 |
| 预览 | `PreviewPanel.qml`、`MediaPreview.qml` | 300 宽、头部 32、元数据 |
| 终端 | `TerminalPanel.qml` | 132 高、平台 shell、黑色底 |
| 传输 | `TransferQueue.qml` | 326 浮层、行样式 |
| 命令面板 | `CommandPalette.qml` | 560 宽、分组、chord |
| 菜单 | `ContextMenu.qml`、`DisplayMenu.qml`、`FilterMenu.qml` | 212–286、32 行、危险色 |
| 对话框 | `ConfirmDialog.qml`、`NamePrompt.qml`、`PropertiesDialog.qml`、`ConnectDialog.qml` 等 | 440/数据宽度、头部 44、圆角 12 |
| 空状态 | 各视图内空状态 | 居中、28 图标、文案 |
| Toast | `Toast.qml` | 34 高、底部 38、撤销 |
| 色板/几何 | `Theme.qml` | 全部 token |
| 设置默认强调色 | `crates/dae-core/src/settings.rs` | brand 默认 + 测试 |
| i18n | `qml/i18n/dae_zh_CN.ts`、`dae_en.ts` | 新增/改文案 |

---

## 11. 执行顺序

每个阶段独立可构建、可截图、可提交。提交信息沿用现有风格：`feat(qt-quick): …` / `fix(qt-quick): …`。

### 阶段 0：基线

- 在 Windows 上运行当前壳，截 `列表 / 网格 / 分栏 / 搜索 / 预览 / 回收站 / 终端 / 传输` 八张图，存 `.perf/instrument-before/`。
- 记录当前窗口尺寸、侧栏/预览宽度、缩放比例；只在 Windows 本机先做，不跨平台。
- 验收：能重复打开同一目录，截图可比对。

### 阶段 1：Theme + 外壳骨架

- 改 `Theme.qml`：色板、selection 透明度、radius/type 常量、`rowRule`、`terminalCanvas`、默认 brand 橙。
- 改 `settings.rs` 默认 accent 和测试；改设置面板「dae 蓝」文案。
- 改 `ShellWindow.qml` 三段高度为 44/44/28；`TitleBar.qml` 44；`TabStrip.qml` 标签 32/上边距 6。
- 导航条：删汉堡、按钮 28、面包屑字号、搜索框 210×28、视图/更多。
- 状态条：28、左中右三段、去 profile/设置/更新。
- 全程核对 `setCaptionClient` 命中测试。
- 验收：116px 外壳；标签点击/拖动/撕出正常；三平台能构建；宽/窄截图无溢出。

### 阶段 2：侧栏 + 图标

- 分组重排：位置 → 卷 → 空间 → 网络 → 云 → 回收站底部；移除顶部概览/回收站头区，概览进命令面板/视图菜单。
- 行 28/12.5、去图标色块、激活左轨、卷行 38 + 容量条、窄侧栏百分比。
- 重写 `ThemedIcon` 单色；扩展 `Glyph`；替换侧栏/标签/菜单的 unicode 图标。
- 验收：侧栏无彩色方块；卷组标签随平台；WSL 只在 Windows；回收站固定底部；拖放侧栏仍可用。

### 阶段 3：列表 / 网格 / 分栏 + 空状态

- `FileList` 行/表头/选中/徽章/排序；`FileGrid` 去卡片；`ColumnView` 统一。
- 空状态居中与文案。
- 验收：三种视图选中、框选、Shift、type-ahead、双击、右键、拖放都正常；1440 与 900 宽无裁切。

### 阶段 4：预览 / 终端 / 传输 / 状态细节

- `PreviewPanel` 300/头部 32/元数据/文件夹/图片；grip 260–420。
- `TerminalPanel` 132/平台 shell/黑色底。
- `TransferQueue` 326/行/进度。
- 状态条传输胶囊与分支显示。
- 验收：预览拖宽、关闭、图片/文本/文件夹/回收站四种状态；终端真实输入；传输浮层不遮列表关键区域。

### 阶段 5：搜索 / 面板 / 菜单 / 对话框 / Toast

- 搜索筛选条与选中同步；命令面板 560；菜单 32 行；对话框头部/圆角；Toast 34。
- 保持所有 chord 平台化。
- 验收：空结果/内容搜索/类型筛选；面板键盘全流程；菜单危险项；属性/连接/确认；撤销 Toast。

### 阶段 6：平台 / 可访问性 / 回归

- Windows 本机构建、截图与键盘回归已完成；Linux/macOS 交给 `qt-check`。
- 键盘全流程、焦点环、`Accessible.name`、对话框 Tab 循环、减少动效已落地。
- `docs/ui-ux-2026.md` 的 token 表、几何表、验收段已同步。
- 验收：§12 清单已勾选，记录见 §12.4。

---

## 12. 验收清单

### 12.1 视觉与尺寸

- [x] 外壳总高 116：44 标题 + 44 导航 + 28 状态。
- [x] 默认行高 32；紧凑 28 / 宽松 40；表头成对。
- [x] 侧栏 216，卷组在空间组之前，回收站固定底部。
- [x] 预览 300，拖拽 260–420；窄屏自动隐藏。
- [x] 菜单 212–286、面板 560、简单对话框 440、Toast 34、传输 326。
- [x] 圆角只出现 2 / 4 / 8 / 12。
- [x] 字号只出现 11 / 11.5 / 12 / 12.5 / 13 / 13.5 / 14 / 22。
- [x] 无彩色文件类型图标；信号色一屏 ≤3 处。
- [x] `grep` 无新增硬编码视觉 hex；允许的例外：第三方品牌色、语法高亮、红绿灯。
- [x] 亮/暗两套下 `inkFaint` 对 chrome/canvas/field 均 ≥4.5:1。

### 12.2 行为回归

- [x] 新标签、关闭、拖动排序、撕出、恢复。
- [x] 每标签独立历史；后退/前进/上一级；`Ctrl/Cmd+L` 路径输入合法与非法路径。
- [x] 列表/网格/分栏切换；排序升降；选中、连选、框选、type-ahead。
- [x] 搜索名称/内容、大小写、类型筛选、空结果、清除。
- [x] 重命名冲突提示；新建文件夹/文件；复制/剪切/粘贴；撤销。
- [x] 回收站：移入、还原、永久删除、清空、空状态。
- [x] 拖到侧栏/文件夹；拖放高亮；不可投放目标。
- [x] 预览文本/图片/文件夹/回收站；宽度记忆。
- [x] 终端真实输入；传输队列；git 分支面板。
- [x] 设置快捷键重绑后菜单/面板显示新绑定。
- [x] 窗口拖动/缩放/最小化/最大化/关闭；标签点击不触发窗口拖动。
- [x] 减少动效开启后无动画；关闭后动画正常。
- [x] 英文界面无截断；中文界面无溢出。

### 12.3 键盘与可访问性

- [x] Tab 顺序：标签 → 导航 → 侧栏 → 列表 → 预览 → 状态条。
- [x] 焦点环始终可见；对话框/菜单 `Esc` 可关。
- [x] 鼠标悬停才出现的信息，键盘也能到达（标题、按钮名）。
- [x] 图标按钮有可读名称；列表行有选中状态。

### 12.4 阶段 6 验证记录（2026-10-29）

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| Windows 构建 | 通过 | `cargo build -p dae-app`，59 s |
| Rust 回归 | 通过 | `cargo test --workspace`：416 passed，13 ignored |
| QML 语法 | 通过 | `qmllint` 全部改动文件无 syntax/error |
| 运行时告警 | 通过 | 启动/交互日志无 QML 警告，仅沙箱样板缩略图失败 |
| 阶段 6 截图 | 通过 | `.perf/phase6/`：Tab 链、侧栏焦点、设置面板 Tab 循环、搜索、900 窄屏、暗/亮主题 |
| 视图回归 | 通过 | `.perf/stage3-test.ps1`：列表/网格/分栏、排序、框选、type-ahead、900 宽 |
| 行为回归 | 通过 | `.perf/regression.ps1`：标签、路径、终端、设置、模态、重命名、IME |
| 文件操作/预览/搜索 | 通过 | 阶段 3/4/5 截图与用例继续有效 |
| 类型/圆角审计 | 通过 | `pixelSize` 仅 11/12/13/14/22；圆角仅 2/4/8/12（胶囊为 width/2） |
| 硬编码颜色审计 | 通过 | 剩余字面量仅 Theme、品牌色、语法高亮、红绿灯、媒体井 |
| 对比度 | 通过 | 亮/暗 `inkFaint` 对 chrome/canvas/field 均 ≥4.5:1，`ink` ≥12:1 |
| 撕出标签 | 通过 | `.perf/phase6/tabdrag/03-torn-window.png` + 源窗口单标签 |
| Linux/macOS 构建 | 通过 | `qt-check` [40881400232](https://github.com/Melaine-iceberg/dae/actions/runs/40881400232)：ubuntu-26.04 + macos-latest 均构建通过；`linux-check` [40881400361](https://github.com/Melaine-iceberg/dae/actions/runs/40881400361)、`macos-check` [40881400222](https://github.com/Melaine-iceberg/dae/actions/runs/40881400222) 同批通过（提交 `0f415d4`） |

说明：Windows 上 `Alt+Enter` 与“全部属性”按设计交给系统属性表，因此应用内
`PropertiesDialog` 的键盘循环由设置面板验证（两者使用同一 `DialogFocus.js`
和 `DialogButton`）。Linux/macOS 的构建结论以 `qt-check` 为准，本记录不等价
于替代该 CI。本轮 CI 还暴露出三处只在非 Windows 平台成立的缺陷，已在
`0f415d4` 一并修好：Wayland 协议文件被当作 C++ 编译导致 `wl_interface` 表
没有 C 链接、cxx-qt 生成的头文件副本绕过 `#pragma once` 造成类重定义、
`desktop_entry` 的 `pub(crate)` 跨 crate 不可见。

---

## 13. 决策与范围

以下是本次执行按“推荐值”处理，若产品另有决定再改：

- **D1 默认强调色**：`brand`（信号橙），系统强调色仍可选。需要改 Rust 默认值与测试。
- **D2 概览入口**：从侧栏顶部移除，保留在命令面板和「视图」菜单；不删除 Overview 功能。
- **D3 额外侧栏条目**：音乐/视频/收藏可保留，但排在五个主位置之后。
- **D4 分屏**：原型未展示，但产品已有；保留，入口放「视图」菜单，分隔线 1px `hairline`，不要为它改三栏主结构。
- **D5 数据型对话框宽度**：属性/批量重命名等保持现宽度，只统一圆角、头部、字号；不要为了 440 强行压缩内容。
- **D6 原型控制条**：只属于演示，不进入 QML；主题/平台/密度/强调色仍走设置。
- **D7 青/蓝强调色**：原型演示用，不加入产品设置；产品只保留「系统强调色 / 信号橙」。

不在本次范围：重写 Rust 模型、改文件操作语义、重新设计设置页/概览页的信息架构、引入 Qt Quick Controls、做 RTL。

---

## 14. 完成定义

当且仅当：

1. `cargo build -p dae-app` 在 Windows、Linux、macOS 通过（CI `qt-check`）。
2. 1180×780 与 900×700 两档、亮/暗两套截图与原型并排无明显结构差异。
3. §12 清单全部勾选。
4. `docs/ui-ux-2026.md` 的 token/几何/验收段已同步到本文件，不再出现两套圆角或两套状态条高度。
5. 没有为了“更像原型”而删掉 §2 里的任何行为。

