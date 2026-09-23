//! Local web UI: axum server over the shared package index.
//!
//! Only compiled with the `web` cargo feature. Serves embedded static assets
//! (no filesystem access) and a read-only JSON API. Binds 127.0.0.1 only;
//! rejects non-loopback peers and non-local Host headers (DNS-rebinding
//! guard). All API responses carry a `generation` so clients can detect
//! index swaps (e.g. a background rebuild).

use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_SECURITY_POLICY,
    CONTENT_TYPE, HOST, ORIGIN, REFERRER_POLICY, VARY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;

use crate::graph::{project, Dir, NODE_BUDGET};
use crate::index::Index;
use crate::indexer::{self, Cancel, IndexEvent};
use crate::search::SearchEngine;

const INDEX_HTML: &str = include_str!("../../web/index.html");
const APP_JS: &str = include_str!("../../web/app.js");
const GRAPH_JS: &str = include_str!("../../web/graph.js");
const STYLE_CSS: &str = include_str!("../../web/style.css");
const FAVICON: &[u8] = include_bytes!("../../web/favicon.svg");

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
                   connect-src 'self'; img-src 'self' data:; \
                   frame-ancestors 'none'; base-uri 'none'";

#[derive(Debug)]
enum WebPhase {
    Loading { done: u64, total: u64 },
    Ready,
    Failed { msg: String },
}

struct StateInner {
    index: Option<Arc<Index>>,
    engine: Option<Arc<SearchEngine>>,
    generation: u64,
    phase: WebPhase,
}

pub struct AppState {
    inner: RwLock<StateInner>,
    semaphore: Arc<tokio::sync::Semaphore>,
    _cancel: Cancel,
}

/// A consistent snapshot of the current index generation.
type Snapshot = (Arc<Index>, Arc<SearchEngine>, u64);

impl AppState {
    /// Spawn the loader (cache or `guix repl`) and a pump thread that swaps
    /// the index into the shared state; serves 503 until an index is ready.
    pub fn start(force_rebuild: bool) -> Arc<AppState> {
        let cancel: Cancel = Arc::new(Default::default());
        let state = Arc::new(AppState {
            inner: RwLock::new(StateInner {
                index: None,
                engine: None,
                generation: 0,
                phase: WebPhase::Loading { done: 0, total: 0 },
            }),
            semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
            _cancel: Arc::clone(&cancel),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let loader = indexer::start_loader(tx, Arc::clone(&cancel), force_rebuild);
        let pump_state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("guixvis-web-pump".into())
            .spawn(move || pump(pump_state, rx, loader))
            .expect("failed to spawn web pump thread");
        state
    }

    /// Clone the current (index, engine, generation) out of the lock.
    /// Poisoned lock and missing index are indistinguishable from a 503.
    fn snapshot(&self) -> Option<Snapshot> {
        let guard = self.inner.read().ok()?;
        let index = guard.index.clone()?;
        let engine = guard.engine.clone()?;
        Some((index, engine, guard.generation))
    }

    /// Build a state around an existing index (tests).
    pub fn with_index(index: Index) -> Arc<AppState> {
        let engine = Arc::new(SearchEngine::new(&index));
        Arc::new(AppState {
            inner: RwLock::new(StateInner {
                index: Some(Arc::new(index)),
                engine: Some(engine),
                generation: 1,
                phase: WebPhase::Ready,
            }),
            semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
            _cancel: Arc::new(Default::default()),
        })
    }

    fn phase(&self) -> serde_json::Value {
        let guard = self.inner.read();
        match guard {
            Ok(g) => match &g.phase {
                WebPhase::Loading { done, total } => json!({
                    "phase": "loading", "done": done, "total": total
                }),
                WebPhase::Ready => json!({ "phase": "ready" }),
                WebPhase::Failed { msg } => json!({ "phase": "failed", "error": msg }),
            },
            Err(_) => json!({ "phase": "failed", "error": "internal state poisoned" }),
        }
    }
}

fn pump(state: Arc<AppState>, rx: Receiver<IndexEvent>, loader: std::thread::JoinHandle<()>) {
    for ev in rx {
        // Prepare the replacement before locking so requests can continue
        // using the previous generation while its search data is built.
        let engine = match &ev {
            IndexEvent::Ready { index, .. } => Some(Arc::new(SearchEngine::new(index))),
            _ => None,
        };
        let mut guard = match state.inner.write() {
            Ok(g) => g,
            Err(_) => continue,
        };
        match ev {
            IndexEvent::Progress { done, total } => {
                guard.phase = WebPhase::Loading { done, total };
            }
            IndexEvent::Ready { index, .. } => {
                guard.index = Some(index);
                guard.engine = engine;
                guard.generation = guard.generation.wrapping_add(1);
                guard.phase = WebPhase::Ready;
            }
            IndexEvent::Failed { msg } => {
                guard.phase = WebPhase::Failed { msg };
            }
        }
    }
    let _ = loader.join();
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

enum ApiError {
    BadRequest(String),
    NotFound,
    Unavailable(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, json!({ "error": msg })),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                json!({ "error": "package not found" }),
            ),
            ApiError::Unavailable(msg) => {
                (StatusCode::SERVICE_UNAVAILABLE, json!({ "error": msg }))
            }
        };
        (status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Security middleware
// ---------------------------------------------------------------------------

/// Reject requests whose Host header is not local and peers that are not on
/// loopback. Together these defeat DNS-rebinding reads of the local API.
async fn local_only(req: Request<axum::body::Body>, next: Next) -> Result<Response, StatusCode> {
    let host = req.headers().get(HOST).and_then(|h| h.to_str().ok());
    let host_ok =
        req.headers().get_all(HOST).iter().count() == 1 && host.and_then(local_authority).is_some();
    let peer_loopback = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false);
    if !host_ok || !peer_loopback {
        return Err(StatusCode::FORBIDDEN);
    }
    // A browser on another site can still reach 127.0.0.1, so refuse
    // requests it marks as cross-origin. Plain navigations send neither
    // header and keep working.
    if let Some(origin) = req.headers().get(ORIGIN) {
        if req.headers().get_all(ORIGIN).iter().count() != 1
            || !origin
                .to_str()
                .ok()
                .is_some_and(|origin| origin_matches_host(origin, host.unwrap_or_default()))
        {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    if let Some(site) = req.headers().get("sec-fetch-site") {
        if req.headers().get_all("sec-fetch-site").iter().count() != 1
            || !site
                .to_str()
                .ok()
                .is_some_and(|site| matches!(site, "same-origin" | "same-site" | "none"))
        {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(next.run(req).await)
}

/// Only this HTTP server's exact origin may access it from a browser.
fn origin_matches_host(origin: &str, host: &str) -> bool {
    let Some((origin_host, origin_port)) = origin.strip_prefix("http://").and_then(local_authority)
    else {
        return false;
    };
    local_authority(host)
        .is_some_and(|(host, port)| host.eq_ignore_ascii_case(origin_host) && port == origin_port)
}

/// Baseline hardening for every response; the API is never cached.
async fn security_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    // Every route carries the policy, not just the HTML document.
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("geolocation=(), microphone=(), camera=(), payment=(), usb=()"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    if api {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

/// Compress API responses when the client asks for it. Graph payloads for 200
/// nodes are tens of kilobytes of very repetitive JSON; gzip cuts them to a
/// fraction and flate2 is already a dependency of the cache.
async fn compress_api(req: Request<axum::body::Body>, next: Next) -> Response {
    use std::io::Write as _;

    let gzip_ok = accepts_gzip(req.headers());
    let api = req.uri().path().starts_with("/api/");
    let res = next.run(req).await;
    if !gzip_ok || !api {
        return res;
    }

    let (mut parts, body) = res.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 8 * 1024 * 1024).await else {
        // Do not retain a successful status or the original Content-Length
        // after collection failed. The outer layer adds security headers.
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "response exceeded the encoding limit"})),
        )
            .into_response();
    };
    if bytes.len() < 1024 {
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
    let compressed = match encoder.write_all(&bytes).and_then(|()| encoder.finish()) {
        Ok(c) => c,
        Err(_) => return Response::from_parts(parts, axum::body::Body::from(bytes)),
    };
    parts
        .headers
        .insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    parts
        .headers
        .insert(VARY, HeaderValue::from_static("accept-encoding"));
    if let Ok(len) = HeaderValue::from_str(&compressed.len().to_string()) {
        parts.headers.insert(CONTENT_LENGTH, len);
    }
    Response::from_parts(parts, axum::body::Body::from(compressed))
}

fn accepts_gzip(headers: &HeaderMap) -> bool {
    let mut gzip = None;
    let mut wildcard = None;
    for header in headers.get_all(ACCEPT_ENCODING) {
        let Ok(header) = header.to_str() else {
            return false;
        };
        for entry in header.split(',') {
            let mut parts = entry.split(';');
            let encoding = parts.next().unwrap_or_default().trim();
            let slot = if encoding.eq_ignore_ascii_case("gzip")
                || encoding.eq_ignore_ascii_case("x-gzip")
            {
                &mut gzip
            } else if encoding == "*" {
                &mut wildcard
            } else {
                continue;
            };
            let mut allowed = true;
            let mut seen_quality = false;
            for parameter in parts {
                let Some((name, value)) = parameter.trim().split_once('=') else {
                    allowed = false;
                    break;
                };
                if !name.trim().eq_ignore_ascii_case("q") || seen_quality {
                    allowed = false;
                    break;
                }
                seen_quality = true;
                allowed = positive_quality(value.trim());
            }
            // Repeated entries honor an explicit refusal. An explicit gzip
            // entry always overrides the wildcard, regardless of order.
            *slot = Some(slot.unwrap_or(true) && allowed);
        }
    }
    gzip.or(wildcard).unwrap_or(false)
}

fn positive_quality(value: &str) -> bool {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match whole {
        "1" => fraction.bytes().all(|b| b == b'0'),
        "0" => fraction.bytes().any(|b| b != b'0'),
        _ => false,
    }
}

/// Package names come from the URL; keep them to what Guix actually uses so
/// nothing exotic reaches the index or the JSON encoder.
fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.' | '_'))
}

fn local_authority(value: &str) -> Option<(&str, u16)> {
    // Parse the entire authority; accepting only a prefix would also admit
    // malformed ports, userinfo, paths, and text after an IPv6 bracket.
    let (host, suffix) = if value.starts_with('[') {
        let end = value.find(']')? + 1;
        (&value[..end], &value[end..])
    } else if let Some(colon) = value.find(':') {
        (&value[..colon], &value[colon..])
    } else {
        (value, "")
    };
    if !host.eq_ignore_ascii_case("localhost") && !matches!(host, "127.0.0.1" | "[::1]") {
        return None;
    }
    let port = if suffix.is_empty() {
        80
    } else {
        let port = suffix.strip_prefix(':')?;
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        port.parse::<u16>().ok()?
    };
    Some((host, port))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_html))
        .route("/favicon.ico", get(favicon))
        .route("/app.js", get(app_js))
        .route("/graph.js", get(graph_js))
        .route("/style.css", get(style_css))
        .route("/api/v1/health", get(health))
        .route("/api/v1/search", get(search))
        .route("/api/v1/package/{name}", get(package))
        .route("/api/v1/graph/{name}", get(graph))
        .layer(middleware::from_fn(compress_api))
        .layer(middleware::from_fn(local_only))
        .layer(middleware::from_fn(security_headers))
        // Read-only API: nothing legitimate arrives with a body.
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(state)
}

async fn index_html() -> impl IntoResponse {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    (headers, Html(INDEX_HTML))
}

async fn app_js() -> impl IntoResponse {
    static_js(APP_JS)
}

async fn favicon() -> impl IntoResponse {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    (headers, FAVICON)
}

async fn graph_js() -> impl IntoResponse {
    static_js(GRAPH_JS)
}

async fn style_css() -> impl IntoResponse {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    (headers, STYLE_CSS)
}

fn static_js(body: &'static str) -> impl IntoResponse {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    (headers, body)
}

#[derive(Debug, Serialize)]
struct Health {
    ok: bool,
    packages: usize,
    generation: u64,
    guix_commit: String,
    #[serde(flatten)]
    phase: serde_json::Value,
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    let snap = state.snapshot();
    let (packages, generation, commit, ok) = match snap {
        Some((index, _, generation)) => (index.len(), generation, index.guix_commit.clone(), true),
        None => (0, 0, String::new(), false),
    };
    let phase = state.phase();
    let mut phase_obj = serde_json::Map::new();
    if let serde_json::Value::Object(m) = phase {
        phase_obj = m;
    }
    let health = Health {
        ok,
        packages,
        generation,
        guix_commit: commit,
        phase: serde_json::Value::Object(phase_obj),
    };
    (StatusCode::OK, Json(health)).into_response()
}

#[derive(Debug, serde::Deserialize)]
struct SearchParams {
    q: Option<String>,
    limit: Option<usize>,
}

async fn search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Result<Response, ApiError> {
    let q = params.q.unwrap_or_default();
    if q.chars().count() > 200 {
        return Err(ApiError::BadRequest(
            "query too long (max 200 chars)".into(),
        ));
    }
    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let Some((index, engine, generation)) = state.snapshot() else {
        return Err(ApiError::Unavailable("index is still building".into()));
    };
    let permit = state
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::Unavailable("shutting down".into()))?;
    let index_for_items = Arc::clone(&index);
    let hits = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        engine.search(&index, &q, limit + 1)
    })
    .await
    .map_err(|_| ApiError::Unavailable("search task failed".into()))?;
    let capped = hits.len() > limit;
    let items: Vec<serde_json::Value> = hits
        .iter()
        .take(limit)
        .map(|h| {
            let p = &index_for_items.packages[h.hit.id as usize];
            json!({
                "name": p.name.as_ref(),
                "version": p.version.as_ref(),
                "synopsis": p.synopsis.as_ref(),
                "deps": p.dep_count(),
                "dependents": index_for_items.dependents_count(p.id),
                "license": p.licenses.first().map(|l| l.as_ref()),
                "nameMatch": h.hit.name_match,
                "name_spans": h.name_ranges,
                "synopsis_spans": h.synopsis_ranges,
            })
        })
        .collect();
    Ok((
        StatusCode::OK,
        Json(json!({
            "generation": generation,
            "capped": capped,
            "items": items,
        })),
    )
        .into_response())
}

async fn package(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    if !valid_package_name(&name) {
        return Err(ApiError::BadRequest("invalid package name".into()));
    }
    let Some((index, _, generation)) = state.snapshot() else {
        return Err(ApiError::Unavailable("index is still building".into()));
    };
    let permit = state
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::Unavailable("shutting down".into()))?;
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ApiError> {
        let _permit = permit;
        let Some(id) = index.names.get(name.as_str()).copied() else {
            return Err(ApiError::NotFound);
        };
        let p = &index.packages[id as usize];
        let deps: Vec<serde_json::Value> = p
            .deps()
            .map(|(dep, kind)| {
                let d = &index.packages[dep as usize];
                json!({
                    "name": d.name.as_ref(),
                    "version": d.version.as_ref(),
                    "kind": match kind {
                        crate::index::DepKind::Input => "input",
                        crate::index::DepKind::Propagated => "propagated",
                        crate::index::DepKind::Native => "native",
                    },
                })
            })
            .collect();
        let dependents: Vec<serde_json::Value> = index.dependents[id as usize]
            .iter()
            .map(|d| {
                let dep = &index.packages[*d as usize];
                json!({
                    "name": dep.name.as_ref(),
                    "version": dep.version.as_ref(),
                    "dependents": index.dependents_count(*d),
                })
            })
            .collect();
        let neighbors: Vec<serde_json::Value> = index
            .module_neighbors(id)
            .iter()
            .filter(|n| **n != id)
            .map(|n| {
                let nb = &index.packages[*n as usize];
                json!({ "name": nb.name.as_ref(), "version": nb.version.as_ref() })
            })
            .collect();
        Ok(json!({
            "generation": generation,
            "name": p.name.as_ref(),
            "version": p.version.as_ref(),
            "synopsis": p.synopsis.as_ref(),
            "description": p.description.as_ref(),
            "homepage": p.homepage.as_ref(),
            "licenses": p.licenses.iter().map(|l| l.as_ref()).collect::<Vec<_>>(),
            "file": p.file.as_ref(),
            "line": p.line,
            "deps": deps,
            "dependents": dependents,
            "dependents_count": index.dependents_count(id),
            "module_neighbors": neighbors,
        }))
    })
    .await
    .map_err(|_| ApiError::Unavailable("package task failed".into()))??;
    Ok((StatusCode::OK, Json(body)).into_response())
}

#[derive(Debug, serde::Deserialize)]
struct GraphParams {
    dir: Option<String>,
    depth: Option<u8>,
    budget: Option<usize>,
}

async fn graph(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(params): Query<GraphParams>,
) -> Result<Response, ApiError> {
    if !valid_package_name(&name) {
        return Err(ApiError::BadRequest("invalid package name".into()));
    }
    let dir = match params.dir.as_deref() {
        None | Some("deps") => Dir::Deps,
        Some("reverse") | Some("dependents") => Dir::Dependents,
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "invalid dir {other:?} (use deps|reverse)"
            )))
        }
    };
    let depth = params.depth.unwrap_or(2);
    if !(1..=8).contains(&depth) {
        return Err(ApiError::BadRequest("depth must be 1..=8".into()));
    }
    let budget = params.budget.unwrap_or(NODE_BUDGET).min(NODE_BUDGET);
    let Some((index, _, generation)) = state.snapshot() else {
        return Err(ApiError::Unavailable("index is still building".into()));
    };
    let permit = state
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| ApiError::Unavailable("shutting down".into()))?;
    let body = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, ApiError> {
        let _permit = permit;
        let Some(root) = index.names.get(name.as_str()).copied() else {
            return Err(ApiError::NotFound);
        };
        let projection = project(&index, root, dir, depth, budget);
        let nodes: Vec<serde_json::Value> = projection
            .nodes
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let p = &index.packages[*id as usize];
                json!({
                    "name": p.name.as_ref(),
                    "version": p.version.as_ref(),
                    "degree": index.dependents_count(*id) + p.dep_count(),
                    "depth": projection.depth_of[i],
                    "kind": projection.kind_of[i].map(|k| match k {
                        crate::index::DepKind::Input => "input",
                        crate::index::DepKind::Propagated => "propagated",
                        crate::index::DepKind::Native => "native",
                    }),
                })
            })
            .collect();
        let edges: Vec<serde_json::Value> = projection
            .edges
            .iter()
            .map(|(from, to)| {
                let from_name = &index.packages[projection.nodes[*from as usize] as usize].name;
                let to_name = &index.packages[projection.nodes[*to as usize] as usize].name;
                json!({
                    "from": from_name.as_ref(),
                    "to": to_name.as_ref(),
                })
            })
            .collect();
        Ok(json!({
            "generation": generation,
            "root": name,
            "dir": if dir == Dir::Deps { "deps" } else { "reverse" },
            "depth": depth,
            "materialized": nodes.len(),
            "truncated": projection.truncated,
            "nodes": nodes,
            "edges": edges,
        }))
    })
    .await
    .map_err(|_| ApiError::Unavailable("graph task failed".into()))??;
    Ok((StatusCode::OK, Json(body)).into_response())
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Bind 127.0.0.1 and serve until Ctrl+C.
pub async fn serve(state: Arc<AppState>, port: u16) -> anyhow::Result<()> {
    let app = router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!(
        "guixvis web: serving on http://{} (Ctrl+C to stop)",
        listener.local_addr()?
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_err() {
        // Signals unavailable (e.g. odd platforms): never exit early.
        std::future::pending::<()>().await;
    }
}
