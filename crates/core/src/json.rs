//! JSON output in Last.fm's conventions: `"@attr"` for attributes of elements with children,
//! `"#text"` for text next to attributes, repeated children as arrays, no trailing newline.

use crate::error::ApiError;
use crate::tree::{Node, Response, Text};

/// Appends a successful response to `out`.
pub fn render(response: &Response, out: &mut Vec<u8>) {
    let mut writer = Writer {
        out,
        escape_slashes: response.escape_slashes,
    };
    writer.out.push(b'{');
    if let Some(root) = &response.root {
        writer.key(&root.name);
        writer.node(root);
    }
    writer.out.push(b'}');
}

/// Appends an error response to `out`. Last.fm puts `message` before `error`.
pub fn render_error(error: &ApiError, out: &mut Vec<u8>) {
    let mut writer = Writer {
        out,
        escape_slashes: false,
    };
    writer.out.extend_from_slice(b"{\"message\":");
    writer.string(error.message());
    writer.out.extend_from_slice(b",\"error\":");
    writer
        .out
        .extend_from_slice(error.code().number().to_string().as_bytes());
    writer.out.push(b'}');
}

enum Entry<'a> {
    Group { name: &'a str, array: bool },
    AttrObject,
    Attr(&'a str, &'a Text),
    Text,
}

impl Entry<'_> {
    fn key(&self) -> &str {
        match self {
            Self::Group { name, .. } | Self::Attr(name, _) => name,
            Self::AttrObject => "@attr",
            Self::Text => "#text",
        }
    }
}

struct Writer<'a> {
    out: &'a mut Vec<u8>,
    escape_slashes: bool,
}

impl Writer<'_> {
    fn node(&mut self, node: &Node) {
        if node.is_object() {
            let entries = order(node, object_entries(node));
            self.entries(node, &entries);
        } else if node.attrs.is_empty() {
            match &node.text {
                Some(text) => self.text(text),
                None => self.out.extend_from_slice(b"\"\""),
            }
        } else {
            let mut entries: Vec<Entry<'_>> = node
                .attrs
                .iter()
                .map(|(name, value)| Entry::Attr(name, value))
                .collect();
            entries.push(Entry::Text);
            let entries = order(node, entries);
            self.entries(node, &entries);
        }
    }

    fn entries(&mut self, node: &Node, entries: &[Entry<'_>]) {
        self.out.push(b'{');
        for (i, entry) in entries.iter().enumerate() {
            if i > 0 {
                self.out.push(b',');
            }
            self.key(entry.key());
            match entry {
                Entry::Group { name, array } => self.group(node, name, *array),
                Entry::AttrObject => self.attr_object(node),
                Entry::Attr(_, value) => self.text(value),
                Entry::Text => match &node.text {
                    Some(text) => self.text(text),
                    None => self.out.extend_from_slice(b"\"\""),
                },
            }
        }
        self.out.push(b'}');
    }

    fn group(&mut self, node: &Node, name: &str, array: bool) {
        let mut members = node.children.iter().filter(|child| child.name == name);
        if !array {
            if let Some(only) = members.next() {
                self.node(only);
            }
            return;
        }
        self.out.push(b'[');
        for (i, member) in members.enumerate() {
            if i > 0 {
                self.out.push(b',');
            }
            self.node(member);
        }
        self.out.push(b']');
    }

    fn attr_object(&mut self, node: &Node) {
        let attrs: Vec<Entry<'_>> = node
            .attrs
            .iter()
            .map(|(name, value)| Entry::Attr(name, value))
            .collect();
        self.out.push(b'{');
        for (i, entry) in order(node, attrs).iter().enumerate() {
            if let Entry::Attr(name, value) = entry {
                if i > 0 {
                    self.out.push(b',');
                }
                self.key(name);
                self.text(value);
            }
        }
        self.out.push(b'}');
    }

    fn key(&mut self, key: &str) {
        self.string(key);
        self.out.push(b':');
    }

    fn text(&mut self, text: &Text) {
        match text {
            Text::Str(s) => self.string(s),
            Text::Num(n) => self.out.extend_from_slice(n.as_bytes()),
        }
    }

    fn string(&mut self, s: &str) {
        self.out.push(b'"');
        let mut start = 0;
        for (i, byte) in s.bytes().enumerate() {
            let escaped: &[u8] = match byte {
                b'"' => b"\\\"",
                b'\\' => b"\\\\",
                b'/' if self.escape_slashes => b"\\/",
                b'\n' => b"\\n",
                b'\r' => b"\\r",
                b'\t' => b"\\t",
                0x08 => b"\\b",
                0x0c => b"\\f",
                0..0x20 => {
                    self.out
                        .extend_from_slice(s.as_bytes().get(start..i).unwrap_or_default());
                    self.out
                        .extend_from_slice(format!("\\u{byte:04x}").as_bytes());
                    start = i + 1;
                    continue;
                }
                _ => continue,
            };
            self.out
                .extend_from_slice(s.as_bytes().get(start..i).unwrap_or_default());
            self.out.extend_from_slice(escaped);
            start = i + 1;
        }
        self.out
            .extend_from_slice(s.as_bytes().get(start..).unwrap_or_default());
        self.out.push(b'"');
    }
}

/// Child groups in order of first appearance, then `"@attr"` when there are attributes.
fn object_entries(node: &Node) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry<'_>> = Vec::new();
    for child in &node.children {
        let seen = entries.iter().any(|e| e.key() == child.name);
        if !seen {
            let repeats = node
                .children
                .iter()
                .filter(|c| c.name == child.name)
                .nth(1)
                .is_some();
            let declared = node.arrays.contains(&child.name);
            entries.push(Entry::Group {
                name: &child.name,
                array: repeats || declared,
            });
        }
    }
    for declared in &node.arrays {
        if !entries.iter().any(|e| e.key() == declared) {
            entries.push(Entry::Group {
                name: declared,
                array: true,
            });
        }
    }
    if !node.attrs.is_empty() {
        entries.push(Entry::AttrObject);
    }
    entries
}

fn order<'a>(node: &Node, mut entries: Vec<Entry<'a>>) -> Vec<Entry<'a>> {
    if let Some(order) = node.json_order {
        entries.sort_by_key(|entry| {
            order
                .iter()
                .position(|key| *key == entry.key())
                .unwrap_or(usize::MAX)
        });
    }
    entries
}
