import { describe, expect, it } from 'vitest';
import { changeCount, diffLines } from './diff';

describe('diffLines', () => {
  it('reports no changes for equal text', () => {
    expect(changeCount(diffLines('a\nb\n', 'a\nb\n'))).toBe(0);
  });
  it('finds a replaced line with numbers on both sides', () => {
    const d = diffLines('a\nb\nc', 'a\nB\nc');
    expect(d.map((l) => l.kind)).toEqual(['same', 'del', 'add', 'same']);
    expect(d[1]).toMatchObject({ text: 'b', a: 2 });
    expect(d[2]).toMatchObject({ text: 'B', b: 2 });
  });
  it('handles insertions, removals and an empty side', () => {
    expect(changeCount(diffLines('a\nc', 'a\nb\nc'))).toBe(1);
    expect(diffLines('x', '').map((l) => l.kind)).toEqual(['del']);
    expect(diffLines('', 'x').map((l) => l.kind)).toEqual(['add']);
  });
});
