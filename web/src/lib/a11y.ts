// Accessibility preferences (Settings > Accessibility): kept with the other preferences in this browser and applied as
// data-* attributes on <html>, which app.css turns into larger text, stronger focus rings, underlined links, wider text
// spacing and fewer visual effects.
export const TEXT_SIZES = [100, 115, 130, 150] as const;
export interface A11y {
  textSize: (typeof TEXT_SIZES)[number];
  focus: 'standard' | 'strong';
  links: 'hover' | 'always';
  spacing: 'standard' | 'wide';
  effects: 'standard' | 'reduce';
  contrast: 'system' | 'ignore';
}
export const A11Y_DEFAULTS: A11y = { textSize: 100, focus: 'standard', links: 'hover', spacing: 'standard', effects: 'standard', contrast: 'system' };

/** The theme to show: "system" follows light/dark, and becomes high contrast when the system asks for more contrast. */
export function resolveTheme(theme: string, light: boolean, moreContrast: boolean, contrast: A11y['contrast']): string {
  if (theme !== 'system') return theme;
  if (moreContrast && contrast === 'system') return 'contrast';
  return light ? 'light' : 'dark';
}

/** The data-* attributes for <html>. A stored value that is not an option matches no rule in app.css, so it acts as the default. */
export const a11yData = (a: A11y): Record<string, string> => ({ text: '' + a.textSize, focus: a.focus, links: a.links, spacing: a.spacing, effects: a.effects });
