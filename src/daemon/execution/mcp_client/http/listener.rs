// EasyNet CLI — Streamable HTTP / GET listener channel
// ====================================================
//
// File: src/daemon/execution/mcp_client/http/listener.rs
//
// Implements the GET listener channel from MCP spec 2025-06-18
// §"Listening for Messages from the Server". The listener is a
// long-lived background task that opens a `GET` on the MCP endpoint
// with `Accept: text/event-stream`, streams the response, parses SSE
// frames as they arrive, routes server-initiated notifications to a
// caller-supplied sink, and reconnects on transport failure with the
// most recent `Last-Event-Id` replayed per spec §"Resumability and
// Retries".
//
// Why this is its own module
// --------------------------
// The listener is the second-largest concern in the `http` submodule
// (after `HttpConnection` itself). Splitting it out keeps mod.rs
// focused on per-call POST + initialize, and gives the reconnect /
// resumption logic its own unit-test surface in this file.
//
// Public surface (crate-private)
// ------------------------------
// `listener_loop` is the only entry point. `HttpConnection::
// spawn_listener` invokes it once per upstream; everything else is
// `pub(super)`-only.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn::http1;
use hyper::header::ACCEPT;
use hyper::{Request, Uri};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

use super::auth::apply_auth_headers;
use super::hyper_io::HyperTokioIo;
use super::sse::{find_event_terminator, parse_one_sse_event};
use super::tls::AsyncStream;
use super::{HEADER_PROTOCOL_VERSION, HEADER_SESSION_ID, PROTOCOL_VERSION};
use crate::daemon::execution::mcp_client::{McpServerSpec, NotificationSink};

/// Backoff cap for the GET listener reconnect loop. The SSE `retry:`
/// field, when emitted by a server, overrides this for the next
/// reconnect; otherwise we cap exponential backoff here so a
/// permanently-broken server doesn't burn battery on retry storms.
pub(super) const LISTENER_RECONNECT_CAP: Duration = Duration::from_secs(30);

/// Initial reconnect delay; doubles on each consecutive failure up
/// to [`LISTENER_RECONNECT_CAP`].
const LISTENER_RECONNECT_INITIAL: Duration = Duration::from_millis(500);

/// Long-lived GET listener loop. Opens a `GET` on the MCP endpoint
/// with `Accept: text/event-stream`, streams the response, parses
/// SSE frames as they arrive, routes notifications to the sink, and
/// reconnects on transport failure with the latest `Last-Event-Id`.
pub(super) async fn listener_loop(
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
            Err(e) => {
                // Connection error. Exponential backoff capped at
                // LISTENER_RECONNECT_CAP. SRE pipelines grep
                // `kind=listener_reconnect` to surface a
                // permanently-broken upstream rather than letting it
                // retry-storm silently. The doubled delay is computed
                // first so the emitted `next_delay_ms` matches the
                // sleep that follows.
                delay = (delay * 2).min(LISTENER_RECONNECT_CAP);
                let server_label = spec.name.as_str();
                let err_msg = format!("{e:#}");
                let next_delay_ms = delay.as_millis() as u64;
                crate::op_event!(
                    component = mcp_http_client,
                    kind = listener_reconnect,
                    server = server_label,
                    next_delay_ms = next_delay_ms,
                    error = err_msg,
                );
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
            let sni = spec.tls.server_name.clone().unwrap_or_else(|| host.clone());
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
        // the loop will keep waking up but the server keeps saying
        // no, so the cap protects us from a hot loop. Returning
        // Ok keeps the outer loop alive in case the operator
        // later enables the listener server-side.
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
