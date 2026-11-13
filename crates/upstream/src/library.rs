//! Typed helpers for the overall top lists and artist corrections that enrichment reads.

use crate::client::Client;
use crate::error::{Error, Result};
use crate::methods::{CallOptions, PageRequest};
use crate::model::library::{ArtistCorrectionBody, TopAlbumsBody, TopArtistsBody, TopTracksBody};
use crate::model::{ArtistRef, TopAlbum, TopArtist, TopPage, TopTrack};
use crate::request::RawRequest;

/// Last.fm's code for an artist or track it does not know.
const NOT_FOUND: u32 = 6;

impl Client {
    /// One page of the user's all-time `user.getTopArtists`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn top_artists(
        &self,
        user: &str,
        page: PageRequest,
        options: &CallOptions,
    ) -> Result<TopPage<TopArtist>> {
        let request = overall("user.getTopArtists", user, page);
        self.typed::<TopArtistsBody, _>(options.apply(request))
            .await
    }

    /// One page of the user's all-time `user.getTopAlbums`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn top_albums(
        &self,
        user: &str,
        page: PageRequest,
        options: &CallOptions,
    ) -> Result<TopPage<TopAlbum>> {
        let request = overall("user.getTopAlbums", user, page);
        self.typed::<TopAlbumsBody, _>(options.apply(request)).await
    }

    /// One page of the user's all-time `user.getTopTracks`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body, or a transport or parse error.
    pub async fn top_tracks(
        &self,
        user: &str,
        page: PageRequest,
        options: &CallOptions,
    ) -> Result<TopPage<TopTrack>> {
        let request = overall("user.getTopTracks", user, page);
        self.typed::<TopTracksBody, _>(options.apply(request)).await
    }

    /// `artist.getCorrection`: the artist Last.fm files `artist` under, which is often the
    /// artist itself. `None` when Last.fm does not know the name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LastFm`] for a Last.fm error body other than "not found", or a
    /// transport or parse error.
    pub async fn artist_correction(
        &self,
        artist: &str,
        options: &CallOptions,
    ) -> Result<Option<ArtistRef>> {
        let request = RawRequest::new("artist.getCorrection").param("artist", artist);
        let found = self
            .typed::<ArtistCorrectionBody, Option<ArtistRef>>(options.apply(request))
            .await;
        match found {
            Err(Error::LastFm { code, .. }) if code == NOT_FOUND => Ok(None),
            other => other,
        }
    }
}

fn overall(method: &'static str, user: &str, page: PageRequest) -> RawRequest {
    RawRequest::new(method)
        .param("user", user)
        .param("period", "overall")
        .params(page.params())
}
