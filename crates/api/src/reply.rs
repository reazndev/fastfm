//! Rendered replies: status, headers and body, ready for the HTTP layer.

use std::borrow::Cow;

use bytes::Bytes;
use lfm_core::{ApiError, Format, Output, Style};

use crate::body::Body;
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

/// Sent with every JSONP reply, whose first bytes the caller picks; see `Output::from_params`.
/// Last.fm does not send it, but compares equal: only `access-control-*` headers are copied.
const NOSNIFF: (&str, &str) = ("x-content-type-options", "nosniff");

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
    /// The body, with its entity tag and compressed copies.
    pub body: Body,
}

impl Reply {
    /// Replays Last.fm's answer, wrapping JSON in the request's callback.
    pub(crate) fn replay(entry: &Entry, body: Body, output: &Output, source: Source) -> Self {
        let jsonp = output.is_jsonp();
        let (content_type, body) = if jsonp {
            (
                output.content_type().into(),
                Body::hashed(Bytes::from(output.wrap_json(&body))),
            )
        } else {
            (entry.content_type.clone().into(), body)
        };
        let headers = entry
            .headers
            .iter()
            .map(|(name, value)| (name.clone().into(), value.clone().into()))
            .chain(jsonp.then_some((NOSNIFF.0.into(), NOSNIFF.1.into())))
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

    pub(crate) fn reply(self, output: &Output, status: u16, body: Body) -> Reply {
        let utf8 = self.utf8_json && output.format() == Format::Json && output.callback().is_none();
        let content_type = if utf8 {
            "application/json; charset=UTF-8"
        } else {
            output.content_type()
        };
        let headers = [
            self.utf8_json.then_some(REQUEST_HEADERS),
            output.is_jsonp().then_some(NOSNIFF),
        ]
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
        Reply {
            status,
            content_type: content_type.into(),
            headers,
            source: Source::Local,
            body,
        }
    }
}

/// A plain-text reply, as Last.fm's old submissions protocol answers.
pub(crate) fn text_reply(text: &'static str) -> Reply {
    Reply {
        status: 200,
        content_type: "text/plain; charset=utf-8".into(),
        headers: Vec::new(),
        source: Source::Local,
        body: Body::hashed(Bytes::from_static(text.as_bytes())),
    }
}

pub(crate) fn error_reply(output: &Output, error: &ApiError, shape: Shape) -> Reply {
    let body = Body::from(output.render_error_in(error, shape.style));
    shape.reply(output, error.http_status(), body)
}
