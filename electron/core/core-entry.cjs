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
    if (url.pathname !== "/thumb") {
      response.writeHead(404);
      response.end();
      return;
    }
    if (url.searchParams.get("token") !== token) {
      response.writeHead(403);
      response.end("forbidden");
      return;
    }

    const filePath = url.searchParams.get("path");
    if (!filePath) {
      response.writeHead(400);
      response.end("missing path");
      return;
    }
    const size = Number(url.searchParams.get("size") || 160);
    const bytes = await native.thumbnail(filePath, size);
    response.writeHead(200, {
      "content-type": "image/png",
      "cache-control": "public, max-age=60",
    });
    response.end(Buffer.from(bytes));
  } catch (error) {
    response.writeHead(404, { "content-type": "text/plain" });
    response.end(String(error?.message || error));
  }
});

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
