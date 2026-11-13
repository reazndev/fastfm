# Self-hosted Last.fm mirror: plan

A single-container service that keeps a local copy of a user's Last.fm data and serves it through
an API that behaves like `ws.audioscrobbler.com/2.0/`. A client switches over by changing the base
URL and the API key.

## 1. What "100% compatible" can mean

Last.fm data falls into three groups, and each one gets a different treatment.

| Group | Examples | How we serve it | Fidelity |
|---|---|---|---|
| User data | recent tracks, top artists/albums/tracks, weekly charts, loved tracks, library | Computed from our own scrobble table | Exact, no upstream call |
| Global catalogue | `artist.getInfo`, `track.getSimilar`, `tag.*`, `chart.*`, `geo.*`, search | Read-through cache of Last.fm's response, pre-warmed for everything in the user's library | Byte-identical to Last.fm, as fresh as the TTL |
| Writes | `track.scrobble`, `track.love`, tags | Written locally and forwarded to Last.fm | Same as Last.fm |

The global catalogue (listener counts, similar artists, wikis) only exists on Last.fm's servers,
so we cannot own it outright. The first request for an entity nobody has asked about still costs
one upstream call. Everything after that is local.

## 2. Tech stack

- **Rust** for the server: `axum` on `tokio`, `reqwest` for upstream calls, `quick-xml` and
  hand-written writers for output, `moka` for caches, `governor` for rate limiting, `argon2` for
  passwords, `mimalloc` as the allocator. One static binary, no garbage collector pauses.
- **SQLite in WAL mode**, embedded in the process (`rusqlite` with the bundled build). It is the
  durable store. An in-process database removes the network hop that Postgres would add to every
  request. One writer connection on its own thread, one read connection per worker thread with
  cached prepared statements.
- **Angular** for the web UI (standalone components, signals), built to static files and
  embedded in the binary with `rust-embed`. Screens: sign up and log in, Last.fm link and import
  status, API keys, app authorisation, admin (users, approvals, settings).
- **Docker** image with one volume at `/data`. Config through environment variables.

Rust compiles slowly, so the workspace is split into crates (`core`, `store`, `upstream`, `api`,
`server`) to keep incremental builds short during the overnight run.

Postgres becomes the better choice only if this should serve hundreds of users from several
instances. The storage layer sits behind one trait, so that change stays possible.

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
- **Upstream client.** The only code that talks to Last.fm. One global token bucket (default
  4 requests per second, configurable) with a priority queue: client pass-through, then live
  sync, then import, then pre-warming. Retries with backoff on errors 8, 11, 16, 29 and on 5xx.
- **Jobs.** Import, live sync, reconciliation, and pre-warming. State lives in SQLite, so a
  restart resumes where it stopped.

## 4. Getting the data in

### Import

1. Record `T0 = now`. Request `user.getRecentTracks` with `limit=1000&to=T0` (plain, not
   `extended=1`, which blanks the per-track artist MBID), page by page. The fixed `to` keeps the
   pages stable while new scrobbles arrive.
2. Write each time window in one transaction as delete-then-insert for that window. A retry
   cannot create duplicates, and real same-second duplicates on Last.fm are kept.
3. Sync loved tracks, friends, tags, the profile, and `user.getWeeklyChartList`.
4. Queue pre-warming of `getInfo` for every artist, album, and track in the library.
5. Enrich: read the all-time top artists, albums and tracks (1000 per request) and call
   `artist.getCorrection` for scrobbled artist names they do not list. This gives Last.fm's
   library entities (corrected artists, albums under their album artist) with MBIDs, URLs,
   covers and durations, and a per-user map from each scrobbled artist, album (per spelling and
   cover) and track to the library entity it counts under. Every aggregate counts library
   entities; scrobbles keep their own names. New scrobbles of known names map at once, unknown
   ones count as themselves until the next enrichment, which runs daily. About 65 requests for
   160k scrobbles.

100k scrobbles are 500 pages, around 5 to 15 minutes at Last.fm's speed.

File import is a second path: the JSON and CSV formats from the common export tools
(lastfmstats, ghan.nl, benjaminbenben). These files lack MBIDs and cover art, so the API
import stays the default and a file import is followed by an enrichment pass.

### Live bridge

Last.fm has no webhooks, so the bridge polls. One request per user per tick:
`user.getRecentTracks&from=<last uts + 1>`.

- Every 10 seconds while a track is playing, backing off to 60 seconds when idle.
- The now-playing track is held in memory and served as the extra first item, as Last.fm does.
- Clients that send scrobbles to our `track.scrobble` are written locally at once and forwarded.

### Reconciliation

Users delete and edit scrobbles on Last.fm. A nightly job compares `@attr.total` for time ranges
(`limit=1` with `from`/`to`) against local counts, bisects any range that differs, and replaces
the smallest differing window. A full check costs a few dozen requests.

## 5. Storage

```
artists(id, name, name_fold, mbid, url, lib_name)
albums(id, artist_id, name, name_fold, mbid, image_hash, variant, url, artist_mbid)
    UNIQUE (artist_id, name_fold, variant): variant is spelling + cover when scrobbled, '' for
    a library album
tracks(id, artist_id, name, name_fold, mbid, url, duration, artist_mbid, artist_name)
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
- Period top lists (`7day` to `12month`) and weekly charts aggregate a range scan of a covering
  index. That is a few milliseconds for a year of scrobbles.
- A response cache holds the rendered bytes per (user, method, params, format). Each ingest bumps
  a per-user version number, which invalidates that user's entries. Repeat requests skip the
  database and the renderer.
- API keys live in an in-memory map, so auth costs no query.
- Cover art is stored as the image hash only. The four size URLs are generated from it.
- Names are matched through `name_fold` (Unicode case folding done in Go), because Last.fm
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

- Deep pages must seek by timestamp, never use `OFFSET`. Each user gets a page-anchor list (the
  `uts` of every 200th scrobble), kept in memory and updated on ingest. Other page sizes seek to
  the nearest anchor and skip at most 199 rows.
- Overall top lists must come from the `user_*_total` tables, not from aggregation.
- The 3, 6, and 12 month lists add `user_artist_day`, `user_album_day`, and `user_track_day`
  rollups (user, day, entity, plays). A year becomes 365 small groups to merge.
- The response cache covers repeats of everything else.

Targets: under 1 ms p99 server time for every local method, cached or not, measured with `oha`
against a 1.5M-scrobble user in a 50-user database.

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
   unaffected: `limit` above 200 (up to 10,000), a batch call that runs several methods in one
   request, and a streamed NDJSON export of a whole history. Fetching 1.5M scrobbles drops from
   7,500 requests to 150, or to one.
5. **In-memory read model.** Each user's scrobbles as a sorted array of `(uts, track_id)`, 8
   bytes per row, 600 MB for 75M rows, loaded from SQLite at start. A day lookup becomes a
   binary search of about 100 ns. SQLite stays the source of truth and the write path.

Connection handling matters as much as the server: HTTP keep-alive and HTTP/2 are on, because
one TLS handshake costs more than a hundred queries.

### Upstream key pool

`LASTFM_API_KEYS` holds several keys. The upstream client rotates through them with one token
bucket per key. Milestone 0 measures whether Last.fm limits per key or per IP. If the limit is
per IP, the pool gives no extra throughput and the client falls back to one shared bucket.
A 1.5M-scrobble import is 7,500 pages: one to two hours on a single stream, about ten minutes
if ten keys can run in parallel.

## 6. Auth

**Accounts.** People sign up with a username and password (argon2id). Login sets an HttpOnly,
SameSite=Strict session cookie. The first account created becomes the admin.

**Sign-up mode.** An instance setting with three values:

- `approval` (default): a new account is created as pending. It can log in and sees a waiting
  notice, but cannot link Last.fm, import, or create API keys until the admin approves it.
- `open`: new accounts are active at once.
- `closed`: the sign-up form is off.

The admin screen lists pending accounts with approve and reject, and can disable an account or
make another user an admin. Disabling an account stops its sync jobs and rejects its API keys.

**Last.fm link.** Each account links one Last.fm username. Reading needs only the name, since
the data is public. Forwarding writes needs the Last.fm web auth step, which also proves the
account owns that profile.

**Own Last.fm keys.** Each user can enter their own Last.fm API key and shared secret in their
settings. The upstream client then uses that key for the user's import, live sync, and forwarded
writes, with its own token bucket. Without one, the user's jobs share the instance key pool. An
instance may run with an empty pool, in which case every user must bring a key. Stored Last.fm
secrets and session keys are encrypted with a key file generated in `/data` on first start.

**Reads across users.** Any valid API key can read any mirrored user, as on Last.fm.

**API keys.** Each account creates its own. Generated in the UI as 32 hex characters with a 32 hex shared secret, the same
format Last.fm uses, so clients that validate the format accept them. Each key has a label, a
read or read-write scope, a last-used time, and a revoke button. The secret is stored readable,
because verifying `api_sig` (MD5 of the sorted parameters plus the secret) needs the plain value.

**Session keys for writes.** The Last.fm flow is emulated: `auth.getToken`, approval on our own
`/api/auth` page, then `auth.getSession`. `auth.getMobileSession` accepts the local username and
an app password. Our session key maps to the user's real Last.fm session key, and the upstream
client re-signs forwarded writes with the real secret.

**Upstream credentials.** The instance owner creates one Last.fm API application and enters its
key and secret during setup. Reads need only that key and a username. Linking the account through
Last.fm web auth stores a session key, which forwarded writes and private profiles need.

**Unknown users.** A request for a username we do not track passes through to Last.fm and is
cached. An `X-Mirror-Source: local|cache|upstream` header shows where a response came from.

## 7. Method coverage

- **Local:** `user.getRecentTracks`, `getTopArtists`, `getTopAlbums`, `getTopTracks`,
  `getWeeklyChartList`, `getWeeklyArtistChart`, `getWeeklyAlbumChart`, `getWeeklyTrackChart`,
  `getInfo`, `getLovedTracks`, `getFriends`, `getTopTags`, `getPersonalTags`, `library.getArtists`
- **Cached, with local `userplaycount` and `userloved` filled in:** `artist.getInfo`,
  `album.getInfo`, `track.getInfo`, and the three `getTags` methods
- **Cached pass-through:** the remaining `artist.*`, `album.*`, `track.*` read methods, `tag.*`,
  `chart.*`, `geo.*`
- **Write-through:** `track.scrobble`, `track.updateNowPlaying`, `track.love`, `track.unlove`,
  `addTags`, `removeTag`
- **Emulated:** `auth.getToken`, `auth.getSession`, `auth.getMobileSession`

Cache lifetimes: `getInfo` 7 days, similar and top lists 30 days, search 1 day, charts and geo
1 hour. An expired entry is served at once and refreshed in the background.

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
- Still missing: a shared secret for one of the keys (`LASTFM_API_SECRET`), needed only for
  forwarding scrobbles and loves.
- Decided: multi-user with accounts and admin approval, 1 to 50 users, SQLite, Rust.

Last.fm's API terms limit how long API data may be stored. For a personal mirror of your own
listening history the risk is low, but a public multi-user instance is a different case.
