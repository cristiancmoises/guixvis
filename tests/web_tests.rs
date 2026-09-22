//! Web API handler tests (only built with the `web` feature).
//!
//! Driven through `tower::ServiceExt::oneshot` — no sockets, no runtime
//! blocking, deterministic fixture data.

#![cfg(feature = "web")]

mod common;

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use common::load_fixture;
use guixvis::web::{router, AppState};

fn state() -> std::sync::Arc<AppState> {
    AppState::with_index(load_fixture())
}

fn get(path: &str) -> Request<Body> {
    Request::get(path)
        .header("host", "127.0.0.1:8787")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))))
        .body(Body::empty())
        .expect("request builds")
}

async fn json_of(res: axum::response::Response) -> serde_json::Value {
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

#[tokio::test]
async fn health_reports_generation_and_packages() {
    let res = router(state())
        .oneshot(get("/api/v1/health"))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert_eq!(body["ok"], true);
    assert_eq!(body["packages"], 10);
    assert_eq!(body["generation"], 1);
    assert_eq!(body["phase"], "ready");
}

#[tokio::test]
async fn search_ranks_emacs_first() {
    let res = router(state())
        .oneshot(get("/api/v1/search?q=emac&limit=10"))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    let items = body["items"].as_array().expect("items");
    assert_eq!(items[0]["name"], "emacs");
    assert!(!items[0]["name_spans"].as_array().unwrap().is_empty());
    // Fuzzy highlight spans lie within the 5-char name.
    for span in items[0]["name_spans"].as_array().unwrap() {
        let end = span[1].as_u64().unwrap();
        assert!(end <= 5, "span must stay within \"emacs\"");
    }
}

#[tokio::test]
async fn search_bounds() {
    let app = router(state());

    let res = app
        .clone()
        .oneshot(get("/api/v1/search?q=emac&limit=abc"))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "limit=abc must 400");

    let res = app
        .clone()
        .oneshot(get("/api/v1/search?q=emac&limit=1"))
        .await
        .expect("call");
    let body = json_of(res).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["capped"], true);

    let res = app
        .clone()
        .oneshot(get("/api/v1/search?q=emac&limit=600"))
        .await
        .expect("call");
    let body = json_of(res).await;
    assert!(body["items"].as_array().unwrap().len() <= 500);

    // Over-long query rejected.
    let long_q = "x".repeat(201);
    let res = app
        .oneshot(get(&format!("/api/v1/search?q={long_q}")))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn package_detail_and_404() {
    let app = router(state());

    let res = app
        .clone()
        .oneshot(get("/api/v1/package/emacs"))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert_eq!(body["name"], "emacs");
    assert_eq!(body["deps"].as_array().unwrap().len(), 4);
    assert_eq!(body["generation"], 1);

    let res = app
        .oneshot(get("/api/v1/package/does-not-exist"))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn graph_deps_and_reverse() {
    let app = router(state());

    let res = app
        .clone()
        .oneshot(get("/api/v1/graph/emacs?dir=deps&depth=1"))
        .await
        .expect("call");
    let body = json_of(res).await;
    assert_eq!(body["root"], "emacs");
    assert_eq!(body["nodes"].as_array().unwrap().len(), 5, "root + 4 deps");
    // 4 edges from emacs + the transitive gtk+ -> zlib edge.
    assert_eq!(body["edges"].as_array().unwrap().len(), 5);
    assert_eq!(body["truncated"], 0);

    // Reverse of zlib: 4 direct dependents.
    let res = app
        .clone()
        .oneshot(get("/api/v1/graph/zlib?dir=reverse&depth=1"))
        .await
        .expect("call");
    let body = json_of(res).await;
    assert_eq!(body["dir"], "reverse");
    let names: Vec<&str> = body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"zlib"));
    assert!(names.contains(&"emacs"));
    assert_eq!(names.len(), 5);

    // Bad params.
    let res = app
        .clone()
        .oneshot(get("/api/v1/graph/emacs?depth=9"))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = app
        .oneshot(get("/api/v1/graph/emacs?dir=sideways"))
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn host_allowlist_rejects_foreign_hosts() {
    let req = Request::get("/api/v1/health")
        .header("host", "evil.example.com")
        .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))))
        .body(Body::empty())
        .expect("request builds");
    let res = router(state()).oneshot(req).await.expect("call");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn static_assets_and_traversal() {
    let app = router(state());

    let res = app.clone().oneshot(get("/")).await.expect("call");
    assert_eq!(res.status(), StatusCode::OK);
    let csp = res.headers().get("content-security-policy").expect("csp");
    assert!(csp.to_str().unwrap().contains("default-src 'self'"));

    let res = app.clone().oneshot(get("/app.js")).await.expect("call");
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/javascript"));

    // Traversal and unknown paths are plain 404s (no filesystem access).
    for path in [
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/etc/passwd",
        "/api/v1/nope",
    ] {
        let res = app.clone().oneshot(get(path)).await.expect("call");
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn live_bind_loopback_and_fetch() {
    // Real socket smoke: bind an ephemeral port, fetch health over TCP.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let state = state();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    );
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        server
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "got: {response}");
    assert!(response.contains("\"generation\":1"));

    let _ = tx.send(());
    handle.await.unwrap();
}

#[tokio::test]
async fn responses_carry_hardening_headers() {
    let res = router(state())
        .oneshot(get("/api/v1/health"))
        .await
        .expect("call");
    let headers = res.headers();
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(headers.get("x-frame-options").unwrap(), "DENY");
    assert_eq!(headers.get("referrer-policy").unwrap(), "no-referrer");
    assert_eq!(
        headers.get("cross-origin-resource-policy").unwrap(),
        "same-origin"
    );
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
}

#[tokio::test]
async fn cross_site_requests_are_refused() {
    // A browser on another origin sends Origin/Sec-Fetch-Site; loopback alone
    // is not proof that the caller is friendly.
    let res = router(state())
        .oneshot(
            Request::get("/api/v1/health")
                .header("host", "127.0.0.1:8787")
                .header("origin", "http://evil.example")
                .header("sec-fetch-site", "cross-site")
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Our own origin still works.
    let res = router(state())
        .oneshot(
            Request::get("/api/v1/health")
                .header("host", "127.0.0.1:8787")
                .header("origin", "http://127.0.0.1:8787")
                .header("sec-fetch-site", "same-origin")
                .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("call");
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn exotic_package_names_are_rejected() {
    for name in ["has%20space", "dot.dot", "a".repeat(200).as_str()] {
        let res = router(state())
            .oneshot(get(&format!("/api/v1/package/{name}")))
            .await
            .expect("call");
        assert!(
            res.status() == StatusCode::BAD_REQUEST || res.status() == StatusCode::NOT_FOUND,
            "{name} produced {}",
            res.status()
        );
    }
    // A normal name still resolves (and gtk+ keeps its plus sign).
    for name in ["emacs", "gtk%2B", "pkg-config"] {
        let res = router(state())
            .oneshot(get(&format!("/api/v1/package/{name}")))
            .await
            .expect("call");
        assert_eq!(res.status(), StatusCode::OK, "{name} was refused");
    }
}

#[tokio::test]
async fn static_assets_carry_the_policy_too() {
    // The CSP used to be added only by the HTML handler; it belongs on every
    // response, because scripts and styles are exactly what it constrains.
    for path in ["/", "/app.js", "/graph.js", "/style.css"] {
        let res = router(state()).oneshot(get(path)).await.expect("call");
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        let csp = res
            .headers()
            .get("content-security-policy")
            .unwrap_or_else(|| panic!("{path} has no CSP"));
        assert!(
            csp.to_str().unwrap().contains("default-src 'self'"),
            "{path}"
        );
    }
}
