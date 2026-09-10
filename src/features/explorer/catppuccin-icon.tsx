import { useSyncExternalStore, type ComponentType } from "react";

/**
 * Catppuccin artwork, vendored into src/assets/catppuccin-icons by
 * scripts/generate-catppuccin-icons.mjs and served as plain asset URLs.
 * Rendering goes through <img> on purpose: a thousand inline SVG strings
 * would bloat the JS bundle, while asset URLs stream from disk and stay
 * browser-cached.
 */
const LIGHT_URLS = import.meta.glob("../../assets/catppuccin-icons/light/*.svg", {
  eager: true,
  import: "default",
  query: "?url",
}) as Record<string, string>;

const DARK_URLS = import.meta.glob("../../assets/catppuccin-icons/dark/*.svg", {
  eager: true,
  import: "default",
  query: "?url",
}) as Record<string, string>;

const LIGHT_FALLBACK = LIGHT_URLS["../../assets/catppuccin-icons/light/file.svg"];
const DARK_FALLBACK = DARK_URLS["../../assets/catppuccin-icons/dark/file.svg"];

/** Notify mounted icons when the theme class changes so their art follows. */
const THEME_SUBSCRIBERS = new Set<() => void>();
const themeObserver = new MutationObserver(() => {
  for (const notify of THEME_SUBSCRIBERS) {
    notify();
  }
});
themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });

function subscribeTheme(onStoreChange: () => void): () => void {
  THEME_SUBSCRIBERS.add(onStoreChange);
  return () => {
    THEME_SUBSCRIBERS.delete(onStoreChange);
  };
}

function isDarkTheme(): boolean {
  return document.documentElement.classList.contains("dark");
}

export interface EntryIconProps {
  className?: string;
  size?: number | string;
}

export type EntryIcon = ComponentType<EntryIconProps>;

const componentCache = new Map<string, EntryIcon>();

/**
 * Stable component per artwork name: presentations are looked up per row on
 * every render, and a fresh component identity each time would remount the
 * whole icon subtree in virtualized lists. The flavor URL (latte vs mocha)
 * is picked per render so a theme switch swaps art without remounting.
 */
export function catppuccinIcon(name: string): EntryIcon {
  const cached = componentCache.get(name);
  if (cached) {
    return cached;
  }

  const lightKey = `../../assets/catppuccin-icons/light/${name}.svg`;
  const darkKey = `../../assets/catppuccin-icons/dark/${name}.svg`;
  const lightSrc = LIGHT_URLS[lightKey] ?? LIGHT_FALLBACK;
  const darkSrc = DARK_URLS[darkKey] ?? DARK_FALLBACK;

  const component: EntryIcon = function CatppuccinIcon({ className, size }) {
    const dark = useSyncExternalStore(subscribeTheme, isDarkTheme);
    return (
      <img
        alt=""
        className={className}
        draggable={false}
        src={dark ? darkSrc : lightSrc}
        style={size === undefined ? undefined : { width: size, height: size }}
      />
    );
  };
  componentCache.set(name, component);
  return component;
}
