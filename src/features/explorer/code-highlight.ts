import type { HighlighterCore } from "shiki/core";

/** Preview themes paired with the app's light/dark surfaces. */
const LIGHT_THEME = "github-light";
const DARK_THEME = "github-dark";

/**
 * Per-language grammar modules, imported lazily so only the languages that
 * are actually previewed ever load.
 */
const LANGUAGE_IMPORTS = {
  typescript: () => import("shiki/langs/typescript.mjs"),
  tsx: () => import("shiki/langs/tsx.mjs"),
  javascript: () => import("shiki/langs/javascript.mjs"),
  jsx: () => import("shiki/langs/jsx.mjs"),
  json: () => import("shiki/langs/json.mjs"),
  css: () => import("shiki/langs/css.mjs"),
  scss: () => import("shiki/langs/scss.mjs"),
  less: () => import("shiki/langs/less.mjs"),
  html: () => import("shiki/langs/html.mjs"),
  xml: () => import("shiki/langs/xml.mjs"),
  vue: () => import("shiki/langs/vue.mjs"),
  svelte: () => import("shiki/langs/svelte.mjs"),
  markdown: () => import("shiki/langs/markdown.mjs"),
  python: () => import("shiki/langs/python.mjs"),
  ruby: () => import("shiki/langs/ruby.mjs"),
  rust: () => import("shiki/langs/rust.mjs"),
  go: () => import("shiki/langs/go.mjs"),
  java: () => import("shiki/langs/java.mjs"),
  kotlin: () => import("shiki/langs/kotlin.mjs"),
  c: () => import("shiki/langs/c.mjs"),
  cpp: () => import("shiki/langs/cpp.mjs"),
  csharp: () => import("shiki/langs/csharp.mjs"),
  bash: () => import("shiki/langs/bash.mjs"),
  powershell: () => import("shiki/langs/powershell.mjs"),
  sql: () => import("shiki/langs/sql.mjs"),
  yaml: () => import("shiki/langs/yaml.mjs"),
  toml: () => import("shiki/langs/toml.mjs"),
  ini: () => import("shiki/langs/ini.mjs"),
  dockerfile: () => import("shiki/langs/dockerfile.mjs"),
  lua: () => import("shiki/langs/lua.mjs"),
  php: () => import("shiki/langs/php.mjs"),
} as const;

export type CodeLanguage = keyof typeof LANGUAGE_IMPORTS;

/** File extension → grammar id for the preview panel. */
const EXTENSION_TO_LANGUAGE: Record<string, CodeLanguage> = {
  ts: "typescript",
  mts: "typescript",
  cts: "typescript",
  tsx: "tsx",
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  jsx: "jsx",
  json: "json",
  jsonc: "json",
  css: "css",
  scss: "scss",
  less: "less",
  html: "html",
  htm: "html",
  xml: "xml",
  svg: "xml",
  vue: "vue",
  svelte: "svelte",
  md: "markdown",
  markdown: "markdown",
  py: "python",
  rb: "ruby",
  rs: "rust",
  go: "go",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  hh: "cpp",
  cs: "csharp",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  ps1: "powershell",
  sql: "sql",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  ini: "ini",
  conf: "ini",
  dockerfile: "dockerfile",
  lua: "lua",
  php: "php",
};

/** Resolves a file name to a highlightable language, if any. */
export function getPreviewLanguage(fileName: string): CodeLanguage | null {
  const dotIndex = fileName.lastIndexOf(".");
  const extension =
    dotIndex > 0 && dotIndex < fileName.length - 1
      ? fileName.slice(dotIndex + 1).toLowerCase()
      : "";
  return EXTENSION_TO_LANGUAGE[extension] ?? null;
}

let highlighterPromise: Promise<HighlighterCore> | null = null;
const loadedLanguages = new Set<CodeLanguage>();

function getHighlighter(): Promise<HighlighterCore> {
  // The engine, themes and wasm load on first highlight so they stay out of
  // the application's startup bundle. Oniguruma (the reference TextMate
  // engine) beats the JS-regex engine by ~8x on heavy grammars like TS.
  highlighterPromise ??= (async () => {
    const [{ createHighlighterCore }, { createOnigurumaEngine }] = await Promise.all([
      import("shiki/core"),
      import("shiki/engine/oniguruma"),
    ]);
    return createHighlighterCore({
      themes: [import("shiki/themes/github-light.mjs"), import("shiki/themes/github-dark.mjs")],
      langs: [],
      engine: await createOnigurumaEngine(import("shiki/wasm")),
    });
  })();
  return highlighterPromise;
}

async function ensureLanguage(highlighter: HighlighterCore, language: CodeLanguage): Promise<void> {
  if (loadedLanguages.has(language)) return;
  await highlighter.loadLanguage(await LANGUAGE_IMPORTS[language]());
  loadedLanguages.add(language);
}

/**
 * Highlights code into HTML carrying both themes as CSS variables
 * (`--shiki-light` / `--shiki-dark`); App.css picks the active one.
 * Executed inside the highlight worker.
 */
export async function highlightText(code: string, language: CodeLanguage): Promise<string> {
  const highlighter = await getHighlighter();
  await ensureLanguage(highlighter, language);
  return highlighter.codeToHtml(code, {
    lang: language,
    themes: { light: LIGHT_THEME, dark: DARK_THEME },
    defaultColor: false,
  });
}

/** Messages sent to the highlight worker. */
export type HighlightRequestMessage = {
  type: "highlight";
  id: number;
  code: string;
  language: CodeLanguage;
};

/** Messages sent back from the highlight worker. */
export type HighlightResponseMessage =
  | { type: "result"; id: number; html: string }
  | { type: "error"; id: number; message: string };

type PendingRequest = {
  resolve: (html: string | null) => void;
  reject: (error: unknown) => void;
};

let highlightWorker: Worker | null = null;
let nextRequestId = 1;
const pendingRequests = new Map<number, PendingRequest>();

function getHighlightWorker(): Worker {
  if (highlightWorker !== null) return highlightWorker;

  const worker = new Worker(new URL("./highlight-worker.ts", import.meta.url), {
    type: "module",
  });
  worker.onmessage = (event: MessageEvent<HighlightResponseMessage>) => {
    const message = event.data;
    const request = pendingRequests.get(message.id);
    if (request === undefined) return;
    pendingRequests.delete(message.id);
    if (message.type === "result") request.resolve(message.html);
    else request.reject(new Error(message.message));
  };
  worker.onerror = (event: ErrorEvent) => {
    const error = new Error(event.message || "Highlight worker failed");
    for (const request of pendingRequests.values()) request.reject(error);
    pendingRequests.clear();
    worker.terminate();
    highlightWorker = null;
  };
  highlightWorker = worker;
  return worker;
}

/**
 * Highlights code off the main thread. The worker coalesces concurrent
 * requests, so rapid selection changes never stack stale highlighting
 * work. Resolves with `null` when superseded by a newer request.
 */
export function highlightCode(code: string, language: CodeLanguage): Promise<string | null> {
  const id = nextRequestId++;

  // Every pending request is older than this one and will never render;
  // settle them immediately instead of waiting for their results.
  for (const [pendingId, request] of pendingRequests) {
    pendingRequests.delete(pendingId);
    request.resolve(null);
  }

  const worker = getHighlightWorker();
  return new Promise((resolve, reject) => {
    pendingRequests.set(id, { resolve, reject });
    worker.postMessage({
      type: "highlight",
      id,
      code,
      language,
    } satisfies HighlightRequestMessage);
  });
}
