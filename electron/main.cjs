const {
  app,
  BrowserWindow,
  clipboard,
  dialog,
  ipcMain,
  nativeImage,
  net,
  protocol,
  screen,
  shell,
  utilityProcess,
} = require("electron");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const OPENABLE_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);
const APP_IDENTIFIER = "com.melaine.dae";
const DEEP_LINK_SCHEME = "dae:";

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
// The system backdrop this shell asked the window to draw. A renderer that is
// told "none" paints its own surface; anything else lets the material show
// through the transparent background.
let windowMaterial = "none";
let windowMaterialDark = false;
// A directory handed to this instance before its renderer was ready. The
// renderer drains this exactly once through `take_pending_open_directory`.
let pendingOpenDirectory = null;
const deferredActivations = [];
// Channel ids are allocated by the renderer, so main remembers which window
// sent them and routes messages back to that window only.
const channelOwners = new Map();

// Tear-off windows: the label counter, the handoff payloads the renderer
// drains on boot, and the one pre-warmed hidden window kept ready in the pool.
let tabWindowSeq = 0;
const tabHandoffs = new Map();
const tabPool = {
  enabled: false,
  label: null,
  window: null,
  state: "none",
  readyTimer: null,
  replenishTimer: null,
};
let tabDragSession = null;

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

function nativeAddonPath() {
  return path.join(appRoot(), "native", "dae_core_native.node");
}

// Windows drag-out has to run on this process's UI thread, so main loads the
// addon itself for `startFileDrag`; the core process keeps its own copy for
// everything else.
let nativeAddon = null;

function nativeAddonModule() {
  if (!nativeAddon) {
    const addonPath = nativeAddonPath();
    if (!fs.existsSync(addonPath)) {
      throw new Error(`native addon is missing at ${addonPath}; run \`bun run native:build\``);
    }
    nativeAddon = require(addonPath);
  }
  return nativeAddon;
}

let dragPreviewImage = null;

function dragPreview() {
  if (!dragPreviewImage) {
    dragPreviewImage = nativeImage.createFromPath(
      path.join(appRoot(), "src-tauri", "icons", "32x32.png"),
    );
  }
  return dragPreviewImage;
}

// Tauri derived the config directory as `<platform config root>/<identifier>`;
// keeping that exact path means a migrated install still reads its
// settings.toml and workspace stores.
function appConfigDir() {
  return path.join(app.getPath("appData"), APP_IDENTIFIER);
}

function readRegistryDword(key, name) {
  try {
    const { execFileSync } = require("node:child_process");
    const output = execFileSync("reg", ["query", key, "/v", name], {
      windowsHide: true,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
    const match = output.match(new RegExp(`${name}\\s+REG_DWORD\\s+0x([0-9a-fA-F]+)`));
    return match ? Number.parseInt(match[1], 16) : null;
  } catch {
    return null;
  }
}

function windowsBuildNumber() {
  // Electron is manifested for Windows 10, so `os.release()` keeps reporting
  // 10.0.<build> on Windows 11 — the build is the part that matters.
  const build = Number.parseInt(os.release().split(".")[2] ?? "", 10);
  return Number.isFinite(build) ? build : 0;
}

function windowsTransparencyEnabled() {
  const enabled = readRegistryDword(
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
    "EnableTransparency",
  );
  // An absent value means the default: effects on.
  return enabled === null || enabled !== 0;
}

// The same ladder as the Tauri shell, limited to what Electron can ask DWM for:
// the system backdrop API exists from Windows 11 22H2 (build 22621) onwards.
function detectWindowMaterial() {
  if (process.platform === "win32") {
    return windowsBuildNumber() >= 22621 && windowsTransparencyEnabled() ? "acrylic" : "none";
  }
  if (process.platform === "darwin") {
    return "vibrancy";
  }
  if (process.platform === "linux") {
    // Only niri lets a user rule blur a transparent window; elsewhere a
    // transparent window would show the raw desktop.
    const desktop = (process.env.XDG_CURRENT_DESKTOP ?? "").toLowerCase();
    return desktop.includes("niri") || process.env.NIRI_SOCKET ? "blur" : "none";
  }
  return "none";
}

function windowMaterialSupported() {
  return windowMaterial !== "none";
}

function parseOpenDirectory(raw) {
  let url;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol !== DEEP_LINK_SCHEME) return null;

  let target = url.searchParams.get("path");
  if (target === null) {
    const encoded = url.pathname.replace(/^\//, "");
    if (encoded.length === 0) return null;
    try {
      target = decodeURIComponent(encoded);
    } catch {
      return null;
    }
  }

  return resolveExistingDirectory(target);
}

function resolveExistingDirectory(target) {
  try {
    const stat = fs.statSync(target);
    return stat.isDirectory() ? target : path.dirname(target);
  } catch {
    return null;
  }
}

function rawDirectoryArg(argument) {
  try {
    return fs.statSync(argument).isDirectory() ? argument : null;
  } catch {
    return null;
  }
}

function focusMainWindow() {
  const window = windowsByLabel.get("main");
  if (!window || window.isDestroyed()) return;
  if (window.isMinimized()) window.restore();
  window.show();
  window.focus();
}

function handleActivation(args) {
  for (const argument of args) {
    if (typeof argument !== "string" || argument.length === 0) continue;
    const directory = parseOpenDirectory(argument) ?? rawDirectoryArg(argument);
    if (!directory) continue;

    // Buffer first: a link that lands before the renderer subscribes to the
    // event bus survives until `take_pending_open_directory` drains it.
    pendingOpenDirectory = directory;
    broadcastEvent({
      name: "deep-link-open-directory",
      payloadJson: JSON.stringify(directory),
    });
  }

  focusMainWindow();
}

function queueActivation(args) {
  if (windowsByLabel.size === 0) {
    deferredActivations.push(args);
    return;
  }
  handleActivation(args);
}

function flushDeferredActivations() {
  while (deferredActivations.length > 0) {
    handleActivation(deferredActivations.shift());
  }
}

function hwndValue(window) {
  const handle = window.getNativeWindowHandle();
  return handle.length >= 8
    ? Number(handle.readBigUInt64LE(0))
    : handle.readUInt32LE(0);
}

function applyWindowAppearance(window, dark) {
  windowMaterialDark = dark;
  if (process.platform !== "win32" || !coreChild) return;
  try {
    coreChild.postMessage({ type: "window-appearance", hwnd: hwndValue(window), dark });
  } catch (error) {
    console.error(`[main] window appearance failed: ${error}`);
  }
}

// DWM stops compositing a backdrop when the user turns transparency effects
// off. The renderer has to hear about it so its canvas goes opaque, and the
// window's own background has to cover the now-unpainted plate.
function watchWindowBackdrop() {
  if (watchWindowBackdrop.timer) return;
  watchWindowBackdrop.timer = setInterval(() => {
    const next = detectWindowMaterial();
    if (next === windowMaterial) return;
    windowMaterial = next;

    for (const window of windowsByLabel.values()) {
      if (window.isDestroyed()) continue;
      try {
        if (process.platform === "win32") window.setBackgroundMaterial(next);
        window.setBackgroundColor(next === "none" ? "#101116" : "#00000000");
      } catch (error) {
        console.error(`[main] backdrop update failed: ${error}`);
      }
    }

    broadcastEvent({
      name: "window-material-changed",
      payloadJson: JSON.stringify(next),
    });
  }, 5000);
  watchWindowBackdrop.timer.unref?.();
}

function spawnCore() {
  const nativePath = nativeAddonPath();
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
        DAE_CONFIG_DIR: appConfigDir(),
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

// `coreRequest` hands back the JSON text the renderer bridge would parse; main
// callers that need the value itself go through this.
async function coreRequestJson(command, args, options) {
  const data = await coreRequest(command, args, options);
  return typeof data === "string" ? JSON.parse(data) : data;
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

function liveWindow(label) {
  const window = windowsByLabel.get(label);
  return window && !window.isDestroyed() ? window : null;
}

function nextTabLabel() {
  tabWindowSeq += 1;
  return `tab-${tabWindowSeq}`;
}

// Pointer coordinates cross the IPC boundary in physical pixels (the drag
// gesture reads Tauri's coordinate space); Electron windows and `screen` speak
// DIP, so the conversion happens here and nowhere else.
function cursorDip(cursorX, cursorY) {
  if (Number.isFinite(cursorX) && Number.isFinite(cursorY)) {
    const point = { x: Number(cursorX), y: Number(cursorY) };
    const scale = screen.getDisplayNearestPoint(point).scaleFactor;
    return { x: point.x / scale, y: point.y / scale };
  }
  return screen.getCursorScreenPoint();
}

function tabWindowBounds(sourceWindow, grabX, grabY, cursorX, cursorY) {
  const [width, height] = sourceWindow.getContentSize();
  const cursor = cursorDip(cursorX, cursorY);
  const display = screen.getDisplayNearestPoint(cursor);
  const area = display.workArea;
  const x = Math.min(
    Math.max(Math.round(cursor.x - (Number(grabX) || 0)), area.x),
    area.x + Math.max(area.width - width, 0),
  );
  const y = Math.min(
    Math.max(Math.round(cursor.y - (Number(grabY) || 0)), area.y),
    area.y + Math.max(area.height - height, 0),
  );
  return { x, y, width, height };
}

function cursorInWindow(cursor, window, { content = true } = {}) {
  const bounds = content ? window.getContentBounds() : window.getBounds();
  const x = cursor.x - bounds.x;
  const y = cursor.y - bounds.y;
  return {
    x,
    y,
    inside: x >= 0 && y >= 0 && x < bounds.width && y < bounds.height,
  };
}

function windowAt(cursor, excludeLabel) {
  let hit = null;
  for (const window of windowsByLabel.values()) {
    if (window.isDestroyed() || !window.isVisible()) continue;
    if (window.daeLabel === excludeLabel) continue;
    if (window.daePool) continue;
    if (!cursorInWindow(cursor, window).inside) continue;
    // Later windows sit above earlier ones; keeping the last hit approximates
    // Electron's z-order without reaching for a native hit test.
    hit = window;
  }
  return hit;
}

function clearTabPoolReferences(label) {
  if (tabPool.label !== label) return;
  if (tabPool.readyTimer) clearTimeout(tabPool.readyTimer);
  if (tabPool.replenishTimer) clearTimeout(tabPool.replenishTimer);
  tabPool.label = null;
  tabPool.window = null;
  tabPool.state = "none";
  tabPool.readyTimer = null;
  tabPool.replenishTimer = null;
}

function disposeTabPool() {
  const window = tabPool.window;
  clearTabPoolReferences(tabPool.label);
  if (window && !window.isDestroyed()) {
    tabPool.window = null;
    window.close();
  }
}

function primeTabPool() {
  if (!tabPool.enabled || tabPool.window || shuttingDown) return;
  const label = nextTabLabel();
  const window = createWindow({ label, pool: true });
  tabPool.label = label;
  tabPool.window = window;
  tabPool.state = "booting";
  tabPool.readyTimer = setTimeout(() => {
    // A pool window that never reported ready would otherwise sit around
    // invisible forever; tear-off would never get its fast path.
    if (tabPool.state === "booting") disposeTabPool();
  }, 15_000);
  tabPool.readyTimer.unref?.();
}

function scheduleTabPoolReplenish() {
  if (!tabPool.enabled || tabPool.window || shuttingDown) return;
  if (tabPool.replenishTimer) clearTimeout(tabPool.replenishTimer);
  tabPool.replenishTimer = setTimeout(() => {
    tabPool.replenishTimer = null;
    primeTabPool();
  }, 800);
  tabPool.replenishTimer.unref?.();
}

function adoptPoolWindow(bounds, payload) {
  const window = tabPool.window;
  const label = tabPool.label;
  clearTabPoolReferences(label);
  tabPool.window = null;

  window.daePool = false;
  window.setSkipTaskbar(false);
  window.setBounds(bounds);
  window.show();
  window.focus();
  sendWindowEvent(window, "tab-adopted-into-window", { payload });
  scheduleTabPoolReplenish();
  return label;
}

function tearOffTab(args) {
  const source = liveWindow(String(args?.source ?? "main")) ?? [...windowsByLabel.values()].find(
    (window) => !window.isDestroyed() && !window.daePool,
  );
  if (!source) throw new Error("tear_off_tab has no source window");

  const payload = String(args?.payload ?? "");
  const bounds = tabWindowBounds(
    source,
    args?.grabX,
    args?.grabY,
    args?.cursorX,
    args?.cursorY,
  );

  if (tabPool.state === "ready" && tabPool.window && !tabPool.window.isDestroyed()) {
    return adoptPoolWindow(bounds, payload);
  }

  const label = nextTabLabel();
  tabHandoffs.set(label, payload);
  createWindow({ label, bounds });
  scheduleTabPoolReplenish();
  return label;
}

function mergeTabIntoWindow(args) {
  const window = liveWindow(String(args?.target ?? ""));
  if (!window) throw new Error(`merge_tab_into_window: no window ${String(args?.target)}`);
  sendWindowEvent(window, "tab-merged-into-window", {
    payload: args?.payload ?? null,
    x: typeof args?.x === "number" ? args.x : null,
  });
  if (window.isMinimized()) window.restore();
  window.show();
  window.focus();
}

async function startTabDrag(args) {
  const source = liveWindow(String(args?.source ?? "main"));
  if (!source) return { released: false, outside: false, target: null, targetX: null, targetY: null, cursorX: null, cursorY: null };

  // Linux has no process-wide pointer-button state exposed to Electron. The
  // frontend-bounds path already knows the gesture was released outside, so
  // settle it with a detach instead of holding a session that cannot end.
  if (process.platform === "linux") {
    const cursor = screen.getCursorScreenPoint();
    const scale = displayScaleFactor(source);
    return {
      released: true,
      outside: true,
      target: null,
      targetX: null,
      targetY: null,
      cursorX: Math.round(cursor.x * scale),
      cursorY: Math.round(cursor.y * scale),
    };
  }

  if (tabDragSession) {
    clearInterval(tabDragSession.timer);
    tabDragSession = null;
  }

  const session = { source, hover: null, timer: null };
  const pollHover = () => {
    if (source.isDestroyed()) return;
    const cursor = screen.getCursorScreenPoint();
    const target = windowAt(cursor, source.daeLabel);
    if (target === session.hover) return;

    if (session.hover && !session.hover.isDestroyed()) {
      sendWindowEvent(session.hover, "tab-drag-leave", null);
    }
    session.hover = target;
    if (target) {
      const point = cursorInWindow(cursor, target);
      sendWindowEvent(target, "tab-drag-hover", { x: point.x });
    }
  };

  pollHover();
  session.timer = setInterval(pollHover, 60);
  tabDragSession = session;

  let released = false;
  try {
    released = await coreRequestJson("wait_for_tab_drag_release", { timeoutMs: 60_000 }, {
      timeoutMs: 65_000,
    });
  } catch (error) {
    console.error(`[main] tab drag wait failed: ${error}`);
  } finally {
    clearInterval(session.timer);
    if (tabDragSession === session) tabDragSession = null;
    if (session.hover && !session.hover.isDestroyed()) {
      sendWindowEvent(session.hover, "tab-drag-leave", null);
    }
  }

  const cursor = screen.getCursorScreenPoint();
  const scale = displayScaleFactor(source);
  const cursorX = Math.round(cursor.x * scale);
  const cursorY = Math.round(cursor.y * scale);

  if (!released) {
    return {
      released: false,
      outside: false,
      target: null,
      targetX: null,
      targetY: null,
      cursorX,
      cursorY,
    };
  }

  if (cursorInWindow(cursor, source, { content: false }).inside) {
    return {
      released: true,
      outside: false,
      target: null,
      targetX: null,
      targetY: null,
      cursorX,
      cursorY,
    };
  }

  const target = windowAt(cursor, source.daeLabel);
  const point = target ? cursorInWindow(cursor, target) : null;
  return {
    released: true,
    outside: true,
    target: target ? target.daeLabel : null,
    targetX: point ? point.x : null,
    targetY: point ? point.y : null,
    cursorX,
    cursorY,
  };
}

async function syncTabPool() {
  let settings = null;
  try {
    settings = await coreRequestJson("load_settings", null);
  } catch (error) {
    console.error(`[main] tab pool setting read failed: ${error}`);
  }
  tabPool.enabled = Boolean(settings?.warmTabPool);
  if (tabPool.enabled) primeTabPool();
  else disposeTabPool();
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

    // Shell-owned commands never cross the process boundary: the Electron
    // main process is the one that knows the window's backdrop and the deep
    // links this instance has been handed. Their replies are JSON-encoded
    // exactly like the core's, because the preload parses every reply.
    if (command === "take_pending_open_directory") {
      const directory = pendingOpenDirectory;
      pendingOpenDirectory = null;
      return JSON.stringify(directory);
    }
    if (command === "get_window_material") return JSON.stringify(windowMaterial);
    if (command === "set_window_material_appearance") {
      const window = BrowserWindow.fromWebContents(event.sender);
      if (!window) throw new Error("set_window_material_appearance has no sender window");
      if (typeof args?.dark !== "boolean") {
        throw new Error("set_window_material_appearance requires a boolean dark");
      }
      applyWindowAppearance(window, args.dark);
      return "null";
    }

    // Windows drag-out has to run on this process's UI thread, so the OLE
    // gesture is started here rather than in the core process.
    if (command === "start_drag_out") {
      const window = BrowserWindow.fromWebContents(event.sender);
      if (!window) throw new Error("start_drag_out has no sender window");

      const paths = Array.isArray(args?.paths)
        ? args.paths.filter((item) => typeof item === "string" && item.length > 0)
        : [];
      if (paths.length === 0) {
        throw new Error("start_drag_out requires at least one path");
      }

      const mode = typeof args?.mode === "string" ? args.mode : "any";
      if (process.platform === "win32") {
        // DoDragDrop must run on the window's own thread; the addon keeps the
        // requested copy/move/link effect in its OLE data object.
        nativeAddonModule().startFileDrag(paths, mode, hwndValue(window));
        return "null";
      }

      // Electron's startDrag carries a file list and a preview but no effect
      // mask, so macOS and Linux leave the operation to the drop target.
      const icon = dragPreview();
      if (icon.isEmpty()) throw new Error("start_drag_out has no drag preview icon");
      event.sender.startDrag({ files: paths, icon });
      return "null";
    }
    // Tab tear-off owns window creation, which only the main process can do.
    // Labels and handoffs stay here so a window can ask for its payload exactly
    // once, after its renderer is listening.
    if (command === "tear_off_tab") return JSON.stringify(tearOffTab(args));
    if (command === "take_tab_handoff") {
      const label = String(args?.label ?? "");
      const payload = tabHandoffs.get(label) ?? null;
      tabHandoffs.delete(label);
      return JSON.stringify(payload);
    }
    if (command === "pool_window_ready") {
      const label = String(args?.label ?? "");
      if (label && label === tabPool.label && tabPool.state === "booting") {
        if (tabPool.readyTimer) clearTimeout(tabPool.readyTimer);
        tabPool.readyTimer = null;
        tabPool.state = "ready";
      }
      return "null";
    }
    if (command === "sync_tab_pool") {
      await syncTabPool();
      return "null";
    }
    if (command === "merge_tab_into_window") {
      mergeTabIntoWindow(args);
      return "null";
    }
    if (command === "start_tab_drag") return JSON.stringify(await startTabDrag(args));
    if (command === "tab_drag_outside") {
      const window = liveWindow(String(args?.source ?? ""));
      if (!window) return JSON.stringify(true);
      const bounds = window.getBounds();
      const cursor = screen.getCursorScreenPoint();
      const inside =
        cursor.x >= bounds.x &&
        cursor.y >= bounds.y &&
        cursor.x < bounds.x + bounds.width &&
        cursor.y < bounds.y + bounds.height;
      return JSON.stringify(!inside);
    }
    if (command === "tab_drag_uses_frontend_bounds") {
      return JSON.stringify(process.platform === "linux");
    }
    if (command === "tab_drag_hover_ack") return "null";

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

function createWindow({
  label = "main",
  bounds = null,
  pool = false,
} = {}) {
  windowMaterial = detectWindowMaterial();

  const options = {
    width: bounds?.width ?? 1180,
    height: bounds?.height ?? 820,
    x: bounds?.x,
    y: bounds?.y,
    show: !pool,
    // A backdrop only exists if the window itself is transparent; otherwise
    // CSS would fade the canvas over an opaque plate.
    transparent: windowMaterialSupported(),
    backgroundColor: windowMaterialSupported() ? "#00000000" : "#101116",
    title: "dae",
    skipTaskbar: pool,
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: false,
      additionalArguments: [
        `--dae-window-label=${label}`,
        `--dae-window-material=${windowMaterial}`,
        `--dae-window-pool=${pool ? 1 : 0}`,
      ],
    },
  };

  if (process.platform === "win32" && windowMaterialSupported()) {
    options.backgroundMaterial = windowMaterial;
  }
  if (process.platform === "darwin") {
    options.vibrancy = "sidebar";
    options.visualEffectState = "followWindow";
  }

  const window = new BrowserWindow(options);

  window.daeLabel = label;
  window.daePool = pool;
  windowsByLabel.set(label, window);
  window.on("closed", () => {
    windowsByLabel.delete(label);
    tabHandoffs.delete(label);
    if (pool) clearTabPoolReferences(label);
    if (label === "main") disposeTabPool();
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
  window.webContents.on("did-finish-load", () => {
    console.log("[renderer] did-finish-load");
    if (label === "main") flushDeferredActivations();
  });
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
  return window;
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

// A second launch forwards its arguments (including `dae://` URLs) to the
// running instance instead of starting a second shell.
if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on("second-instance", (_event, argv) => queueActivation(argv));
  // macOS delivers deep links through open-url rather than argv.
  app.on("open-url", (event, url) => {
    event.preventDefault();
    queueActivation([url]);
  });

  app.whenReady().then(async () => {
    try {
      if (app.isPackaged) app.setAsDefaultProtocolClient("dae");
      await spawnCore();
      registerIpc();
      registerCoreProtocol();
      registerRendererProtocol();
      createWindow();
      watchWindowBackdrop();
      // Keep the tear-off fast path warm from launch, matching the Tauri shell's
      // startup priming rather than waiting for the first settings save.
      void syncTabPool();
      // The launch's own command line is the first activation (Windows/Linux).
      handleActivation(process.argv);
    } catch (error) {
      console.error(`[main] startup failed: ${error.stack || error}`);
      exitCode = 1;
      app.quit();
    }
  });
}

app.on("window-all-closed", () => {
  app.quit();
});
