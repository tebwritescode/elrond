# HTTP API

Base path `/api/v1`. Every response is JSON. Authentication is by session cookie.

## Error shape

Every failure, from any endpoint, returns:

```json
{
  "code": "field_too_short",
  "message": "password must be at least 12 characters",
  "field": "password"
}
```

- `code` — stable machine-readable identifier. Branch on this, not on `message`.
- `message` — safe to display to a user.
- `field` — present only when the failure concerns one input.

Storage and hashing failures return a generic message; the detail is logged
server-side and never sent.

### Codes

| Code | Status | Meaning |
| --- | --- | --- |
| `field_required` | 422 | A required value was absent or blank. |
| `field_too_short` | 422 | Below the minimum length. |
| `field_too_long` | 422 | Above the maximum length. |
| `field_invalid` | 422 | Structurally invalid. |
| `invalid_credentials` | 401 | Wrong username or password. Indistinguishable from an unknown account. |
| `not_authenticated` | 401 | No session, or it has expired or been revoked. |
| `account_disabled` | 403 | Credentials were correct but the account is deactivated. |
| `forbidden` | 403 | Authenticated without sufficient authority. |
| `not_found` | 404 | No such resource or endpoint. |
| `conflict` | 409 | Conflicts with current state. |
| `already_exists` | 409 | Uniqueness violation. |
| `setup_already_completed` | 409 | First-run setup attempted on an initialized instance. |
| `csrf_check_failed` | 403 | Failed the origin check or the double-submit token check. |
| `rate_limited` | 429 | Too many attempts. Carries `Retry-After`. |
| `request_body_malformed_json` | 400 | Body was not valid JSON. |
| `request_body_invalid` | 400 | Valid JSON of the wrong shape. |
| `request_content_type_invalid` | 400 | Missing or wrong `Content-Type`. |
| `storage_failure` | 500 | A backend failure. Detail is logged, not returned. |

## CSRF

State-changing requests (`POST`, `PUT`, `PATCH`, `DELETE`) must carry the CSRF
token in the `X-Elrond-CSRF` header, matching the `elrond_alt_csrf` cookie.

Obtain one from `GET /api/v1/bootstrap`, which always issues a token. Safe methods
need none, which is what lets a client fetch its first token before signing in.

If an `Origin` header is present it must match `ELROND_PUBLIC_URL` or an entry in
`ELROND_ALLOWED_ORIGINS`.

The token is rotated on sign-in and on setup, so any pre-authentication value
becomes invalid once the privilege level changes.

## Endpoints

### `GET /health`

Unauthenticated. Used by container orchestration.

```json
{ "status": "ok", "version": "0.1.0" }
```

### `GET /bootstrap`

Everything the client needs on first load, in one request. Always sets the CSRF
cookie, reusing an existing token so a second tab does not invalidate the first
tab's in-flight forms.

```json
{
  "requires_setup": false,
  "user": {
    "id": "019fb07b-5f27-75a1-b4f0-c8ba56882c36",
    "username": "records.admin",
    "role": "admin",
    "is_active": true,
    "created_at": "2026-07-30T00:45:03.143Z"
  },
  "csrf_token": "…",
  "version": "0.1.0"
}
```

`user` is `null` when not signed in.

### `POST /setup`

Creates the first administrator and signs in as it. Returns `201`.

Available only while no account exists; afterwards it returns `409`
`setup_already_completed`, permanently. Rate limited, because it is reachable
before any credential exists.

```json
{ "username": "records.admin", "password": "a sufficiently long passphrase" }
```

Response:

```json
{
  "user": { "id": "…", "username": "records.admin", "role": "admin", "is_active": true, "created_at": "…" },
  "csrf_token": "…",
  "expires_at": "2026-08-29T00:45:03.154Z"
}
```

### `POST /session`

Signs in. Returns `201` and the same body as `/setup`.

```json
{ "username": "records.admin", "password": "a sufficiently long passphrase" }
```

A wrong password and an unknown username both return `401 invalid_credentials`,
and the server hashes the supplied password either way so response latency cannot
be used to enumerate accounts.

Rate limited per client address. A successful sign-in clears the counter, so
someone who mistyped their password a few times is not left throttled.

### `DELETE /session`

Signs out. Returns `204` and clears both cookies. Idempotent: signing out without
a session is a success, so a stale tab does not show an error the user can do
nothing about.

### `GET /me`

The signed-in account. `401` when there is no valid session.

### `GET /users`

Every account, oldest first. Administrators only; `403 forbidden` otherwise.

## Roles

A ladder — each role includes everything below it.

| Role | Adds |
| --- | --- |
| `viewer` | Read published material. |
| `reviewer` | Approve or reject documents in review. |
| `editor` | Ingest, edit, and author binders. |
| `admin` | Administer accounts and system settings. |

## Cookies

| Name | Flags | Purpose |
| --- | --- | --- |
| `elrond_alt_session` | `HttpOnly`, `SameSite=Lax`, `Secure` when configured | The session bearer token. |
| `elrond_alt_csrf` | `SameSite=Lax`, `Secure` when configured, readable by scripts | Echoed back in `X-Elrond-CSRF`. |

Names are namespaced because browsers scope cookies by host and ignore the port,
so an unprefixed name would collide with another Elrond build on the same
hostname.
