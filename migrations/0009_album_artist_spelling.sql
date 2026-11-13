-- The album artist as the top-albums list spells it for this library album. Last.fm shows the
-- spelling attached to the album, which can differ from the artist list's: The_Beast_Playz's
-- weekly album chart shows `Jaÿ-Z` where the artist lists say `JAŸ-Z`, and elijah0705's chart
-- shows both spellings on different albums. NULL for scrobbled album variants and for library
-- albums not listed since this column was added.
ALTER TABLE albums ADD COLUMN artist_name TEXT;
