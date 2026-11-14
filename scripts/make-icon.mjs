// 生成 CommandKit 应用图标源图（1024×1024 PNG），随后交给 `tauri icon` 派生各尺寸 / ico / icns
import { deflateSync } from 'node:zlib';
import { writeFileSync } from 'node:fs';

const S = 1024;
const buf = Buffer.alloc(S * S * 4, 0);

const hex = (h) => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
const BG = hex('#101214');
const STROKE = hex('#2A2D30');
const ACCENT = hex('#9EFF00');
const ACCENT_DIM = hex('#5C8F14');

const setPx = (x, y, rgb, a = 1) => {
  if (x < 0 || y < 0 || x >= S || y >= S) return;
  const i = (y * S + x) * 4;
  const alpha = Math.max(0, Math.min(1, a));
  const dst = [buf[i], buf[i + 1], buf[i + 2], buf[i + 3] / 255];
  const out = [0, 0, 0, 0];
  for (let c = 0; c < 3; c++) out[c] = Math.round(rgb[c] * alpha + dst[c] * (1 - alpha) * dst[3]);
  out[3] = Math.round((alpha + dst[3] * (1 - alpha)) * 255);
  buf[i] = out[0];
  buf[i + 1] = out[1];
  buf[i + 2] = out[2];
  buf[i + 3] = out[3];
};

const roundedRect = (x0, y0, x1, y1, r) => {
  for (let y = Math.floor(y0); y < y1; y++) {
    for (let x = Math.floor(x0); x < x1; x++) {
      const dx = Math.max(x0 + r - x, 0, x - (x1 - r));
      const dy = Math.max(y0 + r - y, 0, y - (y1 - r));
      const d = Math.hypot(dx, dy);
      const a = Math.min(1, Math.max(0, r - d + 0.5));
      if (a > 0) setPx(x, y, BG, a);
    }
  }
};

const seg = (x1, y1, x2, y2, w, rgb) => {
  const minX = Math.min(x1, x2) - w;
  const maxX = Math.max(x1, x2) + w;
  const minY = Math.min(y1, y2) - w;
  const maxY = Math.max(y1, y2) + w;
  for (let y = Math.floor(minY); y < maxY; y++) {
    for (let x = Math.floor(minX); x < maxX; x++) {
      const dx = x2 - x1;
      const dy = y2 - y1;
      const len2 = dx * dx + dy * dy || 1;
      let t = ((x - x1) * dx + (y - y1) * dy) / len2;
      t = Math.max(0, Math.min(1, t));
      const px = x1 + t * dx;
      const py = y1 + t * dy;
      const d = Math.hypot(x - px, y - py);
      const a = Math.min(1, Math.max(0, w / 2 - d + 0.5));
      if (a > 0) setPx(x, y, rgb, a);
    }
  }
};

// 背景圆角方块
roundedRect(48, 48, 976, 976, 200);

// 顶部一条细扫描线（隐喻终端光标）
for (let y = 120; y < 128; y++) {
  for (let x = 150; x < 874; x++) {
    const a = 0.10 * Math.min(1, (x - 150) / 60, (874 - x) / 60);
    if (a > 0) setPx(x, y, ACCENT, a);
  }
}

// ">" 提示符
seg(300, 350, 470, 512, 76, ACCENT);
seg(470, 512, 300, 674, 76, ACCENT);
// 下划线
seg(560, 660, 740, 660, 76, ACCENT);

// 左下角小方块装饰
seg(300, 780, 420, 780, 30, ACCENT_DIM);
seg(300, 838, 520, 838, 30, ACCENT_DIM);

// PNG 编码
const crcTable = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();
const crc32 = (b) => {
  let c = -1;
  for (let i = 0; i < b.length; i++) c = crcTable[(c ^ b[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
};

const raw = Buffer.alloc((S * 4 + 1) * S);
for (let y = 0; y < S; y++) {
  raw[y * (S * 4 + 1)] = 0;
  buf.copy(raw, y * (S * 4 + 1) + 1, y * S * 4, (y + 1) * S * 4);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr[8] = 8;
ihdr[9] = 6;
const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk('IHDR', ihdr),
  chunk('IDAT', deflateSync(raw, { level: 9 })),
  chunk('IEND', Buffer.alloc(0)),
]);

writeFileSync(process.argv[2] ?? 'icon.png', png);
console.log('icon written', png.length, 'bytes');
