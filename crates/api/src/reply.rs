//! Rendered replies: status, headers and body, ready for the HTTP layer.

use std::borrow::Cow;

use bytes::Bytes;
use lfm_core::{ApiError, Format, Output, Style};

use crate::method::Method;
use crate::passthrough::Entry;

/// A header name or value.
pub type HeaderText = Cow<'static, str>;

/// The header Last.fm adds to the methods whose JSON carries `charset=UTF-8`.
const REQUEST_HEADERS: (&str, &str) = (
    "access-control-request-headers",
    "Origin, X-Atmosphere-tracking-id, X-Atmosphere-Framework, X-Cache-Date, Content-Type, \
     X-Atmosphere-Transport, *",
);

/// Where a reply came from, as the `X-Mirror-Source` header shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Computed from the store, or an error raised here.
    Local,
    /// A fresh copy of Last.fm's answer.
    Cache,
    /// An expired copy of Last.fm's answer, served while it is refreshed.
    Stale,
    /// Last.fm's answer, fetched for this request.
    Upstream,
}

impl Source {
    /// The header value: `local`, `cache`, `stale` or `upstream`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Cache => "cache",
            Self::Stale => "stale",
            Self::Upstream => "upstream",
        }
    }
}

/// A rendered reply, ready for the HTTP layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The `Content-Type` header.
    pub content_type: HeaderText,
    /// Headers beyond the content type. The HTTP layer adds Last.fm's usual CORS headers unless
    /// these carry `access-control-allow-origin`, as replayed Last.fm answers do.
    pub headers: Vec<(HeaderText, HeaderText)>,
    /// Where the reply came from.
    pub source: Source,
    /// The body.
    pub body: Bytes,
}

impl Reply {
    /// Replays Last.fm's answer, wrapping JSON in the request's callback.
    pub(crate) fn replay(entry: &Entry, body: Bytes, output: &Output, source: Source) -> Self {
        let jsonp = output.format() == Format::Json && output.callback().is_some();
        let (content_type, body) = if jsonp {
            (
                output.content_type().into(),
                Bytes::from(output.wrap_json(&body)),
            )
        } else {
            (entry.content_type.clone().into(), body)
        };
        let headers = entry
            .headers
            .iter()
            .map(|(name, value)| (name.clone().into(), value.clone().into()))
            .collect();
        Self {
            status: entry.status,
            content_type,
            headers,
            source,
            body,
        }
    }
}

/// How a local method's replies look apart from the body.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Shape {
    style: Style,
    utf8_json: bool,
}

/// Errors raised before a method is chosen, and key errors, look like the classic backend's.
pub(crate) const CLASSIC: Shape = Shape {
    style: Style::Classic,
    utf8_json: false,
};

impl Shape {
    pub(crate) fn of(method: &dyn Method) -> Self {
        Self {
            style: method.style(),
            utf8_json: method.utf8_json(),
        }
    }

    fn reply(self, output: &Output, status: u16, body: Bytes) -> Reply {
        let utf8 = self.utf8_json && output.format() == Format::Json && output.callback().is_none();
        let content_type = if utf8 {
            "application/json; charset=UTF-8"
        } else {
            output.content_type()
        };
        let headers = if self.utf8_json {
            vec![(REQUEST_HEADERS.0.into(), REQUEST_HEADERS.1.into())]
        } else {
            Vec::new()
        };
        Reply {
            status,
            content_type: content_type.into(),
            headers,
            source: Source::Local,
            body,
        }
    }
}

pub(crate) fn ok_reply(output: &Output, body: &Bytes, shape: Shape) -> Reply {
    let body = match (output.format(), output.callback()) {
        (Format::Json, Some(_)) => Bytes::from(output.wrap_json(body)),
        _ => body.clone(),
    };
    shape.reply(output, 200, body)
}

pub(crate) fn error_reply(output: &Output, error: &ApiError, shape: Shape) -> Reply {
    let body = Bytes::from(output.render_error_in(error, shape.style));
    shape.reply(output, error.http_status(), body)
}
