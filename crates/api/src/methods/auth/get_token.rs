//! `auth.getToken`: a token for the user to approve on this host's `/api/auth/` page.

use async_trait::async_trait;
use lfm_core::{ApiError, Node, Response};
use lfm_store::AuthToken;

use super::{TOKEN_LIFETIME, api_key, now, random_token};
use crate::method::{Context, Method};
use crate::methods::shared::OrFailed;

/// `<token>…</token>`: 32 characters, valid for an hour. Works unsigned, as on Last.fm.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetToken;

#[async_trait]
impl Method for GetToken {
    fn name(&self) -> &'static str {
        "auth.getToken"
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let token = AuthToken {
            token: random_token()?,
            api_key: api_key(ctx)?.to_owned(),
            account: None,
            expires_at: now(ctx).saturating_add(TOKEN_LIFETIME),
        };
        ctx.services
            .store
            .create_auth_token(token.clone(), now(ctx))
            .await
            .or_failed()?;
        Ok(Response::new(Node::leaf("token", token.token)))
    }
}
