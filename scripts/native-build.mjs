import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const debug = process.argv.includes("--debug");
const manifest = path.join(root, "native", "Cargo.toml");

const args = ["build", "--manifest-path", manifest];
if (!debug) args.push("--release");

console.log(`[native] cargo ${args.join(" ")}`);
const cargo = spawnSync("cargo", args, { cwd: root, stdio: "inherit" });
if (cargo.error) {
  console.error(`[native] unable to run cargo: ${cargo.error.message}`);
  process.exit(1);
}
if (cargo.status !== 0) process.exit(cargo.status ?? 1);

const profile = debug ? "debug" : "release";
const artifact = {
  win32: "dae_core_phase0.dll",
  darwin: "libdae_core_phase0.dylib",
  linux: "libdae_core_phase0.so",
}[process.platform];
if (!artifact) {
  console.error(`[native] unsupported platform: ${process.platform}`);
  process.exit(1);
}

const built = path.join(root, "native", "target", profile, artifact);
if (!existsSync(built)) {
  console.error(`[native] cargo finished but ${built} is missing`);
  process.exit(1);
}

const outDir = path.join(root, "native");
mkdirSync(outDir, { recursive: true });
const out = path.join(outDir, "dae_core_phase0.node");
copyFileSync(built, out);
console.log(`[native] wrote ${path.relative(root, out)}`);
