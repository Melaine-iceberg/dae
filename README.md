# dae

An aesthetic desktop file manager with an integrated terminal. Rust behind a
Qt Quick interface (QML + `cxx-qt`).

The Tauri 2 + React 19 frontend this repository grew up with was removed on
2026-11-01, once the Qt shell had every feature it had and the release pipeline
no longer built it — `docs/decisions/engine-qt-quick.md` records why the engine
changed and §9.13 what the retirement touched. Nothing in this tree is from
that shell; `git log` and the decision document are where it lives now.

- **Tabs and windows.** Tabs drag between windows and tear off into their own
  window on demand.
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

## Install

Every release carries one artifact per platform, all built from the Qt shell
(`apps/dae` + `qml/`):

| Platform | Artifact | How it updates |
| --- | --- | --- |
| Windows | `dae_<version>_x64.msix`; portable `dae-<version>-windows-x86_64.zip` | The MSIX is a real package identity: Windows installs it under `C:\Program Files\WindowsApps`, the manifest owns `dae://`, and the app cannot replace itself, so a new version arrives as a new package. The zip is the portable form and updates itself from `latest-qt.json`, found through the releases API because dae ships previews only (`releases/latest/` never resolves; see `docs/packaging.md` §5). One caveat on the packed build: it cannot register itself as the default folder handler — Windows redirects a packaged app's `HKCU\Software\Classes` writes into a per-app hive Explorer does not read — so pick the portable zip if double-click-a-folder should open dae (`docs/decisions/engine-qt-quick.md` §9.15). |
| Linux | `dae_<version>_amd64.deb`, `dae-<version>-1.x86_64.rpm`; portable `dae-<version>-linux-x86_64.zip` | `sudo apt install ./dae_<version>_amd64.deb` on Debian/Ubuntu, `sudo dnf install ./dae-<version>-1.x86_64.rpm` on Fedora/RHEL and `sudo zypper install ./dae-<version>-1.x86_64.rpm` on openSUSE. Both carry the same payload; the `.deb` derives its `Depends` from the payload's `ldd`, the `.rpm` lets rpm's own ELF generator require sonames. GStreamer is a weak dependency (`Recommends`) in both. |

**There is no macOS artifact.** The bundle used to be Developer ID signed and
notarized on a runner, but nothing here could confirm it actually starts on a
Mac, and shipping a platform nobody can launch is worse than not shipping it —
so the leg was removed on 2026-11-01. The macOS-only Rust still compiles in CI
(`qt-check.yml`); only the release artifact is gone. See
[`docs/packaging.md`](docs/packaging.md) §1, which also lists what restoring it
would take.

**Media playback on Linux needs GStreamer.** Qt Multimedia's Linux backend
decodes through GStreamer; the Qt plugin ships inside the package, the GStreamer
runtime and codec plugins come from the system. Both packages recommend the
decoder set, under each distribution's own names — `gstreamer1.0-plugins-base`,
`-good`, `-bad`, `gstreamer1.0-libav` on Debian/Ubuntu; `gstreamer1-plugins-base`,
`-good`, `gstreamer1-plugin-libav` on Fedora/RHEL; `gstreamer-plugins-*` on
openSUSE — and `apt`/`dnf`/`zypper` install recommendations by default. On a
system that declined them (or under the portable zip on a minimal install) the
preview pane says which packages are missing instead of failing silently. The
portable zip has no package manager, so on a bare system install them by hand:

```bash
sudo apt install gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-libav
```

Manual verification steps for both installers — signing credentials,
`dpkg`/`Get-AppxPackage` checks, the upgrade path — are in
[`docs/packaging.md`](docs/packaging.md).

## Prerequisites

- A stable Rust toolchain. The crates are edition 2024, so 1.85 or newer.
- **Qt 6.12** with Qt Quick and the `qtmultimedia` module. `cxx-qt` finds
  `qmake` on `PATH` or in `QMAKE`; set both before `cargo build -p dae-app`
  (`QMAKE=…/bin/qmake`, and Qt's `bin/` on `PATH`).
- Platform build dependencies, the same sets the release workflow installs:
  - **Linux** — `build-essential curl wget file pkg-config cmake ninja-build`,
    plus the GL, XCB/Wayland, xkbcommon, fontconfig, dbus and secret
    development headers; the authoritative list is the `qt` job in
    `.github/workflows/release.yml`.
  - **Windows** — MSVC build tools.
    [`winapp`](https://github.com/microsoft/winappCli) is needed only for the
    MSIX packaging script.
  - **macOS** — the Xcode command line tools.
- [Bun](https://bun.sh/) and Node 24 — only for the release tooling in
  `scripts/` (packaging, icons, PGO/BOLT, signing) and as the version source the
  release jobs read. A plain `cargo build` does not touch either.

## Getting started

```bash
export QMAKE=/path/to/Qt/6.12.0/msvc2022_64/bin/qmake   # or the gcc/macos kit
export PATH=/path/to/Qt/6.12.0/msvc2022_64/bin:$PATH
cargo build -p dae-app
cargo run -p dae-app
```

The build compiles the QML into the binary (see [QML AOT](docs/qml-aot.md)), so
it needs the QML tooling Qt ships next to `qmake` — `qmlcachegen`, `qmltc` and
`qmllint` are all looked up through it. A *deployed* tree additionally needs
`platforms/` beside the executable; without it the updater refuses to run, which
is what keeps a `target/debug` build from replacing itself.

To try the Windows package locally:

```bash
bun install             # the release scripts only
bun run pack:msix       # builds the shell, deploys Qt, packs the MSIX
bun run verify:msix     # installs it, checks launch, dae:// and the upgrade leg
```

## Scripts

Everything below is release tooling; none of it is needed to build or run the
app.

| Command | What it does |
| --- | --- |
| `cargo build -p dae-app` | The shell. Add `--release` for the profile releases ship with |
| `cargo test -p dae-qt --lib` | The bridge under test (models, appearance, logging, platform reads) |
| `cargo test -p dae-core --lib` | The domain: filesystem backends, archives, terminal, the update engine |
| `bun run test:rust` / `test:rust:setup` | The whole workspace through cargo-nextest |
| `bun run lint:qml` | `qmllint` over `qml/` (needs Qt's `bin/` on `PATH`) |
| `bun run pack:msix` | Windows: builds the Qt shell, deploys Qt with `windeployqt`, packs the tree as MSIX (`scripts/pack-msix.ps1`) |
| `bun run verify:msix` | Windows: installs a `.msix` and checks the launch, `dae://` and upgrade legs (`scripts/msix-verify.ps1`; `-InspectOnly` reads the manifest and changes nothing) |
| `bun run update:feed` | Builds `latest-qt.json` from the signed per-platform zips (`scripts/qt-update-feed.mjs`) |
| `bun run icons:icon` | Re-render the icon set from the artwork, see [App icon](#app-icon) |
| `bun run perf:pgo:qt:instrument` / `:train` / `:merge` / `:build` | Staged LLVM IR PGO build for the shell, see [PGO and BOLT](docs/pgo-bolt.md) |
| `bun run perf:pgo:qt:build:bolt` | The same, plus the BOLT pass (Linux only) |
| `bun run perf:post-link` | The BOLT step on its own; `release.yml` calls it after the profile build |

Releases are cut by pushing a `v*` tag. `.github/workflows/release.yml` then
runs one leg per platform, and the tag must match the version in
`package.json` and the three crates (`apps/dae`, `crates/dae-qt`,
`crates/dae-core`) or the `draft` job fails before anything is uploaded. The
`qt` legs build the shell, zip it, sign it with the release minisign key and
upload the archives; `msix` attaches the Windows package; `qt-feed` writes
`latest-qt.json` and attaches it last, so the feed never advertises a zip that
is not on the release yet. The release stays a draft for manual publishing.

`qt-check.yml` compiles the shell on Linux and macOS on every push, so
`#[cfg(target_os = …)]` code a Windows machine cannot see still meets a compiler
before release day; that job and the release legs also compile the QML ahead of
time ([QML AOT](docs/qml-aot.md)).

## Where things live

- `apps/dae/` — the binary. `src/main.rs` creates the application and the QML
  engine; `cpp/` holds the one C++ file the shell links directly.
- `crates/dae-core/` — the domain, with no Qt in it: filesystem backends (local,
  `sftp`, `smb`, `archive`), the PTY, thumbnails, search, trash, the update
  engine and signed-feed verification.
- `crates/dae-qt/` — the bridge. QML-facing models and commands (`workspace`,
  `model`, `terminal`, `appearance`, `updater`), and `cpp/` for what only the
  platform knows: window chrome and hit testing, materials, the IME focus guard,
  the macOS menu and platform reads, Wayland background blur.
- `qml/` — the interface: `ShellWindow.qml` is the whole window, `Theme.qml` the
  token source of truth, one file per surface otherwise.
- `packaging/` — the `.desktop` entries, the launcher wrapper, the macOS plist
  and entitlements kept for a future re-run, and `icons/` (the artwork and the
  derived set).
- `Assets/` — the MSIX tile resources Windows indexes by name.

## Performance

Work is judged by measurement, and
[`docs/decisions/`](docs/decisions/README.md) is where the measurements live:
what was run, the numbers behind each one, and the conditions that would reopen
a decision. Measure before changing anything.

Two build-time optimisations sit on top of the release profile: LLVM IR PGO and,
on Linux, a post-link pass with LLVM BOLT. The cxx-qt C++ bridge joins the
profile where the compiler is clang with rustc's LLVM major; prebuilt Qt itself
is out of reach. How to run them, why BOLT is ELF-only, and what CI falls back to
when a profile or a tool is missing is [`docs/pgo-bolt.md`](docs/pgo-bolt.md).

## Appearance

The shell's accent is the OS accent — the colour the user already picked in
Windows or their Linux desktop — because that is what makes one cross-platform
app read as native without imitating any of them.
`crates/dae-qt/src/platform.rs` reads it (`HKCU\…\DWM!AccentColor` on Windows,
`org.freedesktop.appearance accent-color` on Linux), `cpp/platform_watch.cpp`
watches for changes, and the workspace hands the result to QML, where the accent
roles derive from `Theme.accent`. Category colours (`Theme.folder`, the tone set)
deliberately do not follow it: a symlink stays the colour `ls --color` paints it.

## Decisions

`docs/decisions/` is the decision log (written in Chinese): why the engine moved
to Qt Quick, why the file-IO mechanism stayed as it is, why Windows directory
enumeration needed the large-fetch flag, why SFTP host keys are trusted on first
use, and why the gtk/gio stack is pinned at 0.18 — each with the measurements
behind it, plus the conditions that would reopen the question. Start at
[`docs/decisions/README.md`](docs/decisions/README.md); the records that only
describe the retired WebView shell are archived on the `tauri` branch.

Feature planning sits outside that log: [`docs/files-parity.md`](docs/files-parity.md)
is the gap list against Files and the cross-platform plan for each item.

## Tests

Rust tests use [cargo-nextest](https://nexte.st/):

```bash
bun run test:rust:setup    # cargo binstall cargo-nextest
bun run test:rust
```

`cargo test -p dae-qt --lib` is the fastest loop while working on the bridge. The
platform manuals no unit test can cover — window chrome, menus, IME, thumbnails,
installers — are the checklists in
[`docs/qt-platform-parity.md`](docs/qt-platform-parity.md) §4. Linux was run
against that list on 2026-11-01 (Wayland and X11, fcitx5, the `.rpm`) and passed
every applicable row; the macOS rows are marked not-applicable rather than
untested, for the same reason there is no macOS artifact above. The record is
`docs/decisions/engine-qt-quick.md` §9.14.

## App icon

The app icon (a white goose with an orange beak and a red line on a rounded
charcoal plate) starts from a 2048×2048 raster artwork:

- `packaging/icons/source/goose-source.jpg` — the original artwork
- `packaging/icons/source/goose-master.png` — 1024×1024 master rendered from it
- `packaging/icons/source/goose-preview.png` — 256px preview for quick review

`bun run icons:icon` re-renders the master, the preview and the MSIX tiles in
`Assets/`. The per-size PNGs and `icon.ico` the packages use come from the same
master through `bun tauri icon packaging/icons/source/goose-master.png` — the
Tauri CLI stays a dev dependency for that and for signing.

## License

GPL-3.0-or-later — see [`LICENSE`](LICENSE). Chosen because the interface layer
is Qt 6, whose GPLv3 option this matches without the LGPL relinking obligations;
the trade-off is that macOS App Store distribution is out.
