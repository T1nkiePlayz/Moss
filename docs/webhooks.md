# Event webhooks

Event webhooks let another system react to what happens on your Moss instance: post to a chat bot when a server crashes, trigger a script when a backup finishes, feed a dashboard. Moss sends an HTTPS `POST` with a signed JSON body to each address you register. (The notification webhook in Settings, Notifications is separate: it sends one human-readable alert to a chat channel. Event webhooks are for programs.)

Set them up in **Settings, Webhooks** (administrators only). Each webhook has a name, an HTTPS address, the events it wants, and optionally the servers it is limited to. When you add one, Moss shows its **signing secret once**. Copy it into your receiver straight away; if you lose it, use *New secret* and update the receiver.

## Events

| Event | When |
| --- | --- |
| `server.started` | A server finished starting and is ready |
| `server.stopped` | A server stopped |
| `server.crashed` | A server's process died unexpectedly (`data.detail` has the reason) |
| `server.restarted` | A server became ready again within two minutes of going down, whether you restarted it or Moss did after a crash |
| `backup.completed` | A backup finished (`data.backup` has id, size, checksum) |
| `backup.failed` | A backup, or its copy to a remote destination, failed |
| `operation.completed` / `operation.failed` | Any long-running job (installs, updates, restores, ...) ended; `data.operation` has its id, kind, title and error |
| `player.joined` / `player.left` | A player appeared in or vanished from the server's player list |
| `content.installed` | A mod, plugin or modpack was installed |
| `update.available` | A content update check found newer versions (`data.updates`), or Moss raised an "update available" notification |
| `health.critical` | A new critical finding appeared on the Health page (checked once a minute) |

Limits worth knowing:

- **Player events** are derived from the server's status response, which Moss polls every 5 seconds. A player who joins and leaves between two polls is not reported. Minecraft only lists up to 12 names, so on a busy server some joins and leaves are not reported, and servers that hide their player list send no player events. Nobody is reported as joining just because the server came back up.
- A server that was already running when Moss itself started does not produce `server.started`.
- `server.restarted` replaces `server.started` when the start follows a stop or crash closely; a plain stop before a restart still sends `server.stopped`.
- Delivery is **at least once**. Retries and a Moss restart can deliver an event twice: de-duplicate on the event `id`. Events for different endpoints, and different events, are not guaranteed to arrive in order.
- Events that are not tied to a server (`update.available` from a notification, host-wide `health.critical`) are only sent to webhooks that are not limited to specific servers.

## What Moss sends

```
POST /your/path HTTP/1.1
Content-Type: application/json
User-Agent: Moss/0.2.0
X-Moss-Event: server.crashed
X-Moss-Delivery: 0194f3a2-...        (same value on every retry of this delivery)
X-Moss-Timestamp: 1767225600         (Unix seconds, set on every attempt)
X-Moss-Signature: sha256=3b1f...     (see below)

{"id":"0194f3a2-...","type":"server.crashed","time":"2026-01-01T00:00:00.000Z",
 "server":{"id":"...","name":"Survival"},"data":{"detail":"exit code 1"}}
```

`server` is `null` for events that do not belong to one server. `data` is capped so a body never exceeds 64 KiB. Your receiver should answer with any `2xx` status within 10 seconds; the answer body is not needed (Moss keeps the first 2 KB for the delivery log).

## Verifying the signature

The signature is `sha256=` followed by the hex HMAC-SHA256 of the string `<timestamp>.<raw body>`, keyed with your webhook's secret. Compute it over the **raw bytes** you received (not a re-serialised JSON object), compare in constant time, and reject requests whose timestamp is more than a few minutes old to stop replays.

Node.js (Express-style, using the raw body):

```js
import crypto from 'node:crypto';

export function verify(secret, headers, rawBody, toleranceSeconds = 300) {
  const ts = headers['x-moss-timestamp'];
  const given = headers['x-moss-signature'] ?? '';
  if (Math.abs(Date.now() / 1000 - Number(ts)) > toleranceSeconds) return false;
  const expected = 'sha256=' + crypto.createHmac('sha256', secret).update(`${ts}.${rawBody}`).digest('hex');
  const a = Buffer.from(expected), b = Buffer.from(given);
  return a.length === b.length && crypto.timingSafeEqual(a, b);
}
// app.post('/hook', express.raw({ type: '*/*' }), (req, res) =>
//   verify(SECRET, req.headers, req.body.toString('utf8')) ? res.sendStatus(204) : res.sendStatus(401));
```

Python:

```python
import hashlib, hmac, time

def verify(secret: str, headers: dict, raw_body: bytes, tolerance: int = 300) -> bool:
    ts = headers.get("X-Moss-Timestamp", "")
    given = headers.get("X-Moss-Signature", "")
    if not ts.isdigit() or abs(time.time() - int(ts)) > tolerance:
        return False
    expected = "sha256=" + hmac.new(secret.encode(), ts.encode() + b"." + raw_body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, given)
```

## Delivery, retries and the log

- Moss tries again after 5 s, 30 s, 2 min, 10 min, 30 min, 1 h and 2 h (8 attempts in total). Any answer other than `2xx` counts as a failure, redirects are **not** followed, and an attempt that takes longer than 10 seconds is abandoned.
- The queue is stored in Moss's database, so pending deliveries survive a restart. A delivery that used all its attempts is marked **gave up** (dead letter) and stays in the log; use *Send again* to queue it once more.
- After **15 failed attempts in a row** (counted across events; any success resets the count) Moss switches the webhook off and raises an administrator notification. Fix the receiver, then turn it back on.
- Each webhook shows its last 100 deliveries: state, attempts, HTTP status, latency and the first 2 KB of the response. *Send test event* sends a `test` event once, immediately, and is never retried or counted towards switching the webhook off.

## Security notes

- Addresses must be `https://`. Moss resolves the host once, refuses private, loopback, link-local and other non-public addresses, and connects to the address it checked. To deliver to a receiver on your own network, the administrator must start Moss with `MOSS_ALLOW_PRIVATE_WEBHOOKS=true` (this also applies to the notification webhook).
- Secrets are stored in `webhook-secrets.json` in the data directory (readable by the Moss user only) and are never returned by the API after creation, written to logs, or included in support bundles. The address is treated as sensitive too: it is not logged.
- Anyone who has the secret can forge events, so give each receiver its own webhook.
