const { contextBridge, ipcRenderer } = require("electron");

const listeners = new Set();

ipcRenderer.on("dae:event", (_event, payload) => {
  for (const listener of listeners) {
    try {
      listener(payload);
    } catch (error) {
      console.error("dae event listener failed", error);
    }
  }
});

contextBridge.exposeInMainWorld("daeBridge", {
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
  resetLag: () => ipcRenderer.invoke("dae:lag:reset"),
  readLag: () => ipcRenderer.invoke("dae:lag:read"),
  info: () => ipcRenderer.invoke("dae:info"),
  benchLog: (message) => ipcRenderer.invoke("dae:bench:log", message),
  benchDone: (result) => ipcRenderer.invoke("dae:bench:done", result),
});
