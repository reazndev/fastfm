//! Test hook for checking reconciliation by hand: deletes a user's scrobbles in `[from, to)`
//! through the store, so its counts and rollups stay consistent.
//!
//! `cargo run -p lfm-sync --example drop_window -- <db> <user> <from> <to>`

use std::path::PathBuf;
use std::process::ExitCode;

use lfm_store::{Store, TimeRange};
use tracing::{error, info};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [db, user, from, to] = args.as_slice() else {
        error!("usage: drop_window <db> <user> <from> <to>");
        return ExitCode::FAILURE;
    };
    let (Ok(from), Ok(to)) = (from.parse(), to.parse()) else {
        error!("from and to must be Unix times");
        return ExitCode::FAILURE;
    };
    match drop_window(PathBuf::from(db), user, TimeRange::new(from, to)).await {
        Ok(dropped) => {
            info!(dropped, from, to, "window deleted");
            ExitCode::SUCCESS
        }
        Err(err) => {
            error!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn drop_window(db: PathBuf, user: &str, range: TimeRange) -> lfm_store::Result<u64> {
    let store = Store::open(&db)?;
    let user = store
        .user_by_name(user)
        .await?
        .ok_or(lfm_store::StoreError::NotFound)?;
    let dropped = store.count_range(user.id, range).await?;
    store.replace_window(user.id, range, Vec::new()).await?;
    Ok(dropped)
}
