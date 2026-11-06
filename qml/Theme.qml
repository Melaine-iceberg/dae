pragma Singleton
import QtQuick
// The Rust singletons (`Appearance`, `Workspace`) live in module `dae`, which
// the shell's own files reach through the qrc path's implicit import. A
// singleton has no such implicit import, so it is named here.
import dae

// The shell's single source of colour. See docs/ui-instrument-migration.md §3.
//
// Before this file every component carried its own copy of the palette, and the
// copies had drifted into six families: a dialog wrote `#e7ebf2` where the list
// wrote `#e6e9ef`, a menu painted `#202329` where the command palette painted
// `#1c1f24`. None of those differences were decisions; they were edits made in
// one file and not the others. The roles below are the decisions, and there are
// deliberately few of them — a role that only one surface uses is a literal
// with extra steps.
//
// Naming note: `onX` is not available here. QML reads `on` + capital as a signal
// handler once the base name exists, so the ink that sits on `interactive` is
// called `onInteractive` only because nothing else is named `onInter...`; the
// moment `interactive` is declared the pair would collide. Keeping the prefixed
// names out of the vocabulary avoids the trap entirely.
QtObject {
    id: theme

    // ── Source ──────────────────────────────────────────────────────────────
    /// Mirrors `ShellWindow.dark` exactly. Every component already receives
    /// `dark` as a property; this exists so a surface that is constructed
    /// outside the shell tree (a dialog, a tear-off window, a toast) still
    /// resolves the right palette without being handed the flag.
    readonly property bool dark: Appearance.theme === "dark"
        || (Appearance.theme === "system" && Workspace.systemDark)

    // ── Interaction ─────────────────────────────────────────────────────────
    /// The brand pair: the instrument signal orange. Used when the user has not
    /// asked to follow the OS.
    readonly property color brand: dark ? "#f0a24a" : "#b45309"
    /// The window's one signal colour, independent of the accent preference.
    /// Selection tracks, focus rings and the active tab underline take this so
    /// the shell keeps its instrument character even when the OS accent is on.
    readonly property color signal: dark ? "#f0a24a" : "#b45309"
    /// The only saturated colour in the window. It marks what can be acted on,
    /// so it must not also be used as decoration: a tinted panel reads as a
    /// button to anyone who has used the app for a minute.
    readonly property color interactive: Appearance.accent === "brand"
        ? brand : Workspace.accent
    /// The accent under the pointer. Derived rather than tabulated so an OS
    /// accent the app has never seen still has a hover step.
    readonly property color interactiveHover: Qt.lighter(interactive, 1.15)
    /// Ink on top of `interactive`. Dark ink on the pale orange dark theme, the
    /// way the page uses a warm accent rather than a dark one.
    readonly property color onInteractive: dark ? "#1b1306" : "#ffffff"
    /// Ink on a fill that is saturated in both themes and belongs to someone
    /// else: a provider tile, the Windows close button, an error toast. Always
    /// white, because none of those surfaces follow the theme.
    readonly property color onSolid: "#ffffff"

    // ── Surfaces ────────────────────────────────────────────────────────────
    /// Where content lives: the listing, the editor, the preview text.
    readonly property color canvas: dark ? "#151619" : "#fbfbf9"
    /// A second canvas for surfaces that sit *inside* a canvas — a grid card,
    /// the alternating plate, the palette's shortcut plate.
    readonly property color canvasAlt: dark ? "#1b1c20" : "#f4f4f1"
    /// Whether a background image is set. The listing surfaces below switch
    /// from the solid canvas to a translucent one so the image shows through;
    /// chrome stays solid, which keeps the frame readable.
    readonly property bool hasBackground: ExplorerPrefs.backgroundImage !== ""
    /// How much canvas stays over the image: 1 is solid, 0.1 is the floor.
    /// `backgroundDim` is the user-facing 0–90 contrast guard.
    readonly property real contentAlpha: hasBackground
        ? Math.max(0.1, 1 - ExplorerPrefs.backgroundDim / 100) : 1
    /// The listing canvas. Identical to `canvas` without a background image.
    readonly property color content: Qt.rgba(canvas.r, canvas.g, canvas.b, contentAlpha)
    readonly property color contentAlt: Qt.rgba(canvasAlt.r, canvasAlt.g,
                                               canvasAlt.b, contentAlpha)
    /// The window's frame: title, tabs, path bar, status bar, sidebar. This is
    /// the only surface the OS material is allowed to show through, and the
    /// shell currently asks for no material at all so this value is what paints.
    readonly property color chrome: dark ? "#101114" : "#efefec"
    /// A surface that floats above everything else: menus, dialogs, the
    /// command palette. There is exactly one elevation, so there is exactly
    /// one value.
    readonly property color raised: dark ? "#202125" : "#ffffff"
    /// An input well: text fields, search, the rename box.
    readonly property color field: dark ? "#17181b" : "#e9e9e5"

    // ── Material tint ───────────────────────────────────────────────────────
    /// The veil painted over a native backdrop, by backdrop kind.
    ///
    /// Windows' Mica and Acrylic are composited by DWM with a tint of their
    /// own, so the QML layer stays light there (α .30–.46). A live blur —
    /// Wayland's `ext-background-effect-v1` or macOS's `NSVisualEffectView`
    /// — has no tint: at the same alpha the wallpaper's colour and motion
    /// came straight through the sidebar, so the chrome read as glass instead
    /// of as a surface and the 11–13 px labels lost contrast against it.
    /// Native file managers keep roughly a fifth of the backdrop visible
    /// behind chrome, which is the α .76–.86 band below.
    ///
    /// `material` is what `WindowController.appliedMaterial` reports (`mica`,
    /// `acrylic`, `blur` or `none`); `panel` is the sidebar, which has always
    /// run slightly deeper than the title, path and status bands.
    function backdropTint(material, isDark, panel) {
        if (material === "none")
            return theme.chrome
        const blur = material === "blur"
        const alpha = blur
            ? (panel ? (isDark ? 0.76 : 0.84) : (isDark ? 0.78 : 0.86))
            : (panel ? (isDark ? 0.30 : 0.34) : (isDark ? 0.38 : 0.46))
        return isDark
            ? Qt.rgba(panel ? 0.05 : 0.055, panel ? 0.06 : 0.063,
                      panel ? 0.07 : 0.075, alpha)
            : Qt.rgba(panel ? 0.95 : 0.98, panel ? 0.96 : 0.985,
                      panel ? 0.97 : 0.99, alpha)
    }

    // ── Ink ─────────────────────────────────────────────────────────────────
    /// Names and primary content.
    readonly property color ink: dark ? "#ecedef" : "#161719"
    /// Secondary: column headers, metadata, timestamps, hints. Kept above
    /// 4.5:1 against `canvas` in both themes so it stays readable.
    readonly property color inkMuted: dark ? "#9ba0a8" : "#5c6066"
    /// Tertiary: group labels, disabled glyphs, metadata. Above 4.5:1 against
    /// chrome, canvas and field in both themes, so it can carry the 11 px group
    /// labels and meta text — not just icons as the previous palette allowed.
    readonly property color inkFaint: dark ? "#8b9098" : "#646970"

    // ── Lines ───────────────────────────────────────────────────────────────
    /// Row separators and the edge of an unfocused control.
    /// Note the order: Qt reads eight digits as `#AARRGGBB`, so a translucent
    /// white is `#14ffffff`, never `#ffffff14` — the CSS order parses as a
    /// nearly opaque yellow and it does not fail, it just paints.
    readonly property color hairline: dark ? "#16ffffff" : "#14000000"
    /// The edge of something that holds: a field, a menu, a raised plate.
    readonly property color hairlineStrong: dark ? "#25ffffff" : "#25000000"
    /// The hairline between list rows: lighter than a structural line.
    readonly property color rowRule: dark ? "#0bffffff" : "#09000000"

    // ── State ───────────────────────────────────────────────────────────────
    /// Pointer is over a row or a quiet control. Deliberately below the
    /// threshold where it could be confused with selection.
    readonly property color hover: dark ? "#0dffffff" : "#0c000000"
    /// A state fill: a hovered menu item, an active chip, a row selected while
    /// the pane is unfocused. One step above `hover` so a menu can show both.
    readonly property color fill: dark ? "#14ffffff" : "#12000000"
    /// Behind a modal. Never opaque: the window has to stay legible so the
    /// user keeps their place.
    readonly property color scrim: dark ? "#61000000" : "#1a181816"

    // ── Accent-derived ──────────────────────────────────────────────────────
    /// The signal at 10 % (light) / 16 % (dark). This is the selected row, not
    /// a saturated plate, and it stays orange even when the system accent wins
    /// the interactive role.
    readonly property color selection: Qt.rgba(signal.r, signal.g, signal.b,
                                               dark ? 0.16 : 0.10)
    /// The wash behind an active chip or a focused drop target.
    readonly property color signalSoft: dark ? "#1af0a24a" : "#1ab45309"
    /// One step stronger: a drop edge or a focused search field.
    readonly property color signalSoft2: dark ? "#29f0a24a" : "#2eb45309"
    /// The ring around a focused control, and the edge of a drop target.
    readonly property color focusRing: Qt.rgba(interactive.r, interactive.g,
                                               interactive.b, 0.30)

    // ── Meaning ─────────────────────────────────────────────────────────────
    /// Destructive text and icons. Pairs with an explicit verb, never alone.
    readonly property color danger: dark ? "#f0918a" : "#b42318"
    /// Destructive fill for the one primary button that deletes something.
    readonly property color dangerSolid: dark ? "#c6403a" : "#c62c26"
    readonly property color dangerSolidHover: Qt.lighter(dangerSolid, 1.15)
    readonly property color success: dark ? "#8fd18f" : "#2f7d32"
    readonly property color warn: dark ? "#e0a340" : "#9a6700"
    /// Reference ink for links, commit hashes and the git status chips. Kept
    /// apart from `interactive` because it reports state rather than inviting
    /// a click from the listing itself.
    readonly property color link: dark ? "#6cb6ff" : "#0969da"
    /// The terminal follows the shell's theme. Its grid palette is resolved by
    /// the core per snapshot (`dae-core::pty`), and these are the matching
    /// panel, header plate and header ink: near-black on the dark canvas, the
    /// same paper as the rest of the window in light.
    readonly property color terminalCanvas: dark ? "#0b0b0c" : "#fbfbf9"
    readonly property color terminalHeader: dark ? "#141518" : "#f4f4f1"
    readonly property color terminalInk: dark ? "#8b9099" : "#5c6066"
    readonly property color terminalInkStrong: dark ? "#e8eaf0" : "#161719"

    // ── Motion ──────────────────────────────────────────────────────────────
    /// Qt 6 has no system-wide reduced-motion query, so `explorer.ini` carries
    /// the switch and every `Behavior` asks this before it runs. When it is
    /// off, transitions are replaced by instant state changes — not merely
    /// shortened, because "faster" still animates.
    readonly property bool motion: !ExplorerPrefs.reduceMotion
    /// Pointer feedback.
    readonly property int motionFast: 110
    /// A panel or a field that changes size.
    readonly property int motionPanel: 180
    /// A menu, a popover, the palette.
    readonly property int motionOverlay: 160

    // ── Type ────────────────────────────────────────────────────────────────
    /// Fixed-pitch for terminals, diffs, sizes and rates. Chosen per platform
    /// because QML cannot express a fallback list on `font.family`, and the
    /// shipped-with-another-app faces (`Cascadia Mono` arrives with Windows
    /// Terminal, not Windows) are not safe to name.
    readonly property string monoFamily: Qt.platform.os === "osx" ? "SF Mono"
        : Qt.platform.os === "windows" ? "Consolas"
        : "DejaVu Sans Mono"

    // ── Geometry ────────────────────────────────────────────────────────────
    /// The only corner radii the shell draws — four steps, no more, and the
    /// same four numbers the Web shell's `App.css` spends (`--radius-xs` 4,
    /// `--radius-md` 6, `--radius-xl` 12, `--radius-panel` 16).
    ///
    /// The Qt shell used to run 2 / 4 / 8 / 12, a full step below, on the
    /// argument that a denser surface wants tighter corners. It does not
    /// survive two shells side by side: the same button changed shape when the
    /// window switched shells, which reads as a rendering fault rather than as
    /// a density choice, and 2 px on a 16 px plate is a corner nobody can see.
    /// The two ladders now share their numbers, and `radiusLg` is the Web
    /// shell's `panel` step — dialogs, the palette's plate, the settings
    /// surface — so `panel` needs no separate name here.
    ///
    /// Anything that squares off a corner by painting over it has to clear the
    /// larger radius: `SettingsPanel`'s nav column covers 16 px of its own
    /// right edge, not the 10 it needed when this step was 12.
    readonly property int radiusXs: 4
    readonly property int radiusSm: 6
    readonly property int radiusMd: 12
    readonly property int radiusLg: 16
    /// Row heights; the header is always the paired value.
    readonly property int rowCompact: 28
    readonly property int rowDefault: 32
    readonly property int rowSpacious: 40
    /// Control heights — one ladder for everything the pointer can hit, on the
    /// 4 dp grid, five steps and no more.
    ///
    /// The shell used to draw its controls at 18 / 20 / 22 / 24 / 26 / 28 /
    /// 30 / 34, half of which are not multiples of four. Two things followed.
    /// The odd ones sat 2 dp from a neighbour they were supposed to match, so a
    /// chip and the row it lived in, or a form field and the button beside it,
    /// were *nearly* aligned — the one state the eye reads as a mistake rather
    /// than as a decision. And because none of the eight had a name, the same
    /// 34 was written out by hand in five unrelated files.
    ///
    /// These five share the 4 dp grid with `rowCompact` / `rowDefault`, which is
    /// the point: a 28 dp row and a 28 dp button are the same band. They keep
    /// separate names because they answer different questions — a row can grow
    /// to fit a second line of text, a control cannot — and because a caller
    /// that needs "one row tall" should not have to know it equals "one button
    /// tall".
    ///
    ///   xs  20  an icon plate, a close target, a keycap
    ///   sm  24  a chip, a toggle, a small action button
    ///   md  28  the standard control — button, field, menu row
    ///   lg  32  a form row, a field row, a column header
    ///   xl  36  a strip carrying one control plus its label
    readonly property int controlXs: 20
    readonly property int controlSm: 24
    readonly property int controlMd: 28
    readonly property int controlLg: 32
    readonly property int controlXl: 36
    /// The three chrome bands. Their sum is the 116 px shell.
    readonly property int shellTitle: 44
    readonly property int shellNav: 44
    readonly property int shellStatus: 28

    // ── Type scale ──────────────────────────────────────────────────────────
    /// Six real steps, every one of them an integer.
    ///
    /// `font.pixelSize` is an **int** in Qt — it lands on
    /// `QFont::setPixelSize(int)` — so a half-pixel rung is not a smaller step,
    /// it is a step that cannot be drawn. The scale used to run
    /// 11 / 11.5 / 12 / 12.5 / 13 / 13.5 / 14, and each of those three halves
    /// was rounded onto the integer beside it, whichever way the conversion
    /// went: `caption` merged into `micro` or `small`, `body` into `small` or
    /// `name`, `emphasis` into `name` or `title`. Eight names were painting at
    /// most five sizes, so the hierarchy the names promised was not the
    /// hierarchy on screen — which is exactly what makes a control and the word
    /// inside it look measured by two different rulers.
    ///
    /// The rungs are integers now, with a visible gap between the ones that
    /// have to be told apart. The three retired names survive as aliases so a
    /// stale call site cannot reintroduce a half step: `small` is `caption` and
    /// `name` is `body`. Those pairs were never distinguishable in paint, and a
    /// second name for one size is how the halves got invented in the first
    /// place.
    ///
    /// `title` moved 14 → 17 because 14 was the one rung nothing else could
    /// reach: a dialog head is 44 dp tall and the largest text in it was
    /// smaller than a list row's own type, so the head read as a band of empty
    /// chrome rather than as a name. The step is capped at 17 rather than the
    /// Web shell's 22 on purpose — see the note in `DialogHeader.qml` about the
    /// hosts that position this rung from absolute coordinates.
    readonly property int typeMicro: 11
    readonly property int typeCaption: 12
    readonly property int typeSmall: 12 /* retired alias → caption */
    readonly property int typeBody: 13
    readonly property int typeName: 13 /* retired alias → body */
    readonly property int typeEmphasis: 15
    readonly property int typeTitle: 17
    readonly property int typeDisplay: 22

    // ── Leading ─────────────────────────────────────────────────────────────
    /// Line height as a multiple of the type size, not as a pixel value: Qt's
    /// `lineHeight` is proportional by default (`Text.ProportionalHeight`), and
    /// a pixel value per rung would have to be retuned by hand every time a
    /// rung moves — the coupling that made the old scale drift in the first
    /// place.
    ///
    /// Three bands, and only three. Call sites used to carry 1.2 / 1.25 / 1.3 /
    /// 1.4 / 1.5 / 1.6 — six values for one job, picked per file, which rounded
    /// to 14 / 15 / 16 px at a 12 px size and put a one-pixel wobble between
    /// paragraphs that were supposed to be identical.
    ///
    ///   tight   a single-line chip, a keycap, a two-line hint
    ///   body    the default — matches the 1.448 that the Web shell's
    ///           14.5 / 21 px pairing works out to
    ///   prose   a paragraph someone actually reads
    readonly property real leadingTight: 1.35
    readonly property real leadingBody: 1.45
    readonly property real leadingProse: 1.6
}
