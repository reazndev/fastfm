//! The web UI: the Angular build in `web/dist/browser`, embedded at compile time. Paths that
//! are not files get `index.html`, so the UI's own routes load on refresh. A server built
//! without `web/dist` serves a short page that says so.

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::{Embed, EmbeddedFile};

#[derive(Embed)]
#[folder = "../../web/dist/browser"]
#[allow_missing = true]
struct Assets;

/// Paths that belong to the APIs; an unknown one is a 404, never the UI.
const RESERVED: [&str; 4] = ["/2.0", "/app/api", "/api/", "/healthz"];

const NOT_BUILT: &str = "<!doctype html><meta charset=\"utf-8\"><title>lfm-mirror</title>\
<p>The web UI is not built into this server. Run <code>npm run build</code> in <code>web/</code> \
and build the server again. The API at <code>/2.0/</code> works without it.</p>\n";

/// Hashed build output never changes under its name.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

/// The fallback handler for every path no route matched.
pub(crate) async fn serve(method: Method, uri: Uri) -> Response {
    let path = uri.path();
    if RESERVED
        .iter()
        .any(|prefix| path == prefix.trim_end_matches('/') || path.starts_with(prefix))
    {
        return not_found(path);
    }
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let name = path.trim_start_matches('/');
    if let Some(file) = Assets::get(name).filter(|_| !name.is_empty()) {
        let cache = if is_hashed(name) {
            IMMUTABLE
        } else {
            REVALIDATE
        };
        return file_response(&file, cache);
    }
    // A missing file is a 404; anything else is a UI route.
    if name
        .rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
    {
        return not_found(path);
    }
    match Assets::get("index.html") {
        Some(index) => file_response(&index, REVALIDATE),
        None => html(NOT_BUILT),
    }
}

fn file_response(file: &EmbeddedFile, cache: &'static str) -> Response {
    let mime = file.metadata.mimetype();
    let content_type =
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"));
    let headers = [
        (header::CONTENT_TYPE, content_type),
        (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
    ];
    (headers, Body::from(file.data.clone().into_owned())).into_response()
}

fn html(body: &'static str) -> Response {
    let headers = [
        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
        (header::CACHE_CONTROL, REVALIDATE),
    ];
    (headers, body).into_response()
}

/// The web API answers in its own error shape; everything else gets plain text.
fn not_found(path: &str) -> Response {
    if path.starts_with("/app/api") {
        let body = r#"{"error":{"code":"not_found","message":"Not found."}}"#;
        let headers = [(header::CONTENT_TYPE, "application/json")];
        return (StatusCode::NOT_FOUND, headers, body).into_response();
    }
    (StatusCode::NOT_FOUND, "Not found\n").into_response()
}

/// Whether the build put a content hash in the name. The Angular build hashes every script,
/// stylesheet and `media/` file it emits (`outputHashing: all`); files copied from `public/`
/// keep their names.
fn is_hashed(name: &str) -> bool {
    name.starts_with("media/")
        || [".js", ".mjs", ".css"]
            .iter()
            .any(|extension| name.ends_with(extension))
}

#[cfg(test)]
mod tests {
    use super::is_hashed;

    #[test]
    fn build_output_is_hashed_and_public_files_are_not() {
        assert!(is_hashed("main-AJ44GOIV.js"));
        assert!(is_hashed("chunk-C-h6c80r.js"));
        assert!(is_hashed("styles-LG56UX75.css"));
        assert!(is_hashed("media/font-ABCDEFGH.woff2"));
        assert!(!is_hashed("favicon.ico"));
        assert!(!is_hashed("index.html"));
    }
}
