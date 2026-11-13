# Operations

How to run lfm-mirror for real: what it stores, how to back it up and upgrade it, how to put it
behind HTTPS, and what it costs in disk, memory and Last.fm requests. The configuration table is
in the [README](../README.md#configuration).

## Data directory

Everything lives in `LFM_DATA_DIR` (`/data` in the Docker image, a volume in
[docker-compose.yml](../docker-compose.yml)).

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
