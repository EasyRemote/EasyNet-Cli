// EasyNet CLI — Streamable HTTP transport for outbound MCP
// =========================================================
//
// File: src/runtime/execution/mcp_client/http/mod.rs
//
// Per MCP spec 2025-06-18 §"Streamable HTTP" transport. Sibling of
// the stdio transport in `mcp_client/mod.rs`.
//
// Module layout
// -------------
// The transport was originally a single 2100+-line file; it is now
// split into focused submodules so each concern can be read and
// reviewed in isolation:
//
//   mod.rs        — [`HttpConnection`] (this file): connection
//                   lifecycle, `initialize`, `rpc`/`rpc_with_sink`,
//                   POST + SSE response decoding, the GET listener
//                   spawner (delegates to `listener::listener_loop`),
//                   and the in-process integration tests.
//   listener.rs   — Long-lived GET listener (`listener_loop`,
//                   `listener_connect_and_pump`) per spec
//                   §"Listening for Messages from the Server" plus
//                   the reconnect-backoff constants.
//   sse.rs        — SSE wire parsing (`parse_sse_body`,
//                   `parse_one_sse_event`, `find_event_terminator`).
//   tls.rs        — rustls connector building, `InsecureCertVerifier`,
//                   `AsyncStream` marker trait.
//   auth.rs       — `apply_auth_headers` for the four `AuthSpec`
//                   variants (none / Bearer / BearerEnv / Headers).
//   hyper_io.rs   — `HyperTokioIo`, the hyper-rt ↔ tokio-IO shim.
//
// Public surface
// --------------
// Only [`HttpConnection`] is exported. Everything else is
// `pub(super)` so call sites in this crate go through the
// connection type rather than poking into transport internals.
//
// What this transport implements (round-1 + round-2)
// --------------------------------------------------
//   * POST JSON-RPC requests with `Content-Type: application/json`
//     and `Content-Type: text/event-stream` response decoding;
//     intervening `notifications/*` frames are routed to a caller-
//     supplied [`NotificationSink`].
//   * Initialize handshake captures the optional `Mcp-Session-Id`
//     header and threads it on every subsequent request, plus
//     `MCP-Protocol-Version` per §"Protocol Version Header".
//   * **TLS** via rustls — Mozilla roots by default; per-server CA
//     bundle override; per-server SNI override; double-gated
//     `insecure_skip_verify` (config flag AND env var).
//   * **Authentication** — Bearer / BearerEnv / arbitrary header
//     map. Applied to every POST and to the GET listener.
//   * **Implicit GET / SSE listener channel** per §"Listening for
//     Messages from the Server" — long-lived background task,
//     routes server-initiated notifications, reconnects with
//     exponential backoff capped at 30s, honours server-supplied
//     `retry:` hint.
//   * **Last-Event-Id resumption** per §"Resumability and Retries":
//     every SSE frame's `id:` is recorded, the listener replays the
//     latest id on reconnect.
//
// Not yet implemented (out of round-2 scope, documented for the
// next maintainer):
//   * Outbound HTTP/2. We use HTTP/1.1; spec MANDATES servers
//     support HTTP/1.1 so this is acceptable. HTTP/2 is a future
//     perf optimisation.
//   * OAuth refresh flows. `AuthSpec::BearerEnv` covers the
//     "rotate token externally" case; richer OAuth belongs behind a
//     sidecar.
//   * End-to-end HTTPS round-trip against an in-process rustls
//     server. The TLS connector is unit-tested (see `tls.rs`) but
//     a full TLS-wire e2e needs an in-process server with a
//     self-signed cert via `rcgen`; lands as
//     `tests/streamable_http_tls_e2e.rs` in a follow-up.
//   * GET listener e2e against a mock server emitting interleaved
//     server-initiated notifications. The unit tests cover the
//     parse path; the integration test would pin reconnect +
//     `Last-Event-Id` replay end-to-end.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::header::{ACCEPT, CONTENT_TYPE};
use hyper::{Request, Uri};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use super::{McpServerSpec, NotificationSink, ObservedNotification};

mod auth;
mod hyper_io;
mod listener;
mod sse;
mod tls;

use auth::apply_auth_headers;
use hyper_io::HyperTokioIo;
use listener::listener_loop;
use sse::parse_sse_body;
use tls::{build_tls_connector, AsyncStream};

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
pub(super) const PROTOCOL_VERSION: &str = "2025-06-18";

/// Header per MCP spec 2025-06-18 §"Session Management".
pub(super) const HEADER_SESSION_ID: &str = "Mcp-Session-Id";

/// Header per MCP spec 2025-06-18 §"Protocol Version Header".
pub(super) const HEADER_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";

/// Default timeout for any single HTTP round-trip. A real MCP
/// server should respond in milliseconds; if it takes longer than
/// 30s we want to fail loudly rather than wedge the caller.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

// GET listener reconnect constants live in the `listener` submodule
// alongside the loop that consumes them.

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
    /// spawned. `Some(_)` after `spawn_listener` runs; the `Drop`
    /// impl aborts it so a closed `HttpConnection` does not leak a
    /// task.
    ///
    /// Backed by `std::sync::Mutex` rather than `tokio::sync::Mutex`
    /// so the `Drop` impl can `lock()` directly (no `try_lock`
    /// fallback, no theoretical leak window): the only writer is
    /// `spawn_listener`, whose critical section is a tiny synchronous
    /// swap that never crosses an `.await` while holding the guard.
    /// Using a sync mutex here keeps `Drop` infallible without
    /// pulling in `parking_lot`.
    listener_handle: StdMutex<Option<JoinHandle<()>>>,
}

impl Drop for HttpConnection {
    fn drop(&mut self) {
        // `std::sync::Mutex::lock` only returns Err on poison; a
        // poisoned mutex still hands back the inner data (`into_inner`
        // on the error) so we can abort the listener regardless.
        // Using `lock()` here (rather than the previous `try_lock`)
        // eliminates the theoretical leak window that existed when
        // `Drop` raced the `spawn_listener` writer for the lock.
        let mut guard = match self.listener_handle.lock() {
            Ok(g) => g,
            Err(poison) => poison.into_inner(),
        };
        if let Some(h) = guard.take() {
            h.abort();
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
            listener_handle: StdMutex::new(None),
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
            .do_post(request_body, true, sink)
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
    /// thread a sink through `rpc_with_sink` → `do_post`.
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
            .unwrap_or(if self.tls_connector.is_some() {
                443
            } else {
                80
            });

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
            // completes. Aborted explicitly after the response is
            // consumed so we don't leak the task on the slow path
            // where the server holds the connection open.
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
            // posted to the server receive 202 with no body.
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
                // SSE — possibly with interleaved notifications.
                // See `sse::parse_sse_body` for the wire-shape
                // contract.
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
        // `std::sync::Mutex` — the critical section is a synchronous
        // swap that never crosses `.await` while the guard is held
        // (we drop the guard before returning). Pairs with the
        // `Drop` impl's `lock()` to eliminate the previous
        // `try_lock` leak window.
        let mut guard = self
            .listener_handle
            .lock()
            .expect("listener_handle mutex poisoned");
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

// ───────────────────────────────────────────────────────────────
// Integration tests — drive a real in-process hyper server.
// SSE-parser, TLS-build, and auth-header unit tests live next to
// their implementations in the sse / tls / auth submodules; tests
// here cover the HttpConnection end-to-end wire shape.
// ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::execution::mcp_client::McpServerSpec;
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
        use std::sync::atomic::{AtomicBool, Ordering};
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
    /// receives them in arrival order.
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
        assert_eq!(result["content"][0]["text"], "done");
        assert_eq!(result["isError"], false);
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
        let result = conn
            .rpc("tools/list", json!({}))
            .await
            .expect("plain rpc still works on SSE response with notifications");
        assert!(result.get("tools").is_some());
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
        // exists at 127.0.0.1:1 doing MCP, but the failure mode is
        // "connect/handshake" not "HTTPS unsupported".
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
        use crate::runtime::execution::mcp_client::{AuthSpec, TlsSpec};
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
    fn validate_rejects_https_options_on_plain_http() {
        use crate::runtime::execution::mcp_client::TlsSpec;
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
}
