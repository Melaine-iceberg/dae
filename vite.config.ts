import { defineConfig } from "vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import { readFileSync } from "node:fs";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

/**
 * Boot-screen theme injection.
 *
 * `index.html` has to paint the window canvas and the splash spinner before
 * React — and therefore `App.css` — is parsed, so it cannot read the theme
 * tokens at runtime. It used to hardcode them, and silently fell two palette
 * revisions behind: every cold start flashed the wrong canvas colour. Reading
 * the tokens out of `App.css` here makes that file the single source of truth
 * — a renamed or moved token now fails the build with a named error instead of
 * shipping a wrong-coloured splash.
 */
function themeBootStyles() {
  return {
    name: "dae:theme-boot-styles",
    transformIndexHtml: {
      order: "pre" as const,
      handler(html: string) {
        const css = readFileSync(path.resolve(import.meta.dirname, "src/App.css"), "utf8");

        /** The body of the first `selector { … }` block, up to a closing brace
         *  in column 0. Anchoring on the line start keeps `.dark {` from
         *  matching the descendant rules (`.dark .tile-folder {`) further down. */
        const block = (selector: string) => {
          const match = new RegExp(`(?:^|\\n)${selector}\\s*\\{([\\s\\S]*?)\\n\\}`).exec(css);
          if (!match) throw new Error(`[dae] App.css: no ${selector} block found`);
          return match[1];
        };
        const token = (source: string, name: string) => {
          const match = new RegExp(`--${name}:\\s*([^;]+);`).exec(source);
          if (!match) throw new Error(`[dae] App.css: --${name} is not defined where expected`);
          return match[1].trim();
        };

        const light = block(":root");
        const dark = block("\\.dark");
        const values: Record<string, string> = {
          __DAE_CANVAS__: token(light, "background"),
          __DAE_PRIMARY__: token(light, "primary"),
          __DAE_BORDER__: token(light, "border"),
          __DAE_CANVAS_DARK__: token(dark, "background"),
          __DAE_PRIMARY_DARK__: token(dark, "primary"),
          __DAE_BORDER_DARK__: token(dark, "border"),
        };

        return Object.entries(values).reduce(
          (out, [placeholder, value]) => out.replaceAll(placeholder, value),
          html,
        );
      },
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async ({ command }) => ({
  plugins: [
    react(),
    // React Compiler auto-memoizes components and values at build time,
    // so manual React.memo/useMemo/useCallback are no longer needed.
    // Build-only: the Babel pass is expensive per module and would otherwise
    // run on every cold dev start, stretching the dev white screen.
    ...(command === "build" ? [babel({ presets: [reactCompilerPreset()] })] : []),
    tailwindcss(),
    themeBootStyles(),
  ],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  build: {
    // Catppuccin artwork must ship as on-disk asset URLs (see
    // src/features/explorer/catppuccin-icon.tsx): the default 4KB inline limit
    // would embed ~1300 SVGs as data URLs and bloat the entry chunk past 1.7MB.
    assetsInlineLimit: (filePath: string) =>
      filePath.includes("catppuccin-icons") ? false : undefined,
  },
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri` and `.workbuddy`
      ignored: [
        // Rust build trees are huge and change constantly.
        "**/src-tauri/**",
        "**/.workbuddy/**",
        // Scratch trees under `.workbuddy` are not part of the app, but the
        // watcher descends into them anyway — and stat-ing a stray symlink
        // loop there (e.g. the self-referencing `conf*.file` that autoconf
        // leaves behind when a C build aborts) emits an unhandled `error` on
        // the FSWatcher, which kills `vite` before Tauri can report anything.
      ],
    },
  },
}));
