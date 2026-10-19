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
- If that variable is empty and no owner exists yet, the **first verified account to sign in becomes owner** (a warning is logged at startup). Set the variable to avoid a race on a freshly exposed instance.
- Everyone else must be **invited** from *Users* in the panel (admin or owner) before signing in; their grants are linked to the account on first verified sign-in. Without an invite a signed-in user sees "not invited" and has no access.
- Roles: `owner` (everything, can create admins), `admin` (users, servers, settings), `user` (only servers they were granted: manager, operator or viewer).

## Local development without Supabase

`MOSS_AUTH=dev moss serve` disables login entirely. It refuses to bind anything except loopback.
