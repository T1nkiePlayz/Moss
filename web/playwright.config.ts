import { defineConfig } from '@playwright/test';

// UI checks run against the mock backend (no Rust needed): every page renders without console errors and passes axe.
export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? 'github' : 'list',
  use: { baseURL: 'http://127.0.0.1:5199', trace: 'retain-on-failure' },
  webServer: { command: 'npm run dev:mock -- --port 5199 --strictPort --host 127.0.0.1', url: 'http://127.0.0.1:5199', reuseExistingServer: !process.env.CI, timeout: 60_000 },
});
