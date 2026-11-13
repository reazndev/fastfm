//! Serving a request from Last.fm through the read-through cache.

use lfm_core::{ApiError, ErrorCode, Output, Params};

use crate::dispatch::Dispatcher;
use crate::passthrough::{Forward, Forwarded, Overlay, overlay_lookup, overlay_user};
use crate::reply::{CLASSIC, Reply, error_reply};

impl Dispatcher {
    /// Last.fm's answer to `params`, replayed byte for byte, with the user's own counts filled
    /// in on `getInfo` when the user is mirrored here.
    pub(crate) async fn forward(
        &self,
        method: &'static Forwarded,
        params: &Params,
        output: &Output,
    ) -> Reply {
        let Some(passthrough) = &self.passthrough else {
            return error_reply(output, &ErrorCode::InvalidMethod.into(), CLASSIC);
        };
        let request = Forward::new(method, params, output.format());
        let (entry, source) = match passthrough.get(&request).await {
            Ok(found) => found,
            Err(failure) => return error_reply(output, &ApiError::from(failure), CLASSIC),
        };
        let body = match self.overlay(method, params).await {
            Some(overlay) if entry.status == 200 => overlay.apply(&entry.body, output.format()),
            _ => entry.body.clone(),
        };
        Reply::replay(&entry, body, output, source)
    }

    /// The user's play count and loved flag for a `getInfo` entity, when the request names a
    /// user this instance mirrors and the store knows the entity. Otherwise Last.fm's values
    /// stand.
    async fn overlay(&self, method: &Forwarded, params: &Params) -> Option<Overlay> {
        let lookup = overlay_lookup(method.name, params)?;
        let store = &self.services().store;
        let user = store.user_by_name(overlay_user(params)?).await.ok()??;
        let plays = store
            .user_plays(user.id, lookup)
            .await
            .inspect_err(|error| tracing::warn!(%error, "reading user plays failed"))
            .ok()?;
        plays.id?;
        Some(Overlay {
            plays: plays.plays,
            loved: plays.loved,
        })
    }
}
