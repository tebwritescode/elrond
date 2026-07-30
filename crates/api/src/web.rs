//! Delivery of the built frontend.
//!
//! One process serves both the API and the client, so there is no separate web
//! server to configure and no CORS to reason about in production.

use std::path::Path;

use axum::Router;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use tower::ServiceBuilder;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::state::AppState;

/// Subdirectory Vite writes hashed, immutable assets into.
const ASSET_DIR: &str = "assets";

/// Builds the routes that serve the client, or a development placeholder when no
/// build is present.
pub fn routes(web_dir: Option<&Path>) -> Router<AppState> {
    let Some(dir) = web_dir else {
        return Router::new().fallback(placeholder);
    };

    let index = dir.join("index.html");
    if !index.is_file() {
        tracing::warn!(
            path = %dir.display(),
            "no built frontend found; serving the development placeholder"
        );
        return Router::new().fallback(placeholder);
    }

    // Hashed filenames change whenever content changes, so they can be cached
    // indefinitely. This is the single biggest repeat-visit win available.
    //
    // The cache headers are attached with a ServiceBuilder per service rather
    // than with Router::layer. A router-level layer wraps everything mounted so
    // far, so the shell's `no-cache` would override the assets' `immutable` and
    // silently disable asset caching altogether.
    let assets = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("public, max-age=31536000, immutable"),
        ))
        .service(
            tower_http::services::ServeDir::new(dir.join(ASSET_DIR))
                .precompressed_gzip()
                .append_index_html_on_directories(false),
        );

    // index.html must never be cached, or a client keeps loading an old bundle
    // that references assets the new deployment no longer has.
    let shell = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-cache, must-revalidate"),
        ))
        .service(tower_http::services::ServeFile::new(index));

    Router::new()
        .nest_service(&format!("/{ASSET_DIR}"), assets)
        // Any unmatched path is a client-side route, so it gets the shell and
        // React resolves it. Deep links therefore survive a refresh.
        .fallback_service(shell)
}

/// Response shown when the API is running but the client has not been built.
///
/// A blank 404 here is one of the more confusing states to land in during
/// development, so it names the two ways forward explicitly.
async fn placeholder() -> Response {
    let body = format!(
        r#"<main style="font-family: ui-sans-serif, system-ui, sans-serif; max-width: 44rem; margin: 4rem auto; padding: 0 1.5rem; line-height: 1.6; color: #2a2521;">
  <p style="font-size: .75rem; letter-spacing: .12em; text-transform: uppercase; color: #8a7a6a; margin: 0;">Elrond {version}</p>
  <h1 style="font-size: 1.75rem; margin: .25rem 0 1rem;">The API is running. The client is not built.</h1>
  <p>The Rust process is serving requests, but there is no compiled frontend in the configured web directory.</p>
  <h2 style="font-size: 1rem; margin: 2rem 0 .5rem;">During development</h2>
  <p>Use the Vite dev server, which proxies the API and gives you hot reloading:</p>
  <pre style="background: #f4efe8; padding: .875rem 1rem; border-radius: .375rem; overflow-x: auto;">cd web
npm install
npm run dev</pre>
  <p>Then open <a href="http://localhost:5273" style="color: #8a4b2a;">http://localhost:5273</a>.</p>
  <h2 style="font-size: 1rem; margin: 2rem 0 .5rem;">For a single-process deployment</h2>
  <pre style="background: #f4efe8; padding: .875rem 1rem; border-radius: .375rem; overflow-x: auto;">cd web &amp;&amp; npm run build</pre>
  <p>Then restart the server with <code>ELROND_WEB_DIR</code> pointing at <code>web/dist</code>.</p>
  <p style="margin-top: 2rem;"><a href="/api/v1/health" style="color: #8a4b2a;">Check the health endpoint</a></p>
</main>"#,
        version = env!("CARGO_PKG_VERSION")
    );

    // 503 rather than 200: the application genuinely is not ready to serve, and a
    // health checker should be able to tell.
    (StatusCode::SERVICE_UNAVAILABLE, Html(body)).into_response()
}
