// Apply stored theme before first paint to avoid a flash.
try {
  var p = JSON.parse(localStorage.getItem('moss.prefs') || '{}');
  var t = p.theme || 'system';
  if (t === 'system') t = matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  document.documentElement.dataset.theme = t;
  if (p.accent) document.documentElement.dataset.accent = p.accent;
  if (p.density) document.documentElement.dataset.density = p.density;
} catch (e) {}
