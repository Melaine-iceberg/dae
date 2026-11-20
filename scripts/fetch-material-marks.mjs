// Regenerates `qml/material-marks.json` from Google's Material Symbols.
//
// The geometry is copied verbatim from the `google/material-design-icons`
// repository (symbols/web/<name>/materialsymbols{outlined,rounded}/<name>_24px.svg).
// Material Symbols are authored on a 960-unit grid with `viewBox="0 -960 960 960"`;
// the path data is kept as-is and the renderer maps that grid onto the item
// (see crates/dae-qt/cpp/material_marks.cpp). Nothing is re-drawn here: a mark
// that looks wrong is fixed upstream and re-copied, the same rule the Solar
// table followed.
//
// Run: node scripts/fetch-material-marks.mjs
//
// The six marks that stay dae's own (the macOS traffic-light glyphs and the
// space diamond) are carried over from the previous table unchanged; they are
// drawn on the 24-unit grid and listed in `_legacy24`.

import { writeFile } from "node:fs/promises";

const BASE = "https://raw.githubusercontent.com/google/material-design-icons/master";
const STYLE_OUTLINED = "materialsymbolsoutlined";
const STYLE_ROUNDED = "materialsymbolsrounded";

// kind -> Material Symbol name. Outlined unless `rounded` is set. `s` marks the
// silhouette treatment (wash fill + stroke), `f` a plain fill.
const INTERFACE = {
    home: "home",
    desktop: "desktop_windows",
    documents: "file_copy",
    downloads: "download",
    pictures: "photo_library",
    music: "music_note",
    videos: "movie",
    search: "search",
    list: "view_list",
    grid: "grid_view",
    column: "view_column",
    more: "more_vert",
    branch: "fork_right",
    window: "window",
    panel: "left_panel_open",
    terminal: "terminal",
    folder: { name: "folder", rounded: true, fill: true, s: true },
    file: "draft",
    image: "image",
    code: "code",
    archive: "folder_zip",
    text: "description",
    exe: "memory",
    media: "smart_display",
    link: "link",
    drive: "hard_drive",
    network: "lan",
    cloud: "cloud",
    trash: "delete",
    chevron: "chevron_right",
    chevronRight: "chevron_right",
    chevronLeft: "chevron_left",
    chevronUp: "expand_less",
    chevronDown: "expand_more",
    check: "check",
    close: "close",
    pause: "pause",
    play: "play_arrow",
    plus: "add",
    clock: "schedule",
    gear: "settings",
    star: "star",
    tag: { name: "sell", rounded: true, fill: true, s: true },
    refresh: "refresh",
    eye: "visibility",
    info: "info",
    copy: "content_copy",
    cut: "content_cut",
    paste: "content_paste",
    pencil: "edit",
    external: "open_in_new",
    undo: "undo",
    sort: "sort",
    filter: "filter_list",
    split: "vertical_split",
    extract: "upload",
    arrowLeft: "arrow_back",
    arrowRight: "arrow_forward",
    arrowUp: "arrow_upward",
};

// Geometry the file glyph (image://fileglyph) composes: the shared page, the
// folder, their solid variants for wash/clip, and one family mark per category.
const FILE_GLYPH = {
    filePage: "draft",
    filePageFill: { name: "draft", rounded: true, fill: true },
    fileFolder: "folder",
    fileFolderFill: { name: "folder", rounded: true, fill: true },
    famCode: "code",
    famImage: "image",
    famVideo: "play_arrow",
    famAudio: "graphic_eq",
    famArchive: "archive",
    famSpreadsheet: "grid_view",
    famFont: "text_fields",
    famExecutable: "memory",
    famLink: "link",
    famText: "subject",
};

const LEGACY = ["diamond", "windowMin", "windowMax", "windowClose",
    "windowFullScreen", "windowExitFullScreen"];

// The six dae-owned marks, on the 24-unit grid. Kept here verbatim so the
// script stays reproducible after the Solar table is retired.
const LEGACY_MARKS = {
    diamond: [{ d: "M12 3.9L20.1 12L12 20.1L3.9 12Z", w: 1.2 }],
    windowMin: [{ d: "M6 12L18 12", w: 1 }],
    windowMax: [{ d: "M6.3 6.3L17.7 6.3L17.7 17.7L6.3 17.7Z", w: 1 }],
    windowClose: [{ d: "M6.75 6.75L17.25 17.25M17.25 6.75L6.75 17.25", w: 1 }],
    windowFullScreen: [
        { d: "M2.4 2.4L10.95 2.4L2.4 10.95Z", w: 1 },
        { d: "M21.6 21.6L13.05 21.6L21.6 13.05Z", w: 1 },
    ],
    windowExitFullScreen: [
        { d: "M7.8 7.8L10.95 4.65L4.65 10.95Z", w: 1 },
        { d: "M16.2 16.2L13.05 19.35L19.35 13.05Z", w: 1 },
    ],
};

async function fetchText(url, tries = 4) {
    for (let attempt = 1; ; attempt += 1) {
        try {
            const response = await fetch(url);
            if (!response.ok)
                throw new Error(`HTTP ${response.status}`);
            return await response.text();
        } catch (error) {
            if (attempt >= tries)
                throw new Error(`${url}: ${error.message}`);
            await new Promise(resolve => setTimeout(resolve, 400 * attempt));
        }
    }
}

async function fetchSvg(name, style, fill) {
    const file = fill ? `${name}_fill1_24px.svg` : `${name}_24px.svg`;
    const url = `${BASE}/symbols/web/${name}/${style}/${file}`;
    const svg = await fetchText(url);
    const paths = [...svg.matchAll(/<path[^>]*\bd="([^"]+)"/g)].map(m => m[1]);
    if (paths.length === 0)
        throw new Error(`${name}: no <path d> found`);
    return paths;
}

function entry(spec) {
    return typeof spec === "string" ? { name: spec } : spec;
}

async function partsFor(spec) {
    const { name, rounded, fill, s } = entry(spec);
    const paths = await fetchSvg(name, rounded ? STYLE_ROUNDED : STYLE_OUTLINED, fill);
    return paths.map(d => s ? { d, s: true } : { d, f: true });
}

async function main() {
    const table = {};
    const mapping = [];

    for (const [kind, spec] of Object.entries(INTERFACE)) {
        table[kind] = await partsFor(spec);
        const { name, rounded, fill, s } = entry(spec);
        mapping.push(`${kind}=${name}${rounded ? " (rounded)" : ""}${fill ? " fill1" : ""}${s ? " (silhouette)" : ""}`);
    }
    for (const [kind, spec] of Object.entries(FILE_GLYPH)) {
        table[kind] = await partsFor(spec);
        const { name, rounded, fill } = entry(spec);
        mapping.push(`${kind}=${name}${rounded ? " (rounded)" : ""}${fill ? " fill1" : ""}`);
    }

    // The six dae-owned marks stay on the 24-unit grid, verbatim.
    for (const kind of LEGACY)
        table[kind] = LEGACY_MARKS[kind];

    const licence = await fetchText(`${BASE}/LICENSE`);

    table._grid = 960;
    table._legacy24 = LEGACY;
    table._source = "Google Material Symbols (https://github.com/google/material-design-icons), "
        + "outlined style for interface marks, rounded fill1 for the folder/tag silhouettes and the "
        + "file-glyph solid page/folder. Path data copied verbatim from symbols/web/<name>/materialsymbols{outlined,rounded}/"
        + "<name>[_fill1]_24px.svg on the 960-unit grid (viewBox 0 -960 960 960); the renderer maps that grid onto the "
        + "item. `f` fills the path solid, `s` fills it at `wash` and strokes the outline. Six marks stay dae's "
        + "own on the 24-unit grid (_legacy24): the macOS traffic-light glyphs and the space diamond.\n\n"
        + mapping.join("  ");
    table._licence = licence;

    const out = new URL("../qml/material-marks.json", import.meta.url);
    await writeFile(out, JSON.stringify(table, null, 2) + "\n");
    const count = Object.keys(table).filter(k => !k.startsWith("_")).length;
    console.log(`wrote ${count} marks to qml/material-marks.json`);
}

main().catch(error => { console.error(error); process.exit(1); });