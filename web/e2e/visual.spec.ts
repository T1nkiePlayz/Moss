import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Visual regression: key pages in the mock backend, in the light, dark and contrast themes. Everything that could
// differ between runs is pinned: time, randomness, timers, animations and fonts (a small bundled Noto Sans subset
// replaces the system fonts, which differ between machines).
const FIXED_TIME = new Date('2026-03-14T12:00:00Z');
const font = (f: string) => readFileSync(fileURLToPath(new URL(`./fonts/${f}`, import.meta.url)));
const FONT_CSS = `
@font-face{font-family:MossTest;font-weight:100 450;src:url(/__fonts/NotoSans-Regular.woff2)}
@font-face{font-family:MossTest;font-weight:451 599;src:url(/__fonts/NotoSans-Medium.woff2)}
@font-face{font-family:MossTest;font-weight:600 900;src:url(/__fonts/NotoSans-Bold.woff2)}
@font-face{font-family:MossTestMono;src:url(/__fonts/NotoSansMono-Regular.woff2)}
:root{--font:MossTest,sans-serif !important;--mono:MossTestMono,monospace !important}
*,*::before,*::after{animation:none !important;transition:none !important;scroll-behavior:auto !important;caret-color:transparent !important}`;

type Theme = 'light' | 'dark' | 'contrast';
const THEMES: Theme[] = ['light', 'dark', 'contrast'];

async function prepare(page: Page, theme: Theme) {
  await page.route('**/__fonts/*', (r) => r.fulfill({ body: font(new URL(r.request().url()).pathname.split('/').pop()!), contentType: 'font/woff2' }));
  await page.clock.setFixedTime(FIXED_TIME);
  await page.addInitScript(({ css, theme }) => {
    let s = 0x2545f491; // mulberry32: the mock's random numbers (charts, host load) come out the same every run
    Math.random = () => { s = (s + 0x6d2b79f5) | 0; let t = Math.imul(s ^ (s >>> 15), 1 | s); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
    // The mock's live ticks and the delayed demo notification would change the page mid-capture.
    const si = window.setInterval, st = window.setTimeout;
    window.setInterval = ((f: TimerHandler, ms?: number, ...a: unknown[]) => (ms && ms >= 1000 ? 0 : si(f, ms, ...a))) as typeof si;
    window.setTimeout = ((f: TimerHandler, ms?: number, ...a: unknown[]) => (ms && ms >= 5000 ? 0 : st(f, ms, ...a))) as typeof st;
    localStorage.setItem('moss.prefs', JSON.stringify({ theme }));
    document.addEventListener('DOMContentLoaded', () => { const el = document.createElement('style'); el.textContent = css; document.head.append(el); });
  }, { css: FONT_CSS, theme });
}

async function settle(page: Page) {
  await page.waitForLoadState('networkidle');
  await page.evaluate(async () => { await document.fonts.ready; await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))); });
  // The mock answers after short random delays: wait until the page has stopped changing.
  let last = '', same = 0;
  for (let i = 0; i < 100 && same < 5; i++) {
    const now = await page.evaluate(() => document.body.innerHTML.length + ':' + document.body.innerText.length);
    same = now === last ? same + 1 : 0;
    last = now;
    await page.waitForTimeout(100);
  }
  expect(await page.evaluate(() => document.fonts.check('16px MossTest')), 'test font loaded').toBe(true);
}

async function firstServer(page: Page) {
  await page.goto('/');
  const link = page.locator('a[href^="#/servers/"]:not([href$="/new"])').first();
  await link.waitFor({ state: 'attached' });
  return (await link.getAttribute('href'))!.replace('#/', '');
}

for (const theme of THEMES) {
  test.describe(`${theme} theme`, () => {
    test.beforeEach(async ({ page }) => { await prepare(page, theme); });

    const shot = async (page: Page, name: string, o: { clip?: string } = {}) => {
      await settle(page);
      await expect(o.clip ? page.locator(o.clip) : page).toHaveScreenshot(`${name}-${theme}.png`);
    };

    test('overview', async ({ page }) => {
      await page.goto('/#/');
      await page.locator('a[href^="#/servers/"]').first().waitFor({ state: 'attached' });
      await shot(page, 'overview');
    });

    // The contrast theme only changes colours, so it covers the overview, server overview, palette and a dialog;
    // the busier pages are covered in light and dark. This keeps the committed baselines small.
    const extra = theme !== 'contrast';
    for (const tab of ['', 'console', 'files', 'content', 'backups'].filter((t) => extra || !t)) {
      test(`server ${tab || 'overview'}`, async ({ page }, info) => {
        test.skip(theme === 'light' && (tab === 'backups' || tab === 'content'), 'light backups and content are covered by the axe suite; keeps the baselines small');
        test.skip(info.project.name === 'phone' && ['files', 'content', 'backups'].includes(tab), 'the phone layout of these tabs is covered by the axe suite');
        const id = await firstServer(page);
        await page.goto(`/#/${id}${tab ? `/${tab}` : ''}`);
        await page.locator('main .page').waitFor();
        await shot(page, `server-${tab || 'overview'}`);
      });
    }

    (extra ? test : test.skip)('settings appearance', async ({ page }) => {
      await page.goto('/#/settings');
      await expect(page.getByText('Appearance', { exact: true })).toBeVisible();
      await shot(page, 'settings-appearance', { clip: '.card:has(#ap)' });
    });

    (extra ? test : test.skip)('setup', async ({ page }) => {
      await page.goto('/?scenario=setup#/');
      await page.getByRole('heading').first().waitFor();
      await shot(page, 'setup');
    });

    test('command palette', async ({ page }) => {
      await page.goto('/#/');
      await page.locator('a[href^="#/servers/"]').first().waitFor({ state: 'attached' });
      await page.keyboard.press('Control+k');
      await page.getByRole('dialog').waitFor();
      await shot(page, 'palette');
    });

    test('modal', async ({ page }) => {
      await page.goto('/#/settings');
      await page.getByRole('button', { name: 'Add destination' }).click();
      await page.getByRole('dialog').waitFor();
      await shot(page, 'modal');
    });
  });
}
