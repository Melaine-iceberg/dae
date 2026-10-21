const { contextBridge, ipcRenderer, webUtils } = require("electron");

const LABEL_PREFIX = "--dae-window-label=";
const labelArgument = Array.isArray(process.argv)
  ? process.argv.find((argument) => argument.startsWith(LABEL_PREFIX))
  : undefined;
const label = labelArgument ? labelArgument.slice(LABEL_PREFIX.length) : "main";

const MATERIAL_PREFIX = "--dae-window-material=";
const materialArgument = Array.isArray(process.argv)
  ? process.argv.find((argument) => argument.startsWith(MATERIAL_PREFIX))
  : undefined;
const windowMaterial = materialArgument
  ? materialArgument.slice(MATERIAL_PREFIX.length)
  : "none";

const POOL_PREFIX = "--dae-window-pool=";
const poolArgument = Array.isArray(process.argv)
  ? process.argv.find((argument) => argument.startsWith(POOL_PREFIX))
  : undefined;
const poolWindow = poolArgument?.slice(POOL_PREFIX.length) === "1" ? 1 : 0;

// The splash reads this global before the bundle evaluates, so it has to be in
// the main world from the preload rather than fetched over IPC.
contextBridge.exposeInMainWorld("__DAE_WINDOW_MATERIAL__", windowMaterial);
// The pool bootstrap reads this before subscribing to adoption events; the
// window must not be handed a tab until it says it is listening.
contextBridge.exposeInMainWorld("__DAE_POOL_WINDOW", poolWindow);

const listeners = new Set();
const channelListeners = new Set();

function dispatch(payload) {
  for (const listener of [...listeners]) {
    try {
      listener(payload);
    } catch (error) {
      console.error("dae event listener failed", error);
    }
  }
}

function dispatchChannel(payload) {
  for (const listener of [...channelListeners]) {
    try {
      listener(payload);
    } catch (error) {
      console.error("dae channel listener failed", error);
    }
  }
}

ipcRenderer.on("dae:event", (_event, payload) => dispatch(payload));
ipcRenderer.on("dae:channel", (_event, payload) => dispatchChannel(payload));

// Window facts the main process reports on the same event bus Tauri used, so
// the facade's Window methods stay ordinary event listeners.
function windowEvent(name, payload) {
  dispatch({
    name,
    target: label,
    payloadJson: JSON.stringify(payload ?? null),
    data: null,
  });
}

function positionOf(event) {
  const scale = window.devicePixelRatio || 1;
  return {
    x: Math.round(event.clientX * scale),
    y: Math.round(event.clientY * scale),
  };
}

function hasFiles(event) {
  return Array.from(event.dataTransfer?.types ?? []).includes("Files");
}

function pathsOf(event) {
  const paths = [];
  for (const item of event.dataTransfer?.items ?? []) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (!file) continue;
    try {
      const path = webUtils.getPathForFile(file);
      if (path) paths.push(path);
    } catch {
      // A File the renderer built itself has no path on disk.
    }
  }
  return paths;
}

// Chromium delivers external file drags through the DOM; Tauri delivered them
// as window events. The depth counter is what distinguishes leaving the window
// from crossing a child element.
let dragDepth = 0;

window.addEventListener("dragenter", (event) => {
  if (!hasFiles(event)) return;
  event.preventDefault();
  dragDepth += 1;
  if (dragDepth === 1) {
    windowEvent("tauri://drag-enter", {
      paths: pathsOf(event),
      position: positionOf(event),
    });
  }
});

window.addEventListener("dragover", (event) => {
  if (!hasFiles(event)) return;
  event.preventDefault();
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = event.ctrlKey ? "copy" : "move";
  }
  windowEvent("tauri://drag-over", { position: positionOf(event) });
});

window.addEventListener("dragleave", (event) => {
  if (dragDepth === 0) return;
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) windowEvent("tauri://drag-leave", null);
});

window.addEventListener("drop", (event) => {
  if (dragDepth === 0 && !hasFiles(event)) return;
  event.preventDefault();
  dragDepth = 0;
  windowEvent("tauri://drag-drop", {
    paths: pathsOf(event),
    position: positionOf(event),
  });
});

contextBridge.exposeInMainWorld("daeBridge", {
  label,
  invokeJson: (command, args) => ipcRenderer.invoke("dae:invoke", command, args),
  invoke: async (command, args) => {
    const json = await ipcRenderer.invoke("dae:invoke", command, args);
    return JSON.parse(json);
  },
  invokeRaw: (command, args) => ipcRenderer.invoke("dae:invokeRaw", command, args),
  onEvent: (listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  onChannelMessage: (listener) => {
    channelListeners.add(listener);
    return () => channelListeners.delete(listener);
  },
  window: {
    control: (action) => ipcRenderer.invoke("dae:window:control", action),
  },
  dialog: {
    open: (options) => ipcRenderer.invoke("dae:dialog:open", options),
  },
  opener: {
    openPath: (path) => ipcRenderer.invoke("dae:opener:open-path", path),
    openUrl: (url) => ipcRenderer.invoke("dae:opener:open-url", url),
  },
  clipboard: {
    readText: () => ipcRenderer.invoke("dae:clipboard:read-text"),
    writeText: (text) => ipcRenderer.invoke("dae:clipboard:write-text", text),
  },
  log: {
    write: (level, message) => ipcRenderer.invoke("dae:log:write", level, message),
  },
  app: {
    version: () => ipcRenderer.invoke("dae:app:version"),
    logDir: () => ipcRenderer.invoke("dae:app:log-dir"),
  },
  resetLag: () => ipcRenderer.invoke("dae:lag:reset"),
  readLag: () => ipcRenderer.invoke("dae:lag:read"),
  info: () => ipcRenderer.invoke("dae:info"),
  benchLog: (message) => ipcRenderer.invoke("dae:bench:log", message),
  benchDone: (result) => ipcRenderer.invoke("dae:bench:done", result),
});
