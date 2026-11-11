//! `http_get`: a small GET proxy for hosts the webview's CSP doesn't allow
//! (user-added OPDS catalogs, archive.org for the Open Library store).
//!
//! The webview can only reach the hosts listed in its CSP, but OPDS catalogs
//! are arbitrary user-supplied URLs. Rather than opening the CSP to every
//! host, the frontend asks Rust to fetch them. The proxy is deliberately
//! narrow: GET only, http/https only, bounded time and size, text out.
//!
//! The request itself is made by the shared HTTP layer ([`crate::net`]):
//! timeouts, redirect limit, User-Agent and URL validation live there. This
//! module only turns the command's arguments into a request (headers, size
//! cap) and the result into the IPC type.
//!
//! Private and loopback addresses are *not* blocked: a Calibre content server
//! on the LAN or on `localhost` is a legitimate catalog.

use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::net;

/// Body cap when the caller doesn't pass `maxBytes`.
pub const DEFAULT_MAX_BYTES: u64 = net::TEXT_LIMITS.max_bytes;
/// Largest `maxBytes` a caller may ask for.
pub const HARD_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// Result of [`http_get`] (camelCase over IPC, see docs/IPC.md).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    /// Body decoded as UTF-8; invalid sequences become U+FFFD.
    pub body: String,
    /// URL after redirects.
    pub final_url: String,
}

/// GET `url` and return its status, content type and body as text.
///
/// Non-2xx statuses resolve normally (the caller decides); invalid URLs,
/// network errors, timeouts and oversized bodies reject.
#[tauri::command]
pub async fn http_get(
    url: String,
    headers: Option<HashMap<String, String>>,
    max_bytes: Option<u64>,
) -> AppResult<HttpResponse> {
    let url = net::validate_url(&url)?;
    let headers = build_headers(headers.unwrap_or_default())?;
    let limits = net::TEXT_LIMITS.with_max_bytes(effective_cap(max_bytes));
    let response = net::get_text(&url, headers, limits).await?;
    Ok(HttpResponse {
        status: response.status,
        content_type: response.content_type,
        body: response.body,
        final_url: response.final_url,
    })
}

/// Body cap for a request: `max_bytes` if given (at least 1 byte, at most
/// [`HARD_MAX_BYTES`]), otherwise [`DEFAULT_MAX_BYTES`].
pub fn effective_cap(max_bytes: Option<u64>) -> u64 {
    max_bytes.map_or(DEFAULT_MAX_BYTES, |m| m.clamp(1, HARD_MAX_BYTES))
}

/// Turn the caller's header map into a `HeaderMap`, rejecting invalid names
/// or values. `Host` is managed by the client and ignored here.
fn build_headers(raw: HashMap<String, String>) -> AppResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in raw {
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| AppError::InvalidInput(format!("Invalid header name \"{name}\"")))?;
        if name == reqwest::header::HOST {
            continue;
        }
        let value = HeaderValue::from_str(&value)
            .map_err(|_| AppError::InvalidInput(format!("Invalid value for header \"{name}\"")))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;

    #[test]
    fn caps_default_and_clamp() {
        assert_eq!(effective_cap(None), DEFAULT_MAX_BYTES);
        assert_eq!(effective_cap(Some(1000)), 1000);
        assert_eq!(effective_cap(Some(0)), 1);
        assert_eq!(effective_cap(Some(u64::MAX)), HARD_MAX_BYTES);
    }

    #[test]
    fn headers_are_validated() {
        let ok = build_headers([("Accept".to_string(), "application/atom+xml".to_string())].into())
            .unwrap();
        assert_eq!(ok.get("accept").unwrap(), "application/atom+xml");
        let host = build_headers([("Host".to_string(), "evil".to_string())].into()).unwrap();
        assert!(host.is_empty(), "Host is ignored");
        assert!(build_headers([("Bad Name".to_string(), "x".to_string())].into()).is_err());
        assert!(build_headers([("X-Ok".to_string(), "line\nbreak".to_string())].into()).is_err());
    }

    /// The command end to end, against a local server: headers are sent,
    /// the cap applies, bad URLs reject. (Redirects, statuses and timeouts
    /// are covered by `net`'s tests.)
    #[tokio::test]
    async fn the_command_fetches_text_with_headers_and_a_cap() {
        let router = Router::new()
            .route(
                "/echo",
                get(|headers: axum::http::HeaderMap| async move {
                    headers
                        .get("accept")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string()
                }),
            )
            .route("/big", get(|| async { "x".repeat(2048) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let accept = HashMap::from([("Accept".to_string(), "application/atom+xml".to_string())]);
        let echo = http_get(format!("{base}/echo"), Some(accept), None)
            .await
            .unwrap();
        assert_eq!(
            (echo.status, echo.body.as_str()),
            (200, "application/atom+xml")
        );

        let err = http_get(format!("{base}/big"), None, Some(1024))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("too large"), "{err}");
        assert!(http_get("file:///etc/passwd".into(), None, None)
            .await
            .is_err());
    }
}
