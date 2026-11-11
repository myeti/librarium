//! The one HTTP layer. Every request the backend makes goes through here
//! (except the updater plugin's, which brings its own client on the same
//! reqwest + rustls stack).
//!
//! What every request gets:
//! - **one shared client** ([`client`]): connection pooling, one TLS setup
//!   (rustls with the `ring` provider, certificates checked against the OS
//!   store), a connect timeout, expiry of idle pooled connections, at most
//!   [`MAX_REDIRECTS`] redirects, and the User-Agent `Librarium/<version>`;
//! - **per-request [`Limits`]**: an overall deadline, an idle timeout (the
//!   longest silence tolerated while waiting for the headers or between two
//!   body chunks) and a byte cap. The cap is enforced while streaming:
//!   `Content-Length` is only used to fail early, never trusted;
//! - **`http`/`https` only** ([`validate_url`]). Private and loopback
//!   addresses are allowed on purpose: a Calibre content server on the LAN is
//!   a legitimate OPDS catalog (and the tests use local servers).
//!
//! Three ways to read a body:
//! - [`get_bytes`]: into memory, 2xx only (covers);
//! - [`download_to_temp`]: streamed into a temp file, fsynced, 2xx only
//!   (books);
//! - [`get_text`]: any status, body decoded as UTF-8 (the `http_get` proxy).
//!
//! Error messages name the host, never the full URL: query strings can carry
//! API keys or tokens, and these messages reach the UI and the debug log.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::header::{HeaderMap, CONTENT_TYPE};
use reqwest::{Client, Response, Url};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, AppResult};

/// Sent with every request.
pub(crate) const USER_AGENT: &str = concat!("Librarium/", env!("CARGO_PKG_VERSION"));
/// Longest wait for a TCP + TLS connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Pooled connections unused for this long are closed.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Redirects followed before giving up (archive.org and Gutenberg downloads
/// take one or two).
pub(crate) const MAX_REDIRECTS: usize = 10;

/// Limits for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Largest accepted body, in bytes.
    pub max_bytes: u64,
    /// Deadline for the whole request: connection, redirects, headers, body.
    pub total: Duration,
    /// Longest silence tolerated while waiting for the response headers or
    /// between two body chunks. Catches stalled transfers long before
    /// `total` on big downloads.
    pub idle: Duration,
}

impl Limits {
    /// The same limits with another byte cap.
    pub(crate) const fn with_max_bytes(self, max_bytes: u64) -> Self {
        Limits { max_bytes, ..self }
    }
}

/// Covers: images are small; 20 MB is generous for a high-resolution scan.
pub(crate) const COVER_LIMITS: Limits = Limits {
    max_bytes: 20 * 1024 * 1024,
    total: Duration::from_secs(30),
    idle: Duration::from_secs(30),
};

/// Books: large illustrated PDFs reach a few hundred MB. Slow links need a
/// long overall deadline; the idle timeout catches stalled transfers.
pub(crate) const BOOK_LIMITS: Limits = Limits {
    max_bytes: 300 * 1024 * 1024,
    total: Duration::from_secs(30 * 60),
    idle: Duration::from_secs(60),
};

/// Catalog feeds and API answers (`http_get`). Callers usually lower or
/// raise `max_bytes` ([`Limits::with_max_bytes`]).
pub(crate) const TEXT_LIMITS: Limits = Limits {
    max_bytes: 5 * 1024 * 1024,
    total: Duration::from_secs(20),
    idle: Duration::from_secs(20),
};

// ── Client ────────────────────────────────────────────────────────────────────

/// The shared client, built on first use.
pub(crate) fn client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let built = build_client()?;
    // If two calls raced, both clients are equivalent; keep the first.
    Ok(CLIENT.get_or_init(|| built))
}

fn build_client() -> Result<Client, String> {
    install_crypto_provider();
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(POOL_IDLE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .map_err(|e| format!("Cannot initialise the HTTP client: {}", error_chain(&e)))
}

/// reqwest is built without a bundled crypto provider (`rustls-no-provider`)
/// so that it shares `ring` with the updater plugin instead of compiling a
/// second one. A process-wide default must therefore exist before a client
/// is built; the updater does the same check before its own requests.
fn install_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        // Only fails if another thread installed a provider meanwhile: fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

// ── URLs and errors ───────────────────────────────────────────────────────────

/// Accept only absolute `http://` and `https://` URLs with a host; reject
/// `file:`, `data:`, `javascript:` and everything else
/// ([`AppError::InvalidInput`]).
pub(crate) fn validate_url(raw: &str) -> AppResult<Url> {
    let invalid = |why: String| AppError::InvalidInput(format!("Invalid URL: {why}"));
    let url = Url::parse(raw.trim()).map_err(|e| invalid(e.to_string()))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(AppError::InvalidInput(format!(
                "Only http and https URLs are allowed (got \"{other}:\")"
            )))
        }
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid("missing host".into()));
    }
    Ok(url)
}

/// The host of `url`, for error messages.
fn host(url: &Url) -> &str {
    url.host_str().unwrap_or("the server")
}

/// The host of the URL `raw` for a log line (`"an invalid URL"` when it
/// doesn't parse): log lines go to the log file, and query strings can
/// carry keys or tokens.
pub(crate) fn host_of(raw: &str) -> String {
    Url::parse(raw.trim())
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "an invalid URL".to_string())
}

/// A reqwest error and its causes, without the URL. reqwest's own message is
/// often just "error sending request"; the useful part ("connection
/// refused", "dns error"…) is in the source chain.
fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = Vec::new();
    let mut current = Some(e);
    while let Some(err) = current {
        let text = err.to_string();
        // reqwest appends " for url (…)"; drop it (see the module docs).
        let text = text.split(" for url (").next().unwrap_or(&text).to_string();
        if !parts.contains(&text) {
            parts.push(text);
        }
        current = err.source();
    }
    parts.join(": ")
}

/// A user-facing message for a failed request.
fn describe(url: &Url, e: &reqwest::Error, limits: &Limits) -> String {
    if e.is_connect() {
        // Includes connect timeouts, which `is_timeout` would misreport as
        // the overall deadline.
        format!("Could not connect to {}: {}", host(url), error_chain(e))
    } else if e.is_timeout() {
        format!(
            "{} did not respond within {} seconds",
            host(url),
            limits.total.as_secs()
        )
    } else if e.is_redirect() {
        format!("{} redirected too many times", host(url))
    } else {
        format!("Request to {} failed: {}", host(url), error_chain(e))
    }
}

fn stalled(url: &Url, limits: &Limits) -> String {
    format!(
        "{} stopped responding for {} seconds",
        host(url),
        limits.idle.as_secs()
    )
}

/// `n` bytes in a readable unit, for "too large" messages.
fn human_size(n: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if n >= MB {
        format!("{} MB", n / MB)
    } else if n >= 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} bytes")
    }
}

fn too_large(cap: u64) -> String {
    format!("The response is too large (limit {})", human_size(cap))
}

// ── Requests ──────────────────────────────────────────────────────────────────

/// Send a GET and wait for the response headers (at most `limits.idle`).
async fn send(url: &Url, headers: HeaderMap, limits: &Limits) -> Result<Response, String> {
    let request = client()?
        .get(url.clone())
        .headers(headers)
        .timeout(limits.total)
        .send();
    match tokio::time::timeout(limits.idle, request).await {
        Ok(result) => result.map_err(|e| describe(url, &e, limits)),
        Err(_) => Err(stalled(url, limits)),
    }
}

/// [`send`], then refuse error statuses (an error page must never be saved
/// as a cover or a book) and bodies announced as larger than the cap.
async fn send_expecting_success(url: &Url, limits: &Limits) -> Result<Response, String> {
    let response = send(url, HeaderMap::new(), limits).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Download failed: {} answered HTTP {status}",
            host(url)
        ));
    }
    if response
        .content_length()
        .is_some_and(|len| len > limits.max_bytes)
    {
        return Err(too_large(limits.max_bytes));
    }
    Ok(response)
}

/// The next body chunk, or `None` at the end. Fails after `limits.idle` of
/// silence.
async fn next_chunk(
    response: &mut Response,
    url: &Url,
    limits: &Limits,
) -> Result<Option<impl std::ops::Deref<Target = [u8]>>, String> {
    match tokio::time::timeout(limits.idle, response.chunk()).await {
        Ok(result) => result.map_err(|e| describe(url, &e, limits)),
        Err(_) => Err(stalled(url, limits)),
    }
}

/// Accumulates a response body and fails as soon as it would exceed `cap`.
#[derive(Debug)]
pub(crate) struct CappedBody {
    cap: u64,
    buf: Vec<u8>,
}

impl CappedBody {
    /// An empty body that may grow to `cap` bytes.
    pub(crate) fn new(cap: u64) -> Self {
        CappedBody {
            cap,
            buf: Vec::new(),
        }
    }

    /// Fail before downloading anything when the server announces a larger body.
    pub(crate) fn check_announced(&self, content_length: Option<u64>) -> Result<(), String> {
        match content_length {
            Some(len) if len > self.cap => Err(too_large(self.cap)),
            _ => Ok(()),
        }
    }

    /// Append a chunk, or fail if the total would exceed the cap. Servers may
    /// lie about (or omit) `Content-Length`, so this is the real check.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<(), String> {
        if self.buf.len() as u64 + chunk.len() as u64 > self.cap {
            return Err(too_large(self.cap));
        }
        self.buf.extend_from_slice(chunk);
        Ok(())
    }

    /// The body as text, invalid UTF-8 replaced by U+FFFD.
    pub(crate) fn into_text(self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }

    /// The raw body.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// GET `url` into memory. Only 2xx responses are accepted; the body is capped
/// at `limits.max_bytes`. Failures are [`AppError::Network`] (an invalid URL
/// is [`AppError::InvalidInput`]).
pub(crate) async fn get_bytes(url: &str, limits: Limits) -> AppResult<Vec<u8>> {
    let url = validate_url(url)?;
    let read = async {
        let mut response = send_expecting_success(&url, &limits).await?;
        let mut body = CappedBody::new(limits.max_bytes);
        while let Some(chunk) = next_chunk(&mut response, &url, &limits).await? {
            body.push(&chunk)?;
        }
        Ok::<_, String>(body.into_bytes())
    };
    read.await.map_err(AppError::Network)
}

/// A body saved by [`download_to_temp`].
#[derive(Debug)]
pub(crate) struct Downloaded {
    /// The temp file, deleted when dropped (unless persisted).
    pub path: tempfile::TempPath,
    /// Its size in bytes.
    pub bytes: u64,
}

/// GET `url` and stream the body into a uniquely named, hidden temp file
/// (`.download-*.part`) in `dir`, created if needed. Only 2xx responses are
/// accepted; the body is capped at `limits.max_bytes`. The file is fsynced
/// before it is returned. On any error nothing is left behind.
///
/// Put `dir` on the same volume as the final destination, so that keeping
/// the download is a rename.
pub(crate) async fn download_to_temp(
    url: &str,
    dir: &Path,
    limits: Limits,
) -> AppResult<Downloaded> {
    let url = validate_url(url)?;
    let mut response = send_expecting_success(&url, &limits)
        .await
        .map_err(AppError::Network)?;

    let save_error = |e: std::io::Error| AppError::io("Could not save the download", &e);
    tokio::fs::create_dir_all(dir).await.map_err(save_error)?;
    // Unique name: two downloads of "book.epub" can't overwrite each other.
    // Declared before `out` so it is dropped (deleted) after the handle closes.
    let temp = tempfile::Builder::new()
        .prefix(".download-")
        .suffix(".part")
        .tempfile_in(dir)
        .map_err(|e| AppError::io("Could not create a temporary file", &e))?;
    let handle = temp.as_file().try_clone().map_err(save_error)?;
    // tokio's File writes on the blocking pool (the library may be on a NAS);
    // the buffer turns thousands of small chunks into a few large writes.
    let mut out =
        tokio::io::BufWriter::with_capacity(1024 * 1024, tokio::fs::File::from_std(handle));

    let mut written = 0u64;
    while let Some(chunk) = next_chunk(&mut response, &url, &limits)
        .await
        .map_err(AppError::Network)?
    {
        written += chunk.len() as u64;
        if written > limits.max_bytes {
            return Err(AppError::Network(too_large(limits.max_bytes)));
        }
        out.write_all(&chunk).await.map_err(save_error)?;
    }
    out.flush().await.map_err(save_error)?;
    // `fsutil::sync_file`, not tokio's `sync_all`: on macOS the latter is
    // F_FULLFSYNC, which an SMB share may refuse (see `sync_file`). Blocking,
    // so on the blocking pool; `into_std` waits for the writes in flight.
    let file = out.into_inner().into_std().await;
    tokio::task::spawn_blocking(move || crate::fsutil::sync_file(&file))
        .await
        .map_err(AppError::from)?
        .map_err(save_error)?;

    Ok(Downloaded {
        path: temp.into_temp_path(),
        bytes: written,
    })
}

/// A response read as text by [`get_text`].
#[derive(Debug)]
pub(crate) struct TextResponse {
    pub status: u16,
    pub content_type: Option<String>,
    /// Body decoded as UTF-8; invalid sequences become U+FFFD.
    pub body: String,
    /// URL after redirects.
    pub final_url: String,
}

/// GET `url` with extra `headers` and read the body as text, capped at
/// `limits.max_bytes`. Unlike [`get_bytes`], non-2xx statuses resolve
/// normally (the caller decides); network errors, timeouts and oversized
/// bodies reject.
pub(crate) async fn get_text(
    url: &Url,
    headers: HeaderMap,
    limits: Limits,
) -> AppResult<TextResponse> {
    let read = async {
        let mut response = send(url, headers, &limits).await?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        let mut body = CappedBody::new(limits.max_bytes);
        body.check_announced(response.content_length())?;
        while let Some(chunk) = next_chunk(&mut response, url, &limits).await? {
            body.push(&chunk)?;
        }
        Ok::<_, String>(TextResponse {
            status,
            content_type,
            body: body.into_text(),
            final_url,
        })
    };
    read.await.map_err(AppError::Network)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::{IntoResponse, Redirect};
    use axum::routing::get;
    use axum::Router;

    /// Serve `router` on an ephemeral localhost port; returns its base URL.
    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    /// Routes shared by the tests below.
    fn routes() -> Router {
        Router::new()
            .route("/ok", get(|| async { "hello" }))
            .route(
                "/feed",
                get(|| async { ([("content-type", "application/atom+xml")], "<feed/>") }),
            )
            .route("/big", get(|| async { vec![b'x'; 4096] }))
            .route(
                "/missing",
                get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope").into_response() }),
            )
            .route("/moved", get(|| async { Redirect::temporary("/feed") }))
            .route("/loop", get(|| async { Redirect::temporary("/loop") }))
            .route(
                "/ua",
                get(|headers: axum::http::HeaderMap| async move {
                    headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string()
                }),
            )
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
    }

    const SMALL: Limits = Limits {
        max_bytes: 1024,
        total: Duration::from_secs(2),
        idle: Duration::from_secs(1),
    };

    #[test]
    fn only_http_and_https_urls_are_accepted() {
        assert!(validate_url("https://standardebooks.org/feeds/opds").is_ok());
        assert!(validate_url("  http://192.168.1.10:8080/opds  ").is_ok());
        assert!(
            validate_url("HTTP://EXAMPLE.COM/").is_ok(),
            "schemes are case-insensitive"
        );
        for bad in [
            "file:///etc/passwd",
            "data:text/plain,hello",
            "javascript:alert(1)",
            "ftp://example.com/book.epub",
            "about:blank",
            "/relative/path",
            "example.com",
            "http://",
            "",
        ] {
            assert!(validate_url(bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn capped_body_rejects_overflow() {
        let mut body = CappedBody::new(10);
        assert!(body.check_announced(Some(10)).is_ok());
        assert!(body.check_announced(Some(11)).is_err());
        assert!(body.check_announced(None).is_ok());
        body.push(b"hello").unwrap();
        body.push(b"world").unwrap();
        assert!(body.push(b"!").is_err(), "11th byte");
        assert_eq!(body.into_text(), "helloworld");
    }

    #[test]
    fn body_is_utf8_lossy() {
        let mut body = CappedBody::new(10);
        body.push(b"caf\xc3\xa9 \xff").unwrap();
        assert_eq!(body.into_text(), "café \u{fffd}");
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human_size(1000), "1000 bytes");
        assert_eq!(human_size(4096), "4 KB");
        assert_eq!(human_size(300 * 1024 * 1024), "300 MB");
    }

    #[tokio::test]
    async fn get_bytes_enforces_status_size_and_time() {
        let base = serve(routes()).await;
        assert_eq!(
            get_bytes(&format!("{base}/ok"), SMALL).await.unwrap(),
            b"hello"
        );
        assert_eq!(
            get_bytes(&format!("{base}/ua"), SMALL).await.unwrap(),
            USER_AGENT.as_bytes()
        );

        let err = get_bytes(&format!("{base}/big"), SMALL)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("too large (limit 1 KB)"), "{err}");
        let err = get_bytes(&format!("{base}/missing"), SMALL)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("404"), "{err}");
        let err = get_bytes(&format!("{base}/slow"), SMALL)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("stopped responding for 1 seconds"), "{err}");
        let err = get_bytes(&format!("{base}/loop"), SMALL)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("redirected too many times"), "{err}");
    }

    #[tokio::test]
    async fn the_total_deadline_applies_too() {
        let base = serve(routes()).await;
        let strict = Limits {
            total: Duration::from_millis(300),
            idle: Duration::from_secs(10),
            ..SMALL
        };
        let err = get_bytes(&format!("{base}/slow"), strict)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not respond within"), "{err}");
    }

    #[tokio::test]
    async fn rejects_non_http_urls() {
        assert!(get_bytes("file:///etc/passwd", SMALL)
            .await
            .unwrap_err()
            .to_string()
            .contains("http and https"));
        assert!(get_bytes("not a url", SMALL).await.is_err());
    }

    #[tokio::test]
    async fn connection_errors_name_the_host_not_the_url() {
        // A port nothing listens on: bind, note the port, close.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = get_bytes(&format!("http://127.0.0.1:{port}/book?key=SECRET"), SMALL)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("127.0.0.1"), "{err}");
        assert!(!err.contains("SECRET"), "{err}");
    }

    #[tokio::test]
    async fn downloads_stream_into_a_temp_file_that_vanishes_on_error() {
        let base = serve(routes()).await;
        let dir = tempfile::tempdir().unwrap();
        let temp_dir = dir.path().join(".librarium");

        let ok = download_to_temp(&format!("{base}/ok"), &temp_dir, SMALL)
            .await
            .unwrap();
        assert_eq!(ok.bytes, 5);
        assert_eq!(std::fs::read(&ok.path).unwrap(), b"hello");
        let name = ok.path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with(".download-") && name.ends_with(".part"),
            "{name}"
        );
        drop(ok);

        for path in ["/big", "/missing", "/slow"] {
            assert!(download_to_temp(&format!("{base}{path}"), &temp_dir, SMALL)
                .await
                .is_err());
        }
        let left: Vec<_> = std::fs::read_dir(&temp_dir).unwrap().collect();
        assert!(left.is_empty(), "no partial download left behind: {left:?}");
    }

    #[tokio::test]
    async fn text_resolves_any_status_follows_redirects_and_caps() {
        let base = serve(routes()).await;
        let text = |path: &str, limits: Limits| {
            let url = validate_url(&format!("{base}{path}")).unwrap();
            async move { get_text(&url, HeaderMap::new(), limits).await }
        };
        let feed = text("/feed", TEXT_LIMITS).await.unwrap();
        assert_eq!((feed.status, feed.body.as_str()), (200, "<feed/>"));
        assert_eq!(feed.content_type.as_deref(), Some("application/atom+xml"));

        let missing = text("/missing", TEXT_LIMITS).await.unwrap();
        assert_eq!(missing.status, 404, "non-2xx statuses resolve");

        let moved = text("/moved", TEXT_LIMITS).await.unwrap();
        assert_eq!(moved.final_url, format!("{base}/feed"));
        assert_eq!(moved.body, "<feed/>");

        let err = text("/big", TEXT_LIMITS.with_max_bytes(1024))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("too large"), "{err}");

        let err = text("/loop", TEXT_LIMITS).await.unwrap_err().to_string();
        assert!(err.contains("redirected too many times"), "{err}");
    }
}
