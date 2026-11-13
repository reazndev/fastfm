-- What a pass-through reply needs beyond its body and status to be replayed byte for byte.

-- The Content-Type Last.fm sent; it varies by method (`application/json; charset=UTF-8`).
ALTER TABLE upstream_cache ADD COLUMN content_type TEXT NOT NULL DEFAULT '';
-- Last.fm's `access-control-*` headers, one `name: value` per line; they vary by method too.
ALTER TABLE upstream_cache ADD COLUMN headers TEXT NOT NULL DEFAULT '';
