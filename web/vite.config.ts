import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

const target = (globalThis as { process?: { env: Record<string, string> } }).process?.env.MOSS_API ?? 'http://127.0.0.1:8420';

export default defineConfig({
  base: '/',
  plugins: [svelte()],
  server: {
    proxy: {
      '/api': { target, ws: true, changeOrigin: true },
      '/healthz': target,
    },
  },
  build: { target: 'es2022', outDir: 'dist', sourcemap: false, cssCodeSplit: true },
  test: { include: ['src/**/*.test.ts'], environment: 'node' },
});
