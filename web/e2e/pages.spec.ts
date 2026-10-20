import { expect, test, type Page } from '@playwright/test';
import { createRequire } from 'node:module';

const axeSource = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
const SERVER_TABS = ['', 'console', 'files', 'config', 'content', 'upgrade', 'backups', 'settings', 'access'];
const PAGES = ['', 'servers', 'servers/new', 'backups', 'activity', 'users', 'settings'];

async function audit(page: Page, label: string) {
  await page.addScriptTag({ path: axeSource });
  const result = await page.evaluate(async () => {
    // @ts-expect-error injected by the script tag above
    const r = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] } });
    return r.violations.map((v: { id: string; impact: string; help: string; nodes: { target: unknown[] }[] }) => ({ id: v.id, impact: v.impact, help: v.help, nodes: v.nodes.slice(0, 3).map((n) => n.target.join(' ')) }));
  });
  const bad = result.filter((v: { impact: string }) => v.impact === 'serious' || v.impact === 'critical');
  expect(bad, `${label}: accessibility violations`).toEqual([]);
}

for (const scheme of ['light', 'dark'] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });

    test('every page renders without console errors and passes axe', async ({ page }) => {
      const errors: string[] = [];
      page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
      page.on('pageerror', (e) => errors.push(String(e)));
      await page.goto('/');
      await page.locator('a[href^="#/servers/"]').first().waitFor();
      const first = (await page.locator('a[href^="#/servers/"]').first().getAttribute('href'))!.replace('#/', '');
      const routes = [...PAGES, ...SERVER_TABS.map((t) => (t ? `${first}/${t}` : first))];
      for (const r of routes) {
        await page.goto(`/#/${r}`);
        await page.locator('main, [role="main"], .page').first().waitFor();
        await page.waitForLoadState('networkidle');
        await audit(page, `/${r}`);
      }
      expect(errors, 'console errors').toEqual([]);
    });
  });
}
