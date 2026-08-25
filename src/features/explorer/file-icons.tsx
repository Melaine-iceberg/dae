import {
  AppWindowIcon,
  DiscIcon,
  FileArchiveIcon,
  FileAudioIcon,
  FileCIcon,
  FileCodeIcon,
  FileCppIcon,
  FileCSharpIcon,
  FileCssIcon,
  FileCsvIcon,
  FileDashedIcon,
  FileDocIcon,
  FileHtmlIcon,
  FileImageIcon,
  FileIniIcon,
  FileJpgIcon,
  FileJsIcon,
  FileJsxIcon,
  FileLockIcon,
  FileMdIcon,
  FilePdfIcon,
  FilePngIcon,
  FilePptIcon,
  FilePyIcon,
  FileRsIcon,
  FileSqlIcon,
  FileSvgIcon,
  FileTextIcon,
  FileTsIcon,
  FileTsxIcon,
  FileTxtIcon,
  FileVideoIcon,
  FileVueIcon,
  FileXlsIcon,
  FileZipIcon,
  FolderIcon,
  FolderOpenIcon,
  GearSixIcon,
  LinkSimpleIcon,
  PackageIcon,
  TextAaIcon,
  type Icon,
} from "@phosphor-icons/react";

import { i18n } from "@/i18n";

export type PhosphorIcon = Icon;

export const FolderClosedIcon = FolderIcon;
export const FolderOpenedIcon = FolderOpenIcon;

interface ExtensionPresentation {
  icon: PhosphorIcon;
  /** Semantic tone class; entries without one fall back to muted foreground. */
  tone?: string;
  label: string;
}

/** Presentation whose type label resolves against the active locale on read. */
function localizedPresentation(
  icon: PhosphorIcon,
  labelKey: string,
  tone?: string,
): ExtensionPresentation {
  return {
    icon,
    tone,
    get label() {
      return i18n.t(`explorer:fileType.${labelKey}`);
    },
  };
}

const EXTENSION_PRESENTATION: Record<string, ExtensionPresentation> = {
  pdf: localizedPresentation(FilePdfIcon, "pdfDocument", "text-icon-pdf"),
  doc: localizedPresentation(FileDocIcon, "wordDocument", "text-icon-doc"),
  docx: localizedPresentation(FileDocIcon, "wordDocument", "text-icon-doc"),
  odt: localizedPresentation(FileDocIcon, "wordDocument", "text-icon-doc"),
  rtf: localizedPresentation(FileDocIcon, "wordDocument", "text-icon-doc"),
  xls: localizedPresentation(FileXlsIcon, "excelSpreadsheet", "text-icon-sheet"),
  xlsx: localizedPresentation(FileXlsIcon, "excelSpreadsheet", "text-icon-sheet"),
  ods: localizedPresentation(FileXlsIcon, "excelSpreadsheet", "text-icon-sheet"),
  csv: localizedPresentation(FileCsvIcon, "csvSpreadsheet", "text-icon-sheet"),
  ppt: localizedPresentation(FilePptIcon, "presentation", "text-icon-slide"),
  pptx: localizedPresentation(FilePptIcon, "presentation", "text-icon-slide"),
  odp: localizedPresentation(FilePptIcon, "presentation", "text-icon-slide"),
  jpg: localizedPresentation(FileJpgIcon, "jpegImage", "text-icon-image"),
  jpeg: localizedPresentation(FileJpgIcon, "jpegImage", "text-icon-image"),
  png: localizedPresentation(FilePngIcon, "pngImage", "text-icon-image"),
  gif: localizedPresentation(FileImageIcon, "gifImage", "text-icon-image"),
  webp: localizedPresentation(FileImageIcon, "webpImage", "text-icon-image"),
  avif: localizedPresentation(FileImageIcon, "avifImage", "text-icon-image"),
  bmp: localizedPresentation(FileImageIcon, "bitmapImage", "text-icon-image"),
  ico: localizedPresentation(FileImageIcon, "icon", "text-icon-image"),
  tif: localizedPresentation(FileImageIcon, "tiffImage", "text-icon-image"),
  tiff: localizedPresentation(FileImageIcon, "tiffImage", "text-icon-image"),
  heic: localizedPresentation(FileImageIcon, "heicImage", "text-icon-image"),
  svg: localizedPresentation(FileSvgIcon, "svgImage", "text-icon-image"),
  mp4: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  mov: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  avi: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  mkv: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  webm: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  m4v: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  wmv: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  flv: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  mpg: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  mpeg: localizedPresentation(FileVideoIcon, "video", "text-icon-video"),
  srt: localizedPresentation(FileVideoIcon, "subtitle", "text-icon-video"),
  vtt: localizedPresentation(FileVideoIcon, "subtitle", "text-icon-video"),
  ass: localizedPresentation(FileVideoIcon, "subtitle", "text-icon-video"),
  mp3: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  wav: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  flac: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  aac: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  ogg: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  m4a: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  opus: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  wma: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  aiff: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  mid: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  midi: localizedPresentation(FileAudioIcon, "audio", "text-icon-audio"),
  zip: localizedPresentation(FileZipIcon, "archive", "text-icon-archive"),
  rar: localizedPresentation(FileArchiveIcon, "archive", "text-icon-archive"),
  "7z": localizedPresentation(FileArchiveIcon, "archive", "text-icon-archive"),
  tar: localizedPresentation(FileArchiveIcon, "archive", "text-icon-archive"),
  gz: localizedPresentation(FileArchiveIcon, "archive", "text-icon-archive"),
  bz2: localizedPresentation(FileArchiveIcon, "archive", "text-icon-archive"),
  xz: localizedPresentation(FileArchiveIcon, "archive", "text-icon-archive"),
  zst: localizedPresentation(FileArchiveIcon, "archive", "text-icon-archive"),
  iso: localizedPresentation(DiscIcon, "diskImage", "text-icon-archive"),
  img: localizedPresentation(DiscIcon, "diskImage", "text-icon-archive"),
  dmg: localizedPresentation(DiscIcon, "diskImage", "text-icon-archive"),
  txt: localizedPresentation(FileTxtIcon, "textFile"),
  log: localizedPresentation(FileTxtIcon, "logFile"),
  md: localizedPresentation(FileMdIcon, "markdown", "text-icon-code"),
  markdown: localizedPresentation(FileMdIcon, "markdown", "text-icon-code"),
  html: localizedPresentation(FileHtmlIcon, "html", "text-icon-code"),
  htm: localizedPresentation(FileHtmlIcon, "html", "text-icon-code"),
  css: localizedPresentation(FileCssIcon, "css", "text-icon-code"),
  scss: localizedPresentation(FileCssIcon, "scss", "text-icon-code"),
  less: localizedPresentation(FileCssIcon, "less", "text-icon-code"),
  js: localizedPresentation(FileJsIcon, "javaScript", "text-icon-code"),
  mjs: localizedPresentation(FileJsIcon, "javaScript", "text-icon-code"),
  cjs: localizedPresentation(FileJsIcon, "javaScript", "text-icon-code"),
  jsx: localizedPresentation(FileJsxIcon, "jsx", "text-icon-code"),
  ts: localizedPresentation(FileTsIcon, "typeScript", "text-icon-code"),
  tsx: localizedPresentation(FileTsxIcon, "tsx", "text-icon-code"),
  py: localizedPresentation(FilePyIcon, "python", "text-icon-code"),
  rs: localizedPresentation(FileRsIcon, "rust", "text-icon-code"),
  c: localizedPresentation(FileCIcon, "cSource", "text-icon-code"),
  h: localizedPresentation(FileCIcon, "cHeader", "text-icon-code"),
  cpp: localizedPresentation(FileCppIcon, "cpp", "text-icon-code"),
  hpp: localizedPresentation(FileCppIcon, "cppHeader", "text-icon-code"),
  cs: localizedPresentation(FileCSharpIcon, "cSharp", "text-icon-code"),
  vue: localizedPresentation(FileVueIcon, "vue", "text-icon-code"),
  json: localizedPresentation(FileCodeIcon, "json", "text-icon-code"),
  xml: localizedPresentation(FileCodeIcon, "xml", "text-icon-code"),
  sh: localizedPresentation(FileCodeIcon, "shellScript", "text-icon-code"),
  bash: localizedPresentation(FileCodeIcon, "shellScript", "text-icon-code"),
  zsh: localizedPresentation(FileCodeIcon, "shellScript", "text-icon-code"),
  fish: localizedPresentation(FileCodeIcon, "shellScript", "text-icon-code"),
  bat: localizedPresentation(FileCodeIcon, "batchFile", "text-icon-code"),
  cmd: localizedPresentation(FileCodeIcon, "batchFile", "text-icon-code"),
  ps1: localizedPresentation(FileCodeIcon, "powerShell", "text-icon-code"),
  sql: localizedPresentation(FileSqlIcon, "sql", "text-icon-code"),
  go: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  java: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  kt: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  swift: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  php: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  rb: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  lua: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  pl: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  r: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  dart: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  zig: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  hs: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  wasm: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  map: localizedPresentation(FileCodeIcon, "sourceCode", "text-icon-code"),
  patch: localizedPresentation(FileCodeIcon, "patchFile", "text-icon-code"),
  diff: localizedPresentation(FileCodeIcon, "patchFile", "text-icon-code"),
  yaml: localizedPresentation(FileIniIcon, "yaml", "text-icon-config"),
  yml: localizedPresentation(FileIniIcon, "yaml", "text-icon-config"),
  toml: localizedPresentation(FileIniIcon, "toml", "text-icon-config"),
  ini: localizedPresentation(FileIniIcon, "iniConfig", "text-icon-config"),
  conf: localizedPresentation(FileIniIcon, "configFile", "text-icon-config"),
  env: localizedPresentation(FileIniIcon, "configFile", "text-icon-config"),
  properties: localizedPresentation(FileIniIcon, "configFile", "text-icon-config"),
  db: localizedPresentation(FileSqlIcon, "database", "text-icon-sheet"),
  sqlite: localizedPresentation(FileSqlIcon, "database", "text-icon-sheet"),
  sqlite3: localizedPresentation(FileSqlIcon, "database", "text-icon-sheet"),
  mdb: localizedPresentation(FileSqlIcon, "database", "text-icon-sheet"),
  ttf: localizedPresentation(TextAaIcon, "font", "text-icon-font"),
  otf: localizedPresentation(TextAaIcon, "font", "text-icon-font"),
  woff: localizedPresentation(TextAaIcon, "font", "text-icon-font"),
  woff2: localizedPresentation(TextAaIcon, "font", "text-icon-font"),
  eot: localizedPresentation(TextAaIcon, "font", "text-icon-font"),
  exe: localizedPresentation(AppWindowIcon, "executable", "text-icon-exec"),
  msi: localizedPresentation(AppWindowIcon, "installer", "text-icon-exec"),
  appimage: localizedPresentation(AppWindowIcon, "executable", "text-icon-exec"),
  deb: localizedPresentation(PackageIcon, "package", "text-icon-exec"),
  rpm: localizedPresentation(PackageIcon, "package", "text-icon-exec"),
  jar: localizedPresentation(PackageIcon, "package", "text-icon-exec"),
  lnk: localizedPresentation(LinkSimpleIcon, "shortcut", "text-icon-exec"),
  url: localizedPresentation(LinkSimpleIcon, "shortcut", "text-icon-exec"),
  lock: localizedPresentation(FileLockIcon, "lockFile", "text-icon-lock"),
  pem: localizedPresentation(FileLockIcon, "privateKey", "text-icon-lock"),
  key: localizedPresentation(FileLockIcon, "privateKey", "text-icon-lock"),
  crt: localizedPresentation(FileLockIcon, "certificate", "text-icon-lock"),
  cer: localizedPresentation(FileLockIcon, "certificate", "text-icon-lock"),
};

const PLAIN_FILE_PRESENTATION: ExtensionPresentation = localizedPresentation(FileTextIcon, "file");

/** Well-known filenames that carry meaning beyond their extension. */
const FILENAME_PRESENTATION: Record<string, ExtensionPresentation> = {
  license: localizedPresentation(FileLockIcon, "license", "text-icon-lock"),
  licence: localizedPresentation(FileLockIcon, "license", "text-icon-lock"),
  copying: localizedPresentation(FileLockIcon, "license", "text-icon-lock"),
  readme: localizedPresentation(FileMdIcon, "markdown", "text-icon-code"),
  changelog: localizedPresentation(FileMdIcon, "markdown", "text-icon-code"),
  authors: localizedPresentation(FileMdIcon, "markdown", "text-icon-code"),
  contributing: localizedPresentation(FileMdIcon, "markdown", "text-icon-code"),
  makefile: localizedPresentation(FileIniIcon, "configFile", "text-icon-config"),
  justfile: localizedPresentation(FileIniIcon, "configFile", "text-icon-config"),
  dockerfile: localizedPresentation(FileIniIcon, "configFile", "text-icon-config"),
  "package.json": localizedPresentation(FileCodeIcon, "configFile", "text-icon-config"),
  "tsconfig.json": localizedPresentation(FileCodeIcon, "configFile", "text-icon-config"),
};

/** Leading-dot files (.gitignore, .env, .npmrc, ...) read as configuration. */
const DOTFILE_PRESENTATION: ExtensionPresentation = localizedPresentation(
  GearSixIcon,
  "configFile",
  "text-icon-config",
);

export function getFileExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) {
    return "";
  }

  return name.slice(dotIndex + 1).toLowerCase();
}

export function getFilePresentation(name: string): ExtensionPresentation {
  const byName = FILENAME_PRESENTATION[name.toLowerCase()];
  if (byName) {
    return byName;
  }

  const extension = getFileExtension(name);
  if (extension) {
    return EXTENSION_PRESENTATION[extension] ?? PLAIN_FILE_PRESENTATION;
  }

  if (name.startsWith(".")) {
    return DOTFILE_PRESENTATION;
  }

  return PLAIN_FILE_PRESENTATION;
}

export const SYMLINK_PRESENTATION: ExtensionPresentation = localizedPresentation(
  LinkSimpleIcon,
  "symlink",
);

export const OTHER_PRESENTATION: ExtensionPresentation = localizedPresentation(
  FileDashedIcon,
  "other",
);

export const DIRECTORY_PRESENTATION: ExtensionPresentation = localizedPresentation(
  FolderClosedIcon,
  "directory",
  "text-folder",
);

/** Kind-aware presentation used by every view and the preview surface. */
export function getEntryPresentation(
  entry: import("./types").DirectoryEntry,
): ExtensionPresentation {
  switch (entry.kind) {
    case "directory":
      return DIRECTORY_PRESENTATION;
    case "symlink":
      return SYMLINK_PRESENTATION;
    case "other":
      return OTHER_PRESENTATION;
    default:
      return getFilePresentation(entry.name);
  }
}

/** Icon color class for a presentation; untoned kinds stay muted. */
export function getPresentationIconClassName(presentation: ExtensionPresentation): string {
  return presentation.tone ?? "text-muted-foreground";
}
