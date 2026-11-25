pragma Singleton
import QtQuick
// The Rust singletons (`Appearance`, `Workspace`) live in module `dae`, which
// the shell's own files reach through the qrc path's implicit import. A
// singleton has no such implicit import, so it is named here.
import dae

// The shell's single source of colour, shape, type and motion.
//
// The design language is Material 3 Expressive (m3.material.io): colour roles
// rather than raw greys, the tonal surface ladder, the ten-step corner scale,
// the emphasized type styles, and spring-like motion. It is applied at desktop
// density — rows stay 32 dp, controls 28–32 dp — because this is a file
// manager, not a phone. Everything below is still one token layer:
//
//   1. Colour follows M3 roles. `primary`/`inkOnPrimary`/`primaryContainer` and
//      the `surfaceContainer*` ladder replace the old "four greys plus an
//      accent" idea. A selected row is `secondaryContainer`, a floating
//      surface is `surfaceContainerHigh`, and the model keeps the seed colour
//      (system accent or brand teal) flowing into every derived role.
//   2. Shape follows the desktop corner scale: 3 / 6 / 8 / 12. Buttons, tabs,
//      rows and fields take the 6 dp small corner; tooltips, cards and menu
//      rows the 8 dp medium one; menus, dialogs and the palette the 12 dp
//      large one. Only
//      genuinely circular things (icon buttons, switches, dots) are round.
//      This is Material 3 as applied to large screens, and the same restraint
//      Chrome's desktop surfaces use: a 28 dp dialog corner or a 30 dp pill
//      tab belongs to a phone, not to a window with a pointer.
//   3. Type uses the M3 scale translated to the app's density: label 11/12,
//      body 14, title 17, headline 22. Emphasized weights (500/600) mark the
//      selected and primary thing, matching the "emphasized" type styles.
//   4. Motion is the M3 Expressive spring system, used where a desktop app
//      wants it: switches and sliders settle on a spatial spring, while
//      colour and opacity ride effects curves. Surfaces do not pop or squish
//      into place; a menu or dialog appears where it belongs.
//
// Colour values remain literals (or contrast-guarded derivations) so the
// acceptance gate in `docs/ui-redesign/check-contrast.mjs` can still measure
// every pair. The legacy Graphite names are retained as aliases so no call
// site breaks; new work should use the M3 role names.
QtObject {
    id: theme

    // ── Source ──────────────────────────────────────────────────────────────
    /// Mirrors `ShellWindow.dark` exactly. Every component already receives
    /// `dark` as a property; this exists so a surface that is constructed
    /// outside the shell tree (a dialog, a tear-off window, a toast) still
    /// resolves the right palette without being handed the flag.
    readonly property bool dark: Appearance.theme === "dark"
        || (Appearance.theme === "system" && Workspace.systemDark)

    // ── Seed & interaction ──────────────────────────────────────────────────
    /// The app's own seed colour — Chrome GM3 Blue (tone 40 light / tone 80 dark).
    readonly property color brandAccent: dark ? "#a8c7fa" : "#0b57d0"
    /// `primary` in M3 role terms. The preference decides its source: "system"
    /// follows the desktop's accent reading, "brand" stays on the app's teal,
    /// and "custom" uses the seed the user picked in the settings palette.
    readonly property color accent: Appearance.accent === "custom"
        && Appearance.customAccent !== "" ? Appearance.customAccent
        : (Appearance.accent === "system" && Workspace.accent !== ""
            ? Workspace.accent : brandAccent)
    /// M3 primary. The alias exists so new components can speak roles while
    /// old ones keep saying `accent`.
    readonly property color primary: accent
    /// Primary under the pointer. M3 raises a filled control with an 8–10 %
    /// on-primary state layer, which lightens a dark seed and lifts a light one.
    readonly property color accentHover: mix(accent, inkOnAccent, 0.10)
    /// Ink on a fill that is saturated in both themes and belongs to someone
    /// else: a provider tile, the Windows close button, an error toast.
    readonly property color inkOnSolid: "#ffffff"
    /// WCAG relative luminance of the resolved accent, used only to pick the
    /// ink that sits on top of it.
    readonly property real accentLuminance: luminance(accent)
    readonly property real accentHue: rgbToHsl(accent).h

    // ── M3 colour roles ─────────────────────────────────────────────────────
    /// Primary container: a pale seed wash in light, a deep seed in dark.
    /// Derived from the resolved seed so a desktop accent moves the whole
    /// family, with the container ink contrast-guarded.
    readonly property color primaryContainer: dark
        ? mix(accent, window, 0.34) : mix(accent, white, 0.84)
    /// Secondary in M3 is the seed hue at low chroma: it carries selection
    /// without shouting.
    readonly property color secondary: dark
        ? withLightness(desaturate(accent, 0.20), 0.76)
        : withLightness(desaturate(accent, 0.20), 0.34)
    /// Selection is the most frequent colour in the shell: tabs, sidebar
    /// destinations, list rows and grid tiles all wear it at once. It keeps
    /// more chroma than a strict M3 secondary would, because a desktop window
    /// shows a dozen selected things side by side and a grey pill reads as
    /// "disabled" rather than "chosen".
    readonly property color secondaryContainer: dark
        ? hslColor(accentHue, 0.36, 0.25)
        : hslColor(accentHue, 0.72, 0.90)
    /// Tertiary is the seed rotated 60°: the expressive counterweight used for
    /// badges, pinned marks and one-off emphasis.
    readonly property color tertiary: dark
        ? hslColor(accentHue + 60, 0.42, 0.78)
        : hslColor(accentHue + 60, 0.38, 0.36)
    readonly property color tertiaryContainer: dark
        ? hslColor(accentHue + 60, 0.38, 0.27)
        : hslColor(accentHue + 60, 0.52, 0.90)

    /// Error roles. The filled error colour stays the shell's existing red;
    /// the containers are the M3 pairing.
    readonly property color error: dark ? "#f47b74" : "#ba2b2e"
    readonly property color errorContainer: dark ? "#8c1d18" : "#ffdad6"

    // ── Ink on role colours ─────────────────────────────────────────────────
    // Qt reserves the `on…` prefix for signal handlers: a property named
    // `on<Role>` whose `<Role>` is also a property of this object loses its
    // binding at runtime and silently reads the default #000000 (`onSurface`
    // next to `surface`, `onAccent` next to `accent`). The ink roles are
    // therefore spelled `inkOn<Role>`; the comment above each still names the
    // M3 role it answers.
    /// The printer's poles as colours, not hex strings: the helpers read
    /// `.r/.g/.b`, and a string argument would hand them NaN.
    readonly property color white: "#ffffff"
    readonly property color black: "#000000"
    /// The preferred ink over the filled error role and the error container,
    /// before `readableInk` verifies the pair clears 4.5:1.
    readonly property color errorSolidInk: dark ? "#3a0b0a" : white
    readonly property color errorContainerInk: dark ? "#ffdad6" : "#410e0b"
    /// Ink on `primary`.
    readonly property color inkOnAccent: accentLuminance > 0.45
        ? (dark ? "#0b0f0e" : "#101413") : "#ffffff"
    readonly property color inkOnPrimary: inkOnAccent
    readonly property color inkOnPrimaryContainer: readableInk(primaryContainer,
        dark ? mix(accent, white, 0.82) : mix(accent, black, 0.72))
    readonly property color inkOnSecondary: readableInk(secondary)
    readonly property color inkOnSecondaryContainer: readableInk(secondaryContainer,
        dark ? hslColor(accentHue, 0.55, 0.93)
             : hslColor(accentHue, 0.55, 0.13))
    readonly property color inkOnTertiary: readableInk(tertiary)
    readonly property color inkOnTertiaryContainer: readableInk(tertiaryContainer,
        dark ? hslColor(accentHue + 60, 0.65, 0.90)
             : hslColor(accentHue + 60, 0.60, 0.14))
    readonly property color inkOnError: readableInk(error, errorSolidInk)
    readonly property color inkOnErrorContainer: readableInk(errorContainer,
        errorContainerInk)
    readonly property color inkOnSurface: dark ? "#e8eaed" : "#1f1f1f"
    readonly property color inkOnSurfaceVariant: dark ? "#c4c7c5" : "#444746"
    readonly property color inkOnDanger: inkOnError

    // ── M3 surfaces (Chrome Desktop GM3 tonal ladder) ───────────────────────
    /// The tonal surface ladder tuned to Chrome Desktop: iconic periwinkle-blue
    /// tab strip (`tabStrip` #d3e3fd in light / #1f2024 in dark), pure white /
    /// lifted slate toolbar + active tab (`toolbar`), soft cool sidebar
    /// (`chrome`), and `#edf2fa` / `#202124` Omnibox well (`field`).
    readonly property color surface: dark ? "#202124" : "#ffffff"
    readonly property color surfaceDim: dark ? "#191a1d" : "#e6ebf4"
    readonly property color surfaceBright: dark ? "#383b42" : "#ffffff"
    readonly property color surfaceContainerLowest: dark ? "#17181b" : "#ffffff"
    readonly property color surfaceContainerLow: dark ? "#25272b" : "#f3f6fc"
    readonly property color surfaceContainer: dark ? "#2c2e33" : "#edf2fa"
    readonly property color surfaceContainerHigh: dark ? "#303238" : "#ffffff"
    readonly property color surfaceContainerHighest: dark ? "#202124" : "#edf2fa"

    readonly property color outline: dark ? "#8e918f" : "#747775"
    readonly property color outlineVariant: dark ? "#3c4043" : "#e1e3e1"
    readonly property color inverseSurface: dark ? "#e8eaed" : "#303134"
    readonly property color inverseOnSurface: dark ? "#202124" : "#f8f9fa"
    readonly property color inversePrimary: dark
        ? withLightness(accent, 0.34) : withLightness(accent, 0.76)

    // ── Legacy & Chrome surface aliases ─────────────────────────────────────
    /// The window's bottom layer, visible only in the gutters.
    readonly property color window: surfaceDim
    /// The frame: sidebar, status bar, inspector.
    readonly property color chrome: surfaceContainerLow
    /// The tab row's own band: Chrome Refresh 2023 `#d3e3fd` in default light
    /// mode (and dynamically seed-tinted when custom accent is chosen).
    readonly property color tabStrip: dark
        ? (Appearance.accent === "brand" ? "#1f2024" : mix(accent, "#1f2024", 0.08))
        : (Appearance.accent === "brand" ? "#d3e3fd" : mix(accent, "#e9effa", 0.18))
    readonly property color tabStripInactive: dark
        ? "#292b30"
        : "#e8eef7"
    /// Leading tab-search button well inside the tab strip.
    readonly property color tabSearchFill: dark ? "#2d3036" : "#edf2fa"
    readonly property color tabSearchHover: dark ? "#383c44" : "#dde7fb"
    /// The toolbar and active tab share this exact surface so the active tab
    /// connects seamlessly into the toolbar below it without a seam.
    readonly property color toolbar: dark ? "#35373c" : "#ffffff"
    readonly property color tabActive: toolbar
    /// Inactive tab hover pill and vertical divider between tabs and before `+`.
    readonly property color tabHover: dark
        ? Qt.rgba(1, 1, 1, 0.10) : Qt.rgba(0.043, 0.341, 0.816, 0.10)
    readonly property color tabDivider: dark
        ? Qt.rgba(1, 1, 1, 0.22) : "#a8c7fa"
    /// Content: the listing, the grid, the preview body.
    readonly property color canvas: surface
    /// A surface that floats above everything: menus, dialogs, the palette.
    readonly property color raised: surfaceContainerHigh
    /// Whether floating surfaces frost the scene behind them. Off (a settings
    /// switch for weak GPUs) makes `Glass.qml` paint the solid plate and skip
    /// the grab/blur passes entirely.
    readonly property bool glassEnabled: Appearance.frostedGlass
    /// The frosted-glass veil painted over the blurred scene behind a floating
    /// plate (`Glass.qml`).
    readonly property color glassTint: dark ? Qt.rgba(0.145, 0.153, 0.176, 0.82)
                                            : Qt.rgba(1.0, 1.0, 1.0, 0.86)
    /// The hairline edge of a glass plate.
    readonly property color glassEdge: dark ? Qt.rgba(1, 1, 1, 0.14)
                                            : Qt.rgba(0, 0, 0, 0.10)
    /// The one-pixel top highlight inside a glass plate's corner.
    readonly property color glassHighlight: dark ? Qt.rgba(1, 1, 1, 0.08)
                                                 : Qt.rgba(1, 1, 1, 0.75)
    /// An input well: Omnibox resting, search, path editing.
    readonly property color field: surfaceContainerHighest
    readonly property color fieldHover: dark ? "#2b2d33" : "#e3eaf6"
    /// A surface *inside* a surface: preview wells, tag pills, cards.
    readonly property color inset: surfaceContainerLow

    /// Whether a background image is set. The listing surfaces below switch
    /// from the solid canvas to a translucent one so the image shows through;
    /// chrome stays solid, which keeps the frame readable.
    readonly property bool hasBackground: ExplorerPrefs.backgroundImage !== ""
    /// How much canvas stays over the image: 1 is solid, 0.1 is the floor.
    readonly property real contentAlpha: hasBackground
        ? Math.max(0.1, 1 - ExplorerPrefs.backgroundDim / 100) : 1
    /// The listing canvas. Identical to `canvas` without a background image.
    readonly property color content: Qt.rgba(canvas.r, canvas.g, canvas.b,
                                             contentAlpha)

    // ── Material tint ───────────────────────────────────────────────────────
    /// The veil painted over a native backdrop. Only the chrome bar (`panel`
    /// false) stays translucent so the backdrop reads through it; every panel
    /// surface is solid, which confines the material to the tab bar alone.
    function backdropTint(material, isDark, panel) {
        if (material === "none" || panel)
            return isDark ? theme.window : theme.chrome
        const blur = material === "blur"
        const alpha = blur
            ? (isDark ? 0.78 : 0.86)
            : (isDark ? 0.38 : 0.46)
        return isDark
            ? Qt.rgba(0.031, 0.047, 0.043, alpha)
            : Qt.rgba(0.945, 0.961, 0.953, alpha)
    }

    // ── Ink ─────────────────────────────────────────────────────────────────
    readonly property color ink: inkOnSurface
    readonly property color inkSecondary: inkOnSurfaceVariant
    /// Tertiary text is one step lighter than `onSurfaceVariant` so an 11 px
    /// label still clears 4.5:1 on the canvas.
    readonly property color inkTertiary: dark ? "#9aa0a6" : "#5f6368"

    // ── Lines ───────────────────────────────────────────────────────────────
    /// Region boundaries. `outlineVariant` is the M3 divider role.
    readonly property color hairline: outlineVariant
    /// A control's edge.
    readonly property color line: outline
    /// The edge of something that has to be seen: a drop target, a focus ring.
    readonly property color lineStrong: inkOnSurfaceVariant
    /// Floating surfaces keep a crisp 1px hairline in Chrome Desktop.
    readonly property color overlayLine: dark ? Qt.rgba(1, 1, 1, 0.10) : Qt.rgba(0, 0, 0, 0.08)

    // ── State ───────────────────────────────────────────────────────────────
    /// M3's 8 % on-surface state layer.
    readonly property color hover: Qt.rgba(inkOnSurface.r, inkOnSurface.g,
                                          inkOnSurface.b, dark ? 0.08 : 0.06)
    readonly property color ripple: Qt.rgba(inkOnSurface.r, inkOnSurface.g,
                                           inkOnSurface.b, dark ? 0.14 : 0.11)
    readonly property color accentRipple: Qt.rgba(inkOnAccent.r, inkOnAccent.g,
                                                 inkOnAccent.b, 0.20)
    /// Behind a modal. M3 scrim is 32 % black.
    readonly property color scrim: "#52000000"
    /// The selected row / tile / menu item. In M3 that is the secondary
    /// container, not a wash of the accent.
    readonly property color selection: secondaryContainer
    readonly property color selectionInk: inkOnSecondaryContainer

    // ── Accent-derived washes ───────────────────────────────────────────────
    /// Hover, focus and drop washes that are not a full selected container.
    readonly property color accentFaint: Qt.rgba(accent.r, accent.g, accent.b,
                                                 dark ? 0.10 : 0.06)
    readonly property color accentSoft: Qt.rgba(accent.r, accent.g, accent.b,
                                                dark ? 0.24 : 0.12)
    readonly property color accentSoftStrong: Qt.rgba(accent.r, accent.g,
                                                      accent.b,
                                                      dark ? 0.40 : 0.22)

    // ── Meaning (legacy names) ──────────────────────────────────────────────
    readonly property color danger: error
    readonly property color dangerSolid: error
    readonly property color dangerSolidHover: mix(error, inkOnError, 0.10)
    readonly property color success: dark ? "#7bc495" : "#227849"
    readonly property color warning: dark ? "#e6b55d" : "#8d5a00"

    // ── File families ───────────────────────────────────────────────────────
    /// Fourteen families at constant lightness and chroma, hue only. The two
    /// tables are literals so the contrast script can read them; `fileColor()`
    /// is the only accessor, and an unknown family falls back to `other`.
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

    /// A folder's silhouette. The family colour carries both the outline and
    /// the wash, so the mark is one hue; the neutral ink it used to borrow left
    /// a grey edge around a beige shape, which at 18 dp read as dirt rather
    /// than as a folder.
    readonly property color folderOutline: dark
        ? fileColor("folder")
        : mix(fileColor("folder"), black, 0.78)

    /// The eight tag colours: fixed options at one lightness, because a free
    /// picker cannot promise that any two tags stay legible on both themes.
    readonly property var tagPaletteLight: [
        "#a3392f", "#9a5a20", "#7d6a12", "#2f6b34",
        "#1d6560", "#2b5590", "#5b3f92", "#5c6169"
    ]
    readonly property var tagPaletteDark: [
        "#e08585", "#e0a377", "#ddc274", "#8fc48c",
        "#7fc3bd", "#82a9dd", "#ab93dd", "#a8adb4"
    ]
    readonly property var tagColors: dark ? tagPaletteDark : tagPaletteLight
    readonly property var tagLegacyMap: ({
            "#4c8dff": 5,
            "#e0524d": 0,
            "#e5a13a": 1,
            "#3fa66b": 3,
            "#8b6cf0": 6,
            "#d95fa3": 0,
            "#2aa9b8": 4,
            "#8a94a6": 7
        })

    /// The option index a stored colour stands for, or -1 for a hand-edited
    /// value outside the fixed eight.
    function tagColorIndex(value) {
        const key = String(value).toLowerCase()
        let index = tagPaletteLight.indexOf(key)
        if (index < 0)
            index = tagPaletteDark.indexOf(key)
        if (index < 0 && tagLegacyMap[key] !== undefined)
            index = tagLegacyMap[key]
        return index === undefined ? -1 : index
    }

    /// The current theme's paint for one of the eight options.
    function tagColor(index) {
        const clamped = Math.max(0, Math.min(tagColors.length - 1, index))
        return tagColors[clamped]
    }

    /// The colour to paint for a stored tag value. An out-of-palette value is
    /// kept verbatim so a hand-edited file is not silently recoloured.
    function tagColorFor(value) {
        const index = tagColorIndex(value)
        return index < 0 ? String(value) : tagColors[index]
    }

    // ── Terminal ────────────────────────────────────────────────────────────
    readonly property color terminalCanvas: canvas
    readonly property color terminalHeader: chrome
    readonly property color terminalInk: inkSecondary
    readonly property color terminalInkStrong: ink

    // ── Motion ──────────────────────────────────────────────────────────────
    /// Qt 6 has no system-wide reduced-motion query, so `explorer.ini` carries
    /// the switch and every `Behavior` asks this before it runs.
    readonly property bool motion: !ExplorerPrefs.reduceMotion
    /// The product-level motion scheme (m3.material.io/styles/motion). The
    /// expressive scheme overshoots into place; the standard scheme eases in
    /// without bounce, for people who want the calmer feel. Both honour
    /// `reduceMotion`, which collapses everything to an instant switch.
    readonly property bool expressive: ExplorerPrefs.motionScheme !== "standard"

    /// The remaining springs move the parts a desktop app expects to feel
    /// physical — the switch's thumb, the slider's handle — while colour and
    /// opacity ride fixed effects curves. Qt's `SpringAnimation` integrates a
    /// normalised spring (its `spring`/`damping` are not physical stiffness and
    /// damping ratio — large stiffness values diverge), so the six tokens below
    /// were calibrated against the scheme's published overshoot and settle
    /// targets by sampling the animation headless:
    ///
    ///   spatial fast     ~14 % overshoot, ~170 ms  (switch thumb, slider)
    ///   spatial default  ~8 % overshoot, ~270 ms   (moving panels)
    ///   spatial slow     gentle overshoot, ~350 ms (large surfaces)
    ///   effects fast     no overshoot, ~125 ms
    ///   effects default  no overshoot, ~220 ms
    ///   effects slow     no overshoot, ~310 ms
    ///
    /// The standard scheme raises the spatial damping until the overshoot is
    /// gone and keeps the effects springs, which already do not bounce.
    readonly property real springSpatialFast: 2.5
    readonly property real dampSpatialFast: expressive ? 0.20 : 0.40
    readonly property real springSpatial: 1.0
    readonly property real dampSpatial: expressive ? 0.15 : 0.30
    readonly property real springSpatialSlow: 0.7
    readonly property real dampSpatialSlow: expressive ? 0.16 : 0.30
    readonly property real springEffectsFast: 3.0
    readonly property real dampEffectsFast: 0.40
    readonly property real springEffects: 2.0
    readonly property real dampEffects: 0.40
    readonly property real springEffectsSlow: 1.5
    readonly property real dampEffectsSlow: 0.40

    /// Duration tokens for the render-thread animators that cannot spring
    /// (colour and opacity fades). They mirror the effects springs' settle
    /// times so a fade and a spring land together.
    readonly property int motionFast: 120
    readonly property int motionEffects: 180
    readonly property int motionSelect: 180
    readonly property int motionPanel: 280
    readonly property int motionOverlay: 220
    /// Legacy easing aliases. New spatial motion uses the springs above; these
    /// remain for the few `Behavior`s that still ride an animator.
    readonly property int motionEaseSpatial: Easing.OutBack
    readonly property int motionEaseEffects: Easing.OutCubic

    // ── Type ────────────────────────────────────────────────────────────────
    /// M3 roles at desktop density. Every value is an integer because Qt's
    /// `font.pixelSize` is an int.
    ///
    ///   labelSmall 11 (counts, badges only)
    ///   labelMedium/labelLarge 12 (column heads, section heads, status)
    ///   bodyMedium 13 (rows: the one rung a dense desktop list needs and the
    ///                 phone scale does not have)
    ///   bodyLarge 14 (prose, paths, single-line controls)
    ///   titleMedium 17
    ///   titleLarge 22 / headlineSmall 24 (hero moments only)
    readonly property int typeMicro: 11
    readonly property int typeCaption: 12
    /// The listing's own body rung. A 32 dp row holds 13 px text with 9–10 px
    /// of air above and below; 14 read as a phone list on a desktop window.
    readonly property int typeRow: 13
    readonly property int typeBody: 14
    readonly property int typeHeading: 17
    /// The command palette's query line: one rung above the title.
    readonly property int typeInput: 18
    /// Expressive hero sizes; use only for empty states and overview headers.
    readonly property int typeTitleLarge: 22
    readonly property int typeHeadlineSmall: 24

    /// Retired rung names, kept as aliases so an un-migrated call site cannot
    /// reintroduce a size outside the scale.
    readonly property int typeSmall: typeCaption
    readonly property int typeName: typeBody
    readonly property int typeEmphasis: typeHeading
    readonly property int typeTitle: typeHeading
    readonly property int typeDisplay: typeTitleLarge

    /// Emphasized weights, mirroring M3's emphasized type styles.
    readonly property int weightStrong: Font.Medium
    readonly property int weightEmphasis: Font.DemiBold

    /// The families Qt actually has.
    readonly property var installedFamilies: Qt.fontFamilies()

    function bundledFace(names, prefix) {
        for (let index = 0; index < names.length; index += 1) {
            if (theme.installedFamilies.indexOf(names[index]) >= 0)
                return names[index]
        }
        for (let index = 0; index < theme.installedFamilies.length; index += 1) {
            const family = String(theme.installedFamilies[index])
            if (family.indexOf(prefix) === 0)
                return family
        }
        return ""
    }

    /// The UI face: bundled Google Sans Flex, one variable file covering every
    /// weight, width and optical size the type scale uses.
    readonly property string uiFamily: {
        const custom = String(Appearance.appliedUiFont || "").trim()
        if (custom !== "" && theme.installedFamilies.indexOf(custom) >= 0)
            return custom
        const bundled = theme.bundledFace(["Google Sans Flex"], "Google Sans Flex")
        if (bundled !== "")
            return bundled
        return Qt.platform.os === "osx" ? "Helvetica Neue"
            : Qt.platform.os === "windows" ? "Segoe UI"
            : "Noto Sans"
    }

    /// Fixed-pitch for terminals, diffs, sizes and rates.
    readonly property string monoFamily: {
        const custom = String(Appearance.monoFontFamily || "").trim()
        if (custom !== "" && theme.installedFamilies.indexOf(custom) >= 0)
            return custom
        const bundled = theme.bundledFace(["Google Sans Code"], "Google Sans Code")
        if (bundled !== "")
            return bundled
        return Qt.platform.os === "osx" ? "SF Mono"
            : Qt.platform.os === "windows" ? "Consolas"
            : "DejaVu Sans Mono"
    }

    // ── Leading ─────────────────────────────────────────────────────────────
    readonly property real leadingTight: 1.3
    readonly property real leadingBody: 1.45
    readonly property real leadingProse: 1.6

    // ── Shape ───────────────────────────────────────────────────────────────
    /// The desktop corner scale. Components map to roles:
    ///
    ///   xs 3    keycaps, tags, the paper of a file mark, nested wells
    ///   sm 6    rows, tabs, buttons, chips, text fields, breadcrumb plates
    ///   md 8    menu rows, tooltips, floating cards, grid selection, thumbs
    ///   lg 12   menus, dialogs, the command palette, large floating panels
    ///   full    only true circles: icon-button footprints, switches, dots
    ///
    /// There is deliberately no larger rung. A 16 dp card or a 28 dp dialog
    /// reads as a phone surface; a frameless desktop window has a 10 dp frame
    /// and everything inside it stays at or below 12.
    readonly property int radiusXs: 3
    readonly property int radiusSm: 6
    readonly property int radiusMd: 8
    readonly property int radiusLg: 12
    /// Dialog-sized alias. Floating surfaces that own the screen (a dialog,
    /// the palette) name `lg` directly; this keeps the older call sites and
    /// their intent readable without a second, larger value.
    readonly property int radiusXl: radiusLg
    readonly property int radiusFull: 999

    /// The width of a structural rule: one device pixel.
    function ruleWidth(dpr) {
        return 1 / Math.max(1, dpr)
    }

    /// A control's border: one dp.
    readonly property int edgeWidth: 1

    /// Row heights; the header is always the paired value.
    readonly property int rowCompact: 28
    readonly property int rowDefault: 32
    readonly property int rowSpacious: 40

    /// Control heights — one ladder for everything the pointer can hit.
    readonly property int controlXs: 20
    readonly property int controlSm: 24
    readonly property int controlMd: 28
    readonly property int controlLg: 32
    readonly property int controlXl: 36

    /// The chrome bands. The tab row is 40 dp and the native side's
    /// `kTitleBarDp` is the same number.
    readonly property int shellTitle: 40
    readonly property int shellNav: 44
    readonly property int shellStatus: 28
    readonly property int actionBar: 36

    /// Panel widths. Every one is draggable within its band.
    readonly property int sidebarWidth: 240
    readonly property int sidebarMin: 180
    readonly property int sidebarMax: 300
    readonly property int inspectorWidth: 300
    readonly property int inspectorMin: 260
    readonly property int inspectorMax: 420
    readonly property int terminalHeight: 240
    readonly property int terminalMin: 160
    readonly property int terminalMax: 480

    /// How far the nine-patch shadow reaches outside its plate.
    readonly property int shadowMargin: 48

    // ── Colour helpers ──────────────────────────────────────────────────────
    /// Linear interpolation in sRGB. `t` is the weight of `a`.
    function mix(a, b, t) {
        return Qt.rgba(a.r * t + b.r * (1 - t),
                       a.g * t + b.g * (1 - t),
                       a.b * t + b.b * (1 - t),
                       a.a * t + b.a * (1 - t))
    }

    function rgbToHsl(c) {
        const r = c.r
        const g = c.g
        const b = c.b
        const max = Math.max(r, g, b)
        const min = Math.min(r, g, b)
        const lightness = (max + min) / 2
        const delta = max - min
        let hue = 0
        let saturation = 0
        if (delta > 0.00001) {
            saturation = lightness > 0.5
                ? delta / (2 - max - min) : delta / (max + min)
            if (max === r)
                hue = ((g - b) / delta + (g < b ? 6 : 0)) * 60
            else if (max === g)
                hue = ((b - r) / delta + 2) * 60
            else
                hue = ((r - g) / delta + 4) * 60
        }
        return { "h": hue, "s": saturation, "l": lightness, "a": c.a }
    }

    function hslColor(hue, saturation, lightness, alpha) {
        const h = (((hue % 360) + 360) % 360) / 360
        const s = Math.max(0, Math.min(1, saturation))
        const l = Math.max(0, Math.min(1, lightness))
        const q = l < 0.5 ? l * (1 + s) : l + s - l * s
        const p = 2 * l - q
        function channel(t) {
            let value = t
            if (value < 0)
                value += 1
            if (value > 1)
                value -= 1
            if (value < 1 / 6)
                return p + (q - p) * 6 * value
            if (value < 1 / 2)
                return q
            if (value < 2 / 3)
                return p + (q - p) * (2 / 3 - value) * 6
            return p
        }
        return Qt.rgba(channel(h + 1 / 3), channel(h), channel(h - 1 / 3),
                       alpha === undefined ? 1 : alpha)
    }

    function withLightness(c, lightness) {
        const hsl = rgbToHsl(c)
        return hslColor(hsl.h, hsl.s, lightness, hsl.a)
    }

    function desaturate(c, factor) {
        const hsl = rgbToHsl(c)
        return hslColor(hsl.h, hsl.s * factor, hsl.l, hsl.a)
    }

    function shiftHue(c, degrees) {
        const hsl = rgbToHsl(c)
        return hslColor(hsl.h + degrees, hsl.s, hsl.l, hsl.a)
    }

    function linear(channel) {
        return channel <= 0.03928
            ? channel / 12.92 : Math.pow((channel + 0.055) / 1.055, 2.4)
    }

    function luminance(c) {
        return 0.2126 * linear(c.r) + 0.7152 * linear(c.g)
            + 0.0722 * linear(c.b)
    }

    function contrastRatio(a, b) {
        const first = luminance(a)
        const second = luminance(b)
        return (Math.max(first, second) + 0.05)
            / (Math.min(first, second) + 0.05)
    }

    /// Picks an ink for `background` that clears 4.5:1. `preferred` is the
    /// role-correct choice; the fallback is the opposite pole.
    function readableInk(background, preferred) {
        const candidate = preferred !== undefined
            ? preferred : (luminance(background) > 0.45 ? ink : "#ffffff")
        if (contrastRatio(background, candidate) >= 4.5)
            return candidate
        return luminance(background) > 0.45 ? "#101413" : "#ffffff"
    }
}
