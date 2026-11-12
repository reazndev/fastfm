use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lfm_upstream::{Client, Format, HttpMethod, RawResponse};
use serde_json::{Map, Value, json};
use tracing::info;

use crate::error::RecordError;
use crate::spec::Spec;

const FORMATS: [(Format, &str); 2] = [(Format::Json, "json"), (Format::Xml, "xml")];
const REDACTED: &[u8] = b"REDACTED";

/// Writes fixtures under one directory and scrubs the real keys out of everything it writes.
#[derive(Debug)]
pub(crate) struct Writer {
    root: PathBuf,
    secrets: Vec<String>,
    force: bool,
}

impl Writer {
    pub(crate) fn new(root: PathBuf, secrets: Vec<String>, force: bool) -> Self {
        Self {
            root,
            secrets,
            force,
        }
    }

    /// Records one spec in both formats. Returns false when the fixture already existed.
    pub(crate) async fn record(&self, client: &Client, spec: &Spec) -> Result<bool, RecordError> {
        let dir = self.root.join(spec.method());
        let meta_path = dir.join(format!("{}.meta.json", spec.variant()));
        if !self.force && exists(&meta_path).await {
            return Ok(false);
        }
        tokio::fs::create_dir_all(&dir).await?;
        let mut responses = Map::new();
        for (format, ext) in FORMATS {
            let response = client.raw(&spec.request(format)).await?;
            info!(
                method = spec.method(),
                variant = spec.variant(),
                ext,
                status = response.status,
                "recorded"
            );
            let body = self.scrub(&response.body);
            tokio::fs::write(dir.join(format!("{}.{ext}", spec.variant())), body).await?;
            responses.insert(ext.to_owned(), Self::response_meta(spec, format, &response));
        }
        let meta = json!({
            "method": spec.method(),
            "variant": spec.variant(),
            "http_method": match spec.http_method() {
                HttpMethod::Get => "GET",
                HttpMethod::Post => "POST",
            },
            "recorded_at": unix_now(),
            "responses": responses,
        });
        let text = serde_json::to_string_pretty(&meta)?;
        tokio::fs::write(&meta_path, self.scrub(text.as_bytes())).await?;
        Ok(true)
    }

    fn response_meta(spec: &Spec, format: Format, response: &RawResponse) -> Value {
        let params: Vec<Value> = spec
            .request(format)
            .wire_params(spec.key_text())
            .into_iter()
            .map(|(k, v)| json!([k, v]))
            .collect();
        let headers: Map<String, Value> = response
            .headers
            .iter()
            .map(|(name, value)| {
                let text = String::from_utf8_lossy(value.as_bytes()).into_owned();
                (name.as_str().to_owned(), Value::String(text))
            })
            .collect();
        json!({
            "params": params,
            "status": response.status,
            "content_type": response.content_type,
            "headers": headers,
        })
    }

    fn scrub(&self, bytes: &[u8]) -> Vec<u8> {
        self.secrets.iter().fold(bytes.to_vec(), |acc, secret| {
            replace(&acc, secret.as_bytes())
        })
    }
}

/// Replaces every occurrence of `needle` with `REDACTED`.
fn replace(haystack: &[u8], needle: &[u8]) -> Vec<u8> {
    if needle.is_empty() {
        return haystack.to_vec();
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut rest = haystack;
    while let Some(pos) = rest.windows(needle.len()).position(|w| w == needle) {
        let (before, after) = rest.split_at(pos);
        out.extend_from_slice(before);
        out.extend_from_slice(REDACTED);
        rest = after.get(needle.len()..).unwrap_or_default();
    }
    out.extend_from_slice(rest);
    out
}

async fn exists(path: &Path) -> bool {
    tokio::fs::try_exists(path).await.unwrap_or(false)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_every_occurrence() {
        let out = replace(b"a KEY b KEY", b"KEY");
        assert_eq!(out, b"a REDACTED b REDACTED");
    }

    #[test]
    fn leaves_text_without_the_key_alone() {
        assert_eq!(replace(b"nothing here", b"KEY"), b"nothing here");
    }
}
