//! The response tree that every method builds and both renderers read.

use std::borrow::Cow;

/// A string that is either `'static` or owned.
pub type Str = Cow<'static, str>;

/// The value of an attribute or of an element's text.
///
/// Last.fm renders almost every number as a JSON string, so integers convert to [`Text::Str`].
/// Use [`Text::number`] for the rare places where JSON shows a bare number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    /// Rendered as a JSON string.
    Str(Str),
    /// Rendered as a bare JSON number; holds only ASCII digits and an optional sign.
    Num(Str),
}

impl Text {
    /// A value that JSON renders as a bare number, such as `"#text":1628951940`.
    #[must_use]
    pub fn number(value: i64) -> Self {
        Self::Num(Cow::Owned(value.to_string()))
    }

    /// The value as it appears in XML.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Str(s) | Self::Num(s) => s,
        }
    }
}

impl From<&'static str> for Text {
    fn from(value: &'static str) -> Self {
        Self::Str(Cow::Borrowed(value))
    }
}

impl From<String> for Text {
    fn from(value: String) -> Self {
        Self::Str(Cow::Owned(value))
    }
}

impl From<Str> for Text {
    fn from(value: Str) -> Self {
        Self::Str(value)
    }
}

macro_rules! text_from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Text {
            fn from(value: $t) -> Self {
                Self::Str(Cow::Owned(value.to_string()))
            }
        }
    )*};
}

text_from_int!(u8, u16, u32, u64, usize, i32, i64);

/// One element of a response: name, attributes, text, children and JSON shaping hints.
///
/// The XML renderer writes children in insertion order. The JSON renderer groups children by
/// name, writes a group as an array when it repeats or is declared with [`Node::array`], and
/// puts attributes of an element with children under `"@attr"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub(crate) name: Str,
    pub(crate) attrs: Vec<(Str, Text)>,
    pub(crate) text: Option<Text>,
    pub(crate) children: Vec<Node>,
    pub(crate) arrays: Vec<Str>,
    pub(crate) json_order: Option<&'static [&'static str]>,
}

impl Node {
    /// An empty element.
    #[must_use]
    pub fn new(name: impl Into<Str>) -> Self {
        Self {
            name: name.into(),
            attrs: Vec::new(),
            text: None,
            children: Vec::new(),
            arrays: Vec::new(),
            json_order: None,
        }
    }

    /// An element that holds only text.
    #[must_use]
    pub fn leaf(name: impl Into<Str>, text: impl Into<Text>) -> Self {
        Self::new(name).text(text)
    }

    /// Adds an attribute.
    #[must_use]
    pub fn attr(mut self, name: impl Into<Str>, value: impl Into<Text>) -> Self {
        self.attrs.push((name.into(), value.into()));
        self
    }

    /// Sets the text. Ignored by both renderers when the element has children.
    #[must_use]
    pub fn text(mut self, text: impl Into<Text>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Appends a child.
    #[must_use]
    pub fn child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    /// Appends children. An `Option<Node>` works too, for elements that may be absent.
    #[must_use]
    pub fn children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }

    /// Renders children with this name as a JSON array even when there is one or none.
    ///
    /// Without this a single child collapses to an object, which some Last.fm methods do.
    #[must_use]
    pub fn array(mut self, child_name: impl Into<Str>) -> Self {
        self.arrays.push(child_name.into());
        self
    }

    /// Sets the JSON key order of this element, which Last.fm often scrambles.
    ///
    /// The list names children, attributes, `"@attr"` and `"#text"`. Keys that are not listed
    /// follow in their default order. The same list orders the keys inside `"@attr"`.
    #[must_use]
    pub fn json_order(mut self, order: &'static [&'static str]) -> Self {
        self.json_order = Some(order);
        self
    }

    /// The element name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value of an attribute.
    #[must_use]
    pub fn attr_value(&self, name: &str) -> Option<&Text> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// The element text.
    #[must_use]
    pub fn text_value(&self) -> Option<&Text> {
        self.text.as_ref()
    }

    /// The child elements in order.
    #[must_use]
    pub fn child_nodes(&self) -> &[Node] {
        &self.children
    }

    pub(crate) fn is_object(&self) -> bool {
        !self.children.is_empty() || !self.arrays.is_empty()
    }
}

/// Which of Last.fm's two backends a method's formatting copies; see `fixtures/NOTES.md`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Style {
    /// "Style A": indented XML, empty lists self-close, errors as `{"message","error"}`.
    #[default]
    Classic,
    /// "Style B": unindented XML, `?>` after a space, errors as `{"error","message","links"}`.
    Modern,
}

/// A successful response: the element inside `<lfm status="ok">` plus flavour settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub(crate) root: Option<Node>,
    pub(crate) escape_slashes: bool,
    pub(crate) style: Style,
}

impl Response {
    /// A response whose JSON leaves `/` unescaped.
    #[must_use]
    pub fn new(root: Node) -> Self {
        Self {
            root: Some(root),
            escape_slashes: false,
            style: Style::Classic,
        }
    }

    /// A response with an empty envelope, as write methods such as `track.love` send.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            root: None,
            escape_slashes: false,
            style: Style::Classic,
        }
    }

    /// Escapes `/` as `\/` in JSON strings.
    ///
    /// Last.fm does this for some methods (`user.getInfo`, `user.getRecentTracks`,
    /// `user.getTopArtists`) and not for others (`track.getInfo`, `user.getPersonalTags`).
    #[must_use]
    pub fn escape_slashes(mut self) -> Self {
        self.escape_slashes = true;
        self
    }

    /// Renders in the given backend's style.
    #[must_use]
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// The root element, absent for an empty envelope.
    #[must_use]
    pub fn root(&self) -> Option<&Node> {
        self.root.as_ref()
    }
}
