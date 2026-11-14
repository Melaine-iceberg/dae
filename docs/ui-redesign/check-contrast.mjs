#!/usr/bin/env node
// Verifies the colour tokens proposed in docs/ui-redesign/proposal.md §6.
//
//   node docs/ui-redesign/contrast.mjs
//
// No dependencies. Every hex value below is the one written in the proposal;
// what this script adds is the measured WCAG ratio for every pair the proposal
// makes a claim about, plus the hue distance between the accent and the file
// categories. Change a token and this prints whether the change still passes.

// ── OKLCH → sRGB (for regenerating the ramp from L/C/h) ────────────────────
function oklchToRgb(L, C, hDeg) {
  const h = (hDeg * Math.PI) / 180;
  const a = C * Math.cos(h);
  const b = C * Math.sin(h);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const lin = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  const enc = (v) => (v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055);
  return lin.map((v) => Math.min(1, Math.max(0, enc(v))));
}

const hex = (L, C, h) =>
  "#" +
  oklchToRgb(L, C, h)
    .map((v) => Math.round(v * 255).toString(16).padStart(2, "0"))
    .join("");

const parse = (value) => [1, 3, 5].map((i) => parseInt(value.slice(i, i + 2), 16) / 255);

function luminance(rgb) {
  const lin = rgb.map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}

const ratio = (fg, bg) => {
  const a = luminance(parse(fg));
  const b = luminance(parse(bg));
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
};

const mix = (fg, bg, alpha) =>
  "#" +
  parse(fg)
    .map((v, i) => Math.round((v * alpha + parse(bg)[i] * (1 - alpha)) * 255).toString(16).padStart(2, "0"))
    .join("");

// ── The tokens (proposal §6.1) ─────────────────────────────────────────────
const NEUTRAL_HUE = 264;
const ACCENT_HUE = 195;
const CATEGORY_HUE = { l: 0.5, c: 0.075, lDark: 0.79, cDark: 0.06 };

const light = {
  window: "#eff0f2",
  chrome: "#f7f7f9",
  canvas: "#fdfdfd",
  raised: "#fdfdfd",
  field: "#eff0f2",
  inset: "#f7f7f9",
  hairline: "#e5e6e9",
  line: "#d7d9dc",
  lineStrong: "#bcbec2",
  ink: "#1c1e23",
  inkSecondary: "#505358",
  inkTertiary: "#6b6d72",
  accent: "#007a7b",
  danger: "#ba2b2e",
  warning: "#8d5a00",
  success: "#227849",
};

const dark = {
  window: "#090b0f",
  chrome: "#15171c",
  canvas: "#101216",
  raised: "#292b30",
  field: "#1c1e23",
  inset: "#15171c",
  hairline: "#1c1e23",
  line: "#292b30",
  lineStrong: "#3a3d43",
  ink: "#f7f7f9",
  inkSecondary: "#bcbec2",
  inkTertiary: "#96989d",
  accent: "#6bcac9",
  danger: "#f47b74",
  warning: "#e6b55d",
  success: "#7bc495",
};

const families = {
  folder: 75,
  document: 275,
  spreadsheet: 145,
  presentation: 25,
  pdf: 5,
  image: 325,
  video: 300,
  audio: 65,
  archive: 45,
  code: 235,
  text: 248,
  font: 292,
  executable: 105,
  other: 264,
};

// ── Report ─────────────────────────────────────────────────────────────────
const pad = (s, w) => String(s).padEnd(w);
let failures = 0;

function check(label, fg, bg, minimum) {
  const r = ratio(fg, bg);
  const ok = r >= minimum;
  if (!ok) failures += 1;
  console.log(`  ${ok ? "PASS" : "FAIL"}  ${pad(label, 34)} ${r.toFixed(2).padStart(6)}:1  (min ${minimum})`);
  return r;
}

for (const [name, t] of [["LIGHT", light], ["DARK", dark]]) {
  console.log(`\n${name} — text on surfaces`);
  for (const ink of ["ink", "inkSecondary", "inkTertiary"]) {
    for (const surface of ["canvas", "chrome", "raised", "field"]) {
      check(`${ink} / ${surface}`, t[ink], t[surface], ink === "ink" ? 7 : 4.5);
    }
  }

  console.log(`\n${name} — accent`);
  check("accent / canvas (non-text ≥3)", t.accent, t.canvas, 3);
  console.log(`  ${pad("accent / selected row (ink)", 34)} ${ratio(t.ink, mix(t.accent, t.canvas, name === "LIGHT" ? 0.1 : 0.13)).toFixed(2).padStart(6)}:1`);
  check("selected row keeps ink ≥ 4.5", t.ink, mix(t.accent, t.canvas, name === "LIGHT" ? 0.1 : 0.13), 4.5);

  console.log(`\n${name} — semantic text`);
  for (const role of ["danger", "warning", "success"]) check(`${role} / canvas`, t[role], t.canvas, 4.5);
}

console.log("\nNEUTRAL RAMP REGENERATION (hue " + NEUTRAL_HUE + ")");
const ramp = [
  [0, 0.995, 0.0], [50, 0.977, 0.002], [100, 0.955, 0.003], [200, 0.925, 0.004],
  [300, 0.885, 0.005], [400, 0.8, 0.006], [500, 0.68, 0.007], [600, 0.535, 0.008],
  [700, 0.44, 0.009], [800, 0.36, 0.01], [850, 0.29, 0.01], [900, 0.235, 0.01],
  [925, 0.205, 0.01], [950, 0.18, 0.009], [975, 0.15, 0.008],
];
console.log("  " + ramp.map(([step, L, C]) => `${step}:${hex(L, C, NEUTRAL_HUE)}`).join("  "));

console.log("\nFILE CATEGORIES — contrast on canvas, hue distance from accent");
for (const [name, h] of Object.entries(families)) {
  const l = hex(CATEGORY_HUE.l, CATEGORY_HUE.c, h);
  const d = hex(CATEGORY_HUE.lDark, CATEGORY_HUE.cDark, h);
  const distance = Math.min(Math.abs(h - ACCENT_HUE), 360 - Math.abs(h - ACCENT_HUE));
  const close = distance < 30;
  if (close) failures += 1;
  console.log(
    `  ${pad(name, 13)} ${h}°  ${l} ${ratio(l, light.canvas).toFixed(2)}:1  ${d} ${ratio(d, dark.canvas).toFixed(2)}:1  Δ${String(distance).padStart(3)}°${close ? "  <-- within 30° of the accent" : ""}`
  );
}

console.log(
  failures === 0
    ? "\nAll checks pass."
    : `\n${failures} check(s) failed.`
);
process.exitCode = failures === 0 ? 0 : 1;
