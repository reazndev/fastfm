//! Turns parsed parameters into a rendered reply, checking in the order Last.fm does.

use std::sync::Arc;

use bytes::Bytes;
use lfm_core::{ApiError, ErrorCode, Output, Params, verify_signature};
use lfm_store::MirroredUser;

use crate::auth::{ApiKey, Scope, Session};
use crate::cache::CacheKey;
use crate::method::{Access, Context, Method, Services};
use crate::methods::shared::OrFailed;
use crate::now_playing::NowPlayingSource;
use crate::passthrough::{Forwarded, Passthrough, forwarded};
use crate::registry::Registry;
use crate::reply::{CLASSIC, Shape, error_reply, ok_reply};

pub use crate::reply::{Reply, Source};

/// Where a request goes: a local method, or Last.fm through the read-through cache.
enum Target {
    Local(Arc<dyn Method>),
    Forward(&'static Forwarded),
}

/// Routes requests to local methods, and the rest to Last.fm when a [`Passthrough`] is set.
#[derive(Debug, Clone)]
pub struct Dispatcher {
    registry: Registry,
    services: Services,
    pub(crate) passthrough: Option<Passthrough>,
    now_playing: Option<Arc<dyn NowPlayingSource>>,
}

impl Dispatcher {
    /// A dispatcher over these methods and services. Methods it does not have are unknown
    /// (error 3) until [`Dispatcher::with_passthrough`] sets an upstream.
    #[must_use]
    pub fn new(registry: Registry, services: Services) -> Self {
        Self {
            registry,
            services,
            passthrough: None,
            now_playing: None,
        }
    }

    /// Forwards the read methods the registry lacks, and local `user.*` and `library.*`
    /// requests for users this instance does not mirror, through `passthrough`.
    #[must_use]
    pub fn with_passthrough(self, passthrough: Passthrough) -> Self {
        Self {
            passthrough: Some(passthrough),
            ..self
        }
    }

    /// Shows what `source` says users are playing as the first item of recent tracks.
    #[must_use]
    pub fn with_now_playing(self, source: Arc<dyn NowPlayingSource>) -> Self {
        Self {
            now_playing: Some(source),
            ..self
        }
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
        let (target, api_key) = match self.admit(params).await {
            Ok(admitted) => admitted,
            Err(error) => return error_reply(&output, &error, CLASSIC),
        };
        match target {
            Target::Local(method) => self.local(method.as_ref(), &api_key, params, &output).await,
            Target::Forward(method) => self.forward(method, params, &output).await,
        }
    }

    // Observed order: a missing api_key (6) comes before an unknown method (3), which comes
    // before an invalid key (10).
    async fn admit(&self, params: &Params) -> Result<(Target, ApiKey), ApiError> {
        let key = params.required("api_key")?;
        let name = params.optional("method").unwrap_or_default();
        let target = match self.registry.get(name) {
            Some(method) => Target::Local(Arc::clone(method)),
            None => self
                .forwardable(name)
                .map(Target::Forward)
                .ok_or(ErrorCode::InvalidMethod)?,
        };
        let api_key = self.api_key(key).await?;
        Ok((target, api_key))
    }

    fn forwardable(&self, name: &str) -> Option<&'static Forwarded> {
        self.passthrough.as_ref().and(forwarded(name))
    }

    async fn local(
        &self,
        method: &dyn Method,
        api_key: &ApiKey,
        params: &Params,
        output: &Output,
    ) -> Reply {
        let shape = Shape::of(method);
        let session = match self.authorize(method.access(), params, api_key).await {
            Ok(session) => session,
            Err(error) => return error_reply(output, &error, shape),
        };
        let user_name = method.cache_user(params);
        let user = match self.mirrored(user_name).await {
            Ok(user) => user,
            Err(error) => return error_reply(output, &error, shape),
        };
        if user_name.is_some()
            && user.is_none()
            && let Some(method) = self.forwardable(method.name())
        {
            return self.forward(method, params, output).await;
        }
        let call = Call {
            method,
            api_key,
            session: session.as_ref(),
            user: user.as_ref(),
        };
        match self.run(&call, params, output).await {
            Ok(body) => ok_reply(output, &body, shape),
            Err(error) => error_reply(output, &error, shape),
        }
    }

    async fn run(
        &self,
        call: &Call<'_>,
        params: &Params,
        output: &Output,
    ) -> Result<Bytes, ApiError> {
        let method = call.method;
        let cache_key = self.cache_key(call, params, output);
        if let Some(body) = cache_key.as_ref().and_then(|k| self.services.cache.get(k)) {
            return Ok(body);
        }
        let ctx = Context {
            params,
            api_key: call.api_key,
            session: call.session,
            services: &self.services,
            now: self.services.clock.now(),
            user: call.user,
            now_playing: self.now_playing.as_deref(),
        };
        let response = method.call(&ctx).await?.style(method.style());
        let body = Bytes::from(Output::plain(output.format()).render(&response));
        if let Some(key) = cache_key {
            self.services.cache.insert(key, body.clone());
        }
        Ok(body)
    }

    /// The response cache key, or `None` when the reply must not be cached: methods without a
    /// user, and recent tracks while the user plays something, since the now-playing item
    /// changes without a new scrobble.
    fn cache_key(&self, call: &Call<'_>, params: &Params, output: &Output) -> Option<CacheKey> {
        let user_name = call.method.cache_user(params)?;
        let playing = call.method.name() == "user.getRecentTracks"
            && call
                .user
                .zip(self.now_playing.as_ref())
                .is_some_and(|(user, source)| source.now_playing(user.id).is_some());
        if playing {
            return None;
        }
        let version = call.user.map_or(0, |u| u.version);
        let key = CacheKey::new(user_name, call.method.name(), params, output.format());
        Some(key.at_version(version))
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

/// A local method call that passed the key and session checks.
struct Call<'a> {
    method: &'a dyn Method,
    api_key: &'a ApiKey,
    session: Option<&'a Session>,
    user: Option<&'a MirroredUser>,
}
