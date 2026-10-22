#!/usr/bin/env node
/**
 * Packs the electron-builder `win-unpacked` layout into an MSIX with the
 * committed Package.appxmanifest + Assets/. `winapp` wants a package layout
 * whose root contains the executable and every manifest-referenced asset, so
 * the builder output is staged instead of packed in place.
 *
 * Run after `electron-builder --win --dir`. A development certificate is
 * generated when none is supplied; CI writes the optional release certificate
 * from its secret and points MSIX_CERT at it.
 */
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

if (process.platform !== "win32") {
  console.error("✖ MSIX packaging runs on Windows only");
  process.exit(1);
}

function fail(message) {
  console.error(`✖ ${message}`);
  process.exit(1);
}

const version = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")).version;
const outputDir = path.join(root, "dist-electron");
const appDir = path.join(outputDir, "win-unpacked");
const stagingDir = path.join(outputDir, "msix-layout");
const manifest = path.join(root, "Package.appxmanifest");
const assets = path.join(root, "Assets");

if (!existsSync(path.join(appDir, "dae.exe"))) {
  fail(`missing ${path.relative(root, appDir)}\\dae.exe; run \`electron-builder --win --dir\` first`);
}
if (!existsSync(manifest)) fail("Package.appxmanifest is missing");
if (!existsSync(assets)) fail("Assets/ is missing");

rmSync(stagingDir, { recursive: true, force: true });
mkdirSync(stagingDir, { recursive: true });
cpSync(appDir, stagingDir, { recursive: true });
cpSync(assets, path.join(stagingDir, "Assets"), { recursive: true });

function run(command, args) {
  console.log(`[msix] ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error) fail(`unable to run ${command}: ${result.error.message}`);
  if (result.status !== 0) process.exit(result.status ?? 1);
}

const cert = process.env.MSIX_CERT
  ? path.resolve(root, process.env.MSIX_CERT)
  : path.join(root, "devcert.pfx");
const certPassword = process.env.MSIX_CERT_PASSWORD || "password";
if (!existsSync(cert)) {
  run("winapp", [
    "cert",
    "generate",
    "--manifest",
    manifest,
    "--output",
    cert,
    "--password",
    certPassword,
    "--if-exists",
    "skip",
  ]);
}

const output = path.join(outputDir, `dae-${version}-${process.arch}.msix`);
run("winapp", [
  "pack",
  stagingDir,
  "--manifest",
  manifest,
  "--output",
  output,
  "--cert",
  cert,
  "--cert-password",
  certPassword,
]);

console.log(`[msix] wrote ${path.relative(root, output)}`);
