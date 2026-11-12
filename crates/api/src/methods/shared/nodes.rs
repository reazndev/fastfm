//! Pieces of artist, album and track nodes that several methods share.

use lfm_core::Node;
use lfm_store::{Entity, Named};

/// Last.fm's "no image" picture. Artist and track images in user lists are always this one.
pub(crate) const PLACEHOLDER: &str = "2a96cbd8b46e442fc41c2b86b821562f.png";

/// Stands in for the artist of an album or track the store returned without one.
static NO_ARTIST: Named = Named {
    name: String::new(),
    mbid: None,
    url: None,
};

/// The artist of an album or track.
pub(crate) fn artist_of(entity: &Entity) -> &Named {
    entity.artist.as_ref().unwrap_or(&NO_ARTIST)
}

/// An MBID, or the empty string Last.fm sends when there is none.
pub(crate) fn mbid(value: Option<&String>) -> String {
    value.cloned().unwrap_or_default()
}

/// An artist's Last.fm URL as stored, or built from the name.
pub(crate) fn artist_url(name: &str, url: Option<&String>) -> String {
    url.cloned().unwrap_or_else(|| lfm_core::artist_url(name))
}

/// The track's Last.fm URL as stored, or built from the names.
pub(crate) fn track_url(track: &Entity) -> String {
    track
        .url
        .clone()
        .unwrap_or_else(|| lfm_core::track_url(&artist_of(track).name, &track.name))
}

/// The album's Last.fm URL, built from the names.
pub(crate) fn album_url(album: &Entity) -> String {
    lfm_core::album_url(&artist_of(album).name, &album.name)
}

/// `<artist mbid="…">Name</artist>`, in JSON `{"mbid":"…","#text":"Name"}`.
pub(crate) fn artist_mbid_attr(artist: &Named) -> Node {
    Node::leaf("artist", artist.name.clone()).attr("mbid", mbid(artist.mbid.as_ref()))
}

/// `<artist>` with `name`, `mbid` and `url`, in JSON ordered `url`, `name`, `mbid`.
pub(crate) fn named_artist(artist: &Named) -> Node {
    Node::new("artist")
        .json_order(&["url", "name", "mbid"])
        .child(Node::leaf("name", artist.name.clone()))
        .child(Node::leaf("mbid", mbid(artist.mbid.as_ref())))
        .child(Node::leaf(
            "url",
            artist_url(&artist.name, artist.url.as_ref()),
        ))
}

/// `<streamable fulltrack="0">0</streamable>`, as top and loved tracks send it.
pub(crate) fn streamable_full() -> Node {
    Node::leaf("streamable", "0").attr("fulltrack", "0")
}
