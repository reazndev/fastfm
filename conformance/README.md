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
  --base-url http://127.0.0.1:8080/2.0/ [--only <id prefix>]... [--format xml|json] \
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
