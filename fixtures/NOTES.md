# Fixture notes

Observed on 2026-11-12 against `ws.audioscrobbler.com/2.0/` for user `syntiiix` (160,643
scrobbles). Each fixture is `<method>/<variant>.json` and `.xml` (raw bytes) plus
`<variant>.meta.json` with the request parameters, status, content type and all response headers
for both formats. Re-record with `cargo run -p lfm-record` (add `--force` to overwrite,
`--only <method-prefix>` to narrow).

## Two backends

Last.fm answers from two different stacks, and they differ in almost every formatting detail.
Each method sits on one side, so a renderer needs both styles.

| | Style A ("classic") | Style B ("new") |
|---|---|---|
| Methods seen | `user.getRecentTracks`, `getTopArtists/Albums/Tracks`, `getWeeklyArtist/Album/TrackChart`, `getInfo`, `getLovedTracks`, `getFriends`, `library.getArtists`, `album.getInfo`, `auth.*`, `track.love/unlove/updateNowPlaying`, unknown-method error | `user.getWeeklyChartList`, `user.getTopTags`, `user.getPersonalTags`, `artist.*` (incl. `getInfo`), `track.*` reads (incl. `getInfo`), `album.getTags/getTopTags/search`, `tag.*`, `chart.*`, `geo.*` |
| XML declaration | `<?xml version="1.0" encoding="UTF-8"?>` | `<?xml version="1.0" encoding="UTF-8" ?>` (space before `?>`) |
| XML layout | Two-space indent, one element per line, empty container self-closes (`<recenttracks ... />`) | No indent. `<lfm status="ok"><container ...><first-child>` on the declaration's next line, then each element ends with `\n`, empty container is `<x></x>` |
| JSON `/` | Escaped as `\/` | Not escaped |
| JSON content type | `application/json` | `application/json` or `application/json; charset=UTF-8` (varies by method) |
| Error JSON | `{"message":"...","error":N}` | `{"error":N,"message":"...","links":[]}` |
| Error XML | `<lfm status="failed">\n  <error code="N">msg</error>\n</lfm>` | `<lfm status="failed"><error code="N">msg</error>\n</lfm>` |

Common to both: XML bodies end with `\n`, JSON bodies have no trailing newline. XML content type
is always `text/xml; charset=UTF-8`.

## Errors and HTTP status

The status depends on the error and on the backend, not on the error code alone.

| Case | Code | HTTP | Message |
|---|---|---|---|
| Invalid API key (GET or POST) | 10 | 403 | `Invalid API key - You must be granted a valid key by last.fm` |
| Missing `api_key` | 6 | 400 | `Invalid parameters - Your request is missing a required parameter` |
| Unknown method | 3 | 400 | `Invalid Method - No method with that name in this package` |
| `user.*` without `user`, or unknown user | 6 | 404 | `User not found` |
| `album.getInfo` unknown album | 6 | 404 | `Album not found` |
| `limit=0` on recent tracks | 6 | 400 | `limit param out of bounds (1-1000)` |
| `page=0` on recent tracks | 6 | 400 | `page param out of bounds (1-1000000)` |
| `user.getFriends` page past the end | 6 | 400 | `no such page` |
| `track.love/unlove/updateNowPlaying`, `auth.getSession` without signature | 6 | 400 | missing-parameter message |
| `artist.getInfo` unknown name, unknown mbid, or no artist | 6 | 200 | `The artist you supplied could not be found` |
| `track.getInfo` unknown | 6 | 200 | `Track not found` |
| `geo.*` unknown or missing country | 6 | 200 | `country param invalid` |
| `user.getPersonalTags` without `taggingtype` | 6 | 200 | `taggingtype param missing` |
| `artist.getTags` without `user` | 6 | **200 in JSON, 400 in XML** | `User not found` |

Not errors, although they look like they should be:

- Page past the end on recent, top and loved lists: 200, empty list, `page` echoed
  (`page="999999"`), totals intact.
- Invalid `period` (`2week`): silently treated as `overall`; the body equals `period=overall`.
- `tag.getInfo` for a nonexistent tag: 200 with `total` 0, `reach` 0.
- `auth.getToken` without `api_sig` returns a token (200).
- `limit=1000` on recent tracks works and returns 1000 items; the documented maximum of 200 is
  not enforced (the bound is 1-1000).
- POST with form parameters behaves exactly like GET.
- Unsigned `track.scrobble` (POST) returns `200 text/plain; charset=utf-8` with the body
  `FAILED Incorrect protocol version (missing clientID/version/username)\n`, from the old
  submissions protocol, in both formats.

## Single-item lists in JSON

Lists stay arrays with one item almost everywhere: `limit=1` on recent tracks, top lists, loved
tracks, friends, weekly charts, chart.\*, geo.\*, tag.\*, artist.getTopTracks all return `[{...}]`.

The exception seen: **`library.getArtists` with `limit=1` returns `"artist": {...}`** (an
object). Keep the one-or-many handling anyway; older Last.fm code paths are known for it.

Empty lists: `"track":[]` with `@attr` (style A), `{"tag":[],"@attr":{...}}` (style B). Some
style-B empties are odd: `artist.getTags` with no tags is `{"tags":{"#text":" ","@attr":{...}}}`
and in XML `<tags artist="..."> </tags>` (one space).

## Numbers and strings

Almost every number in JSON is a string, including all `@attr` paging values, `playcount`,
`listeners`, `uts`, `duration`, `rank`. The exceptions are real JSON numbers:

- `error` in every error body.
- `user.getInfo`: `registered.#text` and `spotify_expiry_estimate.#text` (but `unixtime` is a
  string). In `user.getFriends`, `registered.#text` is instead a string like `"2002-11-20 11:50"`.
- `album.getInfo`: `tracks.track[].duration`, `tracks.track[].@attr.rank`, `userplaycount`.
  (`track.getInfo` has `userplaycount` and `userloved` as strings.)
- `artist.getTopAlbums`: `playcount`.
- `toptags.tag[].count` in `artist/album.getTopTags`; `tag.getInfo` `total` and `reach`;
  `tag.getTopTags` `count`, `reach` and its `@attr` `offset`, `num_res`, `total`.

Flags are strings: `"0"`/`"1"` (`loved`, `subscriber`, `streamable`), `"true"` for
`nowplaying`. `streamable` is a plain string in recent tracks and top artists, but
`{"fulltrack":"0","#text":"0"}` in loved and top tracks.

## user.getRecentTracks

- `from` is inclusive, `to` is exclusive: `from=t&to=t+1` returns the scrobble at `t`,
  `from=t-1&to=t` and `from=t&to=t` return nothing (`range-*` variants).
- `extended=0` (default): `artist` is `{"mbid","#text"}`, no `loved`. `extended=1`: `artist` is
  `{"url","name","image":[4],"mbid"}` and a `"loved":"0"` field appears.
  In XML, extended artist is a nested element; plain artist is `<artist mbid="...">name</artist>`.
- Date: `"date":{"uts":"1794516558","#text":"12 Nov 2026, 20:49"}` (UTC, `%d %b %Y, %H:%M`).
- Empty result in XML: `<recenttracks user="Syntiiix" page="1" perPage="50" totalPages="0"
  total="0" />`.
- Now playing: **not captured**, the user was not playing during the run. Re-run
  `--force --only user.getRecentTracks` while a track plays to capture it. The documented shape is
  an extra first item with `"@attr":{"nowplaying":"true"}` and no `date`, beyond `limit`.
- `@attr.user` is the canonical name (`Syntiiix`) even when the request says `syntiiix`.
  `library.getArtists` is the exception: it echoes the request's casing.
- Images: 4 sizes (`small` 34s, `medium` 64s, `large` 174s, `extralarge` 300x300). Top artists
  and library add `mega`, which reuses the 300x300 URL. Weekly track charts have only 3 sizes.
  Missing art is the placeholder hash `2a96cbd8b46e442fc41c2b86b821562f.png`.

## Weekly charts

- `user.getWeeklyChartList` returns 1134 weeks starting `Sun 2005-02-13 12:00 UTC`, regardless of
  when the user registered. Every range is exactly 604800 s, from Sunday 12:00 UTC to the next
  Sunday 12:00 UTC, contiguous. The last one ended `Sun 2026-11-08 12:00`; a run on Thursday
  did not list the current week.
- `getWeekly*Chart` without `from`/`to` is **not** the last listed week: it returned
  `Wed 2026-11-04 12:00` to `Wed 2026-11-11 12:00` UTC, recorded Thu 2026-11-12 21:30 UTC: seven
  days ending at a noon about a day and a half before the request. Record again on another
  weekday to pin the rule down.
- Off-grid `from`/`to` (one calendar day) are accepted, echoed in `@attr` as given, and the chart
  covers that range.
- A week before the user's registration returns an empty list, not an error.
- `limit` is honoured (`limit=1` gives one item) although undocumented.
- `@attr` order in JSON is `from`, `user`, `to`.

## Headers

Every response: `access-control-allow-origin: *`, `access-control-allow-methods: POST, GET,
OPTIONS`, `access-control-max-age: 86400`, `server: openresty`, `via: 1.1 google`, `alt-svc`.
The style-B methods whose JSON type is `application/json; charset=UTF-8` (`album.getTags`,
`album.getTopTags`, `album.search`, `artist.getCorrection/getTags/getTopAlbums/getTopTags`,
`chart.getTopTags`, `tag.getInfo/getSimilar/getTopAlbums/getTopTags/getWeeklyChartList`,
`track.getCorrection/getTags/getTopTags`, `user.getTopTags`, `user.getWeeklyChartList`) add
`access-control-request-headers: Origin, X-Atmosphere-tracking-id, X-Atmosphere-Framework,
X-Cache-Date, Content-Type, X-Atmosphere-Transport, *` to both formats. `tag.getTopTags` differs: no `max-age`, adds
`access-control-allow-headers` with that same list, `allow-methods` includes `PUT`, and a
`tsvc-time-spent` header. No caching headers (`cache-control`, `etag`) anywhere.

## JSONP

`format=json&callback=cb` returns `cb({...});` with `text/javascript; charset=utf-8` and no
`content-length`. The callback name is **not validated**: `callback=bad name!` comes back as
`bad name!({...});`. In XML the callback is ignored.

## Key order

JSON key order differs from XML element order and is not alphabetical; `user.getInfo` JSON starts
with `spotify_expiry_estimate`, `playcount`, `playlists`. Copy each method's order from its
fixture. `user.getInfo` also has undocumented `spotify_expiry_estimate` and `corrections`.

## Rate limit

Probe (`cargo run -p lfm-record -- probe`): 160 `user.getInfo` calls at 8 req/s on one key for
20 s, a 60 s pause, then 160 calls at 2 req/s on each of four other keys. **No limit hit at 8
req/s**: no error 29, no 429, no other error in either phase. So this does not settle per-key vs
per-IP; the pool's per-key buckets at the default 4 req/s are well inside what was observed.
The fixture run itself made about 450 calls at 2 req/s with no retries.
