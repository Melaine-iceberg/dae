import { i18n } from "@/i18n";

import { mitIcon, type EntryIcon } from "./mit-icon";
import {
  DEFAULT_FILE_ICON,
  DEFAULT_FOLDER_ICON,
  DEFAULT_FOLDER_OPEN_ICON,
  DOTFILE_ICON,
  EXT_ICONS,
  FILENAME_ICONS,
  FOLDER_ICONS,
  FOLDER_OPEN_ICONS,
  SYMLINK_ICON,
} from "./mit-icons.generated";
import type { DirectoryEntry } from "./types";

/**
 * File/folder presentation is driven by Material Icon Theme's official
 * mapping tables (see mit-icons.generated.ts). Artwork carries its own
 * color, so presentations no longer assign a semantic tone per category.
 */

export interface ExtensionPresentation {
  icon: EntryIcon;
  label: string;
}

/** Localized type labels for the extensions the app names explicitly;
 *  anything else falls back to the bare uppercased extension ("TOML"). */
const EXT_LABEL_KEYS: Record<string, string> = {
  pdf: "pdfDocument",
  doc: "wordDocument",
  docx: "wordDocument",
  odt: "wordDocument",
  rtf: "wordDocument",
  xls: "excelSpreadsheet",
  xlsx: "excelSpreadsheet",
  ods: "excelSpreadsheet",
  csv: "csvSpreadsheet",
  ppt: "presentation",
  pptx: "presentation",
  odp: "presentation",
  jpg: "jpegImage",
  jpeg: "jpegImage",
  png: "pngImage",
  gif: "gifImage",
  webp: "webpImage",
  avif: "avifImage",
  bmp: "bitmapImage",
  ico: "icon",
  tif: "tiffImage",
  tiff: "tiffImage",
  heic: "heicImage",
  svg: "svgImage",
  mp4: "video",
  mov: "video",
  avi: "video",
  mkv: "video",
  webm: "video",
  m4v: "video",
  wmv: "video",
  flv: "video",
  mpg: "video",
  mpeg: "video",
  srt: "subtitle",
  vtt: "subtitle",
  ass: "subtitle",
  mp3: "audio",
  wav: "audio",
  flac: "audio",
  aac: "audio",
  ogg: "audio",
  m4a: "audio",
  opus: "audio",
  wma: "audio",
  aiff: "audio",
  mid: "audio",
  midi: "audio",
  zip: "archive",
  rar: "archive",
  "7z": "archive",
  tar: "archive",
  gz: "archive",
  bz2: "archive",
  xz: "archive",
  zst: "archive",
  iso: "diskImage",
  img: "diskImage",
  dmg: "diskImage",
  txt: "textFile",
  log: "logFile",
  md: "markdown",
  markdown: "markdown",
  html: "html",
  htm: "html",
  css: "css",
  scss: "scss",
  less: "less",
  js: "javaScript",
  mjs: "javaScript",
  cjs: "javaScript",
  jsx: "jsx",
  ts: "typeScript",
  tsx: "tsx",
  py: "python",
  rs: "rust",
  c: "cSource",
  h: "cHeader",
  cpp: "cpp",
  hpp: "cppHeader",
  cs: "cSharp",
  vue: "vue",
  json: "json",
  xml: "xml",
  sh: "shellScript",
  bash: "shellScript",
  zsh: "shellScript",
  fish: "shellScript",
  bat: "batchFile",
  cmd: "batchFile",
  ps1: "powerShell",
  sql: "sql",
  patch: "patchFile",
  diff: "patchFile",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  ini: "iniConfig",
  conf: "configFile",
  env: "configFile",
  properties: "configFile",
  db: "database",
  sqlite: "database",
  sqlite3: "database",
  mdb: "database",
  ttf: "font",
  otf: "font",
  woff: "font",
  woff2: "font",
  eot: "font",
  exe: "executable",
  msi: "installer",
  appimage: "executable",
  deb: "package",
  rpm: "package",
  jar: "package",
  lnk: "shortcut",
  url: "shortcut",
  lock: "lockFile",
  pem: "privateKey",
  key: "privateKey",
  crt: "certificate",
  cer: "certificate",
};

/** Well-known filenames whose label goes beyond their (or a missing) extension. */
const FILENAME_LABEL_KEYS: Record<string, string> = {
  license: "license",
  licence: "license",
  copying: "license",
  readme: "markdown",
  changelog: "markdown",
  authors: "markdown",
  contributing: "markdown",
  makefile: "configFile",
  justfile: "configFile",
  dockerfile: "configFile",
  "package.json": "configFile",
  "tsconfig.json": "configFile",
};

function localizedLabel(labelKey: string): string {
  return i18n.t(`explorer:fileType.${labelKey}`);
}

function presentation(iconName: string, label: () => string): ExtensionPresentation {
  return {
    icon: mitIcon(iconName),
    get label() {
      return label();
    },
  };
}

function extensionLabel(extension: string): string {
  const labelKey = EXT_LABEL_KEYS[extension];
  return labelKey ? localizedLabel(labelKey) : extension.toUpperCase();
}

function filenameLabel(lowerName: string, extension: string): string {
  const labelKey = FILENAME_LABEL_KEYS[lowerName];
  if (labelKey) {
    return localizedLabel(labelKey);
  }
  if (extension) {
    return extensionLabel(extension);
  }
  return localizedLabel(lowerName.startsWith(".") ? "configFile" : "file");
}

export function getFileExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) {
    return "";
  }

  return name.slice(dotIndex + 1).toLowerCase();
}

/** Whether an extension has a theme artwork mapping; unmapped types are
 *  candidates for OS-native icons (see `native-icon.tsx`). */
export function hasKnownFileExtension(extension: string): boolean {
  return extension in EXT_ICONS;
}

export function getFilePresentation(name: string): ExtensionPresentation {
  const lowerName = name.toLowerCase();
  const extension = getFileExtension(name);

  const byFilename = FILENAME_ICONS[lowerName];
  if (byFilename) {
    return presentation(byFilename, () => filenameLabel(lowerName, extension));
  }

  if (extension && extension in EXT_ICONS) {
    return presentation(EXT_ICONS[extension], () => extensionLabel(extension));
  }

  if (name.startsWith(".")) {
    return presentation(DOTFILE_ICON, () => localizedLabel("configFile"));
  }

  return presentation(DEFAULT_FILE_ICON, () => localizedLabel("file"));
}

/** Folders pick up the theme's per-name artwork (src, node_modules, .git,
 *  ...) with a dedicated open variant where the set provides one. */
export function getFolderPresentation(name: string, open = false): ExtensionPresentation {
  const lowerName = name.toLowerCase();
  const iconName =
    (open ? FOLDER_OPEN_ICONS[lowerName] : undefined) ??
    FOLDER_ICONS[lowerName] ??
    (open ? DEFAULT_FOLDER_OPEN_ICON : DEFAULT_FOLDER_ICON);
  return presentation(iconName, () => localizedLabel("directory"));
}

export const SYMLINK_PRESENTATION: ExtensionPresentation = presentation(SYMLINK_ICON, () =>
  localizedLabel("symlink"),
);

export const OTHER_PRESENTATION: ExtensionPresentation = presentation(DEFAULT_FILE_ICON, () =>
  localizedLabel("other"),
);

export const DIRECTORY_PRESENTATION: ExtensionPresentation = presentation(
  DEFAULT_FOLDER_ICON,
  () => localizedLabel("directory"),
);

/** Kind-aware presentation used by every view and the preview surface. */
export function getEntryPresentation(entry: DirectoryEntry): ExtensionPresentation {
  switch (entry.kind) {
    case "directory":
      return getFolderPresentation(entry.name);
    case "symlink":
      return SYMLINK_PRESENTATION;
    case "other":
      return OTHER_PRESENTATION;
    default:
      return getFilePresentation(entry.name);
  }
}
