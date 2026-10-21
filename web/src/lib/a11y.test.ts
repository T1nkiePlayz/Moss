import { describe, expect, it } from 'vitest';
import { A11Y_DEFAULTS, a11yData, resolveTheme } from './a11y';

describe('accessibility preferences', () => {
  it('follows the system contrast request only for the system theme, unless told to ignore it', () => {
    expect(resolveTheme('system', true, false, 'system')).toBe('light');
    expect(resolveTheme('system', false, false, 'system')).toBe('dark');
    expect(resolveTheme('system', true, true, 'system')).toBe('contrast');
    expect(resolveTheme('system', true, true, 'ignore')).toBe('light');
    expect(resolveTheme('dark', false, true, 'system')).toBe('dark');
  });

  it('maps to the attributes app.css reads', () => {
    expect(a11yData({ ...A11Y_DEFAULTS, textSize: 150, effects: 'reduce' })).toEqual({ text: '150', focus: 'standard', links: 'hover', spacing: 'standard', effects: 'reduce' });
  });
});
