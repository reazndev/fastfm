use lfm_upstream::{ApiKey, Format, HttpMethod, RawRequest};

/// A well-formed key that Last.fm has never issued.
const INVALID_KEY: &str = "0123456789abcdef0123456789abcdef";

/// Which key a fixture request sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyMode {
    Pool,
    Invalid,
    Omit,
}

/// One fixture: a request recorded once per format.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    method: String,
    variant: String,
    params: Vec<(String, String)>,
    http_method: HttpMethod,
    key: KeyMode,
}

impl Spec {
    pub(crate) fn new(method: &str, variant: &str) -> Self {
        Self {
            method: method.to_owned(),
            variant: variant.to_owned(),
            params: Vec::new(),
            http_method: HttpMethod::Get,
            key: KeyMode::Pool,
        }
    }

    pub(crate) fn param(mut self, name: &str, value: &str) -> Self {
        self.params.push((name.to_owned(), value.to_owned()));
        self
    }

    pub(crate) fn num(self, name: &str, value: u64) -> Self {
        self.param(name, &value.to_string())
    }

    /// A copy under another variant name, to add parameters to.
    pub(crate) fn with_variant(&self, variant: &str) -> Self {
        Self {
            variant: variant.to_owned(),
            ..self.clone()
        }
    }

    /// Sets the value of an existing parameter.
    pub(crate) fn replace_param(mut self, name: &str, value: &str) -> Self {
        self.params
            .iter_mut()
            .filter(|(k, _)| k == name)
            .for_each(|(_, v)| value.clone_into(v));
        self
    }

    pub(crate) fn post(self) -> Self {
        Self {
            http_method: HttpMethod::Post,
            ..self
        }
    }

    pub(crate) fn invalid_key(self) -> Self {
        Self {
            key: KeyMode::Invalid,
            ..self
        }
    }

    pub(crate) fn omit_key(self) -> Self {
        Self {
            key: KeyMode::Omit,
            ..self
        }
    }

    pub(crate) fn method(&self) -> &str {
        &self.method
    }

    pub(crate) fn variant(&self) -> &str {
        &self.variant
    }

    pub(crate) fn http_method(&self) -> HttpMethod {
        self.http_method
    }

    /// The request for one format.
    pub(crate) fn request(&self, format: Format) -> RawRequest {
        let request = RawRequest::new(self.method.as_str())
            .params(self.params.iter().cloned())
            .format(format)
            .http_method(self.http_method);
        match self.key {
            KeyMode::Pool => request,
            KeyMode::Invalid => request.api_key(ApiKey::new(INVALID_KEY)),
            KeyMode::Omit => request.without_api_key(),
        }
    }

    /// The key as written into the fixture's metadata.
    pub(crate) fn key_text(&self) -> Option<&'static str> {
        match self.key {
            KeyMode::Pool => Some("REDACTED"),
            KeyMode::Invalid => Some(INVALID_KEY),
            KeyMode::Omit => None,
        }
    }
}
