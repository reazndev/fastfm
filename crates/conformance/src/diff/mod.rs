//! Compares a received response with the recorded one, level by level.

mod body;
mod http;
mod json;
mod json_parse;
mod pairing;
mod xml;
mod xml_parse;

use serde::Serialize;

use crate::client::Received;
use crate::fixture::Case;

/// The kind of a difference. The order is the order of the comparison levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// The request did not complete (connection refused, timeout).
    Transport,
    /// HTTP status.
    Status,
    /// `Content-Type` header.
    ContentType,
    /// A CORS (`access-control-*`) header is missing, extra or different.
    Header,
    /// The received body is not valid JSON or XML although the recorded one is.
    Parse,
    /// JSONP callback wrapper.
    Wrapper,
    /// XML declaration (`<?xml ... ?>`).
    Declaration,
    /// A key, element or attribute is in the fixture but not in the answer.
    Missing,
    /// A key, element or attribute is in the answer but not in the fixture.
    Extra,
    /// Array vs object vs scalar, or element with children vs text.
    Shape,
    /// JSON scalar type: string vs number vs boolean vs null.
    Type,
    /// Key, element or attribute order.
    Order,
    /// A scalar value or text.
    Value,
    /// Same value, different escaping (`\/` vs `/`, `&apos;` vs `'`).
    Escape,
    /// Indentation, line breaks, self-closing tags, trailing newline.
    Whitespace,
    /// Bodies differ and no structural difference explains it (plain-text bodies).
    Bytes,
}

impl Category {
    /// The name used in reports and ignore rules.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Status => "status",
            Self::ContentType => "content-type",
            Self::Header => "header",
            Self::Parse => "parse",
            Self::Wrapper => "wrapper",
            Self::Declaration => "declaration",
            Self::Missing => "missing",
            Self::Extra => "extra",
            Self::Shape => "shape",
            Self::Type => "type",
            Self::Order => "order",
            Self::Value => "value",
            Self::Escape => "escape",
            Self::Whitespace => "whitespace",
            Self::Bytes => "bytes",
        }
    }

    /// Parses a name produced by [`Category::as_str`].
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        ALL.into_iter().find(|c| c.as_str() == name)
    }
}

const ALL: [Category; 16] = [
    Category::Transport,
    Category::Status,
    Category::ContentType,
    Category::Header,
    Category::Parse,
    Category::Wrapper,
    Category::Declaration,
    Category::Missing,
    Category::Extra,
    Category::Shape,
    Category::Type,
    Category::Order,
    Category::Value,
    Category::Escape,
    Category::Whitespace,
    Category::Bytes,
];

/// One difference between the recorded and the received response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Difference {
    /// What kind of difference.
    pub category: Category,
    /// Where: `recenttracks.track[3].image[2].#text`, `header.content-type`, `status`.
    pub path: String,
    /// The recorded value, shortened.
    pub expected: String,
    /// The received value, shortened.
    pub actual: String,
}

impl Difference {
    pub(crate) fn new(
        category: Category,
        path: &str,
        expected: impl AsRef<str>,
        actual: impl AsRef<str>,
    ) -> Self {
        Self {
            category,
            path: display_path(path),
            expected: preview(expected.as_ref()),
            actual: preview(actual.as_ref()),
        }
    }
}

/// Every difference between the recorded answer of `case` and `received`, in level order.
/// Empty only when status, content type, CORS headers and body bytes all match.
#[must_use]
pub fn compare(case: &Case, received: &Received) -> Vec<Difference> {
    http::compare(&case.expected, received)
        .into_iter()
        .chain(body::compare(
            case.format,
            &case.expected.body,
            &received.body,
        ))
        .collect()
}

const PREVIEW_CHARS: usize = 120;

/// Shortens a value for the report, keeping control characters visible.
fn preview(text: &str) -> String {
    let escaped: String = text.chars().flat_map(char::escape_debug).collect();
    if escaped.chars().count() <= PREVIEW_CHARS {
        return escaped;
    }
    let cut: String = escaped.chars().take(PREVIEW_CHARS).collect();
    format!("{cut}… ({} chars)", text.chars().count())
}

fn display_path(path: &str) -> String {
    if path.is_empty() {
        "$".to_owned()
    } else {
        path.to_owned()
    }
}

/// `parent.key`, or `key` at the root.
fn child(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_owned()
    } else {
        format!("{parent}.{key}")
    }
}
