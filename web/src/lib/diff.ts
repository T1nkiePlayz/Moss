// A small line diff (longest common subsequence) for the configuration history view.
export type DiffLine = { kind: 'same' | 'add' | 'del'; text: string; a?: number; b?: number };

export function diffLines(before: string, after: string): DiffLine[] {
  const a = before === '' ? [] : before.split('\n'), b = after === '' ? [] : after.split('\n');
  // trim the common head and tail so large files with one change stay cheap
  let h = 0;
  while (h < a.length && h < b.length && a[h] === b[h]) h++;
  let t = 0;
  while (t < a.length - h && t < b.length - h && a[a.length - 1 - t] === b[b.length - 1 - t]) t++;
  const am = a.slice(h, a.length - t), bm = b.slice(h, b.length - t);
  const out: DiffLine[] = a.slice(0, h).map((text, i) => ({ kind: 'same', text, a: i + 1, b: i + 1 }));
  if (am.length * bm.length > 4_000_000) {
    am.forEach((text, i) => out.push({ kind: 'del', text, a: h + i + 1 }));
    bm.forEach((text, i) => out.push({ kind: 'add', text, b: h + i + 1 }));
  } else {
    const w = bm.length + 1, L = new Uint32Array((am.length + 1) * w);
    for (let i = am.length - 1; i >= 0; i--) for (let j = bm.length - 1; j >= 0; j--) L[i * w + j] = am[i] === bm[j] ? L[(i + 1) * w + j + 1] + 1 : Math.max(L[(i + 1) * w + j], L[i * w + j + 1]);
    let i = 0, j = 0;
    while (i < am.length || j < bm.length) {
      if (i < am.length && j < bm.length && am[i] === bm[j]) { out.push({ kind: 'same', text: am[i], a: h + i + 1, b: h + j + 1 }); i++; j++; }
      else if (j < bm.length && (i === am.length || L[i * w + j + 1] > L[(i + 1) * w + j])) { out.push({ kind: 'add', text: bm[j], b: h + j + 1 }); j++; }
      else { out.push({ kind: 'del', text: am[i], a: h + i + 1 }); i++; }
    }
  }
  a.slice(a.length - t).forEach((text, i) => out.push({ kind: 'same', text, a: a.length - t + i + 1, b: b.length - t + i + 1 }));
  return out;
}

export const changeCount = (d: DiffLine[]) => d.filter((l) => l.kind !== 'same').length;
