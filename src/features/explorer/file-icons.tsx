import {
  FileArchiveIcon,
  FileArrowUpIcon,
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
  LinkSimpleIcon,
  type Icon,
} from "@phosphor-icons/react";

import { i18n } from "@/i18n";

export type PhosphorIcon = Icon;

export const FolderClosedIcon = FolderIcon;
export const FolderOpenedIcon = FolderOpenIcon;

interface ExtensionPresentation {
  icon: PhosphorIcon;
  label: string;
}

/** Presentation whose type label resolves against the active locale on read. */
function localizedPresentation(icon: PhosphorIcon, labelKey: string): ExtensionPresentation {
  return {
    icon,
    get label() {
      return i18n.t(`explorer:fileType.${labelKey}`);
    },
  };
}

const EXTENSION_PRESENTATION: Record<string, ExtensionPresentation> = {
  pdf: localizedPresentation(FilePdfIcon, "pdfDocument"),
  doc: localizedPresentation(FileDocIcon, "wordDocument"),
  docx: localizedPresentation(FileDocIcon, "wordDocument"),
  odt: localizedPresentation(FileDocIcon, "wordDocument"),
  rtf: localizedPresentation(FileDocIcon, "wordDocument"),
  xls: localizedPresentation(FileXlsIcon, "excelSpreadsheet"),
  xlsx: localizedPresentation(FileXlsIcon, "excelSpreadsheet"),
  ods: localizedPresentation(FileXlsIcon, "excelSpreadsheet"),
  csv: localizedPresentation(FileCsvIcon, "csvSpreadsheet"),
  ppt: localizedPresentation(FilePptIcon, "presentation"),
  pptx: localizedPresentation(FilePptIcon, "presentation"),
  odp: localizedPresentation(FilePptIcon, "presentation"),
  jpg: localizedPresentation(FileJpgIcon, "jpegImage"),
  jpeg: localizedPresentation(FileJpgIcon, "jpegImage"),
  png: localizedPresentation(FilePngIcon, "pngImage"),
  gif: localizedPresentation(FileImageIcon, "gifImage"),
  webp: localizedPresentation(FileImageIcon, "webpImage"),
  bmp: localizedPresentation(FileImageIcon, "bitmapImage"),
  ico: localizedPresentation(FileImageIcon, "icon"),
  tif: localizedPresentation(FileImageIcon, "tiffImage"),
  tiff: localizedPresentation(FileImageIcon, "tiffImage"),
  heic: localizedPresentation(FileImageIcon, "heicImage"),
  svg: localizedPresentation(FileSvgIcon, "svgImage"),
  mp4: localizedPresentation(FileVideoIcon, "video"),
  mov: localizedPresentation(FileVideoIcon, "video"),
  avi: localizedPresentation(FileVideoIcon, "video"),
  mkv: localizedPresentation(FileVideoIcon, "video"),
  webm: localizedPresentation(FileVideoIcon, "video"),
  m4v: localizedPresentation(FileVideoIcon, "video"),
  mp3: localizedPresentation(FileAudioIcon, "audio"),
  wav: localizedPresentation(FileAudioIcon, "audio"),
  flac: localizedPresentation(FileAudioIcon, "audio"),
  aac: localizedPresentation(FileAudioIcon, "audio"),
  ogg: localizedPresentation(FileAudioIcon, "audio"),
  m4a: localizedPresentation(FileAudioIcon, "audio"),
  zip: localizedPresentation(FileZipIcon, "archive"),
  rar: localizedPresentation(FileArchiveIcon, "archive"),
  "7z": localizedPresentation(FileArchiveIcon, "archive"),
  tar: localizedPresentation(FileArchiveIcon, "archive"),
  gz: localizedPresentation(FileArchiveIcon, "archive"),
  bz2: localizedPresentation(FileArchiveIcon, "archive"),
  xz: localizedPresentation(FileArchiveIcon, "archive"),
  txt: localizedPresentation(FileTxtIcon, "textFile"),
  log: localizedPresentation(FileTxtIcon, "logFile"),
  md: localizedPresentation(FileMdIcon, "markdown"),
  markdown: localizedPresentation(FileMdIcon, "markdown"),
  html: localizedPresentation(FileHtmlIcon, "html"),
  htm: localizedPresentation(FileHtmlIcon, "html"),
  css: localizedPresentation(FileCssIcon, "css"),
  scss: localizedPresentation(FileCssIcon, "scss"),
  less: localizedPresentation(FileCssIcon, "less"),
  js: localizedPresentation(FileJsIcon, "javaScript"),
  mjs: localizedPresentation(FileJsIcon, "javaScript"),
  cjs: localizedPresentation(FileJsIcon, "javaScript"),
  jsx: localizedPresentation(FileJsxIcon, "jsx"),
  ts: localizedPresentation(FileTsIcon, "typeScript"),
  tsx: localizedPresentation(FileTsxIcon, "tsx"),
  py: localizedPresentation(FilePyIcon, "python"),
  rs: localizedPresentation(FileRsIcon, "rust"),
  c: localizedPresentation(FileCIcon, "cSource"),
  h: localizedPresentation(FileCIcon, "cHeader"),
  cpp: localizedPresentation(FileCppIcon, "cpp"),
  hpp: localizedPresentation(FileCppIcon, "cppHeader"),
  cs: localizedPresentation(FileCSharpIcon, "cSharp"),
  vue: localizedPresentation(FileVueIcon, "vue"),
  json: localizedPresentation(FileCodeIcon, "json"),
  xml: localizedPresentation(FileCodeIcon, "xml"),
  yaml: localizedPresentation(FileIniIcon, "yaml"),
  yml: localizedPresentation(FileIniIcon, "yaml"),
  toml: localizedPresentation(FileIniIcon, "toml"),
  ini: localizedPresentation(FileIniIcon, "iniConfig"),
  conf: localizedPresentation(FileIniIcon, "configFile"),
  sh: localizedPresentation(FileCodeIcon, "shellScript"),
  bash: localizedPresentation(FileCodeIcon, "shellScript"),
  zsh: localizedPresentation(FileCodeIcon, "shellScript"),
  bat: localizedPresentation(FileCodeIcon, "batchFile"),
  ps1: localizedPresentation(FileCodeIcon, "powerShell"),
  sql: localizedPresentation(FileSqlIcon, "sql"),
  exe: localizedPresentation(FileArrowUpIcon, "executable"),
  msi: localizedPresentation(FileArrowUpIcon, "installer"),
  appimage: localizedPresentation(FileArrowUpIcon, "executable"),
  deb: localizedPresentation(FileArrowUpIcon, "package"),
  rpm: localizedPresentation(FileArrowUpIcon, "package"),
  dmg: localizedPresentation(FileArrowUpIcon, "diskImage"),
  lock: localizedPresentation(FileLockIcon, "lockFile"),
};

const DEFAULT_FILE_PRESENTATION: ExtensionPresentation = localizedPresentation(
  FileTextIcon,
  "file",
);

export function getFileExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) {
    return "";
  }

  return name.slice(dotIndex + 1).toLowerCase();
}

export function getFilePresentation(name: string): ExtensionPresentation {
  const extension = getFileExtension(name);
  if (!extension) {
    return DEFAULT_FILE_PRESENTATION;
  }

  return EXTENSION_PRESENTATION[extension] ?? DEFAULT_FILE_PRESENTATION;
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
