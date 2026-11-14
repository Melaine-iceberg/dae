pragma Singleton
import QtQuick
// The Rust singletons (`Appearance`, `Workspace`) live in module `dae`, which
// the shell's own files reach through the qrc path's implicit import. A
// singleton has no such implicit import, so it is named here.
import dae

// The shell's single source of colour and metrics.
//
// Rewritten for the Graphite direction — see `docs/ui-redesign/proposal.md`
// §6.1 (colour), §6.2 (type), §6.4 (geometry) and appendix A (the migration
// table). Two rules hold everywhere below and are worth stating before the
// values:
//
//   1. Every colour is a literal in a two-arm `dark ? … : …` pair. Nothing is
//      derived with `Qt.lighter()` / `Qt.darker()` any more: a derived tone
//      cannot be checked against the contrast script, and the script is what
//      the acceptance gate reads (§13.0, §13.5).
//   2. There is one accent. Selection, focus, the primary action and progress
//      take it; nothing else does. A tinted panel reads as a button.
//
// Names retired by appendix A are gone (`brand`, `interactive`, `link`,
// `folderTab`, `folderBody`, `canvasAlt`, `fill`, `rowRule`, `signal*`,
// `serifFamily`, `shadowOffset`). Call sites were migrated with them, so a
// stale reference is a load error rather than a silent second palette.
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
    /// The window's only saturated colour (proposal §6.1.4): OKLCH 195°, kept
    /// 30° away from the nearest file family so a type mark can never be
    /// mistaken for the selection.
    readonly property color accent: dark ? "#6bcac9" : "#007a7b"
    /// The accent under the pointer. A literal, not `Qt.lighter()`, so both
    /// themes can be measured.
    readonly property color accentHover: dark ? "#7dd4d3" : "#00696a"
    /// Ink on top of `accent`.
    readonly property color onAccent: dark ? "#101216" : "#ffffff"
    /// Ink on a fill that is saturated in both themes and belongs to someone
    /// else: a provider tile, the Windows close button, an error toast. Always
    /// white, because none of those surfaces follow the theme.
    readonly property color onSolid: "#ffffff"

    // ── Surfaces (proposal §6.1.2) ──────────────────────────────────────────
    /// The window's bottom layer, visible only in the gutters around the
    /// panels. One step away from `chrome` so a hidden panel still reads as a
    /// panel-shaped hole.
    readonly property color window: dark ? "#090b0f" : "#eff0f2"
    /// The frame: tab row, sidebar, status bar, inspector.
    readonly property color chrome: dark ? "#15171c" : "#f7f7f9"
    /// Content: the listing, the grid, the preview body. In both themes this is
    /// the surface closest to the text's contrast.
    readonly property color canvas: dark ? "#101216" : "#fdfdfd"
    /// A surface that floats above everything else: menus, dialogs, the command
    /// palette, the action bar.
    readonly property color raised: dark ? "#292b30" : "#fdfdfd"
    /// An input well: text fields, search, path editing.
    readonly property color field: dark ? "#1c1e23" : "#eff0f2"
    /// A surface *inside* a surface: the inspector's preview well, tag pills,
    /// a grid card. Replaces `canvasAlt`.
    readonly property color inset: dark ? "#15171c" : "#f7f7f9"

    /// Whether a background image is set. The listing surfaces below switch
    /// from the solid canvas to a translucent one so the image shows through;
    /// chrome stays solid, which keeps the frame readable.
    readonly property bool hasBackground: ExplorerPrefs.backgroundImage !== ""
    /// How much canvas stays over the image: 1 is solid, 0.1 is the floor.
    /// `backgroundDim` is the user-facing 0–90 contrast guard.
    readonly property real contentAlpha: hasBackground
        ? Math.max(0.1, 1 - ExplorerPrefs.backgroundDim / 100) : 1
    /// The listing canvas. Identical to `canvas` without a background image.
    readonly property color content: Qt.rgba(canvas.r, canvas.g, canvas.b,
                                             contentAlpha)

    // ── Material tint ───────────────────────────────────────────────────────
    /// The veil painted over a native backdrop, by backdrop kind.
    ///
    /// Decision 1 (§14) makes the opaque surfaces above the default: the
    /// material is off unless the user turns it on, and this function is only
    /// reached in that case. The veil is neutral now — drift tinted it with
    /// paper, which is exactly the warm cast §6.1 drops.
    ///
    /// `material` is what `WindowController.appliedMaterial` reports (`mica`,
    /// `acrylic`, `blur` or `none`); `panel` is the sidebar, which has always
    /// run slightly deeper than the title, path and status bands.
    function backdropTint(material, isDark, panel) {
        if (material === "none")
            return isDark ? theme.window : theme.chrome
        const blur = material === "blur"
        const alpha = blur
            ? (panel ? (isDark ? 0.76 : 0.84) : (isDark ? 0.78 : 0.86))
            : (panel ? (isDark ? 0.30 : 0.34) : (isDark ? 0.38 : 0.46))
        return isDark
            ? Qt.rgba(0.055, 0.063, 0.071, alpha)
            : Qt.rgba(0.949, 0.953, 0.961, alpha)
    }

    // ── Ink (proposal §6.1.3) ───────────────────────────────────────────────
    /// Names and primary content. 16.4:1 on canvas, 15.6:1 on chrome.
    readonly property color ink: dark ? "#f7f7f9" : "#1c1e23"
    /// Secondary: column headers, metadata, status bar. 7.6:1 / 7.2:1.
    readonly property color inkSecondary: dark ? "#bcbec2" : "#505358"
    /// Tertiary: extensions, group labels, placeholders. 5.1:1 / 4.8:1, which
    /// is the floor the gate allows for text.
    readonly property color inkTertiary: dark ? "#96989d" : "#6b6d72"

    // ── Lines (proposal §6.1.2) ─────────────────────────────────────────────
    /// Region boundaries. A hairline carries no information, so it has no
    /// contrast requirement — only visibility.
    readonly property color hairline: dark ? "#1c1e23" : "#e5e6e9"
    /// A control's edge, a menu's border. Replaces `hairlineStrong`.
    readonly property color line: dark ? "#292b30" : "#d7d9dc"
    /// The edge of something that has to be seen: a drop target, a focus ring.
    /// A floating surface uses this in the dark theme, where `line` and
    /// `raised` are the same value (§6.5).
    readonly property color lineStrong: dark ? "#3a3d43" : "#bcbec2"

    // ── State ───────────────────────────────────────────────────────────────
    /// Pointer is over a row or a quiet control: ink at 4 %, which stays below
    /// the threshold where it could be confused with selection.
    readonly property color hover: dark ? "#0af7f7f9" : "#0a1c1e23"
    /// Behind a modal. Never opaque: the window has to stay legible so the
    /// user keeps their place.
    readonly property color scrim: dark ? "#73000000" : "#1a1c1e23"

    // ── Accent-derived ──────────────────────────────────────────────────────
    /// The selected row is a pale mask, not a colour block: 10 % in light,
    /// 13 % in dark, with the text staying `ink`, so the weight stays on the
    /// file name (§7.1.1).
    readonly property color selection: dark ? "#216bcac9" : "#1a007a7b"
    /// The wash behind an active chip or a drop target.
    readonly property color accentSoft: dark ? "#1f6bcac9" : "#1f007a7b"
    /// One step stronger: a drop edge or a focused search field.
    readonly property color accentSoftStrong: dark ? "#3d6bcac9" : "#3d007a7b"
    /// Transitional: the old half-transparent ring. §6.1.4 asks for a 1.5 dp
    /// accent border instead, which the P2 list work adopts; until then this
    /// keeps the seven call sites on the accent hue rather than a second one.
    readonly property color focusRing: dark ? "#596bcac9" : "#59007a7b"

    // ── Meaning (proposal §6.1.5) ───────────────────────────────────────────
    /// Destructive text and icons. Pairs with an explicit verb, never alone.
    readonly property color danger: dark ? "#f47b74" : "#ba2b2e"
    /// Destructive fill for the one primary button that deletes something.
    /// `danger` is already dark enough to carry white text in light; the dark
    /// theme steps it down so it does not glare.
    readonly property color dangerSolid: dark ? "#e06b64" : "#ba2b2e"
    readonly property color dangerSolidHover: dark ? "#ea857e" : "#a3272a"
    readonly property color success: dark ? "#7bc495" : "#227849"
    readonly property color warning: dark ? "#e6b55d" : "#8d5a00"

    // ── File families (proposal §6.1.6) ─────────────────────────────────────
    /// Fourteen families at constant lightness and chroma, hue only. The two
    /// tables are literals so the contrast script can read them; `fileColor()`
    /// is the only accessor, and an unknown family falls back to `other`.
    ///
    /// `link` is deliberately absent: the hue wheel has no position 30° from
    /// the accent that does not collide with a family, so a symbolic link is
    /// marked by `↗` at the end of the row instead (§6.1.6).
    readonly property var fileFamiliesLight: ({
            "folder": "#7c5d2f",
            "document": "#57608e",
            "spreadsheet": "#476f48",
            "presentation": "#89524d",
            "pdf": "#87515d",
            "image": "#79547b",
            "video": "#6a5988",
            "audio": "#815a33",
            "archive": "#87553e",
            "code": "#336a88",
            "text": "#3f678c",
            "font": "#645b8b",
            "executable": "#6a6630",
            "other": "#4d638e"
        })
    readonly property var fileFamiliesDark: ({
            "folder": "#d2b690",
            "document": "#afb8e2",
            "spreadsheet": "#a3c5a3",
            "presentation": "#dfaca7",
            "pdf": "#ddabb5",
            "image": "#cfaed0",
            "video": "#c0b2dc",
            "audio": "#d6b393",
            "archive": "#dcaf9b",
            "code": "#95c1dc",
            "text": "#9dbfe0",
            "font": "#bbb4df",
            "executable": "#c0bd91",
            "other": "#a7bbe2"
        })

    /// The mark colour for a file family. `family` is what
    /// `dae-qt::model::category_of()` reports; an unknown value is `other`.
    function fileColor(family) {
        const table = dark ? fileFamiliesDark : fileFamiliesLight
        return table[family] !== undefined ? table[family] : table["other"]
    }

    /// The eight tag colours (§6.1.7): fixed options at one lightness, because
    /// a free picker cannot promise that any two tags stay legible on both
    /// themes. Shared by the list's 6 dp dots and the inspector's pills.
    readonly property var tagColors: dark
        ? ["#e08585", "#e0a377", "#ddc274", "#8fc48c", "#7fc3bd", "#82a9dd", "#ab93dd", "#a8adb4"]
        : ["#a3392f", "#9a5a20", "#7d6a12", "#2f6b34", "#1d6560", "#2b5590", "#5b3f92", "#5c6169"]

    // ── Terminal ────────────────────────────────────────────────────────────
    /// The terminal follows the shell's theme. Its grid palette is resolved by
    /// the core per snapshot (`dae-core::pty`), and these are the matching
    /// panel, header plate and header ink.
    readonly property color terminalCanvas: canvas
    readonly property color terminalHeader: chrome
    readonly property color terminalInk: inkSecondary
    readonly property color terminalInkStrong: ink

    // ── Motion (proposal §6.6) ──────────────────────────────────────────────
    /// Qt 6 has no system-wide reduced-motion query, so `explorer.ini` carries
    /// the switch and every `Behavior` asks this before it runs. When it is
    /// off, transitions are replaced by instant state changes — not merely
    /// shortened, because "faster" still animates.
    readonly property bool motion: !ExplorerPrefs.reduceMotion
    /// Hover, press, focus: colour and opacity only.
    readonly property int motionFast: 80
    /// A selection mask appearing.
    readonly property int motionSelect: 90
    /// A panel or a field that changes size.
    readonly property int motionPanel: 160
    /// A menu, a popover, the palette.
    readonly property int motionOverlay: 120

    // ── Type (proposal §6.2) ────────────────────────────────────────────────
    /// Four rungs, every one an integer: `font.pixelSize` is an int in Qt, so a
    /// half step is not a smaller step, it is a step that cannot be drawn.
    ///
    ///   micro   11  group labels, column heads, status bar, key hints
    ///   caption 12  metadata, extensions, tags, menu hints
    ///   body    13  file names, listing text, menu items, fields
    ///   heading 15  inspector title, dialog title, empty-state sentence
    readonly property int typeMicro: 11
    readonly property int typeCaption: 12
    readonly property int typeBody: 13
    readonly property int typeHeading: 15

    /// Retired rung names, kept as aliases so an un-migrated call site cannot
    /// reintroduce a size outside the four. They map by role, not by old value:
    /// `typeName` was 14 and is now the body rung, because 14 is not on the
    /// scale any more.
    readonly property int typeSmall: typeCaption
    readonly property int typeName: typeBody
    readonly property int typeEmphasis: typeHeading
    readonly property int typeTitle: typeHeading
    readonly property int typeDisplay: typeHeading

    /// The UI face. Decision 2 (§14) keeps Chinese on the system face, so this
    /// names the platform's own sans rather than shipping Inter yet; P6 swaps
    /// this for the bundled `Inter` once the font is in qrc.
    readonly property string uiFamily: Qt.platform.os === "osx" ? "Helvetica Neue"
        : Qt.platform.os === "windows" ? "Segoe UI"
        : "Noto Sans"

    /// Fixed-pitch for terminals, diffs, sizes and rates. Chosen per platform
    /// because QML cannot express a fallback list on `font.family`. P6 ships
    /// JetBrains Mono and this becomes one name on all three platforms.
    readonly property string monoFamily: Qt.platform.os === "osx" ? "SF Mono"
        : Qt.platform.os === "windows" ? "Consolas"
        : "DejaVu Sans Mono"

    // ── Leading ─────────────────────────────────────────────────────────────
    /// Line height as a multiple of the type size, not as a pixel value: Qt's
    /// `lineHeight` is proportional by default (`Text.ProportionalHeight`).
    ///
    ///   tight   a single-line chip, a keycap, a heading
    ///   body    the default
    ///   prose   a paragraph someone actually reads
    readonly property real leadingTight: 1.3
    readonly property real leadingBody: 1.45
    readonly property real leadingProse: 1.6

    // ── Geometry (proposal §6.4) ────────────────────────────────────────────
    /// Four radii, and only four: a pill, a row, an overlay, a dialog.
    /// Concentric: an inner radius is the outer one minus the padding.
    readonly property int radiusXs: 3
    readonly property int radiusSm: 6
    readonly property int radiusMd: 8
    readonly property int radiusLg: 12

    /// The width of a structural rule. §6.4.3 wants one *device* pixel, which
    /// at 125 % is `1 / 1.25` dp; `Screen` is not reachable from a singleton,
    /// so the P6 pixel audit converts the four call sites. Until then this is
    /// one dp, which Qt draws with `pixelAligned` borders.
    readonly property int edgeWidth: 1

    /// Row heights; the header is always the paired value.
    readonly property int rowCompact: 28
    readonly property int rowDefault: 32
    readonly property int rowSpacious: 40

    /// Control heights — one ladder for everything the pointer can hit, on the
    /// 4 dp grid, five steps and no more.
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

    /// The chrome bands (§8.1). The tab row is 38 dp and the native side's
    /// `kTitleBarDp` is the same number; moving one without the other leaves an
    /// invisible drag strip over the other row.
    readonly property int shellTitle: 38
    readonly property int shellNav: 44
    readonly property int shellStatus: 28
    /// The status bar's other state: 36 dp while something is selected. The
    /// change happens once, on the first selection, so the list does not jump
    /// on every click.
    readonly property int actionBar: 36

    /// Panel widths (§6.4.1). Every one is draggable within its band.
    readonly property int sidebarWidth: 240
    readonly property int sidebarMin: 180
    readonly property int sidebarMax: 300
    readonly property int inspectorWidth: 300
    readonly property int inspectorMin: 260
    readonly property int inspectorMax: 420
    readonly property int terminalHeight: 240
    readonly property int terminalMin: 160
    readonly property int terminalMax: 480

    /// How far the nine-patch shadow reaches outside its plate. The source
    /// texture's 48 px border is used at 1:1, so the falloff keeps the shape it
    /// was drawn with.
    readonly property int shadowMargin: 48
}
