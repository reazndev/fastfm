//! Picks the structural diff for a body: XML, JSON, JSONP, or bytes for plain text.

use super::{Category, Difference, json, json_parse, xml, xml_parse};
use crate::fixture::Format;

const CONTEXT_BYTES: usize = 40;

/// Nothing when the bytes match. Otherwise the structural differences, or one `bytes`
/// difference when the structure explains nothing, so a byte mismatch is never lost.
pub(super) fn compare(format: Format, expected: &[u8], actual: &[u8]) -> Vec<Difference> {
    if expected == actual {
        return Vec::new();
    }
    let structural = match format {
        Format::Xml => xml_body(expected, actual),
        Format::Json => json_body(expected, actual),
    };
    if structural.is_empty() {
        vec![bytes_difference(expected, actual)]
    } else {
        structural
    }
}

fn xml_body(expected: &[u8], actual: &[u8]) -> Vec<Difference> {
    // A fixture that is not XML (the plain-text scrobble answer) falls back to bytes.
    let Ok(want) = xml_parse::parse(expected) else {
        return Vec::new();
    };
    match xml_parse::parse(actual) {
        Ok(got) => xml::diff(&want, &got),
        Err(err) => vec![parse_difference("XML", &err.to_string())],
    }
}

fn json_body(expected: &[u8], actual: &[u8]) -> Vec<Difference> {
    if let Some((callback, inner)) = split_jsonp(expected) {
        return jsonp_body(callback, inner, actual);
    }
    let Ok(want) = json_parse::parse(expected) else {
        return Vec::new();
    };
    match json_parse::parse(actual) {
        Ok(got) => json::diff(&want, &got),
        Err(err) => vec![parse_difference("JSON", &err.to_string())],
    }
}

fn jsonp_body(callback: &[u8], inner: &[u8], actual: &[u8]) -> Vec<Difference> {
    let (wrapper, actual_inner) = if let Some((got, got_inner)) = split_jsonp(actual) {
        let wrapper = (got != callback)
            .then(|| Difference::new(Category::Wrapper, "", text(callback), text(got)));
        (wrapper, got_inner)
    } else {
        let wrapper = Difference::new(
            Category::Wrapper,
            "",
            format!("{}(…);", text(callback)),
            "no callback wrapper",
        );
        (Some(wrapper), actual)
    };
    let inner_diffs = match (json_parse::parse(inner), json_parse::parse(actual_inner)) {
        (Ok(want), Ok(got)) => json::diff(&want, &got),
        (Ok(_), Err(err)) => vec![parse_difference("JSON", &err.to_string())],
        (Err(_), _) => Vec::new(),
    };
    wrapper.into_iter().chain(inner_diffs).collect()
}

/// `callback(` … `);` → (callback, inner).
fn split_jsonp(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let open = body.iter().position(|&b| b == b'(')?;
    let (callback, rest) = body.split_at(open);
    let inner = rest.get(1..)?.strip_suffix(b");")?;
    let plausible = !callback.is_empty()
        && !callback
            .iter()
            .any(|b| matches!(b, b'{' | b'[' | b'"' | b'<'));
    plausible.then_some((callback, inner))
}

fn parse_difference(language: &str, error: &str) -> Difference {
    Difference::new(Category::Parse, "", format!("valid {language}"), error)
}

fn bytes_difference(expected: &[u8], actual: &[u8]) -> Difference {
    let offset = expected
        .iter()
        .zip(actual)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let around = |bytes: &[u8]| {
        let window = bytes.get(offset..).unwrap_or_default();
        let window = window.get(..CONTEXT_BYTES).unwrap_or(window);
        format!(
            "{} bytes, at {offset}: {}",
            bytes.len(),
            String::from_utf8_lossy(window)
        )
    };
    Difference::new(Category::Bytes, "", around(expected), around(actual))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
