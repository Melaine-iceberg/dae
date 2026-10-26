#!/usr/bin/env node
/**
 * Builds `latest-qt.json` from the per-platform Qt update archives.
 *
 * `release.yml` builds one zip per runner and signs it with the Tauri updater
 * key, which writes the minisign signature as base64 into `<zip>.sig`. Tauri's
 * feed format embeds that file verbatim, so this script copies it instead of
 * re-encoding it.
 *
 * The script is deliberately strict: a feed is only written when every
 * expected archive is present, its signature parses, and its layout matches
 * §4.10 of `docs/decisions/engine-qt-quick.md` (Windows/Linux: executable plus
 * `platforms/` at the root; macOS: one `dae.app`). Publishing a feed that
 * points at a missing or malformed artifact turns a silent no-update into a
 * failed install, so it is better to fail the release job here.
 *
 * Usage:
 *   node scripts/qt-update-feed.mjs --dir dist/qt-release [--version 0.6.0]
 *       [--notes-file docs/release-notes/v0.6.0.md] [--notes "..."]
 *       [--pub-date 2026-10-26T00:00:00Z] [--output latest-qt.json]
 */
import {
  closeSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const PLATFORM_KEYS = [
  "windows-x86_64",
  "windows-aarch64",
  "darwin-x86_64",
  "darwin-aarch64",
  "linux-x86_64",
  "linux-aarch64",
];

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const EOCD_SIGNATURE = 0x06054b50;
const CENTRAL_ENTRY_SIGNATURE = 0x02014b50;

const { values } = parseArgs({
  options: {
    dir: { type: "string" },
    version: { type: "string" },
    notes: { type: "string" },
    "notes-file": { type: "string" },
    "pub-date": { type: "string" },
    output: { type: "string" },
    help: { type: "boolean", short: "h" },
  },
});

if (values.help) {
  console.log(
    [
      "Usage: node scripts/qt-update-feed.mjs --dir <artifact-dir> [options]",
      "",
      "Options:",
      "  --version <semver>   Feed version (defaults to package.json)",
      "  --notes <text>       Override release notes",
      "  --notes-file <path>  Markdown notes (defaults to docs/release-notes/v<version>.md)",
      "  --pub-date <iso>     Publication timestamp (defaults to now)",
      "  --output <path>      Output file (defaults to <dir>/latest-qt.json)",
    ].join("\n"),
  );
  process.exit(0);
}

function fail(message) {
  console.error(`qt-update-feed: ${message}`);
  process.exit(1);
}

function stripVersionPrefix(value) {
  return value.trim().replace(/^v/i, "");
}

function resolveVersion() {
  if (values.version) {
    return stripVersionPrefix(values.version);
  }
  const packageJson = JSON.parse(
    readFileSync(path.join(ROOT, "package.json"), "utf8"),
  );
  return stripVersionPrefix(packageJson.version);
}

const version = resolveVersion();
if (!/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
  fail(`"${version}" is not a semver version`);
}

const artifactDir = path.resolve(values.dir ?? path.join(ROOT, "dist", "qt-release"));
if (!existsSync(artifactDir) || !statSync(artifactDir).isDirectory()) {
  fail(`artifact directory does not exist: ${artifactDir}`);
}

const output = path.resolve(
  values.output ?? path.join(artifactDir, "latest-qt.json"),
);

function plainNotes(markdown) {
  const blocks = markdown
    .replace(/```[\s\S]*?```/g, "")
    .split(/\n\s*\n/)
    .map((block) => block.trim())
    .filter(Boolean);
  const paragraph = blocks.find(
    (block) => !block.startsWith("#") && !block.startsWith("---") && !block.startsWith("!["),
  );
  if (!paragraph) {
    return "";
  }
  const plain = paragraph
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^>\s?/gm, "")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/(^|\s)[*_]([^*_]+)[*_](\s|$)/g, "$1$2$3")
    .replace(/\s+/g, " ")
    .trim();
  return plain.length > 1200 ? `${plain.slice(0, 1199).trimEnd()}…` : plain;
}

function resolveNotes() {
  if (values.notes !== undefined) {
    return values.notes.trim();
  }
  const notesFile = path.resolve(
    values["notes-file"] ??
      path.join(ROOT, "docs", "release-notes", `v${version}.md`),
  );
  if (existsSync(notesFile)) {
    const notes = plainNotes(readFileSync(notesFile, "utf8"));
    if (notes) {
      return notes;
    }
  }
  return `dae v${version} 已发布，包含多项改进与修复。`;
}

const pubDate = values["pub-date"] ?? new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
if (Number.isNaN(Date.parse(pubDate))) {
  fail(`"${pubDate}" is not a valid publication timestamp`);
}

/**
 * Reads the zip's central directory without depending on `unzip` being
 * installed on the runner. Qt zips stay far below the Zip64 thresholds, so a
 * plain central-directory read is enough; Zip64 is rejected rather than
 * silently publishing an artifact that cannot be inspected.
 */
function zipEntryNames(zipPath) {
  const fd = openSync(zipPath, "r");
  try {
    const size = statSync(zipPath).size;
    if (size < 22) {
      fail(`${path.basename(zipPath)} is too small to be a zip`);
    }

    const tailLength = Math.min(size, 22 + 0xffff);
    const tail = Buffer.alloc(tailLength);
    readSync(fd, tail, 0, tailLength, size - tailLength);

    let eocd = -1;
    for (let offset = tail.length - 22; offset >= 0; offset -= 1) {
      if (tail.readUInt32LE(offset) !== EOCD_SIGNATURE) {
        continue;
      }
      const commentLength = tail.readUInt16LE(offset + 20);
      if (offset + 22 + commentLength === tail.length) {
        eocd = offset;
        break;
      }
    }
    if (eocd < 0) {
      fail(`${path.basename(zipPath)} has no zip end-of-central-directory record`);
    }

    const disk = tail.readUInt16LE(eocd + 4);
    const centralDisk = tail.readUInt16LE(eocd + 6);
    const entriesOnDisk = tail.readUInt16LE(eocd + 8);
    const entries = tail.readUInt16LE(eocd + 10);
    const centralSize = tail.readUInt32LE(eocd + 12);
    const centralOffset = tail.readUInt32LE(eocd + 16);
    if (
      disk !== 0 ||
      centralDisk !== 0 ||
      entriesOnDisk !== entries ||
      entries === 0xffff ||
      centralSize === 0xffffffff ||
      centralOffset === 0xffffffff
    ) {
      fail(`${path.basename(zipPath)} is a multi-disk or Zip64 archive`);
    }
    if (centralOffset + centralSize > size) {
      fail(`${path.basename(zipPath)} has an out-of-range central directory`);
    }

    const central = Buffer.alloc(centralSize);
    readSync(fd, central, 0, centralSize, centralOffset);

    const names = [];
    let offset = 0;
    for (let index = 0; index < entries; index += 1) {
      if (offset + 46 > central.length) {
        fail(`${path.basename(zipPath)} has a truncated central directory`);
      }
      if (central.readUInt32LE(offset) !== CENTRAL_ENTRY_SIGNATURE) {
        fail(`${path.basename(zipPath)} has a malformed central directory`);
      }
      const nameLength = central.readUInt16LE(offset + 28);
      const extraLength = central.readUInt16LE(offset + 30);
      const commentLength = central.readUInt16LE(offset + 32);
      const nameStart = offset + 46;
      const nameEnd = nameStart + nameLength;
      if (nameEnd > central.length) {
        fail(`${path.basename(zipPath)} has a truncated file name`);
      }
      names.push(central.toString("utf8", nameStart, nameEnd));
      offset = nameEnd + extraLength + commentLength;
    }
    return names;
  } finally {
    closeSync(fd);
  }
}

function validateArchive(zipPath, platformKey) {
  const entries = zipEntryNames(zipPath).map((entry) =>
    entry.replaceAll("\\", "/"),
  );
  const files = entries.filter((entry) => !entry.endsWith("/"));
  if (files.length === 0) {
    fail(`${path.basename(zipPath)} is empty`);
  }

  for (const entry of entries) {
    if (
      entry.startsWith("/") ||
      /^[A-Za-z]:/.test(entry) ||
      entry.split("/").includes("..")
    ) {
      fail(`${path.basename(zipPath)} contains an unsafe path: ${entry}`);
    }
  }

  if (platformKey.startsWith("darwin")) {
    const roots = new Set(
      entries.map((entry) => entry.split("/")[0]).filter(Boolean),
    );
    if (roots.size !== 1 || !roots.has("dae.app")) {
      fail(`${path.basename(zipPath)} must contain exactly one dae.app`);
    }
    if (!files.includes("dae.app/Contents/MacOS/dae")) {
      fail(`${path.basename(zipPath)} is missing dae.app/Contents/MacOS/dae`);
    }
    return;
  }

  const executable = platformKey.startsWith("windows") ? "dae.exe" : "dae";
  if (!files.includes(executable)) {
    fail(`${path.basename(zipPath)} is missing ${executable} at its root`);
  }
  if (!files.some((entry) => entry.startsWith("platforms/"))) {
    fail(`${path.basename(zipPath)} is missing platforms/ at its root`);
  }
}

function readSignature(sigPath) {
  const signature = readFileSync(sigPath, "utf8").trim();
  if (!signature) {
    fail(`${path.basename(sigPath)} is empty`);
  }

  // `tauri signer sign` writes base64; a hand-made minisign `.sig` is the
  // text itself. Both are valid feed values because the app accepts both.
  let text = signature;
  if (!text.startsWith("untrusted comment:")) {
    text = Buffer.from(signature, "base64").toString("utf8");
  }
  const lines = text.split("\n").map((line) => line.trim());
  if (
    !lines[0]?.startsWith("untrusted comment:") ||
    !lines[2]?.startsWith("trusted comment:") ||
    !lines[1] ||
    !lines[3]
  ) {
    fail(`${path.basename(sigPath)} is not a minisign signature`);
  }
  // Keep a normal Tauri `.sig` byte-for-byte, and normalize a raw or wrapped
  // signature file to the same single-line base64 form.
  if (/^[A-Za-z0-9+/]+={0,2}$/.test(signature)) {
    return signature;
  }
  return Buffer.from(lines.join("\n"), "utf8").toString("base64");
}

const platforms = {};
for (const platformKey of PLATFORM_KEYS) {
  const zipName = `dae-${version}-${platformKey}.zip`;
  const zipPath = path.join(artifactDir, zipName);
  const sigPath = `${zipPath}.sig`;
  const hasZip = existsSync(zipPath);
  const hasSignature = existsSync(sigPath);

  if (!hasZip && !hasSignature) {
    continue;
  }
  if (!hasZip || !hasSignature) {
    fail(`${zipName} and ${zipName}.sig must exist together`);
  }

  validateArchive(zipPath, platformKey);
  platforms[platformKey] = {
    url: zipName,
    signature: readSignature(sigPath),
  };
}

if (Object.keys(platforms).length === 0) {
  fail(`no Qt update archives found in ${artifactDir}`);
}

const feed = {
  version,
  notes: resolveNotes(),
  pub_date: pubDate,
  platforms,
};

mkdirSync(path.dirname(output), { recursive: true });
writeFileSync(output, `${JSON.stringify(feed, null, 2)}\n`);
console.log(
  `Wrote ${output} for ${Object.keys(platforms).join(", ")}`,
);
