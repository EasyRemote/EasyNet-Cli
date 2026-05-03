// EasyNet CLI — Hub: pages HTTP listener
// ======================================
//
// File: src/runtime/hub/pages_listener.rs
// Description: in-daemon axum listener bound to
//              `127.0.0.1:<port>`. Per-request: parses the Host
//              header into `(project, user)`, calls the Hub's
//              `pages.serve` adapter, and frames the result as
//              an HTTP response.
//
//              This listener is the MVP's HTTP boundary. Production
//              traffic enters EasyNet through the Go backend's
//              wildcard listener at `*.*.pages.easynet.run`; the
//              cut-over (Phase 2) replaces this listener but
//              keeps the same `01HUB.pages.serve` ability shape.
//
// Conformance: RFC-006-B v0.6 §3.2 (URL form),
//              INV-1 (Adapter Purity), INV-3 (Deterministic
//              Projection — bytes are returned verbatim from the
//              fetch ability).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::any;
use axum::Router;

use super::pages_serve_ability::{serve_bytes, ServedBytes};

/// Bind the listener and return a future the daemon can `tokio::spawn`.
/// Returns immediately; the listener runs until the process exits.
pub async fn run(port: u16) -> anyhow::Result<()> {
    let app = Router::new().fallback(any(handle));
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("pages listener bind failed on {addr}: {e}"))?;

    eprintln!("[pages-listener] bound to http://{addr}/");

    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("pages listener serve loop exited: {e}"))?;
    Ok(())
}

/// Parse a Host header of the form `<project>.<user>.pages.localhost[:<port>]`
/// (or `*.*.pages.<realm>` in production) into the two leading
/// segments. Returns `None` for any host that does not match the
/// `*.*.pages.*` pattern; the listener uses 404 for those.
fn parse_pages_host(host: &str) -> Option<(String, String)> {
    // strip ":<port>" if present
    let host_no_port = host.split_once(':').map(|(h, _)| h).unwrap_or(host);
    let segments: Vec<&str> = host_no_port.split('.').collect();
    // Need at least 4 segments: <project>.<user>.pages.<rest>
    if segments.len() < 4 {
        return None;
    }
    if segments[2] != "pages" {
        return None;
    }
    Some((segments[0].to_string(), segments[1].to_string()))
}

/// Single fallback handler for every method + every path. Reads
/// the Host header, parses it, dispatches to the serve adapter,
/// builds the HTTP response.
async fn handle(req: Request<Body>) -> Response<Body> {
    let method = req.method().clone();

    let host_header = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let (project_id, user) = match parse_pages_host(host_header) {
        Some(pair) => pair,
        None => return text_response(StatusCode::NOT_FOUND, "unknown host\n"),
    };

    let path = req.uri().path().to_string();
    if path.is_empty() {
        return text_response(StatusCode::NOT_FOUND, "missing path\n");
    }

    // ─── /api/<verb> route — RFC-006-B v0.6 §10 dynamic backend ─
    if let Some(verb) = path.strip_prefix("/api/") {
        if verb.is_empty() || verb.contains('/') {
            return text_response(StatusCode::NOT_FOUND, "api verb invalid\n");
        }
        if !matches!(
            method,
            axum::http::Method::GET | axum::http::Method::POST | axum::http::Method::OPTIONS
        ) {
            return text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n");
        }
        if matches!(method, axum::http::Method::OPTIONS) {
            return cors_preflight();
        }

        let body_bytes = match axum::body::to_bytes(req.into_body(), 4 * 1024 * 1024).await {
            Ok(b) => b,
            Err(_) => return text_response(StatusCode::BAD_REQUEST, "body too large\n"),
        };
        let body_value: serde_json::Value = if body_bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&body_bytes).unwrap_or(serde_json::Value::Null)
        };

        let user_owned = user.clone();
        let project_owned = project_id.clone();
        let verb_owned = verb.to_string();
        let method_str = method.to_string();
        let api_result = tokio::task::spawn_blocking(move || {
            crate::runtime::agents::pages::api::handle_api(
                &user_owned,
                &project_owned,
                &verb_owned,
                serde_json::json!({ "body": body_value, "method": method_str }),
            )
        })
        .await;

        return match api_result {
            Ok(Ok(value)) => api_response(value),
            Ok(Err(e)) => {
                let msg = e.to_string();
                let status = if msg.contains("not published") {
                    StatusCode::SERVICE_UNAVAILABLE
                } else if msg.contains("file not found") || msg.contains("manifest") {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                text_response(status, &format!("api error: {msg}\n"))
            }
            Err(_) => text_response(StatusCode::INTERNAL_SERVER_ERROR, "api panic\n"),
        };
    }

    // ─── static byte route ──────────────────────────────────────
    if !matches!(method, axum::http::Method::GET | axum::http::Method::HEAD) {
        return text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n");
    }

    // For root path, serve `/index.html` so a browser request to
    // `https://<project>.<user>.pages.<realm>/` lands somewhere
    // useful. Note: this is a Hub-side convention (Adapter
    // Purity is preserved — no state change), not a project-level
    // SPA fallback.
    let request_path = if path == "/" {
        "/index.html".to_string()
    } else {
        path
    };

    // Run the fetch handler synchronously inside spawn_blocking so
    // the file read doesn't block the tokio reactor.
    let user_owned = user.clone();
    let project_owned = project_id.clone();
    let path_owned = request_path.clone();
    let served = tokio::task::spawn_blocking(move || {
        serve_bytes(&user_owned, &project_owned, &path_owned)
    })
    .await
    .unwrap_or_else(|_| ServedBytes {
        status: 500,
        bytes: Vec::new(),
        content_type: "text/plain; charset=utf-8".to_string(),
        force_attachment: false,
        sha256: String::new(),
    });

    if served.status != 200 {
        let msg = match served.status {
            404 => "not found\n",
            502 => "upstream too large\n",
            503 => "project not published\n",
            _ => "error\n",
        };
        return text_response(StatusCode::from_u16(served.status).unwrap_or(StatusCode::NOT_FOUND), msg);
    }

    let mut builder = Response::builder().status(StatusCode::OK);
    let headers = builder.headers_mut().expect("builder always has headers");
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&served.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from(served.bytes.len()),
    );
    if served.force_attachment {
        let _ = headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }
    if !served.sha256.is_empty() {
        // ETag derived from the byte hash. INV-3 (Deterministic
        // Projection) makes this stable across hosts.
        if let Ok(v) = HeaderValue::from_str(&format!("\"sha256-{}\"", &served.sha256[..16])) {
            headers.insert(header::ETAG, v);
        }
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );

    let body = if matches!(method, axum::http::Method::HEAD) {
        Body::empty()
    } else {
        Body::from(served.bytes)
    };

    builder.body(body).expect("response build")
}

fn text_response(status: StatusCode, msg: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(msg.to_owned()))
        .expect("text response build")
}

/// CORS preflight — the Hub allows cross-origin POST so a frontend
/// served at `<project>.<user>.pages.localhost:<port>` can call
/// `<other-project>.<user>.pages.localhost:<port>/api/...` without
/// the browser refusing. v0 uses a wide-open policy because the
/// MVP target is a single-host demo; production tightens this with
/// declared origins per-project (post-MVP).
fn cors_preflight() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header("Access-Control-Allow-Headers", "content-type")
        .header("Access-Control-Max-Age", "600")
        .body(Body::empty())
        .expect("preflight response")
}

/// Wrap an api ability's `{status, body, content_type}` shape into
/// an HTTP response. Adds `Access-Control-Allow-Origin: *` so the
/// frontend on a sibling subdomain receives the body.
fn api_response(value: serde_json::Value) -> Response<Body> {
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_u64)
        .and_then(|s| u16::try_from(s).ok())
        .unwrap_or(200);
    let content_type = value
        .get("content_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("application/json; charset=utf-8")
        .to_string();
    let body = match value.get("body") {
        Some(b) => serde_json::to_vec(b).unwrap_or_default(),
        None => Vec::new(),
    };

    let mut builder = Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK));
    let headers = builder.headers_mut().expect("builder always has headers");
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/json; charset=utf-8")),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
    headers.insert(
        "Access-Control-Allow-Origin",
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    builder.body(Body::from(body)).expect("api response build")
}

#[cfg(test)]
mod tests {
    use super::parse_pages_host;

    #[test]
    fn parses_localhost() {
        let (p, u) = parse_pages_host("papers.alice.pages.localhost:8787").unwrap();
        assert_eq!(p, "papers");
        assert_eq!(u, "alice");
    }

    #[test]
    fn parses_real_realm() {
        let (p, u) = parse_pages_host("shop.bob.pages.easynet.run").unwrap();
        assert_eq!(p, "shop");
        assert_eq!(u, "bob");
    }

    #[test]
    fn rejects_short_host() {
        assert!(parse_pages_host("alice.pages.localhost").is_none());
    }

    #[test]
    fn rejects_no_pages_segment() {
        assert!(parse_pages_host("papers.alice.elsewhere.com").is_none());
    }
}
