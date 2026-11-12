-- Initial schema. See PLAN.md section 5.
--
-- Hot tables (scrobbles and the per-user aggregates) carry no foreign keys: the store writes them
-- only through code that resolves every id first, and a foreign key check per row would double
-- the cost of an import.

CREATE TABLE artists (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    name_fold TEXT NOT NULL UNIQUE,
    mbid TEXT,
    url TEXT
);
CREATE INDEX artists_mbid ON artists (mbid) WHERE mbid IS NOT NULL;

CREATE TABLE albums (
    id INTEGER PRIMARY KEY,
    artist_id INTEGER NOT NULL REFERENCES artists (id),
    name TEXT NOT NULL,
    name_fold TEXT NOT NULL,
    mbid TEXT,
    image TEXT,
    UNIQUE (artist_id, name_fold)
);
CREATE INDEX albums_mbid ON albums (mbid) WHERE mbid IS NOT NULL;

CREATE TABLE tracks (
    id INTEGER PRIMARY KEY,
    artist_id INTEGER NOT NULL REFERENCES artists (id),
    name TEXT NOT NULL,
    name_fold TEXT NOT NULL,
    mbid TEXT,
    url TEXT,
    UNIQUE (artist_id, name_fold)
);
CREATE INDEX tracks_mbid ON tracks (mbid) WHERE mbid IS NOT NULL;

-- Mirrored Last.fm users. The counters are kept equal to the aggregates by every ingest.
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    name_fold TEXT NOT NULL UNIQUE,
    realname TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL DEFAULT '',
    country TEXT NOT NULL DEFAULT '',
    gender TEXT NOT NULL DEFAULT '',
    age INTEGER NOT NULL DEFAULT 0,
    subscriber INTEGER NOT NULL DEFAULT 0,
    playlists INTEGER NOT NULL DEFAULT 0,
    bootstrap INTEGER NOT NULL DEFAULT 0,
    registered_uts INTEGER NOT NULL DEFAULT 0,
    image TEXT,
    user_type TEXT NOT NULL DEFAULT 'user',
    sync_cursor INTEGER NOT NULL DEFAULT 0,
    -- UTC day number the period totals end on; NULL until they are first built.
    period_day INTEGER,
    version INTEGER NOT NULL DEFAULT 0,
    playcount INTEGER NOT NULL DEFAULT 0,
    artist_count INTEGER NOT NULL DEFAULT 0,
    album_count INTEGER NOT NULL DEFAULT 0,
    track_count INTEGER NOT NULL DEFAULT 0
);

-- seq numbers scrobbles within one second, so real same-second duplicates survive.
CREATE TABLE scrobbles (
    user_id INTEGER NOT NULL,
    uts INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    track_id INTEGER NOT NULL,
    album_id INTEGER,
    artist_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, uts, seq)
) WITHOUT ROWID;
CREATE INDEX scrobbles_artist ON scrobbles (user_id, artist_id, uts);
CREATE INDEX scrobbles_track ON scrobbles (user_id, track_id, uts);

-- Per-user totals for each period: 0 is overall, 1 to 5 are the last 7, 30, 90, 180, and 365
-- UTC days up to users.period_day. The plays index ends in the entity id so ties come out in a
-- fixed order.
CREATE TABLE user_artist_total (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    artist_id INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, artist_id)
) WITHOUT ROWID;
CREATE INDEX user_artist_total_plays ON user_artist_total (user_id, period, plays DESC, artist_id);

CREATE TABLE user_album_total (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    album_id INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, album_id)
) WITHOUT ROWID;
CREATE INDEX user_album_total_plays ON user_album_total (user_id, period, plays DESC, album_id);

CREATE TABLE user_track_total (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    track_id INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, track_id)
) WITHOUT ROWID;
CREATE INDEX user_track_total_plays ON user_track_total (user_id, period, plays DESC, track_id);

-- How many entities have each play count, per period. Paging a top list walks this instead of
-- an OFFSET.
CREATE TABLE user_artist_hist (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, plays)
) WITHOUT ROWID;

CREATE TABLE user_album_hist (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, plays)
) WITHOUT ROWID;

CREATE TABLE user_track_hist (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, plays)
) WITHOUT ROWID;

-- The same counts split by id block (id / 4096), so a page that starts deep inside a large
-- group of equal play counts seeks to the right block instead of skipping the whole group.
CREATE TABLE user_artist_tie (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    block INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, plays, block)
) WITHOUT ROWID;

CREATE TABLE user_album_tie (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    block INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, plays, block)
) WITHOUT ROWID;

CREATE TABLE user_track_tie (
    user_id INTEGER NOT NULL,
    period INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    block INTEGER NOT NULL,
    n INTEGER NOT NULL,
    PRIMARY KEY (user_id, period, plays, block)
) WITHOUT ROWID;

-- Time rollups. lvl picks the bucket width (1, 8 or 64 UTC days, see buckets.rs) and bucket is
-- the bucket number since the epoch at that width.
CREATE TABLE user_artist_rollup (
    user_id INTEGER NOT NULL,
    lvl INTEGER NOT NULL,
    bucket INTEGER NOT NULL,
    artist_id INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    PRIMARY KEY (user_id, lvl, bucket, artist_id)
) WITHOUT ROWID;

CREATE TABLE user_album_rollup (
    user_id INTEGER NOT NULL,
    lvl INTEGER NOT NULL,
    bucket INTEGER NOT NULL,
    album_id INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    PRIMARY KEY (user_id, lvl, bucket, album_id)
) WITHOUT ROWID;

CREATE TABLE user_track_rollup (
    user_id INTEGER NOT NULL,
    lvl INTEGER NOT NULL,
    bucket INTEGER NOT NULL,
    track_id INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    PRIMARY KEY (user_id, lvl, bucket, track_id)
) WITHOUT ROWID;

-- Scrobble counts per bucket, used to count ranges and to seek deep pages of history.
CREATE TABLE user_scrobble_rollup (
    user_id INTEGER NOT NULL,
    lvl INTEGER NOT NULL,
    bucket INTEGER NOT NULL,
    plays INTEGER NOT NULL,
    PRIMARY KEY (user_id, lvl, bucket)
) WITHOUT ROWID;

CREATE TABLE loved (
    user_id INTEGER NOT NULL REFERENCES users (id),
    track_id INTEGER NOT NULL REFERENCES tracks (id),
    uts INTEGER NOT NULL,
    PRIMARY KEY (user_id, track_id)
) WITHOUT ROWID;
CREATE INDEX loved_uts ON loved (user_id, uts DESC, track_id);

CREATE TABLE weekly_chart_list (
    user_id INTEGER NOT NULL REFERENCES users (id),
    from_uts INTEGER NOT NULL,
    to_uts INTEGER NOT NULL,
    PRIMARY KEY (user_id, from_uts)
) WITHOUT ROWID;

CREATE TABLE upstream_cache (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL,
    format TEXT NOT NULL,
    body BLOB NOT NULL,
    status INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    UNIQUE (key, format)
);
CREATE INDEX upstream_cache_expires ON upstream_cache (expires_at);

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    name_fold TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'user')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'active', 'disabled')),
    lastfm_user TEXT,
    own_key BLOB,
    own_secret BLOB,
    lastfm_sk BLOB,
    created_at INTEGER NOT NULL
);
CREATE INDEX accounts_status ON accounts (status);

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE api_keys (
    key TEXT PRIMARY KEY,
    secret TEXT NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('read', 'write')),
    created_at INTEGER NOT NULL,
    last_used INTEGER
);
CREATE INDEX api_keys_account ON api_keys (account_id);

-- Web login sessions.
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_account ON sessions (account_id);
CREATE INDEX sessions_expires ON sessions (expires_at);

-- Last.fm-style auth: auth.getToken creates a token, the user approves it on /api/auth, and
-- auth.getSession trades it for a session key.
CREATE TABLE auth_tokens (
    token TEXT PRIMARY KEY,
    api_key TEXT NOT NULL,
    account_id INTEGER REFERENCES accounts (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX auth_tokens_account ON auth_tokens (account_id);
CREATE INDEX auth_tokens_expires ON auth_tokens (expires_at);

CREATE TABLE api_sessions (
    session_key TEXT PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    api_key TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX api_sessions_account ON api_sessions (account_id);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    user_id INTEGER REFERENCES users (id) ON DELETE CASCADE,
    cursor TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'done', 'failed')),
    error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX jobs_status ON jobs (status);
CREATE INDEX jobs_user ON jobs (user_id);
