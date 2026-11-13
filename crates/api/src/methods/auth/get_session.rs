//! `auth.getSession`: trades an approved token for a session key.

use async_trait::async_trait;
use lfm_core::{ApiError, ErrorCode, Response};
use lfm_store::AccountStatus;

use super::{api_key, now, random_hex, session_response};
use crate::method::{Access, Context, Method};
use crate::methods::shared::OrFailed;

/// Signed; `token` from `auth.getToken`. Errors as Last.fm: 4 for a token this key never got,
/// 14 for one nobody approved yet, 15 for one that expired. A token is spent once.
#[derive(Debug, Clone, Copy, Default)]
pub struct GetSession;

#[async_trait]
impl Method for GetSession {
    fn name(&self) -> &'static str {
        "auth.getSession"
    }

    fn access(&self) -> Access {
        Access::Signed
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let token = ctx.params.required("token")?;
        let caller = api_key(ctx)?;
        let store = &ctx.services.store;
        let found = store
            .find_auth_token(token)
            .await
            .or_failed()?
            .filter(|found| found.api_key == caller)
            .ok_or_else(invalid_token)?;
        if found.expires_at <= now(ctx) {
            return Err(expired_token());
        }
        let account_id = found.account.ok_or(ErrorCode::UnauthorizedToken)?;
        let account = store
            .account(account_id)
            .await
            .or_failed()?
            .filter(|account| account.status == AccountStatus::Active)
            .ok_or(ErrorCode::AuthenticationFailed)?;
        let key = random_hex(16)?;
        let session = store
            .exchange_auth_token(token, &key, now(ctx))
            .await
            .or_failed()?
            .ok_or_else(invalid_token)?;
        Ok(session_response(&account, &session.session_key))
    }
}

fn invalid_token() -> ApiError {
    ApiError::new(ErrorCode::AuthenticationFailed)
        .with_message("Invalid authentication token supplied")
}

fn expired_token() -> ApiError {
    // Code 15 means "not streamable" elsewhere; for tokens Last.fm uses it for expiry.
    ApiError::new(ErrorCode::NotStreamable)
        .with_message("Unauthorized Token - This token has expired")
}
