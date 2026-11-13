//! Opening connections, pragmas, and migrations.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::error::{Result, StoreError};

/// Numbered migrations, applied in order. `PRAGMA user_version` holds how many have run.
const MIGRATIONS: &[&str] = &[
    include_str!("../../../migrations/0001_init.sql"),
    include_str!("../../../migrations/0002_details.sql"),
    include_str!("../../../migrations/0003_track_artist_mbid.sql"),
    include_str!("../../../migrations/0004_upstream_cache_headers.sql"),
    include_str!("../../../migrations/0005_write_through.sql"),
    include_str!("../../../migrations/0006_library.sql"),
    include_str!("../../../migrations/0007_unique_lastfm_link.sql"),
    include_str!("../../../migrations/0008_track_artist_spelling.sql"),
    include_str!("../../../migrations/0009_album_artist_spelling.sql"),
    include_str!("../../../migrations/0010_reconcile_state.sql"),
];

/// Prepared statements kept per connection. The store uses fewer than this.
const STATEMENT_CACHE: usize = 256;

/// Memory settings of every connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning {
    /// SQLite's own page cache per connection, in MiB.
    pub cache_mib: u32,
    /// How much of the file each connection maps, in MiB. Mapped pages are the OS page cache,
    /// shared by all connections, but each connection's mapping counts towards the process's
    /// resident size again.
    pub mmap_mib: u32,
}

impl Default for Tuning {
    /// 8 MiB of cache and no mapping. Measured with 15 users (1.78M scrobbles, 320 MB) under
    /// 50 clients paging through history: 210 MB resident against 1.9 GB with a 64 MiB cache
    /// and a 16 GiB mapping per connection, at the same throughput, since the OS page cache
    /// serves the reads either way.
    fn default() -> Self {
        Self {
            cache_mib: 8,
            mmap_mib: 0,
        }
    }
}

/// Opens the writer connection, switches to WAL, and applies pending migrations.
pub(crate) fn open_writer(path: &Path, tuning: Tuning) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
    configure(&conn, tuning)?;
    migrate(&mut conn)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(conn)
}

/// Opens a read-only connection to a database the writer has already set up.
pub(crate) fn open_reader(path: &Path, tuning: Tuning) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags)?;
    configure(&conn, tuning)?;
    conn.pragma_update(None, "query_only", true)?;
    Ok(conn)
}

/// Opens a private in-memory database with the full schema.
pub(crate) fn open_memory() -> Result<Connection> {
    let mut conn = Connection::open_in_memory()?;
    configure(&conn, Tuning::default())?;
    migrate(&mut conn)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(conn)
}

fn configure(conn: &Connection, tuning: Tuning) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE);
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    // A negative cache size is in KiB.
    conn.pragma_update(None, "cache_size", -i64::from(tuning.cache_mib) * 1024)?;
    let mmap = i64::from(tuning.mmap_mib) << 20;
    // In-memory databases answer this pragma with no row.
    conn.pragma_update_and_check(None, "mmap_size", mmap, |row| row.get::<_, i64>(0))
        .optional()?;
    Ok(())
}

/// Runs with foreign keys off, as SQLite's procedure for rebuilding a table requires, and
/// checks them before each commit instead. The bundled SQLite turns them on by default.
fn migrate(conn: &mut Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", false)?;
    let applied: usize = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        let broken: Option<String> = tx
            .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
            .optional()?;
        if let Some(table) = broken {
            return Err(StoreError::InvalidValue(format!(
                "migration {} broke a foreign key in {table}",
                index + 1
            )));
        }
        tx.pragma_update(None, "user_version", index + 1)?;
        tx.commit()?;
        tracing::info!(migration = index + 1, "applied database migration");
    }
    Ok(())
}
