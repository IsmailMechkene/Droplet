// Dev-time only: draws the Droplet mark and writes src-tauri/icons/icon.ico.
// Usage: node tools/make-icon.js
const fs = require('fs'), zlib = require('zlib'), path = require('path');
const COLOR = [0x2b, 0x7c, 0xbf];

// Coverage of the mark at a point in [-1,1]²: a rounded square with one sharp
// corner, rotated so the tip points down (same shape as the CSS logo).
function inside(x, y) {
  const c = Math.SQRT1_2, u = (x * c - y * c) / 0.6, v = (x * c + y * c) / 0.6;
  if (u * u + v * v <= 1) return true;                       // the three round corners
  return u <= 0 && v >= 0 && u >= -1 && v <= 1;              // the sharp corner (tip)
}
function render(n) {
  const px = Buffer.alloc(n * n * 4), SS = 4;
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    let hit = 0;
    for (let j = 0; j < SS; j++) for (let i = 0; i < SS; i++) {
      const fx = ((x + (i + 0.5) / SS) / n) * 2 - 1, fy = ((y + (j + 0.5) / SS) / n) * 2 - 0.9;
      if (inside(fx, fy)) hit++;
    }
    const o = (y * n + x) * 4;
    px[o] = COLOR[0]; px[o + 1] = COLOR[1]; px[o + 2] = COLOR[2]; px[o + 3] = Math.round(255 * hit / (SS * SS));
  }
  return px;
}
function png(n, rgba) {
  const crcT = Array.from({ length: 256 }, (_, k) => { let c = k; for (let i = 0; i < 8; i++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (b) => { let c = ~0; for (const x of b) c = crcT[(c ^ x) & 255] ^ (c >>> 8); return (~c) >>> 0; };
  const chunk = (t, d) => { const len = Buffer.alloc(4); len.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const cr = Buffer.alloc(4); cr.writeUInt32BE(crc(td)); return Buffer.concat([len, td, cr]); };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(n, 0); ihdr.writeUInt32BE(n, 4); ihdr[8] = 8; ihdr[9] = 6;
  const raw = Buffer.alloc(n * (n * 4 + 1));
  for (let y = 0; y < n; y++) rgba.copy(raw, y * (n * 4 + 1) + 1, y * n * 4, (y + 1) * n * 4);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}
function bmp(n, rgba) {
  const head = Buffer.alloc(40); head.writeUInt32LE(40, 0); head.writeInt32LE(n, 4); head.writeInt32LE(n * 2, 8); head.writeUInt16LE(1, 12); head.writeUInt16LE(32, 14);
  const body = Buffer.alloc(n * n * 4);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) { const s = ((n - 1 - y) * n + x) * 4, d = (y * n + x) * 4; body[d] = rgba[s + 2]; body[d + 1] = rgba[s + 1]; body[d + 2] = rgba[s]; body[d + 3] = rgba[s + 3]; }
  return Buffer.concat([head, body, Buffer.alloc(Math.ceil(n / 32) * 4 * n)]);
}
const sizes = [16, 20, 24, 32, 48, 64, 256];
const images = sizes.map((n) => (n === 256 ? png(n, render(n)) : bmp(n, render(n))));
const dir = Buffer.alloc(6 + 16 * sizes.length); dir.writeUInt16LE(1, 2); dir.writeUInt16LE(sizes.length, 4);
let off = dir.length;
sizes.forEach((n, i) => { const e = 6 + i * 16; dir[e] = n & 255; dir[e + 1] = n & 255; dir.writeUInt16LE(1, e + 4); dir.writeUInt16LE(32, e + 6); dir.writeUInt32LE(images[i].length, e + 8); dir.writeUInt32LE(off, e + 12); off += images[i].length; });
const out = path.join(__dirname, '..', 'src-tauri', 'icons');
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(out, 'icon.ico'), Buffer.concat([dir, ...images]));
fs.writeFileSync(path.join(out, 'icon.png'), png(256, render(256)));
console.log('wrote icon.ico, icon.png');
