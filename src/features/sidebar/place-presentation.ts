import {
  Monitor,
  Download,
  FileText,
  Home,
  Image,
  Music,
  Video,
  type LucideIcon,
} from "lucide-react";

import type { PlaceKind } from "@/bindings";

import { i18n } from "@/i18n";

/** Sidebar place icons stay on Lucide (UI glyphs, outside catppuccin scope). */

/** Icons and labels for the well-known system places. */
export const PLACE_PRESENTATION: Record<PlaceKind, { icon: LucideIcon; label: string }> = {
  home: {
    icon: Home,
    get label() {
      return i18n.t("sidebar:places.home");
    },
  },
  desktop: {
    icon: Monitor,
    get label() {
      return i18n.t("sidebar:places.desktop");
    },
  },
  documents: {
    icon: FileText,
    get label() {
      return i18n.t("sidebar:places.documents");
    },
  },
  downloads: {
    icon: Download,
    get label() {
      return i18n.t("sidebar:places.downloads");
    },
  },
  pictures: {
    icon: Image,
    get label() {
      return i18n.t("sidebar:places.pictures");
    },
  },
  music: {
    icon: Music,
    get label() {
      return i18n.t("sidebar:places.music");
    },
  },
  videos: {
    icon: Video,
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
