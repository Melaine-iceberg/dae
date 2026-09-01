import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * The app UI is the webview's own document, so a bare anchor click would
 * navigate the whole file manager away (the markdown preview renders user
 * content with real links). Intercept clicks in the capture phase before
 * they reach any anchor: scheme links hand off to the system browser and
 * stop propagating so no other handler double-opens them; relative links
 * keep propagating so context-aware handlers (e.g. the preview panel,
 * which resolves them against the source file) can take over.
 */
export function setupExternalLinkGuard(): void {
  document.addEventListener(
    "click",
    (event) => {
      const anchor = (event.target as Element | null)?.closest?.("a");
      if (!anchor) return;

      const href = anchor.getAttribute("href");
      if (!href || href.startsWith("#")) return;
      if (!/^[a-z][a-z0-9+.-]*:/i.test(href)) return;

      event.preventDefault();
      event.stopPropagation();
      void openUrl(href).catch((error) => {
        console.warn(`Unable to open link ${href}`, error);
      });
    },
    true,
  );
}
