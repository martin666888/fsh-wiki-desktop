import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const SIZE = 1024;

function crc32(buf) {
  let table = crc32.table;
  if (!table) {
    table = crc32.table = new Uint32Array(256);
    for (let i = 0; i < 256; i++) {
      let c = i;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      table[i] = c >>> 0;
    }
  }
  let crc = 0xffffffff;
  for (const b of buf) crc = table[(crc ^ b) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

// rounded-square blue gradient with white document glyph
const px = Buffer.alloc(SIZE * (SIZE * 4 + 1));
const cornerR = 180;

function inRoundRect(x, y) {
  const m = cornerR;
  if (x < m && y < m) return (x - m) ** 2 + (y - m) ** 2 <= m * m;
  if (x > SIZE - 1 - m && y < m) return (x - (SIZE - 1 - m)) ** 2 + (y - m) ** 2 <= m * m;
  if (x < m && y > SIZE - 1 - m) return (x - m) ** 2 + (y - (SIZE - 1 - m)) ** 2 <= m * m;
  if (x > SIZE - 1 - m && y > SIZE - 1 - m)
    return (x - (SIZE - 1 - m)) ** 2 + (y - (SIZE - 1 - m)) ** 2 <= m * m;
  return true;
}

// document sheet bounds
const dx0 = 322, dx1 = 702, dy0 = 232, dy1 = 792;
const fold = 90; // folded corner size

for (let y = 0; y < SIZE; y++) {
  const rowStart = y * (SIZE * 4 + 1);
  px[rowStart] = 0; // filter: none
  for (let x = 0; x < SIZE; x++) {
    const o = rowStart + 1 + x * 4;
    let r = 0, g = 0, b = 0, a = 0;
    if (inRoundRect(x, y)) {
      // gradient #3370FF -> #245BDB diagonal
      const t = (x + y) / (2 * SIZE);
      r = Math.round(0x33 + (0x24 - 0x33) * t);
      g = Math.round(0x70 + (0x5b - 0x70) * t);
      b = Math.round(0xff + (0xdb - 0xff) * t);
      a = 255;

      const inSheetX = x >= dx0 && x <= dx1;
      const inSheetY = y >= dy0 && y <= dy1;
      const inFoldCut = x > dx1 - fold && y < dy0 + fold && (x - (dx1 - fold)) + ((dy0 + fold) - y) < fold;
      if (inSheetX && inSheetY && !inFoldCut) {
        r = 255; g = 255; b = 255;
        // folded corner triangle (light gray)
        if (x >= dx1 - fold && y <= dy0 + fold) {
          const insideFold = (x - (dx1 - fold)) + ((dy0 + fold) - y) >= fold;
          if (!insideFold) { r = 0xd8; g = 0xe2; b = 0xf0; }
        }
        // text lines
        const lx0 = dx0 + 56, lx1 = dx1 - 56;
        const lineH = 22, gap = 74, top = dy0 + 170;
        const lineIdx = Math.floor((y - top) / gap);
        const inLineBand = y >= top && (y - top) % gap < lineH;
        if (inLineBand && x >= lx0 && lineIdx >= 0 && lineIdx < 5) {
          const lineEnd = lineIdx === 4 ? lx0 + (lx1 - lx0) * 0.55 : lx1;
          if (x <= lineEnd) { r = 0xb9; g = 0xc9; b = 0xe3; }
        }
      }
    }
    px[o] = r; px[o + 1] = g; px[o + 2] = b; px[o + 3] = a;
  }
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8;  // bit depth
ihdr[9] = 6;  // color type RGBA
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(px, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);
writeFileSync(new URL("./app-icon.png", import.meta.url), png);
console.log("app-icon.png written:", png.length, "bytes");
