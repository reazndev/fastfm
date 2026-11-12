//! The `user.*` methods served from the store.

mod get_info;
mod get_loved_tracks;
mod get_recent_tracks;
mod get_top_albums;
mod get_top_artists;
mod get_top_tracks;
mod get_weekly_album_chart;
mod get_weekly_artist_chart;
mod get_weekly_chart_list;
mod get_weekly_track_chart;

pub use get_info::GetInfo;
pub use get_loved_tracks::GetLovedTracks;
pub use get_recent_tracks::GetRecentTracks;
pub use get_top_albums::GetTopAlbums;
pub use get_top_artists::GetTopArtists;
pub use get_top_tracks::GetTopTracks;
pub use get_weekly_album_chart::GetWeeklyAlbumChart;
pub use get_weekly_artist_chart::GetWeeklyArtistChart;
pub use get_weekly_chart_list::GetWeeklyChartList;
pub use get_weekly_track_chart::GetWeeklyTrackChart;
