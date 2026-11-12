-- Fields Last.fm shows that the first schema did not keep.

-- Track length in seconds, shown as `duration` in user.getTopTracks. NULL until known.
ALTER TABLE tracks ADD COLUMN duration INTEGER;

-- user.getInfo `corrections` and `spotify_expiry_estimate` (unix seconds, 0 when unknown).
ALTER TABLE users ADD COLUMN corrections INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN spotify_expiry INTEGER NOT NULL DEFAULT 0;
