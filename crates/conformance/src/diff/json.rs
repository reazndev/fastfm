//! Structural diff of two JSON documents.

use super::json_parse::{Document, Str, Value};
use super::pairing::pair;
use super::{Category, Difference, child};

const ABSENT: &str = "(absent)";

/// Values, types, shapes and key order from the root down, then whitespace.
pub(super) fn diff(expected: &Document, actual: &Document) -> Vec<Difference> {
    let layout = (expected.whitespace != actual.whitespace)
        .then(|| whitespace_difference(&expected.whitespace, &actual.whitespace));
    value("", &expected.root, &actual.root)
        .into_iter()
        .chain(layout)
        .collect()
}

fn value(path: &str, expected: &Value, actual: &Value) -> Vec<Difference> {
    match (expected, actual) {
        (Value::Object(want), Value::Object(got)) => object(path, want, got),
        (Value::Array(want), Value::Array(got)) => array(path, want, got),
        (Value::String(want), Value::String(got)) => string(path, want, got).into_iter().collect(),
        (Value::Number(want), Value::Number(got)) => (want != got)
            .then(|| Difference::new(Category::Value, path, want, got))
            .into_iter()
            .collect(),
        (Value::Bool(want), Value::Bool(got)) => (want != got)
            .then(|| Difference::new(Category::Value, path, want.to_string(), got.to_string()))
            .into_iter()
            .collect(),
        (Value::Null, Value::Null) => Vec::new(),
        _ => vec![kind_difference(path, expected, actual)],
    }
}

fn string(path: &str, expected: &Str, actual: &Str) -> Option<Difference> {
    if expected.value != actual.value {
        Some(Difference::new(
            Category::Value,
            path,
            &expected.value,
            &actual.value,
        ))
    } else if expected.raw != actual.raw {
        Some(Difference::new(
            Category::Escape,
            path,
            &expected.raw,
            &actual.raw,
        ))
    } else {
        None
    }
}

fn kind_difference(path: &str, expected: &Value, actual: &Value) -> Difference {
    let category = if is_container(expected) || is_container(actual) {
        Category::Shape
    } else {
        Category::Type
    };
    let describe = |v: &Value| format!("{} {}", kind(v), render(v));
    Difference::new(category, path, describe(expected), describe(actual))
}

fn object<'a>(
    path: &str,
    expected: &'a [(Str, Value)],
    actual: &'a [(Str, Value)],
) -> Vec<Difference> {
    let named = |(k, v): &'a (Str, Value)| (k.value.as_str(), v);
    let paired = pair(expected.iter().map(named), actual.iter().map(named));
    let missing = paired.missing.iter().map(|((name, _), v)| {
        Difference::new(Category::Missing, &child(path, name), render(v), ABSENT)
    });
    let extra = paired.extra.iter().map(|((name, _), v)| {
        Difference::new(Category::Extra, &child(path, name), ABSENT, render(v))
    });
    let children = paired
        .both
        .iter()
        .flat_map(|((name, _), want, got)| value(&child(path, name), want, got));
    missing
        .chain(extra)
        .chain(paired.order_difference(path, ""))
        .chain(children)
        .collect()
}

fn array(path: &str, expected: &[Value], actual: &[Value]) -> Vec<Difference> {
    let index = |i: usize| format!("{path}[{i}]");
    let paired = expected
        .iter()
        .zip(actual)
        .enumerate()
        .flat_map(|(i, (want, got))| value(&index(i), want, got));
    let missing = expected
        .iter()
        .enumerate()
        .skip(actual.len())
        .map(|(i, v)| Difference::new(Category::Missing, &index(i), render(v), ABSENT));
    let extra = actual
        .iter()
        .enumerate()
        .skip(expected.len())
        .map(|(i, v)| Difference::new(Category::Extra, &index(i), ABSENT, render(v)));
    paired.chain(missing).chain(extra).collect()
}

fn whitespace_difference(expected: &[(usize, String)], actual: &[(usize, String)]) -> Difference {
    let first = expected
        .iter()
        .zip(actual)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let describe = |runs: &[(usize, String)]| match runs.get(first) {
        Some((token, run)) => format!(
            "{} runs; run {first}: {run:?} after token {token}",
            runs.len()
        ),
        None => format!("{} runs", runs.len()),
    };
    Difference::new(
        Category::Whitespace,
        "",
        describe(expected),
        describe(actual),
    )
}

fn is_container(value: &Value) -> bool {
    matches!(value, Value::Array(_) | Value::Object(_))
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Compact JSON text from the raw parts, so a preview shows the value as it was sent.
fn render(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(raw) => raw.clone(),
        Value::String(s) => format!("\"{}\"", s.raw),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(render).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(members) => {
            let inner: Vec<String> = members
                .iter()
                .map(|(k, v)| format!("\"{}\":{}", k.raw, render(v)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}
