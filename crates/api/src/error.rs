//! HTTP error contract.
//!
//! Every failure leaves the API as the same JSON envelope, so the TypeScript
//! client has one shape to parse and one field (`code`) to branch on:
//!
//! ```json
//! { "code": "invalid_credentials", "message": "invalid credentials", "field": null }
//! ```

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use elrond_application::ApplicationError;
use elrond_application::ports::{BlobError, RenderError, RepositoryError};
use serde::Serialize;

/// Result alias for handlers.
pub type ApiResult<T> = Result<T, ApiError>;

/// A failure on the way out of a handler.
#[derive(Debug)]
pub enum ApiError {
    /// A use case failed.
    Application(ApplicationError),

    /// The request body was absent, malformed, or the wrong content type.
    MalformedRequest {
        /// Machine-stable reason code.
        code: &'static str,
        /// Human-readable explanation.
        message: String,
    },

    /// CSRF validation failed.
    CsrfRejected {
        /// Which check failed, for the client-facing message.
        reason: &'static str,
    },

    /// The caller exceeded a rate limit.
    RateLimited {
        /// Seconds until the caller may retry.
        retry_after_seconds: u64,
    },
}

impl From<ApplicationError> for ApiError {
    fn from(error: ApplicationError) -> Self {
        Self::Application(error)
    }
}

/// Domain validation failures reach handlers directly when a handler parses a
/// value object itself, so they route through the same mapping as a use-case
/// failure rather than getting a second, divergent status code.
impl From<elrond_domain::DomainError> for ApiError {
    fn from(error: elrond_domain::DomainError) -> Self {
        Self::Application(ApplicationError::Domain(error))
    }
}

/// Likewise for the few endpoints that read a repository directly because the
/// operation has no business rules beyond authorization.
impl From<RepositoryError> for ApiError {
    fn from(error: RepositoryError) -> Self {
        Self::Application(ApplicationError::Repository(error))
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        // Axum's own rejection renders as plain text, which would be the only
        // response in the API that is not JSON. It is translated here so clients
        // never need a second parsing path.
        let code = match rejection {
            JsonRejection::JsonSyntaxError(_) => "request_body_malformed_json",
            JsonRejection::MissingJsonContentType(_) => "request_content_type_invalid",
            JsonRejection::BytesRejection(_) => "request_body_unreadable",
            // Covers `JsonDataError` — valid JSON of the wrong shape — plus any
            // variant a future axum release adds.
            _ => "request_body_invalid",
        };
        Self::MalformedRequest {
            code,
            message: rejection.body_text(),
        }
    }
}

/// The JSON body of an error response.
#[derive(Debug, Serialize)]
struct ErrorBody {
    /// Machine-stable code.
    code: &'static str,
    /// Human-readable message, safe to display.
    message: String,
    /// Offending field, when the failure is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<&'static str>,
}

impl ApiError {
    /// The status this failure maps to.
    fn status(&self) -> StatusCode {
        match self {
            Self::Application(error) => application_status(error),
            Self::MalformedRequest { .. } => StatusCode::BAD_REQUEST,
            Self::CsrfRejected { .. } => StatusCode::FORBIDDEN,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        }
    }

    /// The machine-stable code clients branch on.
    fn code(&self) -> &'static str {
        match self {
            Self::Application(error) => error.code(),
            Self::MalformedRequest { code, .. } => code,
            Self::CsrfRejected { .. } => "csrf_check_failed",
            Self::RateLimited { .. } => "rate_limited",
        }
    }

    /// The client-facing message.
    fn message(&self) -> String {
        match self {
            // A storage or hashing failure must not describe itself. The detail
            // goes to the log; the client gets a generic sentence.
            Self::Application(
                error @ (ApplicationError::Repository(RepositoryError::Backend(_))
                | ApplicationError::Hashing(_)),
            ) => {
                tracing::error!(error = ?error, "internal failure while handling a request");
                "Something went wrong on the server. The failure has been logged.".to_owned()
            }
            Self::Application(error) => error.to_string(),
            Self::MalformedRequest { message, .. } => message.clone(),
            Self::CsrfRejected { reason } => {
                format!("This request was blocked by a cross-site request check ({reason}).")
            }
            Self::RateLimited {
                retry_after_seconds,
            } => format!("Too many attempts. Try again in {retry_after_seconds} seconds."),
        }
    }

    /// The offending field, when there is one.
    fn field(&self) -> Option<&'static str> {
        match self {
            Self::Application(error) => error.field(),
            _ => None,
        }
    }
}

/// Maps a use-case failure onto a status code.
///
/// Written as an exhaustive match so a new [`ApplicationError`] variant fails to
/// compile until someone decides what clients should see.
fn application_status(error: &ApplicationError) -> StatusCode {
    match error {
        // The request was well-formed JSON but broke a domain rule, which is
        // what 422 is for. A source document that cannot be read joins it: the
        // fault is in the library's content rather than in the request, but the
        // caller is the one who can act on it.
        ApplicationError::Domain(_)
        | ApplicationError::Render(
            RenderError::UnreadableSource { .. } | RenderError::EmptySource { .. },
        ) => StatusCode::UNPROCESSABLE_ENTITY,
        ApplicationError::Repository(RepositoryError::UniqueViolation { .. })
        | ApplicationError::SetupAlreadyCompleted
        | ApplicationError::Conflict { .. }
        | ApplicationError::Render(RenderError::EmptyPlan) => StatusCode::CONFLICT,
        ApplicationError::Repository(RepositoryError::Backend(_))
        | ApplicationError::Hashing(_)
        | ApplicationError::Storage(BlobError::Backend(_) | BlobError::IntegrityFailure { .. })
        | ApplicationError::Render(RenderError::Backend(_)) => StatusCode::INTERNAL_SERVER_ERROR,
        ApplicationError::Storage(BlobError::TooLarge { .. }) => StatusCode::PAYLOAD_TOO_LARGE,
        ApplicationError::InvalidCredentials | ApplicationError::NotAuthenticated => {
            StatusCode::UNAUTHORIZED
        }
        ApplicationError::AccountDisabled | ApplicationError::Forbidden { .. } => {
            StatusCode::FORBIDDEN
        }
        // Content recorded in the database but missing from the blob store is a
        // server-side inconsistency rather than a bad request, but 404 is what a
        // client can actually act on, so it shares the not-found mapping.
        ApplicationError::NotFound { .. }
        | ApplicationError::Storage(BlobError::NotFound { .. }) => StatusCode::NOT_FOUND,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ErrorBody {
            code: self.code(),
            message: self.message(),
            field: self.field(),
        };

        let mut response = (status, Json(body)).into_response();

        if let Self::RateLimited {
            retry_after_seconds,
        } = self
            && let Ok(value) = retry_after_seconds.to_string().parse()
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }

        // A 401 from an API endpoint must not make the browser pop up its native
        // basic-auth dialog, so no `WWW-Authenticate` challenge is sent.
        response
    }
}

#[cfg(test)]
mod tests {
    use elrond_domain::{DomainError, Role};

    use super::*;

    fn status_of(error: ApplicationError) -> StatusCode {
        ApiError::Application(error).status()
    }

    #[test]
    fn validation_failures_are_unprocessable_entity() {
        assert_eq!(
            status_of(ApplicationError::Domain(DomainError::Required {
                field: "email"
            })),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn credential_failures_are_unauthorized() {
        assert_eq!(
            status_of(ApplicationError::InvalidCredentials),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_of(ApplicationError::NotAuthenticated),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn authorization_failures_are_forbidden_not_unauthorized() {
        assert_eq!(
            status_of(ApplicationError::Forbidden {
                required: Role::Editor
            }),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_of(ApplicationError::AccountDisabled),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn state_conflicts_are_conflict() {
        assert_eq!(
            status_of(ApplicationError::SetupAlreadyCompleted),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_of(ApplicationError::Repository(
                RepositoryError::UniqueViolation {
                    resource: "user",
                    field: "email"
                }
            )),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn backend_failures_are_internal_and_do_not_describe_themselves() {
        #[derive(Debug, thiserror::Error)]
        #[error("connection to secret-host:5432 refused")]
        struct Leaky;

        let error = ApiError::Application(ApplicationError::Repository(RepositoryError::backend(
            Leaky,
        )));
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let message = error.message();
        assert!(
            !message.contains("secret-host"),
            "internal detail leaked to the client: {message}"
        );
    }

    #[test]
    fn the_offending_field_is_reported_for_validation_errors() {
        let error = ApiError::Application(ApplicationError::Domain(DomainError::TooShort {
            field: "password",
            min: 12,
        }));
        assert_eq!(error.field(), Some("password"));
    }

    #[test]
    fn rate_limiting_advertises_retry_after() {
        let response = ApiError::RateLimited {
            retry_after_seconds: 42,
        }
        .into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("42")
        );
    }

    #[test]
    fn errors_never_send_a_basic_auth_challenge() {
        let response = ApiError::Application(ApplicationError::NotAuthenticated).into_response();
        assert!(
            !response.headers().contains_key(header::WWW_AUTHENTICATE),
            "a browser would show its native login dialog"
        );
    }
}
