# dae

An aesthetic desktop file manager with an integrated terminal. The shell is
Rust ([Tauri 2](https://tauri.app/)) and the interface is React 19 +
TypeScript in Vite. It is meant to read as native on Windows, macOS and Linux
without imitating any of them.

- **Tabs and windows.** Tabs drag between windows and tear off into their own;
  a pool of pre-warmed windows makes a tear-off look immediate.
- **More than the local disk.** Mounted volumes, WSL distros, the trash,
  SSH/SFTP and SMB connections, and archives (`zip`, 7z) browsed in place.
- **A terminal in the frame.** An xterm panel beside the listing.
- **Views for the job.** List, grid, column and split views, previews, content
  search, bulk rename, per-folder git status.
- **System-native chrome.** The OS accent colour retints the shell, and window
  material (Mica / Acrylic, macOS vibrancy) shows through where the desktop
  has one.
- **Yours to configure.** A human-editable `settings.toml`, rebindable
  shortcuts, `dae://` deep links, and registering dae as the default file
  manager. Interface in English and Simplified Chinese.

## Prerequisites

- [Bun](https://bun.sh/) — every frontend script runs through it.
- A stable Rust toolchain. The crate is edition 2024, so 1.85 or newer.
- Platform build dependencies, the same set the release workflow installs:
  - **Linux** — `build-essential curl wget file pkg-config cmake`, plus
    `libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev
    libgtk-3-dev librsvg2-dev libxdo-dev libssl-dev libdbus-1-dev zlib1g-dev
    rpm`.
  - **Windows** — MSVC build tools and the WebView2 runtime (built into
    Windows 11). [`winapp`](https://github.com/microsoft/winappCli) is needed
    only for the MSIX packaging script.
  - **macOS** — the Xcode command line tools.
- Node 24 only for the release tooling (`scripts/sync-msix-version.mjs`, the
  version check); CI pins it rather than trusting the runner image.

## Getting started

```bash
bun install
bun run tauri dev
```

`tauri dev` starts Vite from `beforeDevCommand` and builds the Rust side, which
— in a debug build — regenerates the TypeScript bindings into
`src/bindings.ts` (`src-tauri/src/lib.rs`). That file is gitignored and is
imported by the frontend, so anything that type-checks the frontend without
building Rust first needs it explicitly:

```bash
bun run bindings:export   # regenerate src/bindings.ts alone
```

`bun run build` does this for you: `beforeBuildCommand` is
`bun run bindings:export && bun run build`.

## Scripts

| Command | What it does |
| --- | --- |
| `bun run tauri dev` | Vite + the Rust app, with HMR; refreshes `src/bindings.ts` |
| `bun run bindings:export` | Regenerate `src/bindings.ts` from the Rust commands |
| `bun run dev` | Vite alone on `http://localhost:1420` |
| `bun run build` | `tsc`, then Vite, into `dist/` |
| `bun run tauri:build:release` | Release bundle, with `src-tauri/tauri.release.conf.json` |
| `bun run pack:msix` | Windows: release exe plus a development certificate, packed as MSIX |
| `bun run tauri:dev:withidentity` | Windows: register the app with its identity (`winapp run`) |
| `bun run lint` / `lint:type-aware` | Oxlint over `src` and `vite.config.ts`; add `:fix` to apply |
| `bun run test:rust:setup` / `test:rust` | cargo-nextest, see [Tests](#tests) |
| `bun run icons:icon` | Re-render the platform icon set, see [App icon](#app-icon) |

Releases are cut by pushing a `v*` tag, which runs
`.github/workflows/release.yml` for Windows, Linux and Apple Silicon. The tag
must match the version in `src-tauri/tauri.conf.json` **and**
`package.json`; CI fails the build otherwise. `main` is additionally
type-checked for macOS on every push, since `#[cfg(target_os = "macos")]` code
is invisible on the other platforms.

## Where things live

- `src-tauri/src/` — Rust. `file_system/` holds the backends (local, `sftp.rs`,
  `smb.rs`, `archive.rs`) behind one VFS; also `terminal.rs` (the PTY),
  `system_accent/`, `system_theme/`, `window_material/`, `tab_windows.rs`,
  `settings.rs`, `default_manager.rs`.
- `src/features/` — React by area: `explorer/`, `sidebar/`, `workspace/`,
  `terminal/`, `settings/`, `shell-commands/`.
- `src/lib/` — the seams between the two halves: `system-accent.ts`,
  `theme.ts`, `window-material.ts`, `window-focus.ts`.

## Design

Visual and interaction changes are judged against
[`docs/design-v3.md`](docs/design-v3.md) (the "Aura" language): the two
planes the window is drawn as — canvas and the one floating sheet — the type
ramp, the state vocabulary for hover, selection and focus, and the motion
curves. The token source of truth is `src/App.css`; the document explains why
the tokens are shaped the way they are.
[`docs/design-v2.md`](docs/design-v2.md) (Atlas) and
[`docs/design-principles.md`](docs/design-principles.md) (Quiet Desk) are kept
as the historical record of the directions Aura replaced.

Performance work is judged against
[`docs/performance-opportunities.md`](docs/performance-opportunities.md): what is
already measured and done, and the remaining opportunities with the harness and
the numbers behind each one. Measure before changing anything — `docs/decisions/`
holds the experiments that were run and declined, and the conditions that would
reopen them.

## System accent

The shell's accent is the OS accent — the colour the user already picked in
Windows, macOS or their Linux desktop — because that is what makes one
cross-platform app read as native on all three without imitating any of them.

The two halves meet at a pair of functions, `applySystemAccent` /
`watchSystemAccent`, mirroring the light/dark pair in `src/lib/theme.ts`
(`applySystemTheme` / `watchSystemTheme`):

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
shipped teal (`--accent-default`: `#1f7a72` light, `#6bbcb6` dark) while they
are unset. Category colours (`--folder`, the six `--tone-*`) and the semantic
set (`--destructive` / `--success` / `--warning` / `--info`) deliberately do
not follow the accent.

`applySystemAccent` derives a WCAG-readable ink from the hue and rejects
anything it cannot parse, so a platform reader only supplies the colour. Pass
an ink explicitly where the platform already guarantees one (macOS resolves
`labelColor` against `controlAccentColor` and is better placed to pick it).

The integrated terminal deliberately does not follow the accent.
`src/features/terminal/terminal-palette.ts` hands xterm plain hex values, and
its ANSI 16-colour table is accent-independent by design: ANSI blue means
*blue* the way `--success` means *success*, so a symlink `ls --color` paints
stays that colour whatever hue the window is tinted to. Only the terminal's
own cursor and selection read `--primary` from CSS and so follow the accent
live.

## Decisions

`docs/decisions/` is the decision log (written in Chinese): why the tab
tear-off went through a pre-warmed window pool, why the browser-style webview
`reparent` route was rejected, and why the gtk/gio stack is pinned at 0.18 —
each with the measurements behind it, plus the conditions that would reopen the
question. Start at [`docs/decisions/README.md`](docs/decisions/README.md); the
archived PoC it links to is not part of the build.

## Tests

Rust tests use [cargo-nextest](https://nexte.st/). Install it once, then run
the suite:

```bash
bun run test:rust:setup
bun run test:rust
```

Both scripts are Windows-only: `test:rust` invokes
`src-tauri\target\bin\cargo-nextest.exe`, and `test:rust:setup` installs it
there with `cargo binstall`. On macOS and Linux, install nextest your usual way
and call it directly:

```bash
cargo nextest run --manifest-path src-tauri/Cargo.toml
```

There is no frontend test runner configured; the frontend is held by `tsc`
(`bun run build`) and the lint scripts below.

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

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
