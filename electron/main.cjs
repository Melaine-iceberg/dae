const {
  app,
  BrowserWindow,
  clipboard,
  dialog,
  ipcMain,
  net,
  protocol,
  screen,
  shell,
  utilityProcess,
} = require("electron");
const fs = require("node:fs");
const path = require("node:path");

const OPENABLE_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

function cliValue(name) {
  const prefix = `--${name}=`;
  const hit = process.argv.find((argument) => argument.startsWith(prefix));
  return hit ? hit.slice(prefix.length) : null;
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
  {
    // The renderer is a real ES-module bundle, which file:// cannot serve
    // (opaque origin). A standard, secure scheme gives it a normal origin and
    // lets Vite's absolute `/assets/...` URLs resolve unchanged.
    scheme: "dae",
    privileges: {
      standard: true,
      secure: true,
      supportFetchAPI: true,
      corsEnabled: true,
      stream: true,
    },
  },
]);

let coreChild = null;
let coreInfo = null;
let shuttingDown = false;
let exitCode = 0;
let requestSeq = 0;
const pending = new Map();
const windowsByLabel = new Map();
// Channel ids are allocated by the renderer, so main remembers which window
// sent them and routes messages back to that window only.
const channelOwners = new Map();

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
  const nativePath = path.join(appRoot(), "native", "dae_core_native.node");
  const coreEntry = path.join(appRoot(), "electron", "core", "core-entry.cjs");
  if (!fs.existsSync(nativePath)) {
    throw new Error(
      `native addon is missing at ${nativePath}; run \`bun run native:build\``,
    );
  }

  return new Promise((resolve, reject) => {
    const child = utilityProcess.fork(coreEntry, [], {
      serviceName: "dae-core",
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

      if (message.type === "channel") {
        sendChannel(message);
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

function sendChannel(message) {
  const payload = {
    channelId: message.channelId,
    payloadJson: message.payloadJson ?? null,
    data: message.data ? new Uint8Array(message.data) : null,
  };

  const owner = channelOwners.get(payload.channelId);
  if (owner && !owner.isDestroyed()) {
    owner.send("dae:channel", payload);
    return;
  }

  // A channel whose owner window is gone (or whose invoke raced the window
  // teardown) is dropped rather than broadcast: another window has its own
  // channel id space.
}

function collectChannelIds(value, ids) {
  if (!value || typeof value !== "object" || ids.size >= 256) return;
  if (typeof value.__dae_channel__ === "number") ids.add(value.__dae_channel__);
  if (Array.isArray(value)) {
    for (const item of value) collectChannelIds(item, ids);
    return;
  }
  for (const item of Object.values(value)) collectChannelIds(item, ids);
}

function rememberChannels(sender, args) {
  const ids = new Set();
  collectChannelIds(args, ids);
  for (const id of ids) channelOwners.set(id, sender);
}

// Window facts ride the renderer's event bus under Tauri's names, so the
// Window facade is just another listener rather than a second transport.
function sendWindowEvent(window, name, payload) {
  if (window.isDestroyed()) return;
  window.webContents.send("dae:event", {
    name,
    target: window.daeLabel ?? null,
    payloadJson: JSON.stringify(payload ?? null),
    data: null,
  });
}

function displayScaleFactor(window) {
  return screen.getDisplayMatching(window.getBounds()).scaleFactor;
}

function physicalSize(window) {
  const scale = displayScaleFactor(window);
  const [width, height] = window.getSize();
  return { width: Math.round(width * scale), height: Math.round(height * scale) };
}

function logDirectory() {
  const directory = app.getPath("logs");
  fs.mkdirSync(directory, { recursive: true });
  return directory;
}

// `app.getVersion()` reports the Electron version rather than dae's when the
// app is launched as a bare script (`electron electron/main.cjs`), so the
// version row always reads dae's own manifest. If that manifest cannot be read
// the request fails instead of falling back to a number that would be wrong.
function appVersion() {
  const manifest = app.isPackaged
    ? path.join(app.getAppPath(), "package.json")
    : path.resolve(__dirname, "..", "package.json");
  const { version } = JSON.parse(fs.readFileSync(manifest, "utf8"));
  if (typeof version !== "string" || version.length === 0) {
    throw new Error(`package manifest has no version: ${manifest}`);
  }
  return version;
}

const DEV_SERVER_URL = cliValue("dev-server") || process.env.DAE_DEV_SERVER || null;

const MIME_TYPES = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".json", "application/json"],
  [".svg", "image/svg+xml"],
  [".png", "image/png"],
  [".jpg", "image/jpeg"],
  [".jpeg", "image/jpeg"],
  [".webp", "image/webp"],
  [".gif", "image/gif"],
  [".ico", "image/x-icon"],
  [".woff", "font/woff"],
  [".woff2", "font/woff2"],
  [".ttf", "font/ttf"],
  [".otf", "font/otf"],
  [".wasm", "application/wasm"],
  [".map", "application/json"],
  [".txt", "text/plain; charset=utf-8"],
]);

function rendererRoot() {
  return app.isPackaged
    ? path.join(app.getAppPath(), "dist")
    : path.resolve(__dirname, "..", "dist");
}

// The Vite bundle is ES modules. file:// gives it an opaque origin where module
// imports fail, so the packaged renderer is served over `dae://` instead.
function registerRendererProtocol() {
  const root = path.resolve(rendererRoot());
  protocol.handle("dae", async (request) => {
    try {
      const url = new URL(request.url);
      const relative = decodeURIComponent(url.pathname);
      const resolved = path.resolve(root, `.${relative}`);
      if (resolved !== root && !resolved.startsWith(root + path.sep)) {
        return new Response("forbidden", { status: 403 });
      }

      let filePath = resolved;
      if (!fs.existsSync(filePath) || fs.statSync(filePath).isDirectory()) {
        if (path.extname(relative)) return new Response("not found", { status: 404 });
        filePath = path.join(root, "index.html");
      }

      const body = fs.readFileSync(filePath);
      return new Response(body, {
        headers: {
          "content-type": MIME_TYPES.get(path.extname(filePath).toLowerCase()) || "application/octet-stream",
          "cache-control": "no-cache",
        },
      });
    } catch (error) {
      return new Response(String(error), { status: 500 });
    }
  });
}

function loadRenderer(window) {
  if (DEV_SERVER_URL) {
    window.loadURL(DEV_SERVER_URL);
    return;
  }

  const index = path.join(rendererRoot(), "index.html");
  if (!fs.existsSync(index)) {
    throw new Error(`renderer bundle is missing at ${index}; run \`bun run build\` first`);
  }
  window.loadURL("dae://localhost/index.html");
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
      const response = await net.fetch(upstream.toString());
      const headers = new Headers(response.headers);
      // The grid loads thumbnails with crossOrigin="anonymous"; a custom scheme
      // does not inherit the upstream localhost origin, so it needs CORS.
      headers.set("access-control-allow-origin", "*");
      return new Response(response.body, { status: response.status, headers });
    } catch (error) {
      return new Response(String(error), { status: 500 });
    }
  });
}

function registerIpc() {
  ipcMain.handle("dae:invoke", async (event, command, args) => {
    if (typeof command !== "string") throw new Error("command must be a string");
    rememberChannels(event.sender, args ?? null);
    return coreRequest(command, args ?? null);
  });

  ipcMain.handle("dae:invokeRaw", async (event, command, args) => {
    if (typeof command !== "string") throw new Error("command must be a string");
    rememberChannels(event.sender, args ?? null);
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
  }));

  ipcMain.handle("dae:lag:reset", () => {
    lagMaxMs = 0;
    lagLastTick = performance.now();
    return true;
  });

  ipcMain.handle("dae:lag:read", () => lagMaxMs);

  ipcMain.handle("dae:window:control", (event, action) => {
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window) throw new Error(`window action "${action}" has no sender window`);

    switch (action) {
      case "close":
        window.close();
        return null;
      case "minimize":
        window.minimize();
        return null;
      case "toggleMaximize":
        if (window.isMaximized()) window.unmaximize();
        else window.maximize();
        return null;
      case "isMaximized":
        return window.isMaximized();
      case "isFocused":
        return window.isFocused();
      case "setFocus":
        window.focus();
        return null;
      case "scaleFactor":
        return displayScaleFactor(window);
      case "outerPosition": {
        const scale = displayScaleFactor(window);
        const [x, y] = window.getPosition();
        return { x: Math.round(x * scale), y: Math.round(y * scale) };
      }
      default:
        throw new Error(`unknown window action: ${String(action)}`);
    }
  });

  ipcMain.handle("dae:dialog:open", async (event, options) => {
    const parent = BrowserWindow.fromWebContents(event.sender);
    const settings = options && typeof options === "object" ? options : {};
    const properties = settings.directory ? ["openDirectory"] : ["openFile"];
    if (settings.multiple) properties.push("multiSelections");

    const filters = Array.isArray(settings.filters)
      ? settings.filters
          .filter(
            (filter) =>
              filter && typeof filter.name === "string" && Array.isArray(filter.extensions),
          )
          .map((filter) => ({
            name: filter.name,
            extensions: filter.extensions.map((extension) =>
              String(extension).replace(/^\./, ""),
            ),
          }))
      : [];

    const dialogOptions = {
      title: typeof settings.title === "string" ? settings.title : undefined,
      defaultPath:
        typeof settings.defaultPath === "string" ? settings.defaultPath : undefined,
      filters: filters.length > 0 ? filters : undefined,
      properties,
    };
    const result = parent
      ? await dialog.showOpenDialog(parent, dialogOptions)
      : await dialog.showOpenDialog(dialogOptions);

    if (result.canceled || result.filePaths.length === 0) return null;
    return settings.multiple ? result.filePaths : result.filePaths[0];
  });

  ipcMain.handle("dae:opener:open-path", async (_event, target) => {
    if (typeof target !== "string" || target.length === 0) {
      throw new Error("open-path requires a non-empty path");
    }
    const failure = await shell.openPath(target);
    if (failure) throw new Error(failure);
  });

  ipcMain.handle("dae:opener:open-url", async (_event, target) => {
    if (typeof target !== "string") throw new Error("open-url requires a URL");
    const url = new URL(target);
    if (!OPENABLE_PROTOCOLS.has(url.protocol)) {
      throw new Error(`open-url refuses ${url.protocol} URLs`);
    }
    await shell.openExternal(url.toString());
  });

  ipcMain.handle("dae:clipboard:read-text", () => clipboard.readText());

  ipcMain.handle("dae:clipboard:write-text", (_event, text) => {
    if (typeof text !== "string") throw new Error("write-text requires a string");
    clipboard.writeText(text);
  });

  ipcMain.handle("dae:log:write", async (_event, level, message) => {
    if (level !== "warn" && level !== "error") {
      throw new Error(`log level must be warn or error, got ${String(level)}`);
    }
    const line = `${new Date().toISOString()} [${level}] ${String(message)}\n`;
    await fs.promises.appendFile(path.join(logDirectory(), "dae.log"), line, "utf8");
  });

  ipcMain.handle("dae:app:version", () => appVersion());

  ipcMain.handle("dae:app:log-dir", () => logDirectory());
}

function createWindow() {
  const label = "main";
  const window = new BrowserWindow({
    width: 1180,
    height: 820,
    show: true,
    backgroundColor: "#101116",
    title: "dae",
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
      additionalArguments: [`--dae-window-label=${label}`],
    },
  });

  window.daeLabel = label;
  windowsByLabel.set(label, window);
  window.on("closed", () => {
    windowsByLabel.delete(label);
    if (windowsByLabel.size === 0) app.quit();
  });

  window.on("resize", () => {
    sendWindowEvent(window, "tauri://resize", physicalSize(window));
  });
  window.on("focus", () => sendWindowEvent(window, "tauri://focus", true));
  window.on("blur", () => sendWindowEvent(window, "tauri://blur", false));

  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  const rendererOrigin = DEV_SERVER_URL ? new URL(DEV_SERVER_URL).origin : "dae://localhost";
  window.webContents.on("will-navigate", (event, target) => {
    // In-page routing is the renderer's job; only reloads of the app itself are
    // allowed to navigate the main frame.
    if (target.startsWith(rendererOrigin + "/")) return;
    event.preventDefault();
  });
  const contents = window.webContents;
  contents.on("destroyed", () => {
    for (const [id, owner] of channelOwners) {
      if (owner === contents) channelOwners.delete(id);
    }
  });
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

  loadRenderer(window);
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
    registerRendererProtocol();
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
