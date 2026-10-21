# Raycast 示例对齐 —— 交接文档

状态：**第 1–3 轮已提交**。第 1、2 轮：令牌层 / 外壳 / 列表 / 侧栏 / 网格 / 复选框面板 /
命令面板 / 动作岛 / 终端抽屉 / 图标线性化。第 3 轮（§2 全部清空）：空状态与骨架屏、Miller 列视图、
检查器、终端配色、菜单与对话框、键帽与焦点环、搜索胶囊、窄窗复核、文档收尾。
本文给下一个会话用：环境怎么跑、已经改了什么、还剩什么、每项的验收标准。

参考文件（已入库）：[`reference-raycast.html`](reference-raycast.html) —— 原文件在
`C:\Users\Mt342\Desktop\dae-explorer.html`，本文所有"示例"数值都指它 `<style>` 里的 token 与组件规则。

**方向已定：示例优先。** `docs/design-v3.md` 已在第 3 轮按本方向整篇重写（"代码现在是什么"的
说明书）；§3 的冲突表现在只剩历史价值。

---

## 0. 环境与验证回路（新对话第一件事）

三件事必须都起来，否则要么没有 shell、要么截不了图、要么页面里全是 `[dev-invoke] could not reach…`：

```bash
# 1) 前端 dev server（1420）
cd /k/code/dae && bun run dev

# 2) 真实后端（dev-invoke 监听 3030）——浏览器里的 UI 靠它取真实数据
./src-tauri/target/debug/dae.exe

# 3) Chrome DevTools MCP 打开 http://localhost:1420/ 后 take_screenshot
```

已知坑（都踩过）：

| 现象 | 原因 / 处理 |
|---|---|
| `bash` 工具报 *No bash shell found* | pi 只找 `C:\Program Files\Git\bin\bash.exe`。`~/.pi/agent/settings.json` 的 `shellPath` 要指向 `C:\Users\Mt342\scoop\apps\git\current\bin\bash.exe`；改完**要重启 pi** |
| Chrome MCP 报 *Could not find Google Chrome executable* | `.pi/mcp.json` 里 chrome-devtools 已加 `--executablePath C:\Users\Mt342\scoop\apps\googlechrome-canary\current\chrome.exe`；改完要重启 pi |
| 截图里强调色全是灰的 | 浏览器预览没有窗口焦点事件 → `html[data-window-focused="false"]` 把 accent 抽干。截图前先执行 `document.documentElement.dataset.windowFocused='true'` |
| Ctrl+K / Ctrl+L 没反应 | 同一原因，热键被 `hotkeysPaused` 挡住。改用 UI 触发器（概览页的"搜索命令…"框点开命令面板） |
| 想砍掉我起的进程 | `taskkill //IM dae.exe //F` + 按 `:1420` 的 PID kill |

截图之外还要跑：`bun run build`（= `tsc` + `vp build`，是唯一的类型/打包闸门；`bun run lint` 目前未跑绿）。

---

## 1. 已完成（不要重做）

### 令牌层 `src/App.css`

- `--border` `rgba(255,255,255,.06)`，新增 `--border-soft` `.04`（容器线 / 行分隔线两档），导出 `border-soft`
- 新增第四档墨色 `--meta: #6a6b6c`（暗）/ `#8b929b`（亮）→ `text-meta`
- 选中态暗色 **15% 填充 / 45% 描边**（`--selection` / `--selection-edge`）
- `.tab-pill` = `card` 填充 + 顶部 8% 内高光 + 8% 外环，**无接触阴影**
- `.content-panel` = `rgb(27,28,30)` 外环 + `rgb(7,8,10)` 内环（示例 `--elev-ring`）
- `.row-separator` = `inset 0 -1px 0 0 var(--border-soft)`
- 几何：`--height-toolbar: 54px`、`--height-status-strip: 26px`、`--sidebar-width: 14.5rem`(232)、`--width-preview: 18.75rem`(300)、`--radius-panel: +8px`(16)
- 图标：全窗 Solar `linear`，1.5px 描边；`--solar-secondary-*` 与 `.type-glyph` 纸面色已删（见 §7.5 of `design-v2.md`）

### 外壳

- `explorer-tabs.tsx`：**拍平**——去掉 `gap-2.5 px-2.5 pb-2.5` 与 `.content-panel` 纸板，rail 与主列直接贴窗口；标签 chip `h-[30px] w-[230px] rounded-md text-caption`，关闭键 18px
- `explorer-toolbar.tsx`：路径胶囊 `h-8 rounded-md bg-background/60` + 1px inset hairline（hover 0.10 / focus `--ring` 35% 环）；导航簇去底板（`NAV_CLUSTER_CLASS`），视图簇 `CLUSTER_CLASS` = 示例 `.seg` 凹槽
- `terminal-panel.tsx`：浮岛 → 抽屉（`border-t` + `bg-muted/60`，头 30px）

### 列表 / 网格 / 侧栏

- `file-list.tsx`：表头 30px、11px 大写 +0.07em、hover 只变字色；行 `rounded-md`、名称 13px、修改/大小列 mono 12px、内边距 8px、`!isSelected && "row-separator"`
- `preferences.ts`：`DENSITY_ROW_HEIGHT = { compact: 28, comfortable: 34, spacious: 44 }`
- `sidebar.tsx`：行 30px / `rounded-md` / 13px；分组标题 11px 600 大写 +0.08em；磁盘卡同步
- `file-grid-view.tsx`：卡片静止态 `bg-card/45` + hairline + 12px 圆角

### 浮层

- `command-bar.tsx`：660px / `top-[12vh]` / 头部 py-3.5 / 输入 36px·18px / 列表 `max-h-[min(52vh,26.25rem)]` / 行 38px / 选中 = `bg-accent` + `inset 2px 蓝条` / `bg-card` / 页脚 11px
- `contextual-action-bar.tsx`：胶囊 + 毛玻璃（`bg-background/62`、`blur-18 saturate-150`、内高光 + 大投影 + 1px 环）

### 新增：底部状态栏

`explorer-chrome.tsx` 的 `ExplorerStatusBar`（在 `explorer-view.tsx` 渲染）：

```
● 已就绪 │ 2 个项目 │ 已选 1                      剩 299.1 GB ▬▬▬
```

- 模式点（`listing.mode.ready|loading|searching`）、项目数、已选数、按最长挂载点匹配的磁盘余量
- 同时把 `ListingStats`（路径栏尾部）改成**仅搜索/选中时出现**，避免计数印两遍
- 新增 i18n key：`en|zh-CN/explorer.json` → `listing.mode.*` / `listing.selectedCount` / `listing.freeSpace`

### 顺手修的

- `settings-dialog.tsx` 的 `onValueChange` 类型（`string` → `AccentPreference | null`）——否则 `bun run build` 是红的

---

## 2. 第 3 轮：§2 全部清空

每项写清：改了什么、验收怎么过、有没有偏离示例。

### 2.1 空状态 + 骨架屏 ✅

- `empty.tsx` 换成示例 `.empty`：容器 `64px 24px` 居中、`.plate` 52×52 / 12px 圆角 /
  前景 5% 填充、标题 15px/600、段落 12.5px / 1.6 / **34ch**、`mb-4` 的节奏。
- 空目录给**一个**动作按钮「新建文件夹」（`onCreateDirectory`），搜索无结果给「清除筛选」
  （新增 `FileList.onClearSearch` → `search.setQuery("")`）。新增 i18n：
  `list.searchEmptyHint` / `list.clearFilter`；`list.emptyFolder` 去掉句号、`list.emptyHint`
  改成示例的语气。
- `skeleton.tsx` 换成 4% 前景 wash + `--radius-md`。`FileListSkeleton`：表头 30px 且
  与真表头同款（nano 大写 +0.07em、`bg-background/88`、`px-2` 轨道）、行用品格的密度高度、
  单元格 `mx-2`（原来是 `mx-3`，每次导航横向抖 4px）；网格/分栏分支用示例的 26px 横条。
- 验收截图：空目录、搜索无结果、列表骨架、网格骨架（临时把目录分支短路成骨架屏后截图，已还原）。
- 偏差：**保留 `animate-pulse`**（示例 `.skel` 是静止的）。静止灰块在慢盘上读起来像渲染失败。

### 2.2 Miller 列视图 ✅

- 行改用列表的行文法：`rounded-md`、`text-caption`（13px）、`row-separator` 行线、
  选中只换填充不换字重；列间 hairline 用 `--border`；`PANE_VERTICAL_PADDING_PX` 10 → 8（配 `py-2`）。
- 新增键盘文法（示例没有列视图，按 Finder 惯例补）：
  `↑/↓` 在列内移动（虚拟化行未挂载时按行高滚动并重试）、`→` 钻进焦点行并把手焦交给子列、
  `←` 收回子列并把焦点放回打开它的那一行。跨列焦点用 `PaneFocusRequest`（带 token 的 state，
  不是 ref——已挂载的父列也必须重渲染才能认领）。
- 验收：`→` 进 `geek` 后焦点落在 `geek.exe`，`←` 回来落在 `geek`；空子列时焦点落到列容器本身，
  否则 `←` 会变成死键。

### 2.3 检查器 / 预览面板 ✅

- `entry-preview.tsx` 按 `.insp` 重排：54px 标题栏删除，改成 `信息` 大写小标 + 关闭键；
  `.pv-head`（26px 字形 / 14px 600 名字 / mono 10.5px 副行）；预览井 `.preview-body`
  （canvas 底 + 12px 圆角 + 内线，代码区 mono 11 / 1.7 / `10px 12px` / 最大 220px）；
  元数据改成 74px 标签列 + 11.5px 值 + `--border-soft` 行线（含媒体标签、时长、分辨率、码率）；
  动作改成纵向 `gap 6px` 全宽。多选摘要同样换行文法（内容 / 最近修改 / 位置）。
- 新增 `App.css`：`.inspector`（`72% canvas + 28% card` + 左侧 hairline）、`.preview-body`、`.note`。
- 顺手修了一个旧 bug：Solar 图标用内联 `style="width:16px"`，`className="size-12"` 压不过它，
  所以预览里的回退字形一直是 16px——改成 `size={48}` / `size={26}`。
- 偏差：**没有行号列**（示例 `.pv-code .ln`）。高亮 HTML 里跨行的 token（多行注释、模板字符串）
  一旦按 `
` 切开就会破标签；素材没有稳定的"每行一个 span"约定。
- 偏差：主按钮仍是强调色填充（示例 `.btn-primary` 是近白胶囊 + pill 圆角）。这是整窗的主按钮，
  不在本轮范围。

### 2.4 终端配色 ✅

- 默认 12px / 1.75（Rust `default_font_size`/`default_line_height`、`CLIENT_DEFAULT_SETTINGS`、
  设置面板回退值、`terminal-panel` 常量，五处同步；Rust 测试断言一起改）。
- 抽屉默认高 188px（原 288）。
- 头部标题换成 `.term-tab` 的 22px 会话 chip；正文内边距 10/16（放在 wrapper 上，
  FitAddon 量的是网格本身）。
- xterm 背景改成示例 `.term` 的 `70% canvas + 30% card`。**注意**：Chromium 把 `color-mix()`
  计算成 `oklab(...)` / `color(srgb ...)`，xterm 解析不了会直接画黑；所以 `blendTokens()`
  把两个 token 取成 `rgb()` 再自己混。
- 偏差：终端默认前景仍是 `--foreground`（示例 `.term-body` 是 fg-2）。ANSI 表一个字没动——
  它是按对比度算过的（见 `terminal-palette.ts` 的注释）。

### 2.5 右键菜单 / 对话框逐像素 ✅

- `--min-width-menu` 180 → **220px**（示例 `.ctx`）；context / dropdown / select 三处的面板
  `p-1.5` → `p-[5px]`、底色 `bg-popover` → `bg-card`、`text-body` → 12.5px。
- 行：`h-8 rounded-lg px-2.5` → `min-h-[30px] rounded-md px-[9px]`（`min-h` 而不是 `h`：
  有描述的第二行会换行，固定 30px 会裁字）；图标 gap 保持 10px；危险项 hover 10% → **16%**。
- 分隔线 `-mx-1 my-1` → `mx-1.5 my-[5px]`（示例 `hr` 的 `5px 6px`）。
- 对话框：`max-w-sm`(384) → **440px**、`rounded-xl` → `rounded-panel`(16px)、`bg-popover` → `bg-card`、
  标题 `text-title`(22) → **18px/600**、说明 12.5px/1.6、底部按钮去分隔线（示例 `.foot` 只有 16px
  上边距）、遮罩加 `backdrop-blur-[3px]`。
- 属性对话框的 `dl` 换成 `.sheet .row-line`（9px 上下 + `--border-soft` + 值右对齐）。

### 2.6 键帽与焦点环统一 ✅

- `Kbd` 换成 `.kbd-cap`：19px / 4px 圆角 / mono 11px 500 / 双层边缘（9% 内高光、1.5px 接触影、
  1px 7% 外环）/ card→canvas 的浅浅渐变。值写在 `App.css`（组件层），调用方要更小的键只需覆盖
  高度和字号。
- 焦点两档落地：控件 `ring-[3px] ring-ring/35`（13 个文件里的 `ring-2 … /40` 全部替换，
  含 select 的 `data-popup-open` 与 input-group 的 `has-[…]`）；行保持 `ring-1 … ring-inset`。
  `App.css` 的焦点注释块同步成这两行。

### 2.7 搜索胶囊 ✅

- 高度 `h-10` → `h-8`、`rounded-xl` → `rounded-md` + 1px 内线、输入 13px（`text-caption`）。
- 宽度行为本来就是对的（176 / 256 / `<1360px` 折成放大器），保留；实测 1440=176、1181=32、1081=32。

### 2.8 窄窗让位顺序复核 ✅

- 1440 / 1181 / 1081 / 901 四张截图。**发现并修掉一个真 bug**：900px 时面包屑互相重叠
  （"Mt3Documents"）。原因是非当前项的 `span.truncate` 是 flex 子项但没有 `min-w-0`，
  `max-w-40` 挡不住压缩，文字溢出到邻居身上；给 `.truncate` 的 span 和它所在的 flex 容器补
  `min-w-0` 后，900px 正确折叠成 `C:\ › … › Mt342 › Documents`。

### 2.9 文档收尾 ✅

- `docs/design-v3.md` 整篇重写：从 Aura 说明改成"示例语言"的现状说明书（平面/字阶/形状/尺度/
  状态/动效/各处表面 + 这套语言拒绝什么 + token 在哪）。
- `docs/design-principles.md`：头部状态行说明已被 v3 取代；图标段（`line-duotone` + 58% 强调色）
  就地标注作废并改成 `linear` + 墨色；选中 20/26% 改成 15%。

### 2.10 仍未做的

- **亮色模式只看了改动面**：概览、列表 + 检查器（代码预览）、右键菜单、空状态、键帽。
  `.inspector` / `.kbd-cap` / `.preview-body` / `.note` 全部走 token 混合，亮色下都成立；
  但没有逐个表单/对话框复查。
- **终端抽屉的亮色没截**（xterm 的背景 `blendTokens` 每主题重算，逻辑上跟随，
  但没在亮色下跑过一次 shell）。
- `bun run lint` 仍未跑绿（与第 1 轮相同）。
- `docs/design-v2.md` 仍在，作为 Atlas 历史保留。

## 3. 与旧 `design-v3.md`（Aura）的冲突清单 —— 已归档

第 3 轮已把 `design-v3.md` 重写成"现状说明书"，下表与代码一致，只作为"第 1 轮为什么这么选"
的记录保留。

| 项 | 示例 / 现状 | `design-v3.md` |
|---|---|---|
| 外壳 | 拍平：rail 与主列同平面，无板无槽 | sheet（`.content-panel`）+ 10px canvas 沟槽 + 20px 投影 |
| 标签条 / 工具栏 | 44 / 54 | 50 / 56 |
| 列表行高 | 28 / **34** / 44 | 30 / **38** / 48 |
| 行内文字 | 13px（`text-caption`） | 14.5px（`text-body`） |
| 侧栏 | 232px / 行 30px | 264px / 行 36px |
| 圆角 | 8px 基数（工作档 6px，palette 12，panel 16） | 12px 基数（9 / 12 / 16 / 18） |
| 分组标题 | 11px 600 大写 +0.08em（示例做法） | 11px 小写、靠字重 |
| 菜单/对话框底色 | `--card`（`#101111`） | `--popover`（`#1a1b1d`） |

`App.css` 的排版刻度（title 22 / lead 17 / body 14.5 / caption 13 / micro 12 / nano 11）**两边一致**，不用动。

---

## 4. 工作区里属于 Aura 方向的既有改动（保留未动）

这些不是本轮产物，方向也不同，但为了不留半截状态一起提交了：

- `src/lib/system-accent.ts`（强调色跟随系统的 seam）
- `src/features/settings/settings-dialog.tsx` 的 Appearance / 强调色面板
- `src/main.tsx`
- `docs/redesign/aura-*.png`

如果最终确认"示例优先"，这些仍然可以保留（强调色 seam 与示例不冲突，示例里也写着 accent 可换）。

---

## 5. 每轮收尾清单

1. `bun run build` 绿（tsc + 打包）—— 第 3 轮已绿
2. 截图：列表 / 网格 / 选中 / 动作岛 / 检查器 / 命令面板 / 右键菜单 / 空目录 —— 已过
3. 宽度：1440 / 1180 / 1080 / 900 —— 已过
4. 亮色过一遍：`document.documentElement.classList.toggle('dark')` —— 改动面已过（见 §2.10）
5. 关掉 `dae.exe` 与 dev server

> 截图环境备注：dev server 可能已经在 1420 上跑着（`Get-NetTCPConnection -LocalPort 1420`），
> 别急着再起一个；`dae.exe` 必须单独起，否则页面全是 `[dev-invoke] could not reach`。
> 浏览器预览窗口宽度会被上一次 `resize_page` 记住，测宽度前先确认 `innerWidth`。

---

## 6. 第 4 轮：把"间距和卡片"换成示例的密度

第 3 轮结束时外壳、列表、菜单都已经是示例的数值，但**首页、标签条、网格**仍是 Aura 的
排版：首页是 76px 圆角卡片墙，标签条只有一个 230px 固定宽的 chip，网格卡 112px 见方。
这一轮只做**构图**，没有动 token。

### 6.1 概览从卡片墙改成启动器 ✅

- `WorkspacePage`：`max-w-6xl / gap-9 / pt-9` → `max-w-[52rem] / gap-8 / pt-7`。
- 标题 28px → `text-title` 22px；搜索框 40rem、`h-10`、card 面 + 1px 内线。
- `LocationCard` 从 76px 两行卡（40px 色块 + 标题 + 路径）改成 **36px 行**：16px 图标、
  13px 名称、右侧 mono 10.5px 路径。项目卡（90px 两行）同样改成行：分支 chip 跟在名字
  后面，脏计数在右，时间在最后。
- 收藏 / 最近使用 / 空间全部变成单列行列表；`SectionHeader` 改成 11px/600 大写
  +0.08em 的 `--meta` 分组标题。
- Favorites / Space 两个页面跟着用同一套行（`flex-col gap-0.5`），不再用 2–3 列网格。

### 6.2 标签条与工具栏 ✅

- 标题条：条纹旁边补上 `dae` 字标；右侧加命令面板触发器（胶囊 28px，`<860px` 隐藏）。
  滚动箭头不再用 `invisible` 占 24px，没有溢出时直接不挂载。
- 标签 chip 去掉固定 `w-[230px]`，改成内容宽度 + `max-w-[230px]`；关闭键重新落在 chip 内。
- 工具栏：路径条 `flex-1` 加 `max-w-[44rem]`；git chip 移到路径条右侧（"地点 → 分支"读成
  一句话）；搜索/刷新/收藏/视图/面板用 `ml-auto` 成组贴到右边缘。
- 搜索框占位符从 `搜索 {{scope}}`（在 176px 里被硬切）改成 `筛选` / `Filter`。
- 修了面包屑折叠算法的真实 bug：初始宽度没把**末项**算进去，窄窗会算出一个放不下的组合，
  结果渲染成 `C: › ⋯ › \ › Documents`。现在窄窗得到 `C:\ › ⋯ › Documents`。

### 6.3 检查器成为默认布局 ✅

- `isPreviewOpen` 初始值改成"非分屏且窗口 > 1180px"。**根因不是初始值**：一个
  `[directoryPath, search.query]` 的 effect 每次导航都把检查器关掉，所以哪怕初始为 true，
  点进第一个文件夹就没了。现在只清 error，不动面板。
- 跨过 1180px 时自动收起，回到宽窗不自作主张重开（用户关过就是关过）。

### 6.4 侧栏磁盘与状态条 ✅

- `DiskItem`：两行名称 + 3px 条 + "剩余 X% · A / 共 B" 的三段结构 → **一行**
  （箭头、图标、名称、mono 余量）+ 3px 已用条。侧栏因此能同时放下磁盘和目录树。
- 状态条按示例重排：项目数 → 已选 → **仅 busy 时**出现的模式胶囊；右侧容量条改成画
  **已用**比例（原来画余量，空盘看起来像满盘）；删掉中间那条永远存在的分隔线。

### 6.5 网格与主按钮 ✅

- 网格：最小列宽 104/112/128 → 120/148/172，间距 8 → 12，行高 96/112/124 → 108/128/148。
- 主按钮改成示例 `.btn-primary`：前景色胶囊（暗色近白、亮色近黑），不再是强调色填充。
  强调色从此只用于选中、焦点、当前所在。

### 6.6 仍未做

- 网格卡仍然没有示例的 56px 缩略井（列表/网格的缩略图偏好是关的），只调了尺度。
- 终端抽屉的亮色仍没在真实 shell 里复查；预览里的 terminal 按钮没有真实 PTY。
- `bun run lint` 仍未跑绿（与第 3 轮相同）。
