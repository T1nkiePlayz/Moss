import { describe, expect, it } from 'vitest';
import { bars, heatmap } from './analytics';

const at = (y: number, mo: number, d: number, h: number) => new Date(y, mo, d, h).toISOString();

describe('analytics helpers', () => {
  it('keeps hourly bars for a week and folds longer ranges into local days', () => {
    const hourly = [{ hour: at(2026, 0, 5, 1), peak: 2, avg: 1 }, { hour: at(2026, 0, 5, 2), peak: 6, avg: 3 }, { hour: at(2026, 0, 6, 1), peak: 1, avg: 1 }];
    expect(bars(hourly, 7)).toHaveLength(3);
    const d = bars(hourly, 30);
    expect(d).toHaveLength(2);
    expect(d[0]).toMatchObject({ peak: 6, avg: 2 });
  });

  it('puts a Monday 20:00 sample in row 0, column 20 and averages repeats', () => {
    const h = heatmap([{ hour: at(2026, 0, 5, 20), peak: 4, avg: 4 }, { hour: at(2026, 0, 12, 20), peak: 2, avg: 2 }, { hour: at(2026, 0, 11, 3), peak: 1, avg: 1 }]);
    expect(h.cells[0][20]).toBe(3);
    expect(h.cells[6][3]).toBe(1);
    expect(h.cells[1][0]).toBeNull();
    expect(h.max).toBe(3);
  });
});
