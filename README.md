# lfm-mirror

lfm-mirror is a self-hosted copy of the Last.fm API. It imports your listening history from
Last.fm into a local SQLite database, keeps it in sync, and answers the same requests as
`https://ws.audioscrobbler.com/2.0/`. An app, scrobbler or stats site switches over by changing
the base URL and the API key. Your own history is then served from local data, in milliseconds,
without Last.fm's rate limit. It is meant for people who run tools against their own Last.fm data
and for small groups (one to a few dozen accounts) who share an instance.

## What "compatible" means

Last.fm data falls into three groups, and the mirror treats each one differently.

| Data | Examples | How the mirror serves it |
|---|---|---|
| Your listening data | recent tracks, top artists, albums and tracks, weekly charts, loved tracks, library | Computed locally from the imported scrobbles. No Last.fm call. Matches Last.fm field for field. |
| Catalogue data | artist and track info, similar artists, tags, charts, geo, search | A read-through cache: the first request goes to Last.fm, later ones are answered from the cache until it expires. `getInfo` gets your local play count filled in. |
| Writes | `track.scrobble`, `track.love`, `track.unlove`, `track.updateNowPlaying` | Applied locally at once, then forwarded to Last.fm. Failed forwards are retried. |

The mirror is tested against 420 recorded Last.fm responses (210 requests, each in XML and
JSON). 381 of the 420 cases pass, either byte for byte or with differences that an ignore rule
explains, such as global listener counts that change by the minute. Most of the rest are
catalogue lists whose order changes on Last.fm between two calls seconds apart. See
[Limits and known differences](#limits-and-known-differences) and
[conformance/README.md](conformance/README.md), which also explains how to run the harness
(`scripts/conformance.sh`).

## Quick start with Docker

You need Docker and a Last.fm API account (create one at
<https://www.last.fm/api/account/create>; it gives you an API key and a shared secret).

1. Build the image from the repository root. The build compiles the web UI and the server, so
   the first one takes a while.

   ```sh
   docker build -t lfm-mirror .
   ```

2. Start the server on port 3055 with a volume for its data:

   ```sh
   docker run -d --name lfm-mirror -p 3055:3055 -v lfm-data:/data \
     -e LASTFM_API_KEYS=YOUR_LASTFM_KEY \
     -e LASTFM_API_SECRET=YOUR_LASTFM_SECRET \
     lfm-mirror
   ```

   With Compose instead, put `LASTFM_API_KEYS=…` and `LASTFM_API_SECRET=…` in a `.env` file
   next to [docker-compose.yml](docker-compose.yml) and run `docker compose up -d`.

3. Open <http://localhost:3055/> and sign up. The first account becomes the admin and is active
   at once.
4. If you started the server without `LASTFM_API_KEYS`, open **Settings** and enter your own
   Last.fm API key and shared secret. With the instance key from step 2 you can skip this.
   Without an instance key, catalogue methods and the speed page stay off; your own key is used
   only for your import, sync and writes.
5. On the home page, enter your Last.fm username. The import starts in the background and the
   page shows its progress. It costs about one Last.fm request per 1,000 scrobbles.
6. Optional: choose **Connect Last.fm** to approve the mirror on last.fm. Scrobbles and loves
   from your apps are then forwarded to Last.fm; without it they stay local.
7. Open **API keys** and create a key. Choose read and write if an app will scrobble through it.
8. Call the API:

   ```sh
   curl 'http://localhost:3055/2.0/?method=user.getRecentTracks&user=syntiiix&api_key=YOUR_KEY&format=json&limit=5'
   ```

### Point your app at it

Change the base URL from `https://ws.audioscrobbler.com/2.0/` to `http://HOST:3055/2.0/` and use
the API key and secret from the mirror's **API keys** page instead of your Last.fm key. Requests,
parameters, signatures and responses stay the same. Apps that log users in send them to
`/api/auth/` on the mirror instead of last.fm; see [docs/api.md](docs/api.md#authentication).

## Speed

Measured on one 8-core development machine. The in-app speed page at `/demo` runs the same kind
of comparison against Last.fm for any user you mirror.

| Case | lfm-mirror | Last.fm |
|---|---|---|
| One client fetches the newest 500 scrobbles | about 2 ms | 1.8 to 4.9 s |
| 20 mixed requests, one after another | 73 ms | 30 s (about 400 times slower) |
| Load test, 15 users, 1.78M scrobbles, cached and small requests | 48,000 to 59,000 requests/s, 0.7 ms median | not tested |

## Configuration

Settings come from environment variables. The server also reads a `.env` file in its working
directory for any variable the environment does not set.

| Variable | Default | Meaning |
|---|---|---|
| `LFM_BIND` | `0.0.0.0:3055` | Address and port to listen on. |
| `LFM_DATA_DIR` | `/data` | Directory for the database and `secrets.key`. |
| `LFM_DB` | `lfm.db` in the data directory | Path of the SQLite database. |
| `LASTFM_API_KEYS` | none | Last.fm API keys, comma separated: the instance key pool for imports, sync, the catalogue cache and the speed page. Without keys, catalogue methods answer error 3 and only accounts with their own key can sync. |
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

## How it works

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
instance pool. The secret is encrypted at rest with `secrets.key`.

**Private profiles.** Last.fm refuses to list the scrobbles of a private profile (error 17). The
import then fails with a message and is retried once a day, in case the profile becomes public.

## Limits and known differences

- Catalogue answers can be as old as their cache lifetime (1 hour for charts, up to 30 days for
  similar artists and top lists). See [docs/api.md](docs/api.md) for each method.
- `user.getFriends`, `user.getTopTags` and `user.getPersonalTags` are not computed locally. They
  are forwarded to Last.fm and cached for 5 minutes.
- Tag writes (`addTags`, `removeTag`) and other methods not listed in docs/api.md answer error 3.
- Album counts can differ slightly from Last.fm. Last.fm files some scrobbles under an album
  artist that no API response shows; in the test account 11 of 6,198 albums get no plays
  locally. Every listed album and play count still matches.
- Some Last.fm values change between two identical calls: global counters, a few MusicBrainz ids
  that flip between two values, and the order of hourly charts and search results. The mirror
  cannot match these exactly. [conformance/ignore.toml](conformance/ignore.toml) lists each one
  with its reason.
- A scrobble edited on Last.fm without changing the count of its time slice is not detected.
- Scrobbles older than 14 days or more than a day in the future are ignored, as on Last.fm.
- Some response details have no recorded Last.fm example yet: a successful scrobble, the session
  response, and the now-playing item. They follow Last.fm's documentation.

[fixtures/NOTES.md](fixtures/NOTES.md) records what Last.fm actually does, including where it
contradicts its own documentation.

### Last.fm API terms

Last.fm's API terms limit how API data may be stored and used. A personal mirror of your own
listening history is the intended use. If you run a public instance for other people, you are
responsible for staying within Last.fm's terms.

## Development

Prerequisites: Rust stable (see `rust-toolchain.toml`; at least 1.90), Node.js 24 with npm for
the web UI, and `curl` and `python3` for the conformance script.

1. Build the web UI once, so the server can embed it: `npm --prefix web ci` then
   `npm --prefix web run build`. Without it the server still runs and serves a short notice
   instead of the UI.
2. Put your keys in `.env` at the repository root (`LASTFM_API_KEYS=…`, `LASTFM_API_SECRET=…`,
   `LASTFM_USER=…`). The file is ignored by git; never commit it.
3. Run the server with a local data directory:
   `LFM_DATA_DIR=data cargo run -p lfm-mirror`.

| Command | What it does |
|---|---|
| `scripts/check.sh` | Every check CI runs: file sizes, `cargo fmt`, `clippy` with warnings as errors, tests, and the web lint, format, test and build. Run it before each commit. |
| `scripts/conformance.sh` | Replays the fixtures against a server built from this tree, serving a copy of `data/conformance.db`. Pass `--only <method>` or `--expect conformance/expected.txt`. |
| `cargo run -p lfm-sync -- import --user NAME --db data/conformance.db` | Imports one user into a database file without the server. Also `enrich`, `reconcile` and `live`. |
| `cargo run -p lfm-record` | Records fixtures from the real Last.fm for `LASTFM_USER`. Run by hand only; add `--force` to overwrite and `--only <method-prefix>` to narrow. |
| `cargo run -p lfm-api --release --example bench` | In-process timing of every local method against a database copy; see the file for its variables. |

### Repository layout

| Path | Contents |
|---|---|
| `crates/core` | Response tree, XML and JSON writers, error codes. No I/O. |
| `crates/store` | SQLite storage. The only crate with SQL. |
| `crates/upstream` | The rate-limited Last.fm client. The only crate that calls Last.fm. |
| `crates/sync` | Import, live sync, reconcile and enrichment; the `lfm-sync` command. |
| `crates/api` | Method registry, dispatcher, response cache, read-through cache, one file per method under `src/methods/`. |
| `crates/web` | The JSON API behind the web UI at `/app/api`, and write forwarding. |
| `crates/server` | The `lfm-mirror` binary: configuration, routes, background jobs. |
| `crates/conformance`, `crates/recorder` | The conformance harness and the fixture recorder. |
| `web/` | The Angular web UI. |
| `migrations/` | Numbered SQL migrations, applied at start. |
| `fixtures/`, `conformance/` | Recorded Last.fm responses, ignore rules and the expected-pass list. |
| `docs/` | [API reference](docs/api.md), [web API](docs/web-api.md), [operations](docs/operations.md). |

[PLAN.md](PLAN.md) is the design record. Contribution rules are in [AGENTS.md](AGENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md); they apply to people and coding agents alike.

## Licence

The workspace `Cargo.toml` declares the licence as `AGPL-3.0-or-later`. The repository has no
`LICENSE` file yet.
