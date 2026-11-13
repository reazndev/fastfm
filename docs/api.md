# API reference

lfm-mirror answers Last.fm API 2.0 requests at `/2.0` and `/2.0/`, by `GET` or by `POST` with a
form body, in XML (the default), JSON (`format=json`) or JSONP (`format=json&callback=name`).
Method names are matched ignoring case, as on Last.fm. Parameters, signatures, error codes and
HTTP statuses follow Last.fm; [fixtures/NOTES.md](../fixtures/NOTES.md) records what Last.fm
actually returns where its documentation is wrong or silent.

Every request needs `api_key`, which must be a key created on the mirror's **API keys** page,
not a Last.fm key.

## How each method is served

The terms used in the table:

- **Local**: computed from the mirror's database. No Last.fm call. When the requested user is not
  mirrored on this instance, the request is forwarded to Last.fm instead and cached for 5
  minutes.
- **Cached with local overlay**: Last.fm's answer from the read-through cache, with
  `userplaycount` (and `userloved` for tracks) replaced by the local values when `username` (or
  `user`) names a mirrored user.
- **Cached pass-through**: Last.fm's answer, stored byte for byte and served from the cache until
  it expires.
- **Write-through**: applied locally, then forwarded to Last.fm.
- **Emulated**: implemented by the mirror with its own accounts; Last.fm is not involved.

| Method | Served | Cache lifetime |
|---|---|---|
| `user.getInfo` | Local | |
| `user.getRecentTracks` | Local | |
| `user.getTopArtists`, `user.getTopAlbums`, `user.getTopTracks` | Local | |
| `user.getLovedTracks` | Local | |
| `user.getWeeklyChartList` | Local | |
| `user.getWeeklyArtistChart`, `user.getWeeklyAlbumChart`, `user.getWeeklyTrackChart` | Local | |
| `library.getArtists` | Local | |
| `artist.getInfo`, `album.getInfo`, `track.getInfo` | Cached with local overlay | 7 days |
| `user.getFriends`, `user.getTopTags`, `user.getPersonalTags` | Cached pass-through | 5 minutes |
| `artist.getTags`, `album.getTags`, `track.getTags` | Cached pass-through | 1 hour |
| `chart.getTopArtists`, `chart.getTopTags`, `chart.getTopTracks` | Cached pass-through | 1 hour |
| `geo.getTopArtists`, `geo.getTopTracks` | Cached pass-through | 1 hour |
| `artist.search`, `album.search`, `track.search` | Cached pass-through | 1 day |
| `tag.getInfo` | Cached pass-through | 7 days |
| `artist.getCorrection`, `artist.getSimilar`, `artist.getTopAlbums`, `artist.getTopTags`, `artist.getTopTracks` | Cached pass-through | 30 days |
| `album.getTopTags` | Cached pass-through | 30 days |
| `track.getCorrection`, `track.getSimilar`, `track.getTopTags` | Cached pass-through | 30 days |
| `tag.getSimilar`, `tag.getTopAlbums`, `tag.getTopArtists`, `tag.getTopTags`, `tag.getTopTracks`, `tag.getWeeklyChartList` | Cached pass-through | 30 days |
| `track.scrobble`, `track.updateNowPlaying`, `track.love`, `track.unlove` | Write-through | |
| `auth.getToken`, `auth.getSession`, `auth.getMobileSession` | Emulated | |

Any other method answers error 3 (invalid method). Without `LASTFM_API_KEYS` on the server, the
pass-through methods answer error 3 too, and unmirrored users are not forwarded.

### Cache behaviour

- Lookups by `artist`, `album`, `track` and `country` ignore case, as Last.fm does, so `Cher` and
  `cher` share one cache entry. Search terms, tags and user names keep their case.
- An expired entry is served at once and refreshed in the background. Expired entries are kept
  for 30 days, so they can still be served while Last.fm is down.
- Last.fm error answers (such as "artist not found") are cached for 5 minutes.
- Concurrent requests for the same uncached entry share one Last.fm call.
- When Last.fm is unreachable and nothing is cached, the mirror answers error 16 (temporarily
  unavailable), or error 29 when Last.fm's rate limit outlasted the retries.
- After a user's import, the mirror fetches `artist.getInfo`, `album.getInfo` and
  `track.getInfo` (JSON, with the user's name) for their 2,000 most-played entities of each kind,
  so those first requests are already cached. `LFM_PREWARM_RANKS` changes the number.

### Local methods

- Local answers are rendered once and kept in memory per user, method, parameters and format
  until the user's next new scrobble or change (`LFM_CACHE_MB`).
- `user.getRecentTracks` shows the track playing now as the first item, as Last.fm does, from live
  sync. That answer is not cached.
- `limit` on `user.getRecentTracks` accepts 1 to 1000, which is Last.fm's real bound (its
  documentation says 200). Top, loved and library lists clamp `limit` to 1000.
- Period lists (`7day` to `12month`) and weekly charts are computed from the scrobbles. Top lists
  and charts count each scrobble under the artist, album and track Last.fm counts it under, which
  the enrichment job learns from Last.fm's own top lists.

## Extensions

Extensions are additions. A request that Last.fm also accepts gets Last.fm's answer.

| Extension | What it does |
|---|---|
| `bulk=1` on `user.getRecentTracks` | Raises the `limit` bound from 1000 to 10,000. |
| `GET /2.0/export` | Streams a whole history as NDJSON (below). |
| `X-Mirror-Source` header | Where the answer came from: `local`, `cache` (fresh cached copy), `stale` (expired copy, refresh started) or `upstream` (fetched from Last.fm for this request). |
| `ETag` and `If-None-Match` | Every 200 answer carries a strong `ETag`. A request whose `If-None-Match` names it gets `304 Not Modified` with no body. |
| Compression | A request with `Accept-Encoding` gets zstd or gzip (zstd preferred on a tie) with `Content-Encoding` and `Vary: accept-encoding`. Bodies under 1 KB stay uncompressed. A request without the header gets exactly Last.fm's bytes and headers. |

CORS headers match Last.fm's (`access-control-allow-origin: *`), and `OPTIONS` preflights are
answered. JSONP answers also carry `X-Content-Type-Options: nosniff`.

### Export

```
GET /2.0/export?user=NAME&api_key=YOUR_KEY[&from=UNIX_TIME][&to=UNIX_TIME]
```

Answers `application/x-ndjson`: one JSON object per line, one line per scrobble, newest first.
`from` is inclusive and `to` exclusive, as in `user.getRecentTracks`. The range ends at the time
of the request, so scrobbles arriving during the export do not shift it. Each line has these
fields, in this order:

| Field | Type | Meaning |
|---|---|---|
| `uts` | number | Scrobble time, Unix seconds. |
| `artist`, `artist_mbid` | string | Artist as scrobbled, and its MusicBrainz id or `""`. |
| `album`, `album_mbid` | string | Album as scrobbled, or `""`. |
| `track`, `track_mbid` | string | Track title and MusicBrainz id. |
| `image` | string | URL of the largest cover, or `""`. |
| `loved` | number | `1` if the track is loved, else `0`. |

Errors are Last.fm-style JSON: a missing key is error 6, an unknown key error 10, and a user
this instance does not mirror is error 6 with "User not found". The export does not send
`X-Mirror-Source`.

## Authentication

Read methods need only `api_key`. Writes need a signed request with a session key, as on
Last.fm, and the API key must have the read and write scope.

**Signatures.** `api_sig` is the MD5 hex digest of every parameter except `format`, `callback`
and `api_sig`, sorted by name and written as `namevalue` with nothing between, followed by the
shared secret of the API key. This is Last.fm's scheme; use the secret shown on the mirror's
**API keys** page.

**Web flow** (desktop and web apps):

1. The app calls `auth.getToken` (no signature needed) and gets a token valid for one hour.
2. The app opens `http://HOST:3055/api/auth/?api_key=YOUR_KEY&token=TOKEN` in a browser. The
   mirror redirects to its own approval page, where the user logs in and approves the app.
3. The app calls `auth.getSession` with `token`, signed. The answer is `<session>` with `name`
   (the linked Last.fm username, or the account name if none is linked), `key` and `subscriber`.

**Mobile flow.** `auth.getMobileSession` takes `username` and `password`, signed. These are the
mirror account's name and password, not the Last.fm ones. A wrong pair is error 4. Failed
attempts are rate limited per username together with web logins, and each API key may check
10 passwords a minute and get 30 tokens a minute; past that the answer is error 29.

Session keys do not expire. Revoking the API key, or an admin disabling the account, ends them
at once; the key then answers error 10.
A session key works only with the API key it was issued to.

**Writes.** `track.scrobble` (one item, or a batch of up to 50 as `artist[0]`, `track[0]`, …),
`track.updateNowPlaying`, `track.love` and `track.unlove` take `sk` and `api_sig`. The account
must have a linked Last.fm user. Each write is stored locally and answered at once. If the
account has done Last.fm web auth (**Connect Last.fm**), the write is forwarded to Last.fm,
signed with the account's own Last.fm key or the instance key; a failed forward is retried with
backoff for about a day and a half. Without web auth, writes stay local. An unsigned
`track.scrobble` gets Last.fm's plain-text answer from the old submissions protocol.

## Errors

Errors use Last.fm's codes, messages and HTTP statuses, in the requested format. The checks run
in Last.fm's order: missing `api_key` (6), unknown method (3), invalid key (10); for writes,
missing `sk` or `api_sig` (6), bad signature (13), then the session key (9).

| Code | Meaning | HTTP |
|---|---|---|
| 3 | Invalid method | 400 |
| 4 | Authentication failed (wrong password, key without write scope, no linked Last.fm user) | 403 |
| 6 | Invalid parameters, or not found (the message says which; `User not found` is 404) | 400 or 404 |
| 8 | Operation failed on the server | 500 |
| 9 | Invalid session key | 403 |
| 10 | Invalid API key (unknown, revoked, or its account disabled) | 403 |
| 13 | Invalid signature | 400 |
| 14 | Token not yet approved | 403 |
| 15 | Token expired | 403 |
| 16 | Last.fm unreachable and nothing cached | 503 |
| 29 | Last.fm rate limit exceeded and nothing cached | 429 |

Errors returned by Last.fm for pass-through methods are replayed as Last.fm sent them.
