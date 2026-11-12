//! A small XML reader for Last.fm's documents that keeps what a byte-level comparison needs:
//! the declaration, attribute order and raw values, raw text with its entities, whitespace
//! between elements, and whether an empty element was self-closed.

const MAX_DEPTH: usize = 128;

/// Text as sent and decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Text {
    pub(super) raw: String,
    pub(super) value: String,
}

/// An attribute in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Attr {
    pub(super) name: String,
    pub(super) text: Text,
}

/// An element and everything inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Element {
    pub(super) name: String,
    pub(super) attrs: Vec<Attr>,
    pub(super) children: Vec<Node>,
    /// `<x/>` rather than `<x></x>`.
    pub(super) self_closing: bool,
}

/// Element content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Node {
    Element(Element),
    Text(Text),
}

/// A parsed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Document {
    /// `<?xml ... ?>` as sent.
    pub(super) declaration: Option<String>,
    /// Whitespace between the declaration and the root element.
    pub(super) prolog: String,
    pub(super) root: Element,
    /// Whitespace after the root element.
    pub(super) trailing: String,
}

/// Why a body is not XML.
#[derive(Debug, thiserror::Error)]
#[error("{message} at byte {offset}")]
pub(super) struct ParseError {
    message: &'static str,
    offset: usize,
}

/// Parses a whole body.
pub(super) fn parse(bytes: &[u8]) -> Result<Document, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ParseError {
        message: "invalid UTF-8",
        offset: e.valid_up_to(),
    })?;
    let mut parser = Parser { text, pos: 0 };
    let declaration = parser
        .rest()
        .starts_with("<?xml")
        .then(|| parser.through("?>"))
        .transpose()?;
    let prolog = parser.whitespace();
    if !parser.rest().starts_with('<') {
        return Err(parser.error("expected the root element"));
    }
    let root = parser.element(0)?;
    let trailing = parser.whitespace();
    if !parser.rest().is_empty() {
        return Err(parser.error("content after the root element"));
    }
    Ok(Document {
        declaration,
        prolog,
        root,
        trailing,
    })
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        self.text.get(self.pos..).unwrap_or_default()
    }

    fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            message,
            offset: self.pos,
        }
    }

    /// Consumes up to and including `end`, returning everything consumed.
    fn through(&mut self, end: &str) -> Result<String, ParseError> {
        let len = self
            .rest()
            .find(end)
            .ok_or_else(|| self.error("unterminated markup"))?;
        let taken = self
            .rest()
            .get(..len + end.len())
            .unwrap_or_default()
            .to_owned();
        self.pos += taken.len();
        Ok(taken)
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) -> String {
        let len = self.rest().find(|c| !keep(c)).unwrap_or(self.rest().len());
        let taken = self.rest().get(..len).unwrap_or_default().to_owned();
        self.pos += len;
        taken
    }

    fn whitespace(&mut self) -> String {
        self.take_while(char::is_whitespace)
    }

    fn eat(&mut self, token: &str) -> bool {
        let found = self.rest().starts_with(token);
        if found {
            self.pos += token.len();
        }
        found
    }

    fn name(&mut self) -> Result<String, ParseError> {
        let name = self.take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'));
        if name.is_empty() {
            Err(self.error("expected a name"))
        } else {
            Ok(name)
        }
    }

    fn element(&mut self, depth: usize) -> Result<Element, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        self.pos += 1;
        let name = self.name()?;
        let mut attrs = Vec::new();
        loop {
            self.whitespace();
            if self.eat("/>") {
                return Ok(Element {
                    name,
                    attrs,
                    children: Vec::new(),
                    self_closing: true,
                });
            }
            if self.eat(">") {
                break;
            }
            attrs.push(self.attribute()?);
        }
        let children = self.children(&name, depth)?;
        Ok(Element {
            name,
            attrs,
            children,
            self_closing: false,
        })
    }

    fn attribute(&mut self) -> Result<Attr, ParseError> {
        let name = self.name()?;
        self.whitespace();
        if !self.eat("=") {
            return Err(self.error("expected '='"));
        }
        self.whitespace();
        let Some(quote @ ('"' | '\'')) = self.rest().chars().next() else {
            return Err(self.error("expected a quoted value"));
        };
        self.pos += 1;
        let raw = self.take_while(|c| c != quote);
        if !self.eat(&quote.to_string()) {
            return Err(self.error("unterminated attribute value"));
        }
        let value = decode(&raw);
        Ok(Attr {
            name,
            text: Text { raw, value },
        })
    }

    fn children(&mut self, parent: &str, depth: usize) -> Result<Vec<Node>, ParseError> {
        let mut children = Vec::new();
        loop {
            if self.eat("</") {
                let name = self.name()?;
                self.whitespace();
                if name != parent || !self.eat(">") {
                    return Err(self.error("mismatched end tag"));
                }
                return Ok(children);
            }
            if self.rest().starts_with("<![CDATA[") {
                let raw = self.through("]]>")?;
                let value = raw
                    .get("<![CDATA[".len()..raw.len() - "]]>".len())
                    .unwrap_or_default()
                    .to_owned();
                children.push(Node::Text(Text { raw, value }));
            } else if self.rest().starts_with("<!--") {
                self.through("-->")?;
            } else if self.rest().starts_with('<') {
                children.push(Node::Element(self.element(depth + 1)?));
            } else if self.rest().is_empty() {
                return Err(self.error("unclosed element"));
            } else {
                let raw = self.take_while(|c| c != '<');
                let value = decode(&raw);
                children.push(Node::Text(Text { raw, value }));
            }
        }
    }
}

/// Decodes the five named entities and numeric references. Unknown entities stay as written.
fn decode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        let (before, from_amp) = rest.split_at(amp);
        out.push_str(before);
        let decoded = from_amp
            .find(';')
            .and_then(|semi| Some((entity(from_amp.get(1..semi)?)?, semi)));
        let (c, skip) = decoded.map_or(('&', 1), |(c, semi)| (c, semi + 1));
        out.push(c);
        rest = from_amp.get(skip..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}
