//! Tests for delivery of the built frontend.
//!
//! These exist because the first implementation attached both cache policies with
//! router-level layers, and the shell's `no-cache` silently overrode the assets'
//! `immutable` — disabling asset caching entirely with no visible symptom.

mod support;

use std::path::{Path, PathBuf};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use elrond_api::ApiConfig;
use tower::ServiceExt;

/// Name of the hashed asset written by the fixture.
const ASSET_NAME: &str = "index-abc12345.js";

/// Creates a directory laid out the way `vite build` leaves `web/dist`.
///
/// Uses a unique path per test so tests can run concurrently.
fn build_fixture(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("elrond-web-fixture-{label}"));
    let assets = dir.join("assets");
    std::fs::create_dir_all(&assets).expect("fixture directory");
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><html><body><div id=\"root\"></div></body></html>",
    )
    .expect("index.html");
    std::fs::write(assets.join(ASSET_NAME), "console.log('elrond');").expect("asset");
    dir
}

/// Builds a router serving `web_dir`.
async fn app(web_dir: &Path) -> Router {
    let config = ApiConfig {
        web_dir: Some(web_dir.to_path_buf()),
        ..ApiConfig::development()
    };
    support::build_with(config).await.router
}

/// Issues a GET and returns the response.
async fn get(app: &Router, path: &str) -> Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds")
}

/// Reads a response header as a string.
fn header_value(response: &Response<Body>, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[tokio::test]
async fn the_shell_is_served_at_the_root() {
    let dir = build_fixture("root");
    let app = app(&dir).await;

    let response = get(&app, "/").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_value(&response, header::CONTENT_TYPE).as_deref(),
        Some("text/html")
    );
}

#[tokio::test]
async fn the_shell_is_never_cached() {
    let dir = build_fixture("shell-cache");
    let app = app(&dir).await;

    let cache = header_value(&get(&app, "/").await, header::CACHE_CONTROL)
        .expect("the shell must carry a cache policy");
    assert!(
        cache.contains("no-cache"),
        "a cached shell would keep referencing assets a new deployment no longer has: {cache}"
    );
    assert!(!cache.contains("immutable"), "unexpected policy: {cache}");
}

#[tokio::test]
async fn hashed_assets_are_cached_immutably() {
    let dir = build_fixture("asset-cache");
    let app = app(&dir).await;

    let response = get(&app, &format!("/assets/{ASSET_NAME}")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let cache =
        header_value(&response, header::CACHE_CONTROL).expect("assets must carry a cache policy");
    assert!(
        cache.contains("immutable"),
        "content-hashed assets should be immutable, got {cache}"
    );
    assert!(
        cache.contains("max-age=31536000"),
        "expected a one-year max-age, got {cache}"
    );
    assert!(
        !cache.contains("no-cache"),
        "the shell's policy must not leak onto assets: {cache}"
    );
}

#[tokio::test]
async fn a_deep_link_returns_the_shell_so_a_refresh_works() {
    let dir = build_fixture("deep-link");
    let app = app(&dir).await;

    for path in ["/accounts", "/documents", "/binders/some/nested/route"] {
        let response = get(&app, path).await;
        assert_eq!(response.status(), StatusCode::OK, "for {path}");
        assert_eq!(
            header_value(&response, header::CONTENT_TYPE).as_deref(),
            Some("text/html"),
            "for {path}"
        );
    }
}

#[tokio::test]
async fn a_missing_asset_is_a_404_rather_than_the_shell() {
    let dir = build_fixture("missing-asset");
    let app = app(&dir).await;

    // Returning HTML here would make a failed script load look like a success and
    // produce a confusing syntax error in the browser console.
    let response = get(&app, "/assets/does-not-exist.js").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_api_still_wins_over_the_static_fallback() {
    let dir = build_fixture("api-precedence");
    let app = app(&dir).await;

    let response = get(&app, "/api/v1/health").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        header_value(&response, header::CONTENT_TYPE)
            .is_some_and(|value| value.contains("application/json")),
        "the API must not be shadowed by the shell fallback"
    );
}

#[tokio::test]
async fn security_headers_apply_to_static_responses_too() {
    let dir = build_fixture("static-headers");
    let app = app(&dir).await;

    let response = get(&app, "/").await;
    assert_eq!(
        header_value(&response, header::X_CONTENT_TYPE_OPTIONS).as_deref(),
        Some("nosniff")
    );
    assert!(
        header_value(&response, header::CONTENT_SECURITY_POLICY).is_some(),
        "the document response is the one that actually needs a CSP"
    );
}
