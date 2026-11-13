//! Behaviour the local methods share: finding the user, list envelopes, and entity nodes.

mod list;
mod nodes;
mod now_playing;
mod top;
mod user;

pub(crate) use list::{
    ListAttrs, WeeklyRange, chart_root, history_range, list_root, list_root_collapsing, paging,
    weekly_range,
};
pub(crate) use nodes::{
    PLACEHOLDER, album_url, artist_mbid_attr, artist_of, artist_url, mbid, named_artist,
    streamable_full, track_url,
};
pub(crate) use now_playing::now_playing;
pub(crate) use top::{ListShape, chart_list, top_list, top_page};
pub(crate) use user::{OrFailed, cache_user, mirrored_user};
