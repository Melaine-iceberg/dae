# Notion 设计语言重设计（去 Linear 冷硬感）

应用：dae — Tauri 文件管理器。样式几乎全部集中在 `src/App.css`（token + 组件层），
TSX 用 Tailwind 工具类引用 token → **改 token 层即可全局生效**。

## 翻译表：Linear 冷硬感 → Notion 温暖感
| 现状（Linear 系） | 目标（Notion 系） |
|---|---|
| 12px 大圆角 + 胶囊搜索框 | 小圆角 3/4/6/8/12，仅圆点/进度条用胶囊 |
| glass-active 强调色渐变发光 | 暖灰平面填充（选中=灰底加重，无发光） |
| aurora 光晕 / 渐变装饰 | 极弱暖色纸面晕染或纯平 |
| 内容面板厚阴影+内高光 | 纸面：发丝边 + 呢喃阴影 |
| 浮层磨砂玻璃 90% | 不透明暖白纸面 + Notion 式三层软阴影 |
| 紧字距（-0.02em 级） | 近零字距，暖而放松 |
| 快速临界阻尼弹簧 | 轻柔 fade+微升/微缩落定，ease 曲线，100–190ms |
| 阴影冷硬（短而紧） | 宽散、低透明度、暖色阴影 |

## 保持不动（谨慎项）
- 颜色体系：已是 Notion 暖中性（#f7f7f5 / #37352f / #f1f1ef / #e9e9e7），仅微调阴影透明度
- 七级字阶尺寸与行高（行高被 28/34/42px 行高测量值依赖）
- `--selection` 选中水位、accent seam 机制、z-index 金字塔、focus-visible 政策
- 类名与 DOM 结构（避免破坏 TSX）

## 步骤
1. [ ] token 层：圆角梯、阴影配方、动效曲线/时长、字距、float-in/rise-in 关键帧
2. [ ] 处理层：glass-active / tab-chip-active / content-panel / aurora-* / floating-frost
3. [ ] TSX：directory-search 胶囊 → 小圆角；检查其余胶囊/冷硬点
4. [ ] `npm run build`（tsc + vite）验证
