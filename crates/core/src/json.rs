//! JSON output in Last.fm's conventions: `"@attr"` for attributes of elements with children,
//! `"#text"` for text next to attributes, repeated children as arrays, no trailing newline.

use crate::error::ApiError;
use crate::tree::{Node, Response, Style, Text};

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

/// Appends `node` as a bare JSON value, without the `{"name": …}` envelope and with `/` left
/// unescaped: one line of an NDJSON export.
pub fn render_value(node: &Node, out: &mut Vec<u8>) {
    Writer {
        out,
        escape_slashes: false,
    }
    .node(node);
}

/// Appends an error response to `out`. The classic backend puts `message` before `error`; the
/// modern one puts `error` first and adds an empty `links` list.
pub fn render_error(error: &ApiError, style: Style, out: &mut Vec<u8>) {
    let mut writer = Writer {
        out,
        escape_slashes: false,
    };
    let code = error.code().number().to_string();
    match style {
        Style::Classic => {
            writer.out.extend_from_slice(b"{\"message\":");
            writer.string(error.message());
            writer.out.extend_from_slice(b",\"error\":");
            writer.out.extend_from_slice(code.as_bytes());
            writer.out.push(b'}');
        }
        Style::Modern => {
            writer.out.extend_from_slice(b"{\"error\":");
            writer.out.extend_from_slice(code.as_bytes());
            writer.out.extend_from_slice(b",\"message\":");
            writer.string(error.message());
            writer.out.extend_from_slice(b",\"links\":[]}");
        }
    }
}

enum Entry<'a> {
    /// Children named `name`; `first` is the index of the first one.
    Group {
        name: &'a str,
        array: bool,
        first: usize,
    },
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

/// Whether a JSON string must escape `byte`: controls, `"`, `\\`, and `/` where Last.fm writes
/// `\\/`.
fn needs_escape(byte: u8, slash: bool) -> bool {
    byte < 0x20 || byte == b'"' || byte == b'\\' || (slash && byte == b'/')
}

/// Name equality, checking the pointer first: names are mostly the same string literals.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && (std::ptr::eq(a.as_ptr(), b.as_ptr()) || a == b)
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
        } else if node.json_order.is_none() {
            // A leaf with attributes and no declared order, such as an image: attributes,
            // then the text.
            self.out.push(b'{');
            for (name, value) in &node.attrs {
                self.key(name);
                self.text(value);
                self.out.push(b',');
            }
            self.key("#text");
            self.leaf_text(node);
            self.out.push(b'}');
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
                Entry::Group { name, array, first } => self.group(node, name, *array, *first),
                Entry::AttrObject => self.attr_object(node),
                Entry::Attr(_, value) => self.text(value),
                Entry::Text => self.leaf_text(node),
            }
        }
        self.out.push(b'}');
    }

    fn leaf_text(&mut self, node: &Node) {
        match &node.text {
            Some(text) => self.text(text),
            None => self.out.extend_from_slice(b"\"\""),
        }
    }

    fn group(&mut self, node: &Node, name: &str, array: bool, first: usize) {
        let mut members = node
            .children
            .get(first..)
            .unwrap_or_default()
            .iter()
            .filter(|child| same(&child.name, name));
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
        let slash = self.escape_slashes;
        let bytes = s.as_bytes();
        self.out.reserve(bytes.len() + 2);
        self.out.push(b'"');
        let mut start = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if needs_escape(byte, slash) {
                self.out
                    .extend_from_slice(bytes.get(start..i).unwrap_or_default());
                self.escape(byte);
                start = i + 1;
            }
        }
        self.out
            .extend_from_slice(bytes.get(start..).unwrap_or_default());
        self.out.push(b'"');
    }

    fn escape(&mut self, byte: u8) {
        let escaped: &[u8] = match byte {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'/' => b"\\/",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0x08 => b"\\b",
            0x0c => b"\\f",
            _ => {
                self.out
                    .extend_from_slice(format!("\\u{byte:04x}").as_bytes());
                return;
            }
        };
        self.out.extend_from_slice(escaped);
    }
}

/// Child groups in order of first appearance, then `"@attr"` when there are attributes.
fn object_entries(node: &Node) -> Vec<Entry<'_>> {
    let mut entries: Vec<Entry<'_>> = Vec::with_capacity(node.children.len().min(16) + 1);
    for (index, child) in node.children.iter().enumerate() {
        // Repeated children are usually adjacent, so the last group is checked first.
        let group = entries
            .iter_mut()
            .rev()
            .find(|entry| same(entry.key(), &child.name));
        match group {
            Some(Entry::Group { array, .. }) => *array = true,
            _ => entries.push(Entry::Group {
                name: &child.name,
                array: false,
                first: index,
            }),
        }
    }
    for entry in &mut entries {
        if let Entry::Group { name, array, .. } = entry {
            *array |= node.arrays.iter().any(|declared| same(declared, name));
        }
    }
    for declared in &node.arrays {
        if !entries.iter().any(|e| same(e.key(), declared)) {
            entries.push(Entry::Group {
                name: declared,
                array: true,
                first: node.children.len(),
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
        entries.sort_by_cached_key(|entry| {
            order
                .iter()
                .position(|key| same(key, entry.key()))
                .unwrap_or(usize::MAX)
        });
    }
    entries
}
