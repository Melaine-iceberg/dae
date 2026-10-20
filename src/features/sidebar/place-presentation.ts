import {
  DownloadIcon,
  FileTextIcon,
  GalleryIcon,
  HomeIcon,
  MonitorIcon,
  MusicNoteIcon,
  VideoFrameIcon,
} from "@solar-icons/react/linear";
import type { Icon } from "@solar-icons/react/lib/types";

import type { PlaceKind } from "@/bindings";

import { i18n } from "@/i18n";

/** Sidebar place icons stay on Solar — that is UI chrome, not file type. */

/**
 * Soft identity hues for the well-known places — the same category-colour
 * vocabulary the type glyphs use (`--folder` + the six `--tone-*`), so a place
 * reads as *this kind of thing* without borrowing the accent seam. The glyph
 * is a single-tone line (App.css, ICONS), so the hue is all there is to set.
 *
 * home and the user's own favourites are folders and take the folder hue;
 * the rest take the hue of the file family they collect.
 */
export const PLACE_TONE: Record<PlaceKind, string> = {
  home: "text-folder",
  desktop: "text-tone-cyan",
  documents: "text-tone-blue",
  downloads: "text-tone-emerald",
  pictures: "text-tone-violet",
  music: "text-tone-rose",
  videos: "text-tone-amber",
};

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
