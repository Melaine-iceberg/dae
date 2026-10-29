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
    /// The terminal is the one surface that keeps its own near-black in both
    /// themes: a terminal that turns pale is no longer a terminal. Its header
    /// plate, quiet ink and hovered ink are part of that same fixed palette.
    readonly property color terminalCanvas: "#0b0b0c"
    readonly property color terminalHeader: "#141518"
    readonly property color terminalInk: "#8b9099"
    readonly property color terminalInkStrong: "#e8eaf0"

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
    /// The only corner radii the shell draws.
    readonly property int radiusXs: 2
    readonly property int radiusSm: 4
    readonly property int radiusMd: 8
    readonly property int radiusLg: 12
    /// Row heights; the header is always the paired value.
    readonly property int rowCompact: 28
    readonly property int rowDefault: 32
    readonly property int rowSpacious: 40
    /// The three chrome bands. Their sum is the 116 px shell.
    readonly property int shellTitle: 44
    readonly property int shellNav: 44
    readonly property int shellStatus: 28

    // ── Type scale ──────────────────────────────────────────────────────────
    /// Eight steps and no more. A screen shows at most four of them.
    readonly property real typeMicro: 11
    readonly property real typeCaption: 11.5
    readonly property real typeSmall: 12
    readonly property real typeBody: 12.5
    readonly property real typeName: 13
    readonly property real typeEmphasis: 13.5
    readonly property real typeTitle: 14
    readonly property real typeDisplay: 22
}
