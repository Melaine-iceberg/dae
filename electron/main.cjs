const {
  app,
  BrowserWindow,
  ipcMain,
  net,
  protocol,
  utilityProcess,
} = require("electron");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const isBench = process.argv.includes("--bench");

function cliValue(name) {
  const prefix = `--${name}=`;
  const hit = process.argv.find((argument) => argument.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : null;
}

// A bench run must not collide with a normal instance (or a previous run):
// Electron's userData directory also scopes the single-instance lock and cache.
if (isBench) {
  const userData =
    cliValue("bench-user-data") ||
    path.join(os.tmpdir(), `dae-phase0-userdata-${process.pid}`);
  app.setPath("userData", userData);
}

protocol.registerSchemesAsPrivileged([
  {
    scheme: "thumbnail",
    privileges: {
      standard: true,
      secure: true,
      supportFetchAPI: true,
      stream: true,
      bypassCSP: true,
    },
  },
]);

const benchOut = cliValue("bench-out");
const benchRunId = cliValue("bench-run-id") || "manual";

let coreChild = null;
let coreInfo = null;
let shuttingDown = false;
let exitCode = 0;
let requestSeq = 0;
let benchTimer = null;
const pending = new Map();
const windowsByLabel = new Map();

// Main-thread event-loop lag. The acceptance page resets it before each phase
// and reads it after, so a payload that blocks main shows up as a number
// instead of a vague feeling that the app is slow.
const LAG_INTERVAL_MS = 10;
let lagMaxMs = 0;
let lagLastTick = performance.now();
setInterval(() => {
  const now = performance.now();
  const behind = now - lagLastTick - LAG_INTERVAL_MS;
  if (behind > lagMaxMs) lagMaxMs = behind;
  lagLastTick = now;
}, LAG_INTERVAL_MS);

function appRoot() {
  return app.isPackaged
    ? path.join(process.resourcesPath, "app.asar.unpacked")
    : path.resolve(__dirname, "..");
}

function spawnCore() {
  const nativePath = path.join(appRoot(), "native", "dae_core_phase0.node");
  const coreEntry = path.join(appRoot(), "electron", "core", "core-entry.cjs");
  if (!fs.existsSync(nativePath)) {
    throw new Error(
      `native addon is missing at ${nativePath}; run \`bun run native:build\``,
    );
  }

  return new Promise((resolve, reject) => {
    const child = utilityProcess.fork(coreEntry, [], {
      serviceName: "dae-phase0-core",
      stdio: "pipe",
      env: {
        ...process.env,
        DAE_NATIVE_PATH: nativePath,
        DAE_LOG_FILE: path.join(app.getPath("userData"), "core.log"),
      },
    });
    coreChild = child;

    const timer = setTimeout(() => {
      reject(new Error("core process did not report ready within 10s"));
    }, 10_000);

    child.on("message", (message) => {
      if (!message || typeof message !== "object") return;

      if (message.type === "ready") {
        clearTimeout(timer);
        coreInfo = message;
        resolve(message);
        return;
      }

      if (message.type === "response") {
        const entry = pending.get(message.id);
        if (!entry) return;
        pending.delete(message.id);
        clearTimeout(entry.timer);
        if (message.ok) entry.resolve(message.data);
        else entry.reject(new Error(message.error || "core request failed"));
        return;
      }

      if (message.type === "event") {
        broadcastEvent(message.event);
        return;
      }

      if (message.type === "fatal") {
        console.error(`[core] fatal: ${message.message}`);
        if (!shuttingDown) {
          exitCode = 1;
          rejectPending(new Error(`core fatal: ${message.message}`));
          app.quit();
        }
      }
    });

    child.on("exit", (code) => {
      if (coreChild === child) coreChild = null;
      rejectPending(new Error(`core exited with code ${code}`));
      if (!shuttingDown) {
        console.error(`[core] exited unexpectedly with code ${code}`);
        exitCode = 1;
        app.quit();
      }
    });

    if (child.stdout) child.stdout.on("data", (data) => process.stdout.write(`[core] ${data}`));
    if (child.stderr) child.stderr.on("data", (data) => process.stderr.write(`[core:err] ${data}`));
  });
}

function rejectPending(error) {
  for (const entry of pending.values()) {
    clearTimeout(entry.timer);
    entry.reject(error);
  }
  pending.clear();
}

function coreRequest(command, args, { raw = false, timeoutMs = 120_000 } = {}) {
  if (!coreChild) return Promise.reject(new Error("core process is not running"));
  const id = ++requestSeq;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`core request timed out: ${command}`));
    }, timeoutMs);
    pending.set(id, { resolve, reject, timer });
    coreChild.postMessage({ type: "request", id, command, args, raw });
  });
}

function broadcastEvent(event) {
  if (!event) return;
  const payload = {
    name: event.name,
    target: event.target ?? null,
    payloadJson: event.payloadJson ?? event.payload_json ?? null,
    data: event.data ? new Uint8Array(event.data) : null,
  };

  if (payload.target && windowsByLabel.has(payload.target)) {
    windowsByLabel.get(payload.target).webContents.send("dae:event", payload);
    return;
  }
  for (const window of windowsByLabel.values()) {
    if (!window.isDestroyed()) window.webContents.send("dae:event", payload);
  }
}

function registerCoreProtocol() {
  protocol.handle("thumbnail", async (request) => {
    try {
      const url = new URL(request.url);
      const filePath = url.searchParams.get("path");
      if (!filePath) return new Response("missing path", { status: 400 });
      if (!coreInfo) return new Response("core not ready", { status: 503 });

      const upstream = new URL(`http://127.0.0.1:${coreInfo.port}/thumb`);
      upstream.searchParams.set("path", filePath);
      upstream.searchParams.set("size", url.searchParams.get("size") || "160");
      // The token and port stay in main; the renderer only ever sees the
      // thumbnail:// URL it asked for.
      upstream.searchParams.set("token", coreInfo.token);
      return await net.fetch(upstream.toString());
    } catch (error) {
      return new Response(String(error), { status: 500 });
    }
  });
}

function registerIpc() {
  ipcMain.handle("dae:invoke", async (_event, command, args) => {
    if (typeof command !== "string") throw new Error("command must be a string");
    return coreRequest(command, args ?? null);
  });

  ipcMain.handle("dae:invokeRaw", async (_event, command, args) => {
    if (typeof command !== "string") throw new Error("command must be a string");
    const data = await coreRequest(command, args ?? null, { raw: true });
    return data ? new Uint8Array(data) : null;
  });

  ipcMain.handle("dae:info", () => ({
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node,
    platform: process.platform,
    packaged: app.isPackaged,
    nativePath: coreInfo?.nativePath ?? null,
    coreReady: Boolean(coreInfo),
    userData: app.getPath("userData"),
    runId: benchRunId,
  }));

  ipcMain.handle("dae:lag:reset", () => {
    lagMaxMs = 0;
    lagLastTick = performance.now();
    return true;
  });

  ipcMain.handle("dae:lag:read", () => lagMaxMs);

  ipcMain.handle("dae:bench:log", (_event, message) => {
    console.log(`[bench] ${String(message)}`);
    return true;
  });

  ipcMain.handle("dae:bench:done", async (_event, result) => {
    if (benchTimer) {
      clearTimeout(benchTimer);
      benchTimer = null;
    }
    const enriched = {
      ...result,
      main: { lagMaxMs },
      versions: process.versions,
      packaged: app.isPackaged,
      nativePath: coreInfo?.nativePath ?? null,
      corePort: coreInfo?.port ?? null,
    };
    if (benchOut) {
      fs.mkdirSync(path.dirname(benchOut), { recursive: true });
      fs.writeFileSync(benchOut, JSON.stringify(enriched, null, 2));
      console.log(`[bench] wrote ${benchOut}`);
    }
    exitCode = enriched.pass ? 0 : 1;
    app.quit();
    return true;
  });
}

function createWindow() {
  const window = new BrowserWindow({
    width: 1180,
    height: 820,
    show: true,
    backgroundColor: "#101116",
    title: "dae phase 0",
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
    },
  });

  windowsByLabel.set("main", window);
  window.on("closed", () => {
    windowsByLabel.delete("main");
    if (windowsByLabel.size === 0) app.quit();
  });

  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.on("will-navigate", (event) => event.preventDefault());
  window.webContents.on("did-finish-load", () => console.log("[renderer] did-finish-load"));
  window.webContents.on("did-fail-load", (_event, code, description, url) => {
    console.error(`[renderer] did-fail-load ${code} ${description} ${url}`);
  });
  window.webContents.on("preload-error", (_event, preloadPath, error) => {
    console.error(`[renderer] preload-error ${preloadPath}: ${error.stack || error}`);
  });
  window.webContents.on("console-message", (event, ...legacy) => {
    const level = event.level ?? legacy[0];
    const message = event.message ?? legacy[1];
    const line = event.lineNumber ?? legacy[2];
    const source = event.sourceId ?? legacy[3];
    console.log(`[renderer:${level}] ${message} (${source}:${line})`);
  });
  window.webContents.on("render-process-gone", (_event, details) => {
    console.error(`[renderer] process gone: ${JSON.stringify(details)}`);
  });

  const query = {
    mode: isBench ? "bench" : "demo",
    dir: cliValue("bench-dir") || "",
    images: cliValue("bench-images") || "",
    count: cliValue("bench-count") || "0",
    runId: benchRunId,
  };
  window.loadFile(path.join(__dirname, "bench", "index.html"), { query });

  if (isBench) {
    benchTimer = setTimeout(() => {
      console.error("[bench] timed out after 90s");
      exitCode = 1;
      if (benchOut) {
        fs.mkdirSync(path.dirname(benchOut), { recursive: true });
        fs.writeFileSync(
          benchOut,
          JSON.stringify({ pass: false, error: "bench timed out after 90s" }, null, 2),
        );
      }
      app.quit();
    }, 90_000);
  }
}

async function shutdownCore() {
  if (!coreChild) return;
  const child = coreChild;
  coreChild = null;
  const exited = new Promise((resolve) => {
    const timer = setTimeout(resolve, 2000);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
  try {
    child.postMessage({ type: "shutdown" });
  } catch (error) {
    console.error(`[core] shutdown message failed: ${error}`);
  }
  await exited;
  try {
    child.kill();
  } catch {
    // Already gone.
  }
}

app.on("before-quit", (event) => {
  if (shuttingDown) return;
  event.preventDefault();
  shuttingDown = true;
  void shutdownCore().finally(() => app.exit(exitCode));
});

app.whenReady().then(async () => {
  try {
    await spawnCore();
    registerIpc();
    registerCoreProtocol();
    createWindow();
  } catch (error) {
    console.error(`[main] startup failed: ${error.stack || error}`);
    exitCode = 1;
    app.quit();
  }
});

app.on("window-all-closed", () => {
  app.quit();
});
