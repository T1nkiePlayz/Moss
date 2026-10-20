import { defineConfig } from '@playwright/test';

// Screenshot comparisons against committed baselines (e2e/__screenshots__), mock backend, fixed fonts and time.
// Run with `npm run test:visual`; regenerate with `npm run test:visual:update` (see README.md).
export default defineConfig({
  testDir: './e2e',
  testMatch: 'visual.spec.ts',
  snapshotPathTemplate: '{testDir}/__screenshots__/{projectName}/{arg}{ext}',
  timeout: 60_000,
  retries: 0,
  workers: 1,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never', outputFolder: 'visual-report' }]] : 'list',
  outputDir: 'test-results/visual',
  expect: { toHaveScreenshot: { maxDiffPixelRatio: 0.002, threshold: 0.2, animations: 'disabled', caret: 'hide', scale: 'css' } },
  use: { baseURL: 'http://127.0.0.1:5199', locale: 'en-US', timezoneId: 'UTC', deviceScaleFactor: 1, reducedMotion: 'reduce', trace: 'off' },
  projects: [
    { name: 'desktop', use: { viewport: { width: 1280, height: 760 } } },
    { name: 'phone', use: { viewport: { width: 390, height: 780 } } },
  ],
  webServer: { command: 'npm run dev:mock -- --port 5199 --strictPort --host 127.0.0.1', url: 'http://127.0.0.1:5199', reuseExistingServer: !process.env.CI, timeout: 120_000 },
});
