import { parseHash, type Route } from './route';

export const router = $state<{ route: Route }>({ route: parseHash(location.hash) });

if (typeof window !== 'undefined') {
  window.addEventListener('hashchange', () => {
    router.route = parseHash(location.hash);
    window.scrollTo({ top: 0 });
  });
}

export function navigate(path: string) {
  location.hash = path.startsWith('#') ? path : '#' + path;
}
