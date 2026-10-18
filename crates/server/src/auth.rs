//! Login for Havre: the HTTP side.
//!
//! - `/api/auth/*` routes: first-run setup, sign in, sign out, password.
//! - [`require_session`]: a middleware in front of every other API route;
//!   without a valid session cookie the answer is `401`.
//! - [`same_origin`]: refuses requests that change something when they come
//!   from another website (CSRF protection, on top of `SameSite` cookies).
//!
//! The rules themselves (hashing, sessions) live in `havre_core::auth`.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Extension, Json, Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use havre_core::auth::{Auth, SESSION_DAYS, SessionInfo};
use serde::Deserialize;
use serde_json::json;

use crate::{AppState, api::ApiError};

/// The cookie that holds the session token.
const COOKIE: &str = "havre_session";
/// Wrong passwords allowed per address before waiting.
const MAX_FAILURES: u32 = 8;
const LOCKOUT: Duration = Duration::from_secs(15 * 60);

/// The session token of the current request, put there by [`require_session`].
#[derive(Clone)]
pub struct SessionToken(pub String);

/// Counts wrong passwords per address, to slow down password guessing.
#[derive(Clone, Default)]
pub struct Throttle(Arc<Mutex<HashMap<IpAddr, (u32, Instant)>>>);

impl Throttle {
    /// `Err(minutes)` while this address must wait.
    fn check(&self, ip: IpAddr) -> Result<(), u64> {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (_, since)| since.elapsed() < LOCKOUT);
        match map.get(&ip) {
            Some((failures, since)) if *failures >= MAX_FAILURES => {
                Err((LOCKOUT.saturating_sub(since.elapsed()).as_secs() / 60).max(1))
            }
            _ => Ok(()),
        }
    }

    fn failed(&self, ip: IpAddr) {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let entry = map.entry(ip).or_insert((0, Instant::now()));
        entry.0 += 1;
        entry.1 = Instant::now();
    }

    fn succeeded(&self, ip: IpAddr) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&ip);
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/status", get(status))
        .route("/auth/setup", post(setup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
}

/// Routes that need a session, like the rest of the API.
pub fn protected_routes() -> Router<AppState> {
    Router::new()
        .route("/auth/password", post(change_password))
        .route("/auth/sessions", get(sessions))
        .route("/auth/sessions/others", axum::routing::delete(sign_out_others))
}

/// `GET /api/auth/status` — what the web UI needs to decide between the
/// first-run screen, the sign-in screen and the app.
async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let Some(auth) = &state.auth else {
        // Demo mode: nothing to protect.
        return Ok(Json(json!({ "enabled": false, "setup_needed": false, "signed_in": true })));
    };
    let signed_in = match cookie_token(&headers) {
        Some(token) => auth.check_session(&token).await?,
        None => false,
    };
    Ok(Json(json!({
        "enabled": true,
        "setup_needed": !auth.is_set_up().await?,
        "signed_in": signed_in,
    })))
}

#[derive(Deserialize)]
struct PasswordBody {
    password: String,
}

/// `POST /api/auth/setup` — first run only: choose the password.
async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PasswordBody>,
) -> Result<Response, ApiError> {
    let auth = enabled(&state)?;
    let token = auth.set_up(&body.password, user_agent(&headers)).await?;
    tracing::info!("Havre password set");
    Ok(with_cookie(&headers, &token))
}

/// `POST /api/auth/login`
async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PasswordBody>,
) -> Result<Response, ApiError> {
    let auth = enabled(&state)?;
    let ip = peer.ip();
    if let Err(minutes) = state.throttle.check(ip) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": format!("Too many wrong passwords. Try again in {minutes} min.") })),
        )
            .into_response());
    }
    match auth.sign_in(&body.password, user_agent(&headers)).await {
        Ok(token) => {
            state.throttle.succeeded(ip);
            Ok(with_cookie(&headers, &token))
        }
        Err(err) => {
            state.throttle.failed(ip);
            tracing::warn!(%ip, "wrong Havre password");
            // A small pause makes guessing even slower.
            tokio::time::sleep(Duration::from_millis(400)).await;
            Err(err.into())
        }
    }
}

/// `POST /api/auth/logout` — ends this browser's session.
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let (Some(auth), Some(token)) = (&state.auth, cookie_token(&headers)) {
        auth.sign_out(&token).await?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    let expired = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    if let Ok(value) = HeaderValue::from_str(&expired) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    Ok(response)
}

#[derive(Deserialize)]
struct ChangePassword {
    current: String,
    new: String,
}

/// `POST /api/auth/password` — also signs out the other browsers.
async fn change_password(
    State(state): State<AppState>,
    Extension(SessionToken(token)): Extension<SessionToken>,
    Json(body): Json<ChangePassword>,
) -> Result<StatusCode, ApiError> {
    enabled(&state)?.change_password(&body.current, &body.new, &token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/auth/sessions` — signed-in browsers.
async fn sessions(
    State(state): State<AppState>,
    Extension(SessionToken(token)): Extension<SessionToken>,
) -> Result<Json<Vec<SessionInfo>>, ApiError> {
    Ok(Json(enabled(&state)?.sessions(&token).await?))
}

/// `DELETE /api/auth/sessions/others` — "sign out everywhere else".
async fn sign_out_others(
    State(state): State<AppState>,
    Extension(SessionToken(token)): Extension<SessionToken>,
) -> Result<StatusCode, ApiError> {
    enabled(&state)?.sign_out_others(&token).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Middlewares ---------------------------------------------------------------------

/// Lets the request through only with a valid session cookie.
pub async fn require_session(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let Some(auth) = &state.auth else {
        request.extensions_mut().insert(SessionToken(String::new()));
        return next.run(request).await;
    };
    let token = cookie_token(request.headers());
    let valid = match &token {
        Some(token) => auth.check_session(token).await.unwrap_or(false),
        None => false,
    };
    match token {
        Some(token) if valid => {
            request.extensions_mut().insert(SessionToken(token));
            next.run(request).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Sign in to Havre first.", "signed_out": true })),
        )
            .into_response(),
    }
}

/// Requests that change something (not GET) must come from Havre's own
/// pages: browsers send an `Origin` header, which must match the host.
pub async fn same_origin(request: Request, next: Next) -> Response {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        let headers = request.headers();
        if let (Some(origin), Some(host)) =
            (header_str(headers, header::ORIGIN), header_str(headers, header::HOST))
        {
            let origin_host = origin.split("://").nth(1).unwrap_or_default();
            let forwarded = header_str(headers, "x-forwarded-host");
            if origin_host != host && Some(origin_host) != forwarded {
                tracing::warn!(origin, host, "cross-site request refused");
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({ "error": "Request from another website refused." })),
                )
                    .into_response();
            }
        }
    }
    next.run(request).await
}

// ---- Helpers --------------------------------------------------------------------------

fn enabled(state: &AppState) -> Result<&Auth, ApiError> {
    state.auth.as_ref().ok_or_else(|| ApiError::BadRequest("No login in demo mode.".into()))
}

fn header_str<K: header::AsHeaderName>(headers: &HeaderMap, name: K) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn user_agent(headers: &HeaderMap) -> &str {
    header_str(headers, header::USER_AGENT).unwrap_or_default()
}

/// The session token from the `Cookie` header, if any.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value.to_owned())
        .filter(|v| !v.is_empty())
}

/// `204 No Content` that sets the session cookie:
/// - `HttpOnly`: JavaScript (and so an injected script) can't read it;
/// - `SameSite=Lax`: other websites can't make your browser send it with
///   their forms;
/// - `Secure` over HTTPS: never sent unencrypted.
fn with_cookie(headers: &HeaderMap, token: &str) -> Response {
    let https = header_str(headers, "x-forwarded-proto") == Some("https")
        || std::env::var("HAVRE_PUBLIC_URL").is_ok_and(|url| url.starts_with("https://"));
    let mut cookie =
        format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}", SESSION_DAYS * 86_400);
    if https {
        cookie.push_str("; Secure");
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_session_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static("theme=dark; havre_session=abc123; x=1"));
        assert_eq!(cookie_token(&headers).as_deref(), Some("abc123"));
        headers.insert(header::COOKIE, HeaderValue::from_static("havre_session="));
        assert_eq!(cookie_token(&headers), None);
    }

    #[test]
    fn throttles_after_too_many_failures() {
        let throttle = Throttle::default();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        for _ in 0..MAX_FAILURES {
            assert!(throttle.check(ip).is_ok());
            throttle.failed(ip);
        }
        assert!(throttle.check(ip).is_err());
        assert!(throttle.check("10.0.0.2".parse().unwrap()).is_ok());
        throttle.succeeded(ip);
        assert!(throttle.check(ip).is_ok());
    }
}
