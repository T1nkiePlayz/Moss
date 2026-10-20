// Minimal dependency-free QR encoder: byte mode, error correction M, versions 1-10 (up to 213 bytes).
// Enough for otpauth:// URIs. Throws when the text does not fit.
const EC_PER_BLOCK = [10, 16, 26, 18, 24, 16, 18, 22, 22, 26];
const NUM_BLOCKS = [1, 1, 1, 2, 2, 4, 4, 4, 5, 5];
export const MAX_VERSION = EC_PER_BLOCK.length;

function rawModules(v: number): number {
  let r = (16 * v + 128) * v + 64;
  if (v >= 2) { const n = Math.floor(v / 7) + 2; r -= (25 * n - 10) * n - 55; if (v >= 7) r -= 36; }
  return r;
}
const dataCodewords = (v: number) => Math.floor(rawModules(v) / 8) - EC_PER_BLOCK[v - 1] * NUM_BLOCKS[v - 1];
const bit = (x: number, i: number) => ((x >>> i) & 1) !== 0;

function gfMul(x: number, y: number): number {
  let z = 0;
  for (let i = 7; i >= 0; i--) { z = (z << 1) ^ ((z >>> 7) * 0x11d); z ^= ((y >>> i) & 1) * x; }
  return z;
}
function rsDivisor(deg: number): number[] {
  const r = new Array<number>(deg).fill(0);
  r[deg - 1] = 1;
  let root = 1;
  for (let i = 0; i < deg; i++) {
    for (let j = 0; j < deg; j++) { r[j] = gfMul(r[j], root); if (j + 1 < deg) r[j] ^= r[j + 1]; }
    root = gfMul(root, 2);
  }
  return r;
}
function rsRemainder(data: number[], div: number[]): number[] {
  const r = new Array<number>(div.length).fill(0);
  for (const b of data) {
    const f = b ^ r.shift()!;
    r.push(0);
    div.forEach((c, i) => (r[i] ^= gfMul(c, f)));
  }
  return r;
}

function buildCodewords(bytes: Uint8Array, v: number): number[] {
  const bits: number[] = [];
  const push = (val: number, n: number) => { for (let i = n - 1; i >= 0; i--) bits.push((val >>> i) & 1); };
  push(0b0100, 4);
  push(bytes.length, v < 10 ? 8 : 16);
  bytes.forEach((b) => push(b, 8));
  const cap = dataCodewords(v) * 8;
  push(0, Math.min(4, cap - bits.length));
  push(0, (8 - (bits.length % 8)) % 8);
  const data: number[] = [];
  for (let i = 0; i < bits.length; i += 8) data.push(parseInt(bits.slice(i, i + 8).join(''), 2));
  for (let pad = 0xec; data.length < cap / 8; pad ^= 0xec ^ 0x11) data.push(pad);

  const nb = NUM_BLOCKS[v - 1], ecl = EC_PER_BLOCK[v - 1], raw = Math.floor(rawModules(v) / 8);
  const nShort = nb - (raw % nb), shortLen = Math.floor(raw / nb);
  const blocks: number[][] = [];
  const div = rsDivisor(ecl);
  for (let i = 0, k = 0; i < nb; i++) {
    const d = data.slice(k, k + shortLen - ecl + (i < nShort ? 0 : 1));
    k += d.length;
    const ecc = rsRemainder(d, div);
    if (i < nShort) d.push(0);
    blocks.push(d.concat(ecc));
  }
  const out: number[] = [];
  for (let i = 0; i < blocks[0].length; i++) blocks.forEach((b, j) => { if (i !== shortLen - ecl || j >= nShort) out.push(b[i]); });
  return out;
}

function penalty(m: boolean[][]): number {
  const n = m.length;
  let p = 0, dark = 0;
  const finder = (line: boolean[]) => {
    let c = 0;
    for (let i = 0; i + 11 <= n; i++) {
      const a = [1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0], b = [0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1];
      if (a.every((x, k) => line[i + k] === !!x) || b.every((x, k) => line[i + k] === !!x)) c += 40;
    }
    return c;
  };
  const runs = (line: boolean[]) => {
    let c = 0, run = 1;
    for (let i = 1; i <= n; i++) {
      if (i < n && line[i] === line[i - 1]) run++;
      else { if (run >= 5) c += 3 + run - 5; run = 1; }
    }
    return c;
  };
  for (let y = 0; y < n; y++) {
    const col = m.map((r) => r[y]);
    p += runs(m[y]) + runs(col) + finder(m[y]) + finder(col);
    for (let x = 0; x < n; x++) {
      if (m[y][x]) dark++;
      if (x + 1 < n && y + 1 < n && m[y][x] === m[y][x + 1] && m[y][x] === m[y + 1][x] && m[y][x] === m[y + 1][x + 1]) p += 3;
    }
  }
  const total = n * n;
  return p + (Math.ceil(Math.abs(dark * 20 - total * 10) / total) - 1) * 10;
}

const MASKS: ((x: number, y: number) => boolean)[] = [
  (x, y) => (x + y) % 2 === 0, (_x, y) => y % 2 === 0, (x) => x % 3 === 0, (x, y) => (x + y) % 3 === 0,
  (x, y) => (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0, (x, y) => ((x * y) % 2) + ((x * y) % 3) === 0,
  (x, y) => ((((x * y) % 2) + ((x * y) % 3)) % 2) === 0, (x, y) => ((((x + y) % 2) + ((x * y) % 3)) % 2) === 0,
];

/** Encode text; returns the module matrix (true = dark), without the quiet zone. */
export function encodeQr(text: string): boolean[][] {
  const bytes = new TextEncoder().encode(text);
  let v = 1;
  while (v <= MAX_VERSION && 4 + (v < 10 ? 8 : 16) + bytes.length * 8 > dataCodewords(v) * 8) v++;
  if (v > MAX_VERSION) throw new Error('Text too long for the built-in QR encoder');
  const size = v * 4 + 17;
  const m: boolean[][] = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  const fn: boolean[][] = Array.from({ length: size }, () => new Array<boolean>(size).fill(false));
  const set = (x: number, y: number, d: boolean) => { m[y][x] = d; fn[y][x] = true; };

  for (let i = 0; i < size; i++) { set(6, i, i % 2 === 0); set(i, 6, i % 2 === 0); }
  const finder = (cx: number, cy: number) => {
    for (let dy = -4; dy <= 4; dy++) for (let dx = -4; dx <= 4; dx++) {
      const d = Math.max(Math.abs(dx), Math.abs(dy)), x = cx + dx, y = cy + dy;
      if (x >= 0 && x < size && y >= 0 && y < size) set(x, y, d !== 2 && d !== 4);
    }
  };
  finder(3, 3); finder(size - 4, 3); finder(3, size - 4);
  const na = v === 1 ? 0 : Math.floor(v / 7) + 2;
  const pos: number[] = v === 1 ? [] : [6];
  if (v > 1) { const step = Math.ceil((v * 4 + 4) / (na * 2 - 2)) * 2; for (let p = size - 7; pos.length < na; p -= step) pos.splice(1, 0, p); }
  pos.forEach((cx, i) => pos.forEach((cy, j) => {
    if ((i === 0 && j === 0) || (i === 0 && j === na - 1) || (i === na - 1 && j === 0)) return;
    for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) set(cx + dx, cy + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
  }));
  const formatBits = (mask: number) => {
    const d = mask; // error correction level M has format bits 00
    let r = d;
    for (let i = 0; i < 10; i++) r = (r << 1) ^ ((r >>> 9) * 0x537);
    const b = ((d << 10) | r) ^ 0x5412;
    for (let i = 0; i <= 5; i++) set(8, i, bit(b, i));
    set(8, 7, bit(b, 6)); set(8, 8, bit(b, 7)); set(7, 8, bit(b, 8));
    for (let i = 9; i < 15; i++) set(14 - i, 8, bit(b, i));
    for (let i = 0; i < 8; i++) set(size - 1 - i, 8, bit(b, i));
    for (let i = 8; i < 15; i++) set(8, size - 15 + i, bit(b, i));
    set(8, size - 8, true);
  };
  formatBits(0);
  if (v >= 7) {
    let r = v;
    for (let i = 0; i < 12; i++) r = (r << 1) ^ ((r >>> 11) * 0x1f25);
    const b = (v << 12) | r;
    for (let i = 0; i < 18; i++) { const a = size - 11 + (i % 3), c = Math.floor(i / 3); set(a, c, bit(b, i)); set(c, a, bit(b, i)); }
  }

  const cw = buildCodewords(bytes, v);
  let k = 0;
  for (let right = size - 1; right >= 1; right -= 2) {
    if (right === 6) right = 5;
    for (let vert = 0; vert < size; vert++) for (let j = 0; j < 2; j++) {
      const x = right - j, y = ((right + 1) & 2) === 0 ? size - 1 - vert : vert;
      if (!fn[y][x] && k < cw.length * 8) { m[y][x] = bit(cw[k >>> 3], 7 - (k & 7)); k++; }
    }
  }
  const apply = (mask: number) => { for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (!fn[y][x] && MASKS[mask](x, y)) m[y][x] = !m[y][x]; };
  let best = 0, bestP = Infinity;
  for (let mask = 0; mask < 8; mask++) {
    apply(mask); formatBits(mask);
    const p = penalty(m);
    if (p < bestP) { best = mask; bestP = p; }
    apply(mask);
  }
  apply(best); formatBits(best);
  return m;
}

/** One SVG path for the dark modules (horizontal runs), including a quiet zone of `margin` modules. */
export function qrPath(m: boolean[][], margin = 3): { d: string; size: number } {
  let d = '';
  m.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) {
      if (!row[x]) continue;
      let e = x;
      while (e < row.length && row[e]) e++;
      d += `M${x + margin} ${y + margin}h${e - x}v1h-${e - x}z`;
      x = e;
    }
  });
  return { d, size: m.length + margin * 2 };
}
