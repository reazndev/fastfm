//! Last.fm response model and shared helpers, with no I/O.
//!
//! Methods build a [`Node`] tree, wrap it in a [`Response`], and an [`Output`] renders it as
//! Last.fm's XML, JSON or JSONP. Errors are [`ApiError`] values rendered the same way.

pub mod date;
pub mod error;
pub mod image;
pub mod json;
pub mod output;
pub mod paging;
pub mod params;
pub mod period;
pub mod signature;
pub mod tree;
pub mod url;
pub mod xml;

pub use date::{date_node, date_text};
pub use error::{ApiError, ErrorCode};
pub use image::{ImageSize, image_url, images};
pub use output::{Format, Output};
pub use paging::{PageSpec, Paging};
pub use params::Params;
pub use period::{Period, TimeRange};
pub use signature::{api_signature, verify_signature};
pub use tree::{Node, Response, Str, Style, Text};
pub use url::{album_url, artist_url, track_url, user_url};
