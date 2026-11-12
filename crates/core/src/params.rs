//! Request parameters merged from the query string and the form body.

use crate::error::ApiError;

/// Request parameters in arrival order. A later value for the same name wins, so a form body
/// merged after the query string overrides it, as PHP's `$_REQUEST` does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params {
    pairs: Vec<(String, String)>,
}

impl Params {
    /// No parameters.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses an `application/x-www-form-urlencoded` string and appends its pairs.
    #[must_use]
    pub fn with_urlencoded(mut self, input: &[u8]) -> Self {
        self.pairs
            .extend(form_urlencoded::parse(input).map(|(n, v)| (n.into_owned(), v.into_owned())));
        self
    }

    /// Appends one pair.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.pairs.push((name.into(), value.into()));
        self
    }

    /// The effective value of a parameter, which may be empty.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The value of a parameter, treating an empty value as absent.
    #[must_use]
    pub fn optional(&self, name: &str) -> Option<&str> {
        self.get(name).filter(|v| !v.is_empty())
    }

    /// The value of a required parameter.
    ///
    /// # Errors
    /// Error 6 with the default message when the parameter is absent or empty.
    pub fn required(&self, name: &str) -> Result<&str, ApiError> {
        self.optional(name).ok_or_else(ApiError::missing_parameter)
    }

    /// A boolean flag such as `extended` or `autocorrect`: `1` or `true` is on.
    #[must_use]
    pub fn flag(&self, name: &str) -> bool {
        self.optional(name)
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    }

    /// A non-negative integer parameter; absent or unparsable values give `None`.
    #[must_use]
    pub fn number(&self, name: &str) -> Option<u64> {
        self.optional(name).and_then(|v| v.trim().parse().ok())
    }

    /// The effective value of every parameter name, sorted by name.
    #[must_use]
    pub fn effective(&self) -> std::collections::BTreeMap<&str, &str> {
        self.pairs
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect()
    }
}
