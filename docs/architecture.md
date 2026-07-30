# Architecture

Elrond is a layered monorepo. This document records the decisions that are load
bearing, and the reasoning behind them, so a later change can tell whether it is
breaking something on purpose.

## Layering

```text
server  ──▶  api  ──▶  application  ──▶  domain
                            ▲
                            │ implements ports
                     infrastructure
```

Dependencies point inward only. `domain` has no dependency on Axum, SQLite, the
filesystem, or Stirling-PDF; `application` depends only on `domain` and on traits
it declares itself.

**Why.** The product's hardest requirement is that a published document version
never changes, because binder releases pin version identifiers and must rebuild
byte-identically. That invariant has to live somewhere that cannot be bypassed by
a new HTTP handler or a hand-written query. Putting it in a crate with no I/O
means the only way to reach it is through the use cases that enforce it.

The secondary benefit is substitutability: PostgreSQL, object storage, or a
different PDF processor are adapter changes, not rewrites.

### What belongs where

| Layer | Contains | Never contains |
| --- | --- | --- |
| `domain` | Entities, value objects, state machines, invariants | I/O, async, framework types |
| `application` | Use cases, port traits, orchestration | SQL, HTTP status codes, cookies |
| `infrastructure` | SQLite, Argon2id, CSPRNG, clock | Business rules |
| `api` | Routing, extraction, cookies, CSRF, error mapping | Business rules |
| `server` | Configuration, wiring, background tasks | Anything reusable |

The composition root in `crates/server/src/main.rs` is the only place that knows
which adapter satisfies which port.

## Ports are defined by need, not by convenience

Port traits are written to express what a use case requires, not to mirror what
SQLite makes easy. Two consequences worth noting:

- `NewUser` carries its own id and `created_at`. The repository is a pure sink,
  and time enters the system through exactly one injected clock, which is what
  makes session-expiry rules testable without waiting.
- Session expiry is evaluated in the use case against that clock, not in SQL. The
  repository's `find_by_fingerprint` deliberately ignores expiry.

## Credentials are kept out of the entity that gets serialized

`User` has no password field. The hash travels in a separate `Credentialed`
wrapper, and `PasswordHash` and `SessionToken` redact themselves in both `Debug`
and `Display`.

**Why.** "Do not serialize the password hash" is a rule someone eventually
forgets. A `User` with no such field cannot leak one, and a redacting `Debug`
means a stray `tracing` call cannot either. An integration test asserts that no
setup response body contains credential material.

## Sessions

Opaque 256-bit random tokens, stored only as a SHA-256 fingerprint.

A plain SHA-256 is correct here even though passwords need Argon2id: the token is
already 256 bits of uniform randomness, so there is nothing to brute-force and a
slow KDF would only add latency to every authenticated request.

Two independent limits apply. The idle timeout slides on activity; the absolute
lifetime does not. Failing either check revokes the session immediately rather
than waiting for the hourly sweeper, so a stale cookie cannot be retried.

## Error contract

`DomainError` and `ApplicationError` enumerate their variants, and `api`'s
mapping to status codes is an exhaustive `match`. Adding a failure mode therefore
fails to compile until someone decides what clients should see, rather than
silently inheriting an unrelated status code.

Every failure leaves the API in one JSON shape:

```json
{ "code": "field_too_short", "message": "password must be at least 12 characters", "field": "password" }
```

`code` is the stable identifier clients branch on. Storage and hashing failures
are replaced with a generic sentence before they reach a client; the detail is
logged.

Unmatched paths under `/api` return a JSON 404 rather than falling through to the
SPA fallback. An integration test covers this, because the first implementation
did fall through and answered JSON clients with an HTML page.

## Storage

SQLite in WAL mode with foreign keys enforced per connection and both settings
verified at startup, because a pragma that silently fails to apply would stay
invisible until the first corruption or constraint bug.

Conventions:

- Identifiers are UUIDv7 stored as 16-byte blobs. Time-ordered keys keep inserts
  at the right-hand edge of the index, and `ORDER BY id` gives oldest-first
  without a second index.
- Timestamps are RFC 3339 text in UTC, so lexical order matches chronological
  order and a raw dump stays legible.
- Tables are `STRICT`, and enumerations carry `CHECK` constraints mirroring the
  Rust enums, so a mapping bug surfaces as a constraint violation rather than as
  unreadable data.
- Rows are re-validated on the way out. A hand-edited database fails loudly
  instead of producing entities that break domain invariants.

Migrations are compiled into the binary, so a shipped image cannot drift from the
schema it was built against and no migration CLI is needed at runtime.

`audit_events` has no foreign key to `users`, and `BEFORE UPDATE`/`BEFORE DELETE`
triggers abort. An audit record must outlive the account it refers to, and any
`ON DELETE` action would have to mutate the table the triggers protect.

## HTTP

One process serves the API, the built client, and background work. There is no
separate web server to configure and no CORS to reason about in production.

- `index.html` is served `no-cache`; hashed assets under `/assets` are served
  `immutable` for a year. Caching the shell would leave clients loading a bundle
  that references assets the new deployment no longer has.
- Unmatched non-API paths return the shell, so deep links survive a refresh.
- CSRF combines an `Origin` allowlist with a double-submit token. Each covers a
  case the other misses: `Origin` is absent on some same-origin requests, and a
  cross-site attacker can cause the cookie to be sent but cannot read it to
  populate the header.
- Session cookies use `SameSite=Lax` rather than `Strict`. `Strict` would drop
  the cookie when someone follows a link to a document from email or chat, which
  for a document library is a routine way to arrive.
- Rate limiting is an in-process fixed-window map. Elrond is a single-process
  deployment, so this avoids a Redis dependency; entries are pruned
  opportunistically so the map cannot grow without bound.

## Frontend

React with strict TypeScript. `exactOptionalPropertyTypes` and
`noUncheckedIndexedAccess` are on, and lint runs type-aware rules, so an
unawaited promise in an event handler is an error rather than a silent failure.

Session state is a single `bootstrap` query that returns setup state, the current
account, and a CSRF token together. The shell picks between setup, sign-in, and
the workspace from that one response, with no render flash and no waterfall.

Authentication is a gate above the router rather than a set of route guards. A
deep link visited while signed out lands on that page once authentication
succeeds, with no redirect bookkeeping.

The bootstrap query polls while it is failing, which is what makes the client
reconnect on its own after the API restarts under `cargo-watch`.

## Accessibility

Targeting WCAG 2.2 AA. Decisions baked into the design system rather than left to
each screen:

- One focus treatment for the whole application, using `:focus-visible` and an
  outline with an offset so it survives forced-colors mode.
- Status is never conveyed by colour alone. Invalid fields double their border
  weight as well as changing colour, and every status pill renders a word.
- Error messages live in a live region that is present before the error is, since
  screen readers do not reliably announce content inserted at the same moment the
  region appears.
- Icons are `aria-hidden` because each sits beside a text label. Icon-only
  controls carry an accessible name instead.
- Loading buttons stay in the tree as disabled controls with `aria-busy` rather
  than being replaced, so focus is not lost mid-submission.
- Interactive targets have a 40px minimum, comfortably above the 24px WCAG 2.2
  floor.

## Deliberate omissions at `v0.1.0`

- No document storage yet. The `Sha256Checksum` value object and the lifecycle
  state machine exist and are tested, but nothing writes files.
- No Stirling-PDF client. Its URL is read and reported at startup only.
- No account management beyond listing. Creating a second account arrives with
  the administration work in `v0.5.0`.
- `sqlx` runs queries at runtime rather than through the compile-time macros, so
  no offline query cache or `sqlx-cli` is needed to build. Worth revisiting once
  the schema settles.
