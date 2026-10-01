import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * The app UI is the webview's own document, so a bare anchor click would
 * navigate the whole file manager away (the markdown preview renders user
 * content with real links). Intercept clicks in the capture phase before they
 * reach any anchor and never let the default navigation through.
 *
 * Only the schemes the system knows how to open leave the app; everything
 * else is a filesystem path, which stays in the DOM so the surface that owns
 * it (the preview panel resolves links against the document it renders) can
 * take over after propagation continues.
 */
const EXTERNAL_SCHEMES = /^(?:https?|mailto|tel):/i;

export function setupExternalLinkGuard(): void {
  document.addEventListener(
    "click",
    (event) => {
      const anchor = (event.target as Element | null)?.closest?.("a");
      if (!anchor) return;

      const href = anchor.getAttribute("href");
      if (!href || href.startsWith("#")) return;

      event.preventDefault();
      if (!EXTERNAL_SCHEMES.test(href)) return;

      event.stopPropagation();
      void openUrl(href).catch((error) => {
        console.warn(`Unable to open link ${href}`, error);
      });
    },
    true,
  );
}
