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
//   2. Shape follows the M3 corner scale: 4 / 8 / 12 / 16 / 28 / 48 / full.
//      Buttons and chips are pills, dialogs take the 28 dp extra-large corner,
//      menus and list selections the 12 dp medium one.
//   3. Type uses the M3 scale translated to the app's density: label 11/12,
//      body 14, title 17, headline 22. Emphasized weights (500/600) mark the
//      selected and primary thing, matching the "emphasized" type styles.
//   4. Motion is the M3 Expressive spring system: spatial springs (position,
//      size, corner radius) overshoot into place, effects springs (colour,
//      opacity) do not. The tokens are calibrated Qt `SpringAnimation` pairs,
//      and the whole scheme can drop to the calmer "standard" preset.
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
    /// The app's own seed colour. In M3 terms this is the key colour's tone 40
    /// (light) / tone 80 (dark). Kept separate from `accent` because the
    /// settings panel shows this swatch even when the preference is "system".
    readonly property color brandAccent: dark ? "#6bcac9" : "#007a7b"
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
        ? hslColor(accentHue, 0.34, 0.24)
        : hslColor(accentHue, 0.34, 0.86)
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
    readonly property color inkOnSurface: dark ? "#dfe4e2" : "#181c1b"
    readonly property color inkOnSurfaceVariant: dark ? "#bec9c6" : "#3f4947"
    readonly property color inkOnDanger: inkOnError

    // ── M3 surfaces ─────────────────────────────────────────────────────────
    /// The tonal surface ladder. Light elevation steps *down* in tone
    /// (surface is brightest); dark elevation steps up, as the model specifies.
    readonly property color surface: dark ? "#0f1413" : "#f7faf9"
    readonly property color surfaceDim: dark ? "#0a0e0d" : "#d9dedc"
    readonly property color surfaceBright: dark ? "#353a39" : "#f7faf9"
    readonly property color surfaceContainerLowest: dark ? "#0a0e0d" : "#ffffff"
    readonly property color surfaceContainerLow: dark ? "#171c1b" : "#f1f5f3"
    readonly property color surfaceContainer: dark ? "#1b201f" : "#ebefee"
    readonly property color surfaceContainerHigh: dark ? "#252a29" : "#e4e9e7"
    readonly property color surfaceContainerHighest: dark ? "#303534" : "#dee4e2"

    readonly property color outline: dark ? "#889390" : "#6f7977"
    readonly property color outlineVariant: dark ? "#3f4947" : "#bec9c6"
    readonly property color inverseSurface: dark ? "#dfe4e2" : "#2d3130"
    readonly property color inverseOnSurface: dark ? "#2c3130" : "#eff1f0"
    readonly property color inversePrimary: dark
        ? withLightness(accent, 0.34) : withLightness(accent, 0.76)

    // ── Legacy surface aliases ──────────────────────────────────────────────
    /// The window's bottom layer, visible only in the gutters.
    readonly property color window: surfaceDim
    /// The frame: tab row, sidebar, status bar, inspector.
    readonly property color chrome: surfaceContainerLow
    /// Content: the listing, the grid, the preview body.
    readonly property color canvas: surface
    /// A surface that floats above everything: menus, dialogs, the palette.
    readonly property color raised: surfaceContainerHigh
    /// Whether floating surfaces frost the scene behind them. Off (a settings
    /// switch for weak GPUs) makes `Glass.qml` paint the solid plate and skip
    /// the grab/blur passes entirely.
    readonly property bool glassEnabled: Appearance.frostedGlass
    /// The frosted-glass veil painted over the blurred scene behind a floating
    /// plate (`Glass.qml`). macOS materials tint *neutral* — an achromatic
    /// grey/white — not the app's seed colour: a seed-tinted veil read green
    /// over the blur. Kept a touch cool (blue ≥ red ≥ green) so it can never
    /// cast green, and translucent enough that the vibrancy reads through
    /// while text keeps its contrast.
    readonly property color glassTint: dark ? Qt.rgba(0.102, 0.102, 0.110, 0.68)
                                            : Qt.rgba(0.965, 0.965, 0.973, 0.74)
    /// The hairline edge of a glass plate. macOS separates a material from its
    /// backdrop with a faint stroke — light-on-dark in the dark theme,
    /// dark-on-light in the light theme.
    readonly property color glassEdge: dark ? Qt.rgba(1, 1, 1, 0.14)
                                            : Qt.rgba(0, 0, 0, 0.12)
    /// The one-pixel top highlight inside a glass plate's corner: caught light
    /// on the material's upper edge.
    readonly property color glassHighlight: dark ? Qt.rgba(1, 1, 1, 0.08)
                                                 : Qt.rgba(1, 1, 1, 0.65)
    /// An input well: text fields, search, path editing.
    readonly property color field: surfaceContainerHighest
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
    readonly property color inkTertiary: dark ? "#a3afac" : "#5c6664"

    // ── Lines ───────────────────────────────────────────────────────────────
    /// Region boundaries. `outlineVariant` is the M3 divider role.
    readonly property color hairline: outlineVariant
    /// A control's edge.
    readonly property color line: outline
    /// The edge of something that has to be seen: a drop target, a focus ring.
    readonly property color lineStrong: inkOnSurfaceVariant
    /// M3 floating surfaces are separated by tone and elevation, not by a
    /// stroke. Kept as a token because call sites still set a border.
    readonly property color overlayLine: "transparent"

    // ── State ───────────────────────────────────────────────────────────────
    /// M3's 8 % on-surface state layer.
    readonly property color hover: Qt.rgba(inkOnSurface.r, inkOnSurface.g,
                                          inkOnSurface.b, dark ? 0.08 : 0.06)
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

    /// M3 Expressive runs on springs, not fixed curves: spatial springs move
    /// position, size and corner radius and overshoot; effects springs move
    /// colour and opacity and never do. Qt's `SpringAnimation` integrates a
    /// normalised spring (its `spring`/`damping` are not physical stiffness and
    /// damping ratio — large stiffness values diverge), so the six tokens below
    /// were calibrated against the scheme's published overshoot and settle
    /// targets by sampling the animation headless:
    ///
    ///   spatial fast     ~14 % overshoot, ~170 ms  (press morph, small parts)
    ///   spatial default  ~8 % overshoot, ~270 ms   (menus, dialogs, panels)
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
    ///   labelSmall 11 / labelMedium 12
    ///   bodyMedium 14
    ///   titleMedium 17
    ///   titleLarge 22 / headlineSmall 24 (hero moments only)
    readonly property int typeMicro: 11
    readonly property int typeCaption: 12
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
    /// The M3 Expressive corner scale. Components map to roles:
    ///
    ///   xs 4    keycaps, tiny marks
    ///   sm 8    nested thumbnails, compact tags
    ///   md 12   menus, list selection, text fields
    ///   lg 16   cards, panels, grid selection
    ///   xl 28   dialogs, search surfaces, command palette
    ///   xxl 48  hero containers
    ///   full    buttons, chips, nav indicators, search fields
    readonly property int radiusXs: 4
    readonly property int radiusSm: 8
    readonly property int radiusMd: 12
    readonly property int radiusLg: 16
    readonly property int radiusLgIncreased: 20
    readonly property int radiusXl: 28
    readonly property int radiusXlIncreased: 32
    readonly property int radiusXxl: 48
    readonly property int radiusFull: 999

    /// Shape morph (m3.material.io/styles/shape/shape-morph). A round control
    /// squishes toward this corner while it is held, then springs back to the
    /// pill on release — the press is felt in the silhouette, not only in the
    /// colour. Selected toggle segments rest at this corner too, so "chosen"
    /// reads as a shape change rather than a fill alone. At desktop control
    /// heights (28–32 dp) the small corner is far enough from the pill's
    /// half-height that the morph actually reads.
    readonly property int radiusMorphPress: 8

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

    /// The chrome bands. The tab row is 38 dp and the native side's
    /// `kTitleBarDp` is the same number.
    readonly property int shellTitle: 38
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
