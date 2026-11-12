//! Reading the API keys from the environment or `.env`.

use lfm_upstream::ApiKey;

/// The keys from `LASTFM_API_KEYS`, comma separated. A process variable wins over `.env`.
pub(crate) fn api_keys() -> Result<Vec<ApiKey>, String> {
    let file = std::fs::read_to_string(".env").unwrap_or_default();
    let value = std::env::var("LASTFM_API_KEYS")
        .ok()
        .or_else(|| file_value(&file, "LASTFM_API_KEYS"))
        .unwrap_or_default();
    let keys: Vec<ApiKey> = value
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(ApiKey::new)
        .collect();
    if keys.is_empty() {
        return Err("LASTFM_API_KEYS is not set".to_owned());
    }
    Ok(keys)
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
