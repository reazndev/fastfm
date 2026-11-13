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
- Now playing: not in these fixtures (nothing was playing), but seen live on 2026-11-13: an
  extra first item beyond `limit` with `"@attr":{"nowplaying":"true"}` and no `date`, on
  **every** page and in every `from`/`to` range, even one that ended the day before. JSON keys:
  `artist,streamable,image,mbid,album,name,@attr,url`, with `extended=1`
  `artist,mbid,name,image,streamable,album,url,@attr,loved`.
- A scrobble without an album has `""` for all four images (128 of 128 seen live); the
  placeholder only appears for an album without a cover.
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

The JSON `@attr` of a paged user list (`user`, `page`, `perPage`, `totalPages`, `total`) comes in
one of four orders, chosen by the number of items in the list (now-playing included), whatever
the method, `limit`, page or range. XML keeps one order. Measured on 2026-11-13 with
`user.getRecentTracks` `limit` 1 to 200 and 41 values up to 1000, and consistent with 450 other
list answers of ten users (repeated requests give the same order):

| Items | Order |
|---|---|
| 0-2, 6-7, 9, 12-19, 36-67, 260-515 | `user,totalPages,page,perPage,total` |
| 3-5, 8, 20-35, 68-131, 516-1000 | `user,totalPages,page,total,perPage` |
| 132-258 | `perPage,totalPages,page,user,total` |
| 10-11, 259 | `perPage,totalPages,page,total,user` |

Past 200 the ranges are sampled (201, 230, 240, 250, 254-263, 270, 280, 300, ... 1000), not
every value.

An album's or track's `artist.url` is the leading part of the item's own `url`, `+noredirect/`
included when the item URL has it (one exception in about 7,800 items), never the URL the artist
has in artist lists.

JSON key order differs from XML element order and is not alphabetical; `user.getInfo` JSON starts
with `spotify_expiry_estimate`, `playcount`, `playlists`. Copy each method's order from its
fixture. `user.getInfo` also has undocumented `spotify_expiry_estimate` and `corrections`.
Last.fm leaves `spotify_expiry_estimate` out for some users, and the other keys then come in
another order (`playcount,playlists,album_count,bootstrap,url,country,name,artist_count,age,
track_count,subscriber,image,registered,realname,gender,corrections,type`).

## Rate limit

Probe (`cargo run -p lfm-record -- probe`): 160 `user.getInfo` calls at 8 req/s on one key for
20 s, a 60 s pause, then 160 calls at 2 req/s on each of four other keys. **No limit hit at 8
req/s**: no error 29, no 429, no other error in either phase. So this does not settle per-key vs
per-IP; the pool's per-key buckets at the default 4 req/s are well inside what was observed.
The fixture run itself made about 450 calls at 2 req/s with no retries.

## Import (lfm-sync, 2026-11-12)

Full import of the same user with `limit=1000&extended=1`, 8 parallel ranges, 8 req/s shared
across the key pool: 160,643 scrobbles in 54 s and 179 requests, `--until 1794518863`.

- **Same-second scrobbles are common.** 1,458 scrobbles share their second with another one, and
  one second holds 11. Any window or page boundary must keep a second whole.
- **`limit=1&page=n` is an exact quantile.** Page 140,000 with `limit=1` and `to=` answers in
  about half a second, so splitting a history into equal parts costs one request per part.
- **Error 8 ("Operation failed") happens on big pages.** It came once in about 180
  `limit=1000` calls and passed on the first retry.
- **Range counts are slow on long ranges.** A `limit=1` call with `from`/`to` spanning months
  takes 0.3 to 0.7 s, so the 33 reconcile comparisons take 12 to 22 s.
- **Last.fm's library counts are not distinct names.** `user.getInfo` says 4,046 artists and
  6,198 albums; the same scrobbles hold 4,052 case-folded artist names and 6,348 albums keyed by
  track artist and name. Track counts agree (15,477).
- The scrobble count for `to=1794518863` (160,643) equals `playcount` in the `user.getInfo`
  fixture, and no request hit a rate limit at 8 req/s with eight requests in flight.

## Pass-through (lfm-api, 2026-11-12)

Replaying the catalogue fixtures through the mirror's read-through cache, about 90 minutes after
recording:

- **`getInfo` with `username` is not the global answer plus two fields.** `artist.getInfo`
  shows the artist's real image with `username` and the placeholder without it, and
  `album.getInfo` orders its JSON keys differently (`name`, `userplaycount`, `image`, `tracks`,
  `listeners`, `playcount`, `url` against `playcount`, `image`, `tracks`, `url`, `name`,
  `listeners`). So the mirror forwards these requests with the user name and replaces only the
  `userplaycount` and `userloved` values with local ones. `userplaycount` is a string in artist
  and track JSON and a number in album JSON; `userloved` is a string.
- **Lookups ignore case and do not echo it.** `artist`, `album`, `track` (getInfo, similar, top
  lists, tags, corrections) and `country` (geo) bodies are identical whatever the request's case,
  so the cache key folds them. Search terms are echoed in `opensearch:Query`, so they are not.
- **Drift within 90 minutes.** Besides the counters already listed: `chart.getTopTags` and
  `tag.getTopTags` reach, taggings and the total tag count, `tag.getInfo` `total`,
  `chart.getTopTracks` play and listener counts (in one run and not the next, as the hourly chart
  turned over), a rank swap in `chart.getTopArtists` and `chart.getTopTracks`, and result order in
  `album.search` and `track.search`, where tracks with close listener counts swap places between
  calls.
- With several keys in the pool, the 186 uncached requests of one run took about 9 s, with no
  error 29.

## Library entities (lfm-sync enrichment, 2026-11-13)

Last.fm's top lists, `library.getArtists`, weekly charts and `user.getInfo` counts do not count
plays under the names in the scrobbles. Compared with the user's all-time top lists
(`period=overall&limit=1000`: 5 + 7 + 16 requests here):

- **Artists are corrected.** 37 scrobbled names count under the name `artist.getCorrection`
  gives: `zzz.` under `ZZZ`, `Travis Scott` under `Travi$ Scott`, `Berlioz` under
  `Hector Berlioz`, `STRFKR` under `Starfucker`, `Ye` under `Yes`, `?` and `06` under
  `[unknown]`. With those, all 4,046 artists and play counts match.
- **Tracks are not.** Top tracks keep the scrobbled artist (`zzz.`), and all 15,477 match by name
  but one (`Udo Lindenberg` / `Horizont (feat. Nena)` is listed under
  `Udo Lindenberg & das Panik-Orchester`; `track.getCorrection` does not say so).
- **Albums are filed under the album artist**, which no API response shows per scrobble. The
  scrobble's exact album spelling and cover tell most of them apart: `FIRST FINALE` (one cover)
  and `First Finale` (another) by the same track artist are two albums by two album artists, and
  the cover shown in recent tracks is that album's. Matching by spelling and cover, then artist,
  then remaining play count places all but 62 of 160,643 scrobbles; 11 albums stay empty.
- **`extended=1` blanks every artist MBID** in recent tracks. Plain pages show an artist MBID per
  track (empty for some tracks of artists that have one); top tracks, weekly track charts and
  loved tracks show the same per-track value. Top albums and weekly album charts show a
  per-album artist MBID. Plain pages lack the artist URL, which is the track URL's start.
- **Ties sort by a Unicode collation**: punctuation, then symbols and emoji, then currency (`$`,
  `¥`), digits, letters. `ø` sorts as `o`, `Ł` as `l`, small capitals (`ʟᴏꜱᴛ`) right after
  their letter, and `🥵` (Unicode 11) after everything. In top tracks the 1-play group is not
  sorted by name at all (it looks like internal id order); no fixture reaches it.
- **Loved tracks in the same second** keep the order of `user.getLovedTracks`.
- **Catalogue values drift**: one day after recording, two track MBIDs and lengths and one album
  cover had changed on Last.fm; `conformance/ignore.toml` names them.
