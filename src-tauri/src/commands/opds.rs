//! Built-in OPDS catalog server, so reading apps can browse and download the
//! library: **OPDS 1.2** (Atom, what KOReader, Moon+ Reader and most e-reader
//! apps speak) and **OPDS 2.0** (JSON, for Readium-based apps).
//!
//! # URLs
//!
//! Users type `http://<address>:<port>/opds` (docs/IPC.md has the table for
//! users). Every link inside the feeds is path-absolute.
//!
//! | Path | What |
//! |---|---|
//! | `/opds` (also `/`, `/opds/`) | root navigation: All books, Recently added, By author, By series; search links. Atom, or OPDS 2 JSON when the `Accept` header prefers it ([`prefers_opds2`]) |
//! | `/opds/all`, `/opds/recent` | every book by title / newest first (acquisition, paginated) |
//! | `/opds/authors`, `/opds/series` | authors / series (navigation, paginated) |
//! | `/opds/authors/{name}`, `/opds/series/{name}` | one author's books / one series in order (acquisition, paginated) |
//! | `/opds/search?q=` | search across title, subtitle, authors, series, ISBN (acquisition, paginated) |
//! | `/opds/opensearch.xml` | OpenSearch description of the search |
//! | `/opds/books/{id}` | one book's Atom entry document |
//! | `/opds/v2` | OPDS 2 root |
//! | `/opds/publications?q=` | OPDS 2 publications feed (paginated, searchable) |
//! | `/opds/publications/{id}` | one OPDS 2 publication |
//! | `/opds/publications/{id}/download` | the book file: streamed, `Range` requests, `Content-Disposition` |
//! | `/opds/publications/{id}/cover`, `…/thumbnail` | `.covers/<id>.*` and `.covers/thumbs/<id>.jpg` (each falls back to the other) |
//!
//! Paginated feeds take `?page=N` (from 1) and `?limit=N` (default
//! [`catalog::DEFAULT_PAGE_SIZE`], at most [`catalog::MAX_PAGE_SIZE`]).
//!
//! # Security model (docs/IPC.md)
//!
//! - **Local mode (default)** binds `127.0.0.1` only: nothing else on the
//!   network can connect.
//! - **LAN mode (opt-in)** binds `0.0.0.0` and every request needs HTTP Basic
//!   auth (user [`access::OPDS_USER`], the password the user chose).
//! - **No CORS headers**, so a web page open in a browser can't read the
//!   catalog through the user's machine.
//! - **The Host header is validated**: only `localhost`/loopback addresses,
//!   plus (in LAN mode) the IP the client actually connected to. This defeats
//!   DNS rebinding, where `evil.example` is re-resolved to `127.0.0.1` and the
//!   browser then sends `Host: evil.example`.
//! - Both checks ([`access::guard`]) run before every route, the fallback
//!   included; then the resource limits ([`limits`]).
//! - Files are only served from inside the library folder; covers only from
//!   `.covers/`, never from a path stored in the index.
//! - The index is only read ([`catalog`]): never locked, migrated or written.
//!
//! # Modules
//!
//! | Module | Responsibility |
//! |---|---|
//! | [`catalog`] | the library as the feeds see it; search, pagination; the self-invalidating cache |
//! | [`atom`] | OPDS 1.2 feeds, entries, OpenSearch description (quick-xml) |
//! | [`json`] | OPDS 2.0 documents |
//! | [`files`] | streaming downloads and covers (Range), MIME types, `Content-Disposition` |
//! | [`access`] | Host check, Basic auth, the LAN address |
//! | [`limits`] | connection, concurrency and time limits |
//! | [`urls`] | the URL space and media types |
//! | [`qr`] | QR codes of catalog URLs |

use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, Request, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

use crate::error::{AppError, AppResult};
use crate::fsutil;
use crate::logging::log;

mod access;
mod atom;
mod catalog;
mod files;
mod json;
mod limits;
mod qr;
mod urls;

use access::{Access, ConnInfo};
use atom::{Entries, Feed, Kind, NavEntry};
use catalog::{
    fold, paginate, Catalog, CatalogCache, Entry, Group, PageRequest, DEFAULT_PAGE_SIZE,
    MAX_QUERY_CHARS,
};
use limits::{LimitedListener, Limits, RequestLimiter};

/// How long a stopping server may take to finish in-flight requests before
/// it is aborted, so a stalled download can't block a restart.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

// ── Public types ─────────────────────────────────────────────────────────────

/// Returned by `start_opds` and `opds_status` (docs/IPC.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpdsStatus {
    pub running: bool,
    pub port: u16,
    /// Catalog URL on this machine.
    pub local_url: String,
    /// Catalog URLs for other devices; empty unless sharing on the LAN.
    pub lan_urls: Vec<String>,
}

/// What the server was started with; a `start_opds` call with identical
/// settings doesn't restart it. (No `Debug`: it holds the password.)
#[derive(Clone, PartialEq, Eq)]
pub struct OpdsSettings {
    pub port: u16,
    pub library_root: PathBuf,
    pub share_on_lan: bool,
    /// Only kept in LAN mode (it is meaningless otherwise).
    pub password: Option<String>,
}

impl OpdsSettings {
    pub fn new(
        port: u16,
        library_root: impl Into<PathBuf>,
        share_on_lan: bool,
        password: Option<String>,
    ) -> Self {
        OpdsSettings {
            port,
            library_root: library_root.into(),
            share_on_lan,
            password: if share_on_lan { password } else { None },
        }
    }
}

// ── Tauri-managed state: the server's lifecycle ──────────────────────────────

/// The running server (if any) and the shared catalog cache.
pub struct OpdsServerState {
    /// An async mutex: held across the stop → rebind sequence so concurrent
    /// `start_opds`/`stop_opds` calls are applied one at a time.
    server: Mutex<Option<RunningServer>>,
    cache: Arc<CatalogCache>,
}

impl Default for OpdsServerState {
    fn default() -> Self {
        Self {
            server: Mutex::new(None),
            cache: Arc::new(CatalogCache::default()),
        }
    }
}

struct RunningServer {
    settings: OpdsSettings,
    status: OpdsStatus,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl RunningServer {
    /// Ask the server to stop and wait until it has released its port
    /// (aborting it after [`SHUTDOWN_GRACE`]).
    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if tokio::time::timeout(SHUTDOWN_GRACE, &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
    }
}

impl OpdsServerState {
    /// Start (or restart) the server. Returns the current status without
    /// restarting when it already runs with the same settings. Always clears
    /// the catalog cache, since this is also called after a user switch.
    pub async fn start(&self, settings: OpdsSettings) -> AppResult<OpdsStatus> {
        if settings.share_on_lan
            && settings
                .password
                .as_deref()
                .is_none_or(|p| p.trim().is_empty())
        {
            return Err(AppError::InvalidInput(
                "A password is required to share the catalog on the local network".into(),
            ));
        }
        let root = settings.library_root.clone();
        let root_ok = tokio::task::spawn_blocking(move || root.is_dir())
            .await
            .unwrap_or(false);
        if !root_ok {
            return Err(AppError::library_unavailable(&settings.library_root));
        }

        let mut slot = self.server.lock().await;
        self.cache.invalidate();
        if let Some(running) = slot.as_ref() {
            if running.settings == settings && !running.task.is_finished() {
                return Ok(running.status.clone());
            }
        }
        // Wait for the previous server to let go of its port before binding.
        if let Some(previous) = slot.take() {
            previous.shutdown().await;
        }

        let bind_ip = if settings.share_on_lan {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        let listener = tokio::net::TcpListener::bind((bind_ip, settings.port))
            .await
            .map_err(|e| {
                let message = format!(
                    "Couldn't start the OPDS server on port {}: {e}",
                    settings.port
                );
                if e.kind() == io::ErrorKind::AddrInUse {
                    AppError::Conflict(message)
                } else {
                    AppError::Io(message)
                }
            })?;
        let port = listener
            .local_addr()
            .map_err(|e| AppError::io("Couldn't start the OPDS server", &e))?
            .port();

        let lan_ips: Vec<IpAddr> = if settings.share_on_lan {
            access::primary_lan_ip().into_iter().collect()
        } else {
            vec![]
        };
        let status = OpdsStatus {
            running: true,
            port,
            local_url: format!("http://127.0.0.1:{port}{}", urls::ROOT),
            lan_urls: lan_ips
                .iter()
                .map(|ip| format!("http://{ip}:{port}{}", urls::ROOT))
                .collect(),
        };

        let state = ServerState {
            library_root: Arc::new(settings.library_root.clone()),
            cache: Arc::clone(&self.cache),
        };
        let access = Access {
            share_on_lan: settings.share_on_lan,
            password: settings.password.clone(),
            lan_ips,
        };
        let router = build_router(state, access, Limits::DEFAULT);
        let listener = LimitedListener::new(listener, &Limits::DEFAULT);
        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<ConnInfo>(),
            )
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;
        });

        *slot = Some(RunningServer {
            settings,
            status: status.clone(),
            shutdown: Some(tx),
            task,
        });
        Ok(status)
    }

    /// Stop the server (no-op when it isn't running).
    pub async fn stop(&self) {
        let mut slot = self.server.lock().await;
        if let Some(running) = slot.take() {
            running.shutdown().await;
        }
        self.cache.invalidate();
    }

    /// Current status, or `None` if the server was never started or was stopped.
    pub async fn status(&self) -> Option<OpdsStatus> {
        let slot = self.server.lock().await;
        slot.as_ref().map(|s| OpdsStatus {
            running: !s.task.is_finished(),
            ..s.status.clone()
        })
    }

    /// Forget the cached catalog; the next request reloads the library index.
    pub fn invalidate_cache(&self) {
        self.cache.invalidate();
    }
}

// ── Router ───────────────────────────────────────────────────────────────────

/// What the route handlers share.
#[derive(Clone)]
struct ServerState {
    library_root: Arc<PathBuf>,
    cache: Arc<CatalogCache>,
}

impl ServerState {
    async fn catalog(&self) -> Arc<Catalog> {
        self.cache.get(&self.library_root).await
    }
}

fn build_router(state: ServerState, access: Access, limits: Limits) -> Router {
    Router::new()
        .route("/", get(route_root))
        .route("/opds/", get(route_root))
        .route(urls::ROOT, get(route_root))
        .route(urls::ALL, get(route_all))
        .route(urls::RECENT, get(route_recent))
        .route(urls::AUTHORS, get(route_authors))
        .route(urls::AUTHOR_ROUTE, get(route_author))
        .route(urls::SERIES, get(route_series_list))
        .route(urls::SERIES_ROUTE, get(route_series))
        .route(urls::SEARCH, get(route_search))
        .route(urls::OPENSEARCH, get(route_opensearch))
        .route(urls::ENTRY_ROUTE, get(route_entry))
        .route(urls::V2_ROOT, get(route_v2_root))
        .route(urls::PUBLICATIONS, get(route_publications))
        .route(urls::PUBLICATION_ROUTE, get(route_publication))
        .route(urls::DOWNLOAD_ROUTE, get(route_download))
        .route(urls::COVER_ROUTE, get(route_cover))
        .route(urls::THUMBNAIL_ROUTE, get(route_thumbnail))
        .fallback(|| async { not_found() })
        // The last layer added runs first: access, then limits, so requests
        // that are refused don't take a slot.
        .layer(middleware::from_fn_with_state(
            Arc::new(RequestLimiter::new(limits)),
            limits::limit_requests,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::new(access),
            access::guard,
        ))
        .with_state(state)
}

/// The query parameters of feeds (each optional; parsed leniently).
#[derive(Debug, Default, Deserialize)]
struct ListQuery {
    page: Option<String>,
    limit: Option<String>,
    q: Option<String>,
}

impl ListQuery {
    fn page_request(&self) -> PageRequest {
        PageRequest::parse(self.page.as_deref(), self.limit.as_deref())
    }

    /// The search text, trimmed and cut to [`MAX_QUERY_CHARS`]; `None` when blank.
    fn search_text(&self) -> Option<String> {
        let q = self.q.as_deref().map(str::trim).filter(|q| !q.is_empty())?;
        Some(q.chars().take(MAX_QUERY_CHARS).collect())
    }
}

/// Everything about a feed except its entries.
struct FeedHead {
    id: String,
    title: String,
    path: String,
    up: (String, Kind),
    /// Query parameters its links keep (`q`).
    params: Vec<(&'static str, String)>,
}

impl FeedHead {
    fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        path: impl Into<String>,
        up: &str,
        up_kind: Kind,
    ) -> Self {
        FeedHead {
            id: id.into(),
            title: title.into(),
            path: path.into(),
            up: (up.to_string(), up_kind),
            params: vec![],
        }
    }
}

/// The links of a feed keep a page size the client chose.
fn params_with_limit(
    mut params: Vec<(&'static str, String)>,
    request: PageRequest,
) -> Vec<(&'static str, String)> {
    if request.size != DEFAULT_PAGE_SIZE {
        params.push(("limit", request.size.to_string()));
    }
    params
}

/// One page of an acquisition feed of `books`.
fn book_feed(catalog: &Catalog, books: &[&Entry], query: &ListQuery, head: FeedHead) -> Response {
    let request = query.page_request();
    let (entries, page) = paginate(books, request);
    let feed = Feed {
        id: head.id,
        title: head.title,
        updated: catalog.updated,
        path: head.path,
        params: params_with_limit(head.params, request),
        up: Some(head.up),
        page: Some(page),
        entries: Entries::Books(entries.to_vec()),
    };
    xml_response(atom::feed(&feed), urls::ACQUISITION_TYPE)
}

/// One page of a navigation feed listing `groups` (authors or series), each
/// linking to its acquisition feed (`href_of`).
fn group_feed(
    catalog: &Catalog,
    groups: &[Group],
    query: &ListQuery,
    head: FeedHead,
    href_of: fn(&str) -> String,
) -> Response {
    let request = query.page_request();
    let (groups, page) = paginate(groups, request);
    let entries = groups
        .iter()
        .map(|group| NavEntry {
            id: format!("{}:{}", head.id, urls::encode(&fold(&group.name))),
            title: group.name.clone(),
            href: href_of(&group.name),
            kind: Kind::Acquisition,
            rel: atom::REL_SUBSECTION,
            content: Some(books_label(group.len())),
            count: Some(group.len()),
        })
        .collect();
    let feed = Feed {
        id: head.id,
        title: head.title,
        updated: catalog.updated,
        path: head.path,
        params: params_with_limit(head.params, request),
        up: Some(head.up),
        page: Some(page),
        entries: Entries::Navigation(entries),
    };
    xml_response(atom::feed(&feed), urls::NAVIGATION_TYPE)
}

fn books_label(count: usize) -> String {
    if count == 1 {
        "1 book".into()
    } else {
        format!("{count} books")
    }
}

// ── Route handlers: OPDS 1.2 ─────────────────────────────────────────────────

/// Whether the client asks for OPDS 2 (JSON) rather than Atom: its `Accept`
/// header rates `application/opds+json` (or `application/json`) strictly
/// higher than `application/atom+xml` (or other XML). No header, `*/*` or a
/// tie gives Atom, which every OPDS client reads.
fn prefers_opds2(headers: &HeaderMap) -> bool {
    let Some(accept) = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let (mut json, mut xml) = (0.0f32, 0.0f32);
    for range in accept.split(',') {
        let mut parts = range.split(';');
        let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = parts
            .filter_map(|p| {
                p.trim()
                    .to_ascii_lowercase()
                    .strip_prefix("q=")
                    .and_then(|v| v.trim().parse::<f32>().ok())
            })
            .find(|q| q.is_finite())
            .unwrap_or(1.0);
        match media.as_str() {
            "application/opds+json" | "application/json" => json = json.max(q),
            "application/atom+xml" | "application/xml" | "text/xml" => xml = xml.max(q),
            _ => {}
        }
    }
    json > xml
}

async fn route_root(AxumState(state): AxumState<ServerState>, headers: HeaderMap) -> Response {
    let mut response = if prefers_opds2(&headers) {
        json_response(&json::root(), urls::OPDS2_TYPE)
    } else {
        root_feed(&*state.catalog().await)
    };
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("accept"));
    response
}

fn root_feed(catalog: &Catalog) -> Response {
    let entry = |id: &str,
                 title: &str,
                 href: &str,
                 kind: Kind,
                 rel: &'static str,
                 content: String,
                 count: Option<usize>| NavEntry {
        id: format!("urn:librarium:{id}"),
        title: title.into(),
        href: href.into(),
        kind,
        rel,
        content: Some(content),
        count,
    };
    let (books, authors, series) = (
        catalog.len(),
        catalog.authors().len(),
        catalog.series_list().len(),
    );
    let entries = vec![
        entry(
            "all",
            "All books",
            urls::ALL,
            Kind::Acquisition,
            atom::REL_SUBSECTION,
            books_label(books),
            Some(books),
        ),
        entry(
            "recent",
            "Recently added",
            urls::RECENT,
            Kind::Acquisition,
            atom::REL_SORT_NEW,
            "Newest first".into(),
            Some(books),
        ),
        entry(
            "authors",
            "By author",
            urls::AUTHORS,
            Kind::Navigation,
            atom::REL_SUBSECTION,
            if authors == 1 {
                "1 author".into()
            } else {
                format!("{authors} authors")
            },
            None,
        ),
        entry(
            "series",
            "By series",
            urls::SERIES,
            Kind::Navigation,
            atom::REL_SUBSECTION,
            format!("{series} series"),
            None,
        ),
    ];
    let feed = Feed {
        id: "urn:librarium:root".into(),
        title: "Librarium".into(),
        updated: catalog.updated,
        path: urls::ROOT.into(),
        params: vec![],
        up: None,
        page: None,
        entries: Entries::Navigation(entries),
    };
    xml_response(atom::feed(&feed), urls::NAVIGATION_TYPE)
}

async fn route_all(
    AxumState(state): AxumState<ServerState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let head = FeedHead::new(
        "urn:librarium:all",
        "All books",
        urls::ALL,
        urls::ROOT,
        Kind::Navigation,
    );
    book_feed(&catalog, &catalog.all(), &query, head)
}

async fn route_recent(
    AxumState(state): AxumState<ServerState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let head = FeedHead::new(
        "urn:librarium:recent",
        "Recently added",
        urls::RECENT,
        urls::ROOT,
        Kind::Navigation,
    );
    book_feed(&catalog, &catalog.recent(), &query, head)
}

async fn route_authors(
    AxumState(state): AxumState<ServerState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let head = FeedHead::new(
        "urn:librarium:author",
        "By author",
        urls::AUTHORS,
        urls::ROOT,
        Kind::Navigation,
    );
    group_feed(&catalog, catalog.authors(), &query, head, urls::author)
}

async fn route_author(
    AxumState(state): AxumState<ServerState>,
    AxumPath(name): AxumPath<String>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let Some(group) = catalog.author(&name) else {
        return not_found();
    };
    let id = format!("urn:librarium:author:{}", urls::encode(&fold(&group.name)));
    let head = FeedHead::new(
        id,
        group.name.clone(),
        urls::author(&group.name),
        urls::AUTHORS,
        Kind::Navigation,
    );
    book_feed(&catalog, &catalog.books_of(group), &query, head)
}

async fn route_series_list(
    AxumState(state): AxumState<ServerState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let head = FeedHead::new(
        "urn:librarium:series",
        "By series",
        urls::SERIES,
        urls::ROOT,
        Kind::Navigation,
    );
    group_feed(&catalog, catalog.series_list(), &query, head, urls::series)
}

async fn route_series(
    AxumState(state): AxumState<ServerState>,
    AxumPath(name): AxumPath<String>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let Some(group) = catalog.series(&name) else {
        return not_found();
    };
    let id = format!("urn:librarium:series:{}", urls::encode(&fold(&group.name)));
    let head = FeedHead::new(
        id,
        group.name.clone(),
        urls::series(&group.name),
        urls::SERIES,
        Kind::Navigation,
    );
    book_feed(&catalog, &catalog.books_of(group), &query, head)
}

async fn route_search(
    AxumState(state): AxumState<ServerState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let text = query.search_text().unwrap_or_default();
    let books = catalog.search(&text);
    let mut head = FeedHead::new(
        format!("urn:librarium:search:{}", urls::encode(&fold(&text))),
        if text.is_empty() {
            "Search".to_string()
        } else {
            format!("Search: {text}")
        },
        urls::SEARCH,
        urls::ROOT,
        Kind::Navigation,
    );
    head.params.push(("q", text));
    book_feed(&catalog, &books, &query, head)
}

async fn route_opensearch() -> Response {
    xml_response(atom::opensearch_description(), urls::OPENSEARCH_TYPE)
}

async fn route_entry(
    AxumState(state): AxumState<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    match state.catalog().await.get(&id) {
        Some(entry) => xml_response(atom::entry_document(entry), urls::ENTRY_TYPE),
        None => not_found(),
    }
}

// ── Route handlers: OPDS 2.0 ─────────────────────────────────────────────────

async fn route_v2_root() -> Response {
    json_response(&json::root(), urls::OPDS2_TYPE)
}

async fn route_publications(
    AxumState(state): AxumState<ServerState>,
    Query(query): Query<ListQuery>,
) -> Response {
    let catalog = state.catalog().await;
    let text = query.search_text();
    let books = match &text {
        Some(text) => catalog.search(text),
        None => catalog.all(),
    };
    let (entries, page) = paginate(&books, query.page_request());
    json_response(
        &json::publications(entries, page, text.as_deref()),
        urls::OPDS2_TYPE,
    )
}

async fn route_publication(
    AxumState(state): AxumState<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    match state.catalog().await.get(&id) {
        Some(entry) => json_response(&json::publication(entry), urls::OPDS2_PUBLICATION_TYPE),
        None => not_found(),
    }
}

// ── Route handlers: files ────────────────────────────────────────────────────

async fn route_download(
    AxumState(state): AxumState<ServerState>,
    AxumPath(id): AxumPath<String>,
    request: Request,
) -> Response {
    let catalog = state.catalog().await;
    let Some(entry) = catalog.get(&id) else {
        return not_found();
    };
    let book = &entry.book;

    // Index paths are absolute (resolved inside the library when the index
    // is read); `join` keeps an absolute path as it is. Checked again here,
    // through symlinks, because the index can be edited by hand.
    let path = state.library_root.join(&book.path);
    let (root, candidate) = (Arc::clone(&state.library_root), path.clone());
    let inside = tokio::task::spawn_blocking(move || {
        candidate.is_file() && fsutil::is_within(&root, &candidate)
    })
    .await
    .unwrap_or(false);
    if !inside {
        return not_found();
    }

    let filename = if book.filename.is_empty() {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "book".into())
    } else {
        book.filename.clone()
    };
    let Ok(disposition) = HeaderValue::from_str(&files::content_disposition(&filename)) else {
        return server_error();
    };
    let mime = files::format_mime(&book.format);
    drop(catalog);

    let mut response = files::serve_file(request, path, mime).await;
    if response.status().is_success() {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, disposition);
    }
    response
}

async fn route_cover(
    state: AxumState<ServerState>,
    id: AxumPath<String>,
    request: Request,
) -> Response {
    serve_image(state.0, &id.0, request, Entry::cover_file).await
}

async fn route_thumbnail(
    state: AxumState<ServerState>,
    id: AxumPath<String>,
    request: Request,
) -> Response {
    serve_image(state.0, &id.0, request, Entry::thumbnail_file).await
}

/// Serve the image `pick` chooses for book `id`, typed from its first bytes.
async fn serve_image(
    state: ServerState,
    id: &str,
    request: Request,
    pick: fn(&Entry) -> Option<&Path>,
) -> Response {
    let catalog = state.catalog().await;
    let Some(path) = catalog.get(id).and_then(pick).map(Path::to_path_buf) else {
        return not_found();
    };
    drop(catalog);
    let probe = path.clone();
    let mime = tokio::task::spawn_blocking(move || files::sniff_image_file(&probe))
        .await
        .unwrap_or("image/jpeg");
    files::serve_file(request, path, mime).await
}

// ── Responses ────────────────────────────────────────────────────────────────
// Built without `Response::builder()` so there is no `unwrap()` to panic on:
// only dynamic header values can fail, and those become a 500.

fn plain(status: StatusCode, text: &'static str) -> Response {
    let mut response = Response::new(Body::from(text));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

fn not_found() -> Response {
    plain(StatusCode::NOT_FOUND, "Not Found")
}

fn server_error() -> Response {
    plain(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
}

/// A `200` with `body` typed `<media_type>; charset=utf-8`.
fn typed_response(media_type: &str, body: Vec<u8>) -> Response {
    let Ok(content_type) = HeaderValue::from_str(&format!("{media_type}; charset=utf-8")) else {
        return server_error();
    };
    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response
}

fn xml_response(document: io::Result<Vec<u8>>, media_type: &str) -> Response {
    match document {
        Ok(body) => typed_response(media_type, body),
        Err(_) => server_error(),
    }
}

fn json_response(value: &serde_json::Value, media_type: &str) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => typed_response(media_type, body),
        Err(_) => server_error(),
    }
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// Start (or restart, if the settings changed) the OPDS server.
///
/// `share_on_lan` and `password` are optional so older frontends that only
/// send `port` and `libraryRoot` get the safe local-only mode.
#[tauri::command]
pub async fn start_opds(
    app: AppHandle,
    state: State<'_, OpdsServerState>,
    port: u16,
    library_root: String,
    share_on_lan: Option<bool>,
    password: Option<String>,
) -> AppResult<OpdsStatus> {
    let settings = OpdsSettings::new(port, library_root, share_on_lan.unwrap_or(false), password);
    let lan = settings.share_on_lan;
    let status = state.start(settings).await?;
    log(
        &app,
        format!(
            "[opds] running at {}{}",
            status.local_url,
            if lan {
                format!(" and on the LAN ({})", status.lan_urls.join(", "))
            } else {
                String::new()
            }
        ),
    );
    Ok(status)
}

/// Stop the OPDS server.
#[tauri::command]
pub async fn stop_opds(app: AppHandle, state: State<'_, OpdsServerState>) -> AppResult<()> {
    state.stop().await;
    log(&app, "[opds] stopped");
    Ok(())
}

/// Status of the OPDS server, or `null` when it isn't started.
#[tauri::command]
pub async fn opds_status(state: State<'_, OpdsServerState>) -> AppResult<Option<OpdsStatus>> {
    Ok(state.status().await)
}

/// Bust the catalog cache so the next OPDS request reloads from disk. The
/// cache also notices index and cover changes by itself within a second;
/// calling this after a library change makes it immediate.
#[tauri::command]
pub async fn invalidate_opds_cache(state: State<'_, OpdsServerState>) -> AppResult<()> {
    state.invalidate_cache();
    Ok(())
}

/// A QR code (SVG markup) of `url`, typically one of `OpdsStatus.lanUrls`,
/// for a phone or tablet to scan. Async only so that, like every command, it
/// runs off the main thread (it takes microseconds).
#[tauri::command]
pub async fn opds_qr_svg(url: String) -> AppResult<String> {
    qr::qr_svg(&url)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::{HashSet, VecDeque};
    use std::net::SocketAddr;

    use axum::extract::ConnectInfo;
    use axum::http::Request as HttpRequest;
    use base64::prelude::*;
    use serde_json::json;
    use tower::ServiceExt;

    use super::access::OPDS_USER;
    use super::atom::xml_tree::{self, ATOM};
    use super::*;

    const TRICKY_TITLE: &str = "Le \"Petit\" <Prince> & l'été\\";
    const BOOK_ID: &str = "11111111-1111-1111-1111-111111111111";
    const PDF_ID: &str = "22222222-2222-2222-2222-222222222222";
    const OUTSIDE_ID: &str = "33333333-3333-3333-3333-333333333333";
    const LAN_IP: &str = "192.168.1.20";
    const AUTHOR_WITH_SLASH: &str = "AC/DC";
    const SERIES_NAME: &str = "Saga #1? 50% & more";

    struct Fixture {
        lib: tempfile::TempDir,
        _outside: tempfile::TempDir,
    }

    /// One index record; `path` is what the index stores (relative in v2,
    /// absolute in v1).
    fn book_json(id: &str, title: &str, path: &str) -> serde_json::Value {
        json!({
            "id": id, "path": path, "filename": path.rsplit('/').next().unwrap(), "format": "epub",
            "sizeBytes": 10, "dateAdded": "2026-01-01T00:00:00Z",
            "metadata": {"title": title, "author": "Antoine de Saint-Exupéry", "categories": ["Conte"]},
        })
    }

    /// A library (index format v2) with:
    /// - a tricky-titled EPUB with a PNG cover and a JPEG thumbnail;
    /// - a PDF without cover, by two authors (one with a `/`), in a series
    ///   whose name needs encoding;
    /// - a hand-edited entry pointing outside the library.
    fn fixture() -> Fixture {
        let lib = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = lib.path();
        std::fs::create_dir_all(root.join(".librarium")).unwrap();
        std::fs::create_dir_all(root.join(".covers/thumbs")).unwrap();
        std::fs::write(root.join("Café: été.epub"), b"0123456789").unwrap();
        std::fs::write(root.join("Plain.pdf"), b"%PDF-1.7\n.").unwrap();
        std::fs::write(outside.path().join("secret.epub"), b"SECRET").unwrap();
        std::fs::write(
            root.join(format!(".covers/{BOOK_ID}.png")),
            b"\x89PNG\r\n\x1a\nrest",
        )
        .unwrap();
        std::fs::write(
            root.join(format!(".covers/thumbs/{BOOK_ID}.jpg")),
            b"\xFF\xD8\xFF\xE0rest",
        )
        .unwrap();
        write_index(
            root,
            &[
                (
                    "Café: été.epub",
                    book_json(BOOK_ID, TRICKY_TITLE, "Café: été.epub"),
                ),
                ("Plain.pdf", pdf_json()),
                (
                    "../secret.epub",
                    book_json(OUTSIDE_ID, "Secret", "../secret.epub"),
                ),
            ],
        );
        Fixture {
            lib,
            _outside: outside,
        }
    }

    fn pdf_json() -> serde_json::Value {
        json!({
            "id": PDF_ID, "path": "Plain.pdf", "filename": "Plain.pdf", "format": "pdf",
            "sizeBytes": 10, "dateAdded": "2026-02-01T00:00:00Z",
            "metadata": {
                "title": "Plain", "author": format!("{AUTHOR_WITH_SLASH} & Victor Hugo"),
                "series": SERIES_NAME, "seriesIndex": 1,
            },
        })
    }

    fn write_index(root: &Path, books: &[(&str, serde_json::Value)]) {
        let books: serde_json::Map<String, serde_json::Value> = books
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let index = json!({"version": 2, "libraryId": "opds-test", "books": books});
        std::fs::write(root.join(".librarium/library.json"), index.to_string()).unwrap();
    }

    fn router_with(root: &Path, share_on_lan: bool, limits: Limits) -> Router {
        build_router(
            ServerState {
                library_root: Arc::new(root.to_path_buf()),
                // Re-checked on every request, so tests see index changes at once.
                cache: Arc::new(CatalogCache::new(Duration::ZERO)),
            },
            Access {
                share_on_lan,
                password: share_on_lan.then(|| "s3cret".to_string()),
                lan_ips: vec![LAN_IP.parse().unwrap()],
            },
            limits,
        )
    }

    fn router(root: &Path, share_on_lan: bool) -> Router {
        router_with(root, share_on_lan, Limits::DEFAULT)
    }

    fn basic(user: &str, pass: &str) -> String {
        format!("Basic {}", BASE64_STANDARD.encode(format!("{user}:{pass}")))
    }

    /// A request to `uri`; `headers` are added as given.
    async fn send(
        router: &Router,
        uri: &str,
        headers: &[(&str, &str)],
        local_ip: Option<&str>,
    ) -> Response {
        let mut req = HttpRequest::builder().uri(uri);
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let mut req = req.body(Body::empty()).unwrap();
        if let Some(ip) = local_ip {
            let local = SocketAddr::new(ip.parse().unwrap(), 8042);
            req.extensions_mut().insert(ConnectInfo(ConnInfo { local }));
        }
        router.clone().oneshot(req).await.unwrap()
    }

    /// GET as a local client in local mode.
    async fn get_local(router: &Router, uri: &str) -> Response {
        send(router, uri, &[("host", "localhost:8042")], None).await
    }

    /// GET as a LAN client with the right password.
    async fn get_lan(router: &Router, uri: &str) -> Response {
        send(
            router,
            uri,
            &[
                ("host", "192.168.1.20:8042"),
                ("authorization", &basic(OPDS_USER, "s3cret")),
            ],
            None,
        )
        .await
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    async fn feed(router: &Router, uri: &str) -> xml_tree::Node {
        let resp = get_local(router, uri).await;
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        xml_tree::parse(&body_bytes(resp).await)
    }

    fn content_type(resp: &Response) -> String {
        resp.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .to_string()
    }

    fn entry_titles(feed: &xml_tree::Node) -> Vec<String> {
        feed.entries()
            .iter()
            .map(|e| e.text_of(ATOM, "title").unwrap().to_string())
            .collect()
    }

    /// Every route, with the ids and names of the fixture.
    fn all_routes() -> Vec<String> {
        let mut routes: Vec<String> = [
            "/",
            "/opds",
            "/opds/",
            "/opds/all",
            "/opds/recent",
            "/opds/authors",
            "/opds/series",
            "/opds/search?q=x",
            "/opds/opensearch.xml",
            "/opds/v2",
            "/opds/publications",
            "/opds/no-such-route",
        ]
        .map(str::to_string)
        .into();
        routes.push(urls::author(AUTHOR_WITH_SLASH));
        routes.push(urls::series(SERIES_NAME));
        for id in [BOOK_ID, PDF_ID] {
            routes.extend([
                urls::entry(id),
                urls::publication(id),
                urls::download(id),
                urls::cover(id),
                urls::thumbnail(id),
            ]);
        }
        routes
    }

    // ── Access ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn local_mode_serves_loopback_hosts_without_cors() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        for host in [
            "localhost:8042",
            "127.0.0.1:8042",
            "[::1]:8042",
            "LOCALHOST.",
            "localhost",
        ] {
            let resp = send(&r, "/opds", &[("host", host)], None).await;
            assert_eq!(resp.status(), StatusCode::OK, "{host}");
            assert!(
                resp.headers().get("access-control-allow-origin").is_none(),
                "no CORS header"
            );
            assert_eq!(
                content_type(&resp),
                format!("{}; charset=utf-8", urls::NAVIGATION_TYPE)
            );
            assert_eq!(resp.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        }
    }

    #[tokio::test]
    async fn foreign_hosts_are_rejected_on_every_route() {
        let f = fixture();
        let local = router(f.lib.path(), false);
        for route in all_routes() {
            for host in [
                Some("evil.example"),
                Some("evil.example:8042"),
                Some(LAN_IP),
                Some("localhost.evil.example"),
                None,
            ] {
                let headers: Vec<(&str, &str)> = host.map(|h| ("host", h)).into_iter().collect();
                let resp = send(&local, &route, &headers, None).await;
                assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{route} {host:?}");
                assert!(resp.headers().get("access-control-allow-origin").is_none());
            }
        }
        // LAN mode: even with valid credentials, rebinding hostnames and IPs
        // that aren't this machine's are refused.
        let lan = router(f.lib.path(), true);
        let auth = basic(OPDS_USER, "s3cret");
        for (host, local_ip) in [
            ("evil.example", Some(LAN_IP)),
            ("10.0.0.8:8042", Some("10.0.0.9")),
        ] {
            let resp = send(
                &lan,
                "/opds",
                &[("host", host), ("authorization", &auth)],
                local_ip,
            )
            .await;
            assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{host}");
        }
    }

    #[tokio::test]
    async fn lan_mode_requires_basic_auth_on_every_route() {
        let f = fixture();
        let r = router(f.lib.path(), true);
        let host = ("host", "192.168.1.20:8042");
        let bad = [
            basic(OPDS_USER, "wrong"),
            basic("admin", "s3cret"),
            basic(OPDS_USER, ""),
            "Bearer s3cret".to_string(),
            "Basic !!!".to_string(),
        ];
        for route in all_routes() {
            let resp = send(&r, &route, &[host], None).await;
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{route}");
            assert!(resp.headers()[header::WWW_AUTHENTICATE]
                .to_str()
                .unwrap()
                .starts_with("Basic "));
            for auth in &bad {
                let resp = send(&r, &route, &[host, ("authorization", auth)], None).await;
                assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{route} {auth}");
            }
            let ok = get_lan(&r, &route).await;
            assert!(
                ok.status() == StatusCode::OK || ok.status() == StatusCode::NOT_FOUND,
                "{route}: {}",
                ok.status()
            );
            assert_eq!(
                ok.status() == StatusCode::NOT_FOUND,
                route.ends_with("no-such-route")
                    || route == urls::cover(PDF_ID)
                    || route == urls::thumbnail(PDF_ID),
                "{route}"
            );
        }
        // The IP the client actually connected to is accepted too.
        let auth = basic(OPDS_USER, "s3cret");
        let ok = send(
            &r,
            "/opds",
            &[("host", "10.0.0.9:8042"), ("authorization", &auth)],
            Some("10.0.0.9"),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);
        // Loopback needs the password as well in LAN mode.
        let resp = send(&r, "/opds", &[("host", "localhost")], None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── OPDS 1.2 ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn root_is_atom_unless_json_is_preferred() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        for accept in [
            None,
            Some("*/*"),
            Some("application/atom+xml, application/opds+json"),
            Some("application/opds+json;q=0.5, application/atom+xml"),
        ] {
            let mut headers = vec![("host", "localhost")];
            headers.extend(accept.map(|a| ("accept", a)));
            let resp = send(&r, "/opds", &headers, None).await;
            assert!(
                content_type(&resp).starts_with(urls::NAVIGATION_TYPE),
                "{accept:?}"
            );
            assert_eq!(resp.headers()[header::VARY], "accept");
        }
        // What the app's own catalog browser sends.
        let resp = send(
            &r,
            "/opds",
            &[
                ("host", "localhost"),
                ("accept", "application/opds+json, application/json"),
            ],
            None,
        )
        .await;
        assert!(content_type(&resp).starts_with(urls::OPDS2_TYPE));
        let root: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(root["navigation"][0]["href"], urls::PUBLICATIONS);
        assert!(content_type(&get_local(&r, "/opds/v2").await).starts_with(urls::OPDS2_TYPE));

        let root = feed(&r, "/opds").await;
        assert_eq!(
            entry_titles(&root),
            ["All books", "Recently added", "By author", "By series"]
        );
        assert_eq!(
            root.entries()[0].text_of(ATOM, "content"),
            Some("2 books"),
            "the entry outside the library isn't counted"
        );
        assert_eq!(root.href("search"), Some(urls::OPENSEARCH));
    }

    /// Follows every link the Atom feeds contain (feeds, entries, covers,
    /// downloads, OpenSearch), as a LAN client: each must resolve, with the
    /// type its link announced.
    #[tokio::test]
    async fn every_link_in_the_feeds_resolves() {
        let f = fixture();
        let r = router(f.lib.path(), true);
        let mut queue =
            VecDeque::from([(urls::ROOT.to_string(), urls::NAVIGATION_TYPE.to_string())]);
        let mut seen = HashSet::new();
        while let Some((href, expected_type)) = queue.pop_front() {
            if !seen.insert(href.clone()) {
                continue;
            }
            let resp = get_lan(&r, &href).await;
            assert_eq!(resp.status(), StatusCode::OK, "{href}");
            let ct = content_type(&resp);
            assert!(
                ct.starts_with(&expected_type),
                "{href}: announced {expected_type}, served {ct}"
            );
            if !(ct.starts_with("application/atom+xml") || ct.starts_with(urls::OPENSEARCH_TYPE)) {
                continue;
            }
            let doc = xml_tree::parse(&body_bytes(resp).await);
            let mut links: Vec<&xml_tree::Node> = doc.all(ATOM, "link").collect();
            links.extend(doc.entries().iter().flat_map(|e| e.all(ATOM, "link")));
            for link in links {
                let (href, kind) = (link.attr("href").unwrap(), link.attr("type").unwrap());
                let href = href.replace("{searchTerms}", "prince");
                queue.push_back((href, kind.to_string()));
            }
            for template in doc.all(xml_tree::OPENSEARCH, "Url") {
                let href = template
                    .attr("template")
                    .unwrap()
                    .replace("{searchTerms}", "hugo");
                queue.push_back((href, template.attr("type").unwrap().to_string()));
            }
        }
        for expected in [
            urls::author(AUTHOR_WITH_SLASH),
            urls::series(SERIES_NAME),
            urls::download(BOOK_ID),
            urls::thumbnail(BOOK_ID),
            "/opds/search?q=prince".into(),
            "/opds/search?q=hugo".into(),
        ] {
            assert!(
                seen.contains(&expected),
                "{expected} not linked; seen: {seen:?}"
            );
        }
    }

    #[tokio::test]
    async fn feeds_list_books_with_escaped_titles() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        let resp = get_local(&r, "/opds/all").await;
        assert_eq!(
            content_type(&resp),
            format!("{}; charset=utf-8", urls::ACQUISITION_TYPE)
        );
        let raw = String::from_utf8(body_bytes(resp).await).unwrap();
        assert!(
            raw.contains("Le &quot;Petit&quot; &lt;Prince&gt; &amp; l&apos;été\\"),
            "{raw}"
        );
        let all = xml_tree::parse(raw.as_bytes());
        assert_eq!(
            entry_titles(&all),
            [TRICKY_TITLE, "Plain"],
            "by title; the entry outside the library isn't served"
        );
        assert_eq!(
            entry_titles(&feed(&r, "/opds/recent").await),
            ["Plain", TRICKY_TITLE],
            "newest first"
        );

        let book = all.entries()[0];
        assert_eq!(
            book.links("http://opds-spec.org/image")
                .next()
                .unwrap()
                .attr("type"),
            Some("image/png")
        );
        assert_eq!(
            book.links("http://opds-spec.org/image/thumbnail")
                .next()
                .unwrap()
                .attr("type"),
            Some("image/jpeg")
        );
        let pdf = all.entries()[1];
        assert!(
            pdf.links("http://opds-spec.org/image").next().is_none(),
            "no cover, no image link"
        );
        let acquisition = pdf
            .links("http://opds-spec.org/acquisition")
            .next()
            .unwrap();
        assert_eq!(acquisition.attr("type"), Some("application/pdf"));
        assert_eq!(acquisition.attr("length"), Some("10"));
        assert_eq!(pdf.text_of(xml_tree::CALIBRE, "series"), Some(SERIES_NAME));
    }

    #[tokio::test]
    async fn authors_and_series_have_their_own_feeds() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        let authors = feed(&r, "/opds/authors").await;
        assert_eq!(
            entry_titles(&authors),
            [AUTHOR_WITH_SLASH, "Antoine de Saint-Exupéry", "Victor Hugo"]
        );
        let link = authors.entries()[0].links("subsection").next().unwrap();
        assert_eq!(link.attr("href"), Some("/opds/authors/AC%2FDC"));
        assert_eq!(link.attr("thr:count"), Some("1"));

        assert_eq!(
            entry_titles(&feed(&r, "/opds/authors/AC%2FDC").await),
            ["Plain"]
        );
        // Folded lookup: case and accents don't matter.
        assert_eq!(
            entry_titles(&feed(&r, "/opds/authors/antoine%20de%20saint-exupery").await),
            [TRICKY_TITLE]
        );
        assert_eq!(
            get_local(&r, "/opds/authors/Nobody").await.status(),
            StatusCode::NOT_FOUND
        );

        let series = feed(&r, "/opds/series").await;
        assert_eq!(entry_titles(&series), [SERIES_NAME]);
        let href = series.entries()[0]
            .links("subsection")
            .next()
            .unwrap()
            .attr("href")
            .unwrap()
            .to_string();
        let saga = feed(&r, &href).await;
        assert_eq!(saga.text_of(ATOM, "title"), Some(SERIES_NAME));
        assert_eq!(entry_titles(&saga), ["Plain"]);
        assert_eq!(saga.href("up"), Some(urls::SERIES));
    }

    #[tokio::test]
    async fn feeds_are_paginated() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        let first = feed(&r, "/opds/all?limit=1").await;
        assert_eq!(entry_titles(&first), [TRICKY_TITLE]);
        assert_eq!(first.href("next"), Some("/opds/all?limit=1&page=2"));
        assert_eq!(first.href("last"), Some("/opds/all?limit=1&page=2"));
        assert!(first.href("previous").is_none());
        let second = feed(&r, first.href("next").unwrap()).await;
        assert_eq!(entry_titles(&second), ["Plain"]);
        assert_eq!(second.href("previous"), Some("/opds/all?limit=1"));
        assert!(second.href("next").is_none());
        assert!(
            feed(&r, "/opds/all?page=9").await.entries().is_empty(),
            "past the end: empty, not an error"
        );
        // Nonsense is read leniently; an oversized page is capped.
        let capped = feed(&r, "/opds/all?limit=100000&page=abc").await;
        assert_eq!(
            capped.text_of(xml_tree::OPENSEARCH, "itemsPerPage"),
            Some("200")
        );
        assert_eq!(capped.entries().len(), 2);
    }

    #[tokio::test]
    async fn search_matches_titles_authors_and_series() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        assert_eq!(
            entry_titles(&feed(&r, "/opds/search?q=PRINCE").await),
            [TRICKY_TITLE]
        );
        assert_eq!(
            entry_titles(&feed(&r, "/opds/search?q=l%27ete").await),
            [TRICKY_TITLE],
            "accent-insensitive"
        );
        assert_eq!(
            entry_titles(&feed(&r, "/opds/search?q=hugo%20saga").await),
            ["Plain"],
            "every word, any field"
        );
        assert!(feed(&r, "/opds/search?q=").await.entries().is_empty());
        assert!(feed(&r, "/opds/search").await.entries().is_empty());
        let results = feed(&r, "/opds/search?q=%3Cscript%3E").await;
        assert_eq!(results.text_of(ATOM, "title"), Some("Search: <script>"));

        let description = get_local(&r, urls::OPENSEARCH).await;
        assert_eq!(
            content_type(&description),
            format!("{}; charset=utf-8", urls::OPENSEARCH_TYPE)
        );
    }

    #[tokio::test]
    async fn entry_documents_describe_one_book() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        let resp = get_local(&r, &urls::entry(BOOK_ID)).await;
        assert!(content_type(&resp).starts_with(urls::ENTRY_TYPE));
        let entry = xml_tree::parse(&body_bytes(resp).await);
        assert_eq!(entry.text_of(ATOM, "title"), Some(TRICKY_TITLE));
        assert_eq!(
            get_local(&r, &urls::entry("nope")).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_local(&r, &urls::entry(OUTSIDE_ID)).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    // ── OPDS 2.0 ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn json_publications_still_work() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        let resp = get_local(&r, "/opds/publications").await;
        assert_eq!(content_type(&resp), "application/opds+json; charset=utf-8");
        let raw = body_bytes(resp).await;
        assert!(String::from_utf8_lossy(&raw).contains(r#"Le \"Petit\" <Prince> & l'été\\"#));
        let feed: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let pubs = feed["publications"].as_array().unwrap();
        let titles: Vec<&str> = pubs
            .iter()
            .map(|p| p["metadata"]["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec![TRICKY_TITLE, "Plain"]);
        assert_eq!(pubs[0]["images"][0]["type"], "image/png");
        assert!(pubs[1].get("images").is_none(), "no cover, no image link");

        let found: serde_json::Value = serde_json::from_slice(
            &body_bytes(get_local(&r, "/opds/publications?q=PRINCE").await).await,
        )
        .unwrap();
        assert_eq!(found["publications"].as_array().unwrap().len(), 1);
        let paged: serde_json::Value = serde_json::from_slice(
            &body_bytes(get_local(&r, "/opds/publications?limit=1").await).await,
        )
        .unwrap();
        assert_eq!(paged["publications"].as_array().unwrap().len(), 1);
        assert_eq!(paged["metadata"]["numberOfItems"], 2);

        let one = get_local(&r, &urls::publication(BOOK_ID)).await;
        assert!(content_type(&one).starts_with(urls::OPDS2_PUBLICATION_TYPE));
        let one: serde_json::Value = serde_json::from_slice(&body_bytes(one).await).unwrap();
        assert_eq!(one["metadata"]["title"], TRICKY_TITLE);
    }

    // ── Files ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn downloads_are_streamed_with_ranges_and_their_name() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        let resp = get_local(&r, &urls::download(BOOK_ID)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(content_type(&resp), "application/epub+zip");
        assert_eq!(resp.headers()[header::CONTENT_LENGTH], "10");
        assert_eq!(resp.headers()[header::ACCEPT_RANGES], "bytes");
        assert_eq!(
            resp.headers()[header::CONTENT_DISPOSITION],
            files::content_disposition("Café: été.epub").as_str()
        );
        assert_eq!(body_bytes(resp).await, b"0123456789");

        let partial = send(
            &r,
            &urls::download(BOOK_ID),
            &[("host", "localhost"), ("range", "bytes=4-")],
            None,
        )
        .await;
        assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(partial.headers()[header::CONTENT_RANGE], "bytes 4-9/10");
        assert!(partial.headers().contains_key(header::CONTENT_DISPOSITION));
        assert_eq!(body_bytes(partial).await, b"456789");

        let pdf = get_local(&r, &urls::download(PDF_ID)).await;
        assert_eq!(content_type(&pdf), "application/pdf");
    }

    /// A link inside the library pointing outside it: the index may name a
    /// path through it, but the download is refused.
    #[cfg(unix)]
    #[tokio::test]
    async fn download_refuses_a_symlink_out_of_the_library() {
        let f = fixture();
        let root = f.lib.path();
        std::os::unix::fs::symlink(f._outside.path(), root.join("link")).unwrap();
        write_index(
            root,
            &[(
                "link/secret.epub",
                book_json(OUTSIDE_ID, "Secret", "link/secret.epub"),
            )],
        );
        let r = router(root, false);
        let resp = get_local(&r, &urls::download(OUTSIDE_ID)).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn download_refuses_files_outside_the_library() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        assert_eq!(
            get_local(&r, &urls::download(OUTSIDE_ID)).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_local(&r, "/opds/publications/nope/download")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_local(&r, "/opds/publications/..%2F..%2Fetc/download")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn covers_and_thumbnails_are_served_with_their_real_type() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        let cover = get_local(&r, &urls::cover(BOOK_ID)).await;
        assert_eq!(cover.status(), StatusCode::OK);
        assert_eq!(content_type(&cover), "image/png");
        assert!(
            cover.headers().get(header::CONTENT_DISPOSITION).is_none(),
            "shown, not downloaded"
        );
        let thumb = get_local(&r, &urls::thumbnail(BOOK_ID)).await;
        assert_eq!(content_type(&thumb), "image/jpeg");
        assert_eq!(body_bytes(thumb).await, b"\xFF\xD8\xFF\xE0rest");
        assert_eq!(
            get_local(&r, &urls::cover(PDF_ID)).await.status(),
            StatusCode::NOT_FOUND
        );

        // A cover file misnamed by an old version is typed from its bytes;
        // without a thumbnail, the cover stands in.
        std::fs::remove_file(f.lib.path().join(format!(".covers/thumbs/{BOOK_ID}.jpg"))).unwrap();
        std::fs::rename(
            f.lib.path().join(format!(".covers/{BOOK_ID}.png")),
            f.lib.path().join(format!(".covers/{BOOK_ID}.jpg")),
        )
        .unwrap();
        assert_eq!(
            content_type(&get_local(&r, &urls::thumbnail(BOOK_ID)).await),
            "image/png"
        );
    }

    // ── Limits and cache ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_download_in_progress_counts_against_the_limit() {
        let f = fixture();
        let limits = Limits {
            max_requests: 1,
            queue_timeout: Duration::from_millis(50),
            ..Limits::DEFAULT
        };
        let r = router_with(f.lib.path(), false, limits);
        let download = get_local(&r, &urls::download(BOOK_ID)).await;
        assert_eq!(download.status(), StatusCode::OK);
        let busy = get_local(&r, "/opds").await;
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(busy.headers().contains_key(header::RETRY_AFTER));
        assert_eq!(body_bytes(download).await, b"0123456789");
        assert_eq!(get_local(&r, "/opds").await.status(), StatusCode::OK);
        // Refused requests don't take a slot.
        let held = get_local(&r, &urls::download(BOOK_ID)).await;
        assert_eq!(
            send(&r, "/opds", &[("host", "evil.example")], None)
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        drop(held);
    }

    #[tokio::test]
    async fn the_catalog_follows_index_changes_without_invalidation() {
        let f = fixture();
        let r = router(f.lib.path(), false);
        assert_eq!(feed(&r, "/opds/all").await.entries().len(), 2);
        std::fs::write(f.lib.path().join("New.epub"), b"EPUB").unwrap();
        write_index(
            f.lib.path(),
            &[
                (
                    "Café: été.epub",
                    book_json(BOOK_ID, TRICKY_TITLE, "Café: été.epub"),
                ),
                ("Plain.pdf", pdf_json()),
                (
                    "New.epub",
                    book_json("44444444-4444-4444-4444-444444444444", "Zebra", "New.epub"),
                ),
            ],
        );
        assert_eq!(
            entry_titles(&feed(&r, "/opds/all").await),
            [TRICKY_TITLE, "Plain", "Zebra"]
        );
    }

    #[test]
    fn a_legacy_v1_index_is_served_before_it_is_migrated() {
        let lib = tempfile::tempdir().unwrap();
        let root = lib.path();
        std::fs::create_dir_all(root.join(".librarium")).unwrap();
        let book = root.join("Dune.epub");
        std::fs::write(&book, b"EPUB").unwrap();
        let path = book.to_string_lossy().into_owned();
        let v1 =
            json!({ path.clone(): book_json(BOOK_ID, "Dune", &path), "broken": {"id": "broken"} });
        std::fs::write(root.join(".librarium/books.json"), v1.to_string()).unwrap();
        let catalog = Catalog::load(root, chrono::Utc::now());
        assert_eq!(catalog.len(), 1, "the broken entry is skipped");
        assert_eq!(PathBuf::from(&catalog.all()[0].book.path), book);
        // Read-only: the OPDS server never migrates or rewrites the index.
        assert_eq!(
            std::fs::read_to_string(root.join(".librarium/books.json")).unwrap(),
            v1.to_string()
        );
        assert!(!root.join(".librarium/library.json").exists());
        assert_eq!(
            Catalog::load(&root.join("not-there"), chrono::Utc::now()).len(),
            0
        );
    }

    #[test]
    fn catalog_falls_back_to_the_backup_index() {
        let f = fixture();
        let dir = f.lib.path().join(".librarium");
        std::fs::rename(dir.join("library.json"), dir.join("library.json.bak")).unwrap();
        std::fs::write(dir.join("library.json"), "{ truncated").unwrap();
        assert_eq!(Catalog::load(f.lib.path(), chrono::Utc::now()).len(), 2);
    }

    #[test]
    fn accept_negotiation() {
        let prefers = |accept: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
            prefers_opds2(&headers)
        };
        assert!(prefers("application/opds+json"));
        assert!(prefers("application/json, */*;q=0.1"));
        assert!(prefers(
            "application/atom+xml;q=0.4, application/opds+json;Q=0.9"
        ));
        assert!(!prefers("application/atom+xml, application/opds+json"));
        assert!(!prefers("*/*"));
        assert!(!prefers("text/html"));
        assert!(!prefers_opds2(&HeaderMap::new()));
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    /// A port that was free a moment ago.
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[tokio::test]
    async fn lifecycle_start_restart_stop() {
        let f = fixture();
        let other = tempfile::tempdir().unwrap();
        let state = OpdsServerState::default();
        assert_eq!(state.status().await, None);

        let port = free_port();
        let status = state
            .start(OpdsSettings::new(port, f.lib.path(), false, None))
            .await
            .unwrap();
        assert_eq!(
            status,
            OpdsStatus {
                running: true,
                port,
                local_url: format!("http://127.0.0.1:{port}/opds"),
                lan_urls: vec![],
            }
        );
        // No connection pooling: the old server closes its keep-alive
        // connections on shutdown, and a pooled one would be reused below.
        // (reqwest is built without a default TLS provider; net.rs installs
        // one in the app, this test may run before anything else does.)
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap();
        let resp = client.get(&status.local_url).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert!(resp.headers().get("access-control-allow-origin").is_none());
        assert!(
            resp.text().await.unwrap().contains("<feed"),
            "Atom by default"
        );
        // A real socket goes through the connection limiter and ConnInfo.
        let range = client
            .get(format!(
                "http://127.0.0.1:{port}{}",
                urls::download(BOOK_ID)
            ))
            .header("range", "bytes=0-3")
            .send()
            .await
            .unwrap();
        assert_eq!(range.status().as_u16(), 206);
        assert_eq!(range.bytes().await.unwrap().as_ref(), b"0123");

        // Same settings: no restart. Different library on the same port: the
        // old server must release the port before the new one binds it.
        assert_eq!(
            state
                .start(OpdsSettings::new(port, f.lib.path(), false, None))
                .await
                .unwrap(),
            status
        );
        let restarted = state
            .start(OpdsSettings::new(port, other.path(), false, None))
            .await
            .unwrap();
        assert_eq!(restarted.port, port);
        let feed: serde_json::Value = client
            .get(format!("http://127.0.0.1:{port}/opds/publications"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            feed["publications"].as_array().unwrap().len(),
            0,
            "serves the new library"
        );

        state.stop().await;
        assert_eq!(state.status().await, None);
        assert!(
            client.get(&status.local_url).send().await.is_err(),
            "port closed"
        );
    }

    #[tokio::test]
    async fn lan_mode_needs_a_password_and_a_library() {
        let f = fixture();
        let state = OpdsServerState::default();
        for password in [None, Some(String::new()), Some("   ".to_string())] {
            let err = state
                .start(OpdsSettings::new(0, f.lib.path(), true, password))
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("password is required"), "{err}");
        }
        let err = state
            .start(OpdsSettings::new(
                0,
                f.lib.path().join("missing"),
                false,
                None,
            ))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("can't be reached"), "{err}");
        assert_eq!(state.status().await, None);
    }

    #[test]
    fn password_is_dropped_outside_lan_mode() {
        let a = OpdsSettings::new(8042, "/lib", false, Some("x".into()));
        let b = OpdsSettings::new(8042, "/lib", false, None);
        assert!(
            a == b,
            "changing an unused password doesn't restart the server"
        );
    }

    #[tokio::test]
    async fn qr_command_encodes_urls() {
        assert!(opds_qr_svg("http://192.168.1.20:8042/opds".into())
            .await
            .unwrap()
            .contains("<svg"));
        assert!(opds_qr_svg(String::new()).await.is_err());
    }
}
