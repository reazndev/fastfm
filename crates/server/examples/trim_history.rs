//! Removes the scrobbles after a cutoff from a database copy, per user, through the store so
//! every total stays consistent. `scripts/conformance-multi.sh prepare` uses it to bring a copy
//! of a synced database back to the moment saved Last.fm answers were taken.
//!
//! ```sh
//! cargo run -p lfm-mirror --example trim_history -- <database> <cutoffs file>
//! ```
//!
//! Each line of the cutoffs file is `<user> <unix time>`; scrobbles after that time go.
#![allow(
    clippy::print_stdout,
    reason = "a command-line tool that reports what it removed"
)]

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use lfm_store::{Store, TimeRange};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [db, cutoffs] = args.as_slice() else {
        bail!("usage: trim_history <database> <cutoffs file>");
    };
    let store = Store::open(&PathBuf::from(db)).context("opening the database")?;
    let cutoffs = std::fs::read_to_string(cutoffs).context("reading the cutoffs")?;
    for line in cutoffs.lines().filter(|line| !line.trim().is_empty()) {
        let (name, at) = line
            .split_once(' ')
            .with_context(|| format!("not `<user> <time>`: {line}"))?;
        let at: i64 = at.trim().parse().context("the cutoff time")?;
        let user = store
            .user_by_name(name)
            .await?
            .with_context(|| format!("no user {name}"))?;
        let after = TimeRange::new(at.saturating_add(1), TimeRange::ALL.to);
        store.replace_window(user.id, after, Vec::new()).await?;
        println!("{name}: removed scrobbles after {at}");
    }
    Ok(())
}
