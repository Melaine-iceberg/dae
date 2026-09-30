import {
  IconAppWindow,
  IconDisc,
  IconFile,
  IconFileCertificate,
  IconFileCode,
  IconFileDatabase,
  IconFileMusic,
  IconFileSettings,
  IconFileText,
  IconFileTypeDoc,
  IconFileTypePdf,
  IconFileTypeSvg,
  IconFileZip,
  IconFolder,
  IconFolderOpen,
  IconKey,
  IconLink,
  IconLock,
  IconMovie,
  IconPackage,
  IconPhoto,
  IconPresentation,
  IconTable,
  IconTerminal2,
  IconTypography,
  IconWorld,
} from "@tabler/icons-react";
import { type ComponentType } from "react";

import { cn } from "@/lib/utils";

/**
 * File-type glyphs, drawn from Tabler (MIT).
 *
 * The vocabulary is *categorical on purpose*. An editor's icon theme can
 * afford a hue per extension because its tree is source code and its rows are
 * 16px; a file manager draws whatever the user's disk happens to hold, so a
 * palette that tries to cover every extension either misses most of them or
 * turns the listing into confetti. One glyph per kind of thing — a
 * spreadsheet is a spreadsheet, not Excel — keeps a real directory legible
 * and keeps the map below small enough to read and edit.
 *
 * Because Tabler draws in `currentColor`, colour is decided here rather than
 * carried in the artwork: see TONES. That is what lets a glyph follow the
 * scheme, the system accent, `prefers-contrast` and forced-colors the way
 * every other surface in this shell does.
 */

/** What a glyph is handed. `aria-hidden` is spelled out because every icon
 *  here is decorative — the name beside it is the accessible label, exactly as
 *  the `alt=""` these glyphs replaced said nothing. */
interface GlyphProps {
  className?: string;
  size?: number | string;
  "aria-hidden"?: boolean;
}

/**
 * The registry is written as render functions rather than as bare Tabler
 * components on purpose: the value type is then this file's own, so the table
 * neither depends on how the icon package types its components nor hands JSX
 * a union of twenty-six different ones to resolve.
 */
const GLYPHS = {
  folder: (props: GlyphProps) => <IconFolder {...props} />,
  folderOpen: (props: GlyphProps) => <IconFolderOpen {...props} />,
  file: (props: GlyphProps) => <IconFile {...props} />,
  text: (props: GlyphProps) => <IconFileText {...props} />,
  config: (props: GlyphProps) => <IconFileSettings {...props} />,
  code: (props: GlyphProps) => <IconFileCode {...props} />,
  script: (props: GlyphProps) => <IconTerminal2 {...props} />,
  // A format mark is only drawn where the mark is true. `document`, `pdf` and
  // `vector` each name one format family, so Tabler's `file-type-*` stamps fit
  // them. Spreadsheets and slides get a grid and a screen instead: a page
  // stamped "XLS" is simply wrong on a .csv or an .ods, and four near-identical
  // stamped pages are harder to tell apart at 16px than four silhouettes.
  document: (props: GlyphProps) => <IconFileTypeDoc {...props} />,
  spreadsheet: (props: GlyphProps) => <IconTable {...props} />,
  presentation: (props: GlyphProps) => <IconPresentation {...props} />,
  pdf: (props: GlyphProps) => <IconFileTypePdf {...props} />,
  image: (props: GlyphProps) => <IconPhoto {...props} />,
  vector: (props: GlyphProps) => <IconFileTypeSvg {...props} />,
  video: (props: GlyphProps) => <IconMovie {...props} />,
  audio: (props: GlyphProps) => <IconFileMusic {...props} />,
  archive: (props: GlyphProps) => <IconFileZip {...props} />,
  disk: (props: GlyphProps) => <IconDisc {...props} />,
  database: (props: GlyphProps) => <IconFileDatabase {...props} />,
  font: (props: GlyphProps) => <IconTypography {...props} />,
  key: (props: GlyphProps) => <IconKey {...props} />,
  certificate: (props: GlyphProps) => <IconFileCertificate {...props} />,
  executable: (props: GlyphProps) => <IconAppWindow {...props} />,
  package: (props: GlyphProps) => <IconPackage {...props} />,
  shortcut: (props: GlyphProps) => <IconWorld {...props} />,
  link: (props: GlyphProps) => <IconLink {...props} />,
  lock: (props: GlyphProps) => <IconLock {...props} />,
};

export type TypeGlyph = keyof typeof GLYPHS;

/**
 * Category colours, from the tokens App.css already reserves for exactly this
 * question: `--folder` plus the six `--tone-*` say what a thing *is* (which is
 * what a type icon reports), as opposed to the accent seam, which says what is
 * selected, or the semantic trio, which says what is wrong.
 *
 * `muted-foreground` is the deliberate non-colour: plain text, configuration
 * and the unknown-file fallback stay ink, so the six hues go on meaning
 * something. The blank lines group hues only to keep the table legible —
 * nothing reads the grouping.
 */
const TONES: Record<TypeGlyph, string> = {
  folder: "text-folder",
  folderOpen: "text-folder",

  document: "text-tone-blue",
  code: "text-tone-blue",
  shortcut: "text-tone-blue",

  spreadsheet: "text-tone-emerald",
  certificate: "text-tone-emerald",

  presentation: "text-tone-amber",
  archive: "text-tone-amber",
  disk: "text-tone-amber",
  package: "text-tone-amber",

  pdf: "text-tone-rose",
  key: "text-tone-rose",
  executable: "text-tone-rose",

  image: "text-tone-violet",
  vector: "text-tone-violet",
  video: "text-tone-violet",
  font: "text-tone-violet",

  audio: "text-tone-cyan",
  database: "text-tone-cyan",
  script: "text-tone-cyan",

  file: "text-muted-foreground",
  text: "text-muted-foreground",
  config: "text-muted-foreground",
  link: "text-muted-foreground",
  lock: "text-muted-foreground",
};

export interface EntryIconProps {
  className?: string;
  size?: number | string;
}

export type EntryIcon = ComponentType<EntryIconProps>;

/**
 * Tabler's own 16px default, which is also what the artwork these glyphs
 * replaced happened to measure. A caller that sizes the icon by class (`size-4`
 * in the tab strip and the tree) keeps the geometry it had; a caller that
 * passes a number keeps passing one.
 */
const DEFAULT_SIZE = 16;

const componentCache = new Map<TypeGlyph, EntryIcon>();

/**
 * Stable component per glyph. Presentations are resolved per row on every
 * render, and handing back a fresh component identity each time would remount
 * the icon subtree of every visible row in a virtualized listing.
 *
 * The glyph's tone and the caller's class go through `cn`, so the usual
 * Tailwind last-wins rule holds: a call site that needs a different colour
 * (a faded hidden row, the preview header) still overrides the category.
 */
export function typeIcon(glyph: TypeGlyph): EntryIcon {
  const cached = componentCache.get(glyph);
  if (cached) {
    return cached;
  }

  const render = GLYPHS[glyph];
  const tone = TONES[glyph];

  const component: EntryIcon = function TypeIcon({ className, size }) {
    return render({
      "aria-hidden": true,
      className: className ? cn(tone, className) : tone,
      size: size ?? DEFAULT_SIZE,
    });
  };

  componentCache.set(glyph, component);
  return component;
}
