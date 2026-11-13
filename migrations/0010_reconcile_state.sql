-- When reconciliation last checked each user, so a restart does not postpone the next pass.
-- NULL until the user is first followed after this migration.
ALTER TABLE users ADD COLUMN reconciled_at INTEGER;

-- Ranges where Last.fm's own counts disagree: `remote` is the range's `@attr.total`, `local`
-- the store's count, which equals what Last.fm lists in the range or the sum of its halves'
-- totals. Reconciliation leaves such a range alone for a week instead of reading it again
-- on every pass.
CREATE TABLE upstream_mismatches (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    from_uts INTEGER NOT NULL,
    to_uts INTEGER NOT NULL,
    remote INTEGER NOT NULL,
    local INTEGER NOT NULL,
    seen_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, from_uts, to_uts)
) WITHOUT ROWID;
