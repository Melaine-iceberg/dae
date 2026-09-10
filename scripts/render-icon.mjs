// 渲染鹅图标: SVG -> 1024 母版 PNG + 256 预览, 并输出 ASCII 字符画便于验收
import sharp from "sharp";
import { readFileSync } from "node:fs";
import zlib from "node:zlib";

const SRC = "src-tauri/icons/source/goose-icon.svg";
const MASTER = "src-tauri/icons/source/goose-master.png";
const PREVIEW = "src-tauri/icons/source/goose-preview.png";

const svg = readFileSync(SRC);
await sharp(svg, { density: 384 }).resize(1024, 1024).png().toFile(MASTER);
await sharp(svg, { density: 384 }).resize(256, 256).png().toFile(PREVIEW);
console.log("rendered", MASTER, "and", PREVIEW);

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
