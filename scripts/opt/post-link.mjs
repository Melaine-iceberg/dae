// `beforeBundleCommand` hook — wired by `src-tauri/tauri.perf.conf.json`, which
// only the Linux release leg adds.
//
// Tauri runs this after `cargo build` and before the installers are assembled,
// which is the one window in which the binary can still be rewritten: bundling
// copies `target/release/dae` into every package and signs the updater artifact
// afterwards, so anything that happens here lands in all of them.
//
// It is deliberately best-effort. A release that loses its BOLT pass is still a
// working release, while a release that fails to build because `llvm-bolt` was
// a version behind is an outage — so every reason to skip is printed and the
// build continues. `DAE_PERF_STRICT=1` flips that for local debugging.

import path from "node:path";
import { existsSync } from "node:fs";
import { appBinary, fileSize, findLlvmTool, humanSize, isElf, isMain, repoRoot, run, runCli, say, step, warn, which } from "./lib.mjs";
import { applyBolt } from "./bolt.mjs";

const stripBinary = (exe) => {
  // The perf build sets CARGO_PROFILE_RELEASE_STRIP=false so BOLT can read the
  // binary; the shipped one must be stripped again, whether or not BOLT ran.
  const tool = findLlvmTool("llvm-strip", { required: false }) ?? which("strip");
  if (!tool) {
    warn("no llvm-strip or strip on PATH: the shipped binary keeps its symbol table");
    return;
  }
  if (!run(tool, ["--strip-all", exe], { allowFailure: true }).ok) {
    warn(`\`${tool} --strip-all\` failed; the shipped binary keeps its symbol table`);
    return;
  }
  say(`stripped: ${path.relative(repoRoot, exe)} — ${humanSize(fileSize(exe))}`);
};

if (isMain(import.meta.url)) runCli(async () => {
  // The Tauri hook never passes an argument; the Qt release job runs this
  // script directly after `cargo build --release`.
  if (process.argv.includes("--qt")) process.env.DAE_PERF_SHELL = "qt";
  const exe = appBinary();
  const seconds = Number(process.env.DAE_PERF_TRAIN_SECONDS ?? 45);

  step("post-link: LLVM BOLT");
  if (!existsSync(exe)) {
    warn(`no binary at ${path.relative(repoRoot, exe)} — nothing to post-process`);
    return;
  }
  if (process.env.DAE_PERF_SKIP_BOLT) {
    say("DAE_PERF_SKIP_BOLT is set: skipping BOLT");
    stripBinary(exe);
    return;
  }
  if (!isElf(exe)) {
    say("not an ELF binary: LLVM BOLT does not support this platform, skipping");
    return;
  }

  try {
    const result = await applyBolt({ exe, seconds });
    if (!result.applied) {
      warn(`BOLT skipped: ${result.reason}`);
      stripBinary(exe);
      return;
    }
    step("post-link: done");
  } catch (error) {
    if (process.env.DAE_PERF_STRICT) throw error;
    warn(`BOLT failed (${error.message.split("\n")[0]}); shipping the binary as it was built`);
    stripBinary(exe);
  }
});
