import { defineConfig } from "vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

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
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
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
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
