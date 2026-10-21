import { test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Captures the README screenshots from the mock backend: a mix of Minecraft and other games, dark theme, pinned time
// and fonts so reruns only change when the UI does. Run `npm run screenshots`; output goes to docs/screenshots.
const OUT = fileURLToPath(new URL('../../docs/screenshots/', import.meta.url));
const font = (f: string) => readFileSync(fileURLToPath(new URL(`./fonts/${f}`, import.meta.url)));
const FONT_CSS = `
@font-face{font-family:MossShot;font-weight:100 450;src:url(/__fonts/NotoSans-Regular.woff2)}
@font-face{font-family:MossShot;font-weight:451 599;src:url(/__fonts/NotoSans-Medium.woff2)}
@font-face{font-family:MossShot;font-weight:600 900;src:url(/__fonts/NotoSans-Bold.woff2)}
@font-face{font-family:MossShotMono;src:url(/__fonts/NotoSansMono-Regular.woff2)}
:root{--font:MossShot,sans-serif !important;--mono:MossShotMono,monospace !important}
*,*::before,*::after{animation:none !important;transition:none !important;caret-color:transparent !important}`;

async function prepare(page: Page) {
  await page.route('**/__fonts/*', (r) => r.fulfill({ body: font(new URL(r.request().url()).pathname.split('/').pop()!), contentType: 'font/woff2' }));
  await page.route('https://gravatar.com/**', (r) => r.abort()); // no network: the avatar falls back to initials
  await page.clock.setFixedTime(new Date('2026-10-21T15:00:00Z'));
  await page.addInitScript((css) => {
    let s = 0x2545f491;
    Math.random = () => { s = (s + 0x6d2b79f5) | 0; let t = Math.imul(s ^ (s >>> 15), 1 | s); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
    const st = window.setTimeout;
    window.setTimeout = ((f: TimerHandler, ms?: number, ...a: unknown[]) => (ms && ms >= 5000 ? 0 : st(f, ms, ...a))) as typeof st;
    localStorage.setItem('moss.prefs', JSON.stringify({ theme: 'dark' }));
    document.addEventListener('DOMContentLoaded', () => { const el = document.createElement('style'); el.textContent = css; document.head.append(el); });
  }, FONT_CSS);
}

async function settle(page: Page) {
  await page.waitForLoadState('networkidle');
  await page.evaluate(async () => { await document.fonts.ready; });
  let last = '', same = 0;
  for (let i = 0; i < 100 && same < 5; i++) {
    const now = await page.evaluate(() => document.body.innerHTML.length + ':' + document.body.innerText.length);
    same = now === last ? same + 1 : 0;
    last = now;
    await page.waitForTimeout(100);
  }
}

async function serverPath(page: Page, name: string) {
  await page.goto('/#/servers');
  const link = page.locator(`a[href^="#/servers/"]`, { hasText: name }).first();
  await link.waitFor({ state: 'attached' });
  return (await link.getAttribute('href'))!.replace('#/', '');
}

/** Scroll so `heading` sits `gap` px below the sticky top bar. */
async function scrollTo(page: Page, heading: string, gap = 90) {
  await page.getByRole('heading', { name: heading, exact: true }).first().evaluate((el, gap) => {
    let p: HTMLElement | null = el.parentElement;
    while (p && !(p.scrollHeight > p.clientHeight && /(auto|scroll)/.test(getComputedStyle(p).overflowY))) p = p.parentElement;
    const sc = p ?? (document.scrollingElement as HTMLElement);
    sc.scrollTop += el.getBoundingClientRect().top - gap;
  }, gap);
}

const shot = async (page: Page, name: string) => { await settle(page); await page.screenshot({ path: `${OUT}${name}.png` }); };

test.beforeEach(async ({ page }) => { await prepare(page); });

test('overview', async ({ page }) => {
  await page.goto('/#/');
  await page.locator('a[href^="#/servers/"]').first().waitFor({ state: 'attached' });
  await settle(page);
  await scrollTo(page, 'Servers', 192);
  await shot(page, 'overview');
});

test('servers', async ({ page }) => {
  await page.goto('/#/servers');
  await page.locator('a[href^="#/servers/"]').first().waitFor({ state: 'attached' });
  await shot(page, 'servers');
});

test('new server', async ({ page }) => {
  await page.goto('/#/servers/new');
  await page.getByRole('heading', { name: 'Other games' }).waitFor();
  await settle(page);
  await scrollTo(page, 'Other games');
  await shot(page, 'new-server');
});

test('server overview (Valheim)', async ({ page }) => {
  const p = await serverPath(page, 'Valheim');
  await page.goto(`/#/${p}`);
  await page.locator('main .page').waitFor();
  await shot(page, 'server');
});

test('console (CS2)', async ({ page }) => {
  const p = await serverPath(page, 'CS2 Wingman');
  await page.goto(`/#/${p}/console`);
  await page.locator('main .page').waitFor();
  await shot(page, 'console');
});

test('player analytics', async ({ page }) => {
  const p = await serverPath(page, 'Survival SMP');
  await page.goto(`/#/${p}/players`);
  await page.locator('main .page').waitFor();
  await shot(page, 'players');
});

test('plugins (Hangar)', async ({ page }) => {
  const p = await serverPath(page, 'Survival SMP');
  await page.goto(`/#/${p}/content`);
  await page.locator('main .page').waitFor();
  await page.getByRole('button', { name: /^Browse/ }).first().click();
  await page.getByRole('radio', { name: 'Hangar' }).click();
  await shot(page, 'plugins');
});

test('backups', async ({ page }) => {
  await page.goto('/#/backups');
  await page.locator('main .page').waitFor();
  await shot(page, 'backups');
});

test('workflows', async ({ page }) => {
  await page.goto('/#/workflows');
  await page.locator('main .page').waitFor();
  await shot(page, 'workflows');
});
