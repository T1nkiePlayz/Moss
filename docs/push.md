# Web Push and installing Moss

The web UI is an installable app (manifest, icons) and can send Web Push notifications to phones and desktops, even when no Moss tab is open.

## Install

Open Moss over HTTPS (or `localhost`) and use the browser's "Install" / "Add to Home Screen" action. Web Push needs a secure context; plain `http://` on a LAN address will not offer it.

## Turn on notifications

Settings, Notifications and audit, "Push notifications on this device". The browser asks for permission, registers `/sw.js` and sends its subscription to `POST /push/subscriptions`. Each browser subscribes separately; turning it off removes the subscription.

A user receives a push when all of these hold:

1. the browser is subscribed;
2. the event kind is selected in Settings (`notify_events`, shared with the webhook);
3. the user can see the server (admins see everything including host-level kinds such as `disk_low`; others only servers they were granted, never admin-only kinds).

Default kinds: `crash`, `crash_loop`, `backup_failed`, `disk_low`, `restart_failed`. `player_join` is opt-in (not in the defaults): one notification per player per server per 10 minutes. Tapping a notification opens `#/servers/<id>`.

## How it works

* VAPID (RFC 8292): an ES256 P-256 key is generated on first use and stored in `<data dir>/push-vapid.key` (mode 0600). The JWT `aud` is the push service origin and expires after 12 hours. The `sub` claim is the `push_subject` setting (default: the Moss repository URL); set it to a `mailto:` address if a push service asks for one.
* Encryption: RFC 8291, `aes128gcm`, one 4096-byte record. A unit test reproduces the RFC 8291 Appendix A vector byte for byte.
* Delivery runs in the background with a 10 s timeout, TTL 24 h, `Urgency: high` for crashes and critical events. A 404 or 410 from the push service deletes the subscription. Failures are logged at debug level only.
* Endpoints must be https and public: loopback, private, link-local and similar addresses are refused when subscribing and again when connecting (unless `MOSS_ALLOW_PRIVATE_WEBHOOKS=true`). Redirects are not followed.
* The service worker caches nothing and never stores API responses. The CSP allows `worker-src 'self'` and `manifest-src 'self'`.
* Payload (readable only by the browser): `{kind, severity, title, body, server_id}`.
