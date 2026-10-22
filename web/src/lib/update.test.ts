import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { markBannerShown, pausableTimer, shouldShowBanner } from './update';

const mem = () => { const m = new Map<string, string>(); return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) }; };

describe('banner session rule', () => {
  it('shows once per version', () => {
    const s = mem();
    expect(shouldShowBanner('v0.1.2', true, s)).toBe(true);
    markBannerShown('v0.1.2', s);
    expect(shouldShowBanner('v0.1.2', true, s)).toBe(false);
    expect(shouldShowBanner('v0.1.3', true, s)).toBe(true);
  });
  it('never shows without an available update', () => {
    expect(shouldShowBanner('v0.1.2', false, mem())).toBe(false);
    expect(shouldShowBanner(null, true, mem())).toBe(false);
  });
  it('survives blocked storage', () => {
    const bad = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } };
    expect(shouldShowBanner('v1', true, bad)).toBe(true);
    expect(() => markBannerShown('v1', bad)).not.toThrow();
    expect(shouldShowBanner('v1', true, null)).toBe(true);
  });
});

describe('pausableTimer', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  it('fires after the delay', () => {
    const f = vi.fn(); pausableTimer(15000, f);
    vi.advanceTimersByTime(14999); expect(f).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1); expect(f).toHaveBeenCalledTimes(1);
  });
  it('pauses and resumes with the remaining time', () => {
    const f = vi.fn(); const t = pausableTimer(15000, f);
    vi.advanceTimersByTime(10000); t.pause();
    vi.advanceTimersByTime(60000); expect(f).not.toHaveBeenCalled();
    t.resume(); vi.advanceTimersByTime(4999); expect(f).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1); expect(f).toHaveBeenCalledTimes(1);
  });
  it('stop cancels', () => {
    const f = vi.fn(); const t = pausableTimer(1000, f); t.stop(); t.resume();
    vi.advanceTimersByTime(5000); expect(f).not.toHaveBeenCalled();
  });
});
