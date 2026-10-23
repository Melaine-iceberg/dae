// LLVM BOLT post-link optimisation for the release binary.
//
// BOLT rewrites a *linked* ELF binary: it reorders basic blocks and functions
// using a profile of the final layout, which is something neither rustc nor PGO
// can do (PGO is decided while the machine code is being generated; BOLT sees
// where the linker and the inliner actually put everything).
//
//   bun run perf:pgo:build --bolt   # link with --emit-relocs, keep symbols
//   bun run perf:bolt               # instrument, replay a workload, rewrite
//
// Two hard requirements, both enforced below with an explanation instead of a
// cryptic BOLT error:
//   * the binary must be ELF — BOLT does not read PE/COFF or Mach-O, so on
//     Windows and macOS this script can only report that it has nothing to do;
//   * the binary must keep its symbol table and, for function reordering, its
//     relocations (`-Wl,--emit-relocs`), both of which the release profile
//     strips by default.
//
// `applyBolt` is also the body of the `beforeBundleCommand` hook
// (`src-tauri/tauri.perf.conf.json`), which is how the release workflow gets a
// BOLT'ed binary into the installers: it runs after `cargo build` and before
// the bundles are assembled from `target/release/dae`.

import path from "node:path";
import { chmodSync, copyFileSync, mkdirSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import {
  appBinary,
  boltDataPath,
  boltDir,
  fail,
  fileSize,
  findLlvmTool,
  humanSize,
  isElf,
  isMain,
  listFiles,
  repoRoot,
  run,
  runCli,
  runTrainingWorkload,
  say,
  step,
  targetDir,
  toolVersion,
  warn,
  which,
} from "./lib.mjs";

const USAGE = `LLVM BOLT for dae's release binary (ELF only).

  bun scripts/opt/bolt.mjs instrument [--exe PATH]   write a BOLT-instrumented copy
  bun scripts/opt/bolt.mjs collect [--seconds N]     run the instrumented copy
  bun scripts/opt/bolt.mjs merge                     merge-fdata the raw dumps
  bun scripts/opt/bolt.mjs optimize [--exe PATH]     rewrite the binary with the profile
  bun scripts/opt/bolt.mjs all [--seconds N]         instrument, collect, merge, optimize, strip
  bun scripts/opt/bolt.mjs status                    tools, relocation check, artifact sizes

Options
  --exe PATH     binary to optimise (default: the release build in src-tauri/target)
  --seconds N    length of the training run (default 45)
  --in-place     \`all\` replaces the binary it was given (default: keep both)
`;

const RAW_DUMP_DIR = path.join(boltDir, "raw");
const INSTRUMENTED = path.join(boltDir, "dae.instr");
const OPTIMIZED = path.join(boltDir, "dae.bolt");

const INSTRUMENT_ARGS = (dumpFile) => [
  "-instrument",
  `-instrumentation-file=${dumpFile}`,
  // A GUI app is killed rather than closed, so the runtime has to dump on a
  // timer; counters are kept across dumps so the last file covers the whole run.
  "-instrumentation-sleep-time=5",
  "-instrumentation-no-counters-clear",
];

const OPTIMIZE_ARGS = [
  "-reorder-blocks=ext-tsp",
  "-reorder-functions=hfsort+",
  "-split-functions",
  "-split-all-cold",
  "-split-eh",
  "-icf=1",
  "-use-gnu-stack",
];

// Packaged Rust binaries without exception tables and binaries without
// relocations trip the more aggressive passes; the conservative set still gets
// the block/function reordering that carries most of the win.
const OPTIMIZE_ARGS_FALLBACK = [
  "-reorder-blocks=ext-tsp",
  "-reorder-functions=hfsort+",
  "-icf=1",
  "-use-gnu-stack",
];

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const flags = { exe: null, seconds: 45, inPlace: false };
  for (let index = 0; index < rest.length; index += 1) {
    const arg = rest[index];
    if (arg === "--exe") flags.exe = rest[(index += 1)];
    else if (arg.startsWith("--exe=")) flags.exe = arg.slice("--exe=".length);
    else if (arg === "--seconds") flags.seconds = Number(rest[(index += 1)]);
    else if (arg.startsWith("--seconds=")) flags.seconds = Number(arg.slice("--seconds=".length));
    else if (arg === "--in-place") flags.inPlace = true;
    else if (arg === "--help" || arg === "-h") {
      console.error(USAGE);
      process.exit(0);
    } else fail(`unknown argument ${arg}\n\n${USAGE}`);
  }
  return { command, flags };
}

const resolveExe = (requested) => path.resolve(requested ?? appBinary());

/** `llvm-bolt` and `merge-fdata` ship together in distro packages
 *  (`llvm-bolt` on Ubuntu 25.04+, `bolt-NN` in the LLVM apt repo). */
const boltTools = () => {
  const bolt = findLlvmTool("llvm-bolt", { required: false });
  const merge = findLlvmTool("merge-fdata", { required: false });
  return bolt && merge ? { bolt, merge } : null;
};

const readelf = () =>
  findLlvmTool("llvm-readelf", { required: false }) ?? which("readelf") ?? which("llvm-readelf");

/** Whether the binary carries what BOLT needs: a symbol table, and relocations
 *  for the function-reordering passes. `null` means "could not tell". */
export function inspectBinary(exe) {
  if (!isElf(exe)) return { elf: false, symtab: null, relocations: null };
  const tool = readelf();
  if (!tool) return { elf: true, symtab: null, relocations: null };
  const sections = run(tool, ["--section-headers", exe], { capture: true, quiet: true, allowFailure: true });
  const text = sections.stdout ?? "";
  return {
    elf: true,
    symtab: /\.symtab\b/.test(text),
    relocations: /\.rela?\.text\b/.test(text),
  };
}

/** The parts of the BOLT pipeline, each callable on its own so a failure can be
 *  reported against the step that actually failed. */
function instrument(exe) {
  const tools = boltTools();
  if (!tools) fail("llvm-bolt is not installed — see docs/pgo-bolt.md#安装");
  mkdirSync(RAW_DUMP_DIR, { recursive: true });
  rmSync(INSTRUMENTED, { force: true });
  step("llvm-bolt -instrument");
  say(`in:  ${path.relative(repoRoot, exe)}`);
  say(`out: ${path.relative(repoRoot, INSTRUMENTED)}`);
  run(tools.bolt, [exe, "-o", INSTRUMENTED, ...INSTRUMENT_ARGS(path.join(RAW_DUMP_DIR, "dae.fdata"))]);
  return INSTRUMENTED;
}

function mergeData() {
  const tools = boltTools();
  if (!tools) fail("merge-fdata is not installed — see docs/pgo-bolt.md#安装");
  const dumps = listFiles(RAW_DUMP_DIR, [".fdata"]);
  if (!dumps.length) {
    fail(
      `no .fdata dumps in ${path.relative(repoRoot, RAW_DUMP_DIR)}.\n` +
        "Run `bun run perf:bolt collect` (or start the instrumented copy yourself) first.",
    );
  }
  step("merge-fdata");
  const merged = run(tools.merge, dumps, { capture: true, quiet: true });
  mkdirSync(boltDir, { recursive: true });
  // merge-fdata writes to stdout; an empty result would be a profile that BOLT
  // then treats as "no data", so refuse it here where the reason is still known.
  if (!merged.stdout.trim()) fail("merge-fdata produced no data — the instrumented run was too short?");
  writeFileSync(boltDataPath, merged.stdout);
  say(`${path.relative(repoRoot, boltDataPath)} — ${humanSize(fileSize(boltDataPath))} from ${dumps.length} dump(s)`);
  return boltDataPath;
}

function optimize(exe, { tools = boltTools() } = {}) {
  if (!tools) fail("llvm-bolt is not installed — see docs/pgo-bolt.md#安装");
  if (!fileSize(boltDataPath)) fail(`no BOLT profile at ${path.relative(repoRoot, boltDataPath)} — run the merge step first`);
  step("llvm-bolt (optimize)");
  const attempt = (args) =>
    run(tools.bolt, [exe, "-o", OPTIMIZED, `-data=${boltDataPath}`, "-dyno-stats", ...args], {
      allowFailure: true,
    });
  let result = attempt(OPTIMIZE_ARGS);
  if (!result.ok) {
    warn("the full pass set failed; retrying with the conservative set (see the BOLT output above)");
    result = attempt(OPTIMIZE_ARGS_FALLBACK);
  }
  if (!result.ok || !fileSize(OPTIMIZED)) {
    fail("llvm-bolt produced no output — the binary probably needs `-Wl,--emit-relocs` and an unstripped symbol table");
  }
  say(`${path.relative(repoRoot, OPTIMIZED)} — ${humanSize(fileSize(OPTIMIZED))}`);
  return OPTIMIZED;
}

/** Strip the symbols and relocations BOLT needed but the shipped binary does
 *  not: they are dead weight, and the release profile normally removes them. */
function strip(file) {
  const tool = findLlvmTool("llvm-strip", { required: false }) ?? which("strip");
  if (!tool) {
    warn("no llvm-strip or strip on PATH: the optimized binary keeps its symbol table");
    return;
  }
  step("strip");
  run(tool, ["--strip-all", file], { allowFailure: true });
  say(`${path.relative(repoRoot, file)} — ${humanSize(fileSize(file))}`);
}

const publish = (optimized, exe) => {
  mkdirSync(path.dirname(exe), { recursive: true });
  const mode = statSync(optimized).mode & 0o777;
  // Same volume (both under the repo), so this is a metadata-only replace and
  // cannot half-write a binary that the bundler is about to read.
  try {
    renameSync(optimized, exe);
  } catch {
    copyFileSync(optimized, exe);
    rmSync(optimized, { force: true });
  }
  chmodSync(exe, mode);
  say(`installed: ${path.relative(repoRoot, exe)} — ${humanSize(fileSize(exe))}`);
};

/**
 * The whole pipeline, with every "cannot" reported as a skip rather than a
 * failure: it runs inside `tauri build`, and a release that loses its BOLT pass
 * must still ship.
 *
 * @returns {{applied: boolean, reason?: string}}
 */
export async function applyBolt({ exe = appBinary(), seconds = 45 } = {}) {
  if (!isElf(exe)) return { applied: false, reason: "not an ELF binary" };
  const tools = boltTools();
  if (!tools) return { applied: false, reason: "llvm-bolt / merge-fdata not installed" };
  const inspection = inspectBinary(exe);
  if (inspection.symtab === false) return { applied: false, reason: "the binary has no symbol table (built with strip = true?)" };
  if (inspection.relocations === false) {
    return { applied: false, reason: "the binary has no .rela.text (built without -Wl,--emit-relocs?)" };
  }

  rmSync(RAW_DUMP_DIR, { recursive: true, force: true });
  instrument(exe);
  await runTrainingWorkload({ exe: INSTRUMENTED, seconds });
  mergeData();
  optimize(exe, { tools });
  strip(OPTIMIZED);
  publish(OPTIMIZED, exe);
  return { applied: true };
}

async function all({ exe, seconds, inPlace }) {
  const target = resolveExe(exe);
  step(`BOLT ${path.relative(repoRoot, target)}`);
  if (!isElf(target)) {
    fail(
      `${path.relative(repoRoot, target)} is not an ELF binary.\n` +
        "LLVM BOLT only rewrites ELF (Linux) executables; on Windows and macOS there " +
        "is no equivalent post-link step, and the release workflow skips it there too.",
    );
  }
  const inspection = inspectBinary(target);
  if (inspection.symtab === false) fail("the binary is stripped — build with CARGO_PROFILE_RELEASE_STRIP=false");
  if (inspection.relocations === false) {
    fail(
      "the binary has no .rela.text section.\n" +
        "Rebuild with `bun run perf:pgo:build --bolt`, which adds -Wl,--emit-relocs and " +
        "keeps the symbol table. Without relocations BOLT cannot reorder functions.",
    );
  }

  rmSync(RAW_DUMP_DIR, { recursive: true, force: true });
  instrument(target);
  await runTrainingWorkload({ exe: INSTRUMENTED, seconds });
  mergeData();
  optimize(target);
  strip(OPTIMIZED);
  if (inPlace) publish(OPTIMIZED, target);
  else {
    say(`kept both: ${path.relative(repoRoot, target)} (untouched) and ${path.relative(repoRoot, OPTIMIZED)}`);
    say("rerun with --in-place to replace the binary, or copy .perf/bolt/dae.bolt over it yourself");
  }
}

function status({ exe }) {
  const target = resolveExe(exe);
  step("tools");
  for (const tool of ["llvm-bolt", "merge-fdata", "perf2bolt"]) {
    const found = findLlvmTool(tool, { required: false });
    say(`${tool.padEnd(11)} ${found ? `${found} (${toolVersion(found)})` : "not found"}`);
  }
  step("binary");
  say(`${path.relative(repoRoot, target)} — ${fileSize(target) ? humanSize(fileSize(target)) : "missing"}`);
  const inspection = inspectBinary(target);
  if (!inspection.elf) {
    say("not ELF — BOLT cannot rewrite this platform's binaries");
  } else {
    say(`symbol table: ${inspection.symtab === null ? "unknown" : inspection.symtab ? "present" : "stripped"}`);
    say(`relocations:  ${inspection.relocations === null ? "unknown" : inspection.relocations ? ".rela.text present" : "missing (link with -Wl,--emit-relocs)"}`);
  }
  step("artifacts");
  say(`instrumented: ${fileSize(INSTRUMENTED) ? humanSize(fileSize(INSTRUMENTED)) : "missing"}`);
  say(`profile:      ${fileSize(boltDataPath) ? humanSize(fileSize(boltDataPath)) : "missing"}`);
  say(`optimized:    ${fileSize(OPTIMIZED) ? humanSize(fileSize(OPTIMIZED)) : "missing"}`);
  say(`target dir:   ${path.relative(repoRoot, targetDir())}`);
}

if (isMain(import.meta.url)) runCli(async () => {
  // The CLI body. Guarded so `post-link.mjs` can import `applyBolt` without
  // running this entry point.
  const { command, flags } = parseArgs(process.argv.slice(2));
  switch (command) {
    case "instrument":
      return instrument(resolveExe(flags.exe));
    case "collect": {
      if (!fileSize(INSTRUMENTED)) fail("nothing to run — `bun run perf:bolt instrument` first");
      return runTrainingWorkload({ exe: INSTRUMENTED, seconds: flags.seconds });
    }
    case "merge":
      return mergeData();
    case "optimize":
      return optimize(resolveExe(flags.exe));
    case "all":
      return all(flags);
    case "status":
      return status(flags);
    default:
      console.error(USAGE);
      process.exit(command ? 1 : 0);
  }
});
