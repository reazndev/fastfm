-- Last.fm's library: the entities its top lists, library and weekly charts count plays under,
-- as read from the user's overall top lists by the enrichment job. Scrobbles keep the entities
-- they were scrobbled with; the aggregate tables count each scrobble under its library entity.

-- How the library spells the artist when it differs from the stored name only in case.
ALTER TABLE artists ADD COLUMN lib_name TEXT;

-- The artist spelling scrobbled with this track when it differs from the artist row's name
-- (`twenty one pilots` next to `Twenty One Pilots`). Last.fm shows it with the track.
ALTER TABLE tracks ADD COLUMN artist_name TEXT;

-- Albums get a variant. A scrobbled album is keyed by its exact spelling and cover as well as
-- its folded name, because Last.fm files one folded name under different album entities and
-- those differ in spelling or cover (`FIRST FINALE` and `First Finale` by the same track artist
-- belong to two album artists). Library albums, created by enrichment, have the variant ''.
-- They carry what top lists show: the URL, the artist MBID shown next to the album (often empty
-- while the artist has one), and the cover in `image` ('' for none).
CREATE TABLE albums_v2 (
    id INTEGER PRIMARY KEY,
    artist_id INTEGER NOT NULL REFERENCES artists (id),
    name TEXT NOT NULL,
    name_fold TEXT NOT NULL,
    mbid TEXT,
    image TEXT,
    variant TEXT NOT NULL,
    url TEXT,
    artist_mbid TEXT,
    UNIQUE (artist_id, name_fold, variant)
);
INSERT INTO albums_v2 (id, artist_id, name, name_fold, mbid, image, variant)
SELECT id, artist_id, name, name_fold, mbid, image, name || char(31) || coalesce(image, '')
FROM albums;
DROP TABLE albums;
ALTER TABLE albums_v2 RENAME TO albums;
CREATE INDEX albums_mbid ON albums (mbid) WHERE mbid IS NOT NULL;

-- Scrobbled entities that Last.fm counts under another entity: corrected artist names
-- (`Berlioz` under `Hector Berlioz`), scrobbled albums under their library album (often filed
-- under the album artist, `Various Artists`), and the odd corrected track. kind is 0 artist,
-- 1 album, 2 track. No row means the entity counts as itself.
CREATE TABLE user_library_map (
    user_id INTEGER NOT NULL,
    kind INTEGER NOT NULL,
    raw_id INTEGER NOT NULL,
    lib_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, kind, raw_id)
) WITHOUT ROWID;

-- Loved tracks that share a second keep Last.fm's order: a higher seq comes first.
ALTER TABLE loved ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;
CREATE INDEX loved_order ON loved (user_id, uts DESC, seq DESC, track_id);
DROP INDEX loved_uts;
