//! Session and CSRF cookies.

use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration;

use crate::config::ApiConfig;

/// Name of the session cookie.
///
/// Prefixed so this deployment cannot collide with another Elrond build on the
/// same host: browsers scope cookies by host and ignore the port, so two
/// implementations on `localhost:3000` and `localhost:3100` would otherwise
/// overwrite each other's sessions.
pub const SESSION_COOKIE: &str = "elrond_alt_session";

/// Name of the CSRF cookie. Readable by scripts on purpose.
pub const CSRF_COOKIE: &str = "elrond_alt_csrf";

/// Header the client echoes the CSRF cookie back in.
pub const CSRF_HEADER: &str = "x-elrond-csrf";

/// Builds the session cookie.
pub fn session_cookie<'a>(value: String, max_age: Duration, config: &ApiConfig) -> Cookie<'a> {
    let mut cookie = Cookie::new(SESSION_COOKIE, value);
    cookie.set_path("/");
    // Unreadable by scripts, so an XSS bug cannot exfiltrate a live session.
    cookie.set_http_only(true);
    // Lax rather than Strict: Strict would drop the cookie when a user follows a
    // link to a document from an email or chat client, which for a document
    // library is a routine way to arrive. CSRF is covered separately by the
    // double-submit token and the origin check.
    cookie.set_same_site(SameSite::Lax);
    cookie.set_secure(config.secure_cookies);
    cookie.set_max_age(max_age);
    cookie
}

/// Builds the CSRF cookie.
pub fn csrf_cookie<'a>(value: String, max_age: Duration, config: &ApiConfig) -> Cookie<'a> {
    let mut cookie = Cookie::new(CSRF_COOKIE, value);
    cookie.set_path("/");
    // Deliberately readable: the client must be able to copy it into a request
    // header, which is precisely what a cross-site attacker cannot do.
    cookie.set_http_only(false);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_secure(config.secure_cookies);
    cookie.set_max_age(max_age);
    cookie
}

/// Builds a cookie that clears `name`.
///
/// The attributes must match the original or some browsers keep the old cookie
/// alongside the expired one.
pub fn expired_cookie<'a>(name: &'static str, config: &ApiConfig) -> Cookie<'a> {
    let mut cookie = Cookie::new(name, "");
    cookie.set_path("/");
    cookie.set_http_only(name == SESSION_COOKIE);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_secure(config.secure_cookies);
    cookie.set_max_age(Duration::ZERO);
    cookie
}

/// Compares two secrets without leaking their contents through timing.
///
/// A naive `==` on strings short-circuits at the first differing byte, which lets
/// an attacker recover a token one byte at a time by measuring responses.
pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    // Length is not secret; token length is fixed and publicly known.
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(secure: bool) -> ApiConfig {
        ApiConfig {
            secure_cookies: secure,
            ..ApiConfig::development()
        }
    }

    #[test]
    fn the_session_cookie_is_not_readable_by_scripts() {
        let cookie = session_cookie("token".to_owned(), Duration::hours(1), &config(false));
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.path(), Some("/"));
    }

    #[test]
    fn the_csrf_cookie_is_readable_by_scripts() {
        let cookie = csrf_cookie("token".to_owned(), Duration::hours(1), &config(false));
        assert_eq!(cookie.http_only(), Some(false));
    }

    #[test]
    fn cookies_are_marked_secure_when_configured() {
        assert_eq!(
            session_cookie("t".to_owned(), Duration::hours(1), &config(true)).secure(),
            Some(true)
        );
        assert_eq!(
            csrf_cookie("t".to_owned(), Duration::hours(1), &config(true)).secure(),
            Some(true)
        );
        assert_eq!(
            session_cookie("t".to_owned(), Duration::hours(1), &config(false)).secure(),
            Some(false)
        );
    }

    #[test]
    fn cookie_names_are_namespaced_to_this_build() {
        // Guards against a rename that would let a co-hosted Elrond build clobber
        // this one's session.
        assert!(SESSION_COOKIE.starts_with("elrond_alt_"));
        assert!(CSRF_COOKIE.starts_with("elrond_alt_"));
        assert_ne!(SESSION_COOKIE, CSRF_COOKIE);
    }

    #[test]
    fn clearing_a_cookie_matches_the_original_attributes() {
        let config = config(true);
        let live = session_cookie("t".to_owned(), Duration::hours(1), &config);
        let cleared = expired_cookie(SESSION_COOKIE, &config);
        assert_eq!(cleared.path(), live.path());
        assert_eq!(cleared.secure(), live.secure());
        assert_eq!(cleared.same_site(), live.same_site());
        assert_eq!(cleared.http_only(), live.http_only());
        assert_eq!(cleared.max_age(), Some(Duration::ZERO));
        assert_eq!(cleared.value(), "");
    }

    #[test]
    fn constant_time_comparison_matches_equality_semantics() {
        assert!(constant_time_eq("", ""));
        assert!(constant_time_eq("abcdef", "abcdef"));
        assert!(!constant_time_eq("abcdef", "abcdeg"));
        assert!(!constant_time_eq("abcdef", "abcde"));
        assert!(!constant_time_eq("abc", "abcdef"));
    }
}
