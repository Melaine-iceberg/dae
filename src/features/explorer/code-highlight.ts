import { createHighlighter } from "@tanstack/highlight/core";
import { css } from "@tanstack/highlight/languages/css";
import { dockerfile } from "@tanstack/highlight/languages/dockerfile";
import { env } from "@tanstack/highlight/languages/env";
import { html } from "@tanstack/highlight/languages/html";
import { js } from "@tanstack/highlight/languages/js";
import { json } from "@tanstack/highlight/languages/json";
import { jsx } from "@tanstack/highlight/languages/jsx";
import { markdown } from "@tanstack/highlight/languages/markdown";
import { python } from "@tanstack/highlight/languages/python";
import { shell } from "@tanstack/highlight/languages/shell";
import { sql } from "@tanstack/highlight/languages/sql";
import { svelte } from "@tanstack/highlight/languages/svelte";
import { toml } from "@tanstack/highlight/languages/toml";
import { ts } from "@tanstack/highlight/languages/ts";
import { tsx } from "@tanstack/highlight/languages/tsx";
import { vue } from "@tanstack/highlight/languages/vue";
import { yaml } from "@tanstack/highlight/languages/yaml";

import { c, cpp, csharp, go, java, kotlin } from "./languages/c-like";
import { lua, php, powershell, ruby } from "./languages/scripting";
import { rust } from "./languages/rust";

/**
 * TanStack Highlight tokenizes synchronously and ships every grammar as a
 * few KB of plain JS — no wasm engine or worker needed, so all preview
 * languages register up front in one static list. Languages the package
 * does not ship (Rust, C-family, scripting) come from local grammars.
 */
const LANGUAGES = [
  ts,
  tsx,
  js,
  jsx,
  json,
  css,
  html,
  vue,
  svelte,
  markdown,
  python,
  shell,
  sql,
  yaml,
  toml,
  env,
  dockerfile,
  rust,
  c,
  cpp,
  csharp,
  go,
  java,
  kotlin,
  lua,
  php,
  powershell,
  ruby,
] as const;

export type CodeLanguage = (typeof LANGUAGES)[number]["name"];

const highlighter = createHighlighter({ languages: LANGUAGES });

/**
 * File extension → grammar id for the preview panel. Dialects without a
 * dedicated grammar borrow the closest supported one (SCSS/LESS → css,
 * XML/SVG → html, INI → env); unrecognized text stays plain and the
 * preview falls back to a text peek.
 */
const EXTENSION_TO_LANGUAGE: Record<string, CodeLanguage> = {
  ts: "ts",
  mts: "ts",
  cts: "ts",
  tsx: "tsx",
  js: "js",
  mjs: "js",
  cjs: "js",
  jsx: "jsx",
  json: "json",
  jsonc: "json",
  css: "css",
  scss: "css",
  less: "css",
  html: "html",
  htm: "html",
  xml: "html",
  svg: "html",
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
  sh: "shell",
  bash: "shell",
  zsh: "shell",
  ps1: "powershell",
  sql: "sql",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  ini: "env",
  conf: "env",
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

/**
 * Highlights code synchronously into a `th-*` class tree; App.css maps the
 * classes onto the GitHub light/dark palettes through CSS variables.
 */
export function highlightCode(code: string, language: CodeLanguage): string {
  return highlighter.highlight(code, { lang: language }).html;
}
