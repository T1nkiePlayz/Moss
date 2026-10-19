# Moss web

Svelte 5 (runes) + TypeScript + Vite static SPA. No UI/router/chart/icon libraries; hash routing.

## Scripts
- `npm run dev` – dev server, proxies `/api` and `/healthz` to `http://127.0.0.1:8420` (override with `MOSS_API=...`)
- `npm run dev:mock` – same, with the in-memory mock backend (`VITE_MOCK=1`)
- `npm run build` – static output in `dist/`
- `npm run check` / `npm run lint` – svelte-check (+ tsc); `npm test` – vitest (ansi, format, route, pulse)

## Structure
- `src/lib/api.ts` typed client (GET de-dup + TTL cache, SSE/WS tickets); `mock.ts` dev-only backend, tree-shaken from prod
- `src/lib/stores.svelte.ts` rune stores (prefs, toasts, confirm, servers via SSE, operations); `auth.svelte.ts` Supabase GoTrue over fetch
- `src/components/` shared UI; `src/routes/` pages (heavy ones lazy-loaded), `routes/server/` server tabs
- `src/app.css` design tokens (deep forest; dark/light/contrast, accents, density)
