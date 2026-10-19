/** Subsequence fuzzy score; higher is better, -1 means no match. */
export function fuzzy(query: string, text: string): number {
  const q = query.toLowerCase().trim();
  if (!q) return 0;
  const t = text.toLowerCase();
  let ti = 0;
  let score = 0;
  let streak = 0;
  for (const ch of q) {
    const idx = t.indexOf(ch, ti);
    if (idx < 0) return -1;
    streak = idx === ti ? streak + 1 : 0;
    score += 1 + streak * 2 + (idx === 0 || /[\s/_-]/.test(t[idx - 1]) ? 4 : 0);
    ti = idx + 1;
  }
  return score - t.length * 0.01;
}
