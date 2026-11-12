//! Name folding and image keys.

/// Folds a name for case-insensitive matching, as Last.fm lookups ignore case.
#[must_use]
pub fn fold(name: &str) -> String {
    caseless::default_case_fold_str(name)
}

/// Extracts the image key from a Last.fm image URL or returns an already bare key.
///
/// `https://lastfm.freetls.fastly.net/i/u/300x300/<hash>.png` gives `<hash>.png`. The extension
/// is kept because Last.fm serves both `.png` and `.jpg`. Returns `None` for an empty value or one
/// that does not end in a hex hash.
#[must_use]
pub fn image_key(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let file = path.rsplit('/').next().unwrap_or_default();
    let (hash, ext) = file.split_once('.').unwrap_or((file, ""));
    let hash_ok = !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit());
    let ext_ok = ext.bytes().all(|b| b.is_ascii_alphanumeric());
    (hash_ok && ext_ok).then(|| file.to_owned())
}

/// Treats an empty or whitespace-only string as missing, as Last.fm sends `""` for none.
pub(crate) fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_non_ascii() {
        assert_eq!(fold("Straße"), fold("STRASSE"));
        assert_eq!(fold("Sigur Rós"), fold("SIGUR RÓS"));
        assert_eq!(fold("ΣΊΣΥΦΟΣ"), fold("σίσυφος"));
    }

    #[test]
    fn parses_image_urls() {
        let url =
            "https://lastfm.freetls.fastly.net/i/u/300x300/2a96cbd8b46e442fc41c2b86b821562f.png";
        assert_eq!(
            image_key(url).as_deref(),
            Some("2a96cbd8b46e442fc41c2b86b821562f.png")
        );
        let jpg = "https://lastfm.freetls.fastly.net/i/u/34s/c6f59c1e5e7240a4c0d427abd71f3dbb.jpg";
        assert_eq!(
            image_key(jpg).as_deref(),
            Some("c6f59c1e5e7240a4c0d427abd71f3dbb.jpg")
        );
        assert_eq!(image_key("abc123.png").as_deref(), Some("abc123.png"));
        assert_eq!(image_key("abc123").as_deref(), Some("abc123"));
        assert_eq!(image_key(""), None);
        assert_eq!(image_key("https://example.com/i/u/300x300/"), None);
        assert_eq!(image_key("https://example.com/not-a-hash.png"), None);
    }
}
