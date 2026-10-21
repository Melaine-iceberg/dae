import { spawn, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const root = path.resolve(import.meta.dirname, "..");
const electronPath = require("electron");
let exitCode = 1;

function argValue(name, fallback) {
  const prefix = `--${name}=`;
  const hit = process.argv.find((argument) => argument.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : fallback;
}

const packaged = process.argv.includes("--packaged");
const keep = process.argv.includes("--keep");
const skipBuild = process.argv.includes("--skip-build");
const quick = process.argv.includes("--quick");
const count = Number(argValue("count", quick ? "3000" : "35000"));
const imageCount = 100;

if (!Number.isFinite(count) || count < 1) {
  console.error("--count must be a positive number");
  process.exit(2);
}

const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const runId = `run_${Date.now().toString(36)}_${randomBytes(3).toString("hex")}`;
const marker = `DAE_PHASE0_${runId}`;
const baseDir = await mkdtemp(path.join(os.tmpdir(), "dae-phase0-"));
const listingDir = path.join(baseDir, "listing");
const imagesDir = path.join(baseDir, "images");
const userDataDir = path.join(baseDir, "userdata");
const resultsDir = path.join(root, "bench-results");
const outFile = path.join(resultsDir, `phase0-${packaged ? "packaged" : "dev"}-${stamp}.json`);
const logFile = path.join(resultsDir, `phase0-${packaged ? "packaged" : "dev"}-${stamp}.log`);

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    shell: process.platform === "win32" && command.endsWith(".cmd"),
    ...options,
  });
  if (result.error) {
    throw new Error(`${command} failed to start: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
}

async function createListing(count) {
  console.log(`[accept] creating ${count} files in ${listingDir}`);
  await mkdir(listingDir, { recursive: true });
  let next = 0;
  let reported = 0;
  const started = Date.now();
  const workers = Array.from({ length: Math.min(192, count) }, async () => {
    for (;;) {
      const index = next;
      next += 1;
      if (index >= count) return;
      await writeFile(path.join(listingDir, `file-${String(index).padStart(6, "0")}.txt`), "x");
      if (index - reported >= 5000) {
        reported = index;
        console.log(`[accept]   ${reported}/${count} files`);
      }
    }
  });
  await Promise.all(workers);
  console.log(`[accept] listing fixture ready in ${Date.now() - started}ms`);
}

async function createImages() {
  const source = path.join(root, "src-tauri", "icons", "128x128.png");
  await mkdir(imagesDir, { recursive: true });
  await Promise.all(
    Array.from({ length: imageCount }, (_, index) =>
      copyFile(source, path.join(imagesDir, `img-${String(index).padStart(3, "0")}.png`)),
    ),
  );
  console.log(`[accept] ${imageCount} image fixtures ready`);
}

function buildNative() {
  if (skipBuild) {
    console.log("[accept] skipping native build");
    return;
  }
  run(process.execPath, [path.join(root, "scripts", "native-build.mjs")]);
}

function buildPackagedApp() {
  const builderCli = path.join(
    root,
    "node_modules",
    "electron-builder",
    "out",
    "cli",
    "cli.js",
  );
  console.log("[accept] packaging Electron app (dir target)");
  run(process.execPath, [builderCli, "--win", "--dir"]);
}

function launch(args) {
  const command = packaged
    ? path.join(root, "dist-electron", "win-unpacked", "dae-phase0.exe")
    : electronPath;
  const launchArgs = packaged ? args : [path.join(root, "electron", "main.cjs"), ...args];

  return new Promise((resolve, reject) => {
    const child = spawn(command, launchArgs, {
      cwd: root,
      env: { ...process.env },
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: false,
    });

    let output = "";
    const append = (chunk) => {
      const text = chunk.toString();
      output += text;
      process.stdout.write(text.startsWith("[") ? text : `[app] ${text}`);
    };
    child.stdout.on("data", append);
    child.stderr.on("data", append);

    const timeout = setTimeout(() => {
      console.error("[accept] app timed out after 300s; killing its process tree");
      killTree(child.pid);
      reject(new Error("app timed out after 300s"));
    }, 300_000);

    child.on("error", (error) => {
      clearTimeout(timeout);
      reject(error);
    });
    child.on("exit", (code, signal) => {
      clearTimeout(timeout);
      resolve({ code, signal, output });
    });
  });
}

function killTree(pid) {
  if (!pid) return;
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
    return;
  }
  try {
    process.kill(-pid, "SIGKILL");
  } catch {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Already gone.
    }
  }
}

function checkOrphans() {
  if (process.platform === "win32") {
    const script = [
      `$m = 'DAE_PHASE0_' + '${runId}'`,
      "$hits = Get-CimInstance Win32_Process |",
      "  Where-Object { $_.CommandLine -and $_.CommandLine.Contains($m) -and $_.ProcessId -ne $PID }",
      "$hits | Select-Object -ExpandProperty ProcessId",
    ].join("; ");
    const result = spawnSync("powershell.exe", ["-NoProfile", "-Command", script], {
      encoding: "utf8",
    });
    const pids = (result.stdout || "")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => /^\d+$/.test(line));
    for (const pid of pids) {
      spawnSync("taskkill", ["/PID", pid, "/T", "/F"], { stdio: "ignore" });
    }
    return pids;
  }

  const result = spawnSync("ps", ["-eo", "pid=,command="], { encoding: "utf8" });
  const pids = (result.stdout || "")
    .split(/\r?\n/)
    .filter((line) => line.includes(marker) && !line.includes("phase0-acceptance"))
    .map((line) => Number(line.trim().split(/\s+/)[0]))
    .filter((pid) => Number.isFinite(pid) && pid !== process.pid);
  for (const pid of pids) killTree(pid);
  return pids;
}

function printSummary(result, orphans, appExit) {
  console.log("");
  console.log("Phase 0 acceptance");
  console.log(`  run id      ${runId}`);
  console.log(`  mode        ${packaged ? "packaged" : "dev"}`);
  console.log(`  app exit    ${appExit.code ?? appExit.signal}`);
  console.log(`  result      ${result?.pass ? "PASS" : "FAIL"}`);
  console.log(`  native      ${result?.nativePath ?? "unknown"}`);
  console.log(`  main lag    ${result?.mainLagMs ?? "n/a"} ms`);
  console.log(`  orphans     ${orphans.length}`);

  if (result?.phases) {
    console.log("");
    console.log("  phases");
    for (const [name, phase] of Object.entries(result.phases)) {
      console.log(`    ${name.padEnd(14)} ${JSON.stringify(phase)}`);
    }
    console.log("");
    console.log("  checks");
    for (const [name, ok] of Object.entries(result.checks || {})) {
      console.log(`    ${ok ? "PASS" : "FAIL"}  ${name}`);
    }
  }
  if (result?.warnings?.length) {
    console.log("");
    for (const warning of result.warnings) console.log(`  warning: ${warning}`);
  }
  if (result?.error) console.log(`  error: ${result.error}`);
}

async function main() {
  await mkdir(resultsDir, { recursive: true });
  await createListing(count);
  await createImages();
  buildNative();
  if (packaged) buildPackagedApp();

  const args = [
    "--bench",
    `--bench-dir=${listingDir}`,
    `--bench-images=${imagesDir}`,
    `--bench-count=${count}`,
    `--bench-out=${outFile}`,
    `--bench-run-id=${runId}`,
    `--bench-user-data=${userDataDir}`,
  ];

  const appExit = await launch(args);
  await writeFile(logFile, appExit.output, "utf8").catch(() => {});
  await new Promise((resolve) => setTimeout(resolve, 1000));

  let result = null;
  try {
    result = JSON.parse(await readFile(outFile, "utf8"));
  } catch (error) {
    console.error(`[accept] unable to read ${outFile}: ${error}`);
  }

  const orphans = checkOrphans();
  printSummary(result, orphans, appExit);

  const failures = [];
  if (appExit.code !== 0) failures.push(`app exited with code ${appExit.code}`);
  if (!result) failures.push("no result JSON");
  else if (!result.pass) failures.push("result.pass is false");
  if (orphans.length > 0) failures.push(`${orphans.length} orphaned PTY process(es)`);
  if (packaged && result && !String(result.nativePath || "").includes("app.asar.unpacked")) {
    failures.push("packaged run did not load the addon from app.asar.unpacked");
  }

  console.log("");
  if (failures.length > 0) {
    for (const failure of failures) console.log(`FAILED: ${failure}`);
    console.log(`Logs: ${logFile}`);
    exitCode = 1;
  } else {
    console.log(`PASS (logs: ${logFile})`);
    exitCode = 0;
  }

  if (!keep) {
    await rm(baseDir, { recursive: true, force: true }).catch((error) => {
      console.error(`[accept] cleanup failed: ${error}`);
    });
  } else {
    console.log(`[accept] kept fixtures at ${baseDir}`);
  }
}

try {
  await main();
} catch (error) {
  console.error(`[accept] ${error?.stack || error}`);
  exitCode = 1;
}
process.exit(exitCode);