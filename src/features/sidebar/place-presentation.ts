import {
  DesktopIcon,
  DownloadSimpleIcon,
  FileTextIcon,
  HouseIcon,
  ImageIcon,
  MusicNotesIcon,
  VideoIcon,
} from "@phosphor-icons/react";

import type { PlaceKind } from "@/bindings";

import type { PhosphorIcon } from "@/features/explorer/file-icons";

/** Icons and labels for the well-known system places. */
export const PLACE_PRESENTATION: Record<PlaceKind, { icon: PhosphorIcon; label: string }> = {
  home: { icon: HouseIcon, label: "主文件夹" },
  desktop: { icon: DesktopIcon, label: "桌面" },
  documents: { icon: FileTextIcon, label: "文档" },
  downloads: { icon: DownloadSimpleIcon, label: "下载" },
  pictures: { icon: ImageIcon, label: "图片" },
  music: { icon: MusicNotesIcon, label: "音乐" },
  videos: { icon: VideoIcon, label: "视频" },
};
