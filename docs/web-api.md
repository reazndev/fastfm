# Web API

The bundled web UI talks to the server through a JSON API under `/app/api`. It is internal to
the UI and may change in any release. Last.fm clients should use `/2.0/` instead; see
[api.md](api.md).

## Conventions

- Request and response bodies are JSON. Field names are camelCase.
- Errors are `{"error": {"code": "…", "message": "…"}}` with an HTTP status. `code` is stable and
  meant for the UI; `message` is for people.
- Login is a cookie named `lfm_session`: `HttpOnly`, `SameSite=Strict`, `Path=/`, valid for 30
  days, and `Secure` according to `LFM_SECURE_COOKIES`.
- Every `POST`, `PUT`, `PATCH` and `DELETE` must carry an `X-Requested-With` header with any
  value, and so must `GET /lastfm/auth/callback`. Without it the answer is 403
  `csrf_header_missing`. A cross-site form cannot set the header, and the API grants no CORS
  preflight, so another site cannot send it.
- Times are Unix seconds.

Access levels used below:

| Level | Who |
|---|---|
| Anyone | No login needed. |
| Viewer | Logged in, account not disabled. Pending accounts pass. |
| Member | Logged in and active. A pending account gets 403 `account_pending`. |
| Admin | An active admin. Others get 403 `admin_only`. |

An `account` object is `{id, username, role, status, lastfmUser, createdAt}`, where `role` is
`admin` or `user` and `status` is `pending`, `active` or `disabled`. A `key` object is
`{key, secret, label, scope, createdAt, lastUsed}`, where `scope` is `read` or `read_write`.

## Accounts

| Method and path | Access | Request | Response |
|---|---|---|---|
| `POST /signup` | Anyone | `{username, password}` | 201 `{account}` and the login cookie |
| `POST /login` | Anyone | `{username, password}` | 200 `{account}` and the login cookie |
| `POST /logout` | Anyone | none | 204, cookie cleared |
| `GET /me` | Viewer | none | `{account}` |

Usernames are 2 to 32 ASCII letters, digits, `_` or `-`, starting with a letter
(`invalid_username`). Passwords are 10 to 256 characters (`weak_password`). The first account
becomes an active admin; later ones follow the sign-up mode (`signup_closed` when closed,
`name_taken` when the name exists). A wrong name or password is 401 `invalid_credentials`. Failed
logins and all sign-ups are counted per client IP (30 per 15 minutes), and failed logins per
username (10 per 15 minutes); past that the answer is 429 `rate_limited`.

## Admin

| Method and path | Access | Request | Response |
|---|---|---|---|
| `GET /admin/accounts?status=` | Admin | optional `status`: `pending`, `active` or `disabled` | `{accounts: [account]}`, oldest first |
| `POST /admin/accounts/{id}/approve` | Admin | none | 204 |
| `POST /admin/accounts/{id}/reject` | Admin | none | 204; deletes the pending account |
| `POST /admin/accounts/{id}/disable` | Admin | none | 204; stops its sync, rejects its keys and sessions |
| `POST /admin/accounts/{id}/enable` | Admin | none | 204; resumes its sync |
| `PUT /admin/accounts/{id}/role` | Admin | `{role: "admin" \| "user"}` | 204 |
| `GET /admin/settings` | Admin | none | `{signupMode, lastfmKeyPool: {keys, canSign}}` |
| `PUT /admin/settings` | Admin | `{signupMode: "approval" \| "open" \| "closed"}` | the settings |

`lastfmKeyPool.keys` is how many keys `LASTFM_API_KEYS` holds; the keys themselves are never
shown. `canSign` is whether `LASTFM_API_SECRET` is set. The last active admin cannot be disabled
or demoted (409 `last_admin`). Unknown statuses, roles or modes are 400 `invalid_status`,
`invalid_role` or `invalid_signup_mode`.

## API keys

| Method and path | Access | Request | Response |
|---|---|---|---|
| `GET /keys` | Member | none | `{keys: [key]}`, oldest first, secrets included |
| `POST /keys` | Member | `{label, scope: "read" \| "read_write"}` | 201 `{key}` |
| `DELETE /keys/{key}` | Member | none | 204; also ends every session key issued to it |

Labels are 1 to 64 characters (`invalid_label`). Keys and secrets are 32 hex characters each.

## Last.fm link

| Method and path | Access | Request | Response |
|---|---|---|---|
| `GET /lastfm` | Member | none | link status |
| `PUT /lastfm` | Member | `{username}` | link status; starts the import in the background |
| `DELETE /lastfm` | Member | none | 204; stops sync and forgets the Last.fm session key. Mirrored data stays. |
| `PUT /lastfm/key` | Member | `{apiKey, secret}` | 204; the account's own Last.fm application |
| `DELETE /lastfm/key` | Member | none | 204; back to the instance key pool |
| `GET /lastfm/auth/start` | Member | none | `{url}`: the last.fm approval page to open |
| `GET /lastfm/auth/callback?token=` | Member, with `X-Requested-With` | none | link status |

The link status is:

```json
{
  "username": "syntiiix",
  "authorized": true,
  "ownKey": false,
  "import": { "status": "done", "imported": 160643, "estimatedTotal": 160643, "error": null },
  "sync": { "cursor": 1794518863, "scrobbles": 160643 },
  "writes": { "forwarding": true, "queued": 0, "failed": 0, "heldLocally": 0, "lastError": null }
}
```

- `authorized`: Last.fm web auth is done, so writes can be forwarded.
- `import`: the newest import job, or `null`. `status` is `queued`, `running`, `done` or
  `failed`.
- `sync.cursor`: everything before this time is mirrored.
- `writes.forwarding`: false means writes from apps are kept locally only. `heldLocally` counts
  those.

Notes:

- Changing the linked username drops the Last.fm session key, which belonged to the old user.
  A Last.fm user already linked by another account is refused with 409 `lastfm_user_taken`.
  A username that is empty or longer than 64 characters is 400 `invalid_username`.
- `PUT /lastfm/key` checks the key with one signed call to Last.fm before storing it, and drops
  the session key. Key and secret must be 1 to 64 letters or digits (`invalid_key`).
- Web auth: `start` returns `https://www.last.fm/api/auth/?api_key=…&cb=…`, where `cb` is
  `LFM_PUBLIC_URL` (or the request's scheme and `Host`) plus `/lastfm/callback`. Last.fm sends
  the browser to that UI route, and the UI calls `callback` with the token. The cookie is
  `SameSite=Strict`, so it is sent on the UI's own request, not on Last.fm's redirect. Without a
  shared secret for the account's key or the instance key, both answer 503
  `lastfm_not_configured`.
- Last.fm failures are 503 `lastfm_unavailable` when transient, else 502 `lastfm_error` with
  Last.fm's message.

## App approval

These back the approval page of the emulated Last.fm auth flow. `GET /api/auth` and
`GET /api/auth/` (outside `/app/api`) redirect with 303 to the UI route `/authorize` with the
same query.

| Method and path | Access | Request | Response |
|---|---|---|---|
| `GET /auth/request?token=` | Member | none | `{apiKey, label, owner, expiresAt, approved}` |
| `POST /auth/approve` | Member | `{token}` | 204 |

`label` and `owner` describe the API key that asked; both are `null` for a key registered from
the environment. An unknown token is 404 `token_not_found`, an expired one 410 `token_expired`,
and a token another account approved is 409 `token_taken`.

## Speed comparison

These back the `/demo` and `/demo/live` pages. They need `LASTFM_API_KEYS` on the server, else
503 `demo_unavailable`. Each account is rate limited per hour: 30 prepares, 120 runs, 5 import
starts and 1,000 Last.fm race pages.

| Method and path | Access | Request | Response |
|---|---|---|---|
| `POST /demo/prepare` | Member | `{username}` | `{username, state, playcount, maxScrobbles, import, scenarios}` |
| `POST /demo/import` | Member | `{username}` | import view (`{status, imported, estimatedTotal, error}`) |
| `GET /demo/import?username=` | Member | none | import view |
| `POST /demo/run` | Member | `{username, scenario}` | `{id, requests, lastfm: {ms, bytes, ok}, mirror: {firstMs, cachedMs, bytes, ok} \| null, mirrorSource, identical}` |
| `POST /demo/race` | Member | `{username, side: "lastfm" \| "mirror", page, tail}` | `{ms, count, total, totalPages, tracks: [{name, artist, image, uts}]}` |

- `state` is `mirrored`, `importing` or `not_mirrored`. `scenarios` lists
  `{id, title, detail, requests}` for a mirrored user.
- `POST /demo/import` copies a history once, without live sync, up to `LFM_DEMO_MAX_SCROBBLES`
  (422 `history_too_large`). One import per account at a time (409 `demo_import_running`). A
  name Last.fm does not know is 404 `lastfm_user_not_found`.
- `run` and `race` need a mirrored user (409 `not_mirrored`). An unknown scenario is 404
  `unknown_scenario`. `tail` is capped at 50.

## Other errors

| Status and code | When |
|---|---|
| 401 `not_logged_in` | No valid login cookie. |
| 403 `account_disabled` | The account is disabled. |
| 404 `not_found` | The account, key or other record does not exist. |
| 500 `internal` | A server failure; details are in the log. |
