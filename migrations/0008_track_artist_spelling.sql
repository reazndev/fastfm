-- Tracks are kept apart by the exact artist spelling they were scrobbled with. Last.fm lists
-- `João Bosco & Vinícius / Chora, me liga - Ao vivo` (2 plays) and `JOÃO BOSCO & VINÍCIUS /
-- Chora, me liga - Ao vivo` (1 play) as two tracks and counts both in `track_count`, while its
-- artist and album lists merge the two spellings. Track titles still match ignoring case: no
-- top-tracks list seen (60,000 entries over three users) has two titles that differ only in
-- case under one artist spelling.
--
-- `artist_spelling` is part of the key; `artist_name` stays what it was, the spelling when it
-- differs from the artist row's name. Existing tracks get the spelling they were first seen
-- with. Plays already merged under one row stay there until enrichment re-reads them.
CREATE TABLE tracks_v2 (
    id INTEGER PRIMARY KEY,
    artist_id INTEGER NOT NULL REFERENCES artists (id),
    name TEXT NOT NULL,
    name_fold TEXT NOT NULL,
    mbid TEXT,
    url TEXT,
    duration INTEGER,
    artist_mbid TEXT,
    artist_name TEXT,
    artist_spelling TEXT NOT NULL,
    UNIQUE (artist_id, name_fold, artist_spelling)
);
INSERT INTO tracks_v2 (id, artist_id, name, name_fold, mbid, url, duration, artist_mbid,
    artist_name, artist_spelling)
SELECT t.id, t.artist_id, t.name, t.name_fold, t.mbid, t.url, t.duration, t.artist_mbid,
    t.artist_name, coalesce(t.artist_name, ar.name)
FROM tracks t JOIN artists ar ON ar.id = t.artist_id;
DROP TABLE tracks;
ALTER TABLE tracks_v2 RENAME TO tracks;
CREATE INDEX tracks_mbid ON tracks (mbid) WHERE mbid IS NOT NULL;
