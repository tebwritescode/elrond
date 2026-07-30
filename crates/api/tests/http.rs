//! End-to-end HTTP tests.
//!
//! These drive the assembled router with the real adapters — SQLite, Argon2id,
//! and the CSPRNG — so the cookie, CSRF, and status-code contracts are checked as
//! a client actually experiences them, not as unit tests of the pieces.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use elrond_api::cookies::{CSRF_COOKIE, CSRF_HEADER, SESSION_COOKIE};
use elrond_api::{ApiConfig, AppState, router};
use elrond_application::auth::AuthService;
use elrond_application::ports::SessionPolicy;
use elrond_infrastructure::{
    Argon2idHasher, Database, RandomSessionTokens, SqliteSessionRepository, SqliteUserRepository,
    SystemClock,
};
use serde_json::{Value, json};
use tower::ServiceExt;

/// A signed-in client's credentials.
struct Client {
    /// Session cookie value, if signed in.
    session: Option<String>,
    /// CSRF token value.
    csrf: Option<String>,
}

impl Client {
    /// An anonymous client with no cookies.
    fn anonymous() -> Self {
        Self {
            session: None,
            csrf: None,
        }
    }

    /// Renders the `Cookie` header value.
    fn cookie_header(&self) -> String {
        let mut parts = Vec::new();
        if let Some(session) = &self.session {
            parts.push(format!("{SESSION_COOKIE}={session}"));
        }
        if let Some(csrf) = &self.csrf {
            parts.push(format!("{CSRF_COOKIE}={csrf}"));
        }
        parts.join("; ")
    }

    /// Absorbs any `Set-Cookie` headers from a response.
    fn absorb(&mut self, response: &Response<Body>) {
        if let Some(value) = set_cookie(response, SESSION_COOKIE) {
            self.session = (!value.is_empty()).then_some(value);
        }
        if let Some(value) = set_cookie(response, CSRF_COOKIE) {
            self.csrf = (!value.is_empty()).then_some(value);
        }
    }
}

/// Builds a router backed by an in-memory database.
async fn app() -> Router {
    app_with_config(ApiConfig::development()).await
}

/// Builds a router with a specific HTTP configuration.
async fn app_with_config(config: ApiConfig) -> Router {
    let database = Database::connect_in_memory()
        .await
        .expect("in-memory database");
    let tokens = Arc::new(RandomSessionTokens);
    let auth = AuthService::new(
        Arc::new(SqliteUserRepository::new(&database)),
        Arc::new(SqliteSessionRepository::new(&database)),
        Arc::new(Argon2idHasher::new()),
        tokens.clone(),
        Arc::new(SystemClock),
        SessionPolicy::default(),
    );
    router(AppState::new(
        auth,
        tokens,
        config,
        SessionPolicy::default(),
    ))
}

/// Extracts a cookie value from the response's `Set-Cookie` headers.
fn set_cookie(response: &Response<Body>, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&prefix))
        .map(|value| {
            value
                .trim_start_matches(&prefix)
                .split(';')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
}

/// Reads a response body as JSON.
async fn json_body(response: Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body is readable");
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(&bytes).expect("body is JSON")
}

/// Sends a GET request.
async fn get(app: &Router, path: &str, client: &Client) -> Response<Body> {
    let mut builder = Request::builder().method("GET").uri(path);
    let cookies = client.cookie_header();
    if !cookies.is_empty() {
        builder = builder.header(header::COOKIE, cookies);
    }
    app.clone()
        .oneshot(builder.body(Body::empty()).expect("request builds"))
        .await
        .expect("router responds")
}

/// Sends a JSON request with the CSRF header populated from the client.
async fn send_json(
    app: &Router,
    method: &str,
    path: &str,
    client: &Client,
    body: Option<Value>,
) -> Response<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");

    let cookies = client.cookie_header();
    if !cookies.is_empty() {
        builder = builder.header(header::COOKIE, cookies);
    }
    if let Some(csrf) = &client.csrf {
        builder = builder.header(CSRF_HEADER, csrf);
    }

    let body = body.map_or_else(Body::empty, |value| Body::from(value.to_string()));
    app.clone()
        .oneshot(builder.body(body).expect("request builds"))
        .await
        .expect("router responds")
}

/// Fetches bootstrap and returns a client holding the issued CSRF token.
async fn bootstrap(app: &Router) -> (Client, Value) {
    let mut client = Client::anonymous();
    let response = get(app, "/api/v1/bootstrap", &client).await;
    assert_eq!(response.status(), StatusCode::OK);
    client.absorb(&response);
    let body = json_body(response).await;
    (client, body)
}

/// Runs first-run setup and returns a signed-in client.
async fn setup_admin(app: &Router) -> Client {
    let (mut client, _) = bootstrap(app).await;
    let response = send_json(
        app,
        "POST",
        "/api/v1/setup",
        &client,
        Some(json!({
            "username": "records.admin",
            "password": "a sufficiently long passphrase"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    client.absorb(&response);
    client
}

#[tokio::test]
async fn health_reports_status_and_version() {
    let app = app().await;
    let response = get(&app, "/api/v1/health", &Client::anonymous()).await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn bootstrap_announces_setup_and_issues_a_csrf_token() {
    let app = app().await;
    let (client, body) = bootstrap(&app).await;

    assert_eq!(body["requires_setup"], true);
    assert_eq!(body["user"], Value::Null);
    let token = client.csrf.expect("a CSRF cookie must be issued");
    assert!(!token.is_empty());
    assert_eq!(body["csrf_token"], token, "cookie and body must agree");
}

#[tokio::test]
async fn bootstrap_reuses_an_existing_csrf_token() {
    let app = app().await;
    let (client, _) = bootstrap(&app).await;

    let response = get(&app, "/api/v1/bootstrap", &client).await;
    let body = json_body(response).await;
    assert_eq!(
        body["csrf_token"],
        client.csrf.expect("token present"),
        "a second tab must not invalidate the first tab's token"
    );
}

#[tokio::test]
async fn setup_creates_an_administrator_and_signs_it_in() {
    let app = app().await;
    let client = setup_admin(&app).await;

    assert!(client.session.is_some(), "a session cookie must be set");

    let response = get(&app, "/api/v1/me", &client).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["username"], "records.admin");
    assert_eq!(body["role"], "admin");
    assert!(
        body.get("email").is_none(),
        "the API must not expose an email field: {body}"
    );
}

#[tokio::test]
async fn setup_closes_permanently_once_an_account_exists() {
    let app = app().await;
    let client = setup_admin(&app).await;

    let (_, body) = bootstrap(&app).await;
    assert_eq!(body["requires_setup"], false);

    let response = send_json(
        &app,
        "POST",
        "/api/v1/setup",
        &client,
        Some(json!({
            "username": "second.admin",
            "password": "another long passphrase"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(response).await["code"], "setup_already_completed");
}

#[tokio::test]
async fn the_response_never_contains_a_password_hash() {
    let app = app().await;
    let (client, _) = bootstrap(&app).await;
    let response = send_json(
        &app,
        "POST",
        "/api/v1/setup",
        &client,
        Some(json!({
            "username": "records.admin",
            "password": "a sufficiently long passphrase"
        })),
    )
    .await;

    let body = json_body(response).await.to_string();
    assert!(
        !body.contains("argon2"),
        "credential material leaked: {body}"
    );
    assert!(
        !body.contains("password"),
        "credential material leaked: {body}"
    );
    assert!(
        !body.contains("passphrase"),
        "credential material leaked: {body}"
    );
}

#[tokio::test]
async fn a_state_changing_request_without_a_csrf_token_is_refused() {
    let app = app().await;
    // No bootstrap call, so the client holds no CSRF cookie at all.
    let response = send_json(
        &app,
        "POST",
        "/api/v1/setup",
        &Client::anonymous(),
        Some(json!({
            "username": "records.admin",
            "password": "a sufficiently long passphrase"
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(json_body(response).await["code"], "csrf_check_failed");
}

#[tokio::test]
async fn a_mismatched_csrf_token_is_refused() {
    let app = app().await;
    let (client, _) = bootstrap(&app).await;

    // Cookie from bootstrap, header from an attacker who cannot read it.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, client.cookie_header())
                .header(CSRF_HEADER, "a-guessed-value")
                .body(Body::from(
                    json!({
                        "username": "records.admin",
                        "password": "a sufficiently long passphrase"
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_request_from_an_unexpected_origin_is_refused() {
    let app = app().await;
    let (client, _) = bootstrap(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "https://attacker.example.net")
                .header(header::COOKIE, client.cookie_header())
                .header(CSRF_HEADER, client.csrf.clone().expect("token"))
                .body(Body::from("{}"))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_request_from_the_configured_origin_is_accepted() {
    let config = ApiConfig::development().with_public_url("http://localhost:3100");
    let app = app_with_config(config).await;
    let (client, _) = bootstrap(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:3100")
                .header(header::COOKIE, client.cookie_header())
                .header(CSRF_HEADER, client.csrf.clone().expect("token"))
                .body(Body::from(
                    json!({
                        "username": "records.admin",
                        "password": "a sufficiently long passphrase"
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn safe_methods_need_no_csrf_token() {
    let app = app().await;
    // Health is reachable with no cookies and no header at all.
    let response = get(&app, "/api/v1/health", &Client::anonymous()).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_weak_password_is_rejected_with_the_offending_field() {
    let app = app().await;
    let (client, _) = bootstrap(&app).await;

    let response = send_json(
        &app,
        "POST",
        "/api/v1/setup",
        &client,
        Some(json!({
            "username": "records.admin",
            "password": "short"
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await;
    assert_eq!(body["code"], "field_too_short");
    assert_eq!(body["field"], "password");
}

#[tokio::test]
async fn a_malformed_username_is_rejected() {
    let app = app().await;
    let (client, _) = bootstrap(&app).await;

    let response = send_json(
        &app,
        "POST",
        "/api/v1/setup",
        &client,
        Some(json!({
            "username": "!!bad",
            "password": "a sufficiently long passphrase"
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json_body(response).await["field"], "username");
}

#[tokio::test]
async fn malformed_json_is_reported_in_the_standard_error_shape() {
    let app = app().await;
    let (client, _) = bootstrap(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, client.cookie_header())
                .header(CSRF_HEADER, client.csrf.clone().expect("token"))
                .body(Body::from("{ this is not json"))
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert_eq!(body["code"], "request_body_malformed_json");
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn signing_in_with_the_wrong_password_is_unauthorized() {
    let app = app().await;
    let admin = setup_admin(&app).await;

    let response = send_json(
        &app,
        "POST",
        "/api/v1/session",
        &admin,
        Some(json!({
            "username": "records.admin",
            "password": "definitely the wrong one"
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(response).await["code"], "invalid_credentials");
}

#[tokio::test]
async fn an_unknown_account_is_indistinguishable_from_a_wrong_password() {
    let app = app().await;
    let admin = setup_admin(&app).await;

    let response = send_json(
        &app,
        "POST",
        "/api/v1/session",
        &admin,
        Some(json!({
            "username": "nobody",
            "password": "definitely the wrong one"
        })),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(response).await["code"], "invalid_credentials");
}

#[tokio::test]
async fn signing_in_rotates_the_csrf_token() {
    let app = app().await;
    let admin = setup_admin(&app).await;
    let before = admin.csrf.clone().expect("token");

    let mut client = admin;
    let response = send_json(
        &app,
        "POST",
        "/api/v1/session",
        &client,
        Some(json!({
            "username": "records.admin",
            "password": "a sufficiently long passphrase"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    client.absorb(&response);

    assert_ne!(
        client.csrf.expect("token"),
        before,
        "the CSRF token must be rotated when the privilege level changes"
    );
}

#[tokio::test]
async fn a_new_sign_in_reports_an_expiry() {
    let app = app().await;
    let admin = setup_admin(&app).await;

    let response = send_json(
        &app,
        "POST",
        "/api/v1/session",
        &admin,
        Some(json!({
            "username": "records.admin",
            "password": "a sufficiently long passphrase"
        })),
    )
    .await;

    let body = json_body(response).await;
    let expires_at = body["expires_at"].as_str().expect("expiry present");
    assert!(
        expires_at.ends_with('Z'),
        "expected UTC RFC 3339: {expires_at}"
    );
}

#[tokio::test]
async fn me_requires_a_session() {
    let app = app().await;
    setup_admin(&app).await;

    let response = get(&app, "/api/v1/me", &Client::anonymous()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json_body(response).await["code"], "not_authenticated");
}

#[tokio::test]
async fn a_fabricated_session_cookie_is_rejected() {
    let app = app().await;
    setup_admin(&app).await;

    let forged = Client {
        session: Some("f".repeat(64)),
        csrf: None,
    };
    let response = get(&app, "/api/v1/me", &forged).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signing_out_revokes_the_session_and_clears_the_cookies() {
    let app = app().await;
    let mut client = setup_admin(&app).await;

    let response = send_json(&app, "DELETE", "/api/v1/session", &client, None).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        set_cookie(&response, SESSION_COOKIE).as_deref(),
        Some(""),
        "the session cookie must be cleared"
    );
    assert_eq!(set_cookie(&response, CSRF_COOKIE).as_deref(), Some(""));

    // The old token must not work even though the test client still holds it.
    let stale = Client {
        session: client.session.take(),
        csrf: None,
    };
    let response = get(&app, "/api/v1/me", &stale).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn signing_out_twice_is_not_an_error() {
    let app = app().await;
    let client = setup_admin(&app).await;

    for _ in 0..2 {
        let response = send_json(&app, "DELETE", "/api/v1/session", &client, None).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
}

#[tokio::test]
async fn administrators_can_list_accounts() {
    let app = app().await;
    let client = setup_admin(&app).await;

    let response = get(&app, "/api/v1/users", &client).await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    let users = body.as_array().expect("an array of accounts");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["username"], "records.admin");
    assert_eq!(users[0]["role"], "admin");
}

#[tokio::test]
async fn listing_accounts_requires_a_session() {
    let app = app().await;
    setup_admin(&app).await;

    let response = get(&app, "/api/v1/users", &Client::anonymous()).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn repeated_credential_failures_are_rate_limited() {
    let config = ApiConfig {
        auth_attempt_limit: 3,
        ..ApiConfig::development()
    };
    let app = app_with_config(config).await;
    let admin = setup_admin(&app).await;

    let attempt = || {
        send_json(
            &app,
            "POST",
            "/api/v1/session",
            &admin,
            Some(json!({
                "username": "records.admin",
                "password": "wrong every time"
            })),
        )
    };

    for _ in 0..3 {
        assert_eq!(attempt().await.status(), StatusCode::UNAUTHORIZED);
    }

    let response = attempt().await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        response.headers().contains_key(header::RETRY_AFTER),
        "a throttled client must be told when to retry"
    );
    assert_eq!(json_body(response).await["code"], "rate_limited");
}

#[tokio::test]
async fn a_successful_sign_in_clears_the_throttle() {
    let config = ApiConfig {
        auth_attempt_limit: 3,
        ..ApiConfig::development()
    };
    let app = app_with_config(config).await;
    let admin = setup_admin(&app).await;

    for _ in 0..2 {
        send_json(
            &app,
            "POST",
            "/api/v1/session",
            &admin,
            Some(json!({ "username": "records.admin", "password": "wrong" })),
        )
        .await;
    }

    let response = send_json(
        &app,
        "POST",
        "/api/v1/session",
        &admin,
        Some(json!({
            "username": "records.admin",
            "password": "a sufficiently long passphrase"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    // The counter was reset, so several further attempts are still allowed.
    for _ in 0..3 {
        let response = send_json(
            &app,
            "POST",
            "/api/v1/session",
            &admin,
            Some(json!({ "username": "records.admin", "password": "wrong" })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn security_headers_are_present_on_api_responses() {
    let app = app().await;
    let response = get(&app, "/api/v1/health", &Client::anonymous()).await;
    let headers = response.headers();

    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .map(|v| v.to_str().unwrap()),
        Some("nosniff")
    );
    assert_eq!(
        headers
            .get(header::X_FRAME_OPTIONS)
            .map(|v| v.to_str().unwrap()),
        Some("DENY")
    );
    assert_eq!(
        headers
            .get(header::REFERRER_POLICY)
            .map(|v| v.to_str().unwrap()),
        Some("no-referrer")
    );

    let csp = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .expect("a content security policy")
        .to_str()
        .expect("ascii");
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("object-src 'none'"));
    assert!(
        !csp.contains("'unsafe-eval'") || csp.contains("'wasm-unsafe-eval'"),
        "the policy must not permit arbitrary eval: {csp}"
    );
}

#[tokio::test]
async fn hsts_is_only_sent_when_cookies_are_secure() {
    let insecure = app().await;
    let response = get(&insecure, "/api/v1/health", &Client::anonymous()).await;
    assert!(
        !response
            .headers()
            .contains_key(header::STRICT_TRANSPORT_SECURITY),
        "HSTS on a plain-HTTP development origin would poison the browser for the whole host"
    );

    let secure = app_with_config(ApiConfig {
        secure_cookies: true,
        ..ApiConfig::development()
    })
    .await;
    let response = get(&secure, "/api/v1/health", &Client::anonymous()).await;
    assert!(
        response
            .headers()
            .contains_key(header::STRICT_TRANSPORT_SECURITY)
    );
}

#[tokio::test]
async fn an_unbuilt_frontend_serves_a_helpful_placeholder() {
    let app = app().await;
    let response = get(&app, "/", &Client::anonymous()).await;

    // 503 so a health checker can distinguish "not built" from "working".
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 64)
        .await
        .expect("body is readable");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("5273"),
        "the placeholder should name the dev port"
    );
}

#[tokio::test]
async fn an_unknown_api_route_is_not_swallowed_by_the_spa_fallback() {
    let app = app().await;
    let response = get(&app, "/api/v1/does-not-exist", &Client::anonymous()).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an unknown API path must 404 rather than return the client shell"
    );
    assert_eq!(json_body(response).await["code"], "not_found");
}
