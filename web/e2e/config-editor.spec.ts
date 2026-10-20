import { expect, test, type Page } from '@playwright/test';
import { createRequire } from 'node:module';

const axeSource = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
async function audit(page: Page, label: string) {
  await page.addScriptTag({ path: axeSource });
  const bad = await page.evaluate(async () => {
    // @ts-expect-error injected above
    const r = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] } });
    return r.violations.filter((v: { impact: string }) => v.impact === "serious" || v.impact === "critical").map((v: { id: string; nodes: { target: unknown[]; any: { data: unknown }[] }[] }) => v.id + " " + v.nodes.slice(0, 4).map((n) => n.target.join(" ") + JSON.stringify(n.any[0]?.data)).join(" | "));
  });
  expect(bad, `${label}: accessibility violations`).toEqual([]);
}

for (const scheme of ['light', 'dark'] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test('Files > Configuration edits a setting, keeps history and shows the diff', async ({ page }) => {
      await page.goto('/#/servers/srv-survival-smp/files');
      await page.getByRole('button', { name: 'Configuration' }).click();
      const joins = page.getByLabel('max-joins-per-tick');
      await expect(joins).toHaveValue('3');
      await audit(page, 'configuration form');

      await joins.fill('0');
      await expect(page.getByText('Must be at least 1.')).toBeVisible();
      await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled();
      await joins.fill('7');
      await page.getByRole('button', { name: 'Save' }).click();
      await expect(joins).toHaveValue('7');

      await page.getByRole('button', { name: /^History/ }).click();
      await expect(page.getByText('Edited config/paper-global.yml').first()).toBeVisible();
      await expect(page.getByRole('region', { name: /Differences/ })).toContainText('max-joins-per-tick: 7');
      await audit(page, 'configuration history');

      await page.getByRole('button', { name: 'Raw' }).click();
      await expect(page.getByLabel('config/paper-global.yml')).toHaveValue(/# Paper global configuration/);
      await audit(page, 'configuration raw');
    });
  });
}
