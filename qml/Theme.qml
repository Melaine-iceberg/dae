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
    /// The brand pair: drift's vermilion. Used when the user has not asked to
    /// follow the OS.
    readonly property color brand: dark ? "#ff6a3d" : "#ff4d1f"
    /// The window's one signal colour, independent of the accent preference.
    /// Selection tracks, focus rings and the active tab underline take this so
    /// the shell keeps its printed character even when the OS accent is on.
    readonly property color signal: dark ? "#ff6a3d" : "#ff4d1f"
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
    readonly property color onInteractive: dark ? "#1a0e08" : "#ffffff"
    /// Ink on a fill that is saturated in both themes and belongs to someone
    /// else: a provider tile, the Windows close button, an error toast. Always
    /// white, because none of those surfaces follow the theme.
    readonly property color onSolid: "#ffffff"

    // ── Surfaces ────────────────────────────────────────────────────────────
    /// Where content lives: the listing, the editor, the preview text. drift
    /// prints on warm paper rather than on screen white; every other surface
    /// below is a step of that same stock, not a separate material.
    readonly property color canvas: dark ? "#131210" : "#ece6d8"
    /// A second canvas for surfaces that sit *inside* a canvas — a grid card,
    /// the alternating plate, the palette's shortcut plate. The lighter card
    /// stock the sheets are cut from.
    readonly property color canvasAlt: dark ? "#1d1b17" : "#f6f2e8"
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
    /// The window's frame: title, tabs, path bar, status bar, sidebar. One ink
    /// line separates it from the paper instead of a shade change: drift draws
    /// regions with rules, so the frame and the listing share a colour.
    readonly property color chrome: dark ? "#131210" : "#ece6d8"
    /// A surface that floats above everything else: menus, dialogs, the
    /// command palette. There is exactly one elevation, so there is exactly
    /// one value; `edge` and `shadow` are what separate it from what is below.
    readonly property color raised: dark ? "#1d1b17" : "#f6f2e8"
    /// An input well: text fields, search, the rename box.
    readonly property color field: dark ? "#1d1b17" : "#f6f2e8"

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
    ///
    /// The veil is paper rather than neutral grey: drift's chrome is a printed
    /// stock, and a material tinted with it reads as the same sheet held up to
    /// the desktop instead of as a second, bluer window behind this one.
    function backdropTint(material, isDark, panel) {
        if (material === "none")
            return theme.chrome
        const blur = material === "blur"
        const alpha = blur
            ? (panel ? (isDark ? 0.76 : 0.84) : (isDark ? 0.78 : 0.86))
            : (panel ? (isDark ? 0.30 : 0.34) : (isDark ? 0.38 : 0.46))
        return isDark
            ? Qt.rgba(0.075, 0.071, 0.063, alpha)
            : Qt.rgba(0.925, 0.902, 0.847, alpha)
    }

    // ── Ink ─────────────────────────────────────────────────────────────────
    /// Names and primary content. drift's ink is warm black, never #000: the
    /// same ink the sheets are outlined with.
    readonly property color ink: dark ? "#ece6d8" : "#16140f"
    /// Secondary: column headers, metadata, timestamps, hints. Kept above
    /// 4.5:1 against `canvas` in both themes so it stays readable.
    readonly property color inkMuted: dark ? "#b9b3a5" : "#4a453b"
    /// Tertiary: group labels, disabled glyphs, metadata. Above 4.5:1 against
    /// chrome, canvas and field in both themes, so it can carry the 11 px group
    /// labels and meta text — not just icons as the previous palette allowed.
    readonly property color inkFaint: dark ? "#948e80" : "#625c50"

    // ── Lines ───────────────────────────────────────────────────────────────
    /// Row separators and the edge of an unfocused control: ink held back to a
    /// fifth, the weight drift gives a rule inside a table.
    /// Note the order: Qt reads eight digits as `#AARRGGBB`, so a translucent
    /// white is `#24ffffff`, never `#ffffff24` — the CSS order parses as a
    /// nearly opaque yellow and it does not fail, it just paints.
    readonly property color hairline: dark ? "#33ece6d8" : "#3316140f"
    /// The edge of something that holds: a field, a menu, a raised plate, and
    /// every panel boundary in the frame. Ink at four fifths, not a shade of
    /// the surface — drift closes a region with a line you can see.
    readonly property color hairlineStrong: dark ? "#ccece6d8" : "#cc16140f"
    /// The hairline between list rows: lighter than a structural line.
    readonly property color rowRule: dark ? "#1aece6d8" : "#1a16140f"

    // ── State ───────────────────────────────────────────────────────────────
    /// Pointer is over a row or a quiet control. Deliberately below the
    /// threshold where it could be confused with selection.
    readonly property color hover: dark ? "#12ece6d8" : "#1216140f"
    /// A state fill: a hovered menu item, an active chip, a row selected while
    /// the pane is unfocused. One step above `hover` so a menu can show both.
    readonly property color fill: dark ? "#1fece6d8" : "#1f16140f"
    /// Behind a modal. Never opaque: the window has to stay legible so the
    /// user keeps their place.
    readonly property color scrim: dark ? "#61000000" : "#1a181816"

    // ── Accent-derived ──────────────────────────────────────────────────────
    /// The signal at 8 % (light) / 12 % (dark). This is the selected row, not
    /// a saturated plate, and it stays vermilion even when the system accent
    /// wins the interactive role.
    readonly property color selection: Qt.rgba(signal.r, signal.g, signal.b,
                                               dark ? 0.12 : 0.08)
    /// The wash behind an active chip or a focused drop target.
    readonly property color signalSoft: dark ? "#1fff6a3d" : "#14ff4d1f"
    /// One step stronger: a drop edge or a focused search field.
    readonly property color signalSoft2: dark ? "#38ff6a3d" : "#2eff4d1f"
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
    readonly property color warn: dark ? "#f0b838" : "#8a6100"
    /// The manila of a folder sheet: drift draws the tab lighter than the
    /// body, and the two values are a pair rather than a colour and its tint.
    readonly property color folderTab: dark ? "#c08c1e" : "#f0b838"
    readonly property color folderBody: dark ? "#a4761a" : "#d99a1c"
    /// Reference ink for links, commit hashes and the git status chips. Kept
    /// apart from `interactive` because it reports state rather than inviting
    /// a click from the listing itself.
    readonly property color link: dark ? "#6cb6ff" : "#0969da"
    /// The terminal follows the shell's theme. Its grid palette is resolved by
    /// the core per snapshot (`dae-core::pty`), and these are the matching
    /// panel, header plate and header ink: card stock on the dark canvas, the
    /// same paper as the rest of the window in light.
    readonly property color terminalCanvas: dark ? "#131210" : "#ece6d8"
    readonly property color terminalHeader: dark ? "#1d1b17" : "#f6f2e8"
    readonly property color terminalInk: dark ? "#b9b3a5" : "#4a453b"
    readonly property color terminalInkStrong: dark ? "#ece6d8" : "#16140f"

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
    /// The serif: a printed name, not a UI label. Only the listing header and
    /// the preview title take it, and only at the two rungs below.
    ///
    /// QML cannot express a fallback list on `font.family`, so each platform
    /// names the face it ships. Windows' CJK serif is SimSun — old, but it is
    /// the only serif the stock system has, and at 30–56 dp its Latin is a
    /// matching Didone rather than a fallback.
    readonly property string serifFamily: Qt.platform.os === "osx" ? "Songti SC"
        : Qt.platform.os === "windows" ? "SimSun"
        : "Noto Serif CJK SC"

    // ── Geometry ────────────────────────────────────────────────────────────
    /// No corner radii. drift is drawn with a straight edge: rectangles,
    /// 1 px ink rules and a hard offset shadow are the whole vocabulary, so a
    /// rounded corner anywhere reads as a different application.
    ///
    /// The four names stay — 216 call sites bound to them, and a call site that
    /// says `radiusSm` states "a corner belongs here", which is worth keeping
    /// even while the value is 0. When a surface genuinely needs a curve it
    /// takes its own number, not a renamed token.
    readonly property int radiusXs: 0
    readonly property int radiusSm: 0
    readonly property int radiusMd: 0
    readonly property int radiusLg: 0
    /// How far a floating surface's shadow steps out from the plate. There is
    /// no blur: the shadow is the plate's own rectangle in solid ink, displaced
    /// by this many pixels (see `HardShadow.qml`).
    readonly property int shadowOffset: 5

    /// The width of every structural rule in the frame. One pixel: a printed
    /// hairline, not a 2 px border — the print look comes from the rule being
    /// ink rather than from it being thick.
    readonly property int edgeWidth: 1
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
    /// Seven real steps, every one of them an integer.
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
    /// have to be told apart. `name` is one rung above `body` for listing
    /// names; `small` survives as an alias of `caption` so a stale call site
    /// cannot reintroduce a half step for metadata that never needed its own
    /// size.
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
    readonly property int typeName: 14 /* one rung above body, for listing names */
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
