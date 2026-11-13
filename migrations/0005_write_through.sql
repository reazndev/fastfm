-- Writes from clients on their way to Last.fm.
--
-- `queued` rows wait for a retry after a transient failure, `failed` rows gave up, and `held`
-- rows were applied locally only because the account has no Last.fm session key. The payload
-- is JSON written by the caller.
CREATE TABLE outbox (
    id INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    method TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued', 'failed', 'held')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_at INTEGER NOT NULL,
    error TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX outbox_due ON outbox (status, next_at);
CREATE INDEX outbox_account ON outbox (account_id, id);

-- Session keys and pending tokens die with the API key they belong to, so revoking a key
-- logs out every client that used it.
CREATE TRIGGER api_keys_revoked AFTER DELETE ON api_keys BEGIN
    DELETE FROM api_sessions WHERE api_key = OLD.key;
    DELETE FROM auth_tokens WHERE api_key = OLD.key;
END;
