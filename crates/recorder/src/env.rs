use std::path::{Path, PathBuf};

use lfm_upstream::ApiKey;

use crate::error::RecordError;

/// What the recorder reads from `.env`.
#[derive(Debug)]
pub(crate) struct Env {
    pub(crate) keys: Vec<ApiKey>,
    pub(crate) user: String,
}

/// The repository root, found from this crate's location so the working directory does not
/// matter.
pub(crate) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

impl Env {
    /// Reads `.env` from the repository root. Process variables win over the file.
    pub(crate) fn load() -> Result<Self, RecordError> {
        let path = repo_root().join(".env");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| RecordError::Env(format!("{}: {e}", path.display())))?;
        let lookup = |name: &str| std::env::var(name).ok().or_else(|| file_value(&text, name));
        let keys: Vec<ApiKey> = lookup("LASTFM_API_KEYS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(ApiKey::new)
            .collect();
        let user = lookup("LASTFM_USER")
            .filter(|u| !u.is_empty())
            .ok_or_else(|| RecordError::Env("LASTFM_USER is not set".to_owned()))?;
        if keys.is_empty() {
            return Err(RecordError::Env("LASTFM_API_KEYS is empty".to_owned()));
        }
        Ok(Self { keys, user })
    }
}

fn file_value(text: &str, name: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| {
            value
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_owned()
        })
}

#[cfg(test)]
mod tests {
    use super::file_value;

    #[test]
    fn reads_plain_and_quoted_values() {
        let text = "# comment\nA=1\nB=\"two\"\n";
        assert_eq!(file_value(text, "A").as_deref(), Some("1"));
        assert_eq!(file_value(text, "B").as_deref(), Some("two"));
        assert_eq!(file_value(text, "C"), None);
    }
}
