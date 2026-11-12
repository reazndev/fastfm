//! Command-line parsing.

use std::path::PathBuf;

pub(crate) const USAGE: &str = "usage:
  lfm-sync import    --user <name> --db <path> [--until <uts>] [--rps <n>] [--concurrency <n>]
  lfm-sync reconcile --user <name> --db <path> [--until <uts>] [--rps <n>]
  lfm-sync live      --user <name> --db <path> [--rps <n>]
Reads LASTFM_API_KEYS from the environment or from .env in the working directory.";

/// What to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Import,
    Reconcile,
    Live,
}

/// Parsed arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Args {
    pub(crate) command: Command,
    pub(crate) user: String,
    pub(crate) db: PathBuf,
    pub(crate) until: Option<i64>,
    pub(crate) rps: u32,
    pub(crate) concurrency: usize,
}

pub(crate) fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let command = match args.next().as_deref() {
        Some("import") => Command::Import,
        Some("reconcile") => Command::Reconcile,
        Some("live") => Command::Live,
        Some(other) => return Err(format!("unknown command {other}")),
        None => return Err("missing command".to_owned()),
    };
    let (mut user, mut db, mut until) = (None, None, None);
    let (mut rps, mut concurrency) = (8, 8);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--user" => user = Some(value),
            "--db" => db = Some(PathBuf::from(value)),
            "--until" => until = Some(number(&flag, &value)?),
            "--rps" => rps = number(&flag, &value)?,
            "--concurrency" => concurrency = number(&flag, &value)?,
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    Ok(Args {
        command,
        user: user.ok_or("--user is required")?,
        db: db.ok_or("--db is required")?,
        until,
        rps,
        concurrency,
    })
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} expects a number, got {value}"))
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    fn args(text: &str) -> impl Iterator<Item = String> {
        text.split_whitespace().map(str::to_owned)
    }

    #[test]
    fn parses_import_with_cutoff() {
        let parsed = parse(args("import --user bob --db x.db --until 42 --rps 3")).unwrap();
        assert_eq!(parsed.command, Command::Import);
        assert_eq!(parsed.user, "bob");
        assert_eq!(parsed.until, Some(42));
        assert_eq!(parsed.rps, 3);
        assert_eq!(parsed.concurrency, 8);
    }

    #[test]
    fn rejects_missing_and_unknown_arguments() {
        assert!(parse(args("import --db x.db")).is_err());
        assert!(parse(args("sync --user a --db b")).is_err());
        assert!(parse(args("live --user a --db b --fast 1")).is_err());
        assert!(parse(args("live --user")).is_err());
    }
}
