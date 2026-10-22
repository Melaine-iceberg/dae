// Startup self-update for packaged builds. Mirrors the Tauri shell's
// `spawn_startup_check`: check the release feed off the startup path, install a
// newer build, then restart so the swap takes effect.
//
// Every failure is a log line rather than a startup error: an offline machine,
// an AppImage that is not running through the AppImage launcher, or a macOS
// build without a Developer ID signature must all still open the app.

const fs = require("node:fs");
const path = require("node:path");
const util = require("node:util");

const STARTUP_DELAY_MS = 3000;

// electron-updater hands over errors whose message can include an entire HTTP
// response (the GitHub feed case), so the log keeps the first line only.
function summarize(error) {
  const text = error instanceof Error ? error.message : String(error);
  const firstLine = text.split(/\r?\n/, 1)[0].trim();
  return firstLine.length > 300 ? `${firstLine.slice(0, 300)}…` : firstLine;
}

function createLogger(logDirectory) {
  const write = (level, message) => {
    const raw = typeof message === "string" ? message : util.inspect(message);
    // electron-updater logs whole stacks and HTTP bodies; one capped line per
    // event keeps dae.log readable and the failure identifiable.
    const firstLine = raw.replace(/\s*\r?\n[\s\S]*$/, "").trim();
    const text = firstLine.length > 500 ? `${firstLine.slice(0, 500)}…` : firstLine;
    const line = `${new Date().toISOString()} [updater:${level}] ${text}\n`;
    if (level === "error" || level === "warn") console.error(line.trimEnd());
    else console.log(line.trimEnd());
    try {
      fs.mkdirSync(logDirectory, { recursive: true });
      fs.appendFileSync(path.join(logDirectory, "dae.log"), line, "utf8");
    } catch {
      // The update check must survive a log directory it cannot write.
    }
  };
  return {
    info: (message) => write("info", message),
    warn: (message) => write("warn", message),
    error: (message) => write("error", message),
    // electron-updater's debug stream narrates every request; opt in only when
    // a specific update failure needs that much detail.
    debug: (message) => {
      if (process.env.DAE_UPDATER_DEBUG) write("debug", message);
    },
  };
}

function scheduleStartupUpdateCheck({ app, logDirectory }) {
  const log = createLogger(logDirectory());

  if (!app.isPackaged) return;

  // electron-updater replaces files next to the executable, which an MSIX
  // install does not allow; packaged Windows installs update through the
  // package itself. `process.windowsStore` is only set when the process has
  // package identity.
  if (process.platform === "win32" && process.windowsStore) {
    log.info("running as an MSIX package; skipping the release-feed update check");
    return;
  }

  // Linux builds update only from AppImage. A deb install is owned by the
  // package manager, and the AppImage updater cannot patch anything else.
  if (process.platform === "linux" && !process.env.APPIMAGE) {
    log.info("not an AppImage install; skipping the release-feed update check");
    return;
  }

  let autoUpdater;
  try {
    ({ autoUpdater } = require("electron-updater"));
  } catch (error) {
    log.warn(`electron-updater is unavailable: ${error}`);
    return;
  }

  autoUpdater.logger = log;
  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = true;

  // The promise rejection and the `error` event can carry the same failure;
  // report a repeated message only once. electron-updater's own logger still
  // writes the capped one-line summary above each event.
  let lastError = "";
  let lastErrorAt = 0;
  const reportError = (error) => {
    const message = summarize(error);
    const now = Date.now();
    if (message === lastError && now - lastErrorAt < 5000) return;
    lastError = message;
    lastErrorAt = now;
    log.warn(`update failed: ${message}`);
  };

  autoUpdater.on("checking-for-update", () => log.info("checking the release feed"));
  autoUpdater.on("update-not-available", (info) =>
    log.info(`up to date (${info?.version ?? app.getVersion()})`),
  );
  autoUpdater.on("update-available", (info) =>
    log.info(`update ${info.version} found; downloading`),
  );
  autoUpdater.on("update-downloaded", (info) => {
    log.info(`update ${info.version} downloaded; restarting to install`);
    // `isSilent` keeps the NSIS installer off the screen; macOS ignores it.
    // `isForceRunAfter` brings dae back after the swap.
    try {
      autoUpdater.quitAndInstall(true, true);
    } catch (error) {
      reportError(error);
    }
  });
  autoUpdater.on("error", reportError);

  const timer = setTimeout(() => {
    autoUpdater.checkForUpdates().catch(reportError);
  }, STARTUP_DELAY_MS);
  // A pending update check should not be the reason a closing app stays alive.
  timer.unref?.();
}

module.exports = { scheduleStartupUpdateCheck };
