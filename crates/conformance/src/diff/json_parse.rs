//! A JSON reader that keeps what a byte-level comparison needs and `serde_json` drops: key
//! order and duplicates, raw number text, raw string escapes, and whitespace.

const MAX_DEPTH: usize = 128;

/// A JSON string: the text between the quotes as sent, and its decoded value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Str {
    pub(super) raw: String,
    pub(super) value: String,
}

/// A JSON value with its source details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Value {
    Null,
    Bool(bool),
    /// The number exactly as written.
    Number(String),
    String(Str),
    Array(Vec<Value>),
    /// Members in source order, duplicates kept.
    Object(Vec<(Str, Value)>),
}

/// A parsed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Document {
    pub(super) root: Value,
    /// Each run of whitespace outside strings, with the number of tokens before it.
    pub(super) whitespace: Vec<(usize, String)>,
}

/// Why a body is not JSON.
#[derive(Debug, thiserror::Error)]
#[error("{message} at byte {offset}")]
pub(super) struct ParseError {
    message: &'static str,
    offset: usize,
}

/// Parses a whole body. Trailing characters other than whitespace are an error.
pub(super) fn parse(bytes: &[u8]) -> Result<Document, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ParseError {
        message: "invalid UTF-8",
        offset: e.valid_up_to(),
    })?;
    let mut parser = Parser {
        text,
        pos: 0,
        tokens: 0,
        whitespace: Vec::new(),
    };
    let root = parser.value(0)?;
    parser.skip_whitespace();
    if parser.pos != text.len() {
        return Err(parser.error("trailing characters"));
    }
    Ok(Document {
        root,
        whitespace: parser.whitespace,
    })
}

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    tokens: usize,
    whitespace: Vec<(usize, String)>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            message,
            offset: self.pos,
        }
    }

    fn expect(&mut self, byte: u8, message: &'static str) -> Result<(), ParseError> {
        self.skip_whitespace();
        if self.peek() == Some(byte) {
            self.pos += 1;
            self.tokens += 1;
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_whitespace();
        let found = self.peek() == Some(byte);
        if found {
            self.pos += 1;
            self.tokens += 1;
        }
        found
    }

    fn skip_whitespace(&mut self) {
        let start = self.pos;
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
        if let Some(run) = self.text.get(start..self.pos).filter(|r| !r.is_empty()) {
            self.whitespace.push((self.tokens, run.to_owned()));
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        self.skip_whitespace();
        self.tokens += 1;
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => Ok(self.number()),
            _ => Err(self.error("expected a value")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1;
        let mut members = Vec::new();
        if self.eat(b'}') {
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_whitespace();
            self.tokens += 1;
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a key"));
            }
            let key = self.string()?;
            self.expect(b':', "expected ':'")?;
            members.push((key, self.value(depth + 1)?));
            if !self.eat(b',') {
                self.expect(b'}', "expected ',' or '}'")?;
                return Ok(Value::Object(members));
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1;
        let mut items = Vec::new();
        if self.eat(b']') {
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            if !self.eat(b',') {
                self.expect(b']', "expected ',' or ']'")?;
                return Ok(Value::Array(items));
            }
        }
    }

    fn literal(&mut self, word: &'static str, value: Value) -> Result<Value, ParseError> {
        if self
            .text
            .get(self.pos..)
            .is_some_and(|t| t.starts_with(word))
        {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("unknown literal"))
        }
    }

    fn number(&mut self) -> Value {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.pos += 1;
        }
        Value::Number(
            self.text
                .get(start..self.pos)
                .unwrap_or_default()
                .to_owned(),
        )
    }

    fn string(&mut self) -> Result<Str, ParseError> {
        self.pos += 1;
        let start = self.pos;
        let mut value = String::new();
        loop {
            let run_start = self.pos;
            while !matches!(self.peek(), None | Some(b'"' | b'\\')) {
                self.pos += 1;
            }
            value.push_str(self.text.get(run_start..self.pos).unwrap_or_default());
            match self.bump() {
                Some(b'"') => break,
                Some(_) => value.push(self.escape()?),
                None => return Err(self.error("unterminated string")),
            }
        }
        let raw = self.text.get(start..self.pos - 1).unwrap_or_default();
        Ok(Str {
            raw: raw.to_owned(),
            value,
        })
    }

    fn escape(&mut self) -> Result<char, ParseError> {
        match self.bump() {
            Some(b'"') => Ok('"'),
            Some(b'\\') => Ok('\\'),
            Some(b'/') => Ok('/'),
            Some(b'b') => Ok('\u{8}'),
            Some(b'f') => Ok('\u{c}'),
            Some(b'n') => Ok('\n'),
            Some(b'r') => Ok('\r'),
            Some(b't') => Ok('\t'),
            Some(b'u') => self.unicode(),
            _ => Err(self.error("invalid escape")),
        }
    }

    fn unicode(&mut self) -> Result<char, ParseError> {
        let first = self.hex4()?;
        let is_high_surrogate = (0xD800..0xDC00).contains(&first);
        let low_follows = self
            .text
            .get(self.pos..)
            .is_some_and(|t| t.starts_with("\\u"));
        if is_high_surrogate && low_follows {
            self.pos += 2;
            let second = self.hex4()?;
            let code = 0x10000 + ((first - 0xD800) << 10) + second.saturating_sub(0xDC00);
            return Ok(char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        Ok(char::from_u32(first).unwrap_or(char::REPLACEMENT_CHARACTER))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let digits = self
            .text
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.error("short \\u escape"))?;
        let code = u32::from_str_radix(digits, 16).map_err(|_| self.error("bad \\u escape"))?;
        self.pos += 4;
        Ok(code)
    }
}
