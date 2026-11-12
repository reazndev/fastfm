//! Turns parsed parameters into a rendered reply, checking in the order Last.fm does.

use std::sync::Arc;

use bytes::Bytes;
use lfm_core::{ApiError, ErrorCode, Format, Output, Params, Style, verify_signature};
use lfm_store::MirroredUser;

use crate::auth::{ApiKey, Scope, Session};
use crate::cache::CacheKey;
use crate::method::{Access, Context, Method, Services};
use crate::methods::shared::OrFailed;
use crate::registry::Registry;

/// The header Last.fm adds to the methods whose JSON carries `charset=UTF-8`.
const REQUEST_HEADERS: (&str, &str) = (
    "access-control-request-headers",
    "Origin, X-Atmosphere-tracking-id, X-Atmosphere-Framework, X-Cache-Date, Content-Type, \
     X-Atmosphere-Transport, *",
);

/// A rendered reply, ready for the HTTP layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The `Content-Type` header.
    pub content_type: &'static str,
    /// Headers beyond the content type and the CORS headers every reply gets.
    pub headers: &'static [(&'static str, &'static str)],
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

    /// The services the methods share.
    #[must_use]
    pub fn services(&self) -> &Services {
        &self.services
    }

    /// Handles one request. Never fails: errors become Last.fm error replies.
    pub async fn dispatch(&self, params: &Params) -> Reply {
        let output = match Output::from_params(params) {
            Ok(output) => output,
            Err(error) => return error_reply(&Output::xml(), &error, CLASSIC),
        };
        let (method, api_key) = match self.admit(params).await {
            Ok(admitted) => admitted,
            Err(error) => return error_reply(&output, &error, CLASSIC),
        };
        let shape = Shape::of(method.as_ref());
        match self.run(method.as_ref(), &api_key, params, &output).await {
            Ok(body) => ok_reply(&output, &body, shape),
            Err(error) => error_reply(&output, &error, shape),
        }
    }

    // Observed order: a missing api_key (6) comes before an unknown method (3), which comes
    // before an invalid key (10).
    async fn admit(&self, params: &Params) -> Result<(Arc<dyn Method>, ApiKey), ApiError> {
        let key = params.required("api_key")?;
        let method = params
            .optional("method")
            .and_then(|name| self.registry.get(name))
            .ok_or(ErrorCode::InvalidMethod)?;
        let api_key = self.api_key(key).await?;
        Ok((Arc::clone(method), api_key))
    }

    async fn run(
        &self,
        method: &dyn Method,
        api_key: &ApiKey,
        params: &Params,
        output: &Output,
    ) -> Result<Bytes, ApiError> {
        let session = self.authorize(method.access(), params, api_key).await?;
        let user_name = method.cache_user(params);
        let user = self.mirrored(user_name).await?;
        let cache_key = user_name.map(|name| {
            CacheKey::new(name, method.name(), params, output.format())
                .at_version(user.as_ref().map_or(0, |u| u.version))
        });
        if let Some(body) = cache_key.as_ref().and_then(|k| self.services.cache.get(k)) {
            return Ok(body);
        }
        let ctx = Context {
            params,
            api_key,
            session: session.as_ref(),
            services: &self.services,
            now: self.services.clock.now(),
            user: user.as_ref(),
        };
        let response = method.call(&ctx).await?.style(method.style());
        let body = Bytes::from(Output::plain(output.format()).render(&response));
        if let Some(key) = cache_key {
            self.services.cache.insert(key, body.clone());
        }
        Ok(body)
    }

    async fn mirrored(&self, name: Option<&str>) -> Result<Option<MirroredUser>, ApiError> {
        match name {
            Some(name) => self.services.store.user_by_name(name).await.or_failed(),
            None => Ok(None),
        }
    }

    async fn api_key(&self, key: &str) -> Result<ApiKey, ApiError> {
        match self.services.api_keys.api_key(key).await? {
            Some(record) if record.active => Ok(record),
            Some(_) => Err(ErrorCode::ApiKeySuspended.into()),
            None => Err(ErrorCode::InvalidApiKey.into()),
        }
    }

    // For writes, a missing sk or api_sig (6) comes before a bad signature (13), which comes
    // before checking the session key.
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

/// How a method's replies look apart from the body.
#[derive(Debug, Clone, Copy)]
struct Shape {
    style: Style,
    utf8_json: bool,
}

/// Errors raised before a method is chosen, and key errors, look like the classic backend's.
const CLASSIC: Shape = Shape {
    style: Style::Classic,
    utf8_json: false,
};

impl Shape {
    fn of(method: &dyn Method) -> Self {
        Self {
            style: method.style(),
            utf8_json: method.utf8_json(),
        }
    }

    fn reply(self, output: &Output, status: u16, body: Bytes) -> Reply {
        let utf8 = self.utf8_json && output.format() == Format::Json && output.callback().is_none();
        let content_type = if utf8 {
            "application/json; charset=UTF-8"
        } else {
            output.content_type()
        };
        let headers: &'static [(&str, &str)] = if self.utf8_json {
            &[REQUEST_HEADERS]
        } else {
            &[]
        };
        Reply {
            status,
            content_type,
            headers,
            body,
        }
    }
}

fn ok_reply(output: &Output, body: &Bytes, shape: Shape) -> Reply {
    let body = match (output.format(), output.callback()) {
        (Format::Json, Some(_)) => Bytes::from(output.wrap_json(body)),
        _ => body.clone(),
    };
    shape.reply(output, 200, body)
}

fn error_reply(output: &Output, error: &ApiError, shape: Shape) -> Reply {
    let body = Bytes::from(output.render_error_in(error, shape.style));
    shape.reply(output, error.http_status(), body)
}
