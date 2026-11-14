# 列表文件字形改走 C++ 栅格缓存

> 日期：2026-11-14 · 状态：已实施（Windows 本机验证，DPR 1.5；macOS/Linux 未跑）
> 相关：`qml/FileIcon.qml`、`crates/dae-qt/cpp/image_provider.cpp`、`crates/dae-qt/src/icons.rs`、
> `docs/ui-redesign/proposal.md` §6.3.2、§12.3、§13.2

## 问题

P2 的行解剖把文件字形固定成 18 dp：所有文件共用一个页形轮廓，内部按族放一个颜色标记，
文件夹是文件夹轮廓加族色条（proposal §6.3.2）。在这之前，列表行用的是 `DriftSheet.qml`：
每个 delegate 一个 `Canvas`，在 `onPaint` 里按 category 走一遍 JS 路径。

`ListView.reuseItems` 用的正是这套 delegate，于是每次复用都要重设尺寸与颜色、重新走一遍
路径并重绘；滚动时这些成本落在 GUI 线程上。§12.3 已经把结论写成「图标走栅格缓存，
delegate 内不再有 Canvas」，缺的是具体落到哪一层。

## 决定

**字形由 C++ 的 `QQuickImageProvider` 一次栅格化，QML 行内只留一个 `Image`。**

- provider 名 `fileglyph`，URL 形如
  `image://fileglyph/<family>?size=<像素边>&ink=<rrggbb>&fc=<rrggbb>`。
  `family` 就是模型 `category` 角色（folder、code、image、video、audio、archive、
  spreadsheet、presentation、pdf、document、text、font、executable、link、file、other）。
- 几何在 20 单位网格上，用 `QPainter`/`QPainterPath` 直接画：页形 `3.6,1.4 → 16.6,18.6`
  加折角，文件夹 `1.6,5.4 → 18.4,16.6` 加体内族色条；描边 1.3 单位、圆角连接。
  没有 SVG、没有第三方栅格库。
- 缓存是 `QCache<QString, QImage>`，键为 `family|edge|ink|tint`，`maxCost = 64`；
  `QMutex` 保护，因为 provider 可能被渲染线程调用。请求边夹在 12–128 像素。
- `qml/FileIcon.qml` 只是薄壳：`pixelEdge = max(12, round(edge × Screen.devicePixelRatio))`，
  把 `sourceSize` 设成同一个像素边，`fillMode: Stretch`（`Pad` 会把 27 px 的栅格按 27 dp
  画进 18 dp 盒，四周被裁掉）。
- 颜色不作为字形数据的一部分：`ink`（文件夹 `inkSecondary`、文件 `inkTertiary`）与
  `fc`（`Theme.fileColor(category)`）都由 QML 传入，所以换主题只是产生新键，旧条目由
  `maxCost` 淘汰；调色板仍然只有 `Theme.qml` 一处。

## 为什么不选别的

- **继续每行 Canvas**：行复用与滚动都会重绘；`reuseItems` 的意义是把 delegate 的构建
  成本摊薄，而不是保留一份每行都要执行的绘制脚本。
- **QtSvg + SVG 资源**：要给构建链加 QtSvg 模块，并把 16 个族做成 16 份 SVG；
  这些字形总共十来条路径，收益只是「可以改文件」。
- **tiny-skia / resvg**：引入新的 Rust 依赖，本机 cargo 缓存里没有，离线构建会直接失败；
  换来的只是另一门绘图 API。
- **预生成 PNG 资产**：主题 × DPR × 族 × 尺寸的笛卡尔积，改一次主题色就要重出资产；
  而且 P0 的 token 是想改就改的。

## 边界

- 缓存键不含 DPR，`size` 已是像素边；1.25 / 1.5 / 2.0 的机器各自一份。
- 64 项够一屏（16 族 × 最多 4 种颜色/尺寸组合）；列表通常只用一种尺寸。
- 本阶段只迁移**列表行**。网格、分栏、冲突对话框与预览的 `DriftSheet` 不动，
  界面里的 Solar 标记仍由 `Glyph.qml` 的 Canvas 渲染；它们各自的阶段再决定是否搬过来。

## 验证

- `cargo test -p dae-qt --lib` 214 项通过；`cargo build -p dae-app --bin dae` 通过；
  `qmllint` 无新增告警（剩余的是既有的 Unqualified access）。
- 1280×800、DPR 1.5 截图：页形与折角完整，文件夹色条在体内、不穿出上沿；
  文件名主体 13 px `ink`、扩展名 12 px `inkTertiary` 分层可见；
  长文件名中段省略后扩展名仍在（`another-…markdown.markdown`）。
- 未测：12 万行下 provider 的实际纹理数量与 PSS 增量（`bench/qt-list-spike` 的 delegate
  不是列表行，量不到这一层）；macOS/Linux 未验证。
