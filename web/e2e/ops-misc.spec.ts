import { expect, test, type Page } from '@playwright/test';
import { createRequire } from 'node:module';

const axeSource = createRequire(import.meta.url).resolve('axe-core/axe.min.js');
async function audit(page: Page, label: string) {
  await page.addScriptTag({ path: axeSource });
  const result = await page.evaluate(async () => {
    // @ts-expect-error injected above
    const r = await window.axe.run(document, { runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa'] } });
    return r.violations.map((v: { id: string; impact: string; nodes: { target: unknown[] }[] }) => ({ id: v.id, impact: v.impact, nodes: v.nodes.slice(0, 3).map((n) => n.target.join(' ')) }));
  });
  expect(result.filter((v: { impact: string }) => v.impact === 'serious' || v.impact === 'critical'), `${label}: accessibility violations`).toEqual([]);
}

for (const scheme of ['light', 'dark'] as const) {
  test.describe(`${scheme} theme`, () => {
    test.use({ colorScheme: scheme });
    test.setTimeout(20000);

    test('Settings: custom colours are checked for contrast, applied, exported and reset', async ({ page }) => {
      await page.goto('/#/settings');
      const card = page.getByRole('region', { name: 'Custom colours' });
      await expect(card).toBeVisible();
      await audit(page, 'settings with theme editor');
      await card.getByRole('radio', { name: 'Light' }).click();
      const apply = card.getByRole('button', { name: 'Apply colours' });
      await expect(apply).toBeDisabled(); // nothing changed yet
      // a faint text colour fails AA and blocks apply
      await card.getByLabel('Text as hex').fill('#f0f0f0');
      await card.getByLabel('Text as hex').press('Tab');
      await expect(card.getByText('Too faint').first()).toBeVisible();
      await expect(card.getByRole('alert').filter({ hasText: "Can't apply yet" })).toBeVisible();
      await expect(apply).toBeDisabled();
      // a readable one applies
      await card.getByLabel('Text as hex').fill('#101820');
      await card.getByLabel('Text as hex').press('Tab');
      await expect(apply).toBeEnabled();
      await apply.click();
      await expect(page.getByText('Colours applied')).toBeVisible();
      const stored = await page.evaluate(() => localStorage.getItem('moss.theme'));
      expect(stored).toContain('#101820');
      // export shows JSON that can be loaded back
      await card.getByText('Share or import as JSON').click();
      await card.getByRole('button', { name: 'Show my colours as JSON' }).click();
      await expect(card.getByLabel('Theme JSON')).toHaveValue(/moss-theme/);
      await card.getByLabel('Theme JSON').fill('{"format":"nope"}');
      await card.getByRole('button', { name: 'Load pasted colours' }).click();
      await expect(card.getByRole('alert').filter({ hasText: /theme/i })).toBeVisible();
      await audit(page, 'settings with applied custom colours');
      // back to standard
      await card.getByRole('button', { name: 'Use standard colours' }).click();
      expect(await page.evaluate(() => localStorage.getItem('moss.theme'))).toBeNull();
    });

    test('Servers: a search and filter can be saved as a view and reapplied', async ({ page }) => {
      await page.goto('/#/servers');
      await page.getByRole('button', { name: 'Running now', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Running', exact: false }).first()).toBeVisible();
      await page.getByLabel('Search servers').fill('survival');
      await page.getByRole('button', { name: 'Save view' }).click();
      await page.getByLabel('Name for this view').fill('Survival only');
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Survival only', exact: true })).toBeVisible();
      await page.getByLabel('Search servers').fill('');
      await page.getByRole('button', { name: 'Survival only', exact: true }).click();
      await expect(page.getByLabel('Search servers')).toHaveValue('survival');
      await page.getByRole('button', { name: 'Remove view Survival only' }).click();
      await expect(page.getByRole('button', { name: 'Survival only', exact: true })).toHaveCount(0);
      await audit(page, 'servers with saved views');
    });

    test('Import: look at an archive, review what was found, import it', async ({ page }) => {
      await page.goto('/#/servers/import');
      await audit(page, 'import page');
      await page.getByLabel('Server archive').setInputFiles({ name: 'friends-world.zip', mimeType: 'application/zip', buffer: Buffer.from('PK') });
      await page.getByRole('button', { name: 'Look at it' }).click();
      await expect(page.getByRole('heading', { name: 'What Moss found' })).toBeVisible();
      await expect(page.getByLabel('Name', { exact: true })).toHaveValue('friends-world');
      await audit(page, 'import review');
      const go = page.getByRole('button', { name: 'Import server' });
      await expect(go).toBeDisabled(); // EULA not accepted
      await page.getByLabel('Port').fill('25565');
      await page.getByRole('checkbox').check();
      await go.click();
      await expect(page.getByRole('alert').filter({ hasText: 'already used' })).toBeVisible();
      await page.getByLabel('Port').fill('25830');
      await go.click();
      await expect(page.getByRole('heading', { name: /Importing|Server imported/ })).toBeVisible();
    });

    test('Import: a system folder path is refused with a reason', async ({ page }) => {
      await page.goto('/#/servers/import');
      await page.getByRole('button', { name: 'Folder on this machine' }).click();
      await page.getByLabel('Folder path').fill('/etc');
      await page.getByRole('button', { name: 'Look at it' }).click();
      await expect(page.getByRole('alert').filter({ hasText: 'system folder' })).toBeVisible();
    });

    test('Blueprints: save from a server, see it in Settings, start a server from it', async ({ page }) => {
      await page.goto('/#/servers/srv-survival-smp/settings');
      await page.getByLabel('Blueprint name').fill('My survival recipe');
      await page.getByRole('button', { name: 'Save blueprint' }).click();
      await expect(page.getByText('Saved blueprint “My survival recipe”')).toBeVisible();
      await page.goto('/#/settings');
      const card = page.getByRole('region', { name: 'Server blueprints' });
      await expect(card.getByRole('cell', { name: 'My survival recipe', exact: true })).toBeVisible();
      await audit(page, 'settings with blueprints');
      await page.goto('/#/servers/new');
      await page.getByRole('button', { name: 'Use a blueprint' }).click();
      await page.getByLabel('Blueprint', { exact: true }).selectOption({ index: 1 });
      await expect(page.getByLabel('Name for the new server')).toHaveValue('My survival recipe');
      await page.getByLabel('Name for the new server').fill('Recipe test');
      await page.getByRole('checkbox').check();
      await page.getByRole('button', { name: 'Create from blueprint' }).click();
      await expect(page.getByRole('heading', { name: /Setting things up|Server ready/ })).toBeVisible();
    });

    test('Blueprints: share code copy, paste, refusal and use on the New server page', async ({ page, context }) => {
      await context.grantPermissions(['clipboard-read', 'clipboard-write']).catch(() => {});
      await page.goto('/#/servers/srv-survival-smp/settings');
      await page.getByLabel('Blueprint name').fill('Shared recipe');
      await page.getByRole('button', { name: 'Save blueprint' }).click();
      await expect(page.getByText('Saved blueprint “Shared recipe”')).toBeVisible();
      await page.goto('/#/settings');
      const card = page.getByRole('region', { name: 'Server blueprints' });
      await card.getByRole('button', { name: 'Copy share code for Shared recipe' }).click();
      await expect(page.getByText(/Share code for .* copied|Copy the code from the box/)).toBeVisible();
      const box = card.getByLabel('Share code', { exact: true });
      let code = '';
      if (await box.isVisible().catch(() => false)) code = await box.inputValue();
      else code = await page.evaluate(() => navigator.clipboard.readText());
      expect(code.startsWith('moss1_')).toBe(true);
      await card.getByText('Import a blueprint or share code').click();
      await card.getByLabel('Or paste its JSON or a share code').fill(code.slice(0, -4) + '!!!!');
      await card.getByRole('button', { name: 'Import', exact: true }).click();
      await expect(card.getByRole('alert')).toContainText('share code');
      await card.getByLabel('Or paste its JSON or a share code').fill('  ' + code + '\n');
      await card.getByRole('button', { name: 'Import', exact: true }).click();
      await expect(card.getByRole('cell', { name: 'Shared recipe', exact: true })).toHaveCount(2);
      await audit(page, 'settings with share code');
      await page.goto('/#/servers/new');
      await page.getByRole('button', { name: 'Use a blueprint' }).click();
      await page.getByLabel('Or paste a share code').fill(code);
      await page.getByRole('button', { name: 'Use code' }).click();
      await expect(page.getByLabel('Name for the new server')).toHaveValue('Shared recipe');
    });

    test('Command palette finds files and shows the groups', async ({ page }) => {
      await page.goto('/');
      await page.locator('a[href^="#/servers/"]').first().waitFor();
      await page.keyboard.press('Control+k');
      const box = page.getByRole('combobox', { name: 'Search' });
      await box.fill('server.prop');
      await expect(page.getByRole('option').filter({ hasText: 'server.properties' }).first()).toBeVisible();
      await expect(page.locator('.grp', { hasText: 'Files' })).toBeVisible();
      await audit(page, 'palette with results');
    });
  });
}
