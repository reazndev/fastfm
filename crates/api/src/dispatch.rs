//! Turns parsed parameters into a rendered reply, checking in the order Last.fm does.

use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use lfm_core::{ApiError, ErrorCode, Format, Output, Params, verify_signature};

use crate::auth::{ApiKey, Scope, Session};
use crate::cache::CacheKey;
use crate::method::{Access, Context, Method, Services};
use crate::registry::Registry;

/// A rendered reply, ready for the HTTP layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The `Content-Type` header.
    pub content_type: &'static str,
    /// The body.
    pub body: Bytes,
}

/// Routes requests to methods.
#[derive(Debug, Clone)]
pub struct Dispatcher {
    registry: Registry,
    services: Services,
}

impl Dispatcher {
    /// A dispatcher over these methods and services.
    #[must_use]
    pub fn new(registry: Registry, services: Services) -> Self {
        Self { registry, services }
    }

    /// Handles one request. Never fails: errors become Last.fm error replies.
    pub async fn dispatch(&self, params: &Params) -> Reply {
        let output = match Output::from_params(params) {
            Ok(output) => output,
            Err(error) => return error_reply(&Output::xml(), &error),
        };
        match self.run(params, &output).await {
            Ok(reply) => reply,
            Err(error) => error_reply(&output, &error),
        }
    }

    // Observed order: a missing api_key (6) comes before an unknown method (3), which comes
    // before an invalid key (10). For writes, a missing sk or api_sig (6) comes before a bad
    // signature (13), which comes before checking the session key.
    async fn run(&self, params: &Params, output: &Output) -> Result<Reply, ApiError> {
        let key = params.required("api_key")?;
        let method = params
            .optional("method")
            .and_then(|name| self.registry.get(name))
            .ok_or(ErrorCode::InvalidMethod)?;
        let api_key = self.api_key(key).await?;
        let session = self.authorize(method.access(), params, &api_key).await?;
        let ctx = Context {
            params,
            api_key: &api_key,
            session: session.as_ref(),
            services: &self.services,
            now: unix_now(),
        };
        self.call(method.as_ref(), &ctx, output).await
    }

    async fn call(
        &self,
        method: &dyn Method,
        ctx: &Context<'_>,
        output: &Output,
    ) -> Result<Reply, ApiError> {
        let cache_key = method
            .cache_user(ctx.params)
            .map(|user| CacheKey::new(user, method.name(), ctx.params, output.format()));
        if let Some(body) = cache_key.as_ref().and_then(|k| self.services.cache.get(k)) {
            return Ok(ok_reply(output, &body));
        }
        let response = method.call(ctx).await?;
        let body = Bytes::from(Output::plain(output.format()).render(&response));
        let reply = ok_reply(output, &body);
        if let Some(key) = cache_key {
            self.services.cache.insert(key, body);
        }
        Ok(reply)
    }

    async fn api_key(&self, key: &str) -> Result<ApiKey, ApiError> {
        match self.services.api_keys.api_key(key).await? {
            Some(record) if record.active => Ok(record),
            Some(_) => Err(ErrorCode::ApiKeySuspended.into()),
            None => Err(ErrorCode::InvalidApiKey.into()),
        }
    }

    async fn authorize(
        &self,
        access: Access,
        params: &Params,
        api_key: &ApiKey,
    ) -> Result<Option<Session>, ApiError> {
        let signed = params.optional("api_sig").is_some();
        let session_key = params.optional("sk");
        match access {
            Access::Read if !(signed && session_key.is_some()) => return Ok(None),
            Access::Signed if !signed => return Err(ApiError::missing_parameter()),
            Access::Session if !signed || session_key.is_none() => {
                return Err(ApiError::missing_parameter());
            }
            _ => {}
        }
        if !verify_signature(params, &api_key.secret) {
            return Err(ErrorCode::InvalidSignature.into());
        }
        let Some(session_key) = session_key else {
            return Ok(None);
        };
        let session = self.services.sessions.session(session_key).await?;
        let session = session.ok_or(ErrorCode::InvalidSessionKey)?;
        if access == Access::Session && api_key.scope != Scope::ReadWrite {
            return Err(ErrorCode::AuthenticationFailed.into());
        }
        Ok(Some(session))
    }
}

fn ok_reply(output: &Output, body: &Bytes) -> Reply {
    let body = match (output.format(), output.callback()) {
        (Format::Json, Some(_)) => Bytes::from(output.wrap_json(body)),
        _ => body.clone(),
    };
    Reply {
        status: 200,
        content_type: output.content_type(),
        body,
    }
}

fn error_reply(output: &Output, error: &ApiError) -> Reply {
    Reply {
        status: error.http_status(),
        content_type: output.content_type(),
        body: Bytes::from(output.render_error(error)),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
