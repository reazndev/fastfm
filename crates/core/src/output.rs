//! The output format chosen by `format=` and `callback=`, and rendering into one buffer.

use crate::error::{ApiError, ErrorCode};
use crate::params::Params;
use crate::tree::{Response, Style};
use crate::{json, xml};

/// The response format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Last.fm's default.
    Xml,
    /// `format=json`.
    Json,
}

impl Format {
    /// Parses `format=`; absent means XML.
    ///
    /// # Errors
    /// Error 5 for any other value. Last.fm renders that error as XML.
    pub fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        match value {
            None | Some("xml") => Ok(Self::Xml),
            Some("json") => Ok(Self::Json),
            Some(_) => Err(ApiError::new(ErrorCode::InvalidFormat)),
        }
    }

    /// The format name as it appears in `format=`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Xml => "xml",
            Self::Json => "json",
        }
    }
}

/// How a response is written: format plus an optional JSONP callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    format: Format,
    callback: Option<String>,
}

impl Output {
    /// Plain XML, used for errors raised before the format is known.
    #[must_use]
    pub fn xml() -> Self {
        Self::plain(Format::Xml)
    }

    /// A format without a JSONP callback.
    #[must_use]
    pub fn plain(format: Format) -> Self {
        Self {
            format,
            callback: None,
        }
    }

    /// Reads `format` and `callback`. A callback only applies to JSON, and only when it is a
    /// plain JavaScript identifier path, so it cannot inject script.
    ///
    /// # Errors
    /// Error 5 for an unknown format.
    pub fn from_params(params: &Params) -> Result<Self, ApiError> {
        let format = Format::parse(params.get("format"))?;
        let callback = params
            .optional("callback")
            .filter(|_| format == Format::Json)
            .filter(|cb| is_safe_callback(cb))
            .map(str::to_owned);
        Ok(Self { format, callback })
    }

    /// The format.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }

    /// The JSONP callback, if any.
    #[must_use]
    pub fn callback(&self) -> Option<&str> {
        self.callback.as_deref()
    }

    /// The `Content-Type` header Last.fm sends for this output.
    #[must_use]
    pub fn content_type(&self) -> &'static str {
        match (self.format, &self.callback) {
            (Format::Xml, _) => "text/xml; charset=UTF-8",
            (Format::Json, None) => "application/json",
            (Format::Json, Some(_)) => "text/javascript; charset=utf-8",
        }
    }

    /// Renders a successful response.
    #[must_use]
    pub fn render(&self, response: &Response) -> Vec<u8> {
        let mut out = Vec::with_capacity(4096);
        match self.format {
            Format::Xml => xml::render(response, &mut out),
            Format::Json => self.wrap(&mut out, |out| json::render(response, out)),
        }
        out
    }

    /// Renders an error response in the classic style.
    #[must_use]
    pub fn render_error(&self, error: &ApiError) -> Vec<u8> {
        self.render_error_in(error, Style::Classic)
    }

    /// Renders an error response in the given backend's style.
    #[must_use]
    pub fn render_error_in(&self, error: &ApiError, style: Style) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        match self.format {
            Format::Xml => xml::render_error(error, style, &mut out),
            Format::Json => self.wrap(&mut out, |out| json::render_error(error, style, out)),
        }
        out
    }

    /// Wraps already rendered JSON in the callback, for bodies served from a cache.
    #[must_use]
    pub fn wrap_json(&self, json: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(json.len() + 32);
        self.wrap(&mut out, |out| out.extend_from_slice(json));
        out
    }

    // Last.fm writes `cb(...);` with no trailing newline.
    fn wrap(&self, out: &mut Vec<u8>, body: impl FnOnce(&mut Vec<u8>)) {
        match &self.callback {
            Some(callback) => {
                out.extend_from_slice(callback.as_bytes());
                out.push(b'(');
                body(out);
                out.extend_from_slice(b");");
            }
            None => body(out),
        }
    }
}

fn is_safe_callback(callback: &str) -> bool {
    callback.len() <= 128
        && callback
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'.'))
}
