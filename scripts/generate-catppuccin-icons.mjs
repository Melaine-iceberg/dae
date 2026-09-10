// Regenerates src/features/explorer/catppuccin-icons.generated.ts and copies
// Catppuccin artwork into src/assets/catppuccin-icons/{latte,mocha}.
//
// Usage: node scripts/generate-catppuccin-icons.mjs <art-dir>
//   <art-dir> holds latte/ and mocha/ folders of renamed SVGs (the rename
//   scheme is documented in the fetch script below). If omitted, the script
//   downloads + renames from the catppuccin/vscode-icons repo itself.
//
// The extension/filename/folder-name mapping tables come from Material Icon
// Theme's official JSON (node_modules/material-icon-theme) — the same data
// the previous MIT artwork used — plus a small hand-maintained SUPPLEMENT of
// entries the app needs beyond that JSON. Artwork references that the
// catppuccin set does not provide are resolved through ALIASES and finally
// fall back to the generic file / folder art, so every rendered slot always
// resolves to a real SVG.
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { execSync } from "node:child_process";
import path from "node:path";

const REPO = "https://github.com/catppuccin/vscode-icons/archive/refs/heads/main.tar.gz";
const ART_OUT = "src/assets/catppuccin-icons";
const GENERATED_OUT = "src/features/explorer/catppuccin-icons.generated.ts";
const FLAVORS = {
  light: "latte",
  dark: "mocha",
};

/** Hand-maintained entries the generated map needs on top of material-icons.json. */
const SUPPLEMENT = {
  EXT: {
    tmlanguage: "xml",
    "yaml-tmlanguage": "yaml",
  },
  FN: {
    ".github/funding.yml": "github-sponsors",
    ".rhistory": "r",
    apkbuild: "console",
    bashrc_apple_terminal: "console",
    caddyfile: "caddy",
    "claude.local.md": "claude",
    "claude.md": "claude",
    "cmakepresets.json": "cmake",
    cname: "http",
    commit_editmsg: "git",
    "gemini.md": "gemini-ai",
    "googleservice-info.plist": "google",
    merge_msg: "git",
    owners: "codeowners",
    pkgbuild: "console",
    pklproject: "pkl",
    "pklproject.deps.json": "pkl",
    snakefile: "snakemake",
    "sonarqube.analysis.xml": "sonarcloud",
    "xamlstyler.json": "xaml",
    zshrc_apple_terminal: "console",
  },
  FD: {
    "-ds_store": "folder-macos",
    "-github/issue_template": "folder-template",
    "-github/pull_request_template": "folder-template",
    "-ipad": "folder-macos",
    "-iphone": "folder-macos",
    "-ipod": "folder-macos",
    "-meta-inf": "folder-config",
    ".ds_store": "folder-macos",
    ".github/issue_template": "folder-template",
    ".github/pull_request_template": "folder-template",
    ".ipad": "folder-macos",
    ".iphone": "folder-macos",
    ".ipod": "folder-macos",
    ".meta-inf": "folder-config",
    __ds_store__: "folder-macos",
    __github_issue_template__: "folder-template",
    __github_pull_request_template__: "folder-template",
    __ipad__: "folder-macos",
    __iphone__: "folder-macos",
    __ipod__: "folder-macos",
    __meta_inf__: "folder-config",
    _ds_store: "folder-macos",
    _github_issue_template: "folder-template",
    _github_pull_request_template: "folder-template",
    _ipad: "folder-macos",
    _iphone: "folder-macos",
    _ipod: "folder-macos",
    _meta_inf: "folder-config",
    ds_store: "folder-macos",
    github_issue_template: "folder-template",
    github_pull_request_template: "folder-template",
    ipad: "folder-macos",
    iphone: "folder-macos",
    ipod: "folder-macos",
    meta_inf: "folder-config",
  },
  FO: {
    "-ds_store": "folder-macos-open",
    "-github/issue_template": "folder-template-open",
    "-github/pull_request_template": "folder-template-open",
    "-ipad": "folder-macos-open",
    "-iphone": "folder-macos-open",
    "-ipod": "folder-macos-open",
    "-meta-inf": "folder-config-open",
    ".ds_store": "folder-macos-open",
    ".github/issue_template": "folder-template-open",
    ".github/pull_request_template": "folder-template-open",
    ".ipad": "folder-macos-open",
    ".iphone": "folder-macos-open",
    ".ipod": "folder-macos-open",
    ".meta-inf": "folder-config-open",
    __ds_store__: "folder-macos-open",
    __github_issue_template__: "folder-template-open",
    __github_pull_request_template__: "folder-template-open",
    __ipad__: "folder-macos-open",
    __iphone__: "folder-macos-open",
    __ipod__: "folder-macos-open",
    __meta_inf__: "folder-config-open",
    _ds_store: "folder-macos-open",
    _github_issue_template: "folder-template-open",
    _github_pull_request_template: "folder-template-open",
    _ipad: "folder-macos-open",
    _iphone: "folder-macos-open",
    _ipod: "folder-macos-open",
    _meta_inf: "folder-config-open",
    ds_store: "folder-macos-open",
    github_issue_template: "folder-template-open",
    github_pull_request_template: "folder-template-open",
    ipad: "folder-macos-open",
    iphone: "folder-macos-open",
    ipod: "folder-macos-open",
    meta_inf: "folder-config-open",
  },
};

/** Folder-name base -> catppuccin base that exists (kept tight on purpose). */
const FOLDER_ALIASES = {
  "src-tauri": "tauri",
  rust: "cargo",
  lua: "luau",
  test: "tests",
  template: "templates",
  theme: "themes",
  hook: "hooks",
  layout: "layouts",
  i18n: "locales",
  css: "styles",
  sass: "styles",
  less: "styles",
  stylus: "styles",
  json: "config",
  environment: "config",
  "cloud-functions": "functions",
  "gh-workflows": "workflows",
  "gitea-workflows": "workflows",
  circleci: "circle-ci",
  drizzle: "drizzle-orm",
  typescript: "javascript",
  serverless: "server",
  favicon: "assets",
  tools: "utils",
  console: "command",
  interface: "types",
  attachment: "assets",
  resource: "assets",
  secure: "private",
  keys: "private",
  mail: "messages",
  android: "mobile",
  desktop: "mobile",
  ios: "mobile",
  script: "scripts",
  xcode: "macos",
  visualstudio: "vscode",
  "open-source": "github",
  pull_request: "github",
  issues: "github",
  moderations: "content",
  middleware: "server",
  handler: "functions",
};

/** File-art name -> catppuccin name that exists. */
const FILE_ALIASES = {
  settings: "config",
  word: "ms-word",
  powerpoint: "ms-powerpoint",
  jar: "java-jar",
  dll: "exe",
  react: "javascript-react",
  react_ts: "typescript-react",
  svelte_js: "svelte",
  svelte_ts: "svelte",
  gemfile: "ruby-gem",
  json_schema: "json-schema",
  document: "file",
  nodejs: "javascript",
  tsconfig: "typescript-config",
  tsdoc: "typescript",
  tsdown: "rollup",
  openapi: "swagger",
  dart_generated: "dart",
  drizzle: "drizzle-orm",
  tailwindcss: "tailwind",
  "python-misc": "python",
};

const DEFAULT_FILE_ICON = "file";
const DEFAULT_FOLDER_ICON = "folder";
const DEFAULT_FOLDER_OPEN_ICON = "folder-open";
const DEFAULT_SYMLINK_ICON = "settings";
const DEFAULT_DOTFILE_ICON = "settings";

function loadArt(srcDir) {
  const sets = {};
  for (const flavor of Object.values(FLAVORS)) {
    const dir = path.join(srcDir, flavor);
    if (!existsSync(dir)) throw new Error(`art source missing: ${dir}`);
    sets[flavor] = new Set(
      readdirSync(dir)
        .filter((f) => f.endsWith(".svg"))
        .map((f) => f.slice(0, -4)),
    );
  }
  return sets;
}

/** Resolves one art name to a name present in the given flavor set. */
function resolve(name, available) {
  if (available.has(name)) return name;
  const folderBase = name.match(/^folder-(.+?)(-open)?$/);
  const alias = folderBase ? FOLDER_ALIASES[folderBase[1]] : FILE_ALIASES[name];
  if (alias) {
    const target = folderBase ? `folder-${alias}${folderBase[2] ?? ""}` : alias;
    if (available.has(target)) return target;
  }
  if (name.startsWith("folder-"))
    return name.endsWith("-open") ? DEFAULT_FOLDER_OPEN_ICON : DEFAULT_FOLDER_ICON;
  return DEFAULT_FILE_ICON;
}

function emitMap(entries, available) {
  const lines = [];
  for (const [key, value] of entries) {
    const resolved = resolve(value, available);
    lines.push(`  ${JSON.stringify(key)}: ${JSON.stringify(resolved)},`);
  }
  return lines;
}

async function main() {
  const artArg = process.argv[2];
  if (artArg && !existsSync(artArg)) throw new Error(`art dir not found: ${artArg}`);

  let srcDir = artArg;
  if (!srcDir) {
    const tmp = path.join(process.cwd(), ".tmp-catppuccin-download");
    rmSync(tmp, { recursive: true, force: true });
    mkdirSync(tmp, { recursive: true });
    console.log("downloading", REPO);
    try {
      execSync(`curl -sL ${REPO} -o ${path.join(tmp, "icons.tar.gz")}`, { stdio: "ignore" });
    } catch (err) {
      throw new Error(`failed to download catppuccin icons: ${err.message}`);
    }
    try {
      execSync(
        `tar -xzf ${path.join(tmp, "icons.tar.gz")} -C ${tmp} --strip-components=1 vscode-icons-main/icons`,
        { stdio: "ignore" },
      );
    } catch (err) {
      throw new Error(`failed to extract catppuccin icons: ${err.message}`);
    }
    for (const flavor of Object.values(FLAVORS)) {
      const dir = path.join(tmp, flavor);
      for (const file of readdirSync(dir)) {
        let name = file;
        name = name.replace(/^_file\.svg$/, "file.svg");
        name = name.replace(/^_folder_open\.svg$/, "folder-open.svg");
        name = name.replace(/^_folder\.svg$/, "folder.svg");
        name = name.replace(/^_root_open\.svg$/, "root-open.svg");
        name = name.replace(/^_root\.svg$/, "root.svg");
        name = name.replace(/^folder_(.+)_open\.svg$/, "folder-$1-open.svg");
        name = name.replace(/^folder_(.+)\.svg$/, "folder-$1.svg");
        if (name !== file) renameSync(path.join(dir, file), path.join(dir, name));
      }
    }
    srcDir = tmp;
  }

  const available = loadArt(srcDir);
  let json;
  try {
    json = JSON.parse(
      readFileSync("node_modules/material-icon-theme/dist/material-icons.json", "utf8"),
    );
  } catch (err) {
    throw new Error(`failed to load material-icons.json: ${err.message}`);
  }

  const extEntries = [...Object.entries(json.fileExtensions), ...Object.entries(SUPPLEMENT.EXT)];
  const fnEntries = [...Object.entries(json.fileNames), ...Object.entries(SUPPLEMENT.FN)];
  const fdEntries = [...Object.entries(json.folderNames), ...Object.entries(SUPPLEMENT.FD)];
  const foEntries = [...Object.entries(json.folderNamesExpanded), ...Object.entries(SUPPLEMENT.FO)];

  const report = new Map();
  for (const avail of Object.values(available)) {
    for (const entries of [extEntries, fnEntries, fdEntries, foEntries]) {
      for (const [, value] of entries) {
        const resolved = resolve(value, avail);
        report.set(resolved, (report.get(resolved) ?? 0) + 1);
      }
    }
  }

  const out = [
    "// Generated by scripts/generate-catppuccin-icons.mjs \u2014 do not edit by hand.",
    "// Maps file extensions / full filenames / folder names to SVG basenames in",
    "// src/assets/catppuccin-icons. The mapping tables come from Material Icon",
    "// Theme's associations; artwork is Catppuccin (latte + mocha flavors) with",
    "// unresolvable names falling back to generic file/folder art.",
    `export const DEFAULT_FILE_ICON = ${JSON.stringify(DEFAULT_FILE_ICON)};`,
    `export const DEFAULT_FOLDER_ICON = ${JSON.stringify(DEFAULT_FOLDER_ICON)};`,
    `export const DEFAULT_FOLDER_OPEN_ICON = ${JSON.stringify(DEFAULT_FOLDER_OPEN_ICON)};`,
    `export const SYMLINK_ICON = ${JSON.stringify(DEFAULT_SYMLINK_ICON)};`,
    `export const DOTFILE_ICON = ${JSON.stringify(DEFAULT_DOTFILE_ICON)};`,
    "",
    `export const EXT_ICONS: Record<string, string> = {`,
    ...emitMap(extEntries, available[FLAVORS.light]),
    `};`,
    "",
    `export const FILENAME_ICONS: Record<string, string> = {`,
    ...emitMap(fnEntries, available[FLAVORS.light]),
    `};`,
    "",
    `export const FOLDER_ICONS: Record<string, string> = {`,
    ...emitMap(fdEntries, available[FLAVORS.light]),
    `};`,
    "",
    `export const FOLDER_OPEN_ICONS: Record<string, string> = {`,
    ...emitMap(foEntries, available[FLAVORS.light]),
    `};`,
    "",
  ];

  // Vendor the artwork beside the generated map.
  rmSync(ART_OUT, { recursive: true, force: true });
  for (const [flavorLabel, flavor] of Object.entries(FLAVORS)) {
    mkdirSync(path.join(ART_OUT, flavorLabel), { recursive: true });
    cpSync(path.join(srcDir, flavor), path.join(ART_OUT, flavorLabel), { recursive: true });
  }
  writeFileSync(GENERATED_OUT, out.join("\n"));

  const generic = [...report.keys()].filter((k) => k.startsWith("folder-") || k === "file").length;
  console.log(
    `art vendored: ${Object.values(FLAVORS)
      .map((f) => `${f}=${available[f].size} svg`)
      .join(", ")}`,
  );
  console.log(
    `generated: ${GENERATED_OUT} (ext ${extEntries.length}, fn ${fnEntries.length}, folder ${fdEntries.length})`,
  );
  console.log(
    `pool: ${report.size} distinct art names referenced; ${generic} fall back to generic file/folder.`,
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
