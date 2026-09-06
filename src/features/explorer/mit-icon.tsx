import type { ComponentType } from "react";

/**
 * Material Icon Theme artwork, vendored into src/assets/mit-icons by
 * scripts/generate-mit-icons.mjs and served as plain asset URLs. Rendering
 * goes through <img> on purpose: a thousand inline SVG strings would bloat
 * the JS bundle, while asset URLs stream from disk and stay browser-cached.
 */
const ICON_URLS = import.meta.glob("../../assets/mit-icons/*.svg", {
  eager: true,
  import: "default",
  query: "?url",
}) as Record<string, string>;

const FALLBACK_URL = ICON_URLS["../../assets/mit-icons/file.svg"];

export interface EntryIconProps {
  className?: string;
  size?: number | string;
}

export type EntryIcon = ComponentType<EntryIconProps>;

const componentCache = new Map<string, EntryIcon>();

/**
 * Stable component per artwork name: presentations are looked up per row on
 * every render, and a fresh component identity each time would remount the
 * whole icon subtree in virtualized lists.
 */
export function mitIcon(name: string): EntryIcon {
  const cached = componentCache.get(name);
  if (cached) {
    return cached;
  }

  const src = ICON_URLS[`../../assets/mit-icons/${name}.svg`] ?? FALLBACK_URL;
  const component: EntryIcon = function MitIcon({ className, size }: EntryIconProps) {
    return (
      <img
        alt=""
        className={className}
        draggable={false}
        src={src}
        style={size === undefined ? undefined : { width: size, height: size }}
      />
    );
  };
  componentCache.set(name, component);
  return component;
}
