//! Loads the recorded request/response pairs from `fixtures/<method>/<variant>.*`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// The placeholder the recorder wrote in place of the real key.
pub const REDACTED: &str = "REDACTED";

/// Response format of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// `format=json`, including JSONP.
    Json,
    /// The default format.
    Xml,
}

impl Format {
    /// The file extension and the name used in case ids.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Xml => "xml",
        }
    }

    /// Parses `json` or `xml`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Json, Self::Xml]
            .into_iter()
            .find(|f| f.as_str() == text)
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// HTTP method of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// Parameters in the query string.
    Get,
    /// Parameters as an urlencoded form body.
    Post,
}

/// What Last.fm answered when the fixture was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// HTTP status.
    pub status: u16,
    /// Response headers, names in lower case.
    pub headers: BTreeMap<String, String>,
    /// Body bytes, with the real key replaced by `REDACTED`.
    pub body: Vec<u8>,
}

/// One request in one format, and the answer it must produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    /// Last.fm method, such as `user.getInfo`.
    pub method: String,
    /// Fixture variant, such as `default`.
    pub variant: String,
    /// Response format.
    pub format: Format,
    /// GET or POST.
    pub http_method: HttpMethod,
    /// Parameters in the recorded order, `api_key` as recorded.
    pub params: Vec<(String, String)>,
    /// The recorded answer.
    pub expected: Recorded,
}

impl Case {
    /// `method/variant.format`, unique across the fixture set.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}/{}.{}", self.method, self.variant, self.format)
    }
}

#[derive(Debug, Deserialize)]
struct Meta {
    method: String,
    variant: String,
    http_method: HttpMethod,
    responses: BTreeMap<Format, ResponseMeta>,
}

#[derive(Debug, Deserialize)]
struct ResponseMeta {
    params: Vec<(String, String)>,
    status: u16,
    headers: BTreeMap<String, String>,
}

/// Loads every case under `dir`, sorted by id.
pub fn load(dir: &Path) -> Result<Vec<Case>, Error> {
    let mut metas = Vec::new();
    for method_dir in read_dir(dir)?.into_iter().filter(|p| p.is_dir()) {
        metas.extend(
            read_dir(&method_dir)?
                .into_iter()
                .filter(|p| p.to_string_lossy().ends_with(".meta.json")),
        );
    }
    let mut cases = Vec::new();
    for path in metas {
        cases.extend(load_meta(&path)?);
    }
    cases.sort_by_key(Case::id);
    Ok(cases)
}

fn read_dir(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    std::fs::read_dir(dir)
        .and_then(|entries| {
            entries
                .map(|entry| entry.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(Error::io(dir))
}

fn load_meta(path: &Path) -> Result<Vec<Case>, Error> {
    let text = std::fs::read_to_string(path).map_err(Error::io(path))?;
    let meta: Meta = serde_json::from_str(&text).map_err(|e| Error::Fixture {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    let dir = path.parent().unwrap_or(Path::new("."));
    meta.responses
        .into_iter()
        .map(|(format, response)| {
            let body_path = dir.join(format!("{}.{format}", meta.variant));
            let body = std::fs::read(&body_path).map_err(Error::io(&body_path))?;
            Ok(Case {
                method: meta.method.clone(),
                variant: meta.variant.clone(),
                format,
                http_method: meta.http_method,
                params: response.params,
                expected: Recorded {
                    status: response.status,
                    headers: lowercase_keys(response.headers),
                    body,
                },
            })
        })
        .collect()
}

fn lowercase_keys(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .collect()
}

/// Keeps the cases a run asked for: id prefixes (any of them, or all cases when empty) and an
/// optional format.
#[must_use]
pub fn select(cases: Vec<Case>, only: &[String], format: Option<Format>) -> Vec<Case> {
    cases
        .into_iter()
        .filter(|c| format.is_none_or(|f| c.format == f))
        .filter(|c| {
            let id = c.id();
            only.is_empty() || only.iter().any(|p| id.starts_with(p.as_str()))
        })
        .collect()
}
