# Operations

How to run lfm-mirror for real: what it stores, how to back it up and upgrade it, how to put it
behind HTTPS, and what it costs in disk, memory and Last.fm requests.

## Configuration

Settings come from environment variables. The server also reads a `.env` file in its working
directory for any variable the environment does not set.

| Variable | Default | Meaning |
|---|---|---|
| `LFM_BIND` | `0.0.0.0:3055` | Address and port to listen on. |
| `LFM_DATA_DIR` | `/data` | Directory for the database and `secrets.key`. |
| `LFM_DB` | `lfm.db` in the data directory | Path of the SQLite database. |
| `LASTFM_API_KEYS` | none | Last.fm API keys, comma separated: the instance key pool for imports, sync, the catalogue cache and the speed page. Without keys, only accounts with their own key can sync, and a catalogue request not yet cached uses the own key of the account behind the calling API key (error 16 when it has none). |
| `LASTFM_API_SECRET` | none | Shared secret of the first key in `LASTFM_API_KEYS`. Needed for Last.fm web auth and forwarded writes of accounts without their own key. |
| `LFM_PUBLIC_URL` | none | Where browsers reach the instance, such as `https://fm.example.org`. Used for the Last.fm web auth callback; without it the request's `Host` header is used. |
| `LFM_TRUST_PROXY` | `false` | Trust `X-Forwarded-For` and `X-Forwarded-Proto` from a reverse proxy. The login rate limit then uses the rightmost `X-Forwarded-For` entry, the one the proxy added. Leave off when clients connect directly. |
| `LFM_SETUP_TOKEN` | none | When set, the first sign-up must give this token to become the admin. The sign-up screen asks for it while no account exists. |
| `LFM_SECURE_COOKIES` | `auto` | `auto`, `always` or `never`. `auto` sets `Secure` on the login cookie when `LFM_PUBLIC_URL` is HTTPS, or else when the request came over HTTPS. |
| `LFM_CACHE_MB` | `128` | Memory for rendered responses and their compressed copies, in MiB. `0` turns the response cache off. |
| `LFM_DB_READERS` | one per CPU | SQLite read connections, each on its own thread. |
| `LFM_DB_CACHE_MB` | `8` | SQLite page cache per connection, in MiB. |
| `LFM_DB_MMAP_MB` | `0` | SQLite memory map per connection, in MiB. |
| `LFM_PREWARM_RANKS` | `2000` | How many of a user's top artists, albums and tracks get `getInfo` fetched ahead of time. |
| `LFM_PREWARM_INTERVAL_MS` | `1000` | Pause after each pre-warm call. |
| `LFM_DEMO_MAX_SCROBBLES` | `2000000` | Largest history the speed page will import for a user who is not mirrored. |
| `RUST_LOG` | `info` | Log filter, in `tracing` syntax. |
| `LFM_FOLLOW` | none | Development: Last.fm users to import and keep in sync besides linked ones. |
| `LFM_NOW` | none | Development and tests: freeze the clock at this Unix time. Turns sync off. |
| `LFM_DEV_API_KEY`, `LFM_DEV_API_SECRET` | none | Development and tests: register one read-write API key in memory. The secret defaults to the key. |

## Accounts and access

**Sign-up modes.** The admin picks one under **Admin**: `approval` (the default; new accounts
wait for an admin), `open`, or `closed`. A pending account can log in but cannot link Last.fm or
create API keys. Admins can approve, reject, disable and re-enable accounts and change roles.

**API keys.** Each account creates its own keys on the **API keys** page: 32 hex characters with
a 32-character shared secret, the same format Last.fm uses. A key is read-only or read and
write. Any valid key can read any user this instance mirrors, as on Last.fm. Revoking a key ends
every session issued to it.

**Your own Last.fm key.** Under **Settings** an account can enter its own Last.fm API key and
shared secret. Its import, live sync and forwarded writes then use that key instead of the
instance pool, and on an instance without a pool so do catalogue requests made with the
account's API keys. The secret is encrypted at rest with `secrets.key`.

**Private profiles.** Last.fm refuses to list the scrobbles of a private profile (error 17). The
import then fails with a message and is retried once a day, in case the profile becomes public.

## What runs where

```
 Last.fm client --> /2.0/ --> dispatcher (lfm-api)
                                 |
                                 +-- user.*, library.* ----> response cache --> SQLite (lfm-store)
                                 |
                                 +-- catalogue reads ------> read-through cache --+
                                 |                           (SQLite + memory)    |
                                 +-- track.* writes -------> local write + outbox -+
                                                                                  |
 browser -------> /app/api (lfm-web): accounts, API keys, Last.fm link            |
                                                                                  v
 lfm-sync: import, live sync, reconcile, enrichment --> lfm-upstream --> Last.fm
 pre-warm: getInfo for each user's top entities ------>   (rate limit,
                                                           key pool)
 lfm-sync writes scrobbles, loved tracks and the library map into SQLite.
```

- **Import** copies a whole history with `user.getRecentTracks` at 1,000 scrobbles per page,
  walking eight time ranges in parallel. It saves its place after every window, so a restart
  resumes. Then it reads the profile, loved tracks and the weekly chart list.
- **Enrichment** reads the user's all-time top lists to learn which artist, album and track
  Last.fm counts each scrobble under (corrected artist names, albums filed under their album
  artist). Local top lists and charts count the same way. It runs after the import and daily.
- **Live sync** polls each linked user for new scrobbles and the now-playing track: every 10
  seconds while something plays, backing off to 60 seconds when idle.
- **Reconcile** compares scrobble counts per time slice with Last.fm once a day and replaces the
  slices that differ, which catches scrobbles deleted on Last.fm.
- **Local methods** read SQLite through rollup tables (1, 8 and 64-day buckets), and the
  rendered bytes are cached per user until the next new scrobble.
- **Catalogue cache** stores Last.fm's answer byte for byte. An expired answer is served at once
  and refreshed in the background. **Pre-warm** fetches `getInfo` for each user's most-played
  entities after the import.
- **Write-through** stores a scrobble or love locally, answers the client, and forwards it to
  Last.fm. A failed forward goes to an outbox and is retried with backoff.

Every call to Last.fm goes through `lfm-upstream`, which limits each key to 4 requests per second
and serves waiting clients before sync, import and pre-warm.

## Data directory

Everything lives in `LFM_DATA_DIR` (`/data` in the Docker image, a volume in
[compose.yaml](../compose.yaml)).

| File | Contents |
|---|---|
| `lfm.db` | The SQLite database: scrobbles, aggregates, accounts, API keys, sessions, jobs, the write outbox and the Last.fm response cache. |
| `lfm.db-wal`, `lfm.db-shm` | SQLite's write-ahead log and its index. The database runs in WAL mode, so recent writes can sit in the `-wal` file until a checkpoint. |
| `secrets.key` | A 32-byte key, created on first start with owner-only permissions. It encrypts the Last.fm shared secrets and session keys that accounts store. |

Back up both `lfm.db` and `secrets.key`. Without `secrets.key`, the stored Last.fm secrets and
session keys cannot be decrypted. A new key file is created on the next start, but every account
that had stored an own Last.fm key or done **Connect Last.fm** then gets an internal error on its
Last.fm settings, and the UI has no way to reset them. Scrobbles, accounts and the mirror's own
API keys do not depend on `secrets.key`.

`LFM_DB` can put the database elsewhere; `secrets.key` always stays in `LFM_DATA_DIR`.

## Backup

Do not copy `lfm.db` alone while the server runs: the newest writes may still be in
`lfm.db-wal`. Use one of these:

- **Online, with SQLite's backup.** With the `sqlite3` command-line tool on a machine that can
  read the data directory, either `sqlite3 lfm.db ".backup lfm-backup.db"` or
  `sqlite3 lfm.db "VACUUM INTO 'lfm-backup.db'"`. Both give a consistent single-file copy while
  the server keeps running. `VACUUM INTO` also compacts the copy. The Docker image does not
  include `sqlite3`.
- **Offline.** Stop the server (`docker compose stop`), copy the whole data directory, and start
  it again. On a clean stop the server finishes its sync state and the copy is consistent.

Copy `secrets.key` along with the database. It does not change after it is created.

The response cache in memory and the Last.fm answers in `upstream_cache` are rebuilt on demand,
so a restored backup only costs some Last.fm requests to warm up again.

## Upgrades and migrations

1. Back up the data directory.
2. Build the new image (`docker build -t lfm-mirror .` or `docker compose build`).
3. Restart the container.

On start the server applies any pending migrations from `migrations/` in order, each in its own
transaction, and logs `applied database migration`. `PRAGMA user_version` records how many have
run. Migrations only move forward: to go back to an older version, restore the backup taken
before the upgrade.

Imports, live sync and pre-warming keep their progress in the database. After a restart, an
unfinished import continues from the last window it saved. The server stops cleanly on `SIGTERM`
or Ctrl-C.

`GET /healthz` answers `ok` when the server is up; the Docker image uses it as its health check.

## Behind a reverse proxy with HTTPS

The server speaks plain HTTP on port 3055. To serve it over HTTPS, put a reverse proxy in front
and set:

| Variable | Value | Why |
|---|---|---|
| `LFM_PUBLIC_URL` | `https://fm.example.org` | The Last.fm web auth callback goes to `<LFM_PUBLIC_URL>/lastfm/callback`. Without it, the server builds the URL from the request's `Host` and scheme, which a proxy may change. |
| `LFM_TRUST_PROXY` | `true` | Read the client IP from `X-Forwarded-For` (for the login rate limit) and the scheme from `X-Forwarded-Proto`. |
| `LFM_SECURE_COOKIES` | `auto` (default) | With an `https://` public URL, `auto` always marks the login cookie `Secure`. |

The proxy should:

- forward every path to the server, including `/2.0/`, `/app/api/`, `/api/auth/` and the UI;
- set `X-Forwarded-For` and `X-Forwarded-Proto`, and pass the `Host` header through;
- pass `If-None-Match`, `Accept-Encoding` and the `ETag`, `Content-Encoding` and `Vary` headers
  through unchanged. The server already compresses with zstd or gzip when asked, so the proxy
  need not compress API answers again.

Only enable `LFM_TRUST_PROXY` when clients cannot reach the server directly. Otherwise a client
could send its own `X-Forwarded-For` and dodge the login rate limit.

The login cookie is `SameSite=Strict` and `HttpOnly`, and lasts 30 days.

## Resource sizing

Measured figures from the development instance:

| Resource | Figure |
|---|---|
| Database size | About 181 bytes per scrobble: 323 MB for 1.78M scrobbles across 15 users. |
| Resident memory | 519 MB idle with 15 users, measured before the cache settings were tuned. |

Memory depends mostly on these settings:

- `LFM_CACHE_MB` (128 MiB): rendered responses and their compressed copies.
- `LFM_DB_CACHE_MB` (8 MiB) per SQLite connection, times `LFM_DB_READERS` (one per CPU) plus the
  writer.
- `LFM_DB_MMAP_MB` (0): each connection's memory map counts towards resident size again, so
  leave it at 0 unless you have measured a gain.
- The catalogue cache keeps up to 64 MiB of Last.fm answers in memory in front of the database.

The operating system's page cache serves most database reads, so free memory for it helps more
than larger SQLite caches.

## Last.fm requests

The server calls Last.fm at most 4 times per second per key in `LASTFM_API_KEYS` (or per
account key), and serves waiting API clients first, then live sync, then imports, then
pre-warming.

| Work | Requests |
|---|---|
| Import | About 1 per 1,000 scrobbles, plus roughly 30 to 65 for enrichment (top lists at 1,000 entries per request and `artist.getCorrection` for unexplained names). |
| Enrichment | The same 30 to 65 again, once a day per user. |
| Live sync | 1 per user per poll: every 10 seconds while a track plays, backing off to 60 seconds when idle. |
| Reconcile | A few dozen per user per day: 32 slice counts, plus one request per split where counts differ. |
| Pre-warm | Up to 3 × `LFM_PREWARM_RANKS` (6,000 by default) `getInfo` calls after an import, one per `LFM_PREWARM_INTERVAL_MS` (1 s). Already-cached entries are skipped. |
| Catalogue requests | 1 per cache miss or background refresh. |

A user whose profile is private cannot be imported (Last.fm error 17). The import is marked
failed, and the user is retried once a day.

## Logs

Logs go to standard output. `RUST_LOG` sets the filter (default `info`), for example
`RUST_LOG=lfm_sync=debug,info`. API keys, secrets, session keys and passwords are never logged.
