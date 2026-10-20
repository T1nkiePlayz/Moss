# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately. Do not open a public issue or pull request for them.

Use GitHub's private vulnerability reporting: open the **Security** tab of this repository and choose **Report a vulnerability**. Include the Moss version (shown in Settings and in `moss --version`), your operating system, how you run Moss (process or container mode, behind a reverse proxy or not), and steps to reproduce. A proof of concept helps; please do not include real credentials or other people's data.

## Supported versions

Moss is pre-1.0. Security fixes are made on the latest released version only; please reproduce on the latest release or on `main` before reporting.

## What to expect

- Acknowledgement within 5 working days.
- An initial assessment (accepted, needs more information, or not a vulnerability) within 14 days.
- Fixes for confirmed issues are developed privately, released with a changelog entry, and credited to you unless you prefer otherwise. We aim to publish a fix within 90 days of the report and will tell you if that is not realistic.
- This is a volunteer project. These are targets, not guarantees.

## Scope

In scope: the Moss controller and its embedded web UI, the HTTP/WebSocket API, authentication and authorisation, the file, import, backup and restore paths, webhook and storage connectors, the container backend, and the CurseForge edge function in this repository.

Out of scope:

- Code run by the Minecraft server, its plugins and mods, or the game itself. Without container isolation, a game server runs as the same user as Moss; see the [threat model](docs/threat-model.md) for what is and is not defended.
- Issues that need an administrator account to abuse something an administrator can already do (for example, running commands in the console).
- Denial of service by overwhelming the network or a reverse proxy; Moss relies on the proxy for request-rate limits.
- Findings that only apply when Moss is deliberately exposed without TLS or with `MOSS_AUTH=dev`.
- Vulnerabilities in third-party dependencies that do not affect Moss; report those upstream. Advisories we have accepted are recorded in [docs/security-review.md](docs/security-review.md).

Known limitations and the results of our internal review are in [docs/threat-model.md](docs/threat-model.md) and [docs/security-review.md](docs/security-review.md). The review is internal and not a substitute for an external audit.
