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
