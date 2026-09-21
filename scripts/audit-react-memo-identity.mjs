/**
 * Which of React Compiler's memo guards survive a fresh object?
 *
 * React Compiler is enabled for production builds (`vite.config.ts`), and it
 * keys many of a component's memo guards on the *identity* of the objects passed
 * to it — the closures that capture a prop are guarded on the prop, not on the
 * fields read off it. So "what happens if this prop is a new object carrying
 * equal values?" is a live question in this codebase, not a hypothetical: it is
 * what decides whether a component re-renders wholesale or bails out.
 *
 * This runs the real plugin, at the version the build uses, over the real
 * sources, then does the dependency propagation by hand:
 *
 *   - a guard comparing a bare `entry`, a closure derived from `entry`, or any
 *     value produced by another failing guard, fails;
 *   - a guard comparing `entry.name` / `entry.path` / `entry.size` holds, since
 *     those are value reads and the values are equal;
 *   - a failing block re-creates the JSX elements inside it, and handing React
 *     the same element objects back is what lets it bail out of a subtree.
 *
 * The element counts it reports are what `scripts/check-entry-codec-parity.ts`
 * quotes when it explains why `ListingView` caches row objects. Run this to
 * re-derive them rather than trust them:
 *
 *   bun scripts/audit-react-memo-identity.mjs
 *
 * What it does NOT measure: React's per-element reconciliation cost, and what a
 * genuinely changed prop costs. Only the guard outcomes and the element counts.
 */
import { parse } from "@babel/parser";
import { transformFileSync } from "@babel/core";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..");

/**
 * The components on the explorer's render path, and the prop whose identity is
 * in question. Add a row to audit another component the same way.
 */
const targets = [
  { file: "src/features/explorer/file-list.tsx", component: "FileListRow", prop: "entry" },
  { file: "src/features/explorer/file-grid-view.tsx", component: "GridCell", prop: "entry" },
  { file: "src/features/explorer/file-column-view.tsx", component: "PaneRow", prop: "entry" },
];

/** The compiler output, JSX left intact so element counts are visible. */
function compile(file, prop) {
  const path = resolve(root, file);
  return transformFileSync(path, {
    babelrc: false,
    configFile: false,
    filename: path,
    parserOpts: { plugins: ["jsx", "typescript"], sourceType: "module" },
    plugins: [[resolve(root, "node_modules/babel-plugin-react-compiler"), {}]],
    generatorOpts: { comments: false },
  }).code;
}

/** True for the `if ($[n] !== X || $[m] !== Y …)` shape the compiler emits. */
function isGuardTest(node) {
  let cursor = node;
  while (cursor?.type === "LogicalExpression" && cursor.operator === "||") {
    cursor = cursor.left;
  }
  if (cursor?.type !== "BinaryExpression" || cursor.operator !== "!==") return false;

  const left = cursor.left;
  return (
    left?.type === "MemberExpression" &&
    left.object?.type === "Identifier" &&
    left.object.name === "$"
  );
}

/** `[$[58], entry.path]`, `[$[59], handleSelect]` … */
function guardDeps(test) {
  const deps = [];
  const walk = (node) => {
    if (!node || typeof node !== "object") return;
    if (node.type === "BinaryExpression" && node.operator === "!==") {
      const left = node.left;
      if (
        left?.type === "MemberExpression" &&
        left.object?.type === "Identifier" &&
        left.object.name === "$"
      ) {
        deps.push(node.right);
        return;
      }
      walk(node.left);
      walk(node.right);
      return;
    }
    if (node.type === "LogicalExpression") {
      walk(node.left);
      walk(node.right);
    }
  };
  walk(test);
  return deps;
}

/** Every identifier assigned inside a block, memo slots excluded. */
function boundNames(consequent) {
  const names = [];
  const walk = (node) => {
    if (!node || typeof node !== "object") return;
    if (node.type === "AssignmentExpression") {
      if (node.left.type === "Identifier") names.push(node.left.name);
      walk(node.right);
      return;
    }
    if (node.type === "VariableDeclarator") {
      if (node.id.type === "Identifier") names.push(node.id.name);
      walk(node.init);
      return;
    }
    for (const key of Object.keys(node)) {
      if (key === "loc" || key === "range" || key === "start" || key === "end") continue;
      const value = node[key];
      if (Array.isArray(value)) value.forEach(walk);
      else if (value && typeof value === "object" && value.type) walk(value);
    }
  };
  walk(consequent);
  return names;
}

function jsxCount(node) {
  let count = 0;
  const walk = (child) => {
    if (!child || typeof child !== "object") return;
    if (child.type === "JSXElement" || child.type === "JSXFragment") count += 1;
    for (const key of Object.keys(child)) {
      if (key === "loc" || key === "range" || key === "start" || key === "end") continue;
      const value = child[key];
      if (Array.isArray(value)) value.forEach(walk);
      else if (value && typeof value === "object" && value.type) walk(value);
    }
  };
  walk(node);
  return count;
}

function collectStatements(node, into) {
  if (!node || typeof node !== "object") return;
  if (node.type === "IfStatement" && isGuardTest(node.test)) into.push(node);
  for (const key of ["body", "consequent", "alternate"]) {
    const value = node[key];
    if (Array.isArray(value)) value.forEach((child) => collectStatements(child, into));
    else if (value && typeof value === "object" && value.type) collectStatements(value, into);
  }
}

function collectLocals(node, into) {
  if (!node || typeof node !== "object") return;
  if (node.type === "VariableDeclarator" && node.id.type === "Identifier" && node.init) {
    into.set(node.id.name, node.init);
  }
  for (const key of ["body", "consequent", "alternate", "declarations"]) {
    const value = node[key];
    if (Array.isArray(value)) value.forEach((child) => collectLocals(child, into));
    else if (value && typeof value === "object" && value.type) collectLocals(value, into);
  }
}

/**
 * Why a dependency does or does not hold when `prop` is a fresh object with the
 * same values. Returns `undefined` when the dependency holds.
 */
function classify(node, context) {
  if (!node) return undefined;

  if (node.type === "Identifier") {
    if (node.name === context.prop) return `\`${context.prop}\` itself`;

    const guardIndex = context.producerOf.get(node.name);
    if (guardIndex !== undefined) {
      return context.failed.has(guardIndex)
        ? `\`${node.name}\` was built by failing guard #${guardIndex}`
        : undefined;
    }

    const init = context.localInit.get(node.name);
    if (init && !context.seen.has(node.name)) {
      context.seen.add(node.name);
      const nested = classify(init, context);
      return nested ? `\`${node.name}\` derives from ${nested}` : undefined;
    }

    return undefined; // a prop or a stable binding: same values, holds
  }

  if (node.type === "MemberExpression") {
    // `entry.path` and friends are value reads: a fresh object with equal values
    // yields an equal string, so the guard holds.
    if (node.object.type === "Identifier" && node.object.name === context.prop) return undefined;
    const onObject = classify(node.object, context);
    return onObject ? `${describe(node)} reads ${onObject}` : undefined;
  }

  if (node.type === "CallExpression") {
    for (const argument of node.arguments) {
      const onArgument = classify(argument, context);
      if (onArgument) {
        return `${describe(node.callee)}(${describe(argument)}) takes ${onArgument}`;
      }
    }
    return undefined;
  }

  if (node.type === "ConditionalExpression") {
    return classify(node.consequent, context) ?? classify(node.alternate, context);
  }

  // Anything else (template literals, logical chains, member reads off derived
  // objects) falls back to a structural scan.
  for (const key of Object.keys(node)) {
    if (key === "loc" || key === "range" || key === "start" || key === "end") continue;
    const value = node[key];
    if (Array.isArray(value)) {
      for (const child of value) {
        const found = child?.type ? classify(child, context) : undefined;
        if (found) return found;
      }
    } else if (value && typeof value === "object" && value.type) {
      const found = classify(value, context);
      if (found) return found;
    }
  }
  return undefined;
}

function describe(node) {
  if (!node) return "?";
  if (node.type === "Identifier") return node.name;
  if (node.type === "MemberExpression") {
    return `${describe(node.object)}.${node.property.name ?? "?"}`;
  }
  if (node.type === "CallExpression") return `${describe(node.callee)}(…)`;
  if (node.type === "StringLiteral") return JSON.stringify(node.value);
  return node.type;
}

function audit({ file, component, prop }) {
  const source = compile(file, prop);
  const ast = parse(source, { sourceType: "module", plugins: ["jsx", "typescript"] });

  const declaration = ast.program.body.find(
    (node) => node.type === "FunctionDeclaration" && node.id?.name === component,
  );
  if (!declaration) throw new Error(`${component} not found in the compiled output of ${file}`);

  const statements = [];
  collectStatements(declaration.body, statements);
  if (statements.length === 0)
    throw new Error(`${component} has no memo guards: is it a component?`);

  const localInit = new Map();
  collectLocals(declaration.body, localInit);

  const failed = new Map();
  const producerOf = new Map();
  const guards = [];

  statements.forEach((statement, index) => {
    const context = { prop, producerOf, failed, localInit, seen: new Set() };
    const reasons = guardDeps(statement.test)
      .map((expr) => {
        const kind = classify(expr, context);
        return kind ? `${describe(expr)} — ${kind}` : undefined;
      })
      .filter(Boolean);

    for (const name of boundNames(statement.consequent)) producerOf.set(name, index);

    const record = {
      index,
      line: statement.loc.start.line,
      elementCount: jsxCount(statement.consequent),
      reasons,
      failed: reasons.length > 0,
    };
    guards.push(record);
    if (record.failed) failed.set(index, reasons);
  });

  return { component, file, prop, guards };
}

const audited = targets.map(audit);

console.log(`\nReplacing the prop with a fresh object of equal values, per row, per render\n`);
console.log(
  "component".padEnd(14) +
    "prop".padEnd(8) +
    "guards".padStart(8) +
    "failing".padStart(9) +
    "elements rebuilt".padStart(18) +
    "of total".padStart(10),
);
for (const row of audited) {
  const failing = row.guards.filter((guard) => guard.failed);
  const rebuilt = failing.reduce((sum, guard) => sum + guard.elementCount, 0);
  const total = row.guards.reduce((sum, guard) => sum + guard.elementCount, 0);
  console.log(
    row.component.padEnd(14) +
      row.prop.padEnd(8) +
      String(row.guards.length).padStart(8) +
      `${failing.length} (${Math.round((failing.length / row.guards.length) * 100)}%)`.padStart(9) +
      String(rebuilt).padStart(18) +
      String(total).padStart(10),
  );
}

for (const row of audited) {
  const failing = row.guards.filter((guard) => guard.failed);
  console.log(`\n${row.component} — ${failing.length} failing guard(s), ${row.file}`);
  for (const guard of failing) {
    console.log(
      `  #${String(guard.index).padStart(3)} (line ${guard.line}) rebuilds ${guard.elementCount} element(s):`,
    );
    for (const reason of guard.reasons) console.log(`        ${reason}`);
  }
}

const rebuiltTotal = audited.reduce(
  (sum, row) => sum + row.guards.filter((g) => g.failed).reduce((n, g) => n + g.elementCount, 0),
  0,
);
console.log(
  `\n${rebuiltTotal} elements across the three components are handed to React fresh on every render\n` +
    "when the prop is a new object. React's own cost per element is not measured here.",
);
