#!/usr/bin/env node
/**
 * Keeps Package.appxmanifest's four-part Version in step with package.json
 * so the Windows "installed apps" list and the in-app About pane never drift
 * apart (run as the first step of `pack:msix`).
 *
 * Rules:
 *  - App version lives in three files (package.json, tauri.conf.json,
 *    Cargo.toml). A mismatch only warns — those are kept in sync by hand at
 *    release time, and blocking a pack over it helps nobody.
 *  - The MSIX base (first three parts) always follows package.json. When the
 *    app version changes, the manifest resets to `<app>.0`.
 *  - Packing the same app version again bumps the revision (fourth part),
 *    because Windows rejects installing a package that is not strictly newer
 *    than the installed one.
 *  - If a *higher* base track is installed (e.g. the pre-unification 1.0.x
 *    track), no revision can catch up: fail loudly and ask for an uninstall
 *    instead of producing an .msix that will be refused at install time.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

/** Reads a JSON file relative to the project root, failing the pack loudly. */
function readJson(relPath) {
  const file = path.join(root, relPath);
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    console.error(
      `✖ cannot read ${relPath}: ${error instanceof SyntaxError ? "invalid JSON" : error.message}`,
    );
    process.exit(1);
  }
}

const appVersion = readJson("package.json").version;
if (!/^\d+\.\d+\.\d+$/.test(appVersion)) {
  console.error(`✖ package.json version "${appVersion}" is not MAJOR.MINOR.PATCH`);
  process.exit(1);
}

// Cross-file drift is informational: package.json is the source of truth for
// this script, and a hard stop here would only ever fire mid-release.
const drift = [
  ["src-tauri/tauri.conf.json", readJson("src-tauri/tauri.conf.json").version],
  ["src-tauri/Cargo.toml", /^version\s*=\s*"([^"]+)"/m.exec(readFileSync(path.join(root, "src-tauri/Cargo.toml"), "utf8"))?.[1]],
].filter(([, version]) => version !== appVersion);
for (const [file, version] of drift) {
  console.warn(`⚠ version drift: package.json ${appVersion} vs ${file} ${version}`);
}

/** Installed MSIX version, or null when nothing is installed / not on Windows. */
function installedVersion() {
  if (process.platform !== "win32") return null;
  try {
    const out = execSync('powershell -NoProfile -Command "(Get-AppxPackage -Name dae).Version"', {
      stdio: ["ignore", "pipe", "ignore"],
    })
      .toString()
      .trim();
    return out === "" ? null : out;
  } catch {
    return null;
  }
}

/** -1 / 0 / 1 over four numeric parts; `a` and `b` are dotted strings. */
function compare(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 4; i++) {
    if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  }
  return 0;
}

const manifestPath = path.join(root, "Package.appxmanifest");
const manifest = readFileSync(manifestPath, "utf8");
const current = /Version="(\d+\.\d+\.\d+\.\d+)"/.exec(manifest)?.[1];
if (!current) {
  console.error("✖ Package.appxmanifest has no four-part Version attribute");
  process.exit(1);
}

// Start from package.json, carrying the revision over (plus one) when the
// base is unchanged since the last pack.
let revision = current.startsWith(`${appVersion}.`)
  ? Number(current.slice(appVersion.length + 1)) + 1
  : 0;

const installed = installedVersion();
if (installed) {
  const installedBase = installed.split(".").slice(0, 3).join(".");
  if (compare(installedBase, appVersion) > 0) {
    console.error(
      `✖ Installed MSIX ${installed} is on a higher version track than the app ` +
        `(package.json ${appVersion}). Windows refuses downgrades — uninstall it first:\n` +
        `    powershell -Command "Remove-AppxPackage -Package (Get-AppxPackage -Name dae)"`,
    );
    process.exit(1);
  }
  let target;
  do {
    target = `${appVersion}.${revision}`;
    if (compare(target, installed) <= 0) revision += 1;
  } while (compare(target, installed) <= 0);
}

const target = `${appVersion}.${revision}`;
writeFileSync(manifestPath, manifest.replace(/(Version=")\d+\.\d+\.\d+\.\d+(")/, `$1${target}$2`));
console.log(`✅ MSIX version: ${current} → ${target} (app ${appVersion}${installed ? `, installed ${installed}` : ""})`);
