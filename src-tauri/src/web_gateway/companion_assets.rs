use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};

include!(concat!(env!("OUT_DIR"), "/companion_assets.rs"));

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

fn lookup(path: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

/// Static files are public (the token guards `/api`). Vite emits content-hashed names under
/// `assets/`, so those are immutable; everything else, `index.html` included, must revalidate
/// or a rebuilt app would keep serving a page that points at assets that no longer exist.
pub(super) async fn serve(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let path = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    let Some(bytes) = lookup(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, content_type(path)),
            (header::CACHE_CONTROL, cache),
        ],
        Body::from(bytes),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_is_always_embedded() {
        assert!(lookup("index.html").is_some());
        assert!(lookup("missing.js").is_none());
    }

    #[test]
    fn content_types_cover_vite_output() {
        assert_eq!(
            content_type("assets/a-1.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("assets/a-1.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
    }
}
