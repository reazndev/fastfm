//! Opening connections, pragmas, and migrations.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::error::Result;

/// Numbered migrations, applied in order. `PRAGMA user_version` holds how many have run.
const MIGRATIONS: &[&str] = &[
    include_str!("../../../migrations/0001_init.sql"),
    include_str!("../../../migrations/0002_details.sql"),
    include_str!("../../../migrations/0003_track_artist_mbid.sql"),
];

/// Prepared statements kept per connection. The store uses fewer than this.
const STATEMENT_CACHE: usize = 256;

/// Opens the writer connection, switches to WAL, and applies pending migrations.
pub(crate) fn open_writer(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
    configure(&conn)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Opens a read-only connection to a database the writer has already set up.
pub(crate) fn open_reader(path: &Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)?;
    configure(&conn)?;
    conn.pragma_update(None, "query_only", true)?;
    Ok(conn)
}

/// Opens a private in-memory database with the full schema.
pub(crate) fn open_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    // 64 MiB page cache per connection; mmap lets readers share the OS page cache.
    conn.pragma_update(None, "cache_size", -65_536)?;
    // In-memory databases answer this pragma with no row.
    conn.pragma_update_and_check(None, "mmap_size", 1_i64 << 34, |row| row.get::<_, i64>(0))
        .optional()?;
    Ok(())
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let applied: usize = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index + 1)?;
        tx.commit()?;
        tracing::info!(migration = index + 1, "applied database migration");
    }
    Ok(())
}
