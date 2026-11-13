#!/usr/bin/env python3
"""Builds fixtures-multi/ from a live QA run: saved Last.fm answers and their classification.

    python3 -I scripts/import-qa-fixtures.py <qa dir> <database copy> <out dir>

<qa dir> holds resp/<tag>.l.<format> (Last.fm's body), resp/<tag>.meta.json (request, time,
status, headers) and classified_{json,xml}.json (the causes of each difference). Only answers
whose every cause is a mirror bug (M-*) become cases, minus the now-playing ones: an offline
server has nothing playing. Cases go in one directory per user and format, with the clock of
its first request in a `now` file. A period list whose window would gain or lose a scrobble by
using that clock instead of its own request time is left out, so every case stays exact.

The database copy may differ from what Last.fm held at request time. Its newest scrobbles may
have reached Last.fm only later (the track playing during the requests, scrobbled once it
ended): <out dir>/cutoffs.txt lists, per user, the newest scrobble Last.fm's recent tracks
showed, and later ones are removed from the copy. It may also lack scrobbles that reached
Last.fm late with an older time, which sync never fetched. Those are found by comparing
Last.fm's newest 200 scrobbles with the copy; cases whose range reaches them are left out,
and all of a user's history-wide cases when they cannot be found.
"""

import json
import pathlib
import sqlite3
import sys

NEEDS_PLAYING = {"M-np-missing", "M-np-ext-order"}
PERIOD_DAYS = {"7day": 7, "1month": 30, "3month": 90, "6month": 180, "12month": 365}
DAY = 86_400


def selected(qa):
    for name in ("classified_json.json", "classified_xml.json"):
        for tag, causes in json.loads((qa / name).read_text()).items():
            if causes and all(c.startswith("M-") for c in causes) and not set(causes) & NEEDS_PLAYING:
                yield tag, json.loads((qa / "resp" / f"{tag}.meta.json").read_text())


def scrobble_times(db, user, start, end):
    sql = (
        "SELECT s.uts FROM scrobbles s JOIN users u ON u.id = s.user_id "
        "WHERE u.name_fold = lower(?) AND s.uts >= ? AND s.uts <= ?"
    )
    return [uts for (uts,) in db.execute(sql, (user, start, end))]


def history(qa, db, user):
    """The newest scrobble Last.fm showed, and the oldest it had that the copy lacks: 0 when
    none is missing, None when some are but not among the newest 200."""
    page = json.loads((qa / "resp" / f"json.{user}.recent.l200p1.l.json").read_text())
    page = page["recenttracks"]
    times = sorted(int(t["date"]["uts"]) for t in page["track"] if "date" in t)
    newest, total = times[-1], int(page["@attr"]["total"])
    held = len(scrobble_times(db, user, 0, newest))
    if held == total:
        return newest, 0
    stored = scrobble_times(db, user, times[0], newest)
    missing = [uts for uts in times if uts not in stored or stored.remove(uts)]
    found = len(missing) == total - held
    return newest, (min(missing) if found else None)


def range_end(params, at):
    """Where a case's answer stops depending on scrobbles: `to`, or the request time."""
    if params["method"] == "user.getLovedTracks":
        return 0
    return int(params.get("to", at))


def gap_has_plays(db, user, params, start, at):
    days = PERIOD_DAYS.get(params.get("period", ""))
    if days is None or at == start:
        return False
    sql = (
        "SELECT count(*) FROM scrobbles s JOIN users u ON u.id = s.user_id "
        "WHERE u.name_fold = lower(?) AND s.uts >= ? AND s.uts < ?"
    )
    (count,) = db.execute(sql, (user, start - days * DAY, at - days * DAY)).fetchone()
    return count > 0


def write_case(group, tag, meta, fmt, body):
    params = dict(meta["params"])
    method = params["method"]
    variant = tag.split(".", 1)[1]
    pairs = [[k, v] for k, v in params.items()]
    pairs.insert(1, ["api_key", "REDACTED"])
    headers = dict(meta["lh"], **{"content-type": meta["l"]["ctype"]})
    out = group / method
    out.mkdir(parents=True, exist_ok=True)
    (out / f"{variant}.{fmt}").write_bytes(body)
    case = {
        "method": method,
        "variant": variant,
        "http_method": "GET",
        "recorded_at": meta["time"],
        "responses": {
            fmt: {
                "params": pairs,
                "status": meta["l"]["status"],
                "content_type": meta["l"]["ctype"],
                "headers": headers,
            }
        },
    }
    (out / f"{variant}.meta.json").write_text(json.dumps(case, indent=1) + "\n")


def main(qa, db_path, out):
    db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    groups = {}
    for tag, meta in selected(qa):
        fmt, user = tag.split(".")[:2]
        groups.setdefault((user, fmt), []).append((tag, meta))
    histories = {user: history(qa, db, user) for user, _ in groups}
    kept, dropped = 0, []
    for (user, fmt), cases in sorted(groups.items()):
        start = min(meta["time"] for _, meta in cases)
        newest, missing_from = histories[user]
        group = out / f"{user}.{fmt}"
        for tag, meta in cases:
            end = range_end(meta["params"], meta["time"])
            incomplete = end > 0 and (missing_from is None or 0 < missing_from < end)
            if incomplete or gap_has_plays(db, user, meta["params"], start, meta["time"]):
                dropped.append(tag)
                continue
            body = (qa / "resp" / f"{tag}.l.{fmt}").read_bytes()
            write_case(group, tag, meta, fmt, body)
            (group / "now").write_text(f"{start}\n")
            kept += 1
    lines = "".join(f"{user} {newest}\n" for user, (newest, _) in sorted(histories.items()))
    (out / "cutoffs.txt").write_text(lines)
    written = len(list(out.glob("*/now")))
    print(f"{kept} cases in {written} groups; left out: {len(dropped)}: {dropped}")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(pathlib.Path(sys.argv[1]), sys.argv[2], pathlib.Path(sys.argv[3]))
