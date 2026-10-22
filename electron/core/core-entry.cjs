const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");

const nativePath = process.env.DAE_NATIVE_PATH;
const logFile = process.env.DAE_LOG_FILE;

function log(...parts) {
  const line = `[core] ${new Date().toISOString()} ${parts.join(" ")}`;
  if (logFile) {
    try {
      fs.mkdirSync(path.dirname(logFile), { recursive: true });
      fs.appendFileSync(logFile, `${line}\n`);
    } catch {
      // Logging must never take the core down.
    }
  }
  console.error(line);
}

const parentPort = process.parentPort;

function post(message) {
  try {
    parentPort.postMessage(message);
  } catch (error) {
    log("postMessage failed:", error?.message || error);
  }
}

let native;
try {
  native = require(nativePath);
  log(`loaded ${nativePath}`);
  native.initialize();
  log("platform observers started");
} catch (error) {
  log("unable to load native addon:", error?.stack || error);
  post({ type: "fatal", message: `unable to load native addon: ${error?.message || error}` });
  setTimeout(() => process.exit(1), 100);
}

const token = crypto.randomBytes(24).toString("hex");
let running = true;

function normalizeEvent(event) {
  return {
    name: event.name,
    target: event.target ?? null,
    payloadJson: event.payloadJson ?? event.payload_json ?? null,
    data: event.data ?? null,
    channelId: event.channelId ?? event.channel_id ?? null,
  };
}

async function pumpEvents() {
  while (running) {
    let events;
    try {
      events = await native.nextEvents(64);
    } catch (error) {
      log("nextEvents failed:", error?.message || error);
      post({ type: "fatal", message: `event pump failed: ${error?.message || error}` });
      return;
    }
    if (!running) return;
    for (const raw of events) {
      const event = normalizeEvent(raw);
      if (event.channelId != null) {
        post({
          type: "channel",
          channelId: event.channelId,
          payloadJson: event.payloadJson,
          data: event.data,
        });
      } else {
        post({ type: "event", event });
      }
    }
  }
}

const server = http.createServer(async (request, response) => {
  try {
    const url = new URL(request.url, "http://127.0.0.1");
    if (url.pathname === "/health") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify({ ok: true }));
      return;
    }
    if (url.pathname !== "/thumb" && url.pathname !== "/icon") {
      response.writeHead(404);
      response.end();
      return;
    }
    if (url.searchParams.get("token") !== token) {
      response.writeHead(403);
      response.end("forbidden");
      return;
    }

    const rendered =
      url.pathname === "/thumb"
        ? await thumbnailFor(url.searchParams)
        : await iconFor(url.searchParams);
    if (!rendered) {
      // A type with no producer, or a path that went away. The frontend's
      // `<img onError>` falls back to its drawn glyph; this is not an error.
      response.writeHead(404, { "cache-control": "no-store" });
      response.end();
      return;
    }

    response.writeHead(200, {
      "content-type": rendered.mime,
      // Every URL is versioned by what it is an icon *of* — an entry's mtime
      // and size, or a type and a size — so a given URL never changes.
      "cache-control": "public, max-age=86400, immutable",
    });
    response.end(Buffer.from(rendered.bytes));
  } catch (error) {
    // A malformed query is the caller's fault; a thrown render is a broken
    // read or a producer failure.
    const status = error?.status ?? 500;
    response.writeHead(status, { "content-type": "text/plain" });
    response.end(String(error?.message || error));
  }
});

async function thumbnailFor(params) {
  const filePath = params.get("path");
  if (!filePath) throw badRequest("missing path");

  const size = Number(params.get("size") || 160);
  return native.thumbnail(filePath, Number.isFinite(size) ? size : 160);
}

async function iconFor(params) {
  const filePath = params.get("path");
  const extension = params.get("ext");
  const name = params.get("name");
  const size = Number(params.get("size") || 32);
  const edge = Number.isFinite(size) ? size : 32;

  // Most specific first, the same order the Tauri protocol parser used: an
  // entry outranks a type, and a type outranks a theme name.
  if (filePath) return native.fileIcon(filePath, params.get("dir") === "1", edge);
  if (extension) return native.typeIcon(extension, edge);
  if (name) return native.namedIcon(name, edge);
  throw badRequest("missing path, ext or name");
}

function badRequest(message) {
  const error = new Error(message);
  error.status = 400;
  return error;
}

async function handleRequest(message) {
  const args = message.args ?? null;
  try {
    const data = message.raw
      ? await native.invokeRaw(message.command, JSON.stringify(args))
      : await native.invoke(message.command, JSON.stringify(args));
    post({ type: "response", id: message.id, ok: true, data });
  } catch (error) {
    post({
      type: "response",
      id: message.id,
      ok: false,
      error: String(error?.message || error),
    });
  }
}

parentPort.on("message", (event) => {
  const message = event?.data;
  if (!message || typeof message !== "object") return;

  if (message.type === "request") {
    void handleRequest(message);
    return;
  }

  if (message.type === "window-appearance") {
    try {
      native.setWindowAppearance(message.hwnd, Boolean(message.dark));
    } catch (error) {
      log("window appearance failed:", error?.message || error);
    }
    return;
  }

  if (message.type === "shutdown") {
    running = false;
    try {
      native.shutdown();
    } catch (error) {
      log("shutdown failed:", error?.message || error);
    }
    server.close();
    post({ type: "bye" });
    setTimeout(() => process.exit(0), 50);
  }
});

process.on("uncaughtException", (error) => {
  log("uncaught exception:", error?.stack || error);
  post({ type: "fatal", message: error?.message || String(error) });
});

server.listen(0, "127.0.0.1", () => {
  const { port } = server.address();
  log(`http listening on 127.0.0.1:${port}`);
  post({ type: "ready", port, token, nativePath });
  void pumpEvents();
});
