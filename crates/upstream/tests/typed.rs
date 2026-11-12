//! The typed helpers parse Last.fm's JSON, including its quirks.

#[cfg(test)]
mod support;

use lfm_upstream::{CallOptions, PageRequest, RecentTracksQuery};
use support::{fast_client, mount_json, seen};
use wiremock::MockServer;

const ATTR: &str =
    r#""@attr":{"user":"Alice","totalPages":"3","page":"1","perPage":"2","total":"5"}"#;

fn recent_track(name: &str, extra: &str) -> String {
    format!(
        r##"{{"artist":{{"url":"https://www.last.fm/music/Cher","name":"Cher","image":[],
        "mbid":"bfcc6d75-a6a5-4bc6-8282-47aec8531818"}},{extra}"mbid":"","name":"{name}",
        "image":[{{"size":"small","#text":"https://img/34s/x.jpg"}},
        {{"size":"extralarge","#text":"https://img/300x300/x.jpg"}}],"streamable":"0",
        "album":{{"mbid":"abc","#text":"Believe"}},"url":"https://www.last.fm/music/Cher/_/{name}",
        "loved":"1"}}"##
    )
}

#[tokio::test]
async fn parses_recent_tracks_with_now_playing() {
    let server = MockServer::start().await;
    let now = recent_track("Now", r#""@attr":{"nowplaying":"true"},"#);
    let first = recent_track("Believe", r##""date":{"uts":"1700000100","#text":"x"},"##);
    let second = recent_track("Strong", r##""date":{"uts":"1700000000","#text":"x"},"##);
    let body = format!(r#"{{"recenttracks":{{"track":[{now},{first},{second}],{ATTR}}}}}"#);
    mount_json(&server, "user.getRecentTracks", &body).await;
    let query = RecentTracksQuery::new("alice")
        .limit(2)
        .page(1)
        .from(10)
        .to(20);
    let recent = fast_client(&server)
        .recent_tracks(&query, &CallOptions::default())
        .await
        .unwrap();

    let now = recent.now_playing.unwrap();
    assert_eq!(now.name, "Now");
    assert_eq!(recent.scrobbles.len(), 2);
    let scrobble = &recent.scrobbles[0];
    assert_eq!(scrobble.uts, 1_700_000_100);
    assert_eq!(scrobble.track.artist.name, "Cher");
    assert_eq!(
        scrobble.track.artist.mbid.as_deref(),
        Some("bfcc6d75-a6a5-4bc6-8282-47aec8531818")
    );
    assert_eq!(
        scrobble.track.artist.url.as_deref(),
        Some("https://www.last.fm/music/Cher")
    );
    assert_eq!(scrobble.track.mbid, None);
    assert_eq!(scrobble.track.album.name, "Believe");
    assert_eq!(scrobble.track.album.mbid.as_deref(), Some("abc"));
    assert_eq!(scrobble.track.images.len(), 2);
    assert_eq!(scrobble.track.images[1].size, "extralarge");
    assert!(scrobble.track.loved);
    assert_eq!(recent.user, "Alice");
    assert_eq!(recent.page.total, 5);
    assert_eq!(recent.page.total_pages, 3);
    assert_eq!(recent.page.per_page, 2);

    assert_eq!(seen(&server, "extended").await, ["1"]);
    assert_eq!(seen(&server, "from").await, ["10"]);
    assert_eq!(seen(&server, "to").await, ["20"]);
    assert_eq!(seen(&server, "format").await, ["json"]);
}

#[tokio::test]
async fn accepts_a_single_track_as_an_object() {
    let server = MockServer::start().await;
    let only = recent_track("Believe", r##""date":{"uts":"1700000100","#text":"x"},"##);
    let body = format!(r#"{{"recenttracks":{{"track":{only},{ATTR}}}}}"#);
    mount_json(&server, "user.getRecentTracks", &body).await;
    let recent = fast_client(&server)
        .recent_tracks(&RecentTracksQuery::new("alice"), &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(recent.scrobbles.len(), 1);
    assert!(recent.now_playing.is_none());
}

#[tokio::test]
async fn accepts_an_empty_page() {
    let server = MockServer::start().await;
    let body = format!(r#"{{"recenttracks":{{"track":[],{ATTR}}}}}"#);
    mount_json(&server, "user.getRecentTracks", &body).await;
    let recent = fast_client(&server)
        .recent_tracks(&RecentTracksQuery::new("alice"), &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(recent.scrobbles, [] as [lfm_upstream::Scrobble; 0]);
}

#[tokio::test]
async fn parses_user_info() {
    let server = MockServer::start().await;
    let body = r##"{"user":{"name":"Alice","realname":"A","url":"https://www.last.fm/user/Alice",
        "country":"Germany","playcount":"160643","artist_count":"4046","album_count":"6198",
        "track_count":"15477","subscriber":"0","type":"user",
        "image":[{"size":"small","#text":""}],
        "registered":{"unixtime":"1628951940","#text":1628951940}}}"##;
    mount_json(&server, "user.getInfo", body).await;
    let user = fast_client(&server)
        .user_info("alice", &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(user.playcount, 160_643);
    assert_eq!(user.artist_count, 4046);
    assert_eq!(user.registered, 1_628_951_940);
    assert_eq!(user.kind, "user");
    assert!(!user.subscriber);
}

#[tokio::test]
async fn parses_loved_tracks() {
    let server = MockServer::start().await;
    let body = format!(
        r##"{{"lovedtracks":{{"track":{{"artist":{{"url":"u","name":"Cher","mbid":""}},
        "date":{{"uts":"1751828106","#text":"x"}},"mbid":"","url":"t","name":"Believe",
        "image":[],"streamable":{{"fulltrack":"0","#text":"0"}}}},{ATTR}}}}}"##
    );
    mount_json(&server, "user.getLovedTracks", &body).await;
    let page = PageRequest {
        page: Some(2),
        limit: Some(1),
    };
    let loved = fast_client(&server)
        .loved_tracks("alice", page, &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(loved.tracks.len(), 1);
    assert_eq!(loved.tracks[0].uts, 1_751_828_106);
    assert_eq!(loved.tracks[0].artist.mbid, None);
    assert_eq!(seen(&server, "page").await, ["2"]);
    assert_eq!(seen(&server, "limit").await, ["1"]);
}

#[tokio::test]
async fn parses_weekly_chart_list() {
    let server = MockServer::start().await;
    let body = r##"{"weeklychartlist":{"chart":[
        {"#text":"","from":"1108296000","to":"1108900800"},
        {"#text":"","from":"1108900800","to":"1109505600"}],"@attr":{"user":"Alice"}}}"##;
    mount_json(&server, "user.getWeeklyChartList", body).await;
    let charts = fast_client(&server)
        .weekly_chart_list("alice", &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(charts.len(), 2);
    assert_eq!(charts[1].from, 1_108_900_800);
    assert_eq!(charts[1].to, 1_109_505_600);
}

#[tokio::test]
async fn parses_friends() {
    let server = MockServer::start().await;
    let body = format!(
        r##"{{"friends":{{"user":[{{"name":"RJ","url":"https://www.last.fm/user/RJ",
        "country":"United Kingdom","playcount":"0","subscriber":"1","type":"alum",
        "registered":{{"unixtime":"1037793040","#text":"2002-11-20 11:50"}},
        "realname":"Richard Jones ","image":[]}}],{ATTR}}}}}"##
    );
    mount_json(&server, "user.getFriends", &body).await;
    let friends = fast_client(&server)
        .friends("alice", PageRequest::default(), &CallOptions::default())
        .await
        .unwrap();
    assert_eq!(friends.users.len(), 1);
    assert_eq!(friends.users[0].registered, 1_037_793_040);
    assert!(friends.users[0].subscriber);
    assert_eq!(friends.page.total, 5);
}
