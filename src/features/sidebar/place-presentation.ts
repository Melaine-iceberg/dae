import {
  DownloadIcon,
  FileTextIcon,
  GalleryIcon,
  HomeIcon,
  MonitorIcon,
  MusicNoteIcon,
  VideoFrameIcon,
} from "@solar-icons/react/line-duotone";
import type { Icon } from "@solar-icons/react/lib/types";

import type { PlaceKind } from "@/bindings";

import { i18n } from "@/i18n";

/** Sidebar place icons stay on Solar — that is UI chrome, not file type. */

/** Icons and labels for the well-known system places. */
export const PLACE_PRESENTATION: Record<PlaceKind, { icon: Icon; label: string }> = {
  home: {
    icon: HomeIcon,
    get label() {
      return i18n.t("sidebar:places.home");
    },
  },
  desktop: {
    icon: MonitorIcon,
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
    icon: DownloadIcon,
    get label() {
      return i18n.t("sidebar:places.downloads");
    },
  },
  pictures: {
    icon: GalleryIcon,
    get label() {
      return i18n.t("sidebar:places.pictures");
    },
  },
  music: {
    icon: MusicNoteIcon,
    get label() {
      return i18n.t("sidebar:places.music");
    },
  },
  videos: {
    icon: VideoFrameIcon,
    get label() {
      return i18n.t("sidebar:places.videos");
    },
  },
};
