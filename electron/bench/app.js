"use strict";

const dae = window.daeBridge;
const params = new URLSearchParams(window.location.search);
const MODE = params.get("mode") === "bench" ? "bench" : "demo";
const RUN_ID = (params.get("runId") || "manual").replace(/[^A-Za-z0-9_-]/g, "_");
const BENCH_DIR = params.get("dir") || "";
const BENCH_IMAGES = params.get("images") || "";
const EXPECTED = Number(params.get("count") || "0");

const THRESHOLDS = {
  mainLagMs: 150,
  mainLagTargetMs: 50,
  listingMs: 5000,
  rawListingMs: 5000,
  thumbnailP95Ms: 3000,
  terminalBytes: 1024 * 1024,
};

const IMAGE_EXTENSION = /\.(png|jpe?g|webp|gif|bmp|tiff?|ico)$/i;
const decoder = new TextDecoder();
const subscribers = new Set();

dae.onEvent((event) => {
  for (const subscriber of [...subscribers]) {
    try {
      subscriber(event);
    } catch (error) {
      console.error("event subscriber failed", error);
    }
  }
});

function subscribeEvents(listener) {
  subscribers.add(listener);
  return () => subscribers.delete(listener);
}

// ConPTY asks the attached terminal for its cursor position before a console
// application settles. xterm.js answers this automatically in the real app;
// the Phase 0 page is not a terminal emulator, so it has to reply itself or
// cmd/PowerShell never paints a prompt.
const deviceQueryTails = new Map();
subscribeEvents((event) => {
  if (!event.data) return;
  const meta = parsePayload(event);
  const id = meta.sessionId;
  if (id == null) return;
  if (event.name === "terminal_exit") {
    deviceQueryTails.delete(id);
    return;
  }
  if (event.name !== "terminal_output") return;

  const tail = deviceQueryTails.get(id) ?? new Uint8Array(0);
  const combined = new Uint8Array(tail.length + event.data.length);
  combined.set(tail);
  combined.set(event.data, tail.length);

  let asked = false;
  for (let index = 0; index + 3 < combined.length; index += 1) {
    if (
      combined[index] === 0x1b &&
      combined[index + 1] === 0x5b &&
      combined[index + 2] === 0x36 &&
      combined[index + 3] === 0x6e
    ) {
      asked = true;
      break;
    }
  }
  deviceQueryTails.set(id, combined.slice(-3));
  if (asked) {
    dae.invoke("terminal_write", { id, data: "\u001b[1;1R" }).catch(() => {});
  }
});

function average(values) {
  if (values.length === 0) return 0;
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function percentile(values, ratio) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * ratio) - 1));
  return sorted[index];
}

function round(value) {
  return Math.round(value * 100) / 100;
}

function withTimeout(promise, ms, label) {
  let timer;
  const timeout = new Promise((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function parsePayload(event) {
  if (!event.payloadJson) return {};
  try {
    return JSON.parse(event.payloadJson);
  } catch {
    return {};
  }
}

function bytesToJson(bytes) {
  return JSON.parse(decoder.decode(bytes));
}

function thumbnailUrl(filePath, size, version) {
  return `thumbnail://img/?path=${encodeURIComponent(filePath)}&size=${size}&v=${version ?? 0}`;
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes)) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function createTerminalCapture() {
  const sessions = new Map();
  const waiters = [];

  const unsubscribe = subscribeEvents((event) => {
    const meta = parsePayload(event);
    const id = meta.sessionId;
    if (id == null) return;

    let sink = sessions.get(id);
    if (!sink) {
      sink = { id, bytes: 0, text: "", exitCode: null, decoder: new TextDecoder() };
      sessions.set(id, sink);
    }

    if (event.name === "terminal_output" && event.data) {
      sink.bytes += event.data.byteLength;
      sink.text = (sink.text + sink.decoder.decode(event.data, { stream: true })).slice(-131072);
    } else if (event.name === "terminal_exit") {
      sink.exitCode = meta.exitCode ?? 0;
    } else {
      return;
    }

    for (let index = waiters.length - 1; index >= 0; index -= 1) {
      const waiter = waiters[index];
      if (waiter.id !== id) continue;
      if (sink.text.includes(waiter.token) || (waiter.exit && sink.exitCode != null)) {
        waiters.splice(index, 1);
        waiter.resolve(sink);
      }
    }
  });

  return {
    session(id) {
      return sessions.get(id) ?? null;
    },
    waitFor(id, token, timeoutMs = 20_000, exit = false) {
      const sink = sessions.get(id);
      if (sink && (sink.text.includes(token) || (exit && sink.exitCode != null))) {
        return Promise.resolve(sink);
      }
      let waiter;
      const found = new Promise((resolve) => {
        waiter = { id, token, exit, resolve };
        waiters.push(waiter);
      });
      return withTimeout(found, timeoutMs, `terminal ${token}`).finally(() => {
        const index = waiters.indexOf(waiter);
        if (index >= 0) waiters.splice(index, 1);
      });
    },
    dispose() {
      unsubscribe();
    },
  };
}

// ---------------------------------------------------------------------------
// Shared rendering
// ---------------------------------------------------------------------------

async function showInfo() {
  const info = await dae.info();
  document.getElementById("versions").textContent = [
    `electron ${info.electron}`,
    `chromium ${info.chrome}`,
    `node ${info.node}`,
    info.platform,
    info.packaged ? "packaged" : "dev",
  ].join(" · ");
  return info;
}

async function benchLog(message) {
  const log = document.getElementById("bench-log");
  const line = `${new Date().toISOString().slice(11, 19)} ${message}`;
  log.textContent += `${line}\n`;
  log.scrollTop = log.scrollHeight;
  try {
    await dae.benchLog(message);
  } catch {
    // Logging is best-effort.
  }
}

function renderBenchSummary(results) {
  const lines = [];
  lines.push(`RESULT  ${results.pass ? "PASS" : "FAIL"}`);
  if (results.error) lines.push(`ERROR   ${results.error}`);
  lines.push("");
  lines.push("PHASES");
  for (const [name, phase] of Object.entries(results.phases || {})) {
    lines.push(`  ${name.padEnd(15)} ${JSON.stringify(phase)}`);
  }
  lines.push("");
  lines.push("CHECKS");
  for (const [name, ok] of Object.entries(results.checks || {})) {
    lines.push(`  ${ok ? "PASS" : "FAIL"}  ${name}`);
  }
  if (results.warnings?.length) {
    lines.push("");
    lines.push("WARNINGS");
    for (const warning of results.warnings) lines.push(`  ${warning}`);
  }
  document.getElementById("bench-summary").textContent = lines.join("\n");
}

// ---------------------------------------------------------------------------
// Terminal bench
// ---------------------------------------------------------------------------

async function runTerminalBench(capture) {
  const marker = `DAE_PHASE0_${RUN_ID}`;
  const readyToken = `DAE_READY_${RUN_ID}`;
  const echoToken = `DAE_ECHO_${RUN_ID}`;
  const bulkToken = `DAE_BULK_${RUN_ID}`;
  await dae.resetLag();
  const id = await dae.invoke("terminal_create", {
    cwd: BENCH_DIR,
    cols: 120,
    rows: 40,
    program: "powershell.exe",
    args: [
      "-NoLogo",
      "-NoProfile",
      "-NoExit",
      "-Command",
      `$Host.UI.RawUI.WindowTitle='${marker}'; Write-Output '${readyToken}'`,
    ],
  });

  try {
    const started = performance.now();
    await capture.waitFor(id, readyToken, 20_000);
    const readyMs = performance.now() - started;

    const echoStarted = performance.now();
    await dae.invoke("terminal_write", {
      id,
      data: `Write-Output ('DAE'+'_ECHO_${RUN_ID}')\r`,
    });
    await capture.waitFor(id, echoToken, 10_000);
    const echoLatencyMs = performance.now() - echoStarted;

    const beforeBulk = capture.session(id)?.bytes ?? 0;
    const bulkStarted = performance.now();
    await dae.invoke("terminal_write", {
      id,
      data: `$s='x'*1048576; [Console]::Out.Write($s); [Console]::Out.WriteLine(); Write-Output ('DAE'+'_BU'+'LK_${RUN_ID}')\r`,
    });
    // Wait for both the token and the payload. The token alone can appear in
    // command echo, and the benchmark is specifically about the bytes.
    const bulkDeadline = performance.now() + 60_000;
    let sink = null;
    for (;;) {
      sink = capture.session(id);
      if (
        sink &&
        sink.text.includes(bulkToken) &&
        sink.bytes - beforeBulk >= THRESHOLDS.terminalBytes
      ) {
        break;
      }
      if (performance.now() > bulkDeadline) {
        throw new Error("terminal bulk output timed out before 1 MiB arrived");
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    const bulkMs = performance.now() - bulkStarted;

    const totalBytes = sink?.bytes ?? 0;
    const bulkBytes = totalBytes - beforeBulk;
    const mainLagMs = await dae.readLag();

    return {
      id,
      marker,
      readyMs: round(readyMs),
      echoLatencyMs: round(echoLatencyMs),
      echoOk: true,
      bulkMs: round(bulkMs),
      bulkBytes,
      totalBytes,
      mainLagMs: round(mainLagMs),
    };
  } finally {
    try {
      await dae.invoke("terminal_kill", { id });
    } catch {
      // The session may have already exited.
    }
  }
}

// ---------------------------------------------------------------------------
// Listing benches
// ---------------------------------------------------------------------------

async function streamListing() {
  const streamId = `bench-stream-${RUN_ID}`;
  const counters = { batches: 0, entries: 0, bytes: 0, done: null };

  const done = new Promise((resolve, reject) => {
    let unsubscribe = () => {};
    const timer = setTimeout(() => {
      unsubscribe();
      reject(new Error("stream listing timed out after 30s"));
    }, 30_000);

    unsubscribe = subscribeEvents((event) => {
      const meta = parsePayload(event);
      if (meta.streamId !== streamId) return;
      if (event.name === "directory_batch") {
        const batch = event.data ? bytesToJson(event.data) : [];
        counters.batches += 1;
        counters.entries += batch.length;
        counters.bytes += event.data?.byteLength ?? 0;
        return;
      }
      if (event.name === "directory_done") {
        counters.done = meta;
        clearTimeout(timer);
        unsubscribe();
        resolve(counters);
        return;
      }
      if (event.name === "directory_error") {
        clearTimeout(timer);
        unsubscribe();
        reject(new Error(meta.message || "stream listing failed"));
      }
    });
  });

  const started = performance.now();
  await dae.invoke("start_directory_listing", {
    path: BENCH_DIR,
    streamId,
    batchSize: 512,
  });
  const result = await done;
  return { ...result, ms: round(performance.now() - started) };
}

// ---------------------------------------------------------------------------
// Thumbnail bench
// ---------------------------------------------------------------------------

async function runThumbnailBench() {
  const listing = JSON.parse(
    await dae.invokeJson("read_directory", { path: BENCH_IMAGES }),
  );
  const images = listing.entries
    .filter((entry) => !entry.isDirectory && IMAGE_EXTENSION.test(entry.name))
    .slice(0, 100);
  if (images.length < 100) {
    throw new Error(`thumbnail bench needs 100 images, found ${images.length}`);
  }

  const container = document.getElementById("thumbs");
  container.replaceChildren();

  await dae.resetLag();
  const started = performance.now();
  const outcomes = await Promise.all(
    images.map(
      (entry) =>
        new Promise((resolve) => {
          const image = document.createElement("img");
          const loadStarted = performance.now();
          image.alt = entry.name;
          image.title = entry.path;
          image.onload = () =>
            resolve({
              ok: true,
              ms: performance.now() - loadStarted,
              width: image.naturalWidth,
              height: image.naturalHeight,
            });
          image.onerror = () =>
            resolve({ ok: false, ms: performance.now() - loadStarted, width: 0, height: 0 });
          image.src = thumbnailUrl(entry.path, 160, entry.modifiedMs);
          container.appendChild(image);
        }),
    ),
  );

  const elapsedMs = performance.now() - started;
  const mainLagMs = await dae.readLag();
  const latencies = outcomes.map((outcome) => outcome.ms);
  const loaded = outcomes.filter((outcome) => outcome.ok && outcome.width > 0);
  const errors = outcomes.filter((outcome) => !outcome.ok).length;

  return {
    count: images.length,
    loaded: loaded.length,
    errors,
    elapsedMs: round(elapsedMs),
    p50Ms: round(percentile(latencies, 0.5)),
    p95Ms: round(percentile(latencies, 0.95)),
    mainLagMs: round(mainLagMs),
    sampleWidth: loaded[0]?.width ?? 0,
    sampleHeight: loaded[0]?.height ?? 0,
  };
}

// ---------------------------------------------------------------------------
// Bench driver
// ---------------------------------------------------------------------------

let benchFinished = false;

async function runBench() {
  const results = {
    runId: RUN_ID,
    kind: "phase0",
    startedAt: new Date().toISOString(),
    expectedEntries: EXPECTED,
    paths: { dir: BENCH_DIR, images: BENCH_IMAGES },
    thresholds: THRESHOLDS,
    phases: {},
    checks: {},
    warnings: [],
  };
  const status = document.getElementById("bench-status");
  const setStatus = (text, className = "") => {
    status.textContent = text;
    status.className = `pill ${className}`.trim();
  };
  const capture = createTerminalCapture();

  try {
    const info = await showInfo();
    results.environment = info;
    await benchLog(`core ready: electron ${info.electron}, node ${info.node}, packaged=${info.packaged}`);
    if (!BENCH_DIR) throw new Error("--bench-dir is required");
    if (!BENCH_IMAGES) throw new Error("--bench-images is required");

    setStatus("ping");
    const pings = [];
    await dae.resetLag();
    for (let index = 0; index < 50; index += 1) {
      const tick = performance.now();
      await dae.invoke("ping", { seq: index });
      pings.push(performance.now() - tick);
    }
    results.phases.ping = {
      count: pings.length,
      avgMs: round(average(pings)),
      p95Ms: round(percentile(pings, 0.95)),
      mainLagMs: round(await dae.readLag()),
    };
    await benchLog(`ping p50=${round(percentile(pings, 0.5))}ms p95=${results.phases.ping.p95Ms}ms`);

    setStatus("listing");
    await dae.resetLag();
    const listingStarted = performance.now();
    const listingJson = await dae.invokeJson("read_directory", { path: BENCH_DIR });
    const listingMs = performance.now() - listingStarted;
    const listing = JSON.parse(listingJson);
    results.phases.listing = {
      ms: round(listingMs),
      entries: listing.entries.length,
      bytes: listingJson.length,
      mainLagMs: round(await dae.readLag()),
    };
    await benchLog(
      `listing ${results.phases.listing.entries} entries in ${results.phases.listing.ms}ms`,
    );

    setStatus("raw listing");
    await dae.resetLag();
    const rawStarted = performance.now();
    const rawBytes = await dae.invokeRaw("read_directory", { path: BENCH_DIR });
    const rawMs = performance.now() - rawStarted;
    const rawListing = rawBytes ? bytesToJson(rawBytes) : { entries: [] };
    results.phases.rawListing = {
      ms: round(rawMs),
      entries: rawListing.entries.length,
      bytes: rawBytes?.byteLength ?? 0,
      mainLagMs: round(await dae.readLag()),
    };
    await benchLog(
      `raw listing ${results.phases.rawListing.entries} entries, ${formatBytes(results.phases.rawListing.bytes)} in ${results.phases.rawListing.ms}ms`,
    );

    setStatus("stream");
    await dae.resetLag();
    const stream = await streamListing();
    results.phases.streamListing = { ...stream, mainLagMs: round(await dae.readLag()) };
    await benchLog(
      `stream ${stream.entries} entries in ${stream.batches} batches, ${stream.ms}ms`,
    );

    setStatus("terminal");
    results.phases.terminal = await runTerminalBench(capture);
    await benchLog(
      `terminal echo ${results.phases.terminal.echoLatencyMs}ms, ${formatBytes(results.phases.terminal.bulkBytes)} bulk in ${results.phases.terminal.bulkMs}ms`,
    );

    setStatus("thumbnails");
    results.phases.thumbnails = await runThumbnailBench();
    await benchLog(
      `thumbnails ${results.phases.thumbnails.loaded}/${results.phases.thumbnails.count} p95=${results.phases.thumbnails.p95Ms}ms`,
    );
  } catch (error) {
    results.error = String(error?.stack || error);
    await benchLog(`FAILED: ${error?.message || error}`);
  } finally {
    capture.dispose();
  }

  const lagValues = Object.values(results.phases)
    .map((phase) => phase.mainLagMs)
    .filter((value) => Number.isFinite(value));
  const mainLagMs = lagValues.length ? Math.max(...lagValues) : 0;
  results.mainLagMs = round(mainLagMs);

  const listing = results.phases.listing;
  const rawListing = results.phases.rawListing;
  const streamListingPhase = results.phases.streamListing;
  const terminal = results.phases.terminal;
  const thumbnails = results.phases.thumbnails;

  results.checks = {
    listing_entries: EXPECTED <= 0 || listing?.entries === EXPECTED,
    listing_ms: (listing?.ms ?? Infinity) <= THRESHOLDS.listingMs,
    raw_listing_entries: EXPECTED <= 0 || rawListing?.entries === EXPECTED,
    raw_listing_ms: (rawListing?.ms ?? Infinity) <= THRESHOLDS.rawListingMs,
    stream_entries: EXPECTED <= 0 || streamListingPhase?.entries === EXPECTED,
    terminal_echo: Boolean(terminal?.echoOk),
    terminal_bytes: (terminal?.bulkBytes ?? 0) >= THRESHOLDS.terminalBytes,
    thumbnails_loaded:
      thumbnails?.count === 100 && thumbnails?.loaded === 100 && thumbnails?.errors === 0,
    thumbnail_p95_ms: (thumbnails?.p95Ms ?? Infinity) <= THRESHOLDS.thumbnailP95Ms,
    main_lag_ms: mainLagMs <= THRESHOLDS.mainLagMs,
  };
  results.pass = !results.error && Object.values(results.checks).every(Boolean);
  if (mainLagMs > THRESHOLDS.mainLagTargetMs) {
    results.warnings.push(
      `main lag ${round(mainLagMs)}ms exceeds the ${THRESHOLDS.mainLagTargetMs}ms target (hard limit ${THRESHOLDS.mainLagMs}ms)`,
    );
  }
  results.finishedAt = new Date().toISOString();

  setStatus(results.pass ? "pass" : "fail", results.pass ? "pass" : "fail");
  renderBenchSummary(results);
  benchFinished = true;
  try {
    await dae.benchDone(results);
  } catch {
    // The app is already quitting.
  }
}

// ---------------------------------------------------------------------------
// Demo mode
// ---------------------------------------------------------------------------

const elements = {
  dirInput: document.getElementById("dir-input"),
  listButton: document.getElementById("list-button"),
  listMeta: document.getElementById("list-meta"),
  listOutput: document.getElementById("list-output"),
  imagesInput: document.getElementById("images-input"),
  thumbsButton: document.getElementById("thumbs-button"),
  thumbsMeta: document.getElementById("thumbs-meta"),
  thumbs: document.getElementById("thumbs"),
  terminalButton: document.getElementById("terminal-button"),
  terminalOutput: document.getElementById("terminal-output"),
  terminalInput: document.getElementById("terminal-input"),
  terminalSend: document.getElementById("terminal-send"),
};

let demoTtyId = null;
let demoCapture = null;

async function listDirectory(target) {
  if (!target) return;
  elements.listMeta.textContent = "reading…";
  const started = performance.now();
  try {
    const json = await dae.invokeJson("read_directory", { path: target });
    const elapsed = performance.now() - started;
    const listing = JSON.parse(json);
    elements.listMeta.textContent = `${listing.entries.length} entries · ${elapsed.toFixed(1)} ms · ${formatBytes(json.length)} JSON`;
    const rows = listing.entries
      .slice(0, 300)
      .map(
        (entry) =>
          `${entry.isDirectory ? "d" : "-"} ${String(entry.size).padStart(10)}  ${entry.name}`,
      );
    if (listing.entries.length > 300) rows.push(`… ${listing.entries.length - 300} more`);
    elements.listOutput.textContent = rows.join("\n");
  } catch (error) {
    elements.listMeta.textContent = "failed";
    elements.listOutput.textContent = String(error);
  }
}

async function loadDemoThumbnails() {
  const target = elements.imagesInput.value.trim();
  if (!target) return;
  elements.thumbsMeta.textContent = "reading…";
  try {
    const listing = JSON.parse(await dae.invokeJson("read_directory", { path: target }));
    const images = listing.entries
      .filter((entry) => !entry.isDirectory && IMAGE_EXTENSION.test(entry.name))
      .slice(0, 100);
    if (images.length === 0) {
      elements.thumbsMeta.textContent = "no images in that directory";
      return;
    }
    elements.thumbs.replaceChildren();
    const started = performance.now();
    let loaded = 0;
    await Promise.all(
      images.map(
        (entry) =>
          new Promise((resolve) => {
            const image = document.createElement("img");
            image.alt = entry.name;
            image.title = entry.path;
            image.onload = () => {
              loaded += 1;
              resolve();
            };
            image.onerror = () => resolve();
            image.src = thumbnailUrl(entry.path, 160, entry.modifiedMs);
            elements.thumbs.appendChild(image);
          }),
      ),
    );
    elements.thumbsMeta.textContent = `${loaded}/${images.length} loaded in ${(performance.now() - started).toFixed(0)} ms`;
  } catch (error) {
    elements.thumbsMeta.textContent = String(error);
  }
}

async function startDemoTerminal() {
  if (demoTtyId != null) return;
  demoCapture = createTerminalCapture();
  demoTtyId = await dae.invoke("terminal_create", {
    cwd: elements.dirInput.value || undefined,
    cols: 100,
    rows: 32,
  });
  const id = demoTtyId;
  elements.terminalOutput.textContent = "";
  const timer = setInterval(() => {
    if (demoTtyId !== id) {
      clearInterval(timer);
      return;
    }
    const sink = demoCapture.session(id);
    if (sink) elements.terminalOutput.textContent = sink.text;
    elements.terminalOutput.scrollTop = elements.terminalOutput.scrollHeight;
  }, 80);
}

async function sendDemoTerminal() {
  if (demoTtyId == null) await startDemoTerminal();
  const data = elements.terminalInput.value;
  elements.terminalInput.value = "";
  await dae.invoke("terminal_write", { id: demoTtyId, data: `${data}\r` });
}

async function initDemo() {
  await showInfo();
  elements.listButton.addEventListener("click", () => listDirectory(elements.dirInput.value));
  elements.thumbsButton.addEventListener("click", loadDemoThumbnails);
  elements.terminalButton.addEventListener("click", startDemoTerminal);
  elements.terminalSend.addEventListener("click", sendDemoTerminal);
  elements.terminalInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void sendDemoTerminal();
    }
  });

  try {
    const home = await dae.invoke("get_home_directory");
    elements.dirInput.value = home;
    elements.imagesInput.value = home;
    await listDirectory(home);
  } catch (error) {
    elements.listMeta.textContent = String(error);
  }
}

async function boot() {
  if (MODE === "bench") {
    document.getElementById("demo").hidden = true;
    document.getElementById("bench").hidden = false;
    await runBench();
    return;
  }
  document.getElementById("bench").hidden = true;
  await initDemo();
}

boot().catch(async (error) => {
  console.error(error);
  if (MODE === "bench" && !benchFinished) {
    benchFinished = true;
    try {
      await dae.benchDone({
        runId: RUN_ID,
        pass: false,
        error: String(error?.stack || error),
        phases: {},
        checks: {},
      });
    } catch {
      // Nothing else to do.
    }
  }
});
