# Modrinth API terms review

Checked on 2026-10-20. This is an engineering summary in our own words, not legal advice. Modrinth can change these pages at any time; re-check before a release that changes how Moss uses the API.

## Pages read

| URL | Result |
|---|---|
| https://docs.modrinth.com/api/ | Fetched. Source for User-Agent, rate limit, authentication, hashes |
| https://modrinth.com/legal/terms | Fetched. Terms of Use, including the API section |
| https://modrinth.com/legal/rules | Fetched. Content Rules (about uploaded projects, not API clients) |
| https://modrinth.com/legal/api | Not found (HTTP 404). No separate "API guidelines" page was located at that address |
| Web search for launcher guidance | Returned only the official docs and third-party mirrors; mirrors disagree on rate limits and were not relied on |

Fetching used a summarising tool, not a raw download, so details such as exact wording were not verified against the original text. Anything that matters legally should be read on the pages themselves.

## What the documents say

**Identification.** Every request must carry a uniquely identifying `User-Agent`. Generic library strings are likely to be blocked. The docs suggest including the project name and, optionally, a version and contact details so Modrinth can reach the author.

**Rate limit.** 300 requests per minute per IP address, the same with or without a token. Responses carry `X-Ratelimit-Limit`, `X-Ratelimit-Remaining` and `X-Ratelimit-Reset` (seconds until reset). Higher limits can be requested from support. A third-party mirror claimed a higher authenticated limit; the official page does not say so.

**Authentication.** Personal access tokens and OAuth2. Tokens are needed to create or change data and to read private data. Read-only public data (search, projects, versions, files) needs none.

**Caching.** The API docs say nothing about caching or attribution. The Terms of Use do not set an attribution requirement for API clients either.

**Hashes.** Version files are identified by SHA-1 or SHA-512, and the API can look files up by hash.

**Test environment.** `staging-api.modrinth.com` exists for testing before using production.

**Stable identifiers.** Store project and version ids, not slugs, because slugs and usernames can change.

**Deprecation.** Old API versions are eventually retired and return 410.

**License to use the API (Terms of Use).** The API section grants a limited, non-exclusive, non-sublicensable, revocable license to download, display and query user-generated content through your own app, conditional on following the Terms, the Community Standards and applicable law. Modrinth can revoke it or change the service without notice.

**Commercial use.** The Terms describe the service as for personal, non-commercial use, while the API section does not itself prohibit commercial clients. The text is ambiguous; anyone planning a paid or ad-supported product on top of Modrinth should ask Modrinth in writing.

**Automated access.** The Terms prohibit robots, spiders and other automatic means of accessing the service, and forbid overloading it. This clearly cannot apply to the documented public API (that would make the API unusable), but it means scraping the website, as opposed to using the API, is not allowed. Using content to train AI models is prohibited.

**Redistribution.** Content stays owned by its creators; uploaders license Modrinth to distribute it through the service. The Content Rules govern uploads and say nothing about third-party clients. Files are served from `cdn.modrinth.com`. The terms do not set a license for individual files; each project has its own license in its metadata.

**Trademarks and endorsement.** Do not use Modrinth's marks without permission and do not imply endorsement.

**Disclosure.** The Content Rules require software that sends data to a remote server the user did not choose to connect to in-game to say so. This is aimed at uploaded mods, not at Moss.

## How Moss complies

| Requirement | Moss behaviour | Code |
|---|---|---|
| Unique User-Agent | `moss/<version> (<contact>)`; the contact is the admin-set `modrinth_contact` setting, falling back to `https://github.com/moss-mc/moss` | `src/download.rs` (`client`), `src/serve.rs`, `src/api/settings.rs` |
| Public API only, no scraping | Uses `https://api.modrinth.com/v2` | `src/content/modrinth.rs` |
| Rate limit awareness | Responses are cached in memory for 90 s; an HTTP 429 is turned into a user-visible message; hash identification is batched (`POST /version_files`) | `src/content/modrinth.rs`, `src/state.rs` (`cached`), `src/content/identify.rs` |
| Hash verification of files | Downloads verify SHA-512 (or SHA-1 fallback) from the version record, and `.mrpack` files require a hash | `src/content/apply.rs`, `src/content/modpack/import.rs`, `src/download.rs` |
| Files from the official CDN | Download host allow-list contains `modrinth.com` (covers `cdn.modrinth.com`); modpack file hosts are an exact-match list including `cdn.modrinth.com`, GitHub and GitLab; hosts are re-checked on redirects | `src/download.rs` (`ALLOWED`, `PACK_HOSTS`) |
| No bulk mirroring | Moss does not store Modrinth catalogue data; it keeps only ids, title, version label, file name and hash for installed items | `src/content/identify.rs` |
| No endorsement implied | UI and docs name Modrinth as a source only | not audited in the UI |
| Respect creators' licenses | Moss installs files onto the user's own server on request; it does not republish them | n/a |
| Public images | CSP allows `https://cdn.modrinth.com` for images | `src/api/assets.rs` |

## Gaps and recommendations

1. **Contact is optional.** If `modrinth_contact` is empty the User-Agent carries only the project URL. That is identifying but gives Modrinth no way to reach the operator of a given install. Consider prompting for it in setup, or documenting it more prominently.
2. **No rate limiter for Modrinth.** Moss relies on a 90 s cache and reacts to 429 after the fact (unlike CurseForge, which has a shared limiter in `src/providers/curseforge.rs`). A large update-all or upgrade plan across many mods could exceed 300 requests per minute from one IP. Add a shared limiter and honour `X-Ratelimit-Remaining` and `X-Ratelimit-Reset`.
3. **Rate-limit headers are ignored.** The 429 message says "try again in a minute" instead of using the reset header.
4. **Shared-IP deployments.** The limit is per IP; several Moss instances behind one NAT share it. Document this.
5. **Cache duration is arbitrary.** The docs are silent on caching, so 90 s is a judgment call; keep it short and never persist catalogue data.
6. **Commercial use is unresolved.** If Moss is ever offered as a hosted or paid service, get written clarification from Modrinth first.
7. **Slugs versus ids.** Moss stores project and version ids (`content_items`), which matches the guidance; confirm no UI path stores slugs as keys.
8. **Staging.** Moss has no switch to use `staging-api.modrinth.com` for development; consider a config override.
9. **Unverified.** The Terms and Content Rules were read through a summarising fetch. The page at `/legal/api` returned 404, so there may be API-specific guidelines at another URL that we did not find. Ask Modrinth or search their GitHub if a formal review is needed.
10. **API deprecation.** Moss targets `/v2`; handle a future 410 with a clear message rather than a generic error.
