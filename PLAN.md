# Self-hosted Last.fm mirror: plan

A single-container service that keeps a local copy of a user's Last.fm data and serves it through
an API that behaves like `ws.audioscrobbler.com/2.0/`. A client switches over by changing the base
URL and the API key.

This file is the design record. User documentation is in `README.md` and `docs/`.

## Status (2026-11-13)

**Built.** Milestones 0 to 8 below:

- Workspace crates `core`, `store`, `upstream`, `sync`, `api`, `web`, `server`, plus
  `conformance` (the harness) and `recorder` (the fixture recorder). One binary, `lfm-mirror`,
  with the Angular UI embedded, and a Docker image.
- Import, live sync, daily reconciliation and daily enrichment; pre-warming of `getInfo`.
- Local `user.getInfo`, `getRecentTracks`, `getTopArtists/Albums/Tracks`, `getLovedTracks`,
  `getWeeklyChartList`, the three weekly charts, and `library.getArtists`.
- Read-through cache for the catalogue methods, with local `userplaycount`/`userloved` on the
  three `getInfo` methods.
- Write-through `track.scrobble`, `updateNowPlaying`, `love`, `unlove`, with a retry outbox.
- Emulated `auth.getToken`, `getSession`, `getMobileSession`.
- Accounts with sign-up modes, API keys, own Last.fm keys, Last.fm web auth, secrets encrypted
  with `secrets.key`.
- Response cache with gzip and zstd copies, ETags and 304s; `bulk=1` and `/2.0/export`.
- Conformance: 381 of 420 fixture cases pass exactly or with an ignore rule
  (`conformance/expected.txt`).

**Changed from the plan.**

- Rust throughout. Earlier drafts said Go in places; those are fixed below.
- Last.fm answers from two backends with different formatting (style A "classic" and style B
  "new", see `fixtures/NOTES.md`). The renderers support both, chosen per method.
- Top lists and charts count a canonical library map, not raw scrobble names. Enrichment reads
  the user's all-time top lists (1,000 per request) and `artist.getCorrection` to learn which
  artist, album and track Last.fm counts each scrobble under (`user_library_map`).
- Period lists use rollups at 1, 8 and 64-day buckets, and deep pages of recent tracks find
  their rows from per-bucket scrobble counts. There is no page-anchor list and no per-day-only
  rollup.
- Last.fm's real `limit` maximum on `user.getRecentTracks` is 1000, not the documented 200; the
  mirror enforces 1000, and `bulk=1` raises it to 10,000. Top, loved and library lists clamp at
  1000. No batch call was built.
- `user.getFriends`, `user.getTopTags` and `user.getPersonalTags` are forwarded and cached for
  5 minutes, not computed locally. The `getTags` methods are plain pass-through, without a local
  overlay.
- The store is a concrete type (`lfm_store::Store`), not a trait.
- The upstream key pool comes from `LASTFM_API_KEYS`; the probe found no rate limit at 8
  requests per second, so the per-key buckets at 4 per second stay.

**Open.**

- Tag writes (`addTags`, `removeTag`) are not implemented; they answer error 3.
- File import (lastfmstats, ghan.nl, benjaminbenben exports) is not built.
- Private profiles cannot be imported: reads never use the linked Last.fm session key, although
  the import's error message suggests connecting Last.fm.
- Losing `secrets.key` leaves accounts with sealed Last.fm credentials unable to open or reset
  them from the UI.
- No recorded Last.fm example yet for a successful scrobble or the session response (the
  now-playing item was seen live, see `fixtures/NOTES.md`); the default range of `getWeekly*Chart` without `from`/`to` is not pinned
  down.
- 11 albums of the fixture user get no plays locally (album count 6,187 against 6,198).
- Deep top-list pages need a stored collation key to get faster.
- No `LICENSE` file; `Cargo.toml` declares `AGPL-3.0-or-later`.

## 1. What "100% compatible" can mean

Last.fm data falls into three groups, and each one gets a different treatment.

| Group | Examples | How we serve it | Fidelity |
|---|---|---|---|
| User data | recent tracks, top artists/albums/tracks, weekly charts, loved tracks, library | Computed from our own scrobble table | Exact, no upstream call |
| Global catalogue | `artist.getInfo`, `track.getSimilar`, `tag.*`, `chart.*`, `geo.*`, search | Read-through cache of Last.fm's response, pre-warmed for everything in the user's library | Byte-identical to Last.fm, as fresh as the TTL |
| Writes | `track.scrobble`, `track.love` (tag writes not built) | Written locally and forwarded to Last.fm | Same as Last.fm |

The global catalogue (listener counts, similar artists, wikis) only exists on Last.fm's servers,
so we cannot own it outright. The first request for an entity nobody has asked about still costs
one upstream call. Everything after that is local.

## 2. Tech stack

- **Rust** for the server: `axum` on `tokio`, `reqwest` for upstream calls, hand-written XML
  and JSON writers for output, `moka` for caches, `governor` for rate limiting, `argon2` for
  passwords, `mimalloc` as the allocator. One static binary, no garbage collector pauses.
- **SQLite in WAL mode**, embedded in the process (`rusqlite` with the bundled build). It is the
  durable store. An in-process database removes the network hop that Postgres would add to every
  request. One writer connection on its own thread, one read connection per worker thread with
  cached prepared statements.
- **Angular** for the web UI (standalone components, signals), built to static files and
  embedded in the binary with `rust-embed`. Screens: sign up and log in, Last.fm link and import
  status, API keys, app authorisation, admin (users, approvals, settings), and a speed
  comparison with Last.fm (`/demo`).
- **Docker** image with one volume at `/data`. Config through environment variables. After CI
  passes on `main`, the `Image` workflow publishes it as `ghcr.io/reazndev/fastfm`, tagged
  `latest` and with the short commit SHA.

Rust compiles slowly, so the workspace is split into crates (`core`, `store`, `upstream`,
`sync`, `api`, `web`, `server`, plus the `conformance` harness and the fixture `recorder`) to
keep incremental builds short during the overnight run.

Postgres becomes the better choice only if this should serve hundreds of users from several
instances. All SQL lives in the `store` crate behind the `Store` type, so that change would stay
inside one crate, though `Store` is a concrete type rather than a trait.

## 3. Architecture

```
clients ──► /2.0/  ──► auth ──► response cache ──► method handler ──► SQLite
                                                        │
                                                        └─► upstream client ──► Last.fm
web UI ───► /app/api   ─► jobs (import, live sync, reconcile, pre-warm) ─┘
```

- **Method router.** Reads `method`, `format`, `callback` from the query string or a form body,
  on `/2.0` and `/2.0/`, for GET and POST. Sends CORS headers like Last.fm does.
- **Method handlers.** Each one builds a response tree. Two renderers turn the tree into XML
  (Last.fm's default) and JSON (`#text`, `@attr`, numbers as strings).
- **Upstream client.** The only code that talks to Last.fm. One token bucket per key (4
  requests per second; the server has no setting for it, `lfm-sync` takes `--rps`) with a
  priority queue: client pass-through, then live sync, then import, then pre-warming. Retries
  with backoff on errors 8, 11, 16, 29 and on 5xx.
- **Jobs.** Import, live sync, reconciliation, enrichment and pre-warming. State lives in
  SQLite, so a restart resumes where it stopped.

## 4. Getting the data in

### Import

1. Record `T0 = now`. Request `user.getRecentTracks` with `limit=1000&to=T0` (plain, not
   `extended=1`, which blanks the per-track artist MBID), page by page. The fixed `to` keeps the
   pages stable while new scrobbles arrive.
2. Write each time window in one transaction as delete-then-insert for that window. A retry
   cannot create duplicates, and real same-second duplicates on Last.fm are kept.
3. Sync loved tracks, the profile, and `user.getWeeklyChartList`. (Friends and tags are not
   synced; those methods are forwarded to Last.fm.)
4. Queue pre-warming of `getInfo` for the user's top artists, albums, and tracks
   (`LFM_PREWARM_RANKS`, default 2,000 of each).
5. Enrich: read the all-time top artists, albums and tracks (1000 per request) and call
   `artist.getCorrection` for scrobbled artist names they do not list. This gives Last.fm's
   library entities (corrected artists, albums under their album artist) with MBIDs, URLs,
   covers and durations, and a per-user map from each scrobbled artist, album (per spelling and
   cover) and track to the library entity it counts under. Every aggregate counts library
   entities; scrobbles keep their own names. New scrobbles of known names map at once, unknown
   ones count as themselves until the next enrichment, which runs daily. About 65 requests for
   160k scrobbles.

With 1,000 scrobbles per page, 100k scrobbles are 100 pages. Eight ranges are fetched in
parallel; an early run for the 160,643-scrobble fixture user (still with `extended=1`) took
54 s and 179 requests (`fixtures/NOTES.md`).

File import was planned as a second path: the JSON and CSV formats from the common export tools
(lastfmstats, ghan.nl, benjaminbenben). It is not built. These files lack MBIDs and cover art, so
the API import would stay the default and a file import would need an enrichment pass.

### Live bridge

Last.fm has no webhooks, so the bridge polls. One request per user per tick:
`user.getRecentTracks&from=<last uts + 1>`.

- Polling follows the user's last scrobble: every 10 seconds while a track plays or right after
  a new scrobble, every minute within 30 minutes of the last one, every 5 minutes within a day,
  and every 15 minutes after that, each pause moved by up to 10% so users drift apart.
  `SyncService::nudge` asks for an early poll when a read finds the data older than a minute.
- Background requests on one key (the instance pool, or an account's own key) share a budget
  of 3 per second per key, below the client's 4, so pass-through reads keep headroom. Live
  polls go first, then imports, then enrichment and reconciliation.
- A profile that hides its listening history (error 17) stops the import with status `private`
  and a message in the UI. It is retried once a day, counted from the last attempt across
  restarts, or at once from the "Retry import" button. Live sync handles a profile that turns
  private later the same way.
- The now-playing track is held in memory and served as the extra first item, as Last.fm does.
- Clients that send scrobbles to our `track.scrobble` are written locally at once and forwarded.

### Reconciliation

Users delete and edit scrobbles on Last.fm. A daily pass per user compares `@attr.total` for up
to 32 slices of the history (`limit=1` with `from`/`to`) against local counts, bisects any slice
that differs, and replaces the smallest differing window. Slices sit on a power-of-two grid, so
the same ranges come back on every pass. The schedule counts from the last pass, stored per
user, so restarts do not postpone it. A full check costs a few dozen requests. An edit that
keeps a slice's count the same is not detected.

Scrobbles sent late (offline scrobbling) arrive with times below the sync cursor, where live
polls never look. Live sync compares the 14 days below the cursor the same way, every 10 minutes
for users who scrobbled within a day and hourly otherwise: one request while nothing differs.

Last.fm's totals are not always additive (a range of 20,894 whose halves report 14,361 and
6,526). A range whose halves both match, or whose listing does not reach its total, is stored in
`upstream_mismatches` with both numbers and left alone for a week.

## 5. Storage

A sketch of the main tables; `migrations/` holds the real schema, including the rollup, tie and
histogram tables.

```
artists(id, name, name_fold, mbid, url, lib_name)
albums(id, artist_id, name, name_fold, mbid, image_hash, variant, url, artist_mbid)
    UNIQUE (artist_id, name_fold, variant): variant is spelling + cover when scrobbled, '' for
    a library album
albums.artist_name: the album artist as the top-albums list spells it (library albums)
tracks(id, artist_id, name, name_fold, mbid, url, duration, artist_mbid, artist_name,
       artist_spelling)
    UNIQUE (artist_id, name_fold, artist_spelling): Last.fm keeps tracks of artist spellings
    that differ only in case apart
scrobbles(user_id, uts, seq, track_id, album_id, artist_id)
    PRIMARY KEY (user_id, uts, seq) WITHOUT ROWID
    INDEX (user_id, artist_id, uts), INDEX (user_id, track_id, uts)
user_artist_total / user_album_total / user_track_total(user_id, entity_id, plays)
    INDEX (user_id, plays DESC)
user_library_map(user_id, kind, raw_id, lib_id)   -- scrobbled entity -> library entity
loved(user_id, track_id, uts, seq)
weekly_chart_list(user_id, from_uts, to_uts)
upstream_cache(key, format, body, status, fetched_at, expires_at)
accounts(id, name, password_hash, role, status, lastfm_user, own_key, own_secret, lastfm_sk)
settings, api_keys, sessions, auth_tokens, jobs
```

Why this is fast:

- `user.getRecentTracks` is a range scan on the primary key. No sort.
- Overall top lists walk the `plays DESC` index, so page 500 costs the same as page 1.
- Period top lists (`7day` to `12month`) are rolling windows, the N × 86,400 seconds before
  the request. Per-period totals start on the window's first whole UTC day and move forward
  daily; a read adds the plays of the partial day before that from the scrobbles table and
  corrects the play-count histogram for them. Weekly charts merge the time rollups.
- A response cache holds the rendered bytes per (user, method, params, format). Each ingest bumps
  a per-user version number, which invalidates that user's entries. Repeat requests skip the
  database and the renderer.
- API keys live in an in-memory map, so auth costs no query.
- Cover art is stored as the image hash only. The four size URLs are generated from it.
- Names are matched through `name_fold` (Unicode case folding done in Rust), because Last.fm
  lookups ignore case.

### Sizing and measured numbers

Target load: 1 to 50 users with 100k to 1.5M scrobbles each, so up to 75M rows. A prototype of
this schema with 20 users at 1.5M scrobbles each (30M rows, 1.3 GB with both indexes, about
44 bytes per row) gave these timings on the dev machine, single-threaded, from Python:

| Query | p50 | Last.fm, same query |
|---|---|---|
| One day of history, 200 rows with joins | 0.11 ms | 270 to 430 ms |
| Same, including JSON encoding | 0.56 ms | |
| Recent tracks, deep page by `uts` seek | 0.03 ms | 1100 ms (page 300) |
| Recent tracks, deep page by `OFFSET` | 10 ms | |
| Count of scrobbles in a 30-day range | 0.11 ms | |
| Top artists, 7 days | 0.19 ms | |
| Top artists, 12 months, raw aggregation | 10 ms | 370 ms |
| Top artists, overall, raw aggregation of 1.5M rows | 41 ms | 350 ms |
| Weekly artist chart | same as 7 days | 1200 ms |

What the numbers require:

- Deep pages must not scan from the newest scrobble. The plan was a page-anchor list; what was
  built instead keeps scrobble counts per 1, 8 and 64-day bucket (`user_scrobble_rollup`),
  updated by the same delta as the other rollups. Finding the n-th newest scrobble walks those
  counts and skips at most one day of rows, and no ingest rebuilds anything
  (`crates/store/src/counts.rs`).
- Overall top lists must come from the `user_*_total` tables, not from aggregation.
- Period lists use `user_artist_rollup`, `user_album_rollup` and `user_track_rollup` at 1, 8
  and 64-day widths. A year becomes about 5 wide buckets plus at most 14 narrower ones on each
  side, instead of 365 days (`crates/store/src/buckets.rs`).
- The response cache covers repeats of everything else.

Targets: under 1 ms p99 server time for every local method, cached or not, measured with `oha`
against a 1.5M-scrobble user in a 50-user database.

Measured on the built server (one 8-core development machine, 15 users, 1.78M scrobbles): the
newest 500 scrobbles in about 2 ms against 1.8 to 4.9 s on Last.fm; 20 mixed requests in 73 ms
against 30 s; cached and small requests at 48,000 to 59,000 requests per second with a 0.7 ms
median; about 181 bytes of database per scrobble (323 MB). No run against the 50-user target
above is recorded here.

The `/demo` page shows the difference live. The server times a fixed list of user calls on
Last.fm (first key of the pool, sequential, at most 4 per second, after an untimed warm-up) and
on its own `/2.0/` over loopback with an internal in-memory key: once after clearing the user's
response cache, once cached. A user who is not mirrored can be imported once for it, without
live sync, up to `LFM_DEMO_MAX_SCROBBLES` (default 2,000,000).

`/demo/live` races the two sides through a mirrored user's whole history. The browser asks
`POST /app/api/demo/race` for one `user.getRecentTracks` page of 200 at a time from each side,
both lanes at once. The server returns only the page's last few scrobbles, the request time and
the totals. Last.fm pages keep the 250 ms spacing and count against a limit of 1,000 per account
per hour. Mirror pages have no limit.

### Going faster than SQLite

Ordered by payoff. Steps 1 to 4 are in scope. Step 5 is built only if the benchmark in
milestone 8 misses the target.

1. **Rendered-response cache with pre-compressed bodies.** A 200-track page is about 120 KB of
   JSON, so bandwidth limits throughput long before the CPU does: 10,000 such responses per
   second is 1.2 GB/s uncompressed. The cache stores each response as identity, gzip, and zstd
   bytes and serves the variant the client accepts.
2. **ETag per user version.** Every response carries an ETag built from the user's version
   number. A client that sends `If-None-Match` gets a 304 with no body until the next scrobble.
3. **Pre-rendered track fragments.** A track's JSON and XML are the same in every response
   apart from the date. Each track keeps its rendered fragments, and a response is assembled by
   copying them. Encoding was four fifths of the day query's cost in the prototype.
4. **Extensions that cut request counts.** These are additions, so Last.fm clients are
   unaffected: `limit` above Last.fm's real maximum of 1000 (up to 10,000 with `bulk=1`), and a
   streamed NDJSON export of a whole history. Fetching 1.5M scrobbles drops from 1,500 requests
   to 150, or to one. A batch call was planned and not built.
5. **In-memory read model.** Each user's scrobbles as a sorted array of `(uts, track_id)`, 8
   bytes per row, 600 MB for 75M rows, loaded from SQLite at start. A day lookup becomes a
   binary search of about 100 ns. SQLite stays the source of truth and the write path.

Connection handling matters as much as the server: HTTP keep-alive and HTTP/2 are on, because
one TLS handshake costs more than a hundred queries.

### What milestone 8 built (2026-11-13)

- **Response cache** (`lfm_api::MemoryCache`, `LFM_CACHE_MB`, default 128): holds the final
  bytes per user, method, canonical parameters, format and JSONP callback, keyed by the user's
  data version. Each body makes its gzip (level 1) and zstd (level 3) copy on first demand and
  keeps it. A request with `Accept-Encoding` gets the best coding it accepts plus `Vary`; one
  without gets exactly the bytes and headers it got before. Bodies under 1 KB stay plain.
  Catalogue answers keep their copies on the pass-through entry. Brotli was left out: zstd and
  gzip cover the clients, and brotli at a useful level costs more per miss than both.
- **ETag and 304**: every 200 carries a strong ETag (`-gzip`/`-zstd` appended for the coded
  copies). Local methods derive it from the user's version and the canonical request, catalogue
  answers from their body. `If-None-Match` naming it gets a 304 without a body.
- **Hot path**: users are remembered by name until the next write (`store::memo`), so a cached
  request makes no database round trip. Bound `LIMIT`/`OFFSET` and range values in the history,
  bucket-walk and histogram statements are written `+?N`: with a bare `?N`, SQLite re-planned
  those statements on every request, which took about two thirds of all CPU under load and
  serialised the readers on SQLite's allocator lock. The JSON and XML writers escape in one pass
  and stop comparing names byte by byte. gzip is level 1 on `zlib-rs`: at level 6 on
  `miniz_oxide` it cost 22 ms per 1000-track page, the "30 µs per row" of the load test.
  Pre-rendered track fragments were built and measured, then removed: keeping a fragment per
  (track, album) item made repeated pages 40% faster but random pages through 15 large
  histories 40% slower, since most items there are seen once and the cache churns.
- **Extensions**: `bulk=1` lets `user.getRecentTracks` take `limit` up to 10,000; without it the
  bounds and errors stay Last.fm's. `GET /2.0/export?user=&api_key=[&from=&to=]` streams a whole
  history as NDJSON, newest first, one store page of 5,000 at a time. No batch call.
- **Store**: deep top-list pages read only names for the tie group and load the page's rows,
  and ASCII names skip Unicode folding (page 1000 of top tracks: 64 ms to about 28 ms; a real fix
  needs a stored collation key). Applying a library runs in transactions of at most 50,000
  moved plays, so the writer is never held for more than about a second (one 18 s transaction
  before). SQLite's cache per connection (`LFM_DB_CACHE_MB`, default 8), memory map
  (`LFM_DB_MMAP_MB`, default 0) and reader count (`LFM_DB_READERS`, default one per CPU) are
  settings; each connection's map counted the shared file pages towards resident size again.
- **Limits**: requests must send their head within 10 s and get a response head within 60 s
  (an export's body streams on), bodies are at most 1 MiB, and the open-file limit is raised to
  its hard maximum at start. The pass-through forwards and keys on each method's documented
  parameters only, and each account's API key may cause 5 Last.fm calls per second for
  uncached answers (`LFM_UPSTREAM_MISSES_PER_SECOND`; error 29 past it). Without
  `LASTFM_API_KEYS`, a miss uses the own Last.fm key of the account behind the API key, through
  `lfm_api::CallerKeys`, which the web crate is to implement; until then it answers error 16.
  A session key works only with the API key it was issued to (error 9 otherwise).

### Upstream key pool

`LASTFM_API_KEYS` holds several keys. The upstream client rotates through them with one token
bucket per key (4 requests per second each). Milestone 0 was to measure whether Last.fm limits
per key or per IP; the probe hit no limit at 8 requests per second on one key, so the question
is still open (`fixtures/NOTES.md`). The client also has a shared-bucket mode for a per-IP limit,
which only the `lfm-sync` command and the recorder use; the server uses per-key buckets. A 1.5M-scrobble import is 1,500 pages of 1,000.

## 6. Auth

**Accounts.** People sign up with a username and password (argon2id). Login sets an HttpOnly,
SameSite=Strict session cookie. The first account created becomes the admin.

**Setup token.** With `LFM_SETUP_TOKEN` set, the first sign-up must give it, so whoever
reaches a fresh instance first cannot claim the admin account. `GET /app/api/signup-mode` is
public and tells the sign-up screen the mode, whether any account exists, and whether the
token is needed.

**Sign-up mode.** An instance setting with three values:

- `approval` (default): a new account is created as pending. It can log in and sees a waiting
  notice, but cannot link Last.fm, import, or create API keys until the admin approves it.
- `open`: new accounts are active at once.
- `closed`: the sign-up form is off.

The admin screen lists pending accounts with approve and reject, and can disable an account or
make another user an admin. Disabling an account stops its sync jobs and rejects its API keys.

**Last.fm link.** Each account links one Last.fm username; a unique index makes the claim
atomic. Reading needs only the name, since the data is public. Writes from clients, local or
forwarded, need the Last.fm web auth step, which proves the account owns that profile; before
it they get error 4. A confirmed web auth link takes the name over from an account that only
claimed it. The web auth start puts a random `state` bound to the session into the callback
URL, and the callback refuses a missing, foreign, expired or reused one.

**Logins.** Lockouts count failures per username and client IP (the rightmost
`X-Forwarded-For` entry behind a trusted proxy), so a stranger cannot lock an owner out; per
username alone there is only a growing delay of up to 2 seconds. `auth.getMobileSession`, which
has no client IP, shares one ceiling of 60 checks a minute. Session cookies are stored as their
SHA-256 hash, and expired sessions and auth tokens are deleted hourly.

**Own Last.fm keys.** Each user can enter their own Last.fm API key and shared secret in their
settings. The upstream client then uses that key for the user's import, live sync, and forwarded
writes, with its own token bucket. Without one, the user's jobs share the instance key pool. An
admin setting, off by default, requires every account to save its own key before linking a
profile; the home screen then asks for the key first. A stored credential that no longer opens
(after losing `secrets.key`) counts as absent and can be entered again. An
instance may run with an empty pool, in which case every user must bring a key; the catalogue
cache and the speed comparison then stay off, because they use only the pool. Stored Last.fm
secrets and session keys are encrypted with a key file (`secrets.key`) generated in the data
directory on first start.

**Reads across users.** Any valid API key can read any mirrored user, as on Last.fm.

**API keys.** Each account creates its own. Generated in the UI as 32 hex characters with a 32 hex shared secret, the same
format Last.fm uses, so clients that validate the format accept them. Each key has a label, a
read or read-write scope, a last-used time, and a revoke button. The secret is stored readable,
because verifying `api_sig` (MD5 of the sorted parameters plus the secret) needs the plain value.

**Session keys for writes.** The Last.fm flow is emulated: `auth.getToken`, approval on our own
`/api/auth` page, then `auth.getSession`. `auth.getMobileSession` accepts the local account's
username and password (there are no separate app passwords). Our session key maps to the
account, whose real Last.fm session key the upstream client uses to re-sign forwarded writes with
the real secret.

**Upstream credentials.** The instance owner creates one Last.fm API application and sets its
key and secret through `LASTFM_API_KEYS` and `LASTFM_API_SECRET`; there is no setup screen.
Reads need only that key and a username. Linking the account through Last.fm web auth stores a
session key, which forwarded writes need. Private profiles were meant to use it too; reads do not
use it yet, so private profiles cannot be imported.

**Unknown users.** A request for a username we do not track passes through to Last.fm and is
cached for 5 minutes. An `X-Mirror-Source: local|cache|stale|upstream` header shows where a
response came from.

## 7. Method coverage

- **Local:** `user.getRecentTracks`, `getTopArtists`, `getTopAlbums`, `getTopTracks`,
  `getWeeklyChartList`, `getWeeklyArtistChart`, `getWeeklyAlbumChart`, `getWeeklyTrackChart`,
  `getInfo`, `getLovedTracks`, `library.getArtists`
- **Cached, with local `userplaycount` and `userloved` filled in:** `artist.getInfo`,
  `album.getInfo`, `track.getInfo`
- **Cached pass-through:** the remaining `artist.*`, `album.*`, `track.*` read methods (the three
  `getTags` included), `tag.*`, `chart.*`, `geo.*`, and `user.getFriends`, `getTopTags`,
  `getPersonalTags` (planned as local)
- **Write-through:** `track.scrobble`, `track.updateNowPlaying`, `track.love`, `track.unlove`.
  `addTags` and `removeTag` were planned and are not built.
- **Emulated:** `auth.getToken`, `auth.getSession`, `auth.getMobileSession`

Cache lifetimes: `getInfo` and `tag.getInfo` 7 days, similar lists, top lists, top tags and
corrections 30 days, search 1 day, charts and geo 1 hour, the `getTags` methods 1 hour, user
methods 5 minutes, Last.fm error answers 5 minutes. An expired entry is served at once and
refreshed in the background. The full table is in `docs/api.md`.

Errors use Last.fm's codes, messages, and HTTP statuses in both formats.

## 8. Proving compatibility

A conformance harness is the main tool, and it gives the overnight run a number to push up.

1. `record`: call real Last.fm for every method, in XML and JSON, with a set of parameter
   variants (pages, limits, periods, `extended`, missing parameters, bad keys). Save the
   responses as fixtures.
2. `replay`: send the same requests to our server after an import of the same account.
3. `diff`: compare structure, key order, types, and values. A short ignore list covers fields
   that change by the minute, such as global listener counts.

The fixtures also settle open questions by observation: the real rate limit, the weekly chart
boundaries, and which single-item lists collapse to an object in JSON.

As built, the fixtures hold 210 requests, each in XML and JSON (420 cases), and
`conformance/expected.txt` lists the 381 that pass exactly or with an ignore rule.
`conformance/README.md` explains how to run the harness and why the rest differ.

One user and a frozen clock hide bugs that only other histories show. A second, offline suite
(`fixtures-multi/`, `scripts/conformance-multi.sh`) replays Last.fm answers saved during a live
comparison for ten users against a copy of the database that comparison ran on, one server
start per user and format with the clock at its requests.

Known source of small differences: Last.fm's own period charts lag and are sometimes
inconsistent with its scrobble list. Ours are computed from the scrobbles, so they can be more
correct than the original. The harness reports these separately.

## 9. Milestones for the overnight run

Each milestone ends with a check that must pass before the next one starts.

| # | Work | Check |
|---|---|---|
| 0 | Repo, Cargo workspace, Docker, upstream client with rate limiter, fixture recorder | Fixtures recorded for all methods |
| 1 | Schema, import job, window-replace ingest | Local scrobble count equals `user.getInfo` playcount |
| 2 | Router, both renderers, error responses, accounts, sign-up modes, API key auth | Error fixtures match, pending account cannot create a key |
| 3 | Local `user.*` and `library.*` methods | Conformance diff clean for the group |
| 4 | Read-through cache, pass-through, `getInfo` overlay, pre-warm job | Conformance diff clean, second request makes no upstream call |
| 5 | Live sync and reconciliation | A new scrobble appears locally within one poll |
| 6 | Auth emulation and write-through | Signed scrobble from a test client reaches Last.fm |
| 7 | Angular web UI with admin screens, embedded in the binary | Sign-up, approval, link, import, first API call work in a fresh container |
| 8 | Response cache, compression, ETags, fragments, extensions, benchmarks, README | Latency targets met |

## 10. Needed before the run starts

- `LASTFM_API_KEYS` is in `.env`. Reads, import, and fixture recording work with it.
- `LASTFM_USER=syntiiix` is the account for fixtures and the first import.
- `LASTFM_API_SECRET` (the shared secret of the first key) is supported. It is needed only for
  Last.fm web auth and forwarding scrobbles and loves for accounts without their own key.
- Decided: multi-user with accounts and admin approval, 1 to 50 users, SQLite, Rust.

Last.fm's API terms limit how long API data may be stored. For a personal mirror of your own
listening history the risk is low, but a public multi-user instance is a different case.
