//! Router assembly and cross-cutting middleware.

pub mod binders;
pub mod categories;
pub mod documents;
pub mod health;
pub mod session;
pub mod users;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get, patch, post};
use axum_extra::extract::cookie::CookieJar;
use elrond_application::ApplicationError;
use tower_http::trace::TraceLayer;

use crate::cookies::{CSRF_COOKIE, CSRF_HEADER, constant_time_eq};
use crate::error::ApiError;
use crate::state::AppState;
use crate::web;

/// Path prefix for every JSON endpoint.
pub const API_PREFIX: &str = "/api/v1";

/// Builds the complete application router.
///
/// Layer order matters and is set deliberately: the CSRF guard wraps only the
/// API routes, while security headers and tracing wrap everything including
/// static assets.
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health::health))
        .route("/bootstrap", get(session::bootstrap))
        .route("/setup", post(session::complete_setup))
        .route("/session", post(session::sign_in))
        .route("/session", delete(session::sign_out))
        .route("/me", get(session::me))
        .route("/users", get(users::list))
        .route(
            "/categories",
            get(categories::tree).post(categories::create),
        )
        .route(
            "/categories/{id}",
            patch(categories::update).delete(categories::delete),
        )
        .route("/tags", get(documents::list_tags))
        .route("/binders/build", post(binders::build))
        .route("/documents", get(documents::list).post(documents::upload))
        .route("/documents/import", post(documents::import))
        .route(
            "/documents/{id}",
            get(documents::detail).patch(documents::update),
        )
        .route("/documents/{id}/lifecycle", post(documents::transition))
        .route("/documents/{id}/versions", post(documents::add_version))
        .route("/versions/{id}/original", get(documents::download_original))
        .route("/versions/{id}/pdf", get(documents::download_pdf))
        // Without this, an unmatched API path would fall through to the SPA
        // fallback and answer a JSON client with an HTML page.
        .fallback(unknown_endpoint)
        // Applied here rather than globally so a static asset request is not
        // subjected to a check that only makes sense for state-changing JSON.
        .layer(middleware::from_fn_with_state(state.clone(), csrf_guard));

    Router::new()
        .nest(API_PREFIX, api)
        // Anything not matched by the API is either a built asset or a client
        // route, both handled by the frontend routes.
        .merge(web::routes(state.config.web_dir.as_deref()))
        .layer(DefaultBodyLimit::max(state.config.max_body_bytes))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Answers unmatched API paths in the standard JSON error shape.
async fn unknown_endpoint() -> ApiError {
    ApiError::Application(ApplicationError::NotFound {
        resource: "endpoint",
    })
}

/// Rejects state-changing requests that fail cross-site checks.
///
/// Two independent checks are applied, because each covers a case the other
/// misses:
///
/// 1. An `Origin` allowlist. Browsers always send `Origin` on cross-site
///    state-changing requests, so a mismatch is decisive. It is skipped when the
///    header is absent, which is the case for same-origin requests from some
///    browsers and for non-browser clients.
/// 2. A double-submit token. The CSRF cookie must be echoed in a header. A
///    cross-site attacker can cause the cookie to be sent but cannot read it to
///    populate the header, and cannot set a custom header on a simple form post
///    at all.
async fn csrf_guard(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Safe methods do not change state, so they need no token. This is also what
    // lets the client fetch its first CSRF token.
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    ) {
        return Ok(next.run(request).await);
    }

    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        && !state.config.allows_origin(origin)
    {
        // Named on both sides so the operator can see which of the public URL,
        // a port mapping, or a proxy is wrong, instead of guessing.
        let mut allowed = state.config.public_origin.clone();
        for extra in &state.config.additional_allowed_origins {
            allowed.push_str(", ");
            allowed.push_str(extra);
        }
        tracing::warn!(
            origin,
            allowed,
            "rejected a request from an unexpected origin"
        );
        return Err(ApiError::OriginRejected {
            received: origin.to_owned(),
            allowed,
        });
    }

    let jar = CookieJar::from_headers(request.headers());
    let Some(cookie) = jar.get(CSRF_COOKIE) else {
        return Err(ApiError::CsrfRejected {
            // The commonest cause by far: ELROND_SECURE_COOKIES enabled without
            // HTTPS makes the browser silently discard the cookie, which then
            // looks identical from every host. Say so here, where it surfaces.
            reason: "no token cookie arrived — if ELROND_SECURE_COOKIES is on but Elrond is \
                     reached over plain http://, the browser silently discards its cookies; \
                     either serve HTTPS or set ELROND_SECURE_COOKIES=false",
        });
    };
    let Some(header) = request
        .headers()
        .get(HeaderName::from_static(CSRF_HEADER))
        .and_then(|value| value.to_str().ok())
    else {
        return Err(ApiError::CsrfRejected {
            reason: "missing token header",
        });
    };

    // An empty token would otherwise match an empty header and defeat the check.
    if cookie.value().is_empty() || !constant_time_eq(cookie.value(), header) {
        return Err(ApiError::CsrfRejected {
            reason: "token mismatch",
        });
    }

    Ok(next.run(request).await)
}

/// Sets a header from a static string, skipping silently if it is not a valid
/// header value.
///
/// Every value passed below is a compile-time constant of ASCII text, so the
/// conversion cannot actually fail.
fn set(headers: &mut header::HeaderMap, name: HeaderName, value: &'static str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

/// Adds response headers that constrain what a browser will do with a response.
async fn security_headers(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    set(
        headers,
        header::CONTENT_SECURITY_POLICY,
        // `wasm-unsafe-eval` and `blob:` workers are required by PDF.js, which
        // renders documents in a worker and compiles a WebAssembly module for
        // some image codecs. Note the absence of `unsafe-eval`.
        "default-src 'self'; \
         base-uri 'none'; \
         object-src 'none'; \
         frame-ancestors 'none'; \
         form-action 'self'; \
         img-src 'self' data: blob:; \
         font-src 'self'; \
         style-src 'self' 'unsafe-inline'; \
         script-src 'self' 'wasm-unsafe-eval'; \
         worker-src 'self' blob:; \
         connect-src 'self'",
    );
    set(headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(headers, header::X_FRAME_OPTIONS, "DENY");
    set(headers, header::REFERRER_POLICY, "no-referrer");
    set(
        headers,
        HeaderName::from_static("cross-origin-opener-policy"),
        "same-origin",
    );
    set(
        headers,
        HeaderName::from_static("cross-origin-resource-policy"),
        "same-origin",
    );
    set(
        headers,
        HeaderName::from_static("permissions-policy"),
        "camera=(), microphone=(), geolocation=(), payment=()",
    );

    if state.config.secure_cookies {
        // Only meaningful over TLS, and actively harmful in development: a
        // browser that sees HSTS on localhost will refuse plain HTTP for the
        // whole host afterwards.
        set(
            headers,
            header::STRICT_TRANSPORT_SECURITY,
            "max-age=31536000; includeSubDomains",
        );
    }

    response
}
