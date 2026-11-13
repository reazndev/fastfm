-- One account per Last.fm user, enforced by the database so two links cannot race.
-- Last.fm usernames are ASCII, so NOCASE matches the store's case folding for them.
-- Earlier versions checked this in code only; keep the oldest claim of any duplicate.
UPDATE accounts SET lastfm_user = NULL
WHERE lastfm_user IS NOT NULL
  AND id <> (SELECT MIN(other.id) FROM accounts other
             WHERE other.lastfm_user = accounts.lastfm_user COLLATE NOCASE);

CREATE UNIQUE INDEX accounts_lastfm_user ON accounts (lastfm_user COLLATE NOCASE)
WHERE lastfm_user IS NOT NULL;
