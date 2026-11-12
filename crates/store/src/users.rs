//! Mirrored Last.fm users: profile, sync cursor, version, and counters.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{Result, changed};
use crate::names::fold;
use crate::pool::Store;
use crate::types::{UserId, UserStats, count};

/// Profile fields as `user.getInfo` returns them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    /// Real name.
    pub realname: String,
    /// Profile URL.
    pub url: String,
    /// Country name.
    pub country: String,
    /// Gender code.
    pub gender: String,
    /// Age, 0 when hidden.
    pub age: i64,
    /// Whether the user subscribes to Last.fm Pro.
    pub subscriber: bool,
    /// Playlist count.
    pub playlists: i64,
    /// Bootstrap flag as Last.fm reports it.
    pub bootstrap: i64,
    /// Registration time in unix seconds.
    pub registered: i64,
    /// Avatar image key.
    pub image: Option<String>,
    /// Account type, such as `user` or `subscriber`.
    pub user_type: String,
}

/// A mirrored user with everything the store keeps about them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirroredUser {
    /// Store id.
    pub id: UserId,
    /// Last.fm username as first seen.
    pub name: String,
    /// Profile fields.
    pub profile: Profile,
    /// Where live sync continues: the last synced unix time.
    pub sync_cursor: i64,
    /// Bumped on every change to the user's data; drives cache invalidation and `ETag` headers.
    pub version: u64,
    /// Scrobble and library counts.
    pub stats: UserStats,
}

const USER_COLUMNS: &str = "id, name, realname, url, country, gender, age, subscriber, playlists, \
     bootstrap, registered_uts, image, user_type, sync_cursor, version, playcount, artist_count, \
     album_count, track_count";

impl Store {
    /// Returns the id of the user named `name` (case-insensitive), creating the user if needed.
    pub async fn ensure_user(&self, name: &str) -> Result<UserId> {
        let name = name.to_owned();
        self.write(move |tx| {
            let sql = "INSERT INTO users (name, name_fold) VALUES (?1, ?2) \
                       ON CONFLICT (name_fold) DO UPDATE SET name = name RETURNING id";
            Ok(tx
                .prepare_cached(sql)?
                .query_row(params![name, fold(&name)], |row| row.get(0))?)
        })
        .await
    }

    /// Looks up a user by name, case-insensitively.
    pub async fn user_by_name(&self, name: &str) -> Result<Option<MirroredUser>> {
        let name_fold = fold(name);
        self.read(move |conn| {
            let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE name_fold = ?1");
            Ok(conn
                .prepare_cached(&sql)?
                .query_row([name_fold], user_from_row)
                .optional()?)
        })
        .await
    }

    /// Looks up a user by id.
    pub async fn user(&self, user: UserId) -> Result<Option<MirroredUser>> {
        self.read(move |conn| {
            let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1");
            Ok(conn
                .prepare_cached(&sql)?
                .query_row([user], user_from_row)
                .optional()?)
        })
        .await
    }

    /// Replaces the user's profile fields and bumps the version.
    pub async fn update_profile(&self, user: UserId, profile: Profile) -> Result<()> {
        self.write(move |tx| {
            let sql = "UPDATE users SET realname = ?2, url = ?3, country = ?4, gender = ?5, \
                       age = ?6, subscriber = ?7, playlists = ?8, bootstrap = ?9, \
                       registered_uts = ?10, image = ?11, user_type = ?12 WHERE id = ?1";
            let p = &profile;
            let image = p.image.as_deref().and_then(crate::names::image_key);
            tx.prepare_cached(sql)?.execute(params![
                user,
                p.realname,
                p.url,
                p.country,
                p.gender,
                p.age,
                p.subscriber,
                p.playlists,
                p.bootstrap,
                p.registered,
                image,
                p.user_type
            ])?;
            bump_version(tx, user)
        })
        .await
    }

    /// Stores where live sync continues.
    pub async fn set_sync_cursor(&self, user: UserId, uts: i64) -> Result<()> {
        self.write(move |tx| {
            let sql = "UPDATE users SET sync_cursor = ?2 WHERE id = ?1";
            changed(tx.prepare_cached(sql)?.execute(params![user, uts])?)
        })
        .await
    }
}

/// Marks the user's data as changed.
pub(crate) fn bump_version(conn: &Connection, user: UserId) -> Result<()> {
    let sql = "UPDATE users SET version = version + 1 WHERE id = ?1";
    changed(conn.prepare_cached(sql)?.execute([user])?)
}

/// The UTC day the user's period totals end on, if they have been built.
pub(crate) fn period_day(conn: &Connection, user: UserId) -> Result<Option<i64>> {
    let sql = "SELECT period_day FROM users WHERE id = ?1";
    let day = conn
        .prepare_cached(sql)?
        .query_row([user], |row| row.get(0))
        .optional()?;
    Ok(day.flatten())
}

fn user_from_row(row: &Row<'_>) -> rusqlite::Result<MirroredUser> {
    let profile = Profile {
        realname: row.get(2)?,
        url: row.get(3)?,
        country: row.get(4)?,
        gender: row.get(5)?,
        age: row.get(6)?,
        subscriber: row.get(7)?,
        playlists: row.get(8)?,
        bootstrap: row.get(9)?,
        registered: row.get(10)?,
        image: row.get(11)?,
        user_type: row.get(12)?,
    };
    let stats = UserStats {
        playcount: count(row.get(15)?),
        artists: count(row.get(16)?),
        albums: count(row.get(17)?),
        tracks: count(row.get(18)?),
    };
    Ok(MirroredUser {
        id: row.get(0)?,
        name: row.get(1)?,
        profile,
        sync_cursor: row.get(13)?,
        version: count(row.get(14)?),
        stats,
    })
}
