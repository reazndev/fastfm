//! XML output in Last.fm's two layouts, each with an `<lfm>` envelope and a trailing newline.
//!
//! Classic: two-space indentation, one element per line, an empty list self-closes.
//! Modern: no indentation, the root starts on the `<lfm>` line, every closing tag ends a line.

use crate::error::ApiError;
use crate::tree::{Node, Response, Style};

const CLASSIC_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
const MODERN_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n";

/// Appends a successful response to `out`.
pub fn render(response: &Response, out: &mut Vec<u8>) {
    match response.style {
        Style::Classic => {
            out.extend_from_slice(CLASSIC_DECLARATION.as_bytes());
            out.extend_from_slice(b"<lfm status=\"ok\">\n");
            if let Some(root) = &response.root {
                write_classic(root, 1, out);
            }
        }
        Style::Modern => {
            out.extend_from_slice(MODERN_DECLARATION.as_bytes());
            out.extend_from_slice(b"<lfm status=\"ok\">");
            if let Some(root) = &response.root {
                write_modern(root, out);
            }
        }
    }
    out.extend_from_slice(b"</lfm>\n");
}

/// Appends an error response in the given style to `out`.
pub fn render_error(error: &ApiError, style: Style, out: &mut Vec<u8>) {
    match style {
        Style::Classic => {
            out.extend_from_slice(CLASSIC_DECLARATION.as_bytes());
            out.extend_from_slice(b"<lfm status=\"failed\">\n  ");
        }
        Style::Modern => {
            out.extend_from_slice(MODERN_DECLARATION.as_bytes());
            out.extend_from_slice(b"<lfm status=\"failed\">");
        }
    }
    out.extend_from_slice(b"<error code=\"");
    out.extend_from_slice(error.code().number().to_string().as_bytes());
    out.extend_from_slice(b"\">");
    escape(error.message(), out);
    out.extend_from_slice(b"</error>\n</lfm>\n");
}

fn write_classic(node: &Node, depth: usize, out: &mut Vec<u8>) {
    indent(depth, out);
    open_tag(node, out);
    if node.children.is_empty() && node.text.is_none() && !node.arrays.is_empty() {
        // An empty list, such as a page past the end: `<recenttracks user="…" … />`.
        out.extend_from_slice(b" />\n");
        return;
    }
    out.push(b'>');
    if node.children.is_empty() {
        if let Some(text) = &node.text {
            escape(text.as_str(), out);
        }
    } else {
        out.push(b'\n');
        for child in &node.children {
            write_classic(child, depth + 1, out);
        }
        indent(depth, out);
    }
    close_tag(node, out);
}

fn write_modern(node: &Node, out: &mut Vec<u8>) {
    open_tag(node, out);
    out.push(b'>');
    if node.children.is_empty() {
        if let Some(text) = &node.text {
            escape(text.as_str(), out);
        }
    } else {
        for child in &node.children {
            write_modern(child, out);
        }
    }
    close_tag(node, out);
}

fn open_tag(node: &Node, out: &mut Vec<u8>) {
    out.push(b'<');
    out.extend_from_slice(node.name.as_bytes());
    for (name, value) in &node.attrs {
        out.push(b' ');
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"=\"");
        escape(value.as_str(), out);
        out.push(b'"');
    }
}

fn close_tag(node: &Node, out: &mut Vec<u8>) {
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
