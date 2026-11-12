//! XML output in Last.fm's layout: an `<lfm>` envelope, two-space indentation, no
//! self-closing tags, and a trailing newline.

use crate::error::ApiError;
use crate::tree::{Node, Response};

const DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";

/// Appends a successful response to `out`.
pub fn render(response: &Response, out: &mut Vec<u8>) {
    out.extend_from_slice(DECLARATION.as_bytes());
    out.extend_from_slice(b"<lfm status=\"ok\">\n");
    if let Some(root) = &response.root {
        write_node(root, 1, out);
    }
    out.extend_from_slice(b"</lfm>\n");
}

/// Appends an error response to `out`.
pub fn render_error(error: &ApiError, out: &mut Vec<u8>) {
    out.extend_from_slice(DECLARATION.as_bytes());
    out.extend_from_slice(b"<lfm status=\"failed\">\n  <error code=\"");
    out.extend_from_slice(error.code().number().to_string().as_bytes());
    out.extend_from_slice(b"\">");
    escape(error.message(), out);
    out.extend_from_slice(b"</error>\n</lfm>\n");
}

fn write_node(node: &Node, depth: usize, out: &mut Vec<u8>) {
    indent(depth, out);
    out.push(b'<');
    out.extend_from_slice(node.name.as_bytes());
    for (name, value) in &node.attrs {
        out.push(b' ');
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"=\"");
        escape(value.as_str(), out);
        out.push(b'"');
    }
    out.push(b'>');
    if node.children.is_empty() {
        if let Some(text) = &node.text {
            escape(text.as_str(), out);
        }
    } else {
        out.push(b'\n');
        for child in &node.children {
            write_node(child, depth + 1, out);
        }
        indent(depth, out);
    }
    out.extend_from_slice(b"</");
    out.extend_from_slice(node.name.as_bytes());
    out.extend_from_slice(b">\n");
}

fn indent(depth: usize, out: &mut Vec<u8>) {
    out.resize(out.len() + depth * 2, b' ');
}

/// Escapes the five XML special characters. Last.fm writes `'` as `&apos;`.
fn escape(text: &str, out: &mut Vec<u8>) {
    let mut start = 0;
    for (i, byte) in text.bytes().enumerate() {
        let entity: &[u8] = match byte {
            b'&' => b"&amp;",
            b'<' => b"&lt;",
            b'>' => b"&gt;",
            b'"' => b"&quot;",
            b'\'' => b"&apos;",
            _ => continue,
        };
        out.extend_from_slice(text.as_bytes().get(start..i).unwrap_or_default());
        out.extend_from_slice(entity);
        start = i + 1;
    }
    out.extend_from_slice(text.as_bytes().get(start..).unwrap_or_default());
}
