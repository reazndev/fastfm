//! Deserialisers for Last.fm's JSON quirks.

use serde::{Deserialize, Deserializer};

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    Many(Vec<T>),
    One(T),
}

/// Accepts a list, or a single object where Last.fm collapses a one-item list.
pub(super) fn one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::Many(items) => items,
        OneOrMany::One(item) => vec![item],
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    Int(u64),
    Text(String),
}

/// Accepts a number or a numeric string; an empty string is 0.
pub(super) fn number<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<u64> + Default,
{
    let value = match Number::deserialize(deserializer)? {
        Number::Int(n) => n,
        Number::Text(text) if text.trim().is_empty() => return Ok(T::default()),
        Number::Text(text) => text.trim().parse().map_err(serde::de::Error::custom)?,
    };
    T::try_from(value).map_err(|_| serde::de::Error::custom("number out of range"))
}

/// Accepts Last.fm's boolean spellings: `"1"`, `"true"`, 1, true.
pub(super) fn flag<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flag {
        Bool(bool),
        Int(u64),
        Text(String),
    }
    Ok(match Flag::deserialize(deserializer)? {
        Flag::Bool(b) => b,
        Flag::Int(n) => n != 0,
        Flag::Text(text) => matches!(text.as_str(), "1" | "true"),
    })
}

/// Turns Last.fm's empty-string MBIDs into `None`.
pub(super) fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}
