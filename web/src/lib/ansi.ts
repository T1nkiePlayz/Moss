// Minimal, safe ANSI SGR parser. Output is structured data rendered as text nodes
// (never innerHTML), and colours are built only from validated numbers.
export interface Seg {
  text: string;
  fg?: string; // css colour
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
}
export type Level = 'info' | 'warn' | 'error' | 'debug' | 'chat';

const BASIC = ['#3b4a41', '#ff6b5e', '#9be37d', '#f2b04c', '#6aa3ff', '#d08cff', '#5cc8e8', '#d7e3db'];
const BRIGHT = ['#8a9d92', '#ff8f85', '#b9f2a0', '#ffd079', '#93bdff', '#e2b0ff', '#8ee0f5', '#ffffff'];

function c256(n: number): string | undefined {
  if (n < 0 || n > 255) return undefined;
  if (n < 8) return BASIC[n];
  if (n < 16) return BRIGHT[n - 8];
  if (n >= 232) {
    const v = 8 + (n - 232) * 10;
    return `rgb(${v},${v},${v})`;
  }
  const i = n - 16;
  const f = (x: number) => (x === 0 ? 0 : 55 + x * 40);
  return `rgb(${f(Math.floor(i / 36))},${f(Math.floor(i / 6) % 6)},${f(i % 6)})`;
}

// eslint-disable-next-line no-control-regex
const RE = /\x1b\[([0-9;]*)m|\x1b\[[0-9;?]*[A-Za-ln-z]/g;

export function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '');
}

export function parseAnsi(line: string): Seg[] {
  if (!line.includes('\x1b')) return [{ text: line }];
  const out: Seg[] = [];
  let st: Omit<Seg, 'text'> = {};
  let last = 0;
  let m: RegExpExecArray | null;
  RE.lastIndex = 0;
  while ((m = RE.exec(line))) {
    if (m.index > last) out.push({ text: line.slice(last, m.index), ...st });
    last = RE.lastIndex;
    if (m[1] === undefined) continue; // non-SGR escape: dropped
    const codes = m[1] === '' ? [0] : m[1].split(';').map((x) => parseInt(x, 10) || 0);
    st = { ...st };
    for (let i = 0; i < codes.length; i++) {
      const c = codes[i];
      if (c === 0) st = {};
      else if (c === 1) st.bold = true;
      else if (c === 2) st.dim = true;
      else if (c === 3) st.italic = true;
      else if (c === 4) st.underline = true;
      else if (c === 22) st.bold = st.dim = false;
      else if (c === 23) st.italic = false;
      else if (c === 24) st.underline = false;
      else if (c >= 30 && c <= 37) st.fg = BASIC[c - 30];
      else if (c >= 90 && c <= 97) st.fg = BRIGHT[c - 90];
      else if (c === 39) st.fg = undefined;
      else if (c === 38 || c === 48) {
        const mode = codes[i + 1];
        let col: string | undefined;
        if (mode === 5) {
          col = c256(codes[i + 2]);
          i += 2;
        } else if (mode === 2) {
          const [r, g, b] = [codes[i + 2], codes[i + 3], codes[i + 4]].map((x) => clampByte(x));
          col = `rgb(${r},${g},${b})`;
          i += 4;
        }
        if (c === 38 && col) st.fg = col;
      }
    }
  }
  if (last < line.length) out.push({ text: line.slice(last), ...st });
  return out.length ? out : [{ text: '' }];
}

const clampByte = (x: number) => Math.min(255, Math.max(0, x | 0));

export function levelOf(line: string): Level {
  const s = line.length > 200 ? line.slice(0, 200) : line;
  if (/\b(ERROR|SEVERE|FATAL)\b|Exception/.test(s)) return 'error';
  if (/\bWARN(ING)?\b/.test(s)) return 'warn';
  if (/\bDEBUG\b|\bTRACE\b/.test(s)) return 'debug';
  if (/\]: <[^>]+> /.test(s)) return 'chat';
  return 'info';
}
