// Moss service worker: Web Push only. It caches nothing (no offline mode, API responses are never stored).
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

self.addEventListener('push', (e) => {
  let d = {};
  try { d = e.data ? e.data.json() : {}; } catch (_) { d = {}; }
  e.waitUntil(self.registration.showNotification(d.title || 'Moss', {
    body: d.body || '',
    icon: '/icon-192.png',
    badge: '/icon-192.png',
    tag: (d.kind || 'moss') + ':' + (d.server_id || ''),
    data: { server_id: d.server_id || null },
    requireInteraction: d.severity === 'critical',
  }));
});

self.addEventListener('notificationclick', (e) => {
  e.notification.close();
  const id = e.notification.data && e.notification.data.server_id;
  const url = '/' + (id ? '#/servers/' + encodeURIComponent(id) : '');
  e.waitUntil((async () => {
    const all = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    const mine = all.find((c) => new URL(c.url).origin === self.location.origin);
    if (mine) {
      await mine.focus();
      try { await mine.navigate(url); } catch (_) { /* focus only */ }
      return;
    }
    await self.clients.openWindow(url);
  })());
});
