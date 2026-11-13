//! `auth.getMobileSession`: a session key for a local account's name and password.

use std::sync::Arc;

use async_trait::async_trait;
use lfm_core::{ApiError, ErrorCode, Response};

use super::{api_key, new_session};
use crate::accounts::AccountServices;
use crate::method::{Access, Context, Method};
use crate::methods::shared::OrFailed;

/// Signed; `username` and `password` of the local account (not the Last.fm one). Any mismatch
/// is error 4, whether the name exists or not.
#[derive(Debug, Clone)]
pub struct GetMobileSession {
    accounts: Arc<dyn AccountServices>,
}

impl GetMobileSession {
    /// The method, checking passwords through `accounts`.
    #[must_use]
    pub fn new(accounts: Arc<dyn AccountServices>) -> Self {
        Self { accounts }
    }
}

#[async_trait]
impl Method for GetMobileSession {
    fn name(&self) -> &'static str {
        "auth.getMobileSession"
    }

    fn access(&self) -> Access {
        Access::Signed
    }

    async fn call(&self, ctx: &Context<'_>) -> Result<Response, ApiError> {
        let name = ctx.params.required("username")?;
        let password = ctx.params.required("password")?;
        let account = self
            .accounts
            .check_password(api_key(ctx)?, name, password)
            .await?
            .ok_or(ErrorCode::AuthenticationFailed)?;
        let account = ctx
            .services
            .store
            .account(account)
            .await
            .or_failed()?
            .ok_or(ErrorCode::AuthenticationFailed)?;
        new_session(ctx, &account).await
    }
}
