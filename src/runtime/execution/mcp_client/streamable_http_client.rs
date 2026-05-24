// EasyNet CLI — Streamable HTTP transport for outbound MCP
// =========================================================
//
// File: src/runtime/execution/mcp_client/streamable_http_client.rs
//
// Per MCP spec 2025-06-18 §"Streamable HTTP" transport. Sibling
// of the stdio transport already in this module.
//
// What this module implements (v1 — round-1 of the plan)
// ------------------------------------------------------
//
//   * POST JSON-RPC requests to the MCP endpoint.
//   * Initialize handshake — captures the optional `Mcp-Session-Id`
//     response header per §"Session Management" and threads it
//     into every subsequent request.
//   * Threads the `MCP-Protocol-Version` header on subsequent
//     requests per §"Protocol Version Header".
//   * Accepts `Content-Type: application/json` responses (the
//     simple unary path).
//
// What this module does NOT implement (deferred to round-2)
// ---------------------------------------------------------
//
//   * `Content-Type: text/event-stream` (SSE) responses. If a
//     server returns SSE, this client surfaces an error directing
//     the operator at the deferral. SSE handling is required for
//     server→client notifications, mid-call progress, and
//     resumable streams; it lands together with the bridge's
//     stream-projection work (plan §B).
//   * Implicit GET / SSE listener channel.
//   * `Last-Event-ID` resumption.
//   * HTTPS / TLS. v1 assumes localhost or trusted-network HTTP.
//     `mcp-bench` itself runs every HTTP upstream on localhost.
//   * Authentication (the spec requires it for production; out of
//     scope for the dev-loop until an operator-facing config knob
//     exists).
//
// Design notes
// ------------
//
// We use raw `hyper` (no `reqwest`, no `hyper-util` outside the
// existing axon-pb feature gate) so this module pulls no new
// crate-graph weight beyond `hyper`'s `client` feature. The
// connection is established per-request — MCP's POST-per-message
// shape means we don't keep an HTTP/1.1 keep-alive pool open
// inside this client. If profiling later shows this is a hot
// path, a pool fits inside `HttpConnection`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::header::{ACCEPT, CONTENT_TYPE};
use hyper::{Request, Uri};
use serde_json::{json, Value};
use tokio::net::TcpStream;

use super::{McpServerSpec, NotificationSink, ObservedNotification};

/// Local "discard everything" sink for the plain `rpc()` path that
/// does not surface notifications. Mirrors the stdio transport's
/// `DiscardSink` so call sites that don't care about progress keep
/// their pre-SSE-aware signature.
struct DiscardSink;
impl NotificationSink for DiscardSink {
    fn observe(&mut self, _note: ObservedNotification) {}
}

/// MCP protocol version this client advertises in the initialize
/// handshake and stamps into the `MCP-Protocol-Version` header on
/// subsequent requests. Matches the stdio transport's claim above
/// to keep one client identity across transports.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Header per MCP spec 2025-06-18 §"Session Management".
const HEADER_SESSION_ID: &str = "Mcp-Session-Id";

/// Header per MCP spec 2025-06-18 §"Protocol Version Header".
const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";

/// Default timeout for any single HTTP round-trip. A real MCP
/// server should respond in milliseconds; if it takes longer than
/// 30s we want to fail loudly rather than wedge the caller.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// One live HTTP MCP connection — really just the captured session
/// state, since v1 issues a fresh TCP connect per request.
#[derive(Debug)]
pub struct HttpConnection {
    /// Base URL of the MCP server (e.g. `http://127.0.0.1:3001`).
    /// Captured from `spec.url` at initialize time so subsequent
    /// rpc() calls don't need to re-resolve.
    base_url: String,
    /// Path on the MCP server (`/mcp` by default per spec).
    endpoint: String,
    /// Session id returned by the server on initialize, threaded on
    /// every subsequent request per §"Session Management". `None`
    /// when the server chose not to assign a session id, which is
    /// spec-legal for stateless servers.
    session_id: Option<String>,
    /// Monotonically incrementing JSON-RPC request id.
    next_id: u64,
}

impl HttpConnection {
    /// Send the MCP `initialize` handshake. Captures any
    /// `Mcp-Session-Id` the server returns so future requests can
    /// thread it.
    pub async fn initialize(spec: &McpServerSpec) -> anyhow::Result<Self> {
        let base_url = spec
            .url
            .as_deref()
            .ok_or_else(|| anyhow!("HTTP MCP server `{}` missing `url`", spec.name))?
            .trim_end_matches('/')
            .to_string();
        let endpoint = if spec.endpoint.starts_with('/') {
            spec.endpoint.clone()
        } else {
            format!("/{}", spec.endpoint)
        };

        let mut conn = Self {
            base_url,
            endpoint,
            session_id: None,
            next_id: 1,
        };

        // First POST: initialize. No session id yet — the server
        // returns one (or doesn't) on the response.
        let init_id = conn.next_id;
        conn.next_id += 1;
        let request_body = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "easynet-daemon",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }
        });
        let (response_body, captured_session) = conn
            .post_and_extract_session(serde_json::to_vec(&request_body)?)
            .await
            .context("MCP initialize")?;
        conn.session_id = captured_session;

        // Verify the initialize response is well-formed JSON-RPC.
        // We don't assert anything beyond shape — capability
        // negotiation is the caller's problem; this client just
        // wants to know the handshake succeeded.
        let response: Value = serde_json::from_slice(&response_body)
            .context("MCP initialize response was not valid JSON")?;
        if response.get("id") != Some(&json!(init_id)) {
            bail!(
                "MCP initialize response id mismatch: expected {init_id}, got {}",
                response.get("id").cloned().unwrap_or(Value::Null)
            );
        }
        if response.get("error").is_some() {
            bail!(
                "MCP initialize returned JSON-RPC error: {}",
                response.get("error").unwrap()
            );
        }

        // Per spec §"Lifecycle", the client SHOULD send
        // `notifications/initialized` after initialize. We make
        // this a best-effort fire-and-forget — if the server is
        // strict about it being absent, no observable difference;
        // if it's lenient, this matches the stdio transport's
        // behaviour. JSON-RPC notifications have no `id`.
        let _ = conn
            .post_raw_no_session_capture(serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
            }))?)
            .await;

        Ok(conn)
    }

    /// Send a JSON-RPC request. Mirrors `McpConnection::rpc` for
    /// the stdio transport — same return contract (the `result`
    /// field, or an error). Notifications interleaved in an SSE
    /// response are silently dropped; use `rpc_with_sink` to surface
    /// them.
    pub async fn rpc(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.rpc_with_sink(method, params, &mut DiscardSink).await
    }

    /// Same as `rpc`, but routes any `notifications/*` frames the
    /// upstream interleaves through `sink` BEFORE the eventual
    /// response. Mirrors the stdio transport's `rpc_with_sink` so
    /// the `McpClientService::rpc_with_progress` entry point can
    /// thread one sink through both transports uniformly.
    pub async fn rpc_with_sink(
        &mut self,
        method: &str,
        params: Value,
        sink: &mut dyn NotificationSink,
    ) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request_body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        let (response_body, _new_session) = self
            .do_post_with_sink(request_body, true, sink)
            .await
            .with_context(|| format!("MCP HTTP rpc method `{method}`"))?;
        // Per spec, the server MAY rotate the session id mid-
        // session. v1 doesn't honour that (rare, undocumented in
        // mcp-bench's servers); future round can refresh.
        let response: Value = serde_json::from_slice(&response_body)
            .context("MCP HTTP response was not valid JSON")?;
        if let Some(err) = response.get("error") {
            bail!("MCP server returned JSON-RPC error: {err}");
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// POST + read full body + capture (and return) the optional
    /// `Mcp-Session-Id` response header. Accepts both
    /// `Content-Type: application/json` (single unary response) and
    /// `Content-Type: text/event-stream` (SSE stream). Any
    /// intervening JSON-RPC notifications in an SSE stream are
    /// silently dropped on this path — callers that need them must
    /// use `do_post_with_sink` instead.
    async fn post_and_extract_session(
        &self,
        body: Vec<u8>,
    ) -> anyhow::Result<(Vec<u8>, Option<String>)> {
        self.do_post(body, true, &mut DiscardSink).await
    }

    /// POST without expecting any meaningful response capture —
    /// used for fire-and-forget notifications. Per spec
    /// §"Sending Messages" #4, a server accepting a
    /// notification returns 202 Accepted with no body.
    async fn post_raw_no_session_capture(&self, body: Vec<u8>) -> anyhow::Result<()> {
        let _ = self.do_post(body, false, &mut DiscardSink).await?;
        Ok(())
    }

    /// POST and route any SSE-interleaved `notifications/*` frames
    /// through `sink` before returning the final JSON-RPC response
    /// body. The unary `application/json` path returns the body
    /// verbatim — no sink invocation, since the spec does not allow
    /// notifications on that content type.
    async fn do_post_with_sink(
        &self,
        body: Vec<u8>,
        capture_session: bool,
        sink: &mut dyn NotificationSink,
    ) -> anyhow::Result<(Vec<u8>, Option<String>)> {
        self.do_post(body, capture_session, sink).await
    }

    /// Core HTTP one-shot. Establishes a fresh TCP connection,
    /// performs HTTP/1.1 handshake via hyper, sends POST, reads
    /// full body, returns (body_bytes, optional_session_id).
    ///
    /// SSE-streamed responses (`Content-Type: text/event-stream`)
    /// are decoded inline: intervening JSON-RPC notifications are
    /// emitted to `sink` and the terminal response body is what
    /// flows out the (body, session) tuple. Unary
    /// `application/json` responses do not touch the sink — there
    /// are no notifications to route on that content type.
    async fn do_post(
        &self,
        body: Vec<u8>,
        capture_session: bool,
        sink: &mut dyn NotificationSink,
    ) -> anyhow::Result<(Vec<u8>, Option<String>)> {
        let target_uri: Uri = format!("{}{}", self.base_url, self.endpoint)
            .parse()
            .with_context(|| format!("invalid MCP URL: {}{}", self.base_url, self.endpoint))?;

        let host = target_uri
            .host()
            .ok_or_else(|| anyhow!("MCP URL missing host"))?;
        let port = target_uri
            .port_u16()
            .unwrap_or(if target_uri.scheme_str() == Some("https") {
                443
            } else {
                80
            });
        if target_uri.scheme_str() == Some("https") {
            // TLS is round-2 work. mcp-bench's only HTTP upstream
            // (Google Maps) is localhost; production deployments
            // that need HTTPS will surface here.
            bail!(
                "HTTPS MCP servers are not yet supported in v1; \
                 use stdio or an HTTP-on-localhost proxy until \
                 round-2 lands TLS"
            );
        }

        let timeout_fut = async {
            let stream = TcpStream::connect((host, port))
                .await
                .with_context(|| format!("TCP connect to {host}:{port}"))?;
            let io = HyperTokioIo::new(stream);
            let (mut sender, conn_driver) = http1::handshake::<_, Full<Bytes>>(io)
                .await
                .context("hyper HTTP/1.1 handshake")?;
            // Drive the connection in the background until it
            // completes. We don't tokio::spawn because we want
            // the driver scoped to this request's lifetime.
            let driver_handle = tokio::spawn(async move {
                if let Err(e) = conn_driver.await {
                    // Driver errors are typically "remote half closed"
                    // after the response completes — expected. Log
                    // at debug-ish level via eprintln (no tracing
                    // crate in Cli's dep graph for this module).
                    let _ = e;
                }
            });

            let mut req_builder = Request::builder()
                .method("POST")
                .uri(
                    target_uri
                        .path_and_query()
                        .map(|p| p.as_str())
                        .unwrap_or("/"),
                )
                .header("host", format!("{host}:{port}"))
                .header(CONTENT_TYPE, "application/json")
                // Per spec §"Sending Messages" #2: client MUST list
                // both content types as accepted.
                .header(ACCEPT, "application/json, text/event-stream")
                .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION);
            if let Some(sid) = &self.session_id {
                req_builder = req_builder.header(HEADER_SESSION_ID, sid);
            }
            let req = req_builder
                .body(Full::new(Bytes::from(body)))
                .context("build hyper request")?;

            let resp = sender
                .send_request(req)
                .await
                .context("HTTP send_request")?;
            let status = resp.status();
            let session = if capture_session {
                resp.headers()
                    .get(HEADER_SESSION_ID)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string())
            } else {
                None
            };
            let content_type = resp
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let body_bytes = resp
                .into_body()
                .collect()
                .await
                .context("read response body")?
                .to_bytes()
                .to_vec();

            // Tear down the driver gracefully.
            driver_handle.abort();

            // Spec §"Sending Messages" #4: notifications/responses
            // posted to the server receive 202 with no body. Our
            // notifications path (post_raw_no_session_capture)
            // accepts that.
            if !capture_session && status.as_u16() == 202 {
                return Ok::<_, anyhow::Error>((body_bytes, session));
            }

            if !status.is_success() {
                bail!(
                    "MCP HTTP server returned non-success status {} (body: {})",
                    status,
                    String::from_utf8_lossy(&body_bytes)
                );
            }
            if content_type.starts_with("text/event-stream") {
                // Per MCP spec 2025-06-18 §"Sending Messages" #5-6:
                // when the server returns SSE, the stream contains a
                // terminal JSON-RPC response matching the request id.
                // The stream MAY also interleave JSON-RPC
                // notifications (`notifications/progress`,
                // `notifications/tools/list_changed`,
                // `notifications/message`, …) BEFORE the response —
                // those are routed through `sink` here so callers
                // that asked for progress get the same shape they
                // would over stdio.
                let parsed = parse_sse_body(&body_bytes).with_context(|| {
                    "MCP HTTP SSE response did not contain a matching JSON-RPC response"
                })?;
                for note in parsed.notifications {
                    sink.observe(note);
                }
                return Ok::<_, anyhow::Error>((parsed.response, session));
            }
            Ok((body_bytes, session))
        };

        tokio::time::timeout(REQUEST_TIMEOUT, timeout_fut)
            .await
            .map_err(|_| anyhow!("MCP HTTP request timed out after {REQUEST_TIMEOUT:?}"))?
    }
}

/// Parsed SSE body, split into the spec-relevant pieces an HTTP
/// MCP caller needs: the JSON-RPC response (mandatory) and any
/// intervening JSON-RPC notifications (optional, may be empty).
///
/// Why both as `Vec<u8>` / `Value` mixed:
///   - The response is returned to the calling `rpc()` as raw bytes
///     so the existing JSON-parse path downstream stays unchanged.
///   - Notifications are pre-parsed into `ObservedNotification` so
///     the sink-routing code does not re-parse on the hot path.
#[derive(Debug)]
struct SseParseResult {
    /// JSON-RPC response body (bytes). The LAST `data:` event whose
    /// payload looks like a JSON-RPC response (`id` + `result`/`error`).
    /// MCP spec REQUIRES a stream to terminate with such a frame; if
    /// none was seen the parse fails.
    response: Vec<u8>,
    /// Every JSON-RPC notification (`{jsonrpc, method, params}` with
    /// no `id`) seen in the stream, in arrival order. Empty when the
    /// upstream did not emit progress.
    notifications: Vec<ObservedNotification>,
}

/// Parse an SSE-encoded MCP response body. Splits intervening
/// `notifications/*` frames from the final JSON-RPC response.
///
/// SSE wire format per [HTML Living Standard §Server-sent events]:
///   * Each event is a block of `field: value\n` lines.
///   * Events are separated by a blank line (`\n\n`).
///   * `data:` lines within one event are joined by `\n`.
///   * Comment lines start with `:` and are ignored.
///   * `id:`, `event:`, `retry:` fields exist but MCP does not use
///     them for response framing; we accept and ignore them so a
///     spec-compliant server that emits them does not break parsing.
///
/// Notification routing: every JSON object with a `"method"` field
/// and no `"id"` is captured as an `ObservedNotification`. Per MCP
/// 2025-06-18 §"Streamable HTTP" the server MAY interleave
/// `notifications/progress`, `notifications/tools/list_changed`,
/// `notifications/message`, etc. before the final response frame —
/// they are now first-class output of this parser rather than being
/// silently dropped.
fn parse_sse_body(body: &[u8]) -> anyhow::Result<SseParseResult> {
    let text = std::str::from_utf8(body).context("SSE body is not valid UTF-8")?;
    let mut last_response: Option<Vec<u8>> = None;
    let mut notifications: Vec<ObservedNotification> = Vec::new();

    // Split on blank-line separators (LF/CRLF agnostic). Normalise
    // CRLF → LF first to keep the splitter simple. The SSE spec says
    // a blank line is a line containing only the line terminator.
    let normalised = text.replace("\r\n", "\n");
    for block in normalised.split("\n\n") {
        let mut data_chunks: Vec<&str> = Vec::new();
        for line in block.lines() {
            // Comments — `:` followed by anything (including nothing,
            // which is a heartbeat). Per spec, ignore.
            if line.starts_with(':') {
                continue;
            }
            // SSE field syntax: `field`, `field: value`, or
            // `field:value`. We only consume `data` here — `id:`,
            // `event:`, `retry:` are spec-legal but MCP-irrelevant.
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line, ""),
            };
            if field == "data" {
                data_chunks.push(value);
            }
        }
        if data_chunks.is_empty() {
            continue;
        }
        let payload = data_chunks.join("\n");
        // Per SSE, non-JSON `data:` lines are spec-legal but MCP
        // never produces them — skip silently.
        let parsed: Value = match serde_json::from_str(&payload) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // JSON-RPC response: has `id` AND (`result` or `error`).
        if parsed.get("id").is_some()
            && (parsed.get("result").is_some() || parsed.get("error").is_some())
        {
            last_response = Some(payload.into_bytes());
            continue;
        }

        // JSON-RPC notification: has `method` AND no `id`.
        // The `params` field is optional in JSON-RPC; when absent we
        // surface a JSON `null` so the sink contract is uniform.
        if parsed.get("id").is_none() {
            if let Some(method) = parsed.get("method").and_then(Value::as_str) {
                notifications.push(ObservedNotification {
                    method: method.to_string(),
                    params: parsed.get("params").cloned().unwrap_or(Value::Null),
                });
            }
        }
    }

    let response = last_response.ok_or_else(|| {
        anyhow!(
            "SSE body had no JSON-RPC response frame (body len = {}, notifications observed = {})",
            body.len(),
            notifications.len()
        )
    })?;
    Ok(SseParseResult {
        response,
        notifications,
    })
}

/// Minimal hyper IO adapter for tokio TcpStream — hyper 1.x
/// expects its own IO trait; tokio's AsyncRead/AsyncWrite needs a
/// thin shim. This is what `hyper-util` provides; we inline a
/// minimal version to avoid pulling hyper-util into the non-
/// axon-pb build path.
struct HyperTokioIo<T> {
    inner: T,
}

impl<T> HyperTokioIo<T> {
    fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: tokio::io::AsyncRead + Unpin> hyper::rt::Read for HyperTokioIo<T> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        // SAFETY: the unfilled portion of the cursor is mut-borrowed
        // and we only write into it via tokio's ReadBuf, which only
        // writes initialised bytes — we then call advance() with
        // exactly the count tokio reports as filled. Standard
        // pattern from hyper-util::rt::TokioIo.
        let n = unsafe {
            let mut tbuf = tokio::io::ReadBuf::uninit(buf.as_mut());
            match std::pin::Pin::new(&mut self.inner).poll_read(cx, &mut tbuf) {
                std::task::Poll::Ready(Ok(())) => tbuf.filled().len(),
                other => return other,
            }
        };
        unsafe {
            buf.advance(n);
        }
        std::task::Poll::Ready(Ok(()))
    }
}

impl<T: tokio::io::AsyncWrite + Unpin> hyper::rt::Write for HyperTokioIo<T> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::Full as RespFull;
    use hyper::body::Incoming;
    use hyper::server::conn::http1::Builder as ServerBuilder;
    use hyper::service::service_fn;
    use hyper::{Response, StatusCode};
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    /// Spin up a minimal MCP-style HTTP server in-process,
    /// returning its base URL. Used by every HTTP transport test
    /// so we don't need a real third-party MCP server.
    async fn spawn_minimal_http_mcp(
        handler: impl Fn(Value) -> (StatusCode, Option<String>, Value) + Send + Sync + 'static,
    ) -> (String, Arc<Mutex<Vec<Value>>>) {
        let captured_requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured_for_handler = captured_requests.clone();
        let handler = Arc::new(handler);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let handler = handler.clone();
                let captured = captured_for_handler.clone();
                tokio::spawn(async move {
                    let io = HyperTokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let handler = handler.clone();
                        let captured = captured.clone();
                        async move {
                            let body = req
                                .into_body()
                                .collect()
                                .await
                                .map(|c| c.to_bytes())
                                .unwrap_or_default();
                            let parsed: Value =
                                serde_json::from_slice(&body).unwrap_or(Value::Null);
                            captured.lock().await.push(parsed.clone());
                            let (status, session, result_value) = handler(parsed);
                            let mut builder = Response::builder()
                                .status(status)
                                .header("content-type", "application/json");
                            if let Some(s) = session {
                                builder = builder.header(HEADER_SESSION_ID, s);
                            }
                            let body_bytes = serde_json::to_vec(&result_value).unwrap_or_default();
                            Ok::<_, Infallible>(
                                builder
                                    .body(RespFull::new(Bytes::from(body_bytes)))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = ServerBuilder::new().serve_connection(io, svc).await;
                });
            }
        });

        (url, captured_requests)
    }

    fn http_spec(url: &str) -> McpServerSpec {
        McpServerSpec {
            name: "test-http".into(),
            transport: "http".into(),
            url: Some(url.to_string()),
            endpoint: "/mcp".into(),
            ..Default::default()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initialize_handshake_round_trips_and_captures_session() {
        let (url, captured) = spawn_minimal_http_mcp(|req| {
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            let method = req.get("method").and_then(Value::as_str).unwrap_or("");
            if method == "initialize" {
                (
                    StatusCode::OK,
                    Some("sess-abc-123".into()),
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": PROTOCOL_VERSION,
                            "capabilities": {},
                            "serverInfo": {"name": "test", "version": "0"}
                        }
                    }),
                )
            } else {
                // notifications/initialized — 202 with no body.
                (StatusCode::ACCEPTED, None, Value::Null)
            }
        })
        .await;

        let spec = http_spec(&url);
        let conn = HttpConnection::initialize(&spec).await.expect("init OK");
        assert_eq!(conn.session_id.as_deref(), Some("sess-abc-123"));

        // Two requests captured: initialize + notifications/initialized.
        let snap = captured.lock().await;
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0]["method"], "initialize");
        assert_eq!(snap[1]["method"], "notifications/initialized");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rpc_threads_session_id_header_on_subsequent_calls() {
        // The server checks for the Mcp-Session-Id header on
        // tools/list and refuses if absent — proves the client
        // is actually threading the captured session id.
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let session_seen_on_tools_list = Arc::new(AtomicBool::new(false));
        let session_flag = session_seen_on_tools_list.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let flag = session_flag.clone();
                tokio::spawn(async move {
                    let io = HyperTokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| {
                        let flag = flag.clone();
                        async move {
                            let session_header = req
                                .headers()
                                .get(HEADER_SESSION_ID)
                                .and_then(|v| v.to_str().ok())
                                .map(String::from);
                            let body = req
                                .into_body()
                                .collect()
                                .await
                                .map(|c| c.to_bytes())
                                .unwrap_or_default();
                            let parsed: Value =
                                serde_json::from_slice(&body).unwrap_or(Value::Null);
                            let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
                            let id = parsed.get("id").cloned().unwrap_or(Value::Null);
                            let (status, body_val) = match method {
                                "initialize" => (
                                    StatusCode::OK,
                                    json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": {
                                            "protocolVersion": PROTOCOL_VERSION,
                                            "capabilities": {},
                                            "serverInfo": {"name": "test", "version": "0"}
                                        }
                                    }),
                                ),
                                "notifications/initialized" => (StatusCode::ACCEPTED, Value::Null),
                                "tools/list" => {
                                    if session_header.as_deref() == Some("sess-xyz") {
                                        flag.store(true, Ordering::SeqCst);
                                        (
                                            StatusCode::OK,
                                            json!({
                                                "jsonrpc": "2.0",
                                                "id": id,
                                                "result": {"tools": []}
                                            }),
                                        )
                                    } else {
                                        (
                                            StatusCode::BAD_REQUEST,
                                            json!({"error": "missing session"}),
                                        )
                                    }
                                }
                                _ => (StatusCode::OK, json!({"jsonrpc":"2.0","id":id,"result":{}})),
                            };
                            let mut builder = Response::builder()
                                .status(status)
                                .header("content-type", "application/json");
                            if method == "initialize" {
                                builder = builder.header(HEADER_SESSION_ID, "sess-xyz");
                            }
                            let body_bytes = serde_json::to_vec(&body_val).unwrap_or_default();
                            Ok::<_, Infallible>(
                                builder
                                    .body(RespFull::new(Bytes::from(body_bytes)))
                                    .unwrap(),
                            )
                        }
                    });
                    let _ = ServerBuilder::new().serve_connection(io, svc).await;
                });
            }
        });

        let spec = http_spec(&url);
        let mut conn = HttpConnection::initialize(&spec).await.expect("init OK");
        assert_eq!(conn.session_id.as_deref(), Some("sess-xyz"));
        let tools = conn.rpc("tools/list", json!({})).await.expect("tools/list");
        assert!(tools.get("tools").is_some());
        assert!(
            session_seen_on_tools_list.load(Ordering::SeqCst),
            "server must have seen Mcp-Session-Id header on tools/list"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_returning_sse_without_response_frame_errs_explicit() {
        // Post-B5 behaviour: SSE is parsed; if the body has NO
        // JSON-RPC response frame at all (only non-MCP data), we
        // fail with a message naming the actual issue rather than
        // pretending the response was OK.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                tokio::spawn(async move {
                    let io = HyperTokioIo::new(stream);
                    let svc = service_fn(move |_req: Request<Incoming>| async move {
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "text/event-stream")
                                .body(RespFull::new(Bytes::from("data: hi\n\n")))
                                .unwrap(),
                        )
                    });
                    let _ = ServerBuilder::new().serve_connection(io, svc).await;
                });
            }
        });

        let spec = http_spec(&url);
        let err = HttpConnection::initialize(&spec).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no JSON-RPC response frame"),
            "error must name the actual SSE-no-response issue, got: {msg}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_returning_sse_with_response_frame_round_trips() {
        // B5 success path: server returns SSE with a JSON-RPC
        // response data event. Client extracts it as the response.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                tokio::spawn(async move {
                    let io = HyperTokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| async move {
                        let body = req
                            .into_body()
                            .collect()
                            .await
                            .map(|c| c.to_bytes())
                            .unwrap_or_default();
                        let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
                        // SSE body: one notification (skipped) +
                        // one response (parsed).
                        let sse = format!(
                            "data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{\"progress\":0.5}}}}\n\n\
                             data: {{\"jsonrpc\":\"2.0\",\"id\":{id_str},\"result\":{{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{{}},\"serverInfo\":{{\"name\":\"sse-srv\",\"version\":\"0\"}}}}}}\n\n",
                            id_str = serde_json::to_string(&id).unwrap()
                        );
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "text/event-stream")
                                .header(HEADER_SESSION_ID, "sse-sess")
                                .body(RespFull::new(Bytes::from(sse)))
                                .unwrap(),
                        )
                    });
                    let _ = ServerBuilder::new().serve_connection(io, svc).await;
                });
            }
        });

        let spec = http_spec(&url);
        let conn = HttpConnection::initialize(&spec)
            .await
            .expect("init OK over SSE");
        assert_eq!(conn.session_id.as_deref(), Some("sse-sess"));
    }

    /// Captures observed notifications so tests can assert the sink
    /// receives them in arrival order. Mirrors the stdio transport's
    /// test sink shape so both transports look the same in test
    /// fixtures.
    struct CollectSink {
        seen: std::sync::Arc<std::sync::Mutex<Vec<ObservedNotification>>>,
    }
    impl NotificationSink for CollectSink {
        fn observe(&mut self, note: ObservedNotification) {
            self.seen.lock().unwrap().push(note);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rpc_with_sink_routes_sse_progress_notifications() {
        // End-to-end SSE progress contract: server emits two
        // `notifications/progress` frames followed by the terminal
        // response. `rpc_with_sink` must (a) return the terminal
        // result verbatim, and (b) deliver every progress frame to
        // the sink in order.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                tokio::spawn(async move {
                    let io = HyperTokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| async move {
                        let body = req
                            .into_body()
                            .collect()
                            .await
                            .map(|c| c.to_bytes())
                            .unwrap_or_default();
                        let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
                        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
                        match method {
                            "initialize" => Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .header("content-type", "application/json")
                                    .header(HEADER_SESSION_ID, "progress-sess")
                                    .body(RespFull::new(Bytes::from(
                                        serde_json::to_vec(&json!({
                                            "jsonrpc": "2.0",
                                            "id": id,
                                            "result": {
                                                "protocolVersion": PROTOCOL_VERSION,
                                                "capabilities": {},
                                                "serverInfo": {"name":"t","version":"0"}
                                            }
                                        }))
                                        .unwrap(),
                                    )))
                                    .unwrap(),
                            ),
                            "notifications/initialized" => Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::ACCEPTED)
                                    .body(RespFull::new(Bytes::new()))
                                    .unwrap(),
                            ),
                            "tools/call" => {
                                // SSE response: two progress notes
                                // then terminal result. Mirrors the
                                // shape mcp-bench's tool servers use
                                // when emitting progress.
                                let id_str = serde_json::to_string(&id).unwrap();
                                let sse = format!(
                                    "data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{\"progressToken\":\"abc\",\"progress\":0.3,\"message\":\"warming\"}}}}\n\n\
                                     data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{\"progressToken\":\"abc\",\"progress\":0.8,\"message\":\"finishing\"}}}}\n\n\
                                     data: {{\"jsonrpc\":\"2.0\",\"id\":{id_str},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"done\"}}],\"isError\":false}}}}\n\n"
                                );
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .status(StatusCode::OK)
                                        .header("content-type", "text/event-stream")
                                        .body(RespFull::new(Bytes::from(sse)))
                                        .unwrap(),
                                )
                            }
                            _ => Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::NOT_FOUND)
                                    .body(RespFull::new(Bytes::new()))
                                    .unwrap(),
                            ),
                        }
                    });
                    let _ = ServerBuilder::new().serve_connection(io, svc).await;
                });
            }
        });

        let spec = http_spec(&url);
        let mut conn = HttpConnection::initialize(&spec).await.expect("init OK");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut sink = CollectSink { seen: seen.clone() };
        let result = conn
            .rpc_with_sink(
                "tools/call",
                json!({"name": "any", "arguments": {}}),
                &mut sink,
            )
            .await
            .expect("tools/call must succeed");
        // Terminal response is the MCP tools/call shape.
        assert_eq!(result["content"][0]["text"], "done");
        assert_eq!(result["isError"], false);
        // Both progress notifications surfaced to the sink, in order.
        let notes = seen.lock().unwrap().clone();
        assert_eq!(notes.len(), 2, "expected 2 progress notes, got: {notes:?}");
        assert_eq!(notes[0].method, "notifications/progress");
        assert_eq!(notes[0].params["progress"], 0.3);
        assert_eq!(notes[0].params["message"], "warming");
        assert_eq!(notes[1].params["progress"], 0.8);
        assert_eq!(notes[1].params["message"], "finishing");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rpc_without_sink_still_works_when_server_emits_progress() {
        // Backwards-compatible path: callers using the plain `rpc()`
        // must keep working even if the server interleaves
        // notifications. The notifications are silently discarded
        // (DiscardSink), the terminal response is returned verbatim.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                tokio::spawn(async move {
                    let io = HyperTokioIo::new(stream);
                    let svc = service_fn(move |req: Request<Incoming>| async move {
                        let body = req
                            .into_body()
                            .collect()
                            .await
                            .map(|c| c.to_bytes())
                            .unwrap_or_default();
                        let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                        let id = parsed.get("id").cloned().unwrap_or(Value::Null);
                        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
                        if method == "initialize" {
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .header("content-type", "application/json")
                                    .body(RespFull::new(Bytes::from(
                                        serde_json::to_vec(&json!({
                                            "jsonrpc": "2.0",
                                            "id": id,
                                            "result": {
                                                "protocolVersion": PROTOCOL_VERSION,
                                                "capabilities": {},
                                                "serverInfo": {"name":"t","version":"0"}
                                            }
                                        }))
                                        .unwrap(),
                                    )))
                                    .unwrap(),
                            );
                        }
                        if method == "notifications/initialized" {
                            return Ok::<_, Infallible>(
                                Response::builder()
                                    .status(StatusCode::ACCEPTED)
                                    .body(RespFull::new(Bytes::new()))
                                    .unwrap(),
                            );
                        }
                        let id_str = serde_json::to_string(&id).unwrap();
                        let sse = format!(
                            "data: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{{\"progress\":0.5}}}}\n\n\
                             data: {{\"jsonrpc\":\"2.0\",\"id\":{id_str},\"result\":{{\"tools\":[]}}}}\n\n"
                        );
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "text/event-stream")
                                .body(RespFull::new(Bytes::from(sse)))
                                .unwrap(),
                        )
                    });
                    let _ = ServerBuilder::new().serve_connection(io, svc).await;
                });
            }
        });

        let spec = http_spec(&url);
        let mut conn = HttpConnection::initialize(&spec).await.expect("init OK");
        // Plain rpc — no sink wired. The notification is consumed by
        // DiscardSink internally; the response still comes through.
        let result = conn
            .rpc("tools/list", json!({}))
            .await
            .expect("plain rpc still works on SSE response with notifications");
        assert!(result.get("tools").is_some());
    }

    #[test]
    fn parse_sse_body_picks_last_response_and_captures_notifications() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.1}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        let parsed = parse_sse_body(body).expect("must find response");
        let v: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["result"]["ok"], true);
        // The intervening progress notification is now first-class
        // output, not silently dropped.
        assert_eq!(parsed.notifications.len(), 1);
        assert_eq!(parsed.notifications[0].method, "notifications/progress");
        assert_eq!(parsed.notifications[0].params["progress"], 0.1);
    }

    #[test]
    fn parse_sse_body_captures_every_notification_in_order() {
        // Multiple progress frames followed by the terminal response.
        // The sink routing contract is "in arrival order"; pin it.
        let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.25,\"message\":\"step1\"}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.75,\"message\":\"step2\"}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{\"level\":\"info\"}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"done\":true}}\n\n";
        let parsed = parse_sse_body(body).expect("must find response");
        assert_eq!(parsed.notifications.len(), 3);
        assert_eq!(parsed.notifications[0].params["progress"], 0.25);
        assert_eq!(parsed.notifications[0].params["message"], "step1");
        assert_eq!(parsed.notifications[1].params["progress"], 0.75);
        assert_eq!(parsed.notifications[2].method, "notifications/message");
        let v: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(v["result"]["done"], true);
    }

    #[test]
    fn parse_sse_body_handles_multiline_data_events() {
        // SSE allows multiple data: lines per event, joined by \n.
        let body = b"data: {\"jsonrpc\":\"2.0\",\n\
                     data: \"id\":1,\n\
                     data: \"result\":{}}\n\n";
        let parsed = parse_sse_body(body).expect("multiline data must parse");
        let v: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(v["id"], 1);
        assert!(parsed.notifications.is_empty());
    }

    #[test]
    fn parse_sse_body_handles_crlf_line_endings() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\"id\":5,\"result\":{}}\r\n\r\n";
        let parsed = parse_sse_body(body).expect("CRLF must work");
        let v: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(v["id"], 5);
    }

    #[test]
    fn parse_sse_body_ignores_comments_and_non_data_fields() {
        let body = b": this is a comment\n\
                     event: ignored\n\
                     id: 42\n\
                     retry: 3000\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":\"x\"}\n\n";
        let parsed = parse_sse_body(body).expect("must find response");
        let v: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(v["id"], 3);
        assert_eq!(v["result"], "x");
        // `event:`, `id:`, `retry:`, comments — none surface as
        // notifications. Pin that contract explicitly so a future
        // change that starts mis-classifying them fails loud.
        assert!(parsed.notifications.is_empty());
    }

    #[test]
    fn parse_sse_body_errors_when_only_notifications_seen() {
        // The MCP spec requires the stream to terminate with a
        // response frame matching the request id. A stream of pure
        // notifications is therefore malformed — the caller's
        // `rpc()` MUST fail loud, not return a phantom `null`.
        let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.5}}\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}\n\n";
        let err = parse_sse_body(body).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("no JSON-RPC response frame"),
            "error must be explicit; got: {msg}"
        );
        assert!(
            msg.contains("notifications observed = 2"),
            "error must surface notification count for debugging; got: {msg}"
        );
    }

    #[test]
    fn parse_sse_body_skips_non_json_data_events() {
        // Spec allows non-JSON `data:` events; MCP never sends them
        // but we must not crash on receipt.
        let body = b"data: hello world\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":11,\"result\":42}\n\n";
        let parsed = parse_sse_body(body).expect("non-JSON skipped, response found");
        let v: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(v["result"], 42);
    }

    #[test]
    fn parse_sse_body_ignores_data_event_with_neither_id_nor_method() {
        // A `data:` payload that's valid JSON but is neither a
        // request, response, nor notification (e.g. an array or a
        // bare object) must not surface as either output kind.
        let body = b"data: [1, 2, 3]\n\n\
                     data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"ok\"}\n\n";
        let parsed = parse_sse_body(body).expect("response found");
        assert!(parsed.notifications.is_empty());
        let v: Value = serde_json::from_slice(&parsed.response).unwrap();
        assert_eq!(v["result"], "ok");
    }

    #[tokio::test]
    async fn missing_url_fails_at_initialize() {
        let spec = McpServerSpec {
            name: "broken".into(),
            transport: "http".into(),
            url: None,
            ..Default::default()
        };
        let err = HttpConnection::initialize(&spec).await.unwrap_err();
        assert!(format!("{err}").contains("missing `url`"));
    }

    #[tokio::test]
    async fn https_url_rejects_with_clear_message() {
        let spec = McpServerSpec {
            name: "tls".into(),
            transport: "http".into(),
            url: Some("https://example.com".into()),
            ..Default::default()
        };
        let err = HttpConnection::initialize(&spec).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("HTTPS") && msg.contains("round-2"),
            "HTTPS error must steer operator at deferral; got: {msg}"
        );
    }
}
