# dae

An aesthetic desktop file manager with an integrated terminal. The shipping
shell is [Electron](https://www.electronjs.org/) with a Rust N-API core
(`native/` on top of `dae-core`), and the interface is React 19 + TypeScript in
Vite. It is meant to read as native on Windows, macOS and Linux without
imitating any of them.

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

The original Tauri 2 shell still lives in `src-tauri/` as the migration
reference and the typed command schema behind `src/bindings.ts`. Releases are
built from the Electron shell; everything that ships is under `electron/`,
`native/` and `src/`.

## Prerequisites

- [Bun](https://bun.sh/) — every frontend script runs through it.
- Node 24 — Electron tooling, `electron-builder` and the release scripts.
- A stable Rust toolchain. The crate is edition 2024, so 1.85 or newer.
- Platform build dependencies for the addon:
  - **Linux** — `build-essential pkg-config cmake zlib1g-dev libssl-dev
    libglib2.0-dev libgtk-3-dev` (the addon binds GTK/GIO directly and builds
    the bundled libgit2; Electron ships its own Chromium).
  - **Windows** — MSVC build tools. [`winapp`](https://github.com/microsoft/winappCli)
    is needed only for MSIX packaging.
  - **macOS** — the Xcode command line tools.
- `bun install` downloads the Electron distribution into
  `node_modules/electron/dist`; `electron-builder` reuses it instead of
  fetching a second copy.

`src/bindings.ts` is checked in, so a clean checkout can build the Electron
app without compiling the Tauri shell first.

## Getting started

```bash
bun install
bun run electron:dev
```

`electron:dev` builds the native addon, builds the renderer and launches
Electron. For renderer HMR, run Vite and Electron separately:

```bash
bun run dev                                            # Vite on :1420
node scripts/native-build.mjs --debug                  # addon, debug profile
electron electron/main.cjs --dev-server=http://localhost:1420
```

The addon is copied to `native/dae_core_native.node`, which is the path both
the main process and the core utility process load. Rebuild it after touching
`native/` or `src-tauri/crates/dae-core/`.

`src/bindings.ts` is generated from the Tauri command registry whenever the
typed command surface changes:

```bash
bun run bindings:export   # regenerate src/bindings.ts
bun run bindings:check    # regenerate and fail if the checked-in file drifted
```

The file is committed so `electron:dev`, `electron:pack` and CI do not need
the Tauri shell. The macOS check workflow regenerates it on every push and
fails on drift.

## Scripts

| Command | What it does |
| --- | --- |
| `bun run electron:dev` | Native addon + renderer, then Electron |
| `bun run electron:pack` | Unpacked app in `dist-electron/` for the host platform |
| `bun run electron:dist` | Installer for the host platform, with updater metadata |
| `bun run pack:msix` | Windows: unpacked app staged and packed as MSIX |
| `bun run native:build` | Release addon → `native/dae_core_native.node` |
| `bun run bindings:export` / `bindings:check` | Regenerate / verify `src/bindings.ts` |
| `bun run test:native` | `cargo test` for the Electron addon |
| `bun run dev` | Vite alone on `http://localhost:1420` |
| `bun run build` | `tsc`, then Vite, into `dist/` |
| `bun run lint` / `lint:type-aware` | Oxlint over `src` and `vite.config.ts`; add `:fix` to apply |
| `bun run icons:icon` | Re-render the platform icon set, see [App icon](#app-icon) |
| `bun run tauri dev` | Legacy Tauri shell, for migration reference only |

## Packaging and releases

Releases are cut by pushing a `v*` tag. `package.json` is the version source of
truth: the workflow fails if the tag and the manifest disagree. It then builds
the native addon and the renderer on Windows, Linux and macOS, and packages:

- **Windows** — NSIS installer (and its blockmap) for the update feed, plus an
  MSIX built by `scripts/pack-msix.mjs` from the same `win-unpacked` layout.
- **macOS** — dmg for first installs and zip for Squirrel.Mac updates (Apple
  Silicon).
- **Linux** — AppImage for the update feed and deb for package managers.

Every artifact is attached to one **draft** GitHub release. Publishing that
draft is the single manual step that makes both the downloads and the update
feed live. The metadata files (`latest.yml`, `latest-mac.yml`,
`latest-linux.yml`) must ship with the installers; publishing the draft is what
moves `releases/latest/download` to them.

### Auto-update

Packaged builds check the feed a few seconds after launch, download a newer
build in the background, install it silently and restart. The feed is the same
`releases/latest/download/...` path the Tauri shell used:

```
https://github.com/Melaine-iceberg/dae/releases/latest/download
```

`electron/updater.cjs` uses `electron-updater`'s generic provider, so the check
is a plain asset request rather than the GitHub provider's `releases/latest`
scrape (which is rate-limited and can answer 406). The outcome is logged to
`dae.log`; an offline machine or a missing release only costs a log line. Dev
builds and unpackaged runs do not check.

Platform behaviour:

- **Windows** — NSIS installs update in place. An MSIX install is owned by the
  package (`process.windowsStore`), so it skips the feed and updates by
  installing a newer MSIX.
- **macOS** — Squirrel.Mac only applies updates to a signed, notarized bundle.
  Without `MAC_CSC_LINK` and the `APPLE_*` secrets the dmg still installs, but
  auto-update cannot.
- **Linux** — AppImage updates in place. A deb install is managed by the
  system package manager and skips the feed.

Code signing is optional for Windows NSIS (updates are still validated against
the sha512 in `latest.yml`) and required for macOS updates. CI uses these
secrets when present:

| Secret | Purpose |
| --- | --- |
| `WIN_CSC_LINK`, `WIN_CSC_KEY_PASSWORD` | Authenticode for the Windows installer |
| `MAC_CSC_LINK`, `MAC_CSC_KEY_PASSWORD` | Developer ID Application identity for macOS |
| `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID` | macOS notarization |
| `MSIX_CERTIFICATE`, `MSIX_CERTIFICATE_PASSWORD` | Base64 PFX for the MSIX |

Without a code-signing certificate the Windows leg builds unsigned; without an
Apple identity the macOS leg builds unsigned. Both are acceptable for a draft
release, but only the signed variants can update themselves.

`scripts/sync-msix-version.mjs` keeps the four-part `Package.appxmanifest`
version in step with `package.json`; `--release` resets the revision so the tag
ships as `<app>.0`. The local `pack:msix` path uses a throwaway development
certificate (`devcert.pfx`) unless `MSIX_CERT`/`MSIX_CERT_PASSWORD` are set.

## Where things live

- `electron/main.cjs` — window lifecycle, IPC handlers, deep links, custom
  protocols, snap layout and the core utility process.
- `electron/updater.cjs` — the startup self-update check.
- `electron/preload.cjs` — the `window.daeBridge` transport.
- `native/src/` — the N-API addon: terminal, shell verbs, thumbnails, icons,
  drag-out, volume watching and the platform backends.
- `src-tauri/crates/dae-core/` — the platform-neutral file system core shared
  by the addon and the legacy Tauri shell.
- `src-tauri/` — the Tauri 2 shell kept as the migration reference; it is not
  part of the release artifacts.
- `src/features/` — React by area: `explorer/`, `sidebar/`, `workspace/`,
  `terminal/`, `settings/`, `shell-commands/`.
- `src/lib/` — the seams between the two halves: `ipc/` (the Tauri-shaped IPC
  facade over the preload bridge), `system-accent.ts`, `theme.ts`,
  `window-material.ts`, `window-focus.ts`.

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

- `native/src/system.rs` — the platform half. `windows` reads
  `HKCU\Software\Microsoft\Windows\DWM!AccentColor` and blocks on
  `RegNotifyChangeKeyValue` for changes; `macos` reads
  `NSColor.controlAccentColor` and polls (macOS posts nothing when the accent
  moves); `linux` reads `org.freedesktop.appearance accent-color` from the
  freedesktop settings portal and polls. Each reports `system-accent-changed`
  through the core event pump.
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

The Electron addon's Rust tests run with the standard toolchain:

```bash
bun run test:native
# or: cargo test --manifest-path native/Cargo.toml -- --test-threads=1
```

The serial flag matters on Windows: two Shell-icon tests race on the Shell COM
apartment when run in parallel.

The legacy Tauri shell still has its own suite; it uses
[cargo-nextest](https://nexte.st/):

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
for Windows, and the `android/`/`ios/` sets). `electron-builder.yml` reads
`icon.ico`, `icon.icns` and `icon.png` from that directory, so the installer
and the app window share one icon. The icon CLI is the last build step that
still needs the Tauri CLI; the generated files are checked in.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
  + the Oxlint extension.
