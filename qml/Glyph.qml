import QtQuick

// A drawn 16 px mark, keyed by what it means.
//
// The shapes are the prototype's sprite — `docs/prototypes/dae-instrument-ui-
// 20261028.html` — with the geometry flattened to absolute `M`/`L`/`C`/`Q`/`Z`
// commands so nothing here needs an SVG parser or a second source of truth to
// drift from. A mark is paths in a 16-unit box and nothing else: callers tint
// it, and the file itself has no colours, because an icon is structure and the
// one signal colour belongs to what can be acted on.
//
// Two things this file does that a plain scale would not:
//
//  * The stroke is held at 1.2 device pixels rather than 1.2 *units*. The shell
//    draws marks at 10, 12 and 14 px as well as 16, and a uniform unit stroke
//    shrinks to 0.75 px at 10 — every edge lands between pixels and the mark
//    reads as grey haze instead of a line. Below 16 px the geometry is scaled
//    up in stroke only, which is the usual optical correction for a small icon.
//  * The folder is a filled silhouette, the way the prototype draws it. At
//    16 px a hollow folder is the lightest thing in a listing, and weight is
//    what makes one row distinguishable from the next at a glance.
//
// Canvas rather than rectangles because a roof line and a search handle are
// diagonals. It paints once per size or tint change, and the marks are shared
// by the surface that shows them — a hover retints one row's icon, not a page.
Item {
    id: root

    /// Navigation: "home", "desktop", "documents", "downloads", "pictures",
    /// "music", "videos", "search", "list", "grid", "column", "more", "branch",
    /// "panel", "terminal".
    /// Sidebar destinations: "drive", "network", "cloud", "trash", "diamond",
    /// "star", "clock", "plus", "gear".
    /// Files: "folder", "file", "image", "text", "code", "archive", "exe",
    /// "media", "link".
    /// Controls: "chevron" (and its left/up/down forms), "check", "close",
    /// "refresh", "eye", "info", "copy", "cut", "paste", "pencil", "external",
    /// "undo", "arrowLeft", "arrowRight", "arrowUp", "windowMin", "windowMax",
    /// "windowClose".
    /// Anything else draws nothing and the caller falls back to its label.
    property string kind: "home"
    property color tint: Theme.inkMuted
    /// Strength of a silhouette's fill, as a fraction of `tint`. At 0.3 the
    /// folder keeps its outline and reads as a shape rather than a block.
    property real wash: 0.3

    implicitWidth: 16
    implicitHeight: 16

    readonly property bool draws: marks[kind] !== undefined

    /// `f` fills the part solid; `s` fills it with `wash` and then strokes the
    /// outline; `w` overrides the stroke width for the marks that are drawn
    /// thinner on purpose (the window buttons).
    readonly property var marks: ({
        "home": [
            { d: "M2.6 7.4L8 2.9L13.4 7.4L13.4 13L9.8 13L9.8 9.8L6.2 9.8L6.2 13L2.6 13Z" }
        ],
        "desktop": [
            { d: "M3.4 3.2L12.6 3.2C13.15 3.2 13.6 3.65 13.6 4.2L13.6 9.4C13.6 9.95 13.15 10.4 12.6 10.4L3.4 10.4C2.85 10.4 2.4 9.95 2.4 9.4L2.4 4.2C2.4 3.65 2.85 3.2 3.4 3.2Z" },
            { d: "M8 10.4L8 13M5.6 13L10.4 13" }
        ],
        "documents": [
            { d: "M4.4 2.6L9.4 2.6L11.6 4.8L11.6 13.4L4.4 13.4Z" },
            { d: "M6.3 8L9.7 8M6.3 10.2L9.7 10.2M6.3 12.1L8.5 12.1" }
        ],
        "downloads": [
            { d: "M8 3L8 10M5 8.5L8 11.5L11 8.5M3 12.8L13 12.8" }
        ],
        "pictures": [
            { d: "M3.4 3L12.6 3C13.15 3 13.6 3.45 13.6 4L13.6 12C13.6 12.55 13.15 13 12.6 13L3.4 13C2.85 13 2.4 12.55 2.4 12L2.4 4C2.4 3.45 2.85 3 3.4 3Z" },
            { d: "M8.9 6.2C8.9 5.7 9.3 5.3 9.8 5.3C10.3 5.3 10.7 5.7 10.7 6.2C10.7 6.7 10.3 7.1 9.8 7.1C9.3 7.1 8.9 6.7 8.9 6.2Z", f: true },
            { d: "M4.2 11.6L6.9 8.6L8.8 10.7L10.2 9.3L12 11.6" }
        ],
        "music": [
            { d: "M10.2 3.2L10.2 9.5" },
            { d: "M10.2 3.2L5.8 4.3L5.8 9.8" },
            { d: "M7 10.2C7 9.54 7.72 9 8.6 9C9.48 9 10.2 9.54 10.2 10.2C10.2 10.86 9.48 11.4 8.6 11.4C7.72 11.4 7 10.86 7 10.2Z" },
            { d: "M2.6 11.1C2.6 10.44 3.32 9.9 4.2 9.9C5.08 9.9 5.8 10.44 5.8 11.1C5.8 11.76 5.08 12.3 4.2 12.3C3.32 12.3 2.6 11.76 2.6 11.1Z" }
        ],
        "videos": [
            { d: "M3.4 3.4L12.6 3.4C13.15 3.4 13.6 3.85 13.6 4.4L13.6 11.6C13.6 12.15 13.15 12.6 12.6 12.6L3.4 12.6C2.85 12.6 2.4 12.15 2.4 11.6L2.4 4.4C2.4 3.85 2.85 3.4 3.4 3.4Z" },
            { d: "M6.8 6.2L9.8 8L6.8 9.8Z", f: true }
        ],
        "search": [
            { d: "M2.9 7C2.9 4.74 4.74 2.9 7 2.9C9.26 2.9 11.1 4.74 11.1 7C11.1 9.26 9.26 11.1 7 11.1C4.74 11.1 2.9 9.26 2.9 7Z", w: 1.25 },
            { d: "M10.2 10.2L13.4 13.4", w: 1.25 }
        ],
        "list": [
            { d: "M2.5 4L13.5 4M2.5 8L13.5 8M2.5 12L13.5 12", w: 1.25 }
        ],
        "grid": [
            { d: "M3.3 2.5L6.2 2.5C6.64 2.5 7 2.86 7 3.3L7 6.2C7 6.64 6.64 7 6.2 7L3.3 7C2.86 7 2.5 6.64 2.5 6.2L2.5 3.3C2.5 2.86 2.86 2.5 3.3 2.5Z", w: 1.25 },
            { d: "M9.8 2.5L12.7 2.5C13.14 2.5 13.5 2.86 13.5 3.3L13.5 6.2C13.5 6.64 13.14 7 12.7 7L9.8 7C9.36 7 9 6.64 9 6.2L9 3.3C9 2.86 9.36 2.5 9.8 2.5Z", w: 1.25 },
            { d: "M3.3 9L6.2 9C6.64 9 7 9.36 7 9.8L7 12.7C7 13.14 6.64 13.5 6.2 13.5L3.3 13.5C2.86 13.5 2.5 13.14 2.5 12.7L2.5 9.8C2.5 9.36 2.86 9 3.3 9Z", w: 1.25 },
            { d: "M9.8 9L12.7 9C13.14 9 13.5 9.36 13.5 9.8L13.5 12.7C13.5 13.14 13.14 13.5 12.7 13.5L9.8 13.5C9.36 13.5 9 13.14 9 12.7L9 9.8C9 9.36 9.36 9 9.8 9Z", w: 1.25 }
        ],
        "column": [
            { d: "M2.8 2.5L4.6 2.5C5.04 2.5 5.4 2.86 5.4 3.3L5.4 12.7C5.4 13.14 5.04 13.5 4.6 13.5L2.8 13.5C2.36 13.5 2 13.14 2 12.7L2 3.3C2 2.86 2.36 2.5 2.8 2.5Z", w: 1.25 },
            { d: "M7.1 2.5L8.9 2.5C9.34 2.5 9.7 2.86 9.7 3.3L9.7 12.7C9.7 13.14 9.34 13.5 8.9 13.5L7.1 13.5C6.66 13.5 6.3 13.14 6.3 12.7L6.3 3.3C6.3 2.86 6.66 2.5 7.1 2.5Z", w: 1.25 },
            { d: "M11.4 2.5L13.2 2.5C13.64 2.5 14 2.86 14 3.3L14 12.7C14 13.14 13.64 13.5 13.2 13.5L11.4 13.5C10.96 13.5 10.6 13.14 10.6 12.7L10.6 3.3C10.6 2.86 10.96 2.5 11.4 2.5Z", w: 1.25 }
        ],
        "more": [
            { d: "M2.2 8C2.2 7.45 2.65 7 3.2 7C3.75 7 4.2 7.45 4.2 8C4.2 8.55 3.75 9 3.2 9C2.65 9 2.2 8.55 2.2 8Z", f: true },
            { d: "M7 8C7 7.45 7.45 7 8 7C8.55 7 9 7.45 9 8C9 8.55 8.55 9 8 9C7.45 9 7 8.55 7 8Z", f: true },
            { d: "M11.8 8C11.8 7.45 12.25 7 12.8 7C13.35 7 13.8 7.45 13.8 8C13.8 8.55 13.35 9 12.8 9C12.25 9 11.8 8.55 11.8 8Z", f: true }
        ],
        "branch": [
            { d: "M2.7 12C2.7 11.17 3.37 10.5 4.2 10.5C5.03 10.5 5.7 11.17 5.7 12C5.7 12.83 5.03 13.5 4.2 13.5C3.37 13.5 2.7 12.83 2.7 12Z" },
            { d: "M10.3 4C10.3 3.17 10.97 2.5 11.8 2.5C12.63 2.5 13.3 3.17 13.3 4C13.3 4.83 12.63 5.5 11.8 5.5C10.97 5.5 10.3 4.83 10.3 4Z" },
            { d: "M10.3 10C10.3 9.17 10.97 8.5 11.8 8.5C12.63 8.5 13.3 9.17 13.3 10C13.3 10.83 12.63 11.5 11.8 11.5C10.97 11.5 10.3 10.83 10.3 10Z" },
            { d: "M4.2 10.5L4.2 6.2C4.2 5 5.2 4 6.4 4L10.3 4M11.8 5.5L11.8 8.5" }
        ],
        "panel": [
            { d: "M3.2 3L12.8 3C13.35 3 13.8 3.45 13.8 4L13.8 12C13.8 12.55 13.35 13 12.8 13L3.2 13C2.65 13 2.2 12.55 2.2 12L2.2 4C2.2 3.45 2.65 3 3.2 3Z" },
            { d: "M10 3L10 13" }
        ],
        "terminal": [
            { d: "M3.2 3L12.8 3C13.35 3 13.8 3.45 13.8 4L13.8 12C13.8 12.55 13.35 13 12.8 13L3.2 13C2.65 13 2.2 12.55 2.2 12L2.2 4C2.2 3.45 2.65 3 3.2 3Z" },
            { d: "M4.6 6.2L6.6 8L4.6 9.8M8.2 10.2L11.2 10.2" }
        ],
        "folder": [
            { d: "M2 5.1C2 4.4 2.6 3.8 3.3 3.8L6.4 3.8L7.8 5.3L12.8 5.3C13.5 5.3 14.1 5.9 14.1 6.6L14.1 13.4C14.1 14.1 13.5 14.7 12.8 14.7L3.3 14.7C2.6 14.7 2 14.1 2 13.4L2 5.1Z", s: true }
        ],
        "file": [
            { d: "M4.2 2.6L8.8 2.6L11.8 6L11.8 13.4L4.2 13.4Z" },
            { d: "M8.8 2.6L8.8 6L11.8 6" }
        ],
        "image": [
            { d: "M4.2 2.6L8.8 2.6L11.8 6L11.8 13.4L4.2 13.4Z" },
            { d: "M8.8 2.6L8.8 6L11.8 6" },
            { d: "M8.9 7.8C8.9 7.36 9.26 7 9.7 7C10.14 7 10.5 7.36 10.5 7.8C10.5 8.24 10.14 8.6 9.7 8.6C9.26 8.6 8.9 8.24 8.9 7.8Z", f: true },
            { d: "M5.7 11.6L7.3 9.8L8.4 11L9.2 10.2L10.3 11.6" }
        ],
        "code": [
            { d: "M4.2 2.6L8.8 2.6L11.8 6L11.8 13.4L4.2 13.4Z" },
            { d: "M8.8 2.6L8.8 6L11.8 6" },
            { d: "M6.4 8.1L5.2 9.4L6.4 10.7M9.6 8.1L10.8 9.4L9.6 10.7" }
        ],
        "archive": [
            { d: "M4.2 2.6L8.8 2.6L11.8 6L11.8 13.4L4.2 13.4Z" },
            { d: "M8.8 2.6L8.8 6L11.8 6" },
            { d: "M6.5 7.8L8.5 7.8M6.5 9.4L8.5 9.4M6.5 11L8.5 11" }
        ],
        "text": [
            { d: "M4.2 2.6L8.8 2.6L11.8 6L11.8 13.4L4.2 13.4Z" },
            { d: "M8.8 2.6L8.8 6L11.8 6" },
            { d: "M6 8.2L10 8.2M6 10.5L9 10.5" }
        ],
        "exe": [
            { d: "M4.2 2.6L8.8 2.6L11.8 6L11.8 13.4L4.2 13.4Z" },
            { d: "M8.8 2.6L8.8 6L11.8 6" },
            { d: "M7.2 8.4L9.5 9.7L7.2 11Z", f: true }
        ],
        "media": [
            { d: "M3.4 3.4L12.6 3.4C13.15 3.4 13.6 3.85 13.6 4.4L13.6 11.6C13.6 12.15 13.15 12.6 12.6 12.6L3.4 12.6C2.85 12.6 2.4 12.15 2.4 11.6L2.4 4.4C2.4 3.85 2.85 3.4 3.4 3.4Z" },
            { d: "M6.8 6.2L9.8 8L6.8 9.8Z", f: true }
        ],
        "link": [
            { d: "M6.6 9.4L9.4 6.6M6.2 4.4L7.2 3.4C8.22 2.38 9.88 2.38 10.9 3.4C11.92 4.42 11.92 6.08 10.9 7.1L9.9 8.1M9.8 11.6L8.8 12.6C7.78 13.62 6.12 13.62 5.1 12.6C4.08 11.58 4.08 9.92 5.1 8.9L6.1 7.9" }
        ],
        "drive": [
            { d: "M3.4 4.8L12.6 4.8C13.26 4.8 13.8 5.34 13.8 6L13.8 10C13.8 10.66 13.26 11.2 12.6 11.2L3.4 11.2C2.74 11.2 2.2 10.66 2.2 10L2.2 6C2.2 5.34 2.74 4.8 3.4 4.8Z" },
            { d: "M4.2 8.9L7.2 8.9" },
            { d: "M10.95 8.9C10.95 8.54 11.24 8.25 11.6 8.25C11.96 8.25 12.25 8.54 12.25 8.9C12.25 9.26 11.96 9.55 11.6 9.55C11.24 9.55 10.95 9.26 10.95 8.9Z", f: true }
        ],
        "network": [
            { d: "M6.5 3.4C6.5 2.57 7.17 1.9 8 1.9C8.83 1.9 9.5 2.57 9.5 3.4C9.5 4.23 8.83 4.9 8 4.9C7.17 4.9 6.5 4.23 6.5 3.4Z" },
            { d: "M2.3 12C2.3 11.17 2.97 10.5 3.8 10.5C4.63 10.5 5.3 11.17 5.3 12C5.3 12.83 4.63 13.5 3.8 13.5C2.97 13.5 2.3 12.83 2.3 12Z" },
            { d: "M10.7 12C10.7 11.17 11.37 10.5 12.2 10.5C13.03 10.5 13.7 11.17 13.7 12C13.7 12.83 13.03 13.5 12.2 13.5C11.37 13.5 10.7 12.83 10.7 12Z" },
            { d: "M7.2 4.8L4.5 10.6M8.8 4.8L11.5 10.6M5.3 12L10.7 12" }
        ],
        "cloud": [
            { d: "M4.9 11.8L11.1 11.8C12.48 11.86 13.64 10.78 13.7 9.4C13.76 8.02 12.68 6.86 11.3 6.8C11.09 5.36 10.01 4.2 8.58 3.89C7.16 3.57 5.69 4.18 4.9 5.4C3.13 5.4 1.7 6.83 1.7 8.6C1.7 10.37 3.13 11.8 4.9 11.8Z" }
        ],
        "trash": [
            { d: "M3.2 4.8L12.8 4.8M6 4.8L6 3.2L10 3.2L10 4.8M4.8 4.8L5.4 13L10.6 13L11.2 4.8M6.8 7.2L6.8 10.8M9.2 7.2L9.2 10.8" }
        ],
        "diamond": [
            { d: "M8 2.6L13.4 8L8 13.4L2.6 8Z" }
        ],
        "chevron": [
            { d: "M6 3.5L10.5 8L6 12.5", w: 1.25 }
        ],
        "chevronLeft": [
            { d: "M10 3.5L5.5 8L10 12.5", w: 1.25 }
        ],
        "chevronUp": [
            { d: "M3.5 10L8 5.5L12.5 10", w: 1.25 }
        ],
        "chevronDown": [
            { d: "M3.5 6L8 10.5L12.5 6", w: 1.25 }
        ],
        "check": [
            { d: "M3 8.3L6.3 11.6L13 4.8", w: 1.25 }
        ],
        "close": [
            { d: "M4 4L12 12M12 4L4 12", w: 1.25 }
        ],
        "plus": [
            { d: "M8 3L8 13M3 8L13 8", w: 1.25 }
        ],
        "clock": [
            { d: "M2.8 8C2.8 5.13 5.13 2.8 8 2.8C10.87 2.8 13.2 5.13 13.2 8C13.2 10.87 10.87 13.2 8 13.2C5.13 13.2 2.8 10.87 2.8 8Z" },
            { d: "M8 5.2L8 8L10 9.2" }
        ],
        "gear": [
            { d: "M3 5L4.6 5" },
            { d: "M7.4 5L13 5" },
            { d: "M4.6 5C4.6 4.23 5.23 3.6 6 3.6C6.77 3.6 7.4 4.23 7.4 5C7.4 5.77 6.77 6.4 6 6.4C5.23 6.4 4.6 5.77 4.6 5Z" },
            { d: "M3 11L8.6 11" },
            { d: "M11.4 11L13 11" },
            { d: "M8.6 11C8.6 10.23 9.23 9.6 10 9.6C10.77 9.6 11.4 10.23 11.4 11C11.4 11.77 10.77 12.4 10 12.4C9.23 12.4 8.6 11.77 8.6 11Z" }
        ],
        "star": [
            { d: "M8 2.8L9.38 6.1L12.95 6.39L10.23 8.73L11.06 12.21L8 10.35L4.94 12.21L5.77 8.73L3.05 6.39L6.62 6.1Z" }
        ],
        "refresh": [
            { d: "M13 8C12.98 10.36 11.31 12.39 9 12.87C6.68 13.34 4.35 12.12 3.41 9.95C2.47 7.78 3.18 5.25 5.11 3.88C7.04 2.52 9.67 2.69 11.4 4.3" },
            { d: "M13 3L13 6L10 6" }
        ],
        "eye": [
            { d: "M1.8 8C1.8 8 4 4.2 8 4.2C12 4.2 14.2 8 14.2 8C14.2 8 12 11.8 8 11.8C4 11.8 1.8 8 1.8 8Z" },
            { d: "M6.3 8C6.3 7.06 7.06 6.3 8 6.3C8.94 6.3 9.7 7.06 9.7 8C9.7 8.94 8.94 9.7 8 9.7C7.06 9.7 6.3 8.94 6.3 8Z" }
        ],
        "info": [
            { d: "M2.6 8C2.6 5.02 5.02 2.6 8 2.6C10.98 2.6 13.4 5.02 13.4 8C13.4 10.98 10.98 13.4 8 13.4C5.02 13.4 2.6 10.98 2.6 8Z" },
            { d: "M8 7.2L8 11.2M8 5.2L8.01 5.2" }
        ],
        "copy": [
            { d: "M6.4 5.4L11.6 5.4C12.15 5.4 12.6 5.85 12.6 6.4L12.6 11.6C12.6 12.15 12.15 12.6 11.6 12.6L6.4 12.6C5.85 12.6 5.4 12.15 5.4 11.6L5.4 6.4C5.4 5.85 5.85 5.4 6.4 5.4Z" },
            { d: "M10.6 5.4L10.6 3.4L3.4 3.4L3.4 10.6L5.4 10.6" }
        ],
        "cut": [
            { d: "M2.8 11.4C2.8 10.52 3.52 9.8 4.4 9.8C5.28 9.8 6 10.52 6 11.4C6 12.28 5.28 13 4.4 13C3.52 13 2.8 12.28 2.8 11.4Z" },
            { d: "M10 11.4C10 10.52 10.72 9.8 11.6 9.8C12.48 9.8 13.2 10.52 13.2 11.4C13.2 12.28 12.48 13 11.6 13C10.72 13 10 12.28 10 11.4Z" },
            { d: "M5.6 10.2L12.6 2.8M10.4 10.2L3.4 2.8" }
        ],
        "paste": [
            { d: "M5.4 3.2L4.2 3.2L4.2 13.4L11.8 13.4L11.8 3.2L10.6 3.2" },
            { d: "M6.6 2.2L9.4 2.2C9.73 2.2 10 2.47 10 2.8L10 3.8C10 4.13 9.73 4.4 9.4 4.4L6.6 4.4C6.27 4.4 6 4.13 6 3.8L6 2.8C6 2.47 6.27 2.2 6.6 2.2Z" },
            { d: "M6.2 7.2L9.8 7.2M6.2 9.4L9.8 9.4" }
        ],
        "pencil": [
            { d: "M3 13L3.8 10.4L10.6 3.6L12.4 5.4L5.6 12.2Z" },
            { d: "M9.8 4.4L11.6 6.2" }
        ],
        "external": [
            { d: "M9 3L13 3L13 7M13 3L7.6 8.4" },
            { d: "M12 9.4L12 13L3 13L3 4L6.6 4" }
        ],
        "undo": [
            { d: "M3 6.5L9.2 6.5C10.97 6.5 12.4 7.93 12.4 9.7C12.4 11.47 10.97 12.9 9.2 12.9L6 12.9" },
            { d: "M5.4 4L3 6.5L5.4 9" }
        ],
        "arrowLeft": [
            { d: "M12.5 8L3.5 8M7 4L3 8L7 12", w: 1.25 }
        ],
        "arrowRight": [
            { d: "M3.5 8L12.5 8M9 4L13 8L9 12", w: 1.25 }
        ],
        "arrowUp": [
            { d: "M8 13L8 3.5M4 7L8 3L12 7", w: 1.25 }
        ],
        "windowMin": [
            { d: "M4 8L12 8", w: 1 }
        ],
        "windowMax": [
            { d: "M4.2 4.2L11.8 4.2L11.8 11.8L4.2 11.8Z", w: 1 }
        ],
        "windowClose": [
            { d: "M4.5 4.5L11.5 11.5M11.5 4.5L4.5 11.5", w: 1 }
        ],
    })

    Canvas {
        id: canvas
        anchors.fill: parent
        visible: root.draws
        renderStrategy: Canvas.Immediate

        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            ctx.scale(width / 16, height / 16)
            ctx.strokeStyle = root.tint
            ctx.fillStyle = root.tint
            ctx.lineCap = "round"
            ctx.lineJoin = "round"

            // 1.2 device pixels at every size the shell asks for. The 16 px
            // design is the ceiling: above it the stroke scales normally.
            const boost = Math.max(1, 16 / Math.min(16, width))
            const parts = root.marks[root.kind] || []
            for (let i = 0; i < parts.length; i += 1) {
                const part = parts[i]
                ctx.beginPath()
                trace(ctx, part.d)
                if (part.f) {
                    ctx.fill()
                } else {
                    if (part.s) {
                        // A silhouette: the wash carries the shape, the stroke
                        // keeps the edge. Filled at full ink the folder turns
                        // into a slab that out-weighs the row's text.
                        ctx.globalAlpha = root.wash
                        ctx.fill()
                        ctx.globalAlpha = 1
                    }
                    ctx.lineWidth = (part.w || 1.2) * boost
                    ctx.stroke()
                }
            }
        }

        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()

        /// The mark's commands, already absolute and reduced to the five the
        /// sprite uses. Kept as one loop over a token list so a 33-mark set
        /// costs no parse tree.
        function trace(ctx, d) {
            const tokens = d.match(/[MLCQZ]|-?\d*\.?\d+/g)
            let i = 0
            while (i < tokens.length) {
                const command = tokens[i]
                if (command === "M" || command === "L") {
                    if (command === "M")
                        ctx.moveTo(Number(tokens[i + 1]), Number(tokens[i + 2]))
                    else
                        ctx.lineTo(Number(tokens[i + 1]), Number(tokens[i + 2]))
                    i += 3
                } else if (command === "Q") {
                    ctx.quadraticCurveTo(Number(tokens[i + 1]), Number(tokens[i + 2]),
                                         Number(tokens[i + 3]), Number(tokens[i + 4]))
                    i += 5
                } else if (command === "C") {
                    ctx.bezierCurveTo(Number(tokens[i + 1]), Number(tokens[i + 2]),
                                      Number(tokens[i + 3]), Number(tokens[i + 4]),
                                      Number(tokens[i + 5]), Number(tokens[i + 6]))
                    i += 7
                } else {
                    ctx.closePath()
                    i += 1
                }
            }
        }
    }

    Connections {
        target: root
        function onTintChanged() { canvas.requestPaint() }
        function onKindChanged() { canvas.requestPaint() }
    }
}
