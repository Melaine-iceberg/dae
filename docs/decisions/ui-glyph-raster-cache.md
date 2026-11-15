# 界面字形改走 C++ 栅格缓存

> 日期：2026-11-15 · 状态：已实施（Windows 本机验证，DPR 1；macOS/Linux 未跑）
> 相关：`qml/Glyph.qml`、`qml/solar-marks.json`、`crates/dae-qt/cpp/solar_marks.cpp`、
> `crates/dae-qt/cpp/image_provider.cpp`、`crates/dae-qt/src/icons.rs`、
> `docs/ui-redesign/proposal.md` §6.3.1、§12.3、§12.7

## 问题

`Glyph.qml` 每个标记一个 `Canvas.Immediate`：屏幕上每个侧栏目的地、菜单项、工具栏按钮、
标签页都是独立的一张栅格缓冲；分栏视图的 delegate 里每行还有一个 chevron 的 `Canvas`。
悬停或选中改 `tint` 会 `requestPaint`，`onPaint` 里再用正则把路径数据重新分词一遍。

两处都与 `proposal.md` 冲突：

- §12.3 第 3 条：**禁止在 delegate 里创建 `Canvas`**——分栏的 chevron 正是这种；
- §12.7：图标首次渲染「每个 (kind, size, DPR) 只栅格一次」，而 `Canvas` 是每次
  tint/尺寸变化都重绘。

`file-glyph-raster-cache.md` 已经把**文件**字形搬到了 `image://fileglyph`，
界面字形是同一套问题的最后一处。

## 决定

**界面字形也由 C++ 的 `QQuickImageProvider` 一次栅格化，`Glyph.qml` 只剩一个 `Image`。**

- provider 名 `glyph`，URL 形如
  `image://glyph/<kind>?size=<像素边>&dp=<边长>&ink=<aarrggbb>&wash=<0..1>`。
- 路径数据从 `qml/SolarMarks.js`（`pragma library`）搬到 `qml/solar-marks.json`：
  65 个 `kind` → `[{ d, w, f?, s? }]`，外加 `_source`（来源、上游映射表）与
  `_licence`（MIT 全文）两个元数据键。数据一字未改，只有包装从 JS 对象字面量变成 JSON，
  因为 provider 在 C++ 侧读它。
- SVG 语法解析与绘制在 `crates/dae-qt/cpp/solar_marks.cpp`：`QPainterPath` 直接构造，
  `A` 圆弧按端点→圆心转换后切成 ≤90° 三次贝塞尔（沿用旧 `trace()`/`arc()` 的算法），
  栅格用 `QPainter`，不引入 SVG 模块与第三方栅格库。
- 缓存是 `QCache<QString, QImage>`，键为 `kind|edge|dp|ink|wash`，`maxCost = 128`；
  `QMutex` 保护，因为 provider 可能被渲染线程调用。请求边夹在 8–128 像素。
- `qml/Glyph.qml` 只是薄壳：`pixelEdge = max(8, round(width × Screen.devicePixelRatio))`，
  `dp` 传 item 的边长，`sourceSize` 设成同一个像素边，`fillMode: Stretch`
  （`Pad` 会把 27 px 的栅格按 27 dp 画进 16 dp 盒）。`kind` 为空时 `source` 也为空——
  命令面板里没有标记的行不该换一个透明栅格的缓存条目。
- 颜色仍在 QML：`ink` 是 8 位 ARGB 十六进制（去掉 `#`，否则 URL 会在那里断成 fragment），
  所以顶栏 45 % 的三色灯仍是半透明；`wash` 是剪影填充比例。换主题只是换键。

## 渲染规则没有变

| 规则 | 实现 |
|---|---|
| 描边不随尺寸缩放 | `lineWidth = w × max(1, dp/16) × 24 / dp`（网格单位；`dp` 由 QML 传入，因为像素边是取整后的，除回去会漂） |
| 剪影 = 淡填充 + 描边 | `s` 标记先以 `tint × wash`（默认 0.3）填充再描边 |
| 单色 | 填充色与描边色都是 `tint`，数据里没有颜色 |
| 24 网格 | 同一份几何按 `edge / 24` 缩放，8 dp 与 28 dp 共用一张表 |

## 为什么不选别的

- **`Shape` + `PathSvg`**：proposal §12.3 已把它列为「需实测」，`PathSvg` 对 `A` 圆弧与
  自交填充规则没有逐命令的官方承诺；comparison 的代价是每个标记一个场景图节点，而这里
  要的只是「二十行共用一张纹理」。
- **`VectorImage`（6.8+）**：没有官方染色 API，官方文档也建议固定尺寸图标走栅格化。
- **QtSvg 模块**：要把 65 份路径包成 SVG 文档，并给构建链（本机 + 三个 CI 工作流）加一个
  Qt 模块；收益只是「可以用工具看单个图标」。
- **继续每图标一个 Canvas**：正是 §12.3 要清掉的东西，且悬停换色会重绘。

## 边界

- `_source` / `_licence` 不是标记：provider 按 `kind` 取数组，取不到就是空。
- 缓存按 `tint` 分键：悬停、选中、禁用各是一种颜色，颜色集合来自 `Theme`，是有限集；
  128 项在实测中只用到 17 项（一次冷启动的界面）。
- 仓库里还剩两个 `Canvas`：`FileList.qml` 的列头排序箭头与 `MediaPreview.qml` 的播放三角。
  两者都只有一个实例、都不在 delegate 里，§12.3 不覆盖，未动。

## 验证

- `cargo build -p dae-app` 通过（C++ 桥与 QML 资源）。
- **A/B 对比**：把 `HEAD` 的 `Glyph.qml`（Canvas）与新的 provider 栅格放进同一个
  10 kind × 4 尺寸（8/12/16/28 dp）网格，同色同背景抓图比较覆盖率：
  40 格里 36 格**逐像素相同**（IoU 1.000），`search` 的细圆环 0.78–0.87，但包围盒与
  着色像素数完全一致（差的只是 1 px 圆环上抗锯齿的边界判据，不是几何）。
- 应用内 `DAE_GLYPH_PERF=1` 跑真实界面：17 个标记各栅格一次，单次 0.05–0.55 ms，
  无重复栅格（缓存命中不打印）。
- 未测：macOS/Linux 渲染；12 万行滚动的帧时间（本次只改图标路径，列表 delegate 的
  `Canvas` 在 file-glyph 那一步就已经清掉）。
