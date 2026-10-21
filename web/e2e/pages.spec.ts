import { expect, test, type Page } from '@playwright/test';
import { createRequire } from 'node:module';

const axeSource = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
const SERVER_TABS = ['', 'console', 'files', 'config', 'content', 'upgrade', 'backups', 'players', 'settings', 'access'];
const PAGES = ['', 'servers', 'servers/new', 'servers/import', 'backups', 'activity', 'workflows', 'users', 'settings'];

async function audit(page: Page, label: string, scope?: string) {
  await page.addScriptTag({ path: axeSource });
  const result = await page.evaluate(async (sel) => {
    // @ts-expect-error injected by the script tag above
    const r = await window.axe.run(sel ? document.querySelector(sel) : document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] } });
    return r.violations.map((v: { id: string; impact: string; help: string; nodes: { target: unknown[] }[] }) => ({ id: v.id, impact: v.impact, help: v.help, nodes: v.nodes.slice(0, 3).map((n) => n.target.join(' ')) }));
  }, scope ?? null);
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

test('the players tab shows populated analytics and switches range', async ({ page }) => {
  await page.goto('/');
  await page.locator('a[href^="#/servers/"]:not([href$="/new"])').first().waitFor();
  const first = (await page.locator('a[href^="#/servers/"]:not([href$="/new"])').first().getAttribute('href'))!.replace('#/', '');
  await page.goto(`/#/${first}/players`);
  await expect(page.getByText('Unique players')).toBeVisible();
  await expect(page.getByText('Alex_Builds').first()).toBeVisible();
  await page.getByRole('button', { name: '30 days' }).click();
  await expect(page.getByRole('button', { name: '30 days' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByText('Unique players')).toBeVisible();
});

test('server settings show the isolation section, and a clear message when no engine exists', async ({ page }) => {
  await page.goto('/');
  await page.locator('a[href^="#/servers/"]:not([href$="/new"])').first().waitFor();
  const first = (await page.locator('a[href^="#/servers/"]:not([href$="/new"])').first().getAttribute('href'))!.replace('#/', '');
  await page.goto(`/#/${first}/settings`);
  await expect(page.getByRole('heading', { name: 'Isolation' }).or(page.getByText('Isolation', { exact: true }))).toBeVisible();
  await expect(page.getByText(/Podman 5\.2\.2 \(rootless\)/)).toBeVisible();
  await page.getByLabel('Mode').selectOption('container');
  await page.getByRole('button', { name: 'Save isolation' }).click();
  await expect(page.getByText('Isolation saved')).toBeVisible();
  await expect(page.getByLabel('Network')).toBeVisible();
  await expect(page.getByRole('option', { name: /Game-only/ })).toBeDisabled(); // mock engine is Podman
  await page.getByLabel('Extra ports').fill('80/tcp');
  await expect(page.getByText(/outside 1024-65535/)).toBeVisible();
  await page.getByLabel('Extra ports').fill('19132/udp, 24454/udp');
  await page.getByRole('button', { name: 'Save isolation' }).click();
  await expect(page.getByLabel('Extra ports')).toHaveValue('19132/udp, 24454/udp');
  await audit(page, 'settings with container mode');
  await page.goto(`/?scenario=noengine#/${first}/settings`);
  await expect(page.getByText('No container engine')).toBeVisible();
  await expect(page.getByText(/Neither Docker nor Podman was found/).first()).toBeVisible();
  await audit(page, 'settings without an engine');
});

test.describe('remote backups', () => {
  test('destinations: add form, test result and per-server choice are accessible', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    await page.goto('/#/settings');
    await expect(page.getByText('Backup destinations', { exact: true })).toBeVisible();
    await expect(page.getByText('Home NAS').first()).toBeVisible();
    await page.getByRole('button', { name: 'Test', exact: true }).first().click();
    await expect(page.getByText('Everything worked')).toBeVisible();
    await expect(page.getByRole('combobox', { name: /Destination for/ }).first()).toBeVisible();
    await page.getByRole('button', { name: 'Add destination' }).click();
    await page.getByRole('radio', { name: /S3-compatible/ }).click();
    await page.getByRole('button', { name: 'Add destination' }).last().click();
    await expect(page.getByText('Enter the service address', { exact: false })).toBeVisible();
    await audit(page, 'add destination dialog');
    expect(errors).toEqual([]);
  });

  test('restore dialog explains a remote-only backup', async ({ page }) => {
    await page.goto('/#/backups');
    await expect(page.getByText('Remote only').first()).toBeVisible();
    await expect(page.getByRole('button', { name: 'Download from remote' }).first()).toBeVisible();
    await page.getByRole('row').filter({ hasText: 'Remote only' }).first().getByRole('button', { name: 'Restore' }).click();
    await expect(page.getByText(/only stored on Home NAS/)).toBeVisible();
    await audit(page, 'restore dialog, remote-only');
  });
});

test('webhooks: add form validates, the secret is shown once, test event and log work', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto('/#/settings');
  await expect(page.getByText('Discord bot').first()).toBeVisible();
  await page.getByRole('button', { name: 'Send test event' }).first().click();
  await expect(page.getByText('Test event delivered')).toBeVisible();
  await page.getByRole('button', { name: 'Show delivery log' }).first().click();
  await expect(page.getByRole('button', { name: 'Send again' }).first()).toBeVisible();
  await audit(page, 'webhooks with delivery log');
  await page.getByRole('button', { name: 'Add webhook' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Add webhook' }).click();
  await expect(page.getByText('Enter the address that should receive events.')).toBeVisible();
  await audit(page, 'add webhook dialog', '[role="dialog"]');
  await page.getByRole('dialog').getByLabel('Name', { exact: true }).fill('Dashboard');
  await page.getByRole('dialog').getByLabel('Address', { exact: true }).fill('https://dash.example.com/moss');
  await page.getByRole('checkbox', { name: /server.crashed/ }).check();
  await page.getByRole('dialog').getByRole('button', { name: 'Add webhook' }).click();
  await expect(page.getByText(/Copy the signing secret for Dashboard now/)).toBeVisible();
  await expect(page.getByLabel('Signing secret')).toHaveValue(/^whsec_/);
  await audit(page, 'webhook secret shown');
  expect(errors).toEqual([]);
});

test('workflows: list, editor with dry run and history, validation and adding one', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto('/#/workflows');
  await expect(page.getByText('Restart after a crash').first()).toBeVisible();
  await expect(page.getByRole('switch', { name: 'Run Welcome new players' })).toBeChecked();
  await audit(page, 'workflows list');
  await page.getByRole('button', { name: 'Edit' }).nth(1).click();
  const dlg = page.getByRole('dialog');
  await dlg.getByRole('button', { name: 'Dry run with a sample event' }).click();
  await expect(dlg.getByText('Would run.')).toBeVisible();
  await expect(dlg.getByRole('list', { name: 'Recent runs of Restart after a crash' })).toBeVisible();
  await audit(page, 'workflow editor', '[role="dialog"]');
  await dlg.getByRole('button', { name: 'Cancel' }).click();
  await page.getByRole('button', { name: 'Add workflow' }).first().click();
  await dlg.getByRole('button', { name: 'Add workflow' }).click();
  await expect(dlg.getByText('Give the workflow a name.')).toBeVisible();
  await audit(page, 'new workflow dialog', '[role="dialog"]');
  await dlg.getByLabel('Name', { exact: true }).fill('Greeter');
  await dlg.getByRole('button', { name: 'Add workflow' }).click();
  await expect(page.getByText('Greeter').first()).toBeVisible();
  expect(errors).toEqual([]);
});

async function openFabricContent(page: Page) {
  await page.goto('/');
  await page.locator('a[href^="#/servers/"]').first().waitFor();
  await page.goto('/#/servers');
  const link = page.getByRole('link', { name: /Modded Fabric/ }).first();
  await link.waitFor();
  const href = (await link.getAttribute('href'))!.replace(/^#?\/?/, '');
  await page.goto(`/#/${href}/content`);
  await page.getByRole('heading', { name: 'Modpacks' }).waitFor();
}

test.describe('modpack versions and staged copies', () => {
  test('a pack update shows its diff, keeps edited files and passes axe', async ({ page }) => {
    await openFabricContent(page);
    await expect(page.getByText('Fabulous Survival')).toBeVisible();
    await page.getByRole('button', { name: 'Check for pack update' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog.getByText('Files you changed (kept as they are)')).toBeVisible();
    await expect(dialog.getByText('config/ferritecore.mixin.properties')).toBeVisible();
    await expect(dialog.getByText('Old Minimap')).toBeVisible();
    await audit(page, 'pack diff', '[role="dialog"]');
    await dialog.getByRole('button', { name: 'Close', exact: true }).last().click();
    await expect(dialog).toBeHidden();
  });

  test('pinning a pack is reflected and a pinned pack is not checked', async ({ page }) => {
    await openFabricContent(page);
    await page.getByRole('button', { name: 'Pin', exact: true }).click();
    await expect(page.getByText('Pinned', { exact: true }).first()).toBeVisible();
    await page.getByRole('button', { name: 'Check for pack update' }).click();
    await expect(page.getByText('This pack is pinned', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Unpin', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Pin', exact: true })).toBeVisible();
  });

  test('a pack update can be tried on a copy, then applied', async ({ page }) => {
    await openFabricContent(page);
    await page.getByRole('button', { name: 'Check for pack update' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Try update on a copy first' }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog.getByText('Include the world')).toBeVisible();
    await audit(page, 'staged setup', '[role="dialog"]');
    await dialog.getByRole('button', { name: 'Start test' }).click();
    await expect(dialog.getByText(/ran without problems/)).toBeVisible({ timeout: 15_000 });
    await expect(dialog.getByText('removed automatically', { exact: false }).or(dialog.getByText('kept for 24 hours'))).toBeVisible();
    await dialog.getByRole('button', { name: 'Show log' }).click();
    await expect(dialog.getByLabel('Console log of the test copy')).toContainText('Done');
    await audit(page, 'staged result', '[role="dialog"]');
    await dialog.getByRole('button', { name: 'Update the pack' }).click();
    await expect(dialog).toBeHidden();
  });
});
