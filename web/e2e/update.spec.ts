import { expect, test } from '@playwright/test';

test.describe('Moss self-update', () => {
  test('banner appears, auto-hides after 15 s and shows once per session', async ({ page }) => {
    await page.clock.install();
    await page.goto('/#/');
    const banner = page.getByRole('status', { name: 'Moss update' });
    await page.clock.runFor(5000);
    await expect(banner).toBeVisible();
    await expect(banner).toContainText('Moss v0.1.2 is available');
    await expect(banner.getByRole('button', { name: 'Update', exact: true })).toBeVisible();
    await page.clock.runFor(16000);
    await expect(banner).toBeHidden();
    await page.reload();
    await page.clock.runFor(8000);
    await expect(banner).toBeHidden(); // same version, same session
  });

  test('banner stays while hovered', async ({ page }) => {
    await page.clock.install();
    await page.goto('/#/');
    const banner = page.getByRole('status', { name: 'Moss update' });
    await page.clock.runFor(5000);
    await expect(banner).toBeVisible();
    await banner.hover();
    await page.clock.runFor(30000);
    await expect(banner).toBeVisible();
  });

  test('Details opens the Settings Updates section; update confirms, restarts and reloads', async ({ page }) => {
    await page.goto('/#/');
    const banner = page.getByRole('status', { name: 'Moss update' });
    await banner.getByRole('button', { name: 'Details' }).click({ timeout: 15000 });
    const card = page.getByRole('region', { name: 'Updates' });
    await expect(card).toBeVisible();
    await expect(card).toContainText('0.1.0-mock');
    await expect(card).toContainText('v0.1.2');
    await expect(card.getByRole('link', { name: 'Release notes' })).toBeVisible();
    await card.getByRole('button', { name: 'Check now' }).click();
    await card.getByRole('button', { name: 'Update to v0.1.2' }).click();
    const dlg = page.getByRole('dialog');
    await expect(dlg).toContainText('Moss will download, verify and install v0.1.2, then restart. Game servers keep running.');
    await dlg.getByRole('button', { name: 'Update and restart' }).click();
    await expect(page.getByText('Restarting Moss…').first()).toBeVisible({ timeout: 15000 });
    await page.waitForEvent('load', { timeout: 30000 });
    await expect(page.getByRole('region', { name: 'Updates' })).toContainText('up to date', { timeout: 15000 });
  });
});
