//! Static frontend assets embedded into the binary at compile time.

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../frontend/out/"]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let raw = uri.path().trim_start_matches('/');
    if raw.starts_with("api/") || raw == "api" {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    let path = if raw.is_empty() {
        "index.html".to_string()
    } else {
        raw.to_string()
    };

    if let Some(response) = file(&path) {
        return response;
    }

    if !path.contains('.') {
        let dir_index = if path.ends_with('/') {
            format!("{path}index.html")
        } else {
            format!("{path}/index.html")
        };
        if let Some(response) = file(&dir_index) {
            return response;
        }
        // Client-side routing fallback for the dashboard.
        if let Some(response) = file("index.html") {
            return response;
        }
    }

    (StatusCode::NOT_FOUND, "Not found").into_response()
}

fn file(path: &str) -> Option<Response> {
    let asset = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("_next/static/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = Response::new(Body::from(asset.data.into_owned()));
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    Some(response)
}
