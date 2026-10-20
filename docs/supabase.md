# Supabase sign-in

Moss uses [Supabase Auth](https://supabase.com/docs/guides/auth) for sign-in. Supabase only proves **who** a user is; **what** they may do (owner/admin/user and per-server grants) is stored and enforced by Moss itself, on every request. Server data, files and backups never leave your machine.

## 1. Create a project

1. Create a free project at <https://supabase.com>.
2. **Authentication → Sign In / Providers**: keep *Email* enabled. Optionally enable GitHub and/or Google (the login screen shows both buttons; they only work once you enable the provider).
3. **Authentication → URL Configuration**: set *Site URL* to the address you open Moss at (for example `http://127.0.0.1:8420`, or your public `https://moss.example.com`) and add it to *Redirect URLs*. OAuth sign-in returns there.
4. Recommended: keep **Confirm email** turned on. Moss refuses to match an invite or owner email to an account whose email is not verified (`MOSS_REQUIRE_VERIFIED_EMAIL=true`, the default).
5. Optional: **disable "Allow new users to sign up"** once your own account exists. Invited people can still be created from the Supabase dashboard; Moss additionally requires an invite (below).

## 2. Configure Moss

From **Project Settings → API** copy the project URL and the *publishable/anon* key (public by design):

```bash
export SUPABASE_URL=https://YOUR-PROJECT.supabase.co
export SUPABASE_ANON_KEY=YOUR-PUBLISHABLE-KEY
export MOSS_OWNER_EMAILS=you@example.com   # pin ownership
moss serve --data-dir /var/lib/moss
```

Tokens are verified locally. Projects using the new **asymmetric JWT signing keys** (ES256/RS256/EdDSA) need nothing else: Moss reads your project's JWKS (`/auth/v1/.well-known/jwks.json`) and caches it. Projects still on the legacy HS256 secret also set `SUPABASE_JWT_SECRET`. The service-role key is never needed and must never be given to Moss.

## 3. Who gets in

- Emails in `MOSS_OWNER_EMAILS` become **owner** on first verified sign-in.
- If that variable is empty and no owner exists yet, Moss creates a **setup token**: 32 random characters written to `<data-dir>/setup-token` (mode 0600) and announced in the startup log. Nobody becomes owner automatically. Sign in, and while setup is pending the panel asks for the token (`POST /api/v1/setup/claim`). Read it on the host (`cat <data-dir>/setup-token`). Wrong guesses are limited to 5 a minute per account; on success the owner is created atomically (only one claimant can win) and the file is deleted. The token is kept across restarts until it is used; delete the file and restart to rotate it. `GET /auth/config` reports `setup_required`.
- Everyone else must be **invited** from *Users* in the panel (admin or owner) before signing in; their grants are linked to the account on first verified sign-in. Without an invite a signed-in user sees "not invited" and has no access. Set the instance setting `access_message` (for example "Ask Sam in #minecraft for an invite") and the login screen shows it (`access_message` in `GET /auth/config`).
- Roles: `owner` (everything, can create admins), `admin` (users, servers, settings), `user` (only servers they were granted: manager, operator or viewer).

## 4. Multi-factor authentication

`MOSS_REQUIRE_MFA=off|admins|all` (default `off`). Users enrol a factor in Supabase (**Authentication → Multi-Factor** must be enabled; TOTP works out of the box) and sign in at assurance level `aal2`. Moss reads the `aal` claim of the already verified JWT: when the policy applies to the account (`admins` = owner and admin roles) and the session is not `aal2`, every endpoint answers 403 `auth.mfa_required` (body fields `aal` and `required`), except `GET /auth/config` and `GET /me`; `/me` reports `mfa: {required, satisfied}` so the UI can prompt. Claiming ownership also needs `aal2` when the policy would apply to an owner. API tokens are non-interactive and not subject to the gate (creating one needs an interactive session that passed it). Dev mode ignores the policy.

## 5. API tokens

Signed-in users create personal tokens (`POST /api/v1/tokens {name, scope: "read"|"full", expires_in_days?}`), for scripts and CI. A token looks like `moss_pat_` plus 40 random characters and is shown once; Moss stores only its SHA-256 and the first 8 characters. Send it as `Authorization: Bearer moss_pat_...`.

- It acts as its owner with their **current** role and grants: demoting or removing the user takes effect immediately.
- `read` tokens may only use `GET`/`HEAD` (403 `auth.token_scope`); `full` tokens can do whatever the owner can, except that no token can create or revoke tokens or manage users and server access (403 `auth.token_forbidden`).
- At most 25 active tokens per user; expired tokens stop working at once; `DELETE /api/v1/tokens/{id}` revokes (admins can list all with `?all=1`).
- The activity log records `you@example.com (token <name>)`; `last_used` is updated at most once a minute.

## 6. Throttling and reverse proxies

More than 30 failed credential checks a minute (invalid JWT, unknown or expired API token) from one client address get 429 `auth.throttled` with `Retry-After`. The address is the socket peer. Behind a reverse proxy every request would look like the proxy, so set `MOSS_TRUST_PROXY=true` **only** when a proxy you control sets `X-Forwarded-For` (Moss uses the right-most entry, the one your proxy appended); otherwise clients could spoof their address. Limits are kept in memory and reset on restart.

## Local development without Supabase

`MOSS_AUTH=dev moss serve` disables login entirely. It refuses to bind anything except loopback.
