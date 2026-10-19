# Moss: app description for OAuth consent screens

Copy these into the GitHub OAuth App form, the Google Auth Platform branding page, or any other provider. Replace the bracketed URLs with your own once Moss has a public home.

## Basics

| Field | Value |
|---|---|
| Application name | Moss |
| Tagline (up to 60 chars) | Game servers that keep growing, quietly. |
| Logo | `docs/brand/moss-logo-1024.png` (square, 1024 px); vector at `docs/brand/moss-logo.svg` |
| Homepage URL | `https://moss.ashtontink.com` |
| Privacy policy URL | `https://moss.ashtontink.com/privacy/` |
| Terms of service URL | `https://moss.ashtontink.com/terms/` |
| Developer / support contact | your email |

## Short description (about 140 characters)

Moss is a fast, self-hosted control panel for Minecraft Java servers. Sign in to manage your servers, consoles, backups and plugins.

## Long description

Moss is a self-hosted web panel for running Minecraft Java servers. It starts and stops servers, streams the live console, edits files and settings, installs plugins and mods from Modrinth, and takes verified backups on a schedule. Game servers keep running even if the panel restarts.

Moss uses sign-in only to identify you. It is invite-only: the instance owner decides who gets an account and what each person can do on each server. Signing in with GitHub or Google never gives Moss access to your repositories, mail, files or contacts.

## What Moss asks for

| Provider | Scopes | Why |
|---|---|---|
| Google | `openid`, `email`, `profile` | Identify the account and show your name and avatar. |
| GitHub | `read:user`, `user:email` | Read your public profile and verified email address to match you to an invitation. |
| Email and password | none | Handled by Supabase Auth. |

Moss requests no write scopes and no access to repositories, Drive, Gmail, calendars or contacts.

## Data handling (basis for a privacy policy)

- Moss stores your account ID, email address and role in the instance's own SQLite database. Server data stays on the host that runs Moss.
- Passwords and sign-in sessions are handled by Supabase Auth. Moss never sees your password and only verifies the signed access token.
- Moss keeps an audit log of actions (who started a server, edited a file, and so on) so the owner can review activity.
- No analytics, advertising or tracking. Account data is not sold or shared with third parties.
- Deleting a user in Moss removes their role and server access. Deleting the Supabase account removes the sign-in itself.

## Redirect and origin URLs

| Where | Value |
|---|---|
| Supabase callback (set as the provider's authorized redirect/callback URL) | `https://ddglnaswgvkqgkgrdspa.supabase.co/auth/v1/callback` |
| Authorized JavaScript origin (Google) | your Moss URL, e.g. `http://127.0.0.1:8420` locally or `https://panel.example.com` |
| Moss Site URL and Redirect URLs (in Supabase) | the same Moss URL, with `/**` on the redirect entry |

## Where to paste it

- **GitHub:** Settings → Developer settings → OAuth Apps → New. Fill in name, homepage and description. Set the callback URL to the Supabase callback above. Copy the Client ID and secret into Supabase → Authentication → Providers → GitHub.
- **Google:** Google Cloud Console → Google Auth Platform → Branding, then Clients → Create OAuth client (Web). Use the origin and callback above. Copy the Client ID and secret into Supabase → Authentication → Providers → Google.
