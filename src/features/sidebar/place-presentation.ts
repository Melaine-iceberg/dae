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
import { i18n } from "@/i18n";

/** Icons and labels for the well-known system places. */
export const PLACE_PRESENTATION: Record<PlaceKind, { icon: PhosphorIcon; label: string }> = {
  home: {
    icon: HouseIcon,
    get label() {
      return i18n.t("sidebar:places.home");
    },
  },
  desktop: {
    icon: DesktopIcon,
    get label() {
      return i18n.t("sidebar:places.desktop");
    },
  },
  documents: {
    icon: FileTextIcon,
    get label() {
      return i18n.t("sidebar:places.documents");
    },
  },
  downloads: {
    icon: DownloadSimpleIcon,
    get label() {
      return i18n.t("sidebar:places.downloads");
    },
  },
  pictures: {
    icon: ImageIcon,
    get label() {
      return i18n.t("sidebar:places.pictures");
    },
  },
  music: {
    icon: MusicNotesIcon,
    get label() {
      return i18n.t("sidebar:places.music");
    },
  },
  videos: {
    icon: VideoIcon,
    get label() {
      return i18n.t("sidebar:places.videos");
    },
  },
};

/**
 * Semantic tone (a theme color variable) per place, used to tint icon chips
 * on workspace cards. Reuses the file-type palette so the whole app speaks
 * one color language: documents blue, pictures teal, music violet, ...
 */
export const PLACE_TONE_VAR: Record<PlaceKind, string> = {
  home: "--primary",
  desktop: "--icon-code",
  documents: "--icon-doc",
  downloads: "--icon-sheet",
  pictures: "--icon-image",
  music: "--icon-audio",
  videos: "--icon-video",
};
