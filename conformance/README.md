# Conformance

`lfm-conformance` replays every recorded fixture (`fixtures/`, 210 requests in XML and JSON, so
420 cases) against a server and compares the answers with what Last.fm sent.

## Run it

Against our server, with the imported history and the clock frozen at recording time:

```sh
scripts/conformance.sh                                  # every case
scripts/conformance.sh --only user.getInfo --format xml # narrow it down
scripts/conformance.sh --expect conformance/expected.txt        # CI: fail only on regressions
scripts/conformance.sh --write-expect conformance/expected.txt  # update the baseline
```

The script needs `data/conformance.db`. It serves a copy of it, so the import stays untouched.
The JSON report goes to `target/conformance-report.json` (set `CONFORMANCE_REPORT` to change it).

Against any server:

```sh
LFM_CONFORMANCE_API_KEY=<key> cargo run -p lfm-conformance -- run \
  --base-url http://127.0.0.1:3055/2.0/ [--only <id prefix>]... [--format xml|json] \
  [--report <path>] [--ignore <path>] [--expect <file>] [--write-expect <file>] \
  [--concurrency 8] [--rps <n>]
```

`--api-key` works too, but the variable keeps the key out of `ps` and shell history. The key
replaces `REDACTED` in the recorded parameters; the invalid-key and missing-key cases are sent
as recorded. The key is scrubbed from answers before comparing, so it never reaches a report.
`--only` matches the start of a case id, `method/variant.format`, and can be repeated.

Exit codes: 0 pass, 1 failures (or, with `--expect`, regressions), 2 the run could not happen.

## Reading a report

Each case is `exact` (status, content type, CORS headers and body bytes match), `ignored` (every
difference is covered by an ignore rule) or `failed`. Scores are per method and overall. Counts
are complete; each case lists at most 5 differences per category and 40 in all.

Differences, in comparison order:

| Category | Meaning |
|---|---|
| `transport` | No answer: connection refused, timeout |
| `status` | HTTP status |
| `content-type` | `Content-Type` header |
| `header` | An `access-control-*` header missing, extra or different |
| `parse` | The answer is not valid JSON or XML although the fixture is |
| `wrapper` | JSONP callback name, or a missing wrapper |
| `declaration` | XML declaration text (`?>` vs ` ?>`) |
| `missing`, `extra` | Key, element or attribute on one side only |
| `shape` | Array vs object vs scalar, or text vs child elements |
| `type` | JSON string vs number vs boolean vs null |
| `order` | Key, element or attribute order |
| `value` | Different value or text |
| `escape` | Same value, different escaping (`\/` vs `/`, `&apos;` vs `'`) |
| `whitespace` | Indentation, line breaks, `<x/>` vs `<x></x>`, trailing newline |
| `bytes` | Bodies differ and no category above explains it (plain-text bodies) |

Paths follow the JSON names in both formats: `recenttracks.track[3].image[2].#text`. In XML the
`lfm` root is left out, attributes are `.@name`, an element's text is `.#text` when it has
attributes and the bare path otherwise, and only repeated elements get an index. `$` is the
document itself.

## Ignore rules

`conformance/ignore.toml` lists differences that legitimately vary. Each rule has a method glob,
a path glob (`*` stops at `.`, `**` does not), optional `variant`, `format`, `categories`
(default `["value"]`) and `values` (both sides must be one of them), and a required `reason`.
Ignored differences are counted and listed apart from failures. A rule that applies to cases in
the run but matched nothing is reported as stale, with how many of those cases failed for other
reasons (a failing method can hide what the rule is for).

## What drifts on Last.fm's side

A live run on 2026-11-12, a few hours after recording (131 JSON requests: every `user.*`
fixture and a sample of catalogue methods, then the drifting ones again in both formats, at
2 requests/s), changed only these fields:

- Global counters: `artist.getInfo` `stats.listeners` and `stats.playcount`, and every
  `listeners` and `playcount` in `chart.getTopArtists`.
- `auth.getToken` returns a new token on every call.
- MBIDs that flap: one album answers with either of two MBIDs, one track with its MBID or none,
  and the choice changes between calls (JSON and XML one second apart disagreed). This showed in
  recent tracks, top albums and tracks, and weekly album and track charts.

Everything else in `user.*` was byte-identical, including recent tracks, period charts and
`user.getInfo`, because the account had not scrobbled since recording. Other catalogue
counters (`track.getInfo`, `album.getInfo`, `geo.*`, `tag.*`) did not move in this sample, but
are the same kind of value and may need rules once they do.

The harness fetches every catalogue method from Last.fm while it runs (`data/conformance.db`
holds no cached answers), so catalogue cases compare a live answer with one recorded earlier.

## Ignore rules by cause (2026-11-13)

About 110 cases pass only with ignores. The rules in `ignore.toml` fall into four groups:

| Group | Rules | Goes away with a fresh recording and import? |
|---|---|---|
| Live counters: listeners, play counts, reach, taggings and totals in `artist.getInfo`, `album.getInfo`, `track.getInfo`, `tag.*`, `chart.*`, `geo.*`, `artist.getTopAlbums`, `artist.getTopTracks`, the search methods, and the play count of a user this instance does not mirror | 30 | No. They move within hours, and the run asks Last.fm live. Some are stale right after a re-recording and come back within a day. |
| `auth.getToken` token | 1 | No: a new random token on every call. |
| MBIDs that flap between two values from call to call | 2 | No: Last.fm answers either value. |
| Album count 6,198 against 6,187 | 2 | No: 11 albums Last.fm files under an album artist that no scrobble shows. |
| Catalogue values changed between recording and the import behind `data/conformance.db` (one cover, five MBIDs, two durations) | 8 | Yes: a recording and an import made the same day would agree. |

The cases that still fail are lists whose order changes on Last.fm between two calls seconds
apart: `chart.getTopArtists` and `chart.getTopTracks` (hourly charts; the JSON and XML of one
recording already disagree), `geo.getTopArtists` (the default and lowercase variants, recorded
seconds apart, disagree), and the `artist.search` and `track.search` results with close
listener counts. No rule can cover them without ignoring every name in the list, and a
re-recording goes stale within the hour, so they stay as failures.

## The multi-user suite

`fixtures` holds one account recorded at one moment, so it missed bugs that only other histories
show (a live comparison of 533 answers for ten users on 2026-11-13 found 49% byte-identical
while this suite scored 90%). `fixtures-multi/` keeps the answers from that comparison whose
every difference was a mirror bug, replayed offline against a copy of the database the
comparison ran on:

```sh
scripts/conformance-multi.sh prepare      # once: builds data/conformance-multi.db
scripts/conformance-multi.sh              # 137 cases in 12 groups
scripts/conformance-multi.sh --only user.getTopAlbums --expect conformance/expected-multi.txt
```

Each `fixtures-multi/<user>.<format>/` group runs against its own server start with `LFM_NOW`
at the group's first request (its `now` file), in clock order on one copy of the database.
Reports go to `target/conformance-multi/<group>.json`, and the script prints a total. Ignore
rules come from `conformance/ignore-multi.toml`, which is empty: none of these differences is
expected to stay.

**The database.** `prepare` copies `lfm.db` out of the Docker volume `lfm-load-data` (set
`LFM_MULTI_VOLUME` for another) with a throwaway `alpine` container, without starting anything
that uses the volume, then removes each user's scrobbles newer than the newest one Last.fm
showed during the comparison (`fixtures-multi/cutoffs.txt`; the track playing at the time was
scrobbled afterwards). `data/` is not committed; without the volume the suite cannot run.

**What was left out.** `scripts/import-qa-fixtures.py` built the cases from the saved
comparison (`resp/` and `classified_*.json`) and the database copy. Answers that need a track
playing (the now-playing bugs) are not taken: an offline server has none. Of the other 174
answers whose causes were all mirror bugs it dropped 37:

- period lists whose window would gain or lose a scrobble between the group's clock and the
  request's own time (2);
- answers whose range reaches scrobbles Last.fm had and the copy lacks, because they reached
  Last.fm late with an older time and sync never fetched them: every history-wide answer of
  `amhkk`, `elijah0705` and `syntiiix` (35).

To rebuild the cases from a new comparison, run
`python3 -I scripts/import-qa-fixtures.py <qa dir> <database copy> fixtures-multi`. No API key
is stored: requests carry `REDACTED`, which the harness replaces.
