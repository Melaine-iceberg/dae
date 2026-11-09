#!/usr/bin/env node
// Accessibility audit for the QML surfaces.
//
// Qt Quick does not infer an accessible name from a drawn `Text`: an item is
// announced as an unnamed button unless it (or the element that owns the
// pointer handling) declares `Accessible.name`. That is invisible in a
// screenshot and in `qmllint`, so this script checks it statically.
//
// Rules
//   * Every pointer/keyboard control — `MouseArea`, `TextInput`, `TextEdit`
//     and `TapHandler` — must have an `Accessible.name` on itself or on a
//     container within the next few levels, where a screen reader looks.
//   * `MouseArea { acceptedButtons: Qt.NoButton }` is a wheel/click blocker,
//     not a control, and is skipped.
//   * `Accessible.ignored: true` on the control or on a container within the
//     next few levels takes it out of the accessible tree, which is the honest
//     answer for a scrim, a rubber band or a resize strip: the pointer path
//     exists, the keyboard path does not, and announcing it would only pad the
//     tree with targets that go nowhere. A conditional `ignored` does not
//     count, because the control is still announced in its other state.
//   * `// a11y:decorative` on the line (or the one above) is the comment form
//     of the same statement, for a control that has no `Accessible` block at
//     all — it marks the node purely decorative in prose.
//
// Usage
//   node scripts/a11y-audit.mjs [qml] [--json] [--quiet] [--no-fail]
//   node scripts/a11y-audit.mjs --baseline [--write-baseline]
//
// With `--baseline`, findings already in `scripts/a11y-baseline.json` are
// reported as debt and do not fail the run; a new one does. `--write-baseline`
// records the current findings — use it only to accept a reviewed change, and
// shrink it whenever a surface is fixed.

import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const CONTROLS = new Set(["MouseArea", "TextInput", "TextEdit", "TapHandler"]);
const SKIP_MARKER = "a11y:decorative";
/// How far up a delegate's tree the audit looks for a name. Deeper than a
/// hand-written row ever needs, shallow enough that a name on the window root
/// cannot silence every control inside it.
const MAX_ANCESTORS = 5;

const root = fileURLToPath(new URL("..", import.meta.url));
const BASELINE_FILE = join(root, "scripts", "a11y-baseline.json");

const OPEN_ELEMENT = /^(\s*)(?:[A-Za-z_][\w.]*\s*:\s*)?([A-Z][A-Za-z0-9_.]*)\s*\{/;
const OPEN_COMPONENT = /^(\s*)component\s+[A-Za-z0-9_]+\s*:\s*([A-Z][A-Za-z0-9_.]*)\s*\{/;

function parseArgs(argv) {
  const options = {
    directory: join(root, "qml"),
    json: false,
    quiet: false,
    fail: true,
    baseline: false,
    writeBaseline: false,
  };
  for (const argument of argv) {
    if (argument === "--json") options.json = true;
    else if (argument === "--quiet") options.quiet = true;
    else if (argument === "--no-fail") options.fail = false;
    else if (argument === "--baseline") options.baseline = true;
    else if (argument === "--write-baseline") {
      options.baseline = true;
      options.writeBaseline = true;
    } else if (argument.startsWith("-")) {
      console.error(`unknown option: ${argument}`);
      process.exit(2);
    } else options.directory = argument;
  }
  return options;
}

function qmlFiles(directory) {
  if (statSync(directory).isFile()) return [directory];
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...qmlFiles(path));
    else if (entry.name.endsWith(".qml")) files.push(path);
  }
  return files.sort();
}

/// Braces outside strings and comments. A JS handler (`onClicked: { … }`) has
/// braces of its own, and only counting the real ones keeps the element stack
/// aligned after every multi-line handler.
function braceDelta(line) {
  const cleaned = line
    .replace(/\/\/.*$/, "")
    .replace(/'(?:\\.|[^'\\])*'/g, "''")
    .replace(/"(?:\\.|[^"\\])*"/g, '""');
  let delta = 0;
  for (const character of cleaned) {
    if (character === "{") delta += 1;
    else if (character === "}") delta -= 1;
  }
  return delta;
}

/// The element blocks of one file. `depth` is the brace depth the element
/// opened at, so a handler's `}` closes the handler rather than the element.
function elementBlocks(lines, path) {
  const blocks = [];
  const stack = [];
  let depth = 0;

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const match = OPEN_COMPONENT.exec(line) ?? OPEN_ELEMENT.exec(line);
    if (match) {
      blocks.push({
        indent: match[1].length,
        name: match[2],
        startLine: index,
        endLine: lines.length - 1,
        depth,
        parent: stack.length > 0 ? stack[stack.length - 1].block : -1,
        file: path,
      });
      stack.push({ depth, block: blocks.length - 1 });
    }

    depth += braceDelta(line);
    while (stack.length > 0 && stack[stack.length - 1].depth >= depth) {
      blocks[stack.pop().block].endLine = index;
    }
  }
  return blocks;
}

/// Whether `property` is declared directly by this block. Nested elements are
/// indented one level deeper, so their properties do not count: a child's
/// name must not answer for its parent.
function ownPropertyLine(lines, blocks, index, property) {
  const block = blocks[index];
  if (!block) return null;
  for (let line = block.startLine + 1; line <= block.endLine; line += 1) {
    const text = lines[line];
    const indent = text.length - text.trimStart().length;
    if (text.trim() !== "" && indent <= block.indent) break;
    if (indent > block.indent + 4) continue;
    const body = text.trimStart();
    if (body.startsWith(property)) return body;
  }
  return null;
}

function ownProperty(lines, blocks, index, property) {
  return ownPropertyLine(lines, blocks, index, property) !== null;
}

function hasAccessibleName(lines, blocks, index) {
  let cursor = index;
  let level = 0;
  while (cursor >= 0 && level < MAX_ANCESTORS) {
    if (ownProperty(lines, blocks, cursor, "Accessible.name")) return true;
    cursor = blocks[cursor].parent;
    level += 1;
  }
  return false;
}

/// `Accessible.ignored: true` is a deliberate removal from the tree, not a
/// missing name, so the audit stays quiet about it. An expression — for
/// example `Accessible.ignored: entry.separator` — is still a control in its
/// announced state and does not opt out.
function optedOut(lines, blocks, index) {
  let cursor = index;
  let level = 0;
  while (cursor >= 0 && level < MAX_ANCESTORS) {
    const declaration = ownPropertyLine(lines, blocks, cursor, "Accessible.ignored");
    if (declaration !== null) {
      const value = declaration.replace(/\/\/.*$/, "").trim();
      if (/^Accessible\.ignored:\s*true$/.test(value)) return true;
    }
    cursor = blocks[cursor].parent;
    level += 1;
  }
  return false;
}

function suppressed(lines, index) {
  const line = lines[index] ?? "";
  const previous = index > 0 ? lines[index - 1] : "";
  return line.includes(SKIP_MARKER) || previous.includes(SKIP_MARKER);
}

function auditFile(path) {
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  const blocks = elementBlocks(lines, path);
  const findings = [];

  blocks.forEach((block, index) => {
    if (!CONTROLS.has(block.name)) return;
    if (suppressed(lines, block.startLine)) return;
    // A scroll blocker is not a control: it exists to swallow the wheel.
    if (ownProperty(lines, blocks, index, "acceptedButtons: Qt.NoButton")) return;
    if (optedOut(lines, blocks, index)) return;
    if (hasAccessibleName(lines, blocks, index)) return;
    const parent = block.parent >= 0 ? blocks[block.parent] : null;
    findings.push({
      file: relative(root, path).replaceAll("\\", "/"),
      line: block.startLine + 1,
      element: block.name,
      parent: parent ? parent.name : "(root)",
    });
  });

  // The baseline key is position-independent: a control is identified by its
  // file, its type, the container that owns it and how many of those came
  // before. Adding a line above a known gap must not turn it into a new one.
  const counters = new Map();
  for (const finding of findings) {
    const base = `${finding.file}:${finding.element}:${finding.parent}`;
    const ordinal = counters.get(base) ?? 0;
    counters.set(base, ordinal + 1);
    finding.key = `${base}#${ordinal}`;
  }
  return findings;
}

const keyOf = (finding) => finding.key ?? `${finding.file}:${finding.line}:${finding.element}`;

function readBaseline() {
  if (!existsSync(BASELINE_FILE)) return new Set();
  try {
    const document = JSON.parse(readFileSync(BASELINE_FILE, "utf8"));
    return new Set((document.findings ?? []).map(keyOf));
  } catch (error) {
    console.error(`a11y: cannot read ${relative(root, BASELINE_FILE)}: ${error.message}`);
    process.exit(2);
  }
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const findings = qmlFiles(options.directory).flatMap(auditFile);

  if (options.writeBaseline) {
    writeFileSync(
      BASELINE_FILE,
      `${JSON.stringify({ note: "Known gaps, shrink-only. Regenerate with --write-baseline after a review.", findings }, null, 2)}\n`,
    );
    process.stdout.write(
      `a11y: baseline written with ${findings.length} known finding(s)\n`,
    );
    return;
  }

  const baseline = options.baseline ? readBaseline() : new Set();
  const known = findings.filter((finding) => baseline.has(keyOf(finding)));
  const fresh = findings.filter((finding) => !baseline.has(keyOf(finding)));

  if (options.json) {
    process.stdout.write(
      `${JSON.stringify({ findings, fresh: fresh.length, known: known.length }, null, 2)}\n`,
    );
  } else {
    if (!options.quiet) {
      for (const finding of fresh) {
        process.stdout.write(
          `${finding.file}:${finding.line}  ${finding.element}  ` +
            `— no Accessible.name on it or on <${finding.parent}>\n`,
        );
      }
      if (fresh.length > 0) process.stdout.write("\n");
    }
    const debt = options.baseline ? ` (${known.length} known)` : "";
    process.stdout.write(
      fresh.length === 0
        ? `a11y: no new controls without an accessible name${debt}\n`
        : `a11y: ${fresh.length} control(s) without an accessible name${debt}\n`,
    );
  }

  process.exit(fresh.length > 0 && options.fail ? 1 : 0);
}

main();
