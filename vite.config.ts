import { defineConfig, type Plugin } from "vite";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import babel from "@rolldown/plugin-babel";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// The app only renders the regular/fill/bold/duotone icon weights, but every
// @phosphor-icons/react def module embeds all six weights as runtime Map
// data that tree-shaking can't remove. Entries are machine-generated with
// fixed indentation, so strip the unused weights with a scoped transform.
// Failure mode is safe: if the format ever changes, nothing matches and the
// module passes through untouched.
function stripPhosphorWeights(): Plugin {
  const entry = /^ {2}\[\n {4}"(?:thin|light)",\n[\s\S]*?^ {2}\],?\n/gm;
  return {
    name: "strip-phosphor-weights",
    transform(code, id) {
      if (!id.includes("@phosphor-icons") || !/defs[\\/][^\\/]+\.es\.js$/.test(id)) {
        return null;
      }
      const stripped = code.replace(entry, "");
      return stripped === code ? null : { code: stripped, map: null };
    },
  };
}

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    stripPhosphorWeights(),
    react(),
    // React Compiler auto-memoizes components and values at build time,
    // so manual React.memo/useMemo/useCallback are no longer needed.
    babel({ presets: [reactCompilerPreset()] }),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },

  // Emit workers as ES modules so dynamic imports (shiki grammars, wasm)
  // stay lazy chunks instead of being inlined into one huge worker bundle.
  worker: {
    format: "es",
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
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
