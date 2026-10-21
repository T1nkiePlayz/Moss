import { defineConfig } from '@playwright/test';

// README screenshots (docs/screenshots) from the mock backend. Regenerate with `npm run screenshots`.
export default defineConfig({
  testDir: './e2e',
  testMatch: 'readme-screenshots.spec.ts',
  timeout: 60_000,
  retries: 0,
  workers: 1,
  reporter: 'list',
  outputDir: 'test-results/readme',
  use: { baseURL: 'http://127.0.0.1:5198', locale: 'en-US', timezoneId: 'UTC', viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, reducedMotion: 'reduce', colorScheme: 'dark' },
  webServer: { command: 'VITE_MOCK=1 npx vite build --outDir dist-mock --emptyOutDir && npx vite preview --outDir dist-mock --port 5198 --strictPort --host 127.0.0.1', url: 'http://127.0.0.1:5198', reuseExistingServer: !process.env.CI, timeout: 120_000 },
});
