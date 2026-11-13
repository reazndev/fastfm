//! Image lists built from a stored image file name such as `6415…5440.jpg`.

use crate::tree::Node;

const IMAGE_BASE: &str = "https://lastfm-img.freetls.fastly.net/i/u/";

/// One entry of Last.fm's `image` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageSize {
    /// 34 px.
    Small,
    /// 64 px.
    Medium,
    /// 174 px.
    Large,
    /// 300 px.
    ExtraLarge,
    /// 300 px again, in artist lists and `*.getInfo`.
    Mega,
    /// `size=""`, 300 px, in `album.getInfo`.
    Unnamed,
}

impl ImageSize {
    /// Small to extralarge, as in recent tracks.
    pub const STANDARD: &'static [ImageSize] =
        &[Self::Small, Self::Medium, Self::Large, Self::ExtraLarge];
    /// Small to mega, as in top artists.
    pub const WITH_MEGA: &'static [ImageSize] = &[
        Self::Small,
        Self::Medium,
        Self::Large,
        Self::ExtraLarge,
        Self::Mega,
    ];

    /// The `size` attribute.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::ExtraLarge => "extralarge",
            Self::Mega => "mega",
            Self::Unnamed => "",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Small => "34s",
            Self::Medium => "64s",
            Self::Large => "174s",
            Self::ExtraLarge | Self::Mega | Self::Unnamed => "300x300",
        }
    }
}

/// The URL of one size of an image file.
#[must_use]
pub fn image_url(file: &str, size: ImageSize) -> String {
    let path = size.path();
    let mut url = String::with_capacity(IMAGE_BASE.len() + path.len() + 1 + file.len());
    url.push_str(IMAGE_BASE);
    url.push_str(path);
    url.push('/');
    url.push_str(file);
    url
}

/// `<image size="…">url</image>` nodes; with no file each node has empty text, as Last.fm sends.
pub fn images<'a>(
    file: Option<&'a str>,
    sizes: &'a [ImageSize],
) -> impl Iterator<Item = Node> + 'a {
    sizes.iter().map(move |size| {
        let url = file.map(|f| image_url(f, *size)).unwrap_or_default();
        Node::leaf("image", url).attr("size", size.name())
    })
}
