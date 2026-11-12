//! `user.getInfo`: the user's profile and counts.

use async_trait::async_trait;
use lfm_core::{ApiError, ImageSize, Node, Params, Response, Text, images, user_url};
use lfm_store::MirroredUser;

use crate::method::{Context, Method};
use crate::methods::shared::{cache_user, mirrored_user};

/// `user.getInfo`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetInfo;

#[async_trait]
impl Method for GetInfo {
    fn name(&self) -> &'static str {
        "user.getInfo"
    }

    fn cache_user<'p>(&self, params: &'p Params) -> Option<&'p str> {
        cache_user(params)
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let user = mirrored_user(ctx)?;
        Ok(Response::new(user_node(user)).escape_slashes())
    }
}

fn user_node(user: &MirroredUser) -> Node {
    let profile = &user.profile;
    let url = Some(profile.url.clone())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| user_url(&user.name));
    Node::new("user")
        .json_order(&[
            "spotify_expiry_estimate",
            "playcount",
            "playlists",
            "album_count",
            "bootstrap",
            "url",
            "age",
            "name",
            "artist_count",
            "subscriber",
            "track_count",
            "realname",
            "image",
            "registered",
            "country",
            "gender",
            "corrections",
            "type",
        ])
        .child(Node::leaf("name", user.name.clone()))
        .child(Node::leaf("realname", profile.realname.clone()))
        .children(images(profile.image.as_deref(), ImageSize::STANDARD))
        .child(Node::leaf("url", url))
        .child(Node::leaf("country", profile.country.clone()))
        .child(Node::leaf("age", profile.age))
        .child(Node::leaf("gender", profile.gender.clone()))
        .child(Node::leaf("subscriber", u8::from(profile.subscriber)))
        .child(Node::leaf("playcount", user.stats.playcount))
        .child(Node::leaf("playlists", profile.playlists))
        .child(Node::leaf("bootstrap", profile.bootstrap))
        .child(unix_time("registered", profile.registered))
        .child(Node::leaf("type", profile.user_type.clone()))
        .child(Node::leaf("artist_count", user.stats.artists))
        .child(Node::leaf("album_count", user.stats.albums))
        .child(Node::leaf("track_count", user.stats.tracks))
        .child(Node::leaf("corrections", user.extras.corrections))
        .child(unix_time(
            "spotify_expiry_estimate",
            user.extras.spotify_expiry,
        ))
}

/// `<registered unixtime="…">…</registered>`; JSON shows the text as a bare number and the
/// attribute as a string.
fn unix_time(name: &'static str, uts: i64) -> Node {
    Node::leaf(name, Text::number(uts)).attr("unixtime", uts)
}
