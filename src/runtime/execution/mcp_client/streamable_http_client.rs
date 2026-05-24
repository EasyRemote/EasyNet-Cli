// EasyNet CLI — Streamable HTTP transport for outbound MCP
// =========================================================
//
// File: src/runtime/execution/mcp_client/streamable_http_client.rs
//
// Per MCP spec 2025-06-18 §"Streamable HTTP" transport. Sibling
// of the stdio transport already in this module.
//
// What this module implements
// ---------------------------
//
//   * POST JSON-RPC requests to the MCP endpoint (round-1).
//   * Initialize handshake — captures the optional `Mcp-Session-Id`
//     response header per §"Session Management" and threads it
//     into every subsequent request (round-1).
//   * Threads the `MCP-Protocol-Version` header on subsequent
//     requests per §"Protocol Version Header" (round-1).
//   * Accepts `Content-Type: application/json` and
//     `Content-Type: text/event-stream` (SSE) responses (round-1).
//     SSE frames carry the terminal JSON-RPC response and any
//     mid-call `notifications/*` interleavings.
//   * **TLS** for `https://` URLs via rustls — Mozilla roots by
//     default; per-server CA bundle override; per-server
//     server-name SNI override; a double-gated
//     insecure_skip_verify (config flag AND env var, for closed
//     test environments only) (round-2).
//   * **Authentication** — Bearer / BearerEnv / arbitrary header
//     map. Applied to every outgoing POST and to the GET listener
//     reconnect path (round-2).
//   * **Implicit GET / SSE listener channel** per spec §"Listening
//     for Messages from the Server" — a long-lived background task
//     opens GET on the same endpoint and routes server-initiated
//     notifications through the same sink contract used by the
//     stdio listener (round-2).
//   * **Last-Event-Id resumption** per spec §"Resumability and
//     Retries". Every SSE frame's `id:` is recorded; on reconnect
//     the listener replays the latest Last-Event-Id so the server
//     resumes the stream from the last acknowledged event. The
//     `retry:` field controls reconnect backoff (round-2).
//
// What this module still does NOT implement
// -----------------------------------------
//
//   * Outbound HTTP/2. We use HTTP/1.1 (`hyper::client::conn::http1`).
//     Streamable HTTP servers MUST support HTTP/1.1 per the spec, so
//     this is acceptable; HTTP/2 is a future perf optimisation.
//   * OAuth refresh flows. `AuthSpec::BearerEnv` covers the
//     "rotate token externally" case; richer OAuth belongs behind
//     a sidecar that converts to one of our AuthSpec variants.
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

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use hyper::{Request, Uri};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use super::{AuthSpec, McpServerSpec, NotificationSink, ObservedNotification, TlsSpec};

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

/// One live HTTP MCP connection — captured session state, an
/// optional cached TLS connector for HTTPS endpoints, and the
/// background GET-listener handle (round-2).
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
    /// Full spec, kept around so round-2 paths (do_post, GET
    /// listener) can read tls/auth fields without the caller
    /// passing them every call.
    spec: Arc<McpServerSpec>,
    /// Lazily built TLS connector for `https://` endpoints. Built
    /// once per HttpConnection so the (relatively expensive) root
    /// certificate parsing happens at initialize time, not on the
    /// hot path. None for `http://`.
    tls_connector: Option<Arc<tokio_rustls::TlsConnector>>,
    /// Latest `id:` field observed on any incoming SSE frame
    /// (POST response or GET listener). Replayed as `Last-Event-Id`
    /// when the GET listener reconnects, per spec §"Resumability
    /// and Retries". Shared with the listener task; the POST path
    /// also updates it whenever a response frame carries an id.
    last_event_id: Arc<RwLock<Option<String>>>,
    /// Handle to the background GET-listener task, if one has been
    /// spawned. Some after `spawn_listener` runs; the Drop impl
    /// aborts it so a closed HttpConnection does not leak a task.
    listener_handle: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for HttpConnection {
    fn drop(&mut self) {
        // Best-effort: the listener task is detached the moment we
        // exit this scope. Mutex::blocking_lock would panic in an
        // async context, so we read the inner Option via
        // try_lock and live with the (impossible-in-practice)
        // contention case — the spawn site only ever takes the
        // lock briefly to swap a Some in.
        if let Ok(mut guard) = self.listener_handle.try_lock() {
            if let Some(h) = guard.take() {
                h.abort();
            }
        }
    }
}

impl std::fmt::Debug for HttpConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpConnection")
            .field("base_url", &self.base_url)
            .field("endpoint", &self.endpoint)
            .field("session_id", &self.session_id)
            .field("next_id", &self.next_id)
            .field("https", &self.tls_connector.is_some())
            .finish()
    }
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

        // Build the TLS connector once at initialize — root
        // certificate parsing (especially with a custom ca_bundle)
        // is the kind of work we don't want repeated on every
        // request. None for plain http://.
        let is_https = base_url.starts_with("https://") || base_url.starts_with("HTTPS://");
        let tls_connector = if is_https {
            Some(Arc::new(build_tls_connector(&spec.tls, &spec.name)?))
        } else {
            None
        };

        let mut conn = Self {
            base_url,
            endpoint,
            session_id: None,
            next_id: 1,
            spec: Arc::new(spec.clone()),
            tls_connector,
            last_event_id: Arc::new(RwLock::new(None)),
            listener_handle: Mutex::new(None),
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

    /// Core HTTP one-shot. Establishes a fresh TCP (or TLS-over-TCP)
    /// connection, performs HTTP/1.1 handshake via hyper, sends POST,
    /// reads full body, returns (body_bytes, optional_session_id).
    ///
    /// SSE-streamed responses (`Content-Type: text/event-stream`)
    /// are decoded inline: intervening JSON-RPC notifications are
    /// emitted to `sink`, the terminal response body is what flows
    /// out the (body, session) tuple, and every frame's optional
    /// `id:` field updates the connection's `last_event_id` so a
    /// subsequent GET listener reconnect can resume the stream.
    /// Unary `application/json` responses do not touch the sink —
    /// there are no notifications to route on that content type.
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
        let host_owned = host.to_string();
        let port = target_uri
            .port_u16()
            .unwrap_or(if self.tls_connector.is_some() { 443 } else { 80 });

        let path = target_uri
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());

        let timeout_fut = async {
            // Connect — TLS-wrap when the spec is https://.
            let tcp = TcpStream::connect((host_owned.as_str(), port))
                .await
                .with_context(|| format!("TCP connect to {host_owned}:{port}"))?;
            let io: Box<dyn AsyncStream> = match &self.tls_connector {
                Some(connector) => {
                    let sni_name = self
                        .spec
                        .tls
                        .server_name
                        .clone()
                        .unwrap_or_else(|| host_owned.clone());
                    let server_name = sni_name.clone().try_into().map_err(|_| {
                        anyhow!(
                            "MCP server `{}`: tls.server_name `{sni_name}` is not a valid DNS name",
                            self.spec.name
                        )
                    })?;
                    let tls = connector
                        .connect(server_name, tcp)
                        .await
                        .with_context(|| format!("TLS handshake with {host_owned}:{port}"))?;
                    Box::new(tls)
                }
                None => Box::new(tcp),
            };

            let (mut sender, conn_driver) =
                http1::handshake::<_, Full<Bytes>>(HyperTokioIo::new(io))
                    .await
                    .context("hyper HTTP/1.1 handshake")?;
            // Drive the connection in the background until it
            // completes. Aborted explicitly after the response
            // is consumed so we don't leak the task on the slow
            // path where the server holds the connection open.
            let driver_handle = tokio::spawn(async move {
                if let Err(e) = conn_driver.await {
                    let _ = e;
                }
            });

            let mut req_builder = Request::builder()
                .method("POST")
                .uri(&path)
                .header("host", format!("{host_owned}:{port}"))
                .header(CONTENT_TYPE, "application/json")
                // Per spec §"Sending Messages" #2: client MUST list
                // both content types as accepted.
                .header(ACCEPT, "application/json, text/event-stream")
                .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION);
            if let Some(sid) = &self.session_id {
                req_builder = req_builder.header(HEADER_SESSION_ID, sid);
            }
            // Round-2: replay Last-Event-Id on every POST too. The
            // spec only requires this on the GET listener
            // reconnect, but threading it through POST as well
            // lets a server pin its stream cursor for sessionful
            // upstreams that key on per-call resumability.
            if let Some(id) = self.last_event_id.read().await.clone() {
                req_builder = req_builder.header("Last-Event-ID", id);
            }
            // Round-2: per-server auth headers.
            req_builder = apply_auth_headers(req_builder, self.spec.auth.as_ref())
                .with_context(|| format!("MCP server `{}`: auth header", self.spec.name))?;

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

            driver_handle.abort();

            // Spec §"Sending Messages" #4: notifications/responses
            // posted to the server receive 202 with no body. Our
            // notifications path (post_raw_no_session_capture)
            // accepts that.
            if !capture_session && status.as_u16() == 202 {
                return Ok::<_, anyhow::Error>((body_bytes, session));
            }

            if status.as_u16() == 401 || status.as_u16() == 403 {
                bail!(
                    "MCP server `{}` rejected the request with HTTP {} — \
                     check `auth` in the server's config (Bearer token / headers).",
                    self.spec.name,
                    status,
                );
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
                if let Some(id) = parsed.last_event_id.clone() {
                    *self.last_event_id.write().await = Some(id);
                }
                return Ok::<_, anyhow::Error>((parsed.response, session));
            }
            Ok((body_bytes, session))
        };

        tokio::time::timeout(REQUEST_TIMEOUT, timeout_fut)
            .await
            .map_err(|_| anyhow!("MCP HTTP request timed out after {REQUEST_TIMEOUT:?}"))?
    }

    /// Spawn the background GET listener (spec §"Listening for
    /// Messages from the Server"). The listener opens a long-lived
    /// GET on the same endpoint with `Accept: text/event-stream`,
    /// streams notifications through `sink`, and reconnects with the
    /// recorded `Last-Event-Id` on transport failure or server
    /// half-close. Re-spawning when one is already running is a
    /// no-op.
    ///
    /// `sink_factory` is called once per (re)connect attempt — the
    /// listener cannot share `&mut dyn NotificationSink` across
    /// async boundaries, so callers hand in a closure that produces
    /// a fresh boxed sink. Typically this clones an Arc to the
    /// same long-lived sink (RegistryRefreshSink) so all
    /// notifications land in one place.
    pub async fn spawn_listener<F>(&self, sink_factory: F) -> anyhow::Result<()>
    where
        F: Fn() -> Box<dyn NotificationSink + Send> + Send + Sync + 'static,
    {
        let mut guard = self.listener_handle.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let base_url = self.base_url.clone();
        let endpoint = self.endpoint.clone();
        let session_id = self.session_id.clone();
        let spec = Arc::clone(&self.spec);
        let tls = self.tls_connector.clone();
        let last_event_id = Arc::clone(&self.last_event_id);
        let handle = tokio::spawn(async move {
            listener_loop(
                base_url,
                endpoint,
                session_id,
                spec,
                tls,
                last_event_id,
                Arc::new(sink_factory),
            )
            .await;
        });
        *guard = Some(handle);
        Ok(())
    }

    /// Test-only accessor for the last observed SSE event id.
    /// Exercised by the resumption round-trip tests.
    #[cfg(test)]
    pub async fn last_event_id_for_test(&self) -> Option<String> {
        self.last_event_id.read().await.clone()
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
    /// Latest `id:` field observed across the parsed stream, if any.
    /// Threaded back into `HttpConnection.last_event_id` so a
    /// subsequent reconnect (POST or GET listener) can replay it
    /// via the `Last-Event-Id` header per spec §"Resumability
    /// and Retries".
    last_event_id: Option<String>,
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
    // Per HTML living spec §"event stream model" the `id:` field is
    // **stream-level**: once an event with an id arrives, the user
    // agent's "last event id" is that value for every subsequent
    // reconnect attempt, even if later events have no id of their
    // own. We mirror that — record the latest id we see, regardless
    // of frame kind. Round-2 `Last-Event-Id` replay reads this.
    let mut last_event_id: Option<String> = None;

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
            // `field:value`. We consume `data` for payloads and
            // `id` for resumption; `event:` and `retry:` are still
            // spec-legal but MCP-irrelevant here. (`retry:` is
            // honoured by the listener loop, not the parser.)
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line, ""),
            };
            if field == "data" {
                data_chunks.push(value);
            } else if field == "id" {
                // Per spec, an empty id resets to "no last event id".
                last_event_id = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
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
        last_event_id,
    })
}

// ─── Round-2: TLS + auth helpers ────────────────────────────────

/// Marker trait so `HyperTokioIo` can wrap either a plain
/// `TcpStream` or a `tokio_rustls::client::TlsStream<TcpStream>`.
/// Both already satisfy `AsyncRead + AsyncWrite + Unpin + Send`,
/// so this is a pure type alias with no behaviour.
trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

/// Build a rustls TLS connector for one MCP server. Mozilla roots
/// from `webpki-roots` are the default trust source; an operator
/// can append a private CA via `TlsSpec.ca_bundle` (PEM file). The
/// double-gated `insecure_skip_verify` path is rejected unless the
/// daemon was started with `EASYNET_ALLOW_INSECURE_TLS=1`, so an
/// attacker who can only write the config file cannot silently
/// downgrade TLS verification.
fn build_tls_connector(
    spec: &TlsSpec,
    server_label: &str,
) -> anyhow::Result<tokio_rustls::TlsConnector> {
    use rustls::pki_types::CertificateDer;
    use rustls::{ClientConfig, RootCertStore};

    let mut roots = RootCertStore::empty();
    // webpki-roots ships Mozilla's CA list as a const slice of
    // `TrustAnchor`s. Calling `extend` here adds every public CA
    // that browsers trust by default.
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if let Some(ca_path) = spec.ca_bundle.as_deref() {
        let raw = std::fs::read(ca_path)
            .with_context(|| format!("read CA bundle `{}`", ca_path.display()))?;
        let mut cursor = std::io::Cursor::new(raw);
        let mut added = 0usize;
        for cert in rustls_pemfile::certs(&mut cursor) {
            let cert: CertificateDer<'_> = cert.with_context(|| {
                format!("parse certificate in CA bundle `{}`", ca_path.display())
            })?;
            roots
                .add(cert)
                .with_context(|| format!("trust CA from `{}`", ca_path.display()))?;
            added += 1;
        }
        if added == 0 {
            anyhow::bail!(
                "MCP server `{server_label}`: tls.ca_bundle `{}` contained no \
                 valid CERTIFICATE PEM blocks",
                ca_path.display()
            );
        }
    }

    let config = if spec.insecure_skip_verify {
        if std::env::var("EASYNET_ALLOW_INSECURE_TLS").ok().as_deref() != Some("1") {
            anyhow::bail!(
                "MCP server `{server_label}`: tls.insecure_skip_verify requested \
                 but daemon was not started with EASYNET_ALLOW_INSECURE_TLS=1. \
                 Refusing to disable certificate verification."
            );
        }
        eprintln!(
            "[easynet warn] MCP server `{server_label}`: TLS verification disabled \
             (tls.insecure_skip_verify=true). DO NOT use this outside closed test \
             environments."
        );
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier))
            .with_no_client_auth()
    } else {
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
    };

    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// **DANGER**: accepts any server certificate, of any name, signed
/// by anyone (including expired or self-signed). Only used when
/// `TlsSpec.insecure_skip_verify` is true AND the daemon was
/// started with `EASYNET_ALLOW_INSECURE_TLS=1`.
#[derive(Debug)]
struct InsecureCertVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        // Accept every scheme rustls supports. We've already
        // promised not to verify anything, so the set just has to
        // cover whatever a server might pick.
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

/// Apply the per-server auth credentials to an outgoing request
/// builder. Called from both the POST path and the GET listener
/// reconnect path so a single config entry covers every direction.
fn apply_auth_headers(
    mut req_builder: hyper::http::request::Builder,
    auth: Option<&AuthSpec>,
) -> anyhow::Result<hyper::http::request::Builder> {
    let Some(auth) = auth else {
        return Ok(req_builder);
    };
    match auth {
        AuthSpec::Bearer { token } => {
            req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        AuthSpec::BearerEnv { env } => {
            let token = std::env::var(env).with_context(|| {
                format!(
                    "AuthSpec::BearerEnv references env var `{env}` which is not set; \
                     the daemon must inherit it or the operator must export it"
                )
            })?;
            req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        AuthSpec::Headers { headers } => {
            for (name, value) in headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
        }
    }
    Ok(req_builder)
}

/// Backoff cap for the GET listener reconnect loop. The SSE `retry:`
/// field, when emitted by a server, overrides this for the next
/// reconnect; otherwise we cap exponential backoff here so a
/// permanently-broken server doesn't burn battery on retry storms.
const LISTENER_RECONNECT_CAP: Duration = Duration::from_secs(30);
/// Initial reconnect delay; doubles on each consecutive failure up
/// to `LISTENER_RECONNECT_CAP`.
const LISTENER_RECONNECT_INITIAL: Duration = Duration::from_millis(500);

/// Long-lived GET listener loop. Opens a `GET` on the MCP endpoint
/// with `Accept: text/event-stream` (per spec §"Listening for
/// Messages from the Server"), streams the response, parses SSE
/// frames as they arrive, routes notifications to the sink, and
/// reconnects on transport failure with the latest `Last-Event-Id`.
async fn listener_loop(
    base_url: String,
    endpoint: String,
    session_id: Option<String>,
    spec: Arc<McpServerSpec>,
    tls: Option<Arc<tokio_rustls::TlsConnector>>,
    last_event_id: Arc<RwLock<Option<String>>>,
    sink_factory: Arc<dyn Fn() -> Box<dyn NotificationSink + Send> + Send + Sync>,
) {
    let mut delay = LISTENER_RECONNECT_INITIAL;
    loop {
        match listener_connect_and_pump(
            &base_url,
            &endpoint,
            session_id.as_deref(),
            &spec,
            tls.as_deref(),
            &last_event_id,
            sink_factory.as_ref(),
        )
        .await
        {
            Ok(server_retry_hint) => {
                // Server closed the stream gracefully. Honour any
                // `retry:` hint observed in this connection, else
                // reset to initial — a clean close is not a
                // failure mode and the server is welcome to start
                // a fresh stream immediately.
                delay = server_retry_hint.unwrap_or(LISTENER_RECONNECT_INITIAL);
            }
            Err(_e) => {
                // Connection error. Exponential backoff capped at
                // LISTENER_RECONNECT_CAP. We don't log the error
                // — a misconfigured upstream would spam stderr;
                // observability hooks are a future thing.
                delay = (delay * 2).min(LISTENER_RECONNECT_CAP);
            }
        }
        tokio::time::sleep(delay).await;
    }
}

/// One iteration of the listener loop: connect, read until close.
/// Returns `Ok(retry_hint)` on a clean close, `Err` on transport
/// failure. The retry hint is the most recent `retry:` field
/// observed on the wire (in ms), if any.
async fn listener_connect_and_pump(
    base_url: &str,
    endpoint: &str,
    session_id: Option<&str>,
    spec: &McpServerSpec,
    tls: Option<&tokio_rustls::TlsConnector>,
    last_event_id: &Arc<RwLock<Option<String>>>,
    sink_factory: &(dyn Fn() -> Box<dyn NotificationSink + Send> + Send + Sync),
) -> anyhow::Result<Option<Duration>> {
    let target_uri: Uri = format!("{base_url}{endpoint}")
        .parse()
        .with_context(|| format!("invalid MCP URL: {base_url}{endpoint}"))?;
    let host = target_uri
        .host()
        .ok_or_else(|| anyhow!("MCP URL missing host"))?
        .to_string();
    let port = target_uri
        .port_u16()
        .unwrap_or(if tls.is_some() { 443 } else { 80 });
    let path = target_uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());

    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .with_context(|| format!("listener TCP connect to {host}:{port}"))?;
    let io: Box<dyn AsyncStream> = match tls {
        Some(connector) => {
            let sni = spec
                .tls
                .server_name
                .clone()
                .unwrap_or_else(|| host.clone());
            let server_name = sni
                .clone()
                .try_into()
                .map_err(|_| anyhow!("invalid TLS server_name `{sni}`"))?;
            let tls_stream = connector
                .connect(server_name, tcp)
                .await
                .context("listener TLS handshake")?;
            Box::new(tls_stream)
        }
        None => Box::new(tcp),
    };
    let (mut sender, conn_driver) = http1::handshake::<_, Full<Bytes>>(HyperTokioIo::new(io))
        .await
        .context("listener HTTP/1.1 handshake")?;
    let driver = tokio::spawn(async move {
        let _ = conn_driver.await;
    });

    let mut req_builder = Request::builder()
        .method("GET")
        .uri(&path)
        .header("host", format!("{host}:{port}"))
        .header(ACCEPT, "text/event-stream")
        .header(HEADER_PROTOCOL_VERSION, PROTOCOL_VERSION);
    if let Some(sid) = session_id {
        req_builder = req_builder.header(HEADER_SESSION_ID, sid);
    }
    if let Some(id) = last_event_id.read().await.clone() {
        req_builder = req_builder.header("Last-Event-ID", id);
    }
    req_builder = apply_auth_headers(req_builder, spec.auth.as_ref())
        .with_context(|| format!("MCP server `{}`: listener auth header", spec.name))?;
    let req = req_builder
        .body(Full::new(Bytes::new()))
        .context("build listener request")?;

    let resp = sender
        .send_request(req)
        .await
        .context("listener send_request")?;
    let status = resp.status();
    // Per spec §"Listening for Messages from the Server", servers
    // that do not offer a server-initiated stream return 405. That
    // is a clean refusal; treat it as "no listener channel here"
    // and don't keep reconnecting.
    if status.as_u16() == 405 {
        driver.abort();
        // Sleep a long time to effectively park this listener —
        // the loop will keep waking up but the server keeps
        // saying no, so the cap above protects us from a hot
        // loop. Returning Ok keeps the outer loop alive in case
        // the operator later enables the listener server-side.
        return Ok(Some(LISTENER_RECONNECT_CAP));
    }
    if !status.is_success() {
        driver.abort();
        bail!("listener got non-success status {status}");
    }

    // Stream the body chunk-by-chunk, accumulating into a buffer
    // and splitting on the SSE `\n\n` event terminator. We do NOT
    // collect the whole body — the GET listener is by design a
    // long-lived stream.
    let mut body = resp.into_body();
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut server_retry_hint: Option<Duration> = None;
    while let Some(frame_res) = body.frame().await {
        let frame = frame_res.context("listener body frame")?;
        if let Some(chunk) = frame.data_ref() {
            buffer.extend_from_slice(chunk);
            // Drain every complete event in the buffer (a complete
            // event is bytes-up-to-and-including the first `\n\n`).
            while let Some((idx, terminator_len)) = find_event_terminator(&buffer) {
                let event_bytes: Vec<u8> = buffer.drain(..idx).collect();
                // Consume the terminator too.
                buffer.drain(..terminator_len);
                let parsed =
                    parse_one_sse_event(&event_bytes).context("listener SSE event parse")?;
                if let Some(id) = parsed.id {
                    *last_event_id.write().await = Some(id);
                }
                if let Some(retry_ms) = parsed.retry_ms {
                    server_retry_hint = Some(Duration::from_millis(retry_ms));
                }
                for note in parsed.notifications {
                    let mut sink = sink_factory();
                    sink.observe(note);
                }
                // Listener stream MAY contain JSON-RPC responses too
                // (response to a client-side request the server is
                // streaming back). v1 ignores those — round-3 if it
                // matters. We don't surface them as notifications
                // because they'd violate the sink contract.
            }
        }
    }
    driver.abort();
    Ok(server_retry_hint)
}

/// Find the byte index where the first complete SSE event ends.
/// Returns (event_body_end, terminator_len) — `buffer[..idx]` is
/// the event body and the next `terminator_len` bytes are the
/// separator to be discarded. Looks for `\r\n\r\n` first so the
/// CRLF form isn't truncated to a bare `\n\n` match.
fn find_event_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    let lf_lf = buf.windows(2).position(|w| w == b"\n\n");
    let crlf_crlf = buf.windows(4).position(|w| w == b"\r\n\r\n");
    match (lf_lf, crlf_crlf) {
        (Some(a), Some(b)) if b <= a => Some((b, 4)),
        (Some(a), _) => Some((a, 2)),
        (None, Some(b)) => Some((b, 4)),
        (None, None) => None,
    }
}

#[derive(Debug, Default)]
struct ParsedSseEvent {
    notifications: Vec<ObservedNotification>,
    id: Option<String>,
    retry_ms: Option<u64>,
}

/// Parse one SSE event's bytes into the listener's view. Stricter
/// than `parse_sse_body` — listener events are never JSON-RPC
/// responses (those return through POST), so anything with an
/// `id` JSON field is silently dropped. Only `notifications/*`
/// frames flow to the sink.
fn parse_one_sse_event(event_bytes: &[u8]) -> anyhow::Result<ParsedSseEvent> {
    let text = std::str::from_utf8(event_bytes).context("SSE event not valid UTF-8")?;
    let normalised = text.replace("\r\n", "\n");
    let mut parsed = ParsedSseEvent::default();
    let mut data_chunks: Vec<&str> = Vec::new();
    for line in normalised.lines() {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => data_chunks.push(value),
            "id" => {
                parsed.id = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "retry" => {
                if let Ok(ms) = value.parse::<u64>() {
                    parsed.retry_ms = Some(ms);
                }
            }
            _ => {}
        }
    }
    if data_chunks.is_empty() {
        return Ok(parsed);
    }
    let payload = data_chunks.join("\n");
    let value: Value = match serde_json::from_str(&payload) {
        Ok(v) => v,
        Err(_) => return Ok(parsed),
    };
    if value.get("id").is_none() {
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            parsed.notifications.push(ObservedNotification {
                method: method.to_string(),
                params: value.get("params").cloned().unwrap_or(Value::Null),
            });
        }
    }
    Ok(parsed)
}

// Silence "unused" for Pin import — we use it transitively
// through the AsyncRead/Write impls below but the compiler can't
// tell from the where-clauses alone in some configurations.
#[allow(dead_code)]
type _Pin<T> = Pin<T>;

// ───────────────────────────────────────────────────────────────

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
    async fn https_url_builds_tls_connector_at_initialize() {
        // Round-2: HTTPS no longer rejects at initialize. The
        // initialize handshake still fails because no real server
        // exists at example.com:443 doing MCP, but the failure
        // mode is "connect/handshake" not "HTTPS unsupported".
        let spec = McpServerSpec {
            name: "tls".into(),
            transport: "http".into(),
            url: Some("https://127.0.0.1:1".into()),
            ..Default::default()
        };
        let err = HttpConnection::initialize(&spec).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            !msg.to_lowercase().contains("not yet supported"),
            "HTTPS must no longer be rejected as unsupported; got: {msg}"
        );
    }

    #[test]
    fn auth_bearer_appends_authorization_header() {
        let spec = McpServerSpec {
            name: "auth".into(),
            transport: "http".into(),
            url: Some("http://127.0.0.1:1".into()),
            auth: Some(AuthSpec::Bearer {
                token: "abc.def.ghi".into(),
            }),
            ..Default::default()
        };
        let req = Request::builder().method("POST").uri("/mcp");
        let req = apply_auth_headers(req, spec.auth.as_ref()).unwrap();
        let built = req.body(()).unwrap();
        let header = built
            .headers()
            .get(AUTHORIZATION)
            .expect("auth header set")
            .to_str()
            .unwrap();
        assert_eq!(header, "Bearer abc.def.ghi");
    }

    #[test]
    fn auth_bearer_env_reads_from_environment() {
        let key = "EASYNET_MCP_TEST_BEARER";
        std::env::set_var(key, "env-sourced-token");
        let req = Request::builder().method("POST").uri("/mcp");
        let req = apply_auth_headers(
            req,
            Some(&AuthSpec::BearerEnv { env: key.into() }),
        )
        .unwrap();
        let built = req.body(()).unwrap();
        assert_eq!(
            built
                .headers()
                .get(AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer env-sourced-token"
        );
        std::env::remove_var(key);
    }

    #[test]
    fn auth_bearer_env_missing_var_fails_with_clear_message() {
        // Picked an unlikely-to-collide name so this test is
        // hermetic regardless of CI env shape.
        let req = Request::builder().method("POST").uri("/mcp");
        let err = apply_auth_headers(
            req,
            Some(&AuthSpec::BearerEnv {
                env: "EASYNET_DEFINITELY_UNSET_xyz123".into(),
            }),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("EASYNET_DEFINITELY_UNSET_xyz123") && msg.contains("not set"),
            "BearerEnv missing-var error must name the var; got: {msg}"
        );
    }

    #[test]
    fn auth_headers_map_injects_arbitrary_pairs() {
        let mut h = std::collections::HashMap::new();
        h.insert("X-Api-Key".to_string(), "sk-1234".to_string());
        h.insert("MCP-Tenant-Id".to_string(), "acme-prod".to_string());
        let req = Request::builder().method("POST").uri("/mcp");
        let req = apply_auth_headers(req, Some(&AuthSpec::Headers { headers: h })).unwrap();
        let built = req.body(()).unwrap();
        assert_eq!(
            built.headers().get("X-Api-Key").unwrap().to_str().unwrap(),
            "sk-1234"
        );
        assert_eq!(
            built
                .headers()
                .get("MCP-Tenant-Id")
                .unwrap()
                .to_str()
                .unwrap(),
            "acme-prod"
        );
    }

    #[test]
    fn no_auth_leaves_request_unchanged() {
        let req = Request::builder().method("POST").uri("/mcp");
        let req = apply_auth_headers(req, None).unwrap();
        let built = req.body(()).unwrap();
        assert!(built.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn tls_spec_default_is_empty_and_serializes_compactly() {
        let spec = McpServerSpec {
            name: "compact".into(),
            transport: "http".into(),
            url: Some("http://127.0.0.1:3001".into()),
            ..Default::default()
        };
        let dumped = serde_json::to_string(&spec).unwrap();
        // Default TlsSpec has every field at its default — must be
        // omitted from the wire so existing operator configs don't
        // pick up a noisy `tls: {}` line.
        assert!(
            !dumped.contains("\"tls\""),
            "empty TLS spec should be skip_serializing_if-skipped; got: {dumped}"
        );
        // auth absent means no `auth` key emitted either.
        assert!(
            !dumped.contains("\"auth\""),
            "auth=None must skip serializing; got: {dumped}"
        );
    }

    #[test]
    fn tls_spec_round_trips_through_serde() {
        let mut headers = std::collections::HashMap::new();
        headers.insert("X-K".into(), "V".into());
        let spec = McpServerSpec {
            name: "round".into(),
            transport: "http".into(),
            url: Some("https://upstream.example.com:8443".into()),
            tls: TlsSpec {
                ca_bundle: Some(std::path::PathBuf::from("/etc/easynet/ca.pem")),
                server_name: Some("upstream.internal".into()),
                insecure_skip_verify: false,
            },
            auth: Some(AuthSpec::Headers { headers }),
            ..Default::default()
        };
        let dumped = serde_json::to_string(&spec).unwrap();
        let parsed: McpServerSpec = serde_json::from_str(&dumped).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn insecure_skip_verify_without_env_fails_build() {
        // Must run with the env var unset; if some other test
        // accidentally left it set, the test would mis-pass. Force
        // it unset for the duration of this test.
        std::env::remove_var("EASYNET_ALLOW_INSECURE_TLS");
        let tls = TlsSpec {
            ca_bundle: None,
            server_name: None,
            insecure_skip_verify: true,
        };
        let err = match build_tls_connector(&tls, "test-server") {
            Ok(_) => panic!("expected double-gate refusal, got Ok"),
            Err(e) => e,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("EASYNET_ALLOW_INSECURE_TLS"),
            "double-gate error must mention the env var; got: {msg}"
        );
    }

    #[test]
    fn validate_rejects_https_options_on_plain_http() {
        let spec = McpServerSpec {
            name: "mismatch".into(),
            transport: "http".into(),
            url: Some("http://127.0.0.1:3001".into()),
            tls: TlsSpec {
                ca_bundle: None,
                server_name: None,
                insecure_skip_verify: true,
            },
            ..Default::default()
        };
        let err = spec.validate().unwrap_err();
        assert!(format!("{err}").contains("insecure_skip_verify"));
    }

    // SSE parser fixture: confirm the round-2 parser surfaces the
    // last observed `id:` field even when only notifications carry
    // it. This is what feeds the `Last-Event-Id` replay path.
    #[test]
    fn sse_parser_records_last_event_id_across_frames() {
        let body = "id: 42\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progress\":0.1}}\n\nid: 43\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let parsed = parse_sse_body(body.as_bytes()).unwrap();
        assert_eq!(parsed.last_event_id.as_deref(), Some("43"));
        assert_eq!(parsed.notifications.len(), 1);
    }

    #[test]
    fn sse_parser_resets_last_event_id_on_empty_id_field() {
        let body = "id: 99\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":null}\n\nid: \ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let parsed = parse_sse_body(body.as_bytes()).unwrap();
        // Per HTML spec, `id:` with empty value resets to "no
        // last event id".
        assert!(parsed.last_event_id.is_none());
    }

    #[test]
    fn find_event_terminator_handles_both_line_endings() {
        assert_eq!(find_event_terminator(b"abc\n\nrest"), Some((3, 2)));
        assert_eq!(find_event_terminator(b"abc\r\n\r\nrest"), Some((3, 4)));
        assert_eq!(find_event_terminator(b"no-terminator-here"), None);
    }

    #[test]
    fn parse_one_sse_event_picks_up_retry_field() {
        let event = b"retry: 1500\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}";
        let parsed = parse_one_sse_event(event).unwrap();
        assert_eq!(parsed.retry_ms, Some(1500));
        assert_eq!(parsed.notifications.len(), 1);
    }

    #[test]
    fn parse_one_sse_event_drops_json_rpc_responses() {
        let event = b"data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}";
        let parsed = parse_one_sse_event(event).unwrap();
        // Listener events are NOT response carriers; they
        // surface as zero notifications.
        assert!(parsed.notifications.is_empty());
    }
}
