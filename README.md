# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Design

Visual and interaction changes are judged against
[`docs/design-principles.md`](docs/design-principles.md): what the shell
borrows from macOS, Windows 11 and GNOME, the closed type/radius/motion
scales, and the tell list of treatments that make an application read as
AI-made. The token source of truth is `src/App.css`; the document explains
why the tokens are shaped the way they are.

## Tests

Rust tests use [cargo-nextest](https://nexte.st/). Run them from the project root:

```bash
bun run test:rust
```

## Lint

Run the regular Oxlint checks with Bun:

```bash
bun run lint
```

Type-aware linting is also available through `oxlint-tsgolint` and TypeScript 7:

```bash
bun run lint:type-aware
```

Auto-fixable issues can be applied with `bun run lint:fix` or
`bun run lint:type-aware:fix`.

## App icon

The app icon (a white goose on a pond-green squircle, orange beak) is designed
as a hand-written SVG:

- `src-tauri/icons/source/goose-icon.svg` — the 1024×1024 design source
- `src-tauri/icons/source/goose-preview.png` — 256px preview for quick review

To regenerate the whole platform icon set after editing the SVG:

```bash
bun run icons:icon
```

This renders `goose-master.png` from the SVG and runs `tauri icon`, which
rewrites `src-tauri/icons/**` (PNG sizes, `icon.icns` for macOS, `icon.ico`
for Windows, and the `android/`/`ios/` sets). The default is the classic
`.icns` look; a macOS 26 Tahoe Liquid Glass `.icon` (Icon Composer) can be
added later — Tauri's bundler supports it since 2.9.

## System accent

The shell's accent is the OS accent — the colour the user already picked in
Windows, macOS or their Linux desktop — because that is what makes one
cross-platform app read as native on all three without imitating any of them.

The two halves meet at two functions, named as a pair with `lib/theme.ts`:

```ts
// src/App.tsx
useEffect(() => watchSystemAccent(applySystemAccent), []);
```

- `src-tauri/src/system_accent/` — the platform half. `windows.rs` reads
  `HKCU\Software\Microsoft\Windows\DWM!AccentColor` and blocks on
  `RegNotifyChangeKeyValue` for changes; `macos.rs` reads
  `NSColor.controlAccentColor` and polls (macOS posts nothing when the accent
  moves); `linux.rs` reads `org.freedesktop.appearance accent-color` from the
  freedesktop settings portal and polls. Each reports `system-accent-changed`.
- `src/lib/system-accent.ts` — the CSS half. `watchSystemAccent()` subscribes
  and pulls once; `applySystemAccent()` is the whole write side.

The CSS contract is in the "accent seam" block of `src/App.css`. Everything
that means "the accent" — `--primary`, `--primary-foreground`, `--ring`,
`--selection`, the sidebar's accent roles — derives from two custom
properties, `--system-accent` and `--system-accent-ink`, and falls back to the
shipped indigo while they are unset. Category colours (`--folder`, the six
`--tone-*`) and the semantic set (`--destructive` / `--success` / `--warning`
/ `--info`) deliberately do not follow the accent.

`applySystemAccent` derives a WCAG-readable ink from the hue and rejects
anything it cannot parse, so a platform reader only supplies the colour. Pass
an ink explicitly where the platform already guarantees one (macOS resolves
`labelColor` against `controlAccentColor` and is better placed to pick it).

One consumer does not follow automatically: `src/features/terminal/terminal-palette.ts`
hands xterm plain hex values, so it cannot read a CSS variable. Call
`getSystemAccent()` when rebuilding the palette and subscribe to the
`app-system-accent-change` event to rebuild it again.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
