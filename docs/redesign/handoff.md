# Raycast 示例对齐 —— 交接文档

状态：**第 1、2 轮已提交**（令牌层 / 外壳 / 列表 / 侧栏 / 网格 / 复选框面板 / 命令面板 / 动作岛 / 终端抽屉 / 图标线性化）。
本文给下一个会话用：环境怎么跑、已经改了什么、还剩什么、每项的验收标准。

参考文件（已入库）：[`reference-raycast.html`](reference-raycast.html) —— 原文件在
`C:\Users\Mt342\Desktop\dae-explorer.html`，本文所有"示例"数值都指它 `<style>` 里的 token 与组件规则。

**方向已定：示例优先。** `docs/design-v3.md`（Aura）与本方向冲突的数字见 §3，收尾后整篇重写。

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

## 2. 待办（按收益排序）

每一项都写清：改哪里、示例的规则、怎么验收。

### 2.1 空状态 + 骨架屏

- 文件：`src/components/ui/empty.tsx`、`skeleton.tsx`、`file-list.tsx`（`FileListSkeleton`）、`file-grid-view.tsx`、`explorer-errors.ts`
- 示例：`.empty` 居中、`.plate` 52×52 / 12px 圆角 / `fg 5%` 填充、`h2` 15px/600、`p` 12.5px / 34ch / 1.6；`.skel i` 26px 行高 + 6px 圆角 + `fg 4%`
- 验收：截空目录、搜索无结果、加载中三态

### 2.2 Miller 列视图

- 文件：`file-column-view.tsx`（`--width-column-view: 15rem` 已存在）
- 示例没画列视图 → 复用 `lhead` / `row` 同一套行文法；列间 hairline 用 `--border`
- 验收：截图 + 键盘左右切换

### 2.3 检查器 / 预览面板

- 文件：`entry-preview.tsx`（`w-preview` 已是 300px）
- 示例 `.insp`：`padding 16px`、`insp-cap` 11px 大写 `--meta`、`.pv-body` 12px 圆角 + inset hairline、`.pv-code` mono 11px / 1.7 / 行号列宽 20px、`.meta-row` = 74px 标签 + 11.5px 值 + `border-soft` 分隔、`.tag` 22px 高、`.insp-actions` 纵向 `gap 6px`（当前是两个并排半宽按钮）
- 验收：选中图片 / 代码 / 文本 / 多选 四种情况

### 2.4 终端配色

- 文件：`terminal-palette.ts`、`terminal-panel.tsx`
- 示例 `.term-body`：mono 12px / 1.75；prompt `--blue`、成功绿、路径 dim、警告黄；`.caret` 7×13 闪烁；drawer 高度 188px（当前 `height` 由用户拖拽，确认默认值）
- 验收：开终端跑 `ls` / `git status` 截图

### 2.5 右键菜单 / 对话框逐像素

- 文件：`src/components/ui/context-menu.tsx`、`dropdown-menu.tsx`、`dialog.tsx`、`select.tsx`、`explorer-dialogs.tsx`、`properties-dialog.tsx`
- 示例 `.ctx`：`min-width 220px`、`padding 5px`、行 30px / 12.5px、图标 gap 10px、`hr` `5px 6px`、危险项红色 16% 底
- 示例 `.sheet`：440px 宽、`padding 20px`、h2 18px、`.row-line` 9px 上下 + `border-soft`
- 验收：菜单、删除确认、属性、设置四处

### 2.6 键帽与焦点环统一

- 文件：`src/components/ui/kbd.tsx` + 各处 `focus-visible:*`
- 示例 `.kbd`：19px 高 / 4px 圆角 / 双层阴影（`rgba(255,255,255,.09) 0 1px 0 inset`, `rgba(0,0,0,.5) 0 1px 1.5px`, `0 0 0 1px rgba(255,255,255,.07)`）
- 焦点：示例 `--focus-ring: 0 0 0 3px hsla(202,100%,67%,.35)`；当前控件是 `ring-2 ring-ring/40`、行是 1px inset。**统一成 3px/35% + 行内 1px**，两档写进 App.css 注释

### 2.7 搜索胶囊

- 文件：`directory-search.tsx`
- 示例 `.searchmini`：空闲 176px / 聚焦 256px / `<1360px` 折成放大镜
- 验收：1440、1180、900 三个宽度截图

### 2.8 窄窗让位顺序复核

- `TOOLBAR_OVERFLOW_*_CLASS`：收藏 1080 → 分屏 900 → git 1180
- 验收：1180 / 1080 / 900 三张截图，确认没有控件被裁到够不着

### 2.9 文档收尾

- 重写 `docs/design-v3.md`（见 §3 的差异表），或者把它合并进 `design-v2.md` 后删掉
- `docs/design-principles.md`（v1）里 §"图标：line-duotone + 58% 强调色" 已经失效，标注或删除

---

## 3. 与 `design-v3.md`（Aura）的冲突清单

代码目前按**示例**走；下表每行都需要在新一轮里改写文档（或回退代码，若届时改主意）。

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

1. `bun run build` 绿（tsc + 打包）
2. 截图：列表 / 网格 / 选中 / 动作岛 / 检查器 / 命令面板 / 右键菜单 / 空目录
3. 宽度：1440 / 1180 / 900
4. 亮色过一遍：`document.documentElement.classList.toggle('dark')`
5. 关掉 `dae.exe` 与 dev server
