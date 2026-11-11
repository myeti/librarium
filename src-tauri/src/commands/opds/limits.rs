//! Resource limits, so a misbehaving client (or many) can't exhaust the app.
//!
//! | Limit ([`Limits`] field) | Default | When reached |
//! |---|---|---|
//! | Open connections (`max_connections`) | 64 | new connections wait in the OS accept queue until one closes |
//! | Idle connection, no byte read or written either way (`idle_timeout`) | 60 s | the connection is closed: stalled downloads, sleeping e-readers, slow-header attacks, idle keep-alives |
//! | Requests being answered at once, body included (`max_requests`) | 16 | the request waits up to `queue_timeout` (10 s), then gets `503` + `Retry-After` |
//! | Time to produce a response, until its headers (`response_timeout`) | 30 s | `503` |
//!
//! Page sizes are capped by the feeds themselves ([`super::catalog::MAX_PAGE_SIZE`]).
//!
//! Two layers, because axum's `serve` gives hyper no timer: hyper's own
//! header-read timeout never fires, and nothing ever closes a connection
//! whose client stopped reading. [`LimitedListener`] wraps each accepted
//! socket in an [`IdleTimeout`] that fails reads and writes after a silence,
//! which makes hyper drop the connection (and the response body with it).
//! [`limit_requests`] is an axum middleware: a semaphore slot per request,
//! held by the response body until it has been sent or dropped, so a long
//! download counts until it ends.

use std::future::Future as _;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use http_body::{Frame, SizeHint};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, Sleep};

/// See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Limits {
    pub max_connections: usize,
    pub idle_timeout: Duration,
    pub max_requests: usize,
    pub queue_timeout: Duration,
    pub response_timeout: Duration,
}

impl Limits {
    /// Generous for a personal server (an e-reader makes one request at a
    /// time; a browser opens about six connections), small enough to bound
    /// open files and threads. `idle_timeout` must stay above
    /// `response_timeout`: a connection is silent while its response is
    /// being prepared.
    pub const DEFAULT: Limits = Limits {
        max_connections: 64,
        idle_timeout: Duration::from_secs(60),
        max_requests: 16,
        queue_timeout: Duration::from_secs(10),
        response_timeout: Duration::from_secs(30),
    };
}

// ── Requests ─────────────────────────────────────────────────────────────────

/// Shared by every request of one server.
pub(super) struct RequestLimiter {
    slots: Arc<Semaphore>,
    limits: Limits,
}

impl RequestLimiter {
    pub fn new(limits: Limits) -> Self {
        RequestLimiter {
            slots: Arc::new(Semaphore::new(limits.max_requests)),
            limits,
        }
    }
}

/// The axum middleware (see the module docs).
pub(super) async fn limit_requests(
    State(limiter): State<Arc<RequestLimiter>>,
    request: Request,
    next: Next,
) -> Response {
    let slot = tokio::time::timeout(
        limiter.limits.queue_timeout,
        Arc::clone(&limiter.slots).acquire_owned(),
    )
    .await;
    // `Err(Closed)` can't happen (the semaphore is never closed); treat it as busy.
    let Ok(Ok(slot)) = slot else {
        let mut response = status_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many requests at once; try again shortly",
        );
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
        return response;
    };
    match tokio::time::timeout(limiter.limits.response_timeout, next.run(request)).await {
        Ok(response) => response.map(|body| Body::new(HoldingBody { body, _slot: slot })),
        Err(_) => status_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "The library took too long to answer",
        ),
    }
}

fn status_response(status: StatusCode, text: &'static str) -> Response {
    let mut response = Response::new(Body::from(text));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// A response body that keeps its request's slot until it is dropped (sent
/// completely, or abandoned with its connection).
struct HoldingBody {
    body: Body,
    _slot: OwnedSemaphorePermit,
}

impl http_body::Body for HoldingBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        Pin::new(&mut self.body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    // Forwarded so the length stays known (`Content-Length`, not chunked).
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

// ── Connections ──────────────────────────────────────────────────────────────

/// A TCP listener that accepts at most `max_connections` connections at a
/// time and closes idle ones (see the module docs).
pub(super) struct LimitedListener {
    listener: TcpListener,
    slots: Arc<Semaphore>,
    idle_timeout: Duration,
}

impl LimitedListener {
    pub fn new(listener: TcpListener, limits: &Limits) -> Self {
        LimitedListener {
            listener,
            slots: Arc::new(Semaphore::new(limits.max_connections)),
            idle_timeout: limits.idle_timeout,
        }
    }
}

impl axum::serve::Listener for LimitedListener {
    type Io = IdleTimeout<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        // A free slot first: while the server is full, new connections queue
        // in the OS backlog instead of being accepted and starved. (`None`
        // only if the semaphore were closed, which it never is.)
        let slot = Arc::clone(&self.slots).acquire_owned().await.ok();
        // axum's own `TcpListener` impl: retries (with a pause) on errors
        // such as "too many open files".
        let (stream, addr) = axum::serve::Listener::accept(&mut self.listener).await;
        (IdleTimeout::new(stream, self.idle_timeout, slot), addr)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

/// A connection that fails with [`io::ErrorKind::TimedOut`] once neither
/// reading nor writing has made progress for `timeout`, and holds its
/// connection slot until dropped.
pub(super) struct IdleTimeout<S> {
    inner: S,
    timeout: Duration,
    /// When the connection counts as idle; pushed back by every byte moved.
    deadline: Pin<Box<Sleep>>,
    _slot: Option<OwnedSemaphorePermit>,
}

impl<S> IdleTimeout<S> {
    pub fn new(inner: S, timeout: Duration, slot: Option<OwnedSemaphorePermit>) -> Self {
        IdleTimeout {
            inner,
            timeout,
            deadline: Box::pin(tokio::time::sleep(timeout)),
            _slot: slot,
        }
    }

    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    fn progressed(&mut self) {
        let next = Instant::now() + self.timeout;
        self.deadline.as_mut().reset(next);
    }

    /// For an operation that is pending: `Pending` (woken at the deadline
    /// at the latest), or the timeout error once the deadline has passed.
    fn pending_or_expired<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        match self.deadline.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "OPDS connection idle for too long",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for IdleTimeout<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(result) => {
                if buf.filled().len() > before {
                    this.progressed();
                }
                Poll::Ready(result)
            }
            Poll::Pending => this.pending_or_expired(cx),
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for IdleTimeout<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(cx, data) {
            Poll::Ready(result) => {
                if matches!(result, Ok(n) if n > 0) {
                    this.progressed();
                }
                Poll::Ready(result)
            }
            Poll::Pending => this.pending_or_expired(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write_vectored(cx, bufs) {
            Poll::Ready(result) => {
                if matches!(result, Ok(n) if n > 0) {
                    this.progressed();
                }
                Poll::Ready(result)
            }
            Poll::Pending => this.pending_or_expired(cx),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(result) => Poll::Ready(result),
            Poll::Pending => this.pending_or_expired(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_shutdown(cx) {
            Poll::Ready(result) => Poll::Ready(result),
            Poll::Pending => this.pending_or_expired(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use http_body::Body as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    fn slot() -> Option<OwnedSemaphorePermit> {
        Arc::new(Semaphore::new(1)).try_acquire_owned().ok()
    }

    #[tokio::test]
    async fn idle_connections_time_out() {
        let (client, server) = tokio::io::duplex(64);
        let mut server = IdleTimeout::new(server, Duration::from_millis(50), slot());
        let mut buf = [0u8; 8];
        let started = std::time::Instant::now();
        let err = server.read(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() >= Duration::from_millis(50));
        drop(client);
    }

    #[tokio::test]
    async fn activity_keeps_a_connection_alive() {
        let (mut client, server) = tokio::io::duplex(64);
        let mut server = IdleTimeout::new(server, Duration::from_millis(200), slot());
        let writer = tokio::spawn(async move {
            for _ in 0..6 {
                tokio::time::sleep(Duration::from_millis(60)).await;
                client.write_all(b"x").await.unwrap();
            }
            client
        });
        let mut buf = [0u8; 6];
        server
            .read_exact(&mut buf)
            .await
            .expect("360 ms of traffic, never 200 ms of silence");
        let _client = writer.await.unwrap();
    }

    #[tokio::test]
    async fn a_client_that_stops_reading_is_cut_off() {
        // 16-byte pipe: the server's writes block as soon as the client
        // stops reading, as they would on a stalled TCP connection.
        let (_client, server) = tokio::io::duplex(16);
        let mut server = IdleTimeout::new(server, Duration::from_millis(50), slot());
        let err = server.write_all(&[0u8; 1024]).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn connections_beyond_the_limit_wait() {
        let limits = Limits {
            max_connections: 1,
            ..Limits::DEFAULT
        };
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = tcp.local_addr().unwrap();
        let mut listener = LimitedListener::new(tcp, &limits);
        use axum::serve::Listener as _;

        let _a = TcpStream::connect(addr).await.unwrap();
        let _b = TcpStream::connect(addr).await.unwrap();
        let (first, _) = listener.accept().await;
        let waiting = tokio::time::timeout(Duration::from_millis(100), listener.accept()).await;
        assert!(
            waiting.is_err(),
            "the second connection waits while the first is open"
        );
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await;
        assert!(second.is_ok(), "accepted once the first closed");
    }

    fn router(limits: Limits) -> Router {
        Router::new()
            .route("/fast", get(|| async { "ok" }))
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                Arc::new(RequestLimiter::new(limits)),
                limit_requests,
            ))
    }

    async fn call(router: &Router, uri: &str) -> Response {
        router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn slow_responses_time_out() {
        let r = router(Limits {
            response_timeout: Duration::from_millis(50),
            ..Limits::DEFAULT
        });
        assert_eq!(
            call(&r, "/slow").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        let ok = call(&r, "/fast").await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(
            ok.into_body().size_hint().exact(),
            Some(2),
            "length still known"
        );
    }

    #[tokio::test]
    async fn a_response_holds_its_slot_until_its_body_is_gone() {
        let r = router(Limits {
            max_requests: 1,
            queue_timeout: Duration::from_millis(50),
            ..Limits::DEFAULT
        });
        let first = call(&r, "/fast").await;
        assert_eq!(first.status(), StatusCode::OK);
        // Its body hasn't been sent yet: the only slot is still taken.
        let busy = call(&r, "/fast").await;
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(busy.headers()[header::RETRY_AFTER], "5");
        let body = axum::body::to_bytes(first.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"ok");
        assert_eq!(
            call(&r, "/fast").await.status(),
            StatusCode::OK,
            "slot released"
        );
    }
}
