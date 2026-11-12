//! Structural diff of two XML documents.
//!
//! Paths leave out the `lfm` root and use the JSON names where Last.fm's two formats agree:
//! `recenttracks.track[3].image[2].#text`. An element's text is `path.#text` when the element
//! has attributes and plain `path` otherwise; attributes are `path.@name`. A repeated element
//! gets an index (`track[3]`), a single one does not.

use std::collections::BTreeMap;

use super::pairing::{compact_names, pair};
use super::xml_parse::{Attr, Document, Element, Node, Text};
use super::{Category, Difference, child};

const ABSENT: &str = "(absent)";

/// Declaration, the space around the root, then the root element downwards.
pub(super) fn diff(expected: &Document, actual: &Document) -> Vec<Difference> {
    let declaration = (expected.declaration != actual.declaration).then(|| {
        let show = |d: &Option<String>| d.clone().unwrap_or_else(|| ABSENT.to_owned());
        Difference::new(
            Category::Declaration,
            "",
            show(&expected.declaration),
            show(&actual.declaration),
        )
    });
    let prolog = (expected.prolog != actual.prolog).then(|| {
        let show = |s: &str| format!("after declaration: {s:?}");
        Difference::new(
            Category::Whitespace,
            "",
            show(&expected.prolog),
            show(&actual.prolog),
        )
    });
    let trailing = (expected.trailing != actual.trailing).then(|| {
        let show = |s: &str| format!("after root: {s:?}");
        Difference::new(
            Category::Whitespace,
            "",
            show(&expected.trailing),
            show(&actual.trailing),
        )
    });
    let root = if expected.root.name == actual.root.name {
        element("", &expected.root, &actual.root)
    } else {
        vec![Difference::new(
            Category::Shape,
            "",
            &expected.root.name,
            &actual.root.name,
        )]
    };
    declaration
        .into_iter()
        .chain(prolog)
        .chain(trailing)
        .chain(root)
        .collect()
}

fn element(path: &str, expected: &Element, actual: &Element) -> Vec<Difference> {
    let closing = (expected.self_closing != actual.self_closing
        && expected.children.is_empty()
        && actual.children.is_empty())
    .then(|| {
        let show = |e: &Element| {
            if e.self_closing {
                format!("<{}/>", e.name)
            } else {
                format!("<{0}></{0}>", e.name)
            }
        };
        Difference::new(Category::Whitespace, path, show(expected), show(actual))
    });
    let content = match (has_elements(expected), has_elements(actual)) {
        (false, false) => leaf_text(path, expected, actual).into_iter().collect(),
        (true, true) => container(path, expected, actual),
        _ => vec![Difference::new(
            Category::Shape,
            path,
            render(expected),
            render(actual),
        )],
    };
    attributes(path, &expected.attrs, &actual.attrs)
        .into_iter()
        .chain(closing)
        .chain(content)
        .collect()
}

fn attributes<'a>(path: &str, expected: &'a [Attr], actual: &'a [Attr]) -> Vec<Difference> {
    let named = |a: &'a Attr| (a.name.as_str(), &a.text);
    let paired = pair(expected.iter().map(named), actual.iter().map(named));
    let at = |name: &str| child(path, &format!("@{name}"));
    let missing = paired
        .missing
        .iter()
        .map(|((name, _), t)| Difference::new(Category::Missing, &at(name), &t.raw, ABSENT));
    let extra = paired
        .extra
        .iter()
        .map(|((name, _), t)| Difference::new(Category::Extra, &at(name), ABSENT, &t.raw));
    let values = paired
        .both
        .iter()
        .filter_map(|((name, _), want, got)| text(&at(name), want, got));
    missing
        .chain(extra)
        .chain(paired.order_difference(path, "@"))
        .chain(values)
        .collect()
}

fn leaf_text(path: &str, expected: &Element, actual: &Element) -> Option<Difference> {
    let path = if expected.attrs.is_empty() {
        path.to_owned()
    } else {
        child(path, "#text")
    };
    text(
        &path,
        &joined_text(expected, |_| true),
        &joined_text(actual, |_| true),
    )
}

fn text(path: &str, expected: &Text, actual: &Text) -> Option<Difference> {
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

fn container<'a>(path: &str, expected: &'a Element, actual: &'a Element) -> Vec<Difference> {
    let named = |c: &'a Element| (c.name.as_str(), c);
    let paired = pair(
        child_elements(expected).map(named),
        child_elements(actual).map(named),
    );
    let repeated = |name: &str| {
        let count = |e: &Element| child_elements(e).filter(|c| c.name == name).count();
        count(expected).max(count(actual)) > 1
    };
    let child_path = |(name, n): (&str, usize)| {
        if repeated(name) {
            format!("{}[{n}]", child(path, name))
        } else {
            child(path, name)
        }
    };
    let missing = paired
        .missing
        .iter()
        .map(|(k, e)| Difference::new(Category::Missing, &child_path(*k), render(e), ABSENT));
    let extra = paired
        .extra
        .iter()
        .map(|(k, e)| Difference::new(Category::Extra, &child_path(*k), ABSENT, render(e)));
    let mixed = text(
        &child(path, "#text"),
        &joined_text(expected, |t| !is_blank(t)),
        &joined_text(actual, |t| !is_blank(t)),
    );
    let children = paired
        .both
        .iter()
        .flat_map(|(k, want, got)| element(&child_path(*k), want, got));
    missing
        .chain(extra)
        .chain(paired.order_difference(path, ""))
        .chain(mixed)
        .chain(layout_difference(path, expected, actual))
        .chain(children)
        .collect()
}

/// The whitespace before each child element and before the end tag, keyed by what follows it,
/// so missing children do not also show up as layout noise.
fn layout(element: &Element) -> Vec<((&str, usize), String)> {
    let mut gaps = Vec::new();
    let mut gap = String::new();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &element.children {
        match node {
            Node::Text(t) if is_blank(t) => gap.push_str(&t.raw),
            Node::Text(_) => {}
            Node::Element(e) => {
                let count = seen.entry(e.name.as_str()).or_insert(0);
                gaps.push(((e.name.as_str(), *count), std::mem::take(&mut gap)));
                *count += 1;
            }
        }
    }
    gaps.push((("</end>", 0), gap));
    gaps
}

fn layout_difference(path: &str, expected: &Element, actual: &Element) -> Option<Difference> {
    let got = layout(actual);
    let (key, want_gap, got_gap) = layout(expected).into_iter().find_map(|(key, want_gap)| {
        let (_, got_gap) = got.iter().find(|(k, _)| *k == key)?;
        (*got_gap != want_gap).then(|| (key, want_gap, got_gap.clone()))
    })?;
    let place = match key {
        ("</end>", _) => format!("before </{}>", expected.name),
        (name, n) => format!("before <{name}> #{n}"),
    };
    let show = |gap: &str| format!("{place}: {gap:?}");
    Some(Difference::new(
        Category::Whitespace,
        path,
        show(&want_gap),
        show(&got_gap),
    ))
}

fn child_elements(element: &Element) -> impl Iterator<Item = &Element> {
    element.children.iter().filter_map(|node| match node {
        Node::Element(e) => Some(e),
        Node::Text(_) => None,
    })
}

fn has_elements(element: &Element) -> bool {
    child_elements(element).next().is_some()
}

fn is_blank(text: &Text) -> bool {
    text.raw.chars().all(char::is_whitespace)
}

fn joined_text(element: &Element, keep: impl Fn(&Text) -> bool) -> Text {
    element
        .children
        .iter()
        .filter_map(|node| match node {
            Node::Text(t) if keep(t) => Some(t),
            _ => None,
        })
        .fold(
            Text {
                raw: String::new(),
                value: String::new(),
            },
            |mut acc, t| {
                acc.raw.push_str(&t.raw);
                acc.value.push_str(&t.value);
                acc
            },
        )
}

/// A short XML-like preview of an element.
fn render(element: &Element) -> String {
    let attrs: String = element
        .attrs
        .iter()
        .flat_map(|a| [" ", &a.name, "=\"", &a.text.raw, "\""])
        .collect();
    let inner = if has_elements(element) {
        compact_names(child_elements(element).map(|e| e.name.clone()))
    } else {
        joined_text(element, |_| true).raw
    };
    format!("<{0}{attrs}>{inner}</{0}>", element.name)
}
