// Generates assets/icon-source.png (1024x1024): rounded accent tile with a white note-like dot pair.
import { deflateSync } from 'node:zlib';
import { writeFileSync, mkdirSync } from 'node:fs';
const N = 1024, px = Buffer.alloc(N * N * 4);
const inRounded = (x, y, r) => { const m = 96, R = 220; const cx = Math.min(Math.max(x, m + R), N - m - R), cy = Math.min(Math.max(y, m + R), N - m - R); return x >= m && x < N - m && y >= m && y < N - m && (x - cx) ** 2 + (y - cy) ** 2 <= R * R; };
const disc = (x, y, cx, cy, r) => (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
for (let y = 0; y < N; y++) for (let x = 0; x < N; x++) {
  const i = (y * N + x) * 4; let c = [0, 0, 0, 0];
  if (inRounded(x, y)) c = [59, 111, 216, 255];
  const note = disc(x, y, 400, 690, 110) || disc(x, y, 680, 620, 110) || (x >= 470 && x <= 510 && y >= 300 && y <= 690) || (x >= 750 && x <= 790 && y >= 230 && y <= 620) || (y >= 230 + (790 - x) * 0 && x >= 470 && x <= 790 && y >= 300 - (x - 470) * 70 / 320 - 70 && y <= 300 - (x - 470) * 70 / 320);
  if (c[3] && note) c = [255, 255, 255, 255];
  px.set(c, i);
}
const raw = Buffer.alloc(N * (N * 4 + 1));
for (let y = 0; y < N; y++) { raw[y * (N * 4 + 1)] = 0; px.copy(raw, y * (N * 4 + 1) + 1, y * N * 4, (y + 1) * N * 4); }
const crcT = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc = b => { let c = 0xffffffff; for (const v of b) c = crcT[(c ^ v) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([l, td, c]); };
const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(N, 0); ihdr.writeUInt32BE(N, 4); ihdr[8] = 8; ihdr[9] = 6;
mkdirSync('assets', { recursive: true });
writeFileSync('assets/icon-source.png', Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]));
