//! The typed helpers enrichment uses: overall top lists, artist corrections, plain recent
//! tracks, and the `user.getInfo` extras.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "tests fail loudly on a broken expectation"
)]

#[cfg(test)]
mod support;

use lfm_upstream::{CallOptions, PageRequest, RecentTracksQuery};
use support::{fast_client, mount_json, seen};
use wiremock::MockServer;

const ATTR: &str =
    r#""@attr":{"user":"Alice","totalPages":"2","page":"1","perPage":"1000","total":"1001"}"#;

fn first_page() -> PageRequest {
    PageRequest {
        page: Some(1),
        limit: Some(1000),
    }
}

#[tokio::test]
async fn parses_overall_top_lists() {
    let server = MockServer::start().await;
    let artists = format!(
        r#"{{"topartists":{{"artist":[{{"streamable":"0","image":[],"mbid":"m1",
        "url":"https://www.last.fm/music/ZZZ","playcount":"3719","@attr":{{"rank":"1"}},
        "name":"ZZZ"}}],{ATTR}}}}}"#
    );
    let albums = format!(
        r##"{{"topalbums":{{"album":{{"artist":{{"url":"u","name":"Various Artists","mbid":""}},
        "image":[{{"size":"small","#text":""}}],"mbid":"","url":"a","playcount":"221",
        "@attr":{{"rank":"1"}},"name":"Spider-Verse"}},{ATTR}}}}}"##
    );
    let tracks = format!(
        r##"{{"toptracks":{{"track":[{{"streamable":{{"fulltrack":"0","#text":"0"}},"mbid":"",
        "name":"Doom","image":[],"artist":{{"url":"u","name":"Juice WRLD","mbid":"m2"}},
        "url":"t","duration":"193","@attr":{{"rank":"1"}},"playcount":"7462"}}],{ATTR}}}}}"##
    );
    mount_json(&server, "user.getTopArtists", &artists).await;
    mount_json(&server, "user.getTopAlbums", &albums).await;
    mount_json(&server, "user.getTopTracks", &tracks).await;
    let client = fast_client(&server);
    let options = CallOptions::default();

    let artists = client
        .top_artists("alice", first_page(), &options)
        .await
        .unwrap();
    assert_eq!(artists.page.total_pages, 2);
    let zzz = &artists.items[0];
    assert_eq!((zzz.name.as_str(), zzz.playcount), ("ZZZ", 3719));
    assert_eq!(zzz.mbid.as_deref(), Some("m1"));
    // A one-item list collapses to an object.
    let albums = client
        .top_albums("alice", first_page(), &options)
        .await
        .unwrap();
    assert_eq!(albums.items[0].artist.name, "Various Artists");
    assert_eq!(albums.items[0].artist.mbid, None);
    let tracks = client
        .top_tracks("alice", first_page(), &options)
        .await
        .unwrap();
    assert_eq!(
        (tracks.items[0].duration, tracks.items[0].playcount),
        (193, 7462)
    );
    assert_eq!(tracks.items[0].artist.mbid.as_deref(), Some("m2"));

    assert_eq!(seen(&server, "period").await, ["overall"; 3]);
    assert_eq!(seen(&server, "limit").await, ["1000"; 3]);
}

#[tokio::test]
async fn reads_artist_corrections() {
    let server = MockServer::start().await;
    let corrected = r#"{"corrections":{"correction":{"artist":{"name":"Travi$ Scott",
        "url":"https://www.last.fm/music/Travi$+Scott"},"@attr":{"index":"0"}}}}"#;
    mount_json(&server, "artist.getCorrection", corrected).await;
    let client = fast_client(&server);
    let found = client
        .artist_correction("Travis Scott", &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(found.unwrap().name, "Travi$ Scott");
    assert_eq!(seen(&server, "artist").await, ["Travis Scott"]);
}

#[tokio::test]
async fn an_unknown_artist_has_no_correction() {
    let server = MockServer::start().await;
    let unknown = r#"{"error":6,"message":"The artist you supplied could not be found",
        "links":[]}"#;
    mount_json(&server, "artist.getCorrection", unknown).await;
    let found = fast_client(&server)
        .artist_correction("xqz", &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(found, None);
}

#[tokio::test]
async fn plain_recent_tracks_keep_the_artist_mbid() {
    let server = MockServer::start().await;
    let body = format!(
        r##"{{"recenttracks":{{"track":[{{"artist":{{"mbid":"m3","#text":"Ledbyher"}},
        "streamable":"0","image":[],"mbid":"","album":{{"mbid":"","#text":"ABSURD"}},
        "name":"ABSURD","url":"https://www.last.fm/music/Ledbyher/_/ABSURD",
        "date":{{"uts":"1794516399","#text":"x"}}}}],{ATTR}}}}}"##
    );
    mount_json(&server, "user.getRecentTracks", &body).await;
    let query = RecentTracksQuery::new("alice").extended(false);
    let recent = fast_client(&server)
        .recent_tracks(&query, &CallOptions::default())
        .await
        .unwrap();
    let artist = &recent.scrobbles[0].track.artist;
    assert_eq!(
        (artist.name.as_str(), artist.mbid.as_deref()),
        ("Ledbyher", Some("m3"))
    );
    assert_eq!(artist.url, None);
    assert_eq!(seen(&server, "extended").await, ["0"]);
}

#[tokio::test]
async fn user_info_carries_corrections_and_spotify_expiry() {
    let server = MockServer::start().await;
    let body = r##"{"user":{"spotify_expiry_estimate":{"unixtime":"1802563200",
        "#text":1802563200},"playcount":"160643","name":"Syntiiix","corrections":"1",
        "registered":{"unixtime":"1628951940","#text":1628951940},"type":"user"}}"##;
    mount_json(&server, "user.getInfo", body).await;
    let user = fast_client(&server)
        .user_info("syntiiix", &CallOptions::default())
        .await
        .unwrap();
    assert_eq!((user.corrections, user.spotify_expiry), (1, 1_802_563_200));
}
