# NOTES — Notion 重设计

## 关键事实
- 样式集中在 `src/App.css`（2188 行）：@theme inline（Tailwind token）→ :root/.dark 语义值 → 处理层（aurora/glass/frost）→ base。
- TSX 用 token 类名（bg-card / rounded-md / shadow-ambient-* …），改 token 全局生效。
- 颜色本来就是 Notion 暖中性（#f7f7f5 / #37352f / #f1f1ef / #e9e9e7），冷感来自：12px 大圆角、accent 渐变发光（glass-active / tab-chip-active）、aurora 光晕、紧字距、弹簧过冲曲线、胶囊搜索框。
- Tailwind v4 translate 用独立 `translate` 属性，keyframes 的 `transform` 与 dialog 居中 -translate-* 不冲突 → 可安全在 float-in 里加 scale。

## 决策
- 圆角基 12px→8px，梯子 3/4/6/8/12 + panel 10（Notion 式安静圆角）。
- 动效：曲线去过冲，改软 ease-out 滑行族；hover 用 CSS ease。时长已足够快（90/140/190/280），不动。
- 阴影：接触层 + 近层 + 宽软层三层配方（Notion 签名式），低透明度宽散。
- glass-active：去掉 accent 渐变+发光 → 暖灰 wash（accent-foreground 7%）+1px 内环；active=暖灰加底、字加重（Notion 侧栏行为）。
- tab-chip-active：去掉渐变+accent glow → bg-card 平板 + 发丝高光 + 呢喃阴影。
- content-panel：去掉 ambient-sm 大阴影 → 呢喃阴影（纸放在桌上）。
- aurora-frame/nav：accent 光晕 → 极弱中性暖灰纸面晕染。
- 字距：display -0.021→-0.012em，title -0.006→-0.002em，lead 不动。
- 不动：--popover-frost 玻璃对（有逐条对比度测量，90/96% 白菜单本就近似纸面）、
  字阶尺寸/行高（行高被 28/34/42px 测量依赖）、accent seam 机制、z-index、focus 政策、语义色。

## 完成状态
- [x] token 层（圆角/阴影/动效曲线/字距 + float-in 加 scale(0.985)）
- [x] 处理层（aurora-frame/nav、glass-active、tab-chip-active、content-panel）
- [x] TSX：directory-search 胶囊→rounded-md；button hover 去 accent 辉光
- [x] build 验证：`bun run build`（tsc + vite）✓ built in 3.91s（token 层后、收尾后各跑一次）

## 遗留说明
- 组件注释里的 "Linear input/kbd/button" 是解剖学出处说明，描述现有结构未失实，未改动其布局（受 28/34/42px 测量约束）。
- chunk>500kB 警告是构建前就存在的，与本次无关。
