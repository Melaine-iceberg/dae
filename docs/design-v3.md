# dae — Aura

> **⚠️ 方向待改写（2026-10-20）.** 当前代码已按
> [`redesign/reference-raycast.html`](redesign/reference-raycast.html) 对齐，
> 与本文的几处数字**相反**（外壳拍平、44/54/34/232、13px 行文字、8px 圆角基数）。
> 差异清单见 [`redesign/handoff.md`](redesign/handoff.md) §3。
> 本文暂作历史保留，等示例对齐收尾后整篇重写。

The visual and interaction language the shell ships today. It replaces the
Atlas language described in [`design-v2.md`](design-v2.md); that document is
kept only as history.

Aura has one idea behind every decision: **a file manager is a quiet room for
content**. Chrome recedes, the listing carries the colour, and every state is
legible without an outline.

## The two planes

The window is drawn as **canvas** and **sheet**, not as three nested rungs.

| role | token | what it is |
| --- | --- | --- |
| canvas | `--background` / `--chrome` | the window itself: the tab strip, the navigation rail, the gutters |
| sheet | `--card` | the one floating plane the content lives on (`.content-panel`) |
| float | `--popover` | menus, the command palette, dialogs, the action island |

The rail is **not** a second panel. It sits directly on the canvas and is
separated from the sheet by 10px of canvas plus one soft shadow — no hairline,
no second fill. That single decision is what makes the window read as an app
rather than as a grid of boxes.

Depth is a 20px shadow under the sheet at 12% and a 1px light bevel along its
top edge. Nothing else floats; a menu lifts with one wider shadow.

## Type

Six steps, and nothing outside them:

| step | size | job |
| --- | --- | --- |
| `title` | 22px / 620 | dialog and section titles |
| `lead` | 17px | the palette input |
| `body` | 14.5px | rows, controls, the workhorse |
| `caption` | 13px | secondary metadata, descriptions |
| `micro` | 12px | dense chrome, timestamps, counts |
| `nano` | 11px | keycaps and badge letters |

Body moved up from 13px. A listing is read at arm's length, and the previous
ramp made every row look like metadata. Hierarchy is carried by weight and
tracking rather than by a second size: only `title` and the page heading (28px,
set at the call site) carry their own weight and negative tracking.

Page headings on the workspace surfaces are 28px/600 at `-0.03em`; they are the
one place a display step is spent, because those surfaces are documents with a
name rather than a tool reporting state.

## Shape

A 12px base radius, four derived steps, and one plane step:

```
8   inline code, keycaps
9   buttons and icon plates
12  inputs, list rows, chips
16  menus, quick-access tiles, dialogs, the command palette
18  the content sheet
```

A capsule is reserved for two shapes that genuinely have no corners: status
dots and the disk capacity bar.

## Space

- Tab strip 50px, toolbar 56px.
- List rows 38px (30 / 38 / 48 across the three densities).
- Navigation rows and menu items 32–36px.
- 10px gutter between the rail and the sheet; 10px of canvas around the sheet.

## State vocabulary

Selection is the accent, never a grey:

- **Selection** — `--selection`, the accent at 15% light / 26% dark, plus a
  1px accent edge (`--selection-edge`). It reads as *chosen* rather than as
  hovered, and it follows the OS accent because the accent is a seam
  (`--system-accent`), not a constant.
- **Hover** — one neutral step (`--accent`), 5% current-colour wash, 150ms.
- **Current place** — `.row-active`: an 11% accent plate with accent ink. Used
  by the rail's active row, the palette's active result and the settings nav.
- **Focus** — a 2px ring at 40% on controls, a 1px inset hairline on rows
  (an outer ring would be clipped by the virtual scroller).
- **Drop target** — a 9% accent wash with a 2px inset accent ring.

Nothing scales on hover. Buttons compress to 0.98 on press; a plate that
bounces is a component-library demo, not desktop chrome.

## Motion

Two curves and three clocks:

- `--ease-standard` (in-out quad) for state changes: colour, fill, ink.
- `--ease-spring-fast` (out) for things that settle into place: the tab plate,
  a chevron, a pressed key.
- 100 / 150 / 200 ms. Exits use `--ease-standard-accelerate`.

The tab plate travels between tabs in 200ms on the settle curve. Floating
layers fade in 150ms with a 3px rise; nothing slides from off-screen.

## Surfaces at a glance

- **Tab strip** — bare labels on the canvas; the current tab sits on a raised
  plate (`.tab-pill`) that slides. No dividers between tabs.
- **Rail** — 264px, 36px rows, 11px section captions, platform-toned place
  icons, one accent-tinted row for where you are.
- **Toolbar** — one row: rail toggle, travel cluster, path field (the spine),
  search capsule, folder actions, view/filter cluster, panel toggles. Fields
  are full-strength `--secondary` capsules; focus turns them into paper with a
  ring.
- **Listing** — 38px rows on a 10px radius, 18px file-type glyphs, full-width
  columns with a sticky 36px header. Selected rows take the accent wash and a
  hairline edge; hover is a neutral wash.
- **Empty states** — one glyph on a recessed plate, one title, one line, one
  affordance.
- **Action island** — the selection's contextual bar floats 20px above the
  bottom edge as a blurred, shadowed capsule.
- **Object pages** — Overview, Recents, Favorites, Trash and Spaces share one
  page shell: a 28px heading with a one-line subtitle, a search affordance
  bound to Mod+K, then sections of quick-access tiles and rows at 1152px.

## What this language refuses

- Hairlines as the primary separator. Tones and space separate; a line is used
  only where a scroll region needs an edge.
- A second accent. Folder yellow is the only warm object on screen; semantic
  hues (success, warning, info, destructive) stay semantic and never follow
  the OS accent.
- Borders on selection. The fill is the state; an outline on top makes a row
  look like a focused control.
- Glass on content. Only the action island is translucent, and only because it
  floats over the listing rather than holding it.

## Tokens

`src/App.css` is the source of truth: the `:root` and `.dark` blocks carry the
palette (and are read back by the boot-splash plugin in `vite.config.ts`), the
`@theme inline` block carries the type, shape and motion scales, and the Aura
layer at the end of the file restates the three structural treatments
(`.content-panel`, `.tab-pill`, `.row-active`).
