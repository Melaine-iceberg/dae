/**
 * Cold-start attribution: where the milliseconds between process start and the
 * first frame actually go.
 *
 * `docs/cold-start-js.md` §1 says to get a waterfall before choosing any of its
 * options, because the five accounts it lists ("Tauri/WebView2 init", "HTML and
 * bundle", "locale and handoff", "React", "first data") are fixed in different
 * layers and an optimization picked without the split lands in the wrong one.
 * The marks that produce it live behind `DAE_STARTUP_PERF=1`:
 *
 * | mark | recorded by |
 * |---|---|
 * | `process-start` | `tab_perf::process_start`, at the top of `run` |
 * | `window-built` / `setup` / `prefetch` | `lib.rs`, around the main window |
 * | `html-start` / `html-parsed` / `boot-eval-start` | `index.html`, inline |
 * | `boot-eval-end` / `locale` / `handoff` / `render` | `main.tsx` |
 * | `commit` / `first-frame` / `first-data` / `paint:*` | `main.tsx` |
 * | `html-frame-N` | `index.html`, three `rAF` callbacks — a frame was produced |
 *
 * The timeline is written by the Rust side to `DAE_STARTUP_PERF_OUT` (a release
 * build has no console, and the log file rotates). This harness launches the
 * binary, waits for the block, kills the process, and folds N runs into per-stage
 * medians plus the spans the doc's decision criterion is stated in.
 *
 * It is not part of the app and nothing imports it. Run it with Bun, from the
 * repository root, after `bun run tauri:build:release --no-bundle`:
 *
 * ```bash
 * bun bench/cold-start/run.ts --runs 7
 * bun bench/cold-start/run.ts --runs 5 --fresh-profile
 * bun bench/cold-start/run.ts --runs 9 --ab          # static skeleton vs spinner
 * bun bench/cold-start/run.ts --runs 6 --ab --yield  # same, yielding a frame first
 * ```
 *
 * `--ab` runs the §2 A/B (`docs/cold-start-js.md` §8 第 3 条) inside one binary: the
 * skeleton is in `index.html` either way and `DAE_BOOT_SKELETON` selects the
 * arm, so the two sets are interleaved on the same machine state instead of
 * being two builds measured an hour apart. `--runs` is then per arm.
 * `--yield` turns on `DAE_BOOT_YIELD` for **both** arms: `main.tsx` gives the
 * engine two frames before `createRoot`, which is the only way the static screen
 * is ever painted (the result, and why that is a product trade, is in
 * `docs/decisions/boot-screen-prerender.md`).
 *
 * `--fresh-profile` deletes `%LOCALAPPDATA%\<identifier>\EBWebView` before every
 * run, which is what a first launch looks like: no V8 code cache, no
 * localStorage. Without it every run is a repeat launch, which is the state a
 * user is in almost all the time — measure both, they do not have the same
 * answer to "how much of this is parse".
 *
 * Per-machine numbers: compare this harness against itself before and after a
 * change, never against another machine's.
 */

import { existsSync, mkdirSync, readFileSync, renameSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";

/** The path `tauri build --no-bundle` writes; overridable with `--exe`. */
const DEFAULT_EXE = "src-tauri/target/release/dae.exe";

/** Where one log per run lands, so a failed run can be read afterwards. */
const OUT_DIR = ".perf/cold-start";

/** How long one launch may take before the harness gives up on it. */
const RUN_TIMEOUT_MS = 30_000;

/** Polling interval for the timeline file. */
const POLL_MS = 50;

type Run = {
  index: number;
  /** Which boot screen this launch showed: the spinner or the static skeleton. */
  arm: Arm;
  log: string;
  /** Stage name -> ms since process start. Sorted by time in the file. */
  marks: Map<string, number>;
};

/**
 * The two boot screens `index.html` can paint before React runs.
 *
 * `splash` is what ships today: the themed canvas and a spinner. `skeleton` is
 * §2's cheapest form, a static shell; Rust turns it on with `DAE_BOOT_SKELETON`.
 */
type Arm = "splash" | "skeleton";

function argValue(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  return index < 0 ? undefined : process.argv[index + 1];
}

function flag(name: string): boolean {
  return process.argv.includes(name);
}

function positiveInt(name: string, fallback: number): number {
  const raw = argValue(name);
  const parsed = raw === undefined ? Number.NaN : Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? Math.floor(parsed) : fallback;
}

const exe = resolve(argValue("--exe") ?? DEFAULT_EXE);
const runs = positiveInt("--runs", 5);
const freshProfile = flag("--fresh-profile");
const ab = flag("--ab");
/** Diagnostic arm: yield a frame before React mounts (`DAE_BOOT_YIELD`). */
const bootYield = flag("--yield");

/** Tauri names the WebView2 user-data directory after the bundle identifier. */
function profileDir(): string | null {
  const local = process.env.LOCALAPPDATA;
  return local ? join(local, "com.melaine.dae", "EBWebView") : null;
}

/**
 * What is left where the real profile was while `--fresh-profile` runs.
 *
 * A first-launch measurement needs the directory gone, and the directory holds
 * the user's theme and sidebar state as well as V8's code cache. So it is moved
 * aside rather than deleted and put back afterwards, including when a run
 * throws - a harness must not cost the person running it their UI state.
 */
function backupDir(): string | null {
  const profile = profileDir();
  return profile ? `${profile}.harness-backup` : null;
}

function setProfileAside(): void {
  const profile = profileDir();
  const backup = backupDir();
  if (!profile || !backup) return;
  if (existsSync(backup)) {
    throw new Error(
      `${backup} is left over from an interrupted run — move it back to ${profile} first`,
    );
  }
  if (existsSync(profile)) renameSync(profile, backup);
}

function restoreProfile(): void {
  const profile = profileDir();
  const backup = backupDir();
  if (!profile || !backup || !existsSync(backup)) return;
  rmSync(profile, { recursive: true, force: true });
  renameSync(backup, profile);
}

function sleep(ms: number): Promise<void> {
  return new Promise((done) => setTimeout(done, ms));
}

/**
 * Whether a `dae` process is already running.
 *
 * The app is single-instance: a second launch forwards its arguments to the
 * first one and exits, so a leftover instance would silently turn every run
 * into a failed launch rather than a cold start. `tasklist` says "no tasks match"
 * on its own, so an empty answer and that message cannot be told apart by
 * emptiness — the exe name has to be looked for.
 */
async function daeRunning(): Promise<boolean> {
  if (process.platform === "win32") {
    const check = Bun.spawnSync(["tasklist", "/FI", "IMAGENAME eq dae.exe", "/NH", "/FO", "CSV"]);
    return check.stdout.toString().toLowerCase().includes("dae.exe");
  }
  return Bun.spawnSync(["pgrep", "-x", "dae"]).exitCode === 0;
}

async function killTree(pid: number): Promise<void> {
  if (process.platform === "win32") {
    Bun.spawnSync(["taskkill", "/PID", String(pid), "/T", "/F"]);
  } else {
    try {
      process.kill(-pid, "SIGKILL");
    } catch {
      // The process tree may already be gone.
    }
  }
  // Wait for the OS to reap it: the next launch is a no-op if this one is still
  // holding the single-instance lock.
  for (let waited = 0; waited < 5000; waited += 100) {
    if (!(await daeRunning())) return;
    await sleep(100);
  }
}

/** Reads one complete timeline block, or `null` while TOTAL has not landed. */
function readTimeline(path: string): Map<string, number> | null {
  if (!existsSync(path)) return null;
  const text = readFileSync(path, "utf8");
  if (!text.includes("TOTAL")) return null;

  const marks = new Map<string, number>();
  for (const line of text.split("\n")) {
    // `  process-start                     41.2 ms`
    const match = /^\s{2}(.+?)\s+(-?\d+(?:\.\d+)?) ms$/.exec(line);
    if (!match) continue;
    const stage = match[1].trim();
    if (stage === "TOTAL") continue;
    // A stage can legitimately repeat (the frontend reports every query that
    // settles); the timeline is sorted, so the first occurrence is the one the
    // waterfall means.
    if (!marks.has(stage)) marks.set(stage, Number(match[2]));
  }
  return marks.size > 0 ? marks : null;
}

async function oneRun(index: number, arm: Arm): Promise<Run> {
  const profile = profileDir();
  if (freshProfile && profile) {
    rmSync(profile, { recursive: true, force: true });
  }

  const log = join(OUT_DIR, `run-${index}.log`);
  rmSync(log, { force: true });

  const child = Bun.spawn([exe], {
    env: {
      ...process.env,
      DAE_STARTUP_PERF: "1",
      DAE_STARTUP_PERF_OUT: resolve(log),
      // Explicit on both arms: an inherited variable must not silently turn a
      // "splash" run into a skeleton one.
      DAE_BOOT_SKELETON: arm === "skeleton" ? "1" : "0",
      DAE_BOOT_YIELD: bootYield ? "1" : "0",
    },
    stdout: "ignore",
    stderr: "ignore",
  });

  const deadline = Date.now() + RUN_TIMEOUT_MS;
  let marks: Map<string, number> | null = null;
  while (Date.now() < deadline) {
    marks = readTimeline(log);
    if (marks) break;
    await sleep(POLL_MS);
  }

  await killTree(child.pid);
  try {
    await child.exited;
  } catch {
    // Killed on purpose; the exit status is not the measurement.
  }

  if (!marks) {
    throw new Error(
      `run ${index}: no timeline in ${log} within ${RUN_TIMEOUT_MS} ms — ` +
        `is the probe compiled in, and is ${exe} the build that has it?`,
    );
  }
  return { index, arm, log, marks };
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[middle - 1] + sorted[middle]) / 2 : sorted[middle];
}

function round(value: number): number {
  return Math.round(value * 10) / 10;
}

/** The median of one stage, or `undefined` when a run never recorded it. */
function stage(runsAt: Run[], name: string): number | undefined {
  const values = runsAt.flatMap((run) => {
    const value = run.marks.get(name);
    return value === undefined ? [] : [value];
  });
  return values.length === 0 ? undefined : median(values);
}

type Span = {
  label: string;
  from: string;
  to: string;
};

/**
 * The spans the report is read in. They add up to `process-start` →
 * `first-frame`, which is the number "cold start" means here; `first-data` is
 * shown after it because a payload that lands after the first frame is not part
 * of that total, and a payload that lands before it is the reason it was late.
 */
const SPANS: Span[] = [
  { label: "process → setup (plugin init)", from: "process-start", to: "setup" },
  { label: "setup → window-config", from: "setup", to: "window-config" },
  { label: "window-config → window-built (WebView2)", from: "window-config", to: "window-built" },
  { label: "window-built → html-start", from: "window-built", to: "html-start" },
  { label: "html-start → boot-eval-start", from: "html-start", to: "boot-eval-start" },
  { label: "boot-eval-start → boot-eval-end", from: "boot-eval-start", to: "boot-eval-end" },
  { label: "boot-eval-end → render", from: "boot-eval-end", to: "render" },
  { label: "render → commit", from: "render", to: "commit" },
  { label: "commit → first-frame", from: "commit", to: "first-frame" },
];

function spanMs(runsAt: Run[], span: Span): number | undefined {
  const from = stage(runsAt, span.from);
  const to = stage(runsAt, span.to);
  return from === undefined || to === undefined ? undefined : to - from;
}

function printTable(header: string, rows: [string, string][]): void {
  const width = Math.max(header.length, ...rows.map(([label]) => label.length));
  console.log(`\n${header}`);
  for (const [label, value] of rows) {
    console.log(`  ${label.padEnd(width)}  ${value.padStart(9)}`);
  }
}

/** Label plus one value per arm and the delta between them. */
function printColumns(header: string, rows: [string, string, string, string][]): void {
  const width = Math.max(header.length, ...rows.map(([label]) => label.length));
  console.log(`\n${header}`);
  for (const [label, left, right, delta] of rows) {
    console.log(
      `  ${label.padEnd(width)}  ${left.padStart(9)}  ${right.padStart(9)}  ${delta.padStart(9)}`,
    );
  }
}

/**
 * The instant this run first showed *anything* from the document.
 *
 * Two independent sources, because neither is reliable on its own: the frame
 * probe in `index.html` (`requestAnimationFrame` only fires when a frame is
 * actually produced) and the engine's paint entries (which WebView2 reports
 * inconsistently, see `docs/cold-start-js.md` §1.4). Taking the earliest of
 * the ones that exist keeps a missing row from hiding a paint that happened.
 */
function firstScreen(run: Run): number | undefined {
  const candidates: number[] = [];
  for (const [name, at] of run.marks) {
    if (name.startsWith("html-frame-")) candidates.push(at);
  }
  for (const name of ["paint:first-paint", "paint:first-contentful-paint"]) {
    const at = run.marks.get(name);
    if (at !== undefined) candidates.push(at);
  }
  return candidates.length === 0 ? undefined : Math.min(...candidates);
}

/** Whether a frame was produced **before** the entry bundle finished evaluating. */
function framedBeforeEval(run: Run): boolean {
  const evalEnd = run.marks.get("boot-eval-end");
  if (evalEnd === undefined) return false;
  for (const [name, at] of run.marks) {
    if (name.startsWith("html-frame-") && at < evalEnd) return true;
  }
  return false;
}

/** How long the pre-React screen stayed on screen before React replaced it. */
function screenLead(run: Run): number | undefined {
  const screen = firstScreen(run);
  const commit = run.marks.get("commit");
  if (screen === undefined || commit === undefined || screen > commit) return undefined;
  return commit - screen;
}

/** `count/total` for a per-run yes/no fact — a median cannot express these. */
function tally(runsAt: Run[], predicate: (run: Run) => boolean): string {
  const hits = runsAt.filter(predicate).length;
  return `${hits}/${runsAt.length}`;
}

/**
 * The §2 numbers, printed for one arm.
 *
 * `first-frame` is deliberately still here even though it cannot move: it is
 * measured after React's commit, so the skeleton can never make it earlier.
 * What can change is what the user is looking at while that frame is being
 * produced, which is the row above it.
 */
function reportArm(label: string, runsAt: Run[]): void {
  // Paint entries are engine-recorded, so they can be absent on an engine that
  // does not implement them; skip their rows rather than print an empty column.
  const extras: [string, string][] = [];
  const firstFrame = stage(runsAt, "first-frame");
  const firstPaint = stage(runsAt, "paint:first-paint");
  const firstContentful = stage(runsAt, "paint:first-contentful-paint");
  const firstFrameProbe = stage(runsAt, "html-frame-1");
  const firstData = stage(runsAt, "first-data");

  if (firstFrameProbe !== undefined) {
    extras.push(["html-frame-1 (offset)", `${round(firstFrameProbe)} ms`]);
  }
  if (firstPaint !== undefined)
    extras.push(["paint:first-paint (offset)", `${round(firstPaint)} ms`]);
  if (firstContentful !== undefined) {
    extras.push(["paint:first-contentful-paint", `${round(firstContentful)} ms`]);
  }
  if (firstData !== undefined) extras.push(["first-data (offset)", `${round(firstData)} ms`]);
  const leads = runsAt.flatMap((run) => {
    const lead = screenLead(run);
    return lead === undefined ? [] : [lead];
  });
  if (leads.length > 0) {
    extras.push([
      `static screen before commit (${leads.length}/${runsAt.length})`,
      `${round(median(leads))} ms`,
    ]);
  }
  extras.push(["frames before bundle eval", tally(runsAt, framedBeforeEval)]);
  if (firstPaint !== undefined && firstFrame !== undefined) {
    extras.push(["paint:first-paint − first-frame", `${round(firstPaint - firstFrame)} ms`]);
  }
  if (firstData !== undefined && firstFrame !== undefined) {
    extras.push(["first-data − first-frame", `${round(firstData - firstFrame)} ms`]);
  }

  const total = spanMs(runsAt, { label: "", from: "process-start", to: "first-frame" });
  const rows: [string, string][] = SPANS.map((span) => {
    const ms = spanMs(runsAt, span);
    if (ms === undefined) return [span.label, "missing"];
    const share = total === undefined || total === 0 ? "" : `  ${Math.round((ms / total) * 100)}%`;
    return [span.label, `${round(ms)} ms${share}`];
  });
  if (total !== undefined) rows.push(["TOTAL (process → first-frame)", `${round(total)} ms`]);
  rows.push(...extras);

  printTable(
    `${label} — medians over ${runsAt.length} run${runsAt.length === 1 ? "" : "s"} (ms since process start)`,
    rows,
  );

  // The doc's criterion, stated in the two numbers it names: is JS parse+exec a
  // small tail (<15%) or a main account (>=30%) of the cold start?
  //
  // The entry graph is preloaded by `dist/index.html`, so `bundle:fetched` often
  // lands before the parser reaches the module tag and the fetch is not part of
  // the bundle span at all. Subtracting from `max(boot-eval-start, fetched)`
  // keeps the label honest in both cases.
  const evalStart = stage(runsAt, "boot-eval-start");
  const fetched = stage(runsAt, "bundle:fetched");
  const evalEnd = stage(runsAt, "boot-eval-end");
  if (total && evalStart !== undefined && evalEnd !== undefined) {
    const parse = evalEnd - Math.max(evalStart, fetched ?? evalStart);
    const boot = evalEnd - evalStart;
    console.log(
      `\nparse+exec: ${round(parse)} ms = ${Math.round((parse / total) * 100)}% of cold start` +
        ` (whole bundle span ${round(boot)} ms = ${Math.round((boot / total) * 100)}%)`,
    );
    const verdict =
      parse / total >= 0.3
        ? ">= 30%: §2 / §4 are the right next step."
        : parse / total < 0.15
          ? "< 15%: only §3 (asset layer) is worth doing; skip §2/§4."
          : "15–30%: the doc does not decide here; weigh §3 against §2/§4.";
    console.log(verdict);
  }
}

/**
 * The comparison the §2 A/B exists to produce.
 *
 * It does not ask whether `first-frame` got earlier — it is measured after
 * React's commit and cannot. It asks whether the engine painted the pre-React
 * document at all, and how long that screen was up while React mounted.
 */
function reportAb(splash: Run[], skeleton: Run[]): void {
  const cell = (runsAt: Run[], name: string) => {
    const value = stage(runsAt, name);
    return value === undefined ? "missing" : `${round(value)}`;
  };
  const delta = (name: string) => {
    const left = stage(splash, name);
    const right = stage(skeleton, name);
    return left === undefined || right === undefined ? "" : `${round(right - left)}`;
  };
  const rows: [string, string, string, string][] = [];
  for (const name of [
    "html-parsed",
    "boot-eval-start",
    "boot-eval-end",
    "commit",
    "first-frame",
    "paint:first-paint",
    "paint:first-contentful-paint",
    "html-frame-1",
    "first-data",
  ]) {
    const left = cell(splash, name);
    const right = cell(skeleton, name);
    if (left === "missing" && right === "missing") continue;
    const d = delta(name);
    rows.push([name, left, right, d === "" ? "" : d]);
  }
  printColumns("§2 A/B — medians (ms since process start); last column = skeleton − splash", rows);

  const leadMedian = (runsAt: Run[]) => {
    const leads = runsAt.flatMap((run) => {
      const lead = screenLead(run);
      return lead === undefined ? [] : [lead];
    });
    return leads.length === 0 ? "none" : `${round(median(leads))} ms`;
  };
  const beforeCommit = (run: Run) => screenLead(run) !== undefined;
  console.log(
    `\n  frames before bundle eval      splash ${tally(splash, framedBeforeEval)}` +
      `   skeleton ${tally(skeleton, framedBeforeEval)}`,
  );
  console.log(
    `  screen before React commit     splash ${tally(splash, beforeCommit)}` +
      `   skeleton ${tally(skeleton, beforeCommit)}`,
  );
  console.log(
    `  that screen's lead (median)    splash ${leadMedian(splash)}` +
      `   skeleton ${leadMedian(skeleton)}`,
  );

  // Three outcomes, and they call for different decisions. Keep them apart
  // instead of collapsing into "did a frame happen": a run that yields before
  // `createRoot` can show the static screen without any pre-eval frame, and
  // that is exactly the trade (a later `first-frame` for a filled window)
  // rather than a win.
  const preEval = skeleton.filter(framedBeforeEval).length;
  const preCommit = skeleton.filter(beforeCommit).length;
  if (preCommit === 0) {
    console.log(
      "\n§2 read-out: no frame before React's commit in any run — the boot is one continuous" +
        " block of main-thread work, so a pre-rendered first screen is never seen." +
        " The content tier is not worth building.",
    );
  } else if (preEval === 0) {
    console.log(
      `\n§2 read-out: the static screen reaches the screen (${preCommit}/${skeleton.length} runs` +
        " before `commit`), but only because the run yields before `createRoot` — with `--yield`," +
        " `boot-eval-end → render` includes that wait. `first-frame` is later, not earlier: the" +
        " window exists only because the boot gave up a frame. Decide it as a product trade," +
        " not as an optimization.",
    );
  } else {
    console.log(
      `\n§2 read-out: the pre-React document reaches the screen without yielding ` +
        `(${preEval}/${skeleton.length} runs framed before \`boot-eval-end\`). \`first-frame\` still` +
        " cannot move — it is measured after React's commit — so §2's value is what is on screen" +
        " during the mount, and the content tier extends that window rather than shortening the total.",
    );
  }
}

async function main(): Promise<void> {
  if (!existsSync(exe)) {
    throw new Error(
      `${exe} does not exist — build it first: bun run tauri:build:release --no-bundle`,
    );
  }
  if (await daeRunning()) {
    throw new Error("a dae process is already running; quit it first (the app is single-instance)");
  }
  if (freshProfile && !profileDir()) {
    throw new Error("--fresh-profile needs LOCALAPPDATA; not running on Windows?");
  }

  mkdirSync(OUT_DIR, { recursive: true });
  console.log(`cold start: ${exe}`);
  console.log(
    ab
      ? `runs: ${runs} per arm (spinner vs static skeleton, interleaved)`
      : `runs: ${runs}${freshProfile ? ", fresh WebView2 profile before each" : ", repeat launches"}`,
  );
  if (bootYield) console.log("yield: two frames before React mounts (DAE_BOOT_YIELD)");

  // The order alternates every round: the machine warms up over the session
  // (page cache, WebView2 leftovers), so running one arm first every time would
  // charge that drift to the other arm.
  const arms: Arm[] = ab ? ["splash", "skeleton"] : [flag("--skeleton") ? "skeleton" : "splash"];
  const schedule: Arm[] = [];
  for (let round = 0; round < runs; round += 1) {
    schedule.push(...(round % 2 === 0 ? arms : [...arms].reverse()));
  }

  const collected: Run[] = [];
  try {
    if (freshProfile) setProfileAside();
    for (let index = 1; index <= schedule.length; index += 1) {
      const arm = schedule[index - 1]!;
      const run = await oneRun(index, arm);
      collected.push(run);
      const total = run.marks.get("first-frame");
      console.log(
        `  run ${index}/${schedule.length} [${arm}]: ${total?.toFixed(1) ?? "?"} ms to first frame`,
      );
    }
  } finally {
    if (freshProfile) {
      // The last launch's WebView2 children are killed with the tree, but they
      // can hold the directory open for a moment after their parent exits.
      await sleep(300);
      restoreProfile();
    }
  }

  if (ab) {
    reportArm(
      "spinner (today)",
      collected.filter((run) => run.arm === "splash"),
    );
    reportArm(
      "static skeleton (§2 shell tier)",
      collected.filter((run) => run.arm === "skeleton"),
    );
    reportAb(
      collected.filter((run) => run.arm === "splash"),
      collected.filter((run) => run.arm === "skeleton"),
    );
  } else {
    reportArm("cold start", collected);
  }

  console.log(`\nraw timelines: ${OUT_DIR}/run-*.log`);
}

void main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
