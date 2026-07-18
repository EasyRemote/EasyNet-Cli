// EasyNet CLI — Hub: pages HTTP listener
// ======================================
//
// File: src/daemon/hub/pages_listener.rs
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
use bytes::Bytes;
use std::convert::Infallible;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;

use super::pages_serve_ability::{serve_bytes, ServedBytes};

const PAGES_HEALTH_PATH: &str = "/_easynet/pages/health";

/// Maximum number of sequential ports the daemon tries after the
/// requested Pages port.
pub const DEFAULT_PORT_PROBE_SPAN: u16 = 20;

/// Bind the listener and return a future the daemon can `tokio::spawn`.
/// Returns immediately; the listener runs until the process exits.
pub async fn run(port: u16) -> anyhow::Result<()> {
    let listener = bind_pages_listener(port).await?;

    crate::op_event!(
        component = pages_listener,
        kind = bound,
        addr = listener.local_addr()?,
    );

    axum::serve(listener, pages_router())
        .await
        .map_err(|e| anyhow::anyhow!("pages listener serve loop exited: {e}"))?;
    Ok(())
}

/// Bind the first available Pages port in `[start, start + span]`,
/// spawn the serve loop, and return the actual chosen port.
///
/// Each rejected port emits an `op_event!` line so a user who sees
/// the listener "silently" bound to 8788 can answer "what's wrong
/// with 8787?" from the daemon log alone.
pub async fn spawn_first_available(
    start: u16,
    span: u16,
) -> anyhow::Result<(u16, JoinHandle<anyhow::Result<()>>)> {
    let mut last_err = None;
    for offset in 0..=span {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        match bind_pages_listener(port).await {
            Ok(listener) => {
                let actual_port = listener.local_addr()?.port();
                crate::op_event!(
                    component = pages_listener,
                    kind = bound,
                    addr = listener.local_addr()?,
                );
                let handle = tokio::spawn(async move {
                    axum::serve(listener, pages_router())
                        .await
                        .map_err(|e| anyhow::anyhow!("pages listener serve loop exited: {e}"))
                });
                return Ok((actual_port, handle));
            }
            Err(err) => {
                crate::op_event!(
                    component = pages_listener,
                    kind = bind_skip,
                    port = port,
                    error = err,
                );
                last_err = Some(err);
            }
        }
    }
    Err(last_err
        .unwrap_or_else(|| anyhow::anyhow!("pages listener could not probe any port from {start}")))
}

fn pages_router() -> Router {
    Router::new().fallback(any(handle))
}

async fn bind_pages_listener(port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    // Bind address: 127.0.0.1 by default (dev mode, Mac host
    // daemon). When running inside a container the daemon needs
    // to accept from the container's published-port mapping, so
    // honour `EASYNET_PAGES_BIND` (e.g. `0.0.0.0`) — INV-1
    // (Adapter Purity) is unaffected; the bind address is purely
    // a transport concern.
    let bind_host = std::env::var("EASYNET_PAGES_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let addr: SocketAddr = format!("{bind_host}:{port}")
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid bind addr {bind_host}:{port}: {e}"))?;
    tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("pages listener bind failed on {addr}: {e}"))
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

    let path = req.uri().path().to_string();
    if path.is_empty() {
        return text_response(StatusCode::NOT_FOUND, "missing path\n");
    }

    if path == PAGES_HEALTH_PATH {
        if !matches!(method, axum::http::Method::GET | axum::http::Method::HEAD) {
            return text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed\n");
        }
        return pages_health_response(matches!(method, axum::http::Method::HEAD));
    }

    // ─── /v1/* — RFC-006-C OpenAI-compatibility endpoints ────────
    // These are realm-level (not per-project), so they route on
    // path BEFORE the Host-based pages routing. CORS preflight
    // is permitted for cross-origin browser-side OpenAI clients.
    if path == "/v1/chat/completions" {
        if matches!(method, axum::http::Method::OPTIONS) {
            return cors_preflight();
        }
        if !matches!(method, axum::http::Method::POST) {
            return text_response(StatusCode::METHOD_NOT_ALLOWED, "use POST\n");
        }
        return handle_v1_chat_completions(req).await;
    }
    if path == "/v1/models" {
        if matches!(method, axum::http::Method::OPTIONS) {
            return cors_preflight();
        }
        if !matches!(method, axum::http::Method::GET | axum::http::Method::POST) {
            return text_response(StatusCode::METHOD_NOT_ALLOWED, "use GET\n");
        }
        return handle_v1_models(req).await;
    }

    let host_header = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let (project_id, user) = match parse_pages_host(host_header) {
        Some(pair) => pair,
        None => return text_response(StatusCode::NOT_FOUND, "unknown host\n"),
    };

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
            crate::daemon::ability::builtins::resources::pages::api::handle_api(
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
    let served =
        tokio::task::spawn_blocking(move || serve_bytes(&user_owned, &project_owned, &path_owned))
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
        return text_response(
            StatusCode::from_u16(served.status).unwrap_or(StatusCode::NOT_FOUND),
            msg,
        );
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

    let mut builder =
        Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK));
    let headers = builder.headers_mut().expect("builder always has headers");
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/json; charset=utf-8")),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
    headers.insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    builder.body(Body::from(body)).expect("api response build")
}

fn pages_health_response(head_only: bool) -> Response<Body> {
    let payload = serde_json::json!({
        "status": "ok",
        "pid": std::process::id(),
        "node_id": std::env::var("EASYNET_NODE_ID").ok(),
    });
    let body = if head_only {
        Vec::new()
    } else {
        serde_json::to_vec(&payload).unwrap_or_default()
    };

    let mut builder = Response::builder().status(StatusCode::OK);
    let headers = builder.headers_mut().expect("builder always has headers");
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    builder.body(Body::from(body)).expect("pages health build")
}

// ─── RFC-006-C OpenAI-compat handlers ──────────────────────────

/// `GET /v1/models` — list chat-base abilities as OpenAI models.
async fn handle_v1_models(req: Request<Body>) -> Response<Body> {
    let openai_runtime = req
        .extensions()
        .get::<crate::daemon::ability::builtins::integrations::openai_compat::OpenAICompatRuntime>()
        .cloned();
    let result = tokio::task::spawn_blocking(move || match openai_runtime {
        Some(runtime) => runtime.handle_list_models(serde_json::json!({})),
        None => crate::daemon::ability::builtins::integrations::openai_compat::handle_list_models(
            serde_json::json!({}),
        ),
    })
    .await;

    match result {
        Ok(Ok(value)) => json_response_with_cors(StatusCode::OK, value),
        Ok(Err(e)) => text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("list_models failed: {e}\n"),
        ),
        Err(_) => text_response(StatusCode::INTERNAL_SERVER_ERROR, "panic\n"),
    }
}

/// `POST /v1/chat/completions` — OpenAI-shape chat completion.
/// Streaming (`stream:true`) emits SSE; non-streaming returns one JSON.
async fn handle_v1_chat_completions(req: Request<Body>) -> Response<Body> {
    let openai_runtime = req
        .extensions()
        .get::<crate::daemon::ability::builtins::integrations::openai_compat::OpenAICompatRuntime>()
        .cloned();

    // Parse Authorization → Bearer token. Missing → 401.
    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ").map(|s| s.trim().to_string()));

    let body_bytes = match axum::body::to_bytes(req.into_body(), 4 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return text_response(StatusCode::BAD_REQUEST, "body too large\n"),
    };
    let request_body: serde_json::Value = match serde_json::from_slice(&body_bytes) {
        Ok(v) => v,
        Err(e) => {
            return text_response(
                StatusCode::BAD_REQUEST,
                &format!("invalid JSON body: {e}\n"),
            );
        }
    };

    let stream = request_body
        .get("stream")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    // Build the adapter args envelope.
    let mut adapter_args = serde_json::json!({ "request": request_body });
    if let Some(token) = &bearer {
        adapter_args["auth_token"] = serde_json::json!(token);
    }

    if stream {
        return streaming_chat_completions_response(openai_runtime, adapter_args);
    }

    // Run the adapter on the blocking pool (it can take a long
    // time — calls into the agent dispatcher).
    let adapter_result = tokio::task::spawn_blocking(move || match openai_runtime {
        Some(runtime) => runtime.handle_chat_completions(adapter_args),
        None => {
            crate::daemon::ability::builtins::integrations::openai_compat::handle_chat_completions(
                adapter_args,
            )
        }
    })
    .await;

    let value = match adapter_result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            let msg = e.to_string();
            // INV-2 auth errors map to 401.
            let status = if msg.contains("auth failed") || msg.contains("api key") {
                StatusCode::UNAUTHORIZED
            } else if msg.contains("not registered") || msg.contains("not chat-base") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return text_response(status, &format!("error: {msg}\n"));
        }
        Err(_) => return text_response(StatusCode::INTERNAL_SERVER_ERROR, "panic\n"),
    };

    // Unary path. Strip easynet-only metadata and return.
    let mut response = value;
    if let serde_json::Value::Object(ref mut m) = response {
        m.remove("easynet_user_ura");
    }
    json_response_with_cors(StatusCode::OK, response)
}

fn streaming_chat_completions_response(
    openai_runtime: Option<
        crate::daemon::ability::builtins::integrations::openai_compat::OpenAICompatRuntime,
    >,
    adapter_args: serde_json::Value,
) -> Response<Body> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(16);
    tokio::spawn(async move {
        if tx
            .send(Ok(Bytes::from_static(b": easynet-openai-stream\n\n")))
            .await
            .is_err()
        {
            return;
        }

        let adapter_result = tokio::task::spawn_blocking(move || match openai_runtime {
            Some(runtime) => runtime.handle_chat_completions(adapter_args),
            None => {
                crate::daemon::ability::builtins::integrations::openai_compat::handle_chat_completions(
                    adapter_args,
                )
            }
        })
        .await;

        match adapter_result {
            Ok(Ok(value)) => {
                let chunks = value
                    .get("chunks")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let done = value
                    .get("done_sentinel")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("[DONE]")
                    .to_string();
                for chunk in chunks {
                    if send_sse_json(&tx, &chunk).await.is_err() {
                        return;
                    }
                }
                let _ = send_sse_data(&tx, &done).await;
            }
            Ok(Err(err)) => {
                let _ = send_sse_json(&tx, &openai_stream_error(err.to_string())).await;
                let _ = send_sse_data(&tx, "[DONE]").await;
            }
            Err(err) => {
                let _ = send_sse_json(
                    &tx,
                    &openai_stream_error(format!("chat completion worker join failed: {err}")),
                )
                .await;
                let _ = send_sse_data(&tx, "[DONE]").await;
            }
        }
    });

    let body = Body::from_stream(ReceiverStream::new(rx));
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
        .header("Access-Control-Allow-Origin", HeaderValue::from_static("*"))
        .header("X-Accel-Buffering", HeaderValue::from_static("no"))
        .body(body)
        .expect("sse response build")
}

async fn send_sse_json(
    tx: &tokio::sync::mpsc::Sender<Result<Bytes, Infallible>>,
    value: &serde_json::Value,
) -> Result<(), tokio::sync::mpsc::error::SendError<Result<Bytes, Infallible>>> {
    let line = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    send_sse_data(tx, &line).await
}

async fn send_sse_data(
    tx: &tokio::sync::mpsc::Sender<Result<Bytes, Infallible>>,
    data: &str,
) -> Result<(), tokio::sync::mpsc::error::SendError<Result<Bytes, Infallible>>> {
    tx.send(Ok(Bytes::from(format!("data: {data}\n\n")))).await
}

fn openai_stream_error(message: String) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "message": message,
            "type": "easynet_error",
        }
    })
}

/// JSON response with CORS open. Used by /v1/models and /v1/chat/completions
/// (non-streaming).
fn json_response_with_cors(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let mut builder = Response::builder().status(status);
    let headers = builder.headers_mut().expect("builder always has headers");
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
    headers.insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    builder
        .body(Body::from(body))
        .expect("json/cors response build")
}

#[cfg(test)]
mod tests {
    use super::{handle, pages_health_response, parse_pages_host};
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use serde_json::json;
    use std::sync::{Arc, OnceLock};
    use std::time::SystemTime;
    use tempfile::TempDir;

    use crate::daemon::ability::builtins::resources::pages::sandbox::open_directory;
    use crate::daemon::ability::builtins::resources::pages::state::{
        PageVisibility, ProjectHandle, DEFAULT_FILE_SIZE_CAP, PUBLISHED_PROJECTS,
    };
    use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};

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

    #[tokio::test]
    async fn health_response_reports_current_process_pid() {
        let resp = pages_health_response(false);
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024).await.expect("body bytes");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(payload.get("status").and_then(|v| v.as_str()), Some("ok"));
        assert_eq!(
            payload.get("pid").and_then(|v| v.as_u64()),
            Some(std::process::id() as u64)
        );
    }

    fn publish_temp_project(user: &str, project_id: &str, files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        for (rel_path, content) in files {
            let full = dir.path().join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("parent dir");
            }
            std::fs::write(full, content).expect("write file");
        }
        let canonical_root = std::fs::canonicalize(dir.path()).expect("canonical root");
        let folder_handle = open_directory(&canonical_root).expect("open directory");
        PUBLISHED_PROJECTS.insert(
            (user.to_string(), project_id.to_string()),
            Arc::new(ProjectHandle {
                user: user.to_string(),
                project_id: project_id.to_string(),
                folder_handle,
                canonical_root,
                visibility: PageVisibility::Public,
                file_size_cap: DEFAULT_FILE_SIZE_CAP,
                started_at: SystemTime::now(),
            }),
        );
        dir
    }

    fn openai_http_runtime(
    ) -> crate::daemon::ability::builtins::integrations::openai_compat::OpenAICompatRuntime {
        let codex_agent_ura = crate::core::ura::agent_ura("easynet.run", "alice", "codex");
        let authority_context =
            crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
                "easynet:///r/easynet.run/device/pages-test",
                vec![codex_agent_ura],
            )
            .expect("pages test hosted Agent authority is canonical");
        let mut reg = AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority_context,
        );
        reg.register_rpc_with_owner_and_action(
            "codex.chat",
            OwnerKind::Agent("codex".into()),
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            Arc::new(|_args| Ok(json!({"reply":"ok"}))),
        );
        let reg = Arc::new(reg);
        let handle = Arc::new(OnceLock::new());
        handle
            .set(reg)
            .expect("dispatch handle OnceLock should set once");
        crate::daemon::ability::builtins::integrations::openai_compat::OpenAICompatRuntime::from_pages_identity(
            handle,
            crate::daemon::ability::builtins::resources::pages::PagesIdentity {
                user: Some("alice".into()),
                realm: Some("easynet.run".into()),
                listener_port: Some(8787),
            },
        )
    }

    fn attach_openai_runtime(req: &mut Request<Body>) {
        req.extensions_mut().insert(openai_http_runtime());
    }

    #[tokio::test]
    async fn api_route_serves_static_json_manifest() {
        let user = "alice";
        let project_id = "recipes";
        let key = (user.to_string(), project_id.to_string());
        let _dir = publish_temp_project(
            user,
            project_id,
            &[(
                "api/hello.toml",
                "kind = \"static_json\"\nresponse = { ok = true, source = \"pages\" }\n",
            )],
        );

        let req = Request::builder()
            .method("POST")
            .uri("/api/hello")
            .header(
                header::HOST,
                format!("{project_id}.{user}.pages.localhost:8787"),
            )
            .body(Body::from("{\"ignored\":true}"))
            .expect("request");
        let resp = handle(req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024).await.expect("body bytes");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["source"], "pages");

        PUBLISHED_PROJECTS.remove(&key);
    }

    #[tokio::test]
    async fn v1_models_lists_chat_ability_models() {
        let mut req = Request::builder()
            .method("GET")
            .uri("/v1/models")
            .body(Body::empty())
            .expect("request");
        attach_openai_runtime(&mut req);
        let resp = handle(req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 4096).await.expect("body bytes");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        let models = payload["data"].as_array().expect("data array");
        assert!(models.iter().any(|entry| {
            entry.get("id").and_then(|v| v.as_str())
                == Some("easynet:///r/easynet.run/ability/alice.codex.chat")
        }));
    }

    #[tokio::test]
    async fn v1_chat_completions_routes_to_chat_ability() {
        let mut req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "model": "easynet:///r/easynet.run/ability/alice.codex.chat",
                    "messages": [
                        {"role": "user", "content": "reply with: ok"}
                    ]
                })
                .to_string(),
            ))
            .expect("request");
        attach_openai_runtime(&mut req);
        let resp = handle(req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 4096).await.expect("body bytes");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(
            payload["model"],
            "easynet:///r/easynet.run/ability/alice.codex.chat"
        );
        assert_eq!(payload["choices"][0]["message"]["content"], "ok");
    }

    #[tokio::test]
    async fn v1_chat_completions_stream_returns_sse_body() {
        let mut req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({
                    "model": "easynet:///r/easynet.run/ability/alice.codex.chat",
                    "stream": true,
                    "messages": [
                        {"role": "user", "content": "reply with: ok"}
                    ]
                })
                .to_string(),
            ))
            .expect("request");
        attach_openai_runtime(&mut req);
        let resp = handle(req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );
        let body = to_bytes(resp.into_body(), 4096).await.expect("body bytes");
        let text = String::from_utf8(body.to_vec()).expect("utf8 body");
        assert!(text.starts_with(": easynet-openai-stream\n\n"));
        assert!(text.contains("\"object\":\"chat.completion.chunk\""));
        assert!(text.contains("data: [DONE]\n\n"));
    }
}
