import {
  BoxIcon,
  DatabaseIcon,
  DocumentIcon,
  DocumentTextIcon,
  FileAudioIcon,
  FileBadgeIcon,
  FileCodeIcon,
  FileCogIcon,
  FileIcon,
  FileTextIcon,
  FileZipIcon,
  FolderIcon,
  FolderOpenIcon,
  GalleryIcon,
  GlobeIcon,
  KeyIcon,
  LinkIcon,
  LockIcon,
  PresentationGraphIcon,
  ProgrammingIcon,
  RulerPenIcon,
  TableIcon,
  TextFormatIcon,
  VideoFrameIcon,
  VinylIcon,
  WindowFrameIcon,
} from "@solar-icons/react/line-duotone";
import { type ComponentType } from "react";

import { cn } from "@/lib/utils";

/**
 * File-type glyphs, drawn from Solar Icons (MIT for the code, CC BY 4.0 for
 * the artwork).
 *
 * The vocabulary is *categorical on purpose*. An editor's icon theme can
 * afford a hue per extension because its tree is source code and its rows are
 * 16px; a file manager draws whatever the user's disk happens to hold, so a
 * palette that tries to cover every extension either misses most of them or
 * turns the listing into confetti. One glyph per kind of thing — a
 * spreadsheet is a spreadsheet, not Excel — keeps a real directory legible
 * and keeps the map below small enough to read and edit.
 *
 * Because Solar Icons draws in `currentColor`, colour is decided here rather
 * than carried in the artwork: see TONES. That is what lets a glyph follow the
 * scheme, the system accent, `prefers-contrast` and forced-colors the way
 * every other surface in this shell does.
 *
 * The `line-duotone` style paints a secondary accent layer as well; its
 * strength is the shell-wide `--solar-secondary-opacity` (see App.css), which
 * is why a glyph has depth without any call site asking for it.
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
 * The registry is written as render functions rather than as bare Solar
 * components on purpose: the value type is then this file's own, so the table
 * neither depends on how the icon package types its components nor hands JSX
 * a union of twenty-six different ones to resolve.
 */
const GLYPHS = {
  folder: (props: GlyphProps) => <FolderIcon {...props} />,
  folderOpen: (props: GlyphProps) => <FolderOpenIcon {...props} />,
  file: (props: GlyphProps) => <FileIcon {...props} />,
  text: (props: GlyphProps) => <FileTextIcon {...props} />,
  config: (props: GlyphProps) => <FileCogIcon {...props} />,
  code: (props: GlyphProps) => <FileCodeIcon {...props} />,
  script: (props: GlyphProps) => <ProgrammingIcon {...props} />,
  // A format mark is only drawn where the mark is true. `document`, `pdf` and
  // `vector` each name one format family, so Solar Icons' `file-type-*` stamps
  // fit them. Spreadsheets and slides get a grid and a screen instead: a page
  // stamped "XLS" is simply wrong on a .csv or an .ods, and four near-identical
  // stamped pages are harder to tell apart at 16px than four silhouettes.
  document: (props: GlyphProps) => <DocumentTextIcon {...props} />,
  spreadsheet: (props: GlyphProps) => <TableIcon {...props} />,
  presentation: (props: GlyphProps) => <PresentationGraphIcon {...props} />,
  pdf: (props: GlyphProps) => <DocumentIcon {...props} />,
  image: (props: GlyphProps) => <GalleryIcon {...props} />,
  vector: (props: GlyphProps) => <RulerPenIcon {...props} />,
  video: (props: GlyphProps) => <VideoFrameIcon {...props} />,
  audio: (props: GlyphProps) => <FileAudioIcon {...props} />,
  archive: (props: GlyphProps) => <FileZipIcon {...props} />,
  disk: (props: GlyphProps) => <VinylIcon {...props} />,
  database: (props: GlyphProps) => <DatabaseIcon {...props} />,
  font: (props: GlyphProps) => <TextFormatIcon {...props} />,
  key: (props: GlyphProps) => <KeyIcon {...props} />,
  certificate: (props: GlyphProps) => <FileBadgeIcon {...props} />,
  executable: (props: GlyphProps) => <WindowFrameIcon {...props} />,
  package: (props: GlyphProps) => <BoxIcon {...props} />,
  shortcut: (props: GlyphProps) => <GlobeIcon {...props} />,
  link: (props: GlyphProps) => <LinkIcon {...props} />,
  lock: (props: GlyphProps) => <LockIcon {...props} />,
};

export type TypeGlyph = keyof typeof GLYPHS;

/**
 * Category colours, from the tokens App.css already reserves for exactly this
 * question: `--folder` plus the six `--tone-*` say what a thing *is* (which is
 * what a type icon reports), as opposed to the accent seam, which says what is
 * selected, or the semantic trio, which says what is wrong.
 *
 * The neutral set is `foreground`, not `muted-foreground`: a type glyph is the
 * row's own artwork, and the muted step is a *text* fade — at row size it read
 * as ink that had not loaded. Plain text, configuration and the unknown-file
 * fallback therefore stay full-strength ink, and the six hues keep meaning
 * "this one is coloured", which is the distinction that carries information.
 * The blank lines group hues only to keep the table legible — nothing reads
 * the grouping.
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

  file: "text-foreground",
  text: "text-foreground",
  config: "text-foreground",
  link: "text-foreground",
  lock: "text-foreground",
};

export interface EntryIconProps {
  className?: string;
  size?: number | string;
}

export type EntryIcon = ComponentType<EntryIconProps>;

/**
 * Solar Icons' own 16px default, which is also what the artwork these glyphs
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
