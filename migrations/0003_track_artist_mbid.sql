-- The artist MBID Last.fm shows next to a track in recent and loved tracks and track charts.
-- It is set per track and can be empty while the artist has one. NULL means not seen.
ALTER TABLE tracks ADD COLUMN artist_mbid TEXT;
