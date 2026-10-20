# 0004. Supabase is used for authentication only

Status: Accepted

## Context

Moss needs real sign-in (email, OAuth, MFA, password reset) without implementing a credential store. It must also keep working, and keep all server data local, without a hosted backend.

## Decision

Moss verifies Supabase-issued JWTs locally (`src/auth/jwt.rs`): signing keys come from the project's JWKS endpoint and are cached, audience and issuer are pinned, and `exp`, `sub`, `aud` and `iss` are required. Authorisation is entirely Moss's own: users, roles (owner, admin, user) and per-server grants live in SQLite (`src/auth/mod.rs`, `src/auth/provision.rs`). A valid token is not enough; the account must be the owner or invited and have a verified email. Ownership of a fresh instance is claimed with a setup token. MFA is enforced from the `aal` claim (`src/auth/mfa.rs`). Personal API tokens (`moss_pat_...`) are Moss's own and do not involve Supabase. A dev mode with no login exists but is limited to loopback binds. Supabase is also used as the identity check for the CurseForge edge function.

## Consequences

- No passwords in Moss; account recovery and MFA enrolment are Supabase's.
- Verification needs no per-request call to Supabase; it needs JWKS reachable at least periodically (stale keys are used for up to 24 hours).
- Revoking a user's Supabase session does not revoke an already-issued JWT before `exp`; revoking access in Moss takes effect immediately.
- The Supabase project is the root of identity trust; its compromise is outside what Moss can defend.
- Everyone must have or create a Supabase project; offline use needs dev mode or the legacy HS256 secret.
