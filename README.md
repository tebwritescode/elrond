# Elrond

Self-hosted document library for preserving source files, managing controlled PDF
versions, and publishing reproducible professional binders.

> **This is a parallel implementation.** Elrond is being built twice, independently,
> so the two results can be compared. This repository shares no code, no database,
> and no git history with the other build, and it deliberately occupies different
> ports (API `3100`, dev server `5273`) and cookie names (`elrond_alt_*`) so both
> can run on one host at the same time.

## Status

`v0.1.0` — architecture, design system, authentication, and deployment.

| Milestone | Scope | State |
| --- | --- | --- |
| `v0.1.0` | Architecture, design system, authentication, deployment | **current** |
| `v0.2.0` | Ingestion, categories, tags, search, viewing | planned |
| `v0.3.0` | Versions, review workflow, annotations, dashboard | planned |
| `v0.4.0` | Binder designer and reproducible PDF generation | planned |
| `v0.5.0` | Audit, import/export, backup, restore | planned |
| `v0.9.0` | Security, accessibility, migration, performance hardening | planned |
| `v1.0.0` | Complete supported release | planned |

## Architecture

A modular monorepo producing one application image. Domain code depends on no
framework, no database, and no HTTP library, which is what keeps a later move to
PostgreSQL, object storage, or a different PDF processor from touching business
rules.

```text
crates/
  domain/          Entities and workflow invariants. No I/O.
  application/     Use cases plus the ports their adapters must satisfy.
  infrastructure/  SQLite, Argon2id, and CSPRNG adapters.
  api/             Axum routing, cookies, CSRF, error contract.
  server/          Composition root and the `elrond` binary.
web/
  src/app/         Routing and the application shell.
  src/components/  The Elrond design system.
  src/features/    Documents, categories, binders, admin.
  src/lib/         API client and shared utilities.
migrations/        Schema, compiled into the binary.
```

Dependencies flow inward only: `server` → `api` → `application` → `domain`, with
`infrastructure` implementing `application`'s ports.

## Running it

### Development

Two processes: the Rust API with `cargo-watch`, and Vite with hot module
replacement. The Vite dev server proxies `/api` to the API, so the browser stays
on a single origin and cookies behave exactly as they do in production.

```bash
cp .env.example .env

# Terminal 1 — API on http://localhost:3100, rebuilt on change
cargo watch -w crates -w migrations -x 'run -p elrond-server'

# Terminal 2 — client on http://localhost:5273, hot reloaded
cd web && npm install && npm run dev
```

Open <http://localhost:5273>. On a fresh database you land on first-run setup;
choose a username and password for the administrator. That endpoint closes
permanently once an account exists.

The client polls while the API is unreachable, so a `cargo-watch` rebuild shows a
brief "waiting for the server" banner and then reconnects on its own.

Development data lives in `dev-data/` and is excluded from git. Delete that
directory to start over from first-run setup.

### Single-process deployment

```bash
docker compose up --build
```

Elrond serves the API, the built client, and background work from one process on
port `3100`, with all persistent state under the `/data` volume. Stirling-PDF is
an independently deployed service reached over HTTP; Elrond treats its responses
as untrusted and validates them, because a failed downstream pipeline can return
an empty but successful HTTP response.

## Configuration

Every setting is read once at startup and validated before the port is bound, so
a misconfiguration fails immediately with a clear message. See
[`.env.example`](.env.example) for the full list.

The ones worth understanding:

| Variable | Default | Notes |
| --- | --- | --- |
| `ELROND_BIND_ADDRESS` | `127.0.0.1:3100` | Loopback by default, so Elrond does not become reachable on every interface by accident. Containers set `0.0.0.0:3100`. |
| `ELROND_PUBLIC_URL` | derived | The origin the CSRF check accepts. Set it to the URL users actually reach. |
| `ELROND_SECURE_COOKIES` | `false` | Turn on behind TLS. A `Secure` cookie is silently dropped over plain HTTP, which presents as a sign-in that does nothing. |
| `ELROND_TRUST_FORWARDED_FOR` | `false` | Only enable behind a proxy that always overwrites the header; otherwise it is attacker-controlled and defeats rate limiting. |
| `ELROND_ALLOWED_ORIGINS` | empty | Extra CSRF-allowed origins. Needed in development for the Vite dev server. |
| `STIRLING_URL` | unset | Base URL of the external Stirling-PDF instance. |
| `STIRLING_API_KEY` | unset | Never logged; only its presence is recorded. |

## Security

- Passwords are hashed with Argon2id at the OWASP minimum (19 MiB, 2 passes),
  run on the blocking pool so the async runtime is not stalled.
- Sessions are 256-bit opaque tokens in `HttpOnly` cookies. Only a SHA-256
  fingerprint is stored, so a database dump yields no usable session.
- Both an idle timeout and an absolute lifetime are enforced; activity cannot
  extend the hard expiry.
- CSRF uses a double-submit token plus an `Origin` allowlist, on unsafe methods
  only. The token is rotated when the privilege level changes.
- Credential endpoints are rate limited per client address, cleared on a
  successful sign-in, and answer with `Retry-After`.
- Storage and hashing failures never describe themselves to a client; the detail
  goes to the log.
- Authentication is a username and a password. Elrond stores no email address or
  other contact detail, so there is nothing to verify and nothing to leak.
- Audit records are append-only in the schema itself, enforced by triggers, so no
  code path can rewrite history.

## Quality gates

```bash
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Web
cd web
npm run typecheck
npm run lint
npm run format:check
npm run test
```

CI runs all of the above plus a dependency audit, a container build, and a
health check against the built image.

## Licence

MIT. See [LICENSE](LICENSE).
