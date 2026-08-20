//! Version-tolerant Chrome DevTools Protocol dispatcher.
//! =====================================================
//!
//! File: plugins/browser/src/cdp.rs
//! Description: One-reader CDP WebSocket dispatcher with bounded correlation.
//!
//! Protocol Responsibility:
//! - CDP JSON is an application payload. Axon remains the only public session
//!   transport and owns terminal receipt semantics.
//!
//! Implementation Approach:
//! - One task exclusively reads the WebSocket.
//! - Command ids index a bounded pending map; response, timeout, send failure,
//!   connection close, and future cancellation each reclaim their row.
//! - Uncorrelated method messages fan out through a bounded event channel.
//!
//! Usage Contract:
//! - Callers supply the plugin-owned target session id.
//! - No caller may place an independently chosen `sessionId` on the wire.
//!
//! Architectural Position:
//! - Browser plugin infrastructure, below ability handlers and above Chrome.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, oneshot, watch, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::constants::{CDP_COMMAND_TIMEOUT_SECONDS, CDP_EVENT_BOUND, CDP_PENDING_BOUND};

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type CdpWriter = futures::stream::SplitSink<CdpSocket, Message>;
type PendingSender = oneshot::Sender<Result<Value, CdpFailure>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdpConnectionState {
    Active,
    Closing,
    Closed,
    Failed,
}

impl CdpConnectionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Closed => "closed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CdpFailure {
    #[error("CDP connection failed: {0}")]
    Connect(String),
    #[error("CDP connection is closed")]
    Closed,
    #[error("CDP pending-call capacity {0} reached")]
    PendingCapacity(usize),
    #[error("CDP command {method:?} timed out after {seconds}s")]
    Timeout { method: String, seconds: u64 },
    #[error("CDP command {method:?} rejected ({code}): {message}")]
    Protocol {
        method: String,
        code: i64,
        message: String,
        data: Option<Value>,
    },
    #[error("CDP wire error: {0}")]
    Wire(String),
}

pub struct CdpClient {
    writer: Arc<Mutex<CdpWriter>>,
    pending: Arc<StdMutex<HashMap<u64, PendingSender>>>,
    next_id: AtomicU64,
    events: broadcast::Sender<CdpEvent>,
    state_rx: watch::Receiver<CdpConnectionState>,
    state_tx: watch::Sender<CdpConnectionState>,
    shutdown_tx: watch::Sender<bool>,
}

/// Removes an admitted correlation row even when the command future is
/// cancelled by attachment teardown. The critical section is a single HashMap
/// removal and never crosses an await point.
struct PendingCallLease {
    id: u64,
    pending: Arc<StdMutex<HashMap<u64, PendingSender>>>,
}

impl Drop for PendingCallLease {
    fn drop(&mut self) {
        self.pending
            .lock()
            .expect("CDP pending map poisoned")
            .remove(&self.id);
    }
}

/// Admit only target-scoped domains through the agent-facing raw CDP surface.
/// Browser/Target routing stays plugin-owned so one attachment cannot escape
/// its resource URA by choosing another target or shutting down the process.
pub fn agent_method_allowed(method: &str) -> bool {
    if method.len() > super::constants::MAX_CDP_METHOD_BYTES || !method.is_ascii() {
        return false;
    }
    if matches!(
        method,
        "DOM.setFileInputFiles"
            | "Network.loadNetworkResource"
            | "Page.close"
            | "Page.crash"
            | "Page.setDownloadBehavior"
            | "Runtime.terminateExecution"
    ) {
        return false;
    }
    let Some((domain, operation)) = method.split_once('.') else {
        return false;
    };
    !operation.is_empty()
        && operation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && matches!(
            domain,
            "Accessibility"
                | "Animation"
                | "Audits"
                | "Autofill"
                | "CSS"
                | "Console"
                | "DOM"
                | "DOMDebugger"
                | "DOMSnapshot"
                | "DOMStorage"
                | "Debugger"
                | "Emulation"
                | "Fetch"
                | "Input"
                | "LayerTree"
                | "Log"
                | "Network"
                | "Overlay"
                | "Page"
                | "Performance"
                | "PerformanceTimeline"
                | "Profiler"
                | "Runtime"
                | "Schema"
                | "Security"
                | "ServiceWorker"
                | "Storage"
                | "WebAudio"
                | "WebAuthn"
        )
}

pub fn validate_agent_command(method: &str, params: Option<&Value>) -> Result<(), String> {
    if !agent_method_allowed(method) {
        return Err(format!(
            "CDP method {method:?} is outside the target policy"
        ));
    }
    if method == "Page.navigate" {
        let url = params
            .and_then(Value::as_object)
            .and_then(|params| params.get("url"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Page.navigate requires a string url".to_string())?;
        if url.len() > super::constants::MAX_URL_BYTES {
            return Err(format!(
                "Page.navigate url exceeds {} bytes",
                super::constants::MAX_URL_BYTES
            ));
        }
        let parsed = url::Url::parse(url)
            .map_err(|error| format!("Page.navigate url is invalid: {error}"))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err("Page.navigate url must be absolute http(s)".to_string());
        }
    }
    Ok(())
}

impl CdpClient {
    pub async fn connect(endpoint: &str) -> Result<Arc<Self>, CdpFailure> {
        let (socket, _) = tokio_tungstenite::connect_async(endpoint)
            .await
            .map_err(|error| CdpFailure::Connect(error.to_string()))?;
        let (writer, mut reader) = socket.split();
        let writer = Arc::new(Mutex::new(writer));
        let pending = Arc::new(StdMutex::new(HashMap::<u64, PendingSender>::new()));
        let (events, _) = broadcast::channel(CDP_EVENT_BOUND);
        let (state_tx, state_rx) = watch::channel(CdpConnectionState::Active);
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        let client = Arc::new(Self {
            writer: Arc::clone(&writer),
            pending: Arc::clone(&pending),
            next_id: AtomicU64::new(1),
            events: events.clone(),
            state_rx,
            state_tx: state_tx.clone(),
            shutdown_tx,
        });

        let reader_pending = Arc::clone(&pending);
        let reader_state = state_tx.clone();
        tokio::spawn(async move {
            let mut failed = false;
            loop {
                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    message = reader.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                dispatch_incoming(&reader_pending, &events, text.as_str());
                            }
                            Some(Ok(Message::Binary(bytes))) => {
                                match std::str::from_utf8(bytes.as_ref()) {
                                    Ok(text) => dispatch_incoming(&reader_pending, &events, text),
                                    Err(_) => {
                                        failed = true;
                                        break;
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                            Some(Ok(Message::Close(_))) | None => break,
                            Some(Ok(_)) => {}
                            Some(Err(_)) => {
                                failed = true;
                                break;
                            }
                        }
                    }
                }
            }
            drain_pending(&reader_pending, CdpFailure::Closed);
            let _ = reader_state.send(if failed {
                CdpConnectionState::Failed
            } else {
                CdpConnectionState::Closed
            });
        });

        let keepalive_writer = Arc::clone(&writer);
        let mut keepalive_shutdown = client.shutdown_tx.subscribe();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.tick().await;
            loop {
                tokio::select! {
                    changed = keepalive_shutdown.changed() => {
                        if changed.is_err() || *keepalive_shutdown.borrow() {
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        let result = keepalive_writer
                            .lock()
                            .await
                            .send(Message::Ping(Vec::new().into()))
                            .await;
                        if result.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(client)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    pub fn state(&self) -> CdpConnectionState {
        *self.state_rx.borrow()
    }

    pub async fn send_command(
        &self,
        method: &str,
        params: Option<Value>,
        session_id: Option<&str>,
    ) -> Result<Value, CdpFailure> {
        if self.state() != CdpConnectionState::Active {
            return Err(CdpFailure::Closed);
        }
        let id = self.next_wire_id();
        let mut command = Map::new();
        command.insert("id".to_string(), Value::from(id));
        command.insert("method".to_string(), Value::String(method.to_string()));
        if let Some(params) = params {
            command.insert("params".to_string(), params);
        }
        if let Some(session_id) = session_id.filter(|value| !value.is_empty()) {
            command.insert(
                "sessionId".to_string(),
                Value::String(session_id.to_string()),
            );
        }
        let encoded = serde_json::to_string(&Value::Object(command))
            .map_err(|error| CdpFailure::Wire(error.to_string()))?;

        let (response_tx, response_rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().expect("CDP pending map poisoned");
            if pending.len() >= CDP_PENDING_BOUND {
                return Err(CdpFailure::PendingCapacity(CDP_PENDING_BOUND));
            }
            pending.insert(id, response_tx);
        }
        let _pending_call = PendingCallLease {
            id,
            pending: Arc::clone(&self.pending),
        };

        if let Err(error) = self
            .writer
            .lock()
            .await
            .send(Message::Text(encoded.into()))
            .await
        {
            self.pending
                .lock()
                .expect("CDP pending map poisoned")
                .remove(&id);
            return Err(CdpFailure::Wire(error.to_string()));
        }

        match tokio::time::timeout(
            Duration::from_secs(CDP_COMMAND_TIMEOUT_SECONDS),
            wait_for_response_or_disconnect(self.state_rx.clone(), response_rx),
        )
        .await
        {
            Ok(Ok(response)) => decode_response(method, response),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(CdpFailure::Timeout {
                method: method.to_string(),
                seconds: CDP_COMMAND_TIMEOUT_SECONDS,
            }),
        }
    }

    pub async fn shutdown(&self) {
        if self.state() != CdpConnectionState::Active {
            return;
        }
        let _ = self.state_tx.send(CdpConnectionState::Closing);
        let _ = self.shutdown_tx.send(true);
        let _ = self.writer.lock().await.send(Message::Close(None)).await;
        drain_pending(&self.pending, CdpFailure::Closed);
    }

    fn next_wire_id(&self) -> u64 {
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            if id != 0 {
                return id;
            }
        }
    }
}

async fn wait_for_response_or_disconnect(
    mut state_rx: watch::Receiver<CdpConnectionState>,
    response_rx: oneshot::Receiver<Result<Value, CdpFailure>>,
) -> Result<Value, CdpFailure> {
    if *state_rx.borrow() != CdpConnectionState::Active {
        return Err(CdpFailure::Closed);
    }
    tokio::select! {
        response = response_rx => response.unwrap_or(Err(CdpFailure::Closed)),
        _ = state_rx.changed() => Err(CdpFailure::Closed),
    }
}

fn dispatch_incoming(
    pending: &Arc<StdMutex<HashMap<u64, PendingSender>>>,
    events: &broadcast::Sender<CdpEvent>,
    text: &str,
) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        let sender = pending
            .lock()
            .expect("CDP pending map poisoned")
            .remove(&id);
        if let Some(sender) = sender {
            let _ = sender.send(Ok(value));
        }
        return;
    }
    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let _ = events.send(CdpEvent {
        method: method.to_string(),
        params: value.get("params").cloned().unwrap_or(Value::Null),
        session_id: value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string),
    });
}

fn decode_response(method: &str, response: Value) -> Result<Value, CdpFailure> {
    if let Some(error) = response.get("error") {
        return Err(CdpFailure::Protocol {
            method: method.to_string(),
            code: error.get("code").and_then(Value::as_i64).unwrap_or(-1),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown CDP protocol error")
                .to_string(),
            data: error.get("data").cloned(),
        });
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

fn drain_pending(pending: &Arc<StdMutex<HashMap<u64, PendingSender>>>, failure: CdpFailure) {
    let senders = {
        let mut pending = pending.lock().expect("CDP pending map poisoned");
        pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>()
    };
    for sender in senders {
        let _ = sender.send(Err(failure.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_protocol_error_without_losing_code() {
        let error = decode_response(
            "Page.navigate",
            json!({"id": 4, "error": {"code": -32602, "message": "bad url"}}),
        )
        .expect_err("protocol error");
        assert!(matches!(error, CdpFailure::Protocol { code: -32602, .. }));
    }

    #[test]
    fn agent_policy_keeps_target_routing_server_side() {
        assert!(agent_method_allowed("DOM.getDocument"));
        assert!(agent_method_allowed("Page.navigate"));
        assert!(!agent_method_allowed("Target.attachToTarget"));
        assert!(!agent_method_allowed("Browser.close"));
        assert!(!agent_method_allowed("Page.crash"));
        assert!(!agent_method_allowed("DOM.setFileInputFiles"));
        assert!(!agent_method_allowed("Network.loadNetworkResource"));
        assert!(!agent_method_allowed("not-a-method"));
    }

    #[test]
    fn agent_navigation_rejects_local_file_urls() {
        assert!(validate_agent_command(
            "Page.navigate",
            Some(&json!({"url":"https://example.com"}))
        )
        .is_ok());
        assert!(validate_agent_command(
            "Page.navigate",
            Some(&json!({"url":"file:///etc/passwd"}))
        )
        .is_err());
        assert!(!agent_method_allowed(&format!(
            "Runtime.{}",
            "x".repeat(super::super::constants::MAX_CDP_METHOD_BYTES)
        )));
    }

    #[tokio::test]
    async fn incoming_response_consumes_exactly_one_pending_row() {
        let pending = Arc::new(StdMutex::new(HashMap::new()));
        let (sender, receiver) = oneshot::channel();
        pending.lock().expect("pending map").insert(7, sender);
        let (events, _) = broadcast::channel(2);
        dispatch_incoming(&pending, &events, r#"{"id":7,"result":{"ok":true}}"#);
        assert!(pending.lock().expect("pending map").is_empty());
        assert_eq!(
            receiver.await.expect("response").expect("ok")["result"]["ok"],
            true
        );
    }

    #[test]
    fn cancelled_command_lease_reclaims_pending_capacity() {
        let pending = Arc::new(StdMutex::new(HashMap::new()));
        let (sender, _receiver) = oneshot::channel();
        pending.lock().expect("pending map").insert(11, sender);
        {
            let _lease = PendingCallLease {
                id: 11,
                pending: Arc::clone(&pending),
            };
        }
        assert!(pending.lock().expect("pending map").is_empty());
    }

    #[tokio::test]
    async fn incoming_event_preserves_flat_session_id() {
        let pending = Arc::new(StdMutex::new(HashMap::new()));
        let (events, mut receiver) = broadcast::channel(2);
        dispatch_incoming(
            &pending,
            &events,
            r#"{"method":"Page.loadEventFired","params":{},"sessionId":"s-1"}"#,
        );
        let event = receiver.recv().await.expect("event");
        assert_eq!(event.session_id.as_deref(), Some("s-1"));
    }

    #[tokio::test]
    async fn connection_state_change_wakes_a_pending_command_without_timeout() {
        let (state_tx, state_rx) = watch::channel(CdpConnectionState::Active);
        let (_response_tx, response_rx) = oneshot::channel();
        state_tx
            .send(CdpConnectionState::Closed)
            .expect("state receiver remains live");

        let result = tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_response_or_disconnect(state_rx, response_rx),
        )
        .await
        .expect("disconnect notification must not wait for command timeout");
        assert!(matches!(result, Err(CdpFailure::Closed)));
    }
}
