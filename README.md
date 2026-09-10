# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Tests

Rust tests use [cargo-nextest](https://nexte.st/). Run them from the project root:

```bash
bun run test:rust
```

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

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
