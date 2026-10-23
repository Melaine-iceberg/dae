// 渲染鹅图标: JPG 原画 -> 1024 母版 PNG + 256 预览 + MSIX 磁贴资源,
// 并输出 ASCII 字符画便于验收
import sharp from "sharp";
import { readFileSync } from "node:fs";
import zlib from "node:zlib";

const SRC = "src-tauri/icons/source/goose-source.jpg";
const MASTER = "src-tauri/icons/source/goose-master.png";
const PREVIEW = "src-tauri/icons/source/goose-preview.png";
const APP_ICON = "src/assets/app-icon.png";
const MSIX = "Assets";

// 18% 圆角，与旧图标在 1024 画布上的 186px 半径一致。透明四角让系统
// 图标底和窗口背景透出来，而不是留下一个黑方块。
const ICON_RADIUS = 186;
const ROUNDED_MASK = Buffer.from(
  `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024"><rect width="1024" height="1024" rx="${ICON_RADIUS}" fill="#fff"/></svg>`,
);

await sharp(SRC)
  .resize(1024, 1024, { kernel: "lanczos3" })
  .ensureAlpha()
  .composite([{ input: ROUNDED_MASK, blend: "dest-in" }])
  .png()
  .toFile(MASTER);
await sharp(MASTER).resize(256, 256, { kernel: "lanczos3" }).png().toFile(PREVIEW);
await sharp(MASTER).resize(256, 256, { kernel: "lanczos3" }).png().toFile(APP_ICON);
console.log("rendered", MASTER, PREVIEW, "and", APP_ICON);

// ---- MSIX 资源 ----
// Windows 的磁贴清单引用 Assets/ 下的固定文件名，尺寸由系统按 DPI 缩放选取。
const scales = [1, 1.25, 1.5, 2, 4];
const scaleSuffix = (scale) =>
  scale === 1 ? "" : `.scale-${String(Math.round(scale * 100))}`;

const squares = [
  ["AppList", 44],
  ["MedTile", 150],
  ["StoreLogo", 50],
];
for (const [name, size] of squares) {
  for (const scale of scales) {
    await sharp(MASTER)
      .resize(Math.round(size * scale), Math.round(size * scale), { kernel: "lanczos3" })
      .png()
      .toFile(`${MSIX}/${name}${scaleSuffix(scale)}.png`);
  }
}

// 列表图标另外需要 targetsize 变体，Windows 在任务栏和开始菜单按像素尺寸取用。
for (const size of [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256]) {
  for (const suffix of ["", "_altform-unplated"]) {
    await sharp(MASTER)
      .resize(size, size, { kernel: "lanczos3" })
      .png()
      .toFile(`${MSIX}/AppList.targetsize-${size}${suffix}.png`);
  }
}

// 宽磁贴是 310×150 的透明画布，图标居中、保持 150px 见方（与旧资源一致）。
for (const scale of scales) {
  const iconSize = Math.round(150 * scale);
  const width = Math.round(310 * scale);
  const height = Math.round(150 * scale);
  await sharp(MASTER)
    .resize(iconSize, iconSize, { kernel: "lanczos3" })
    .extend({
      top: Math.floor((height - iconSize) / 2),
      bottom: Math.ceil((height - iconSize) / 2),
      left: Math.floor((width - iconSize) / 2),
      right: Math.ceil((width - iconSize) / 2),
      background: { r: 0, g: 0, b: 0, alpha: 0 },
    })
    .png()
    .toFile(`${MSIX}/WideTile${scaleSuffix(scale)}.png`);
}
console.log("rendered MSIX tiles in", MSIX);

// ---- ASCII 校验 ----
function decode(f) {
  const buf = readFileSync(f);
  let o = 8,
    w = 0,
    h = 0,
    ct = 0;
  const idat = [];
  while (o < buf.length) {
    const len = buf.readUInt32BE(o),
      t = buf.toString("ascii", o + 4, o + 8);
    if (t === "IHDR") {
      w = buf.readUInt32BE(o + 8);
      h = buf.readUInt32BE(o + 12);
      ct = buf[o + 17];
    }
    if (t === "IDAT") idat.push(buf.subarray(o + 8, o + 8 + len));
    if (t === "IEND") break;
    o += 12 + len;
  }
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const ch = ct === 6 ? 4 : 3;
  const stride = w * ch;
  const out = Buffer.alloc(h * stride);
  let p = 0;
  for (let y = 0; y < h; y++) {
    const fl = raw[p++];
    const line = raw.subarray(p, p + stride);
    p += stride;
    const cur = out.subarray(y * stride, (y + 1) * stride);
    const prev = y > 0 ? out.subarray((y - 1) * stride, y * stride) : Buffer.alloc(stride);
    for (let x = 0; x < stride; x++) {
      const a2 = x >= ch ? cur[x - ch] : 0,
        b = prev[x],
        c = x >= ch ? prev[x - ch] : 0;
      let v = line[x];
      if (fl === 1) v += a2;
      else if (fl === 2) v += b;
      else if (fl === 3) v += (a2 + b) >> 1;
      else if (fl === 4) {
        const pa = Math.abs(b - c),
          pb = Math.abs(a2 - c),
          pc = Math.abs(a2 + b - 2 * c);
        v += pa <= pb && pa <= pc ? a2 : pb <= pc ? b : c;
      }
      cur[x] = v & 255;
    }
  }
  return { w, h, ch, out };
}

const { w, h, ch, out } = decode(PREVIEW);
const px = (x, y) => {
  const q = (y * w + x) * ch;
  return [out[q], out[q + 1], out[q + 2], ch === 4 ? out[q + 3] : 255];
};
const chars = " .:-=+*#%@";
const N = 72;
let art = "";
for (let r = 0; r < N; r++) {
  let line = "";
  for (let c2 = 0; c2 < N; c2++) {
    const x = Math.min(w - 1, Math.floor(((c2 + 0.5) * w) / N)),
      y = Math.min(h - 1, Math.floor(((r + 0.5) * h) / N));
    const [R, G, B, A] = px(x, y);
    if (A < 40) {
      line += " ";
      continue;
    }
    const lum = (0.299 * R + 0.587 * G + 0.114 * B) / 255;
    const sat = Math.max(R, G, B) - Math.min(R, G, B);
    if (sat > 36) line += R > G && R > B ? "R" : G > B ? "G" : "B";
    else line += chars[Math.min(chars.length - 1, Math.round(lum * (chars.length - 1)))];
  }
  art += line + "\n";
}
console.log(art);
