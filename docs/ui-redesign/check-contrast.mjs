#!/usr/bin/env node
// Verifies the Material 3 Expressive colour roles in `qml/Theme.qml`.
//
//   node docs/ui-redesign/check-contrast.mjs
//
// No dependencies. Every hex value below mirrors the resolved light/dark token
// in Theme.qml; what this script adds is the measured WCAG ratio for every pair
// the token layer makes a claim about, plus the derived selection container
// (`secondaryContainer` / `onSecondaryContainer`) that Theme builds from the
// seed with HSL. Change a token and this prints whether it still passes.

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

// ── HSL helpers, mirroring Theme.qml ──────────────────────────────────────
function rgbToHsl(value) {
  const [r, g, b] = parse(value);
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const delta = max - min;
  let h = 0;
  let s = 0;
  if (delta > 0.00001) {
    s = l > 0.5 ? delta / (2 - max - min) : delta / (max + min);
    if (max === r) h = ((g - b) / delta + (g < b ? 6 : 0)) * 60;
    else if (max === g) h = ((b - r) / delta + 2) * 60;
    else h = ((r - g) / delta + 4) * 60;
  }
  return { h, s, l };
}

function hslToHex(hue, saturation, lightness) {
  const h = (((hue % 360) + 360) % 360) / 360;
  const s = Math.max(0, Math.min(1, saturation));
  const l = Math.max(0, Math.min(1, lightness));
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const channel = (t) => {
    let v = t;
    if (v < 0) v += 1;
    if (v > 1) v -= 1;
    if (v < 1 / 6) return p + (q - p) * 6 * v;
    if (v < 1 / 2) return q;
    if (v < 2 / 3) return p + (q - p) * (2 / 3 - v) * 6;
    return p;
  };
  return (
    "#" +
    [channel(h + 1 / 3), channel(h), channel(h - 1 / 3)]
      .map((v) => Math.round(v * 255).toString(16).padStart(2, "0"))
      .join("")
  );
}

const secondaryContainer = (accent, isDark) =>
  hslToHex(rgbToHsl(accent).h, 0.34, isDark ? 0.24 : 0.86);
const onSecondaryContainer = (accent, isDark) =>
  hslToHex(rgbToHsl(accent).h, 0.55, isDark ? 0.93 : 0.13);

function readableInk(background, preferred) {
  const candidate = preferred;
  if (ratio(background, candidate) >= 4.5) return candidate;
  return luminance(parse(background)) > 0.45 ? "#101413" : "#ffffff";
}

// ── The tokens (qml/Theme.qml) ────────────────────────────────────────────
const NEUTRAL_HUE = 264;
const ACCENT_HUE = 195;
const CATEGORY_HUE = { l: 0.5, c: 0.075, lDark: 0.79, cDark: 0.06 };

const light = {
  window: "#e6ebf4",
  chrome: "#f3f6fc",
  canvas: "#ffffff",
  raised: "#ffffff",
  field: "#edf2fa",
  inset: "#f3f6fc",
  hairline: "#e1e3e1",
  line: "#747775",
  lineStrong: "#444746",
  ink: "#1f1f1f",
  inkSecondary: "#444746",
  inkTertiary: "#5f6368",
  accent: "#0b57d0",
  danger: "#ba2b2e",
  warning: "#8d5a00",
  success: "#227849",
};

const dark = {
  window: "#191a1d",
  chrome: "#25272b",
  canvas: "#202124",
  raised: "#303238",
  field: "#202124",
  inset: "#25272b",
  hairline: "#3c4043",
  line: "#8e918f",
  lineStrong: "#c4c7c5",
  ink: "#e8eaed",
  inkSecondary: "#c4c7c5",
  inkTertiary: "#9aa0a6",
  accent: "#a8c7fa",
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

// The fixed tag options (§6.1.7). Stored as the light spelling; the dark list
// is the paint-side variant. They are marks, so the bar is the 3:1 non-text
// threshold.
const tagPaletteLight = ["#a3392f", "#9a5a20", "#7d6a12", "#2f6b34", "#1d6560", "#2b5590", "#5b3f92", "#5c6169"];
const tagPaletteDark = ["#e08585", "#e0a377", "#ddc274", "#8fc48c", "#7fc3bd", "#82a9dd", "#ab93dd", "#a8adb4"];

// The destructive fill is not the accent, so its ink is checked separately.
const dangerSolidLight = "#ba2b2e";
const dangerSolidDark = "#e06b64";

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
  const container = secondaryContainer(t.accent, name === "DARK");
  const containerInk = readableInk(container, onSecondaryContainer(t.accent, name === "DARK"));
  console.log(`  ${pad("selection container", 34)} ${container}  ink ${containerInk}`);
  check("selectionInk / selection ≥ 4.5", containerInk, container, 4.5);

  console.log(`\n${name} — semantic text`);
  for (const role of ["danger", "warning", "success"]) check(`${role} / canvas`, t[role], t.canvas, 4.5);

  console.log(`\n${name} — tag marks (non-text ≥3)`);
  const tags = name === "LIGHT" ? tagPaletteLight : tagPaletteDark;
  tags.forEach((colour, index) => check(`tag ${index + 1} / canvas`, colour, t.canvas, 3));

  console.log(`\n${name} — destructive fill`);
  check("canvas ink on dangerSolid",
          name === "LIGHT" ? light.canvas : dark.canvas,
          name === "LIGHT" ? dangerSolidLight : dangerSolidDark, 4.5);
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
