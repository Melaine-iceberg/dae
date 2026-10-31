// Shared plumbing for the PGO / BOLT builds. `docs/pgo-bolt.md` explains the
// flow; this file owns the parts that were easy to get wrong:
//
//   * **RUSTFLAGS.** `src-tauri/.cargo/config.toml` raises the Windows stack
//     reserve through `target.x86_64-pc-windows-msvc.rustflags`. Setting the
//     `RUSTFLAGS` environment variable *replaces* that array in cargo, so a PGO
//     build that just exported `RUSTFLAGS=-Cprofile-generate=…` would silently
//     ship an exe with the default 1MB stack and reintroduce the
//     STATUS_STACK_OVERFLOW that config exists to prevent. Every profile flag
//     below is therefore appended to the flags cargo would have used anyway,
//     and the result is handed over as `CARGO_ENCODED_RUSTFLAGS`, the only
//     form cargo splits without word-splitting on paths that contain spaces.
//   * **LLVM tools.** `llvm-profdata` must come from the toolchain rustc uses,
//     not from whatever LLVM happens to be on PATH (a mismatched profraw
//     version is an error at profile-use time, hours later). The rustup
//     `llvm-tools` component is preferred; BOLT is never in it, so it falls
//     back to distro paths.
//   * **Training runs.** A GUI app in a headless CI job needs a display, a
//     corpus to list, and — because both profile runtimes write their counters
//     from exit handlers — a *clean* way out of the process. The app grows one
//     for this (`DAE_PERF_TRAIN_QUIT_MS`, see `spawn_training_quit` in
//     `src-tauri/src/lib.rs`), and both callers share `runTrainingWorkload`.

import { spawn, spawnSync } from "node:child_process";
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  readSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = path.resolve(import.meta.dirname, "..", "..");
export const tauriDir = path.join(repoRoot, "src-tauri");
export const qtDir = path.join(repoRoot, "apps", "dae");
export const IS_WINDOWS = process.platform === "win32";
export const IS_LINUX = process.platform === "linux";
export const IS_MACOS = process.platform === "darwin";

/** Which shell the scripts profile. `tauri` is the old WebView shell;
 *  `qt` is the Qt Quick shell (`apps/dae`, cxx-qt bridge). Both are cargo
 *  builds, but they live in different workspaces and tauri build also runs
 *  the bundler, so every output path below depends on this choice. */
export const SHELLS = ["tauri", "qt"];

export function resolveShell(shell) {
  const name = shell ?? process.env.DAE_PERF_SHELL ?? "tauri";
  if (!SHELLS.includes(name)) fail(`unknown shell "${name}" — expected one of ${SHELLS.join(", ")}`);
  return name;
}

/** Profiles are build output, so they live beside the build: untracked, and
 *  outside `target/` so that `cargo clean` does not throw away a profile that
 *  cost a full instrumented build plus a manual training run to produce. */
export const perfDir = process.env.DAE_PERF_DIR
  ? path.resolve(process.env.DAE_PERF_DIR)
  : path.join(repoRoot, ".perf");
export const pgoDir = path.join(perfDir, "pgo");
export const pgoRawDir = path.join(pgoDir, "raw");
export const pgoProfilePath = path.join(pgoDir, "dae.profdata");
export const boltDir = path.join(perfDir, "bolt");
export const boltDataPath = path.join(boltDir, "dae.fdata");
export const corpusDir = path.join(perfDir, "corpus");

// ---------------------------------------------------------------------------
// output

export class PerfError extends Error {}

const paint = (code) => (text) =>
  process.stderr.isTTY && !process.env.NO_COLOR ? `\u001b[${code}m${text}\u001b[0m` : text;
const dim = paint("2");
const bold = paint("1");
const yellow = paint("33");
const red = paint("31");

// Everything diagnostic goes to stderr: `pgo.mjs flags` writes `$GITHUB_ENV`
// lines to stdout and must not be interrupted by a log line.
export const say = (message) => console.error(`${dim("·")} ${message}`);
export const step = (message) => console.error(`\n${bold("▸")} ${bold(message)}`);
export const warn = (message) => console.error(`${yellow("!")} ${message}`);
const detail = (message) => console.error(`  ${dim(message)}`);

export function fail(message) {
  throw new PerfError(message);
}

export function humanSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

export const fileSize = (file) => (existsSync(file) ? statSync(file).size : 0);

// ---------------------------------------------------------------------------
// processes

export const which = (binary) => {
  const names = IS_WINDOWS
    ? [binary, `${binary}.exe`, `${binary}.cmd`, `${binary}.bat`]
    : [binary];
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    if (!dir) continue;
    for (const name of names) {
      const candidate = path.join(dir, name);
      if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
    }
  }
  return null;
};

/** Run to completion, inheriting stdio unless the caller wants the output.
 *  Throws `PerfError` on a non-zero exit: a failed `cargo build` must never be
 *  mistaken for a produced profile. */
export function run(command, args, options = {}) {
  const { env, cwd = repoRoot, capture = false, allowFailure = false, quiet = false } = options;
  if (!quiet) detail(`$ ${command} ${summarizeArgs(args).join(" ")}`);
  const result = spawnSync(command, args, {
    cwd,
    env: { ...process.env, ...env },
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "pipe"] : "inherit",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) {
    if (allowFailure) return { ok: false, stdout: "", stderr: String(result.error) };
    fail(`could not run ${command}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    if (allowFailure) {
      return { ok: false, status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
    }
    const stderr = (result.stderr ?? "").trim();
    fail(
      `${command} ${args[0] ?? ""} exited with ${result.status}` +
        (stderr ? `\n${stderr.split("\n").slice(-20).join("\n")}` : ""),
    );
  }
  return { ok: true, status: 0, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

const quoteArg = (arg) => (/^[\w@%+=:,./-]+$/.test(arg) ? arg : JSON.stringify(arg));

/** `llvm-profdata merge` takes one argument per profraw; a real training run
 *  leaves hundreds, and printing all of them buries the actual command. */
const summarizeArgs = (args) => {
  if (args.length <= 10) return args.map(quoteArg);
  return [...args.slice(0, 6).map(quoteArg), `… (+${args.length - 6} more)`];
};

const firstLine = (text) => (text ?? "").split("\n").map((line) => line.trim()).find(Boolean) ?? "";

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const rustcVerbose = () => {
  const { stdout } = run("rustc", ["-vV"], { capture: true, quiet: true });
  return stdout;
};

/** The host triple is the one cargo compiles for by default, so it is also the
 *  table in `.cargo/config.toml` whose `rustflags` apply. */
export const hostTriple = () => {
  const match = /^host:\s*(\S+)$/m.exec(rustcVerbose());
  if (!match) fail("could not read the host triple from `rustc -vV`");
  return match[1];
};

export const rustcLlvmVersion = () => {
  const match = /^LLVM version:\s*(\S+)$/m.exec(rustcVerbose());
  return match ? match[1] : null;
};

// ---------------------------------------------------------------------------
// rustflags

/** The `rustflags` arrays cargo would use for `triple`, read out of the
 *  workspace's `.cargo/config.toml`.
 *
 *  The Qt workspace is the repository root, so its config (if one is ever
 *  added) lives in `.cargo/config.toml`; the Tauri package is excluded from
 *  that workspace and keeps its own under `src-tauri/`.
 *
 *  A hand-written parser is used instead of a TOML dependency because bun and
 *  node disagree about what is available (`Bun.TOML` exists; node has no TOML
 *  at all) and this file only ever holds arrays of strings. It fails loudly on
 *  anything it does not understand rather than returning `[]`, so a future edit
 *  to the config cannot silently drop the stack reserve. */
export function cargoConfigRustflags(triple, shell) {
  const configRoot = resolveShell(shell) === "qt" ? repoRoot : tauriDir;
  const file = path.join(configRoot, ".cargo", "config.toml");
  if (!existsSync(file)) return [];
  const text = readFileSync(file, "utf8");
  const sections = new Map();
  let current = "";

  for (const line of text.split("\n")) {
    const header = /^\s*\[([^\]]+)\]\s*(?:#.*)?$/.exec(line);
    if (header) {
      current = header[1].replace(/["']/g, "").replace(/\s+/g, "");
      if (!sections.has(current)) sections.set(current, []);
      continue;
    }
    if (current) sections.get(current).push(line);
  }

  const parseFlags = (name, body) => {
    const joined = body.join("\n");
    if (!/^\s*rustflags\s*=/m.test(joined)) return null;
    const array = /^\s*rustflags\s*=\s*\[([\s\S]*?)\]/m.exec(joined);
    if (!array) fail(`${path.relative(repoRoot, file)}: could not parse rustflags under [${name}]`);
    const flags = [...array[1].matchAll(/"([^"]*)"|'([^']*)'/g)].map((m) => m[1] ?? m[2]);
    if (!flags.length) fail(`${path.relative(repoRoot, file)}: rustflags under [${name}] is empty`);
    return flags;
  };

  const fromTarget = parseFlags(`target.${triple}`, sections.get(`target.${triple}`) ?? []);
  if (fromTarget) return fromTarget;

  for (const name of sections.keys()) {
    if (/^target\.cfg\(/.test(name) && sections.get(name).join("\n").includes("rustflags")) {
      warn(
        `[${name}] sets rustflags; cargo would apply it only when the cfg matches, ` +
          "and this script does not evaluate cfg — add the flags here if the build needs them",
      );
    }
  }

  return parseFlags("build", sections.get("build") ?? []) ?? [];
}

/** Environment for a build whose extra flags are `extra`.
 *
 *  Ambient flags win over the config exactly the way cargo resolves them,
 *  unless there are none — in which case the config's flags are carried over,
 *  which is the whole point. */
export function rustflagsEnv(extra, { shell } = {}) {
  const selected = resolveShell(shell);
  const triple = hostTriple();
  const ambientEncoded = process.env.CARGO_ENCODED_RUSTFLAGS;
  const ambient = process.env.RUSTFLAGS?.trim();

  let base;
  if (ambientEncoded) {
    base = ambientEncoded.split("\u001f").filter(Boolean);
    say("using CARGO_ENCODED_RUSTFLAGS from the environment as the base");
  } else if (ambient) {
    base = ambient.split(/\s+/).filter(Boolean);
    say("using RUSTFLAGS from the environment as the base");
  } else {
    base = cargoConfigRustflags(triple, selected);
    if (base.length) {
      const configRoot = selected === "qt" ? repoRoot : tauriDir;
      say(`carrying ${path.relative(repoRoot, path.join(configRoot, ".cargo", "config.toml"))} rustflags for ${triple}`);
    }
  }

  const flags = [...base, ...extra];
  say(`rustflags: ${flags.join(" ")}`);
  return { CARGO_ENCODED_RUSTFLAGS: flags.join("\u001f"), RUSTFLAGS: "" };
}

// ---------------------------------------------------------------------------
// LLVM tools

const TOOL_ENV = {
  "llvm-profdata": "DAE_LLVM_PROFDATA",
  "llvm-bolt": "DAE_LLVM_BOLT",
  "merge-fdata": "DAE_MERGE_FDATA",
  perf2bolt: "DAE_PERF2BOLT",
  "llvm-strip": "DAE_LLVM_STRIP",
};

const llvmVersionDirs = () => {
  if (!IS_LINUX) return [];
  const roots = ["/usr/lib", "/usr/local/lib"];
  const found = [];
  for (const root of roots) {
    if (!existsSync(root)) continue;
    for (const entry of readdirSync(root)) {
      const match = /^llvm-(\d+)$/.exec(entry);
      if (match) found.push({ version: Number(match[1]), dir: path.join(root, entry, "bin") });
    }
  }
  return found.sort((a, b) => b.version - a.version).map((entry) => entry.dir);
};

const versionedNames = (binary) => {
  if (!IS_LINUX) return [];
  const names = [];
  for (let major = 25; major >= 14; major -= 1) names.push(`${binary}-${major}`);
  return names;
};

/** Locate an LLVM tool, preferring the toolchain rustc itself uses. Returns
 *  `null` when the tool is missing and the caller can cope. */
export function findLlvmTool(binary, { required = true, hint = "" } = {}) {
  const searched = [];
  const consider = (candidate) => {
    if (!candidate) return null;
    searched.push(candidate);
    return existsSync(candidate) && statSync(candidate).isFile() ? candidate : null;
  };

  const explicit = process.env[TOOL_ENV[binary] ?? `DAE_${binary.toUpperCase().replace(/-/g, "_")}`];
  let hit = consider(explicit);
  if (hit) return hit;

  if (!process.env.DAE_LLVM_DIR) {
    const sysroot = run("rustc", ["--print", "sysroot"], { capture: true, quiet: true }).stdout.trim();
    hit = consider(path.join(sysroot, "lib", "rustlib", hostTriple(), "bin", IS_WINDOWS ? `${binary}.exe` : binary));
    if (hit) return hit;
  } else {
    hit = consider(path.join(process.env.DAE_LLVM_DIR, IS_WINDOWS ? `${binary}.exe` : binary));
    if (hit) return hit;
  }

  hit = which(binary);
  if (hit) return hit;

  for (const name of versionedNames(binary)) {
    hit = which(name);
    if (hit) return hit;
  }
  for (const dir of llvmVersionDirs()) {
    hit = consider(path.join(dir, binary));
    if (hit) return hit;
  }
  if (IS_MACOS) {
    for (const dir of ["/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"]) {
      hit = consider(path.join(dir, binary));
      if (hit) return hit;
    }
  }
  if (IS_WINDOWS) hit = consider(path.join("C:\\Program Files\\LLVM\\bin", `${binary}.exe`));
  if (hit) return hit;

  if (!required) return null;
  fail(
    `could not find ${binary}. Tried:\n` +
      [...new Set(searched)].slice(0, 12).map((entry) => `  ${entry}`).join("\n") +
      (hint ? `\n${hint}` : "") +
      `\nPoint ${TOOL_ENV[binary] ?? "the matching environment variable"} at it to override.`,
  );
}

export const toolVersion = (binary) => {
  const { stdout } = run(binary, ["--version"], { capture: true, quiet: true, allowFailure: true });
  const version = /\b(\d+\.\d+\.\d+)\b/.exec(stdout ?? "");
  return version ? version[1] : firstLine(stdout);
};

/** Warn — never fail — when the profile tool and rustc disagree about LLVM.
 *  profraw and profdata formats are versioned, so a mismatch is either a hard
 *  error at `-Cprofile-use` time or, worse, a silently ignored profile. */
export function warnOnLlvmMismatch(profdataBinary) {
  const rustcLlvm = rustcLlvmVersion();
  const tool = /(\d+)\.(\d+)\.(\d+)/.exec(toolVersion(profdataBinary));
  if (!rustcLlvm || !tool) return;
  const rustcMajor = Number(rustcLlvm.split(".")[0]);
  if (rustcMajor !== Number(tool[1])) {
    warn(
      `llvm-profdata is LLVM ${tool[1]}.${tool[2]}.${tool[3]} but rustc was built with LLVM ${rustcLlvm}; ` +
        "profiles written by this toolchain may not be readable",
    );
  }
}

/** The two properties BOLT needs and the release profile removes: emitted
 *  relocations, and an intact symbol table. */
export const boltLinkFlags = ["-C", "link-arg=-Wl,--emit-relocs"];

/**
 * Extra `-C` flags for a `-Cprofile-use` build.
 *
 * Empty on purpose. `-C llvm-args=-pgo-warn-missing-function` is the flag every
 * PGO guide suggests, and on this crate it emits **77,886 lines** of "no profile
 * data available for function …" — build scripts and untrained crates drown out
 * the handful of lines that would matter. A profile's health is checked with
 * `llvm-profdata show` after the merge instead (`pgo.mjs merge` prints it). Set
 * `DAE_PGO_WARN_MISSING=1` to get those warnings back while debugging.
 */
export const PROFILE_USE_ARGS = process.env.DAE_PGO_WARN_MISSING
  ? ["-C", "llvm-args=-pgo-warn-missing-function"]
  : [];

// ---------------------------------------------------------------------------
// build outputs

export const targetDir = (shell) => {
  const configured = process.env.CARGO_TARGET_DIR;
  const root = resolveShell(shell) === "qt" ? repoRoot : tauriDir;
  return configured ? path.resolve(repoRoot, configured) : path.join(root, "target");
};

/** The binary the selected shell produced. Hooks run by tauri get
 *  `TAURI_ENV_TARGET_TRIPLE`; a cross build puts the exe one directory deeper.
 *  The Qt workspace is compiled from the repository root, so its target
 *  directory and `dae` binary sit at the top level. */
export const appBinary = (profile = "release", shell) => {
  const selected = resolveShell(shell);
  const name = IS_WINDOWS ? "dae.exe" : "dae";
  const bare = path.join(targetDir(selected), profile, name);
  const triple =
    process.env.DAE_PERF_TARGET_TRIPLE ||
    (selected === "tauri" ? process.env.TAURI_ENV_TARGET_TRIPLE : "");
  if (!triple) return bare;
  const scoped = path.join(targetDir(selected), triple, profile, name);
  // The tauri CLI sets `TAURI_ENV_TARGET_TRIPLE` to the host triple even when
  // `tauri build` ran without `--target`, so that variable is a hint about the
  // platform, not a fact about the output directory: a host build still lands
  // in `target/release/`. Trusting it unconditionally is what made the Linux
  // `beforeBundleCommand` look for `target/x86_64-unknown-linux-gnu/release/dae`
  // while the build had written `target/release/dae` — post-link then warned
  // "nothing to post-process" and skipped BOLT on a release that asked for it.
  // Prefer the triple directory only when a binary is actually in it.
  return existsSync(scoped) || !existsSync(bare) ? scoped : bare;
};

/** `tauri build` with the given environment. `--no-bundle` is the default here
 *  because a profiled build is an intermediate artifact: bundling it would
 *  build installers around a binary that is about to be rebuilt.
 *
 *  `DAE_TAURI_BUILD_ARGS` defaults to the release overlay, and that default is
 *  load-bearing: the overlay is what supplies `plugins.updater`, and a release
 *  build compiled without it panics on startup (`PluginInitialization("updater",
 *  "invalid type: null")`). A profiled build that cannot start produces no
 *  profile, so the overlay is not optional here. */
export function tauriBuild({ args = [], env = {}, bundle = false } = {}) {
  const bun = which("bun");
  if (!bun) fail("bun is not on PATH — every build in this repo goes through it");
  const configured = (process.env.DAE_TAURI_BUILD_ARGS ?? "--config src-tauri/tauri.release.conf.json").trim();
  const extraArgs = configured ? configured.split(/\s+/) : [];
  return run(bun, ["tauri", "build", ...(bundle ? [] : ["--no-bundle"]), ...extraArgs, ...args], { env });
}

/** `cargo build -p dae-app` with the given environment. No bundler and no
 *  frontend step: the Qt shell's QML is compiled into the binary by its build
 *  script, so this is the same artifact `release.yml` packages. */
export function qtBuild({ args = [], env = {} } = {}) {
  const cargo = which("cargo");
  if (!cargo) fail("cargo is not on PATH");
  return run(cargo, ["build", "--release", "--package", "dae-app", ...args], { env });
}

/** Build whichever shell the scripts were told to profile. `--qt` selects
 *  `cargo build -p dae-app`; otherwise the historical `tauri build` path. */
export function buildShell({ shell, args = [], env = {}, bundle = false } = {}) {
  return resolveShell(shell) === "qt"
    ? qtBuild({ args, env })
    : tauriBuild({ args, env, bundle });
}

export const isElf = (file) => {
  if (!existsSync(file)) return false;
  // Read four bytes, not the whole binary: this runs on a 100MB+ exe.
  let fd;
  try {
    fd = openSync(file, "r");
    const head = Buffer.alloc(4);
    readSync(fd, head, 0, 4, 0);
    return head.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]));
  } finally {
    if (fd !== undefined) closeSync(fd);
  }
};

// ---------------------------------------------------------------------------
// training

/** A directory worth listing: several directories, a few hundred names, a mix
 *  of extensions. Enough for the listing pipeline and the icon path to do real
 *  work, small enough that creating it costs nothing. */
export function prepareCorpus(count = 320) {
  const dirs = ["src", "src/features/explorer", "docs", "bench", "assets/icons", "node_modules"];
  for (const dir of dirs) mkdirSync(path.join(corpusDir, dir), { recursive: true });
  const extensions = [".ts", ".tsx", ".rs", ".md", ".json", ".png"];
  for (let index = 0; index < count; index += 1) {
    const dir = dirs[index % dirs.length];
    const extension = extensions[index % extensions.length];
    const file = path.join(corpusDir, dir, `entry-${String(index).padStart(4, "0")}${extension}`);
    if (!existsSync(file)) writeFileSync(file, `training corpus file ${index}\n`);
  }
  return corpusDir;
}

const clearDirectory = (dir) => {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
};

export const resetProfileDirectory = (dir) => {
  clearDirectory(dir);
  return dir;
};

/** Files with one of `extensions`, searched recursively. */
export function listFiles(dir, extensions) {
  if (!existsSync(dir)) return [];
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) found.push(...listFiles(full, extensions));
    else if (extensions.some((extension) => entry.name.endsWith(extension))) found.push(full);
  }
  return found.sort();
}

const TRAINING_ENV = {
  // WebKitGTK in a headless job: no dmabuf renderer, no compositing, software GL.
  WEBKIT_DISABLE_DMABUF_RENDERER: "1",
  WEBKIT_DISABLE_COMPOSITING_MODE: "1",
  LIBGL_ALWAYS_SOFTWARE: "1",
  NO_AT_BRIDGE: "1",
};

/**
 * Launch the app on the corpus, let it settle, then let it *quit itself*.
 *
 * `DAE_PERF_TRAIN_QUIT_MS` (see `spawn_training_quit` in `src-tauri/src/lib.rs`)
 * makes the app call `AppHandle::exit` after the given delay, which is a clean
 * shutdown: `atexit` and `.fini_array` hooks run, and that is where both profile
 * runtimes write their counters. Killing the process instead was tried and does
 * not work — compiler-rt's continuous mode (`LLVM_PROFILE_FILE=…%c`) leaves a
 * truncated profraw that `llvm-profdata` refuses, and BOLT's runtime only dumps
 * on its timer, which misses everything after the last tick.
 *
 * The kill path below stays as a backstop for a build without the hook (an
 * older binary, or a platform where the app refuses to quit).
 */
export async function runTrainingWorkload({ exe, seconds = 45, env = {}, directory, shell } = {}) {
  if (!existsSync(exe)) fail(`no binary at ${exe} — build it first`);
  const selected = resolveShell(shell);
  const corpus = directory ?? prepareCorpus();
  const binaryArgs = [corpus];

  let command = exe;
  let commandArgs = binaryArgs;
  if (IS_LINUX && !process.env.DISPLAY) {
    const xvfb = which("xvfb-run");
    if (xvfb) {
      say(`no DISPLAY: running under ${xvfb}`);
      command = xvfb;
      commandArgs = ["-a", "-s", "-screen 0 1440x900x24", exe, ...binaryArgs];
    } else {
      warn("no DISPLAY and no xvfb-run: the app will probably fail to start, and the profile will be thin");
    }
  }

  // Leave a second of slack so the app is up and painted before the timer ends.
  const quitAfterMs = Math.max(1000, (seconds - 1) * 1000);
  say(`training: ${path.basename(exe)} on ${corpus} — app quits itself after ${Math.round(quitAfterMs / 1000)}s`);

  // The app's own output is the only clue when it dies before the timer — a
  // panic in a `windows_subsystem = "windows"` build is otherwise completely
  // silent. Keep it, and show the tail when the run ends early.
  mkdirSync(perfDir, { recursive: true });
  const logFile = path.join(perfDir, "training.log");
  const logFd = openSync(logFile, "w");
  const childEnv = { ...process.env, ...TRAINING_ENV, DAE_PERF_TRAIN_QUIT_MS: String(quitAfterMs), ...env };
  // Qt refuses to start without a platform plugin when there is no display and
  // no xvfb-run; offscreen still executes the startup path the profile needs.
  if (selected === "qt" && IS_LINUX && !process.env.DISPLAY) {
    childEnv.QT_QPA_PLATFORM ??= "offscreen";
  }
  // An inherited profile path (a shell that exported it, or a previous run)
  // would send the counters somewhere this script does not read.
  if (!("LLVM_PROFILE_FILE" in env)) delete childEnv.LLVM_PROFILE_FILE;

  const child = spawn(command, commandArgs, {
    cwd: repoRoot,
    env: childEnv,
    stdio: ["ignore", "ignore", logFd],
    detached: IS_LINUX,
  });
  const output = () => {
    const text = existsSync(logFile) ? readFileSync(logFile, "utf8").trim() : "";
    return text ? `\n${text.split("\n").slice(-12).join("\n")}` : "";
  };

  const exited = new Promise((resolve) => child.once("exit", (code, signal) => resolve({ code, signal })));
  // Wait out the training window plus a shutdown grace period: WebKit teardown
  // and the profile flush both happen after `exit(0)` is requested.
  const early = await Promise.race([exited, sleep((seconds + 20) * 1000).then(() => null)]);

  if (early) {
    const clean = early.code === 0;
    if (clean) say(`training run finished (code 0)`);
    else {
      warn(
        `the training run exited with code ${early.code}${early.signal ? `, signal ${early.signal}` : ""} ` +
          `before its timer — the profile covers only what it reached${output()}`,
      );
    }
  } else {
    // Backstop: the app never asked to go, so nothing has been written yet.
    // The tail is printed here as well as above: whether the hook armed at all
    // is the difference between "the timer never started" and "it fired and the
    // exit did not finish", and that decision belongs to whoever reads this.
    warn(
      `the app did not quit on its own within ${seconds + 20}s; killing it ` +
        `(the profile will be incomplete)${output()}`,
    );
    signal(child, "SIGTERM");
    const stopped = await Promise.race([exited, sleep(4000).then(() => null)]);
    if (!stopped) {
      signal(child, "SIGKILL");
      await Promise.race([exited, sleep(4000)]);
    }
  }

  closeSync(logFd);
  return corpus;
}

const signal = (child, name) => {
  if (child.pid === undefined) return;
  try {
    if (IS_LINUX) process.kill(-child.pid, name);
    else child.kill(name === "SIGKILL" ? "SIGKILL" : "SIGTERM");
  } catch {
    // already gone, or the group no longer exists
  }
};

// ---------------------------------------------------------------------------
// CLI entry

export const isMain = (url) => {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    return path.resolve(entry) === path.resolve(fileURLToPath(url));
  } catch {
    return false;
  }
};

/** Wrap a CLI body: `PerfError` prints its message and exits 1, anything else
 *  keeps its stack because it is a bug in this script, not in the build. */
export const runCli = (body) => {
  body().catch((error) => {
    if (error instanceof PerfError) {
      console.error(`\n${red("✖")} ${error.message}`);
      process.exit(1);
    }
    console.error(error);
    process.exit(1);
  });
};
