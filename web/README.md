# Moss web

Svelte 5 (runes) + TypeScript + Vite static SPA. No UI/router/chart/icon libraries; hash routing.

## Scripts
- `npm run dev` – dev server, proxies `/api` and `/healthz` to `http://127.0.0.1:8420` (override with `MOSS_API=...`)
- `npm run dev:mock` – same, with the in-memory mock backend (`VITE_MOCK=1`)
- `npm run build` – static output in `dist/`
- `npm run check` / `npm run lint` – svelte-check (+ tsc); `npm test` – vitest (ansi, format, route, pulse)

- `npm run test:ui` – Playwright + axe against the mock backend (every page renders cleanly and passes accessibility checks)
- `npm run test:visual` – screenshot comparison of key pages (overview, server tabs, settings appearance, setup, command palette, a dialog) in light, dark and contrast themes at desktop (900x560) and phone (360x640) sizes, against `e2e/__screenshots__`
- `npm run test:visual:update` – regenerate those baselines after an intended visual change; review the changed PNGs in the diff before committing

## Visual tests
They run on the mock backend with time, random numbers, timers and animations pinned, and with a small bundled font (`e2e/fonts`, Noto Sans, SIL OFL) instead of the system fonts, so results do not depend on the machine. Baselines should be generated on Linux with the same Playwright Chromium CI uses (`npx playwright install chromium`). When CI fails, download the `visual-diffs` artifact: it holds the expected, actual and diff images and an HTML report. Keep the baselines small (about 5 MB in total): add a shot only for a page that is visually distinct.

## Structure
- `src/lib/api.ts` typed client (GET de-dup + TTL cache, SSE/WS tickets); `mock.ts` dev-only backend, tree-shaken from prod
- `src/lib/stores.svelte.ts` rune stores (prefs, toasts, confirm, servers via SSE, operations); `auth.svelte.ts` Supabase GoTrue over fetch
- `src/components/` shared UI; `src/routes/` pages (heavy ones lazy-loaded), `routes/server/` server tabs
- `src/app.css` design tokens (deep forest; dark/light/contrast, accents, density)
