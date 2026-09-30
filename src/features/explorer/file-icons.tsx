import { i18n } from "@/i18n";

import { typeIcon, type EntryIcon, type TypeGlyph } from "./type-icon";
import type { DirectoryEntry } from "./types";

/**
 * What a name draws and what it is called.
 *
 * One table, not two. The glyph and the label for an extension used to live in
 * separate lists (a generated icon map and a hand-written label map) that had
 * to be kept in step by hand; the answer to "what is a .mp4" is now written
 * down once.
 *
 * The vocabulary is categorical rather than per-brand — see `type-icon.tsx` for
 * why — which is what keeps this table small enough to read, and what stops a
 * directory of mixed files from turning into a colour chart.
 */

/** A row of the type table. `label` is an `explorer:fileType.*` key; when it is
 *  absent the bare uppercased extension is the label ("GLSL" beats "Source"). */
interface EntryType {
  glyph: TypeGlyph;
  label?: string;
}

/** Extensions grouped by kind, so the table reads as a taxonomy rather than as
 *  an alphabet. Order inside a group is by prominence, not by name. */
const EXTENSION_TYPES: Record<string, EntryType> = {
  // Documents and office formats.
  pdf: { glyph: "pdf", label: "pdfDocument" },
  doc: { glyph: "document", label: "wordDocument" },
  docx: { glyph: "document", label: "wordDocument" },
  odt: { glyph: "document", label: "wordDocument" },
  rtf: { glyph: "document", label: "wordDocument" },
  pages: { glyph: "document", label: "wordDocument" },
  epub: { glyph: "document" },
  xps: { glyph: "document" },
  xls: { glyph: "spreadsheet", label: "excelSpreadsheet" },
  xlsx: { glyph: "spreadsheet", label: "excelSpreadsheet" },
  ods: { glyph: "spreadsheet", label: "excelSpreadsheet" },
  numbers: { glyph: "spreadsheet", label: "excelSpreadsheet" },
  csv: { glyph: "spreadsheet", label: "csvSpreadsheet" },
  tsv: { glyph: "spreadsheet" },
  ppt: { glyph: "presentation", label: "presentation" },
  pptx: { glyph: "presentation", label: "presentation" },
  odp: { glyph: "presentation", label: "presentation" },

  // Plain and lightly-marked text.
  txt: { glyph: "text", label: "textFile" },
  log: { glyph: "text", label: "logFile" },
  md: { glyph: "text", label: "markdown" },
  markdown: { glyph: "text", label: "markdown" },
  rst: { glyph: "text" },
  adoc: { glyph: "text" },
  asciidoc: { glyph: "text" },
  tex: { glyph: "text" },
  bib: { glyph: "text" },
  org: { glyph: "text" },
  srt: { glyph: "text", label: "subtitle" },
  vtt: { glyph: "text", label: "subtitle" },
  ass: { glyph: "text", label: "subtitle" },
  sub: { glyph: "text", label: "subtitle" },
  sbv: { glyph: "text", label: "subtitle" },

  // Configuration and serialised data. Kept apart from source code because the
  // file says how something is set up rather than what it does.
  json: { glyph: "config", label: "json" },
  json5: { glyph: "config" },
  jsonc: { glyph: "config" },
  yaml: { glyph: "config", label: "yaml" },
  yml: { glyph: "config", label: "yaml" },
  toml: { glyph: "config", label: "toml" },
  ini: { glyph: "config", label: "iniConfig" },
  conf: { glyph: "config", label: "configFile" },
  cfg: { glyph: "config" },
  env: { glyph: "config", label: "configFile" },
  properties: { glyph: "config", label: "configFile" },
  plist: { glyph: "config" },
  desktop: { glyph: "config" },
  svc: { glyph: "config" },
  gradle: { glyph: "config" },
  cmake: { glyph: "config" },
  mk: { glyph: "config" },
  sln: { glyph: "config" },
  csproj: { glyph: "config" },
  vbproj: { glyph: "config" },
  fsproj: { glyph: "config" },
  vcxproj: { glyph: "config" },

  // Source and markup.
  html: { glyph: "code", label: "html" },
  htm: { glyph: "code", label: "html" },
  xhtml: { glyph: "code", label: "html" },
  css: { glyph: "code", label: "css" },
  scss: { glyph: "code", label: "scss" },
  sass: { glyph: "code" },
  less: { glyph: "code", label: "less" },
  js: { glyph: "code", label: "javaScript" },
  mjs: { glyph: "code", label: "javaScript" },
  cjs: { glyph: "code", label: "javaScript" },
  jsx: { glyph: "code", label: "jsx" },
  ts: { glyph: "code", label: "typeScript" },
  mts: { glyph: "code", label: "typeScript" },
  cts: { glyph: "code", label: "typeScript" },
  tsx: { glyph: "code", label: "tsx" },
  py: { glyph: "code", label: "python" },
  pyi: { glyph: "code", label: "python" },
  ipynb: { glyph: "code", label: "python" },
  rs: { glyph: "code", label: "rust" },
  c: { glyph: "code", label: "cSource" },
  h: { glyph: "code", label: "cHeader" },
  cpp: { glyph: "code", label: "cpp" },
  cc: { glyph: "code", label: "cpp" },
  cxx: { glyph: "code", label: "cpp" },
  hpp: { glyph: "code", label: "cppHeader" },
  hh: { glyph: "code", label: "cppHeader" },
  cs: { glyph: "code", label: "cSharp" },
  vue: { glyph: "code", label: "vue" },
  svelte: { glyph: "code" },
  go: { glyph: "code" },
  java: { glyph: "code" },
  kt: { glyph: "code" },
  kts: { glyph: "code" },
  swift: { glyph: "code" },
  lua: { glyph: "code" },
  rb: { glyph: "code" },
  php: { glyph: "code" },
  pl: { glyph: "code" },
  pm: { glyph: "code" },
  r: { glyph: "code" },
  scala: { glyph: "code" },
  dart: { glyph: "code" },
  elm: { glyph: "code" },
  hs: { glyph: "code" },
  clj: { glyph: "code" },
  cljs: { glyph: "code" },
  ex: { glyph: "code" },
  exs: { glyph: "code" },
  erl: { glyph: "code" },
  zig: { glyph: "code" },
  nim: { glyph: "code" },
  v: { glyph: "code" },
  asm: { glyph: "code" },
  s: { glyph: "code" },
  glsl: { glyph: "code" },
  hlsl: { glyph: "code" },
  wgsl: { glyph: "code" },
  vert: { glyph: "code" },
  frag: { glyph: "code" },
  comp: { glyph: "code" },
  proto: { glyph: "code" },
  graphql: { glyph: "code" },
  gql: { glyph: "code" },
  xml: { glyph: "code", label: "xml" },
  patch: { glyph: "code", label: "patchFile" },
  diff: { glyph: "code", label: "patchFile" },

  // Executable text: the shell runs these, they are not build input.
  sh: { glyph: "script", label: "shellScript" },
  bash: { glyph: "script", label: "shellScript" },
  zsh: { glyph: "script", label: "shellScript" },
  fish: { glyph: "script", label: "shellScript" },
  ksh: { glyph: "script", label: "shellScript" },
  csh: { glyph: "script", label: "shellScript" },
  nu: { glyph: "script", label: "shellScript" },
  bat: { glyph: "script", label: "batchFile" },
  cmd: { glyph: "script", label: "batchFile" },
  ps1: { glyph: "script", label: "powerShell" },
  psm1: { glyph: "script", label: "powerShell" },

  // Images, and the vector format that is really a document.
  jpg: { glyph: "image", label: "jpegImage" },
  jpeg: { glyph: "image", label: "jpegImage" },
  png: { glyph: "image", label: "pngImage" },
  gif: { glyph: "image", label: "gifImage" },
  webp: { glyph: "image", label: "webpImage" },
  avif: { glyph: "image", label: "avifImage" },
  bmp: { glyph: "image", label: "bitmapImage" },
  ico: { glyph: "image", label: "icon" },
  tif: { glyph: "image", label: "tiffImage" },
  tiff: { glyph: "image", label: "tiffImage" },
  heic: { glyph: "image", label: "heicImage" },
  heif: { glyph: "image" },
  jfif: { glyph: "image" },
  apng: { glyph: "image" },
  jxl: { glyph: "image" },
  exr: { glyph: "image" },
  hdr: { glyph: "image" },
  tga: { glyph: "image" },
  psd: { glyph: "image" },
  ai: { glyph: "image" },
  indd: { glyph: "image" },
  raw: { glyph: "image" },
  cr2: { glyph: "image" },
  cr3: { glyph: "image" },
  nef: { glyph: "image" },
  arw: { glyph: "image" },
  dng: { glyph: "image" },
  svg: { glyph: "vector", label: "svgImage" },

  // Time-based media.
  mp4: { glyph: "video", label: "video" },
  mov: { glyph: "video", label: "video" },
  avi: { glyph: "video", label: "video" },
  mkv: { glyph: "video", label: "video" },
  webm: { glyph: "video", label: "video" },
  m4v: { glyph: "video", label: "video" },
  wmv: { glyph: "video", label: "video" },
  flv: { glyph: "video", label: "video" },
  mpg: { glyph: "video", label: "video" },
  mpeg: { glyph: "video", label: "video" },
  "3gp": { glyph: "video", label: "video" },
  vob: { glyph: "video", label: "video" },
  ogv: { glyph: "video", label: "video" },
  m2ts: { glyph: "video", label: "video" },
  mp3: { glyph: "audio", label: "audio" },
  wav: { glyph: "audio", label: "audio" },
  flac: { glyph: "audio", label: "audio" },
  aac: { glyph: "audio", label: "audio" },
  ogg: { glyph: "audio", label: "audio" },
  m4a: { glyph: "audio", label: "audio" },
  opus: { glyph: "audio", label: "audio" },
  wma: { glyph: "audio", label: "audio" },
  aiff: { glyph: "audio", label: "audio" },
  mid: { glyph: "audio", label: "audio" },
  midi: { glyph: "audio", label: "audio" },
  amr: { glyph: "audio", label: "audio" },
  ape: { glyph: "audio", label: "audio" },
  mka: { glyph: "audio", label: "audio" },
  m3u: { glyph: "audio", label: "audio" },
  m3u8: { glyph: "audio", label: "audio" },
  cue: { glyph: "audio", label: "audio" },

  // Containers, and the disc images that carry a filesystem inside them.
  zip: { glyph: "archive", label: "archive" },
  rar: { glyph: "archive", label: "archive" },
  "7z": { glyph: "archive", label: "archive" },
  tar: { glyph: "archive", label: "archive" },
  gz: { glyph: "archive", label: "archive" },
  bz2: { glyph: "archive", label: "archive" },
  xz: { glyph: "archive", label: "archive" },
  zst: { glyph: "archive", label: "archive" },
  cab: { glyph: "archive", label: "archive" },
  tgz: { glyph: "archive", label: "archive" },
  tbz2: { glyph: "archive", label: "archive" },
  lz: { glyph: "archive" },
  lzma: { glyph: "archive" },
  lz4: { glyph: "archive" },
  z: { glyph: "archive" },
  iso: { glyph: "disk", label: "diskImage" },
  img: { glyph: "disk", label: "diskImage" },
  dmg: { glyph: "disk", label: "diskImage" },
  vhd: { glyph: "disk", label: "diskImage" },
  vhdx: { glyph: "disk", label: "diskImage" },
  vmdk: { glyph: "disk", label: "diskImage" },
  qcow2: { glyph: "disk", label: "diskImage" },
  toast: { glyph: "disk", label: "diskImage" },

  // Data stores.
  sql: { glyph: "database", label: "sql" },
  db: { glyph: "database", label: "database" },
  sqlite: { glyph: "database", label: "database" },
  sqlite3: { glyph: "database", label: "database" },
  mdb: { glyph: "database", label: "database" },
  accdb: { glyph: "database", label: "database" },
  dbf: { glyph: "database", label: "database" },
  parquet: { glyph: "database", label: "database" },
  avro: { glyph: "database", label: "database" },
  orc: { glyph: "database", label: "database" },

  // Fonts, keys and signatures.
  ttf: { glyph: "font", label: "font" },
  otf: { glyph: "font", label: "font" },
  woff: { glyph: "font", label: "font" },
  woff2: { glyph: "font", label: "font" },
  eot: { glyph: "font", label: "font" },
  ttc: { glyph: "font", label: "font" },
  fnt: { glyph: "font", label: "font" },
  fon: { glyph: "font", label: "font" },
  pfb: { glyph: "font", label: "font" },
  pem: { glyph: "key", label: "privateKey" },
  key: { glyph: "key", label: "privateKey" },
  ppk: { glyph: "key" },
  gpg: { glyph: "key" },
  asc: { glyph: "key" },
  crt: { glyph: "certificate", label: "certificate" },
  cer: { glyph: "certificate", label: "certificate" },
  pfx: { glyph: "certificate", label: "certificate" },
  p12: { glyph: "certificate", label: "certificate" },
  jks: { glyph: "certificate", label: "certificate" },
  der: { glyph: "certificate", label: "certificate" },

  // Things the shell owns rather than the app.
  exe: { glyph: "executable", label: "executable" },
  appimage: { glyph: "executable", label: "executable" },
  com: { glyph: "executable", label: "executable" },
  scr: { glyph: "executable", label: "executable" },
  cpl: { glyph: "executable", label: "executable" },
  dll: { glyph: "executable", label: "executable" },
  so: { glyph: "executable", label: "executable" },
  dylib: { glyph: "executable", label: "executable" },
  bin: { glyph: "executable", label: "executable" },
  wasm: { glyph: "executable", label: "executable" },
  msi: { glyph: "executable", label: "installer" },
  msix: { glyph: "executable", label: "installer" },
  deb: { glyph: "package", label: "package" },
  rpm: { glyph: "package", label: "package" },
  jar: { glyph: "package", label: "package" },
  war: { glyph: "package", label: "package" },
  ear: { glyph: "package", label: "package" },
  apk: { glyph: "package", label: "package" },
  pkg: { glyph: "package", label: "package" },
  snap: { glyph: "package", label: "package" },
  appx: { glyph: "package", label: "package" },
  crx: { glyph: "package", label: "package" },
  xpi: { glyph: "package", label: "package" },
  lnk: { glyph: "shortcut", label: "shortcut" },
  url: { glyph: "shortcut", label: "shortcut" },
  webloc: { glyph: "shortcut", label: "shortcut" },
  lock: { glyph: "lock", label: "lockFile" },

  // Byproducts. Neutral on purpose: the listing should not shout about them.
  bak: { glyph: "file" },
  tmp: { glyph: "file" },
  temp: { glyph: "file" },
  part: { glyph: "file" },
  crdownload: { glyph: "file" },
  old: { glyph: "file" },
  orig: { glyph: "file" },
  swp: { glyph: "file" },
};

/** Names whose type is in the name, not the suffix — build files, licences and
 *  the shell's own rc files. Everything that merely reads as configuration is
 *  already covered by the dotfile branch in `getFilePresentation`, so only the
 *  exceptions live here. */
const FILENAME_TYPES: Record<string, EntryType> = {
  license: { glyph: "text", label: "license" },
  licence: { glyph: "text", label: "license" },
  copying: { glyph: "text", label: "license" },
  notice: { glyph: "text", label: "license" },
  readme: { glyph: "text", label: "markdown" },
  changelog: { glyph: "text", label: "markdown" },
  authors: { glyph: "text", label: "markdown" },
  contributing: { glyph: "text", label: "markdown" },
  makefile: { glyph: "config", label: "configFile" },
  gnumakefile: { glyph: "config", label: "configFile" },
  "makefile.am": { glyph: "config", label: "configFile" },
  "makefile.in": { glyph: "config", label: "configFile" },
  justfile: { glyph: "config", label: "configFile" },
  dockerfile: { glyph: "config", label: "configFile" },
  containerfile: { glyph: "config", label: "configFile" },
  gemfile: { glyph: "config", label: "configFile" },
  rakefile: { glyph: "config", label: "configFile" },
  procfile: { glyph: "config", label: "configFile" },
  vagrantfile: { glyph: "config", label: "configFile" },
  "package.json": { glyph: "config", label: "configFile" },
  "package-lock.json": { glyph: "config", label: "configFile" },
  "tsconfig.json": { glyph: "config", label: "configFile" },
  "jsconfig.json": { glyph: "config", label: "configFile" },
  "biome.json": { glyph: "config", label: "configFile" },
  "deno.json": { glyph: "config", label: "configFile" },
  "composer.json": { glyph: "config", label: "configFile" },
  "go.mod": { glyph: "config", label: "configFile" },
  "go.sum": { glyph: "config", label: "configFile" },
  "cmakelists.txt": { glyph: "config", label: "configFile" },
  gradlew: { glyph: "script", label: "shellScript" },
  mvnw: { glyph: "script", label: "shellScript" },
  ".bashrc": { glyph: "script", label: "shellScript" },
  ".bash_profile": { glyph: "script", label: "shellScript" },
  ".bash_login": { glyph: "script", label: "shellScript" },
  ".bash_aliases": { glyph: "script", label: "shellScript" },
  ".zshrc": { glyph: "script", label: "shellScript" },
  ".zprofile": { glyph: "script", label: "shellScript" },
  ".zshenv": { glyph: "script", label: "shellScript" },
  ".zlogin": { glyph: "script", label: "shellScript" },
  ".profile": { glyph: "script", label: "shellScript" },
  ".ds_store": { glyph: "file", label: "file" },
};

const DEFAULT_FILE: EntryType = { glyph: "file" };
const DOTFILE: EntryType = { glyph: "config" };
const DIRECTORY: EntryType = { glyph: "folder" };
const DIRECTORY_OPEN: EntryType = { glyph: "folderOpen" };
const SYMLINK: EntryType = { glyph: "link" };

export interface ExtensionPresentation {
  icon: EntryIcon;
  label: string;
}

function localizedLabel(labelKey: string): string {
  return i18n.t(`explorer:fileType.${labelKey}`);
}

/** Labels stay lazy: a listing can be sorted by type label, and that path is
 *  documented to resolve each entry's label exactly once per ordering pass. */
function presentation(type: EntryType, label: () => string): ExtensionPresentation {
  return {
    icon: typeIcon(type.glyph),
    get label() {
      return label();
    },
  };
}

/** Bare uppercased extension when the type table has no label for it — the
 *  suffix the user typed says more than a generic word does. */
function extensionLabel(extension: string): string {
  const label = EXTENSION_TYPES[extension]?.label;
  return label ? localizedLabel(label) : extension.toUpperCase();
}

export function getFileExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) {
    return "";
  }

  return name.slice(dotIndex + 1).toLowerCase();
}

/** Whether the type table knows this extension. Unmapped types are candidates
 *  for OS-native icons (see `native-icon.tsx`). */
export function hasKnownFileExtension(extension: string): boolean {
  return extension in EXTENSION_TYPES;
}

export function getFilePresentation(name: string): ExtensionPresentation {
  const lowerName = name.toLowerCase();
  const extension = getFileExtension(name);

  const byFilename = FILENAME_TYPES[lowerName];
  if (byFilename) {
    return presentation(byFilename, () =>
      byFilename.label ? localizedLabel(byFilename.label) : extensionLabel(extension),
    );
  }

  const byExtension = EXTENSION_TYPES[extension];
  if (byExtension) {
    return presentation(byExtension, () => extensionLabel(extension));
  }

  if (name.startsWith(".")) {
    return presentation(DOTFILE, () => localizedLabel("configFile"));
  }

  return presentation(DEFAULT_FILE, () => localizedLabel("file"));
}

/**
 * Folders draw one glyph, closed or open.
 *
 * The previous set carried artwork per folder *name* — a distinct drawing for
 * `src`, `node_modules`, `.git` and the rest — which no general icon family
 * offers, and which no system file manager does either: Explorer and Finder
 * both draw every folder as a folder. The open variant is kept, because
 * expansion is a real state the tree has to show.
 */
export function getFolderPresentation(open = false): ExtensionPresentation {
  return presentation(open ? DIRECTORY_OPEN : DIRECTORY, () => localizedLabel("directory"));
}

export const SYMLINK_PRESENTATION: ExtensionPresentation = presentation(SYMLINK, () =>
  localizedLabel("symlink"),
);

export const OTHER_PRESENTATION: ExtensionPresentation = presentation(DEFAULT_FILE, () =>
  localizedLabel("other"),
);

export const DIRECTORY_PRESENTATION: ExtensionPresentation = presentation(DIRECTORY, () =>
  localizedLabel("directory"),
);

/** Kind-aware presentation used by every view and the preview surface. */
export function getEntryPresentation(entry: DirectoryEntry): ExtensionPresentation {
  switch (entry.kind) {
    case "directory":
      return getFolderPresentation();
    case "symlink":
      return SYMLINK_PRESENTATION;
    case "other":
      return OTHER_PRESENTATION;
    default:
      return getFilePresentation(entry.name);
  }
}
