// Staged LLVM IR PGO for the app (`docs/pgo-bolt.md`).
//
//   bun run perf:pgo:instrument   # build with -Cprofile-generate
//   … use the app for a few minutes …
//   bun run perf:pgo:merge        # llvm-profdata merge -> .perf/pgo/dae.profdata
//   bun run perf:pgo:build        # build with -Cprofile-use
//
// The split is deliberate: what the profile says is entirely a function of how
// the instrumented build was used, so the training run belongs to whoever knows
// the hot paths. `train` exists for the one case where there is nobody to drive
// the app — the release workflow — and it only replays a scripted workload.

import path from "node:path";
import { existsSync, rmSync } from "node:fs";
import {
  appBinary,
  boltLinkFlags,
  corpusDir,
  fail,
  fileSize,
  findLlvmTool,
  humanSize,
  IS_LINUX,
  isElf,
  isMain,
  listFiles,
  PROFILE_USE_ARGS,
  pgoProfilePath,
  pgoRawDir,
  perfDir,
  prepareCorpus,
  repoRoot,
  resetProfileDirectory,
  run,
  runCli,
  runTrainingWorkload,
  rustcLlvmVersion,
  rustflagsEnv,
  say,
  step,
  tauriBuild,
  toolVersion,
  warn,
  warnOnLlvmMismatch,
} from "./lib.mjs";

const USAGE = `LLVM IR PGO for dae.

  bun scripts/opt/pgo.mjs instrument [--bolt]   build with -Cprofile-generate
  bun scripts/opt/pgo.mjs train [--seconds N]   instrument, replay a workload, merge (CI)
  bun scripts/opt/pgo.mjs merge                 llvm-profdata merge the raw counters
  bun scripts/opt/pgo.mjs build [--bolt]        build with -Cprofile-use
  bun scripts/opt/pgo.mjs flags [--bolt]        print $GITHUB_ENV lines for a CI build
  bun scripts/opt/pgo.mjs status                tool versions and artifact sizes
  bun scripts/opt/pgo.mjs clean                 remove ${path.relative(repoRoot, perfDir) || ".perf"}/

Options
  --bolt            also link with --emit-relocs and keep symbols, i.e. the
                    input LLVM BOLT needs (see scripts/opt/bolt.mjs). Linux only:
                    the flag is a GNU ld flag and BOLT only rewrites ELF.
  --seconds N       training run length for \`train\` (default 45)
  --target TRIPLE   cross build, as in \`tauri build --target\`; also where the
                    produced binary is looked for (macOS aarch64 on CI)
  --bundle          let \`build\` produce installers as well (slow; rarely wanted)
`;

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const flags = { bolt: false, bundle: false, seconds: 45, target: "" };
  for (let index = 0; index < rest.length; index += 1) {
    const arg = rest[index];
    if (arg === "--bolt") flags.bolt = true;
    else if (arg === "--bundle") flags.bundle = true;
    else if (arg === "--seconds") flags.seconds = Number(rest[(index += 1)]);
    else if (arg.startsWith("--seconds=")) flags.seconds = Number(arg.slice("--seconds=".length));
    else if (arg === "--target") flags.target = rest[(index += 1)] ?? "";
    else if (arg.startsWith("--target=")) flags.target = arg.slice("--target=".length);
    else if (arg === "--help" || arg === "-h") {
      console.error(USAGE);
      process.exit(0);
    } else fail(`unknown argument ${arg}\n\n${USAGE}`);
  }
  // `appBinary()` resolves the target sub-directory from this variable, so a
  // cross build looks in `target/<triple>/release/` instead of the host's.
  if (flags.target) process.env.DAE_PERF_TARGET_TRIPLE = flags.target;
  return { command, flags };
}

const targetArgs = (target) => (target ? ["--target", target] : []);

const profdataTool = () => {  const tool = findLlvmTool("llvm-profdata", {
    hint: "Install it with `rustup component add llvm-tools-preview` — rustc writes profraw files that a mismatched LLVM may not read.",
  });
  warnOnLlvmMismatch(tool);
  return tool;
};

const rawProfiles = () => listFiles(pgoRawDir, [".profraw"]);

/** `beforeBuildCommand` runs `bindings:export`, which builds and *runs* another
 *  instrumented binary before the app itself exists. Those counters describe the
 *  build tooling, not the app, so they are thrown away as soon as the build is
 *  done: whatever is in the directory afterwards was produced by the training
 *  run and nothing else. */
function dropBuildTimeProfiles() {
  const stale = rawProfiles();
  if (stale.length) say(`dropping ${stale.length} profraw file(s) written by the bindings export`);
  resetProfileDirectory(pgoRawDir);
}

const instrumentFlags = () => ["-C", `profile-generate=${pgoRawDir}`];

/** The instrumented build is the only one that links compiler-rt's profiling
 *  runtime, and `perf-train` is what lets `lib.rs::spawn_training_quit` call
 *  `__llvm_profile_write_file` — flushing the counters itself instead of
 *  trusting the exit path to finish. Any other build with this feature fails to
 *  link, so it cannot be switched on by accident. */
const TRAIN_FEATURE_ARGS = ["--features", "perf-train"];

const useFlags = () => ["-C", `profile-use=${pgoProfilePath}`, ...PROFILE_USE_ARGS];

function describeArtifacts() {
  const raw = rawProfiles();
  const exe = appBinary();
  say(`profiles: ${path.relative(repoRoot, pgoRawDir)} (${raw.length} profraw, ${humanSize(raw.reduce((sum, file) => sum + fileSize(file), 0))})`);
  say(`merged:   ${path.relative(repoRoot, pgoProfilePath)} (${existsSync(pgoProfilePath) ? humanSize(fileSize(pgoProfilePath)) : "missing"})`);
  say(`binary:   ${path.relative(repoRoot, exe)} (${existsSync(exe) ? humanSize(fileSize(exe)) : "missing"})`);
}

const warnBoltPlatform = (bolt) => {
  if (bolt && !IS_LINUX) {
    warn(
      "--bolt adds `-Wl,--emit-relocs`, which only GNU ld–style linkers accept, and LLVM BOLT " +
        "only rewrites ELF binaries — this is a Linux-only flag",
    );
  }
};

function instrument({ bolt, target }) {
  step("instrumented build (-Cprofile-generate)");
  warnBoltPlatform(bolt);
  resetProfileDirectory(pgoRawDir);
  profdataTool();
  say(`profile directory emptied: ${path.relative(repoRoot, pgoRawDir)}`);
  tauriBuild({ env: rustflagsEnv(instrumentFlags()), args: [...targetArgs(target), ...TRAIN_FEATURE_ARGS] });
  dropBuildTimeProfiles();

  const exe = appBinary();
  console.log(
    [
      "",
      "Now train the binary that was just built:",
      "",
      `  ${exe}`,
      "",
      "Open a few real directories (the ones that feel slow), run a search, open a",
      "terminal, switch views — a few minutes of ordinary use is what the profile",
      "should describe. Then close it and run:",
      "",
      "  bun run perf:pgo:merge",
      "",
      bolt
        ? "The merge is followed by `bun run perf:pgo:build --bolt`, which keeps symbols\nand relocations so scripts/opt/bolt.mjs can run LLVM BOLT afterwards."
        : "The merge is followed by `bun run perf:pgo:build`.",
    ]
      .filter(Boolean)
      .join("\n"),
  );
}

function merge() {
  step("llvm-profdata merge");
  const raw = rawProfiles();
  if (!raw.length) {
    fail(
      `no .profraw files in ${path.relative(repoRoot, pgoRawDir)}.\n` +
        "Build with `bun run perf:pgo:instrument`, run that binary, then merge. " +
        "If you did run it: rustc writes the counters next to the exe only when " +
        "`LLVM_PROFILE_FILE` is unset, so an inherited value in your shell would " +
        "have sent them elsewhere.",
    );
  }
  const tool = profdataTool();
  run(tool, ["merge", "-o", pgoProfilePath, ...raw]);
  step("merged");
  say(`${path.relative(repoRoot, pgoProfilePath)} — ${humanSize(fileSize(pgoProfilePath))} from ${raw.length} profraw file(s)`);
  // The only honest signal that the training run covered anything: a profile
  // with data in it reports tens of thousands of functions and a non-zero total
  // count. An empty one reports zero, and `-Cprofile-use` would then do nothing
  // at all while still looking like a successful build.
  const summary = run(tool, ["show", pgoProfilePath], { capture: true, quiet: true, allowFailure: true });
  for (const line of (summary.stdout ?? "").split("\n")) {
    if (/^(Total functions|Total count|Maximum function count|Total number of blocks):/.test(line.trim())) {
      say(line.trim());
    }
  }
  say("next: bun run perf:pgo:build");
}

function build({ bolt, bundle, target }) {
  step(`profile-use build${bolt ? " + BOLT link flags" : ""}`);
  warnBoltPlatform(bolt);
  if (!existsSync(pgoProfilePath)) {
    fail(
      `no merged profile at ${path.relative(repoRoot, pgoProfilePath)}.\n` +
        "Run `bun run perf:pgo:instrument`, use the app, then `bun run perf:pgo:merge`.",
    );
  }
  const env = rustflagsEnv([...(bolt ? boltLinkFlags : []), ...useFlags()]);
  // The release profile strips, which removes exactly what BOLT reads. The
  // post-link step strips the binary again once BOLT is done with it.
  if (bolt) env.CARGO_PROFILE_RELEASE_STRIP = "false";
  tauriBuild({ env, bundle, args: targetArgs(target) });

  const exe = appBinary();
  step("built");
  describeArtifacts();
  if (bolt) {
    say("next: bun run perf:bolt   (Linux only — BOLT rewrites ELF, not PE or Mach-O)");
  }
}

function flags({ bolt }) {
  // stdout is appended to $GITHUB_ENV by the workflow, so it must contain
  // nothing but assignments; the diagnostics from rustflagsEnv go to stderr.
  warnBoltPlatform(bolt);
  const extra = [...(bolt ? boltLinkFlags : []), ...(existsSync(pgoProfilePath) ? useFlags() : [])];
  if (!existsSync(pgoProfilePath)) {
    warn(
      `no merged profile at ${path.relative(repoRoot, pgoProfilePath)}: ` +
        "this build gets the link flags but no profile, so it is not PGO",
    );
  }
  const env = rustflagsEnv(extra);
  const lines = [
    `CARGO_ENCODED_RUSTFLAGS=${env.CARGO_ENCODED_RUSTFLAGS}`,
    ...(bolt ? ["CARGO_PROFILE_RELEASE_STRIP=false"] : []),
  ];
  process.stdout.write(`${lines.join("\n")}\n`);
}

async function train({ seconds, target }) {
  step(`PGO training run (${seconds}s)`);
  if (!Number.isFinite(seconds) || seconds <= 0) fail("--seconds must be a positive number");
  resetProfileDirectory(pgoRawDir);
  profdataTool();
  prepareCorpus();
  tauriBuild({ env: rustflagsEnv(instrumentFlags()), args: [...targetArgs(target), ...TRAIN_FEATURE_ARGS] });
  dropBuildTimeProfiles();

  const exe = appBinary();
  if (!existsSync(exe)) fail(`the instrumented build produced no binary at ${exe}`);
  // No `LLVM_PROFILE_FILE` here: rustc already told the runtime to write into
  // `pgoRawDir`, and the app quits cleanly (DAE_PERF_TRAIN_QUIT_MS), so the
  // default per-process naming is what we want.
  await runTrainingWorkload({ exe, seconds, directory: corpusDir });

  const raw = rawProfiles();
  if (!raw.length) {
    fail(
      `the training run wrote no .profraw files into ${path.relative(repoRoot, pgoRawDir)}.\n` +
        "The app most likely never started (no display, or a missing WebKit/WebView " +
        "dependency), so there is nothing to merge.\n" +
        `Read ${path.relative(repoRoot, path.join(perfDir, "training.log"))}: ` +
        "`[perf] training quit armed` means the hook ran, `profile counters flushed` " +
        "means the counters reached the disk. Neither line means the app never got as " +
        "far as its setup hook.",
    );
  }
  merge();
}

function status() {
  step("toolchain");
  const rustcLlvm = rustcLlvmVersion();
  say(`rustc LLVM ${rustcLlvm ?? "unknown"}`);
  for (const tool of ["llvm-profdata", "llvm-bolt", "merge-fdata", "perf2bolt"]) {
    const found = findLlvmTool(tool, { required: false });
    say(`${tool.padEnd(14)} ${found ? `${found} (${toolVersion(found)})` : "not found"}`);
  }
  step("artifacts");
  describeArtifacts();
  const exe = appBinary();
  if (existsSync(exe)) say(`binary format: ${isElf(exe) ? "ELF (BOLT-able)" : "not ELF (BOLT cannot rewrite it)"}`);
}

function clean() {
  step(`removing ${path.relative(repoRoot, perfDir)}`);
  rmSync(perfDir, { recursive: true, force: true });
  say("done");
}

if (isMain(import.meta.url)) runCli(async () => {
  const { command, flags: parsed } = parseArgs(process.argv.slice(2));
  switch (command) {
    case "instrument":
      return instrument(parsed);
    case "train":
      return train(parsed);
    case "merge":
      return merge();
    case "build":
      return build(parsed);
    case "flags":
      return flags(parsed);
    case "status":
      return status();
    case "clean":
      return clean();
    default:
      console.error(USAGE);
      process.exit(command ? 1 : 0);
  }
});
