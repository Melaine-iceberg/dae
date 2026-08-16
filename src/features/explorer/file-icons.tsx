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

export type PhosphorIcon = Icon;

export const FolderClosedIcon = FolderIcon;
export const FolderOpenedIcon = FolderOpenIcon;

interface ExtensionPresentation {
  icon: PhosphorIcon;
  label: string;
}

const EXTENSION_PRESENTATION: Record<string, ExtensionPresentation> = {
  pdf: { icon: FilePdfIcon, label: "PDF 文档" },
  doc: { icon: FileDocIcon, label: "Word 文档" },
  docx: { icon: FileDocIcon, label: "Word 文档" },
  odt: { icon: FileDocIcon, label: "Word 文档" },
  rtf: { icon: FileDocIcon, label: "Word 文档" },
  xls: { icon: FileXlsIcon, label: "Excel 表格" },
  xlsx: { icon: FileXlsIcon, label: "Excel 表格" },
  ods: { icon: FileXlsIcon, label: "Excel 表格" },
  csv: { icon: FileCsvIcon, label: "CSV 表格" },
  ppt: { icon: FilePptIcon, label: "演示文稿" },
  pptx: { icon: FilePptIcon, label: "演示文稿" },
  odp: { icon: FilePptIcon, label: "演示文稿" },
  jpg: { icon: FileJpgIcon, label: "JPEG 图片" },
  jpeg: { icon: FileJpgIcon, label: "JPEG 图片" },
  png: { icon: FilePngIcon, label: "PNG 图片" },
  gif: { icon: FileImageIcon, label: "GIF 图片" },
  webp: { icon: FileImageIcon, label: "WebP 图片" },
  bmp: { icon: FileImageIcon, label: "位图图片" },
  ico: { icon: FileImageIcon, label: "图标" },
  tif: { icon: FileImageIcon, label: "TIFF 图片" },
  tiff: { icon: FileImageIcon, label: "TIFF 图片" },
  heic: { icon: FileImageIcon, label: "HEIC 图片" },
  svg: { icon: FileSvgIcon, label: "SVG 图片" },
  mp4: { icon: FileVideoIcon, label: "视频" },
  mov: { icon: FileVideoIcon, label: "视频" },
  avi: { icon: FileVideoIcon, label: "视频" },
  mkv: { icon: FileVideoIcon, label: "视频" },
  webm: { icon: FileVideoIcon, label: "视频" },
  m4v: { icon: FileVideoIcon, label: "视频" },
  mp3: { icon: FileAudioIcon, label: "音频" },
  wav: { icon: FileAudioIcon, label: "音频" },
  flac: { icon: FileAudioIcon, label: "音频" },
  aac: { icon: FileAudioIcon, label: "音频" },
  ogg: { icon: FileAudioIcon, label: "音频" },
  m4a: { icon: FileAudioIcon, label: "音频" },
  zip: { icon: FileZipIcon, label: "压缩文件" },
  rar: { icon: FileArchiveIcon, label: "压缩文件" },
  "7z": { icon: FileArchiveIcon, label: "压缩文件" },
  tar: { icon: FileArchiveIcon, label: "压缩文件" },
  gz: { icon: FileArchiveIcon, label: "压缩文件" },
  bz2: { icon: FileArchiveIcon, label: "压缩文件" },
  xz: { icon: FileArchiveIcon, label: "压缩文件" },
  txt: { icon: FileTxtIcon, label: "文本文件" },
  log: { icon: FileTxtIcon, label: "日志文件" },
  md: { icon: FileMdIcon, label: "Markdown" },
  markdown: { icon: FileMdIcon, label: "Markdown" },
  html: { icon: FileHtmlIcon, label: "HTML" },
  htm: { icon: FileHtmlIcon, label: "HTML" },
  css: { icon: FileCssIcon, label: "CSS" },
  scss: { icon: FileCssIcon, label: "SCSS" },
  less: { icon: FileCssIcon, label: "Less" },
  js: { icon: FileJsIcon, label: "JavaScript" },
  mjs: { icon: FileJsIcon, label: "JavaScript" },
  cjs: { icon: FileJsIcon, label: "JavaScript" },
  jsx: { icon: FileJsxIcon, label: "JSX" },
  ts: { icon: FileTsIcon, label: "TypeScript" },
  tsx: { icon: FileTsxIcon, label: "TSX" },
  py: { icon: FilePyIcon, label: "Python" },
  rs: { icon: FileRsIcon, label: "Rust" },
  c: { icon: FileCIcon, label: "C 语言" },
  h: { icon: FileCIcon, label: "C 头文件" },
  cpp: { icon: FileCppIcon, label: "C++" },
  hpp: { icon: FileCppIcon, label: "C++ 头文件" },
  cs: { icon: FileCSharpIcon, label: "C#" },
  vue: { icon: FileVueIcon, label: "Vue" },
  json: { icon: FileCodeIcon, label: "JSON" },
  xml: { icon: FileCodeIcon, label: "XML" },
  yaml: { icon: FileIniIcon, label: "YAML" },
  yml: { icon: FileIniIcon, label: "YAML" },
  toml: { icon: FileIniIcon, label: "TOML" },
  ini: { icon: FileIniIcon, label: "INI 配置" },
  conf: { icon: FileIniIcon, label: "配置文件" },
  sh: { icon: FileCodeIcon, label: "Shell 脚本" },
  bash: { icon: FileCodeIcon, label: "Shell 脚本" },
  zsh: { icon: FileCodeIcon, label: "Shell 脚本" },
  bat: { icon: FileCodeIcon, label: "批处理" },
  ps1: { icon: FileCodeIcon, label: "PowerShell" },
  sql: { icon: FileSqlIcon, label: "SQL" },
  exe: { icon: FileArrowUpIcon, label: "可执行文件" },
  msi: { icon: FileArrowUpIcon, label: "安装程序" },
  appimage: { icon: FileArrowUpIcon, label: "可执行文件" },
  deb: { icon: FileArrowUpIcon, label: "安装包" },
  rpm: { icon: FileArrowUpIcon, label: "安装包" },
  dmg: { icon: FileArrowUpIcon, label: "磁盘映像" },
  lock: { icon: FileLockIcon, label: "锁定文件" },
};

const DEFAULT_FILE_PRESENTATION: ExtensionPresentation = {
  icon: FileTextIcon,
  label: "文件",
};

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

export const SYMLINK_PRESENTATION: ExtensionPresentation = {
  icon: LinkSimpleIcon,
  label: "符号链接",
};

export const OTHER_PRESENTATION: ExtensionPresentation = {
  icon: FileDashedIcon,
  label: "其他",
};

export const DIRECTORY_PRESENTATION: ExtensionPresentation = {
  icon: FolderClosedIcon,
  label: "文件夹",
};

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
