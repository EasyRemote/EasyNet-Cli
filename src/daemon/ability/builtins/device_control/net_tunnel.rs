// EasyNet CLI - governed TCP tunnel ability
// =========================================
//
// File: src/daemon/ability/builtins/device_control/net_tunnel.rs
// Description: Descriptor-bound TCP connect/listen execution over InvokeBidi.
//
// Security Contract
// -----------------
// The baseline policy permits loopback destinations and loopback listeners
// only. DNS answers are validated after resolution and before connect. Caller
// limits may narrow, never widen, daemon maxima.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::daemon::ability::dispatch::{
    AxonAbilityCatalog, BidiOutputFrame, BidiSource, OwnerKind, BIDI_CHANNEL_BOUND,
};

pub const ABILITY_NET_TUNNEL: &str = crate::daemon::ability::names::device_control::NET_TUNNEL;

const MAX_CONNECTIONS: usize = 32;
const MAX_BYTES_PER_CONNECTION: u64 = 1024 * 1024 * 1024;
const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_RATE_BYTES_PER_SECOND: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct TunnelPolicy {
    max_connections: usize,
    max_bytes_per_connection: u64,
    max_frame_bytes: usize,
    max_rate_bytes_per_second: u64,
    max_connect_timeout: Duration,
    max_idle_timeout: Duration,
}

impl TunnelPolicy {
    fn baseline() -> Self {
        Self {
            max_connections: MAX_CONNECTIONS,
            max_bytes_per_connection: MAX_BYTES_PER_CONNECTION,
            max_frame_bytes: READ_CHUNK_BYTES,
            max_rate_bytes_per_second: MAX_RATE_BYTES_PER_SECOND,
            max_connect_timeout: MAX_CONNECT_TIMEOUT,
            max_idle_timeout: MAX_IDLE_TIMEOUT,
        }
        .validate()
        .expect("static loopback tunnel policy must be valid")
    }

    fn validate(self) -> anyhow::Result<Self> {
        anyhow::ensure!(self.max_connections > 0, "max_connections must be positive");
        anyhow::ensure!(
            self.max_bytes_per_connection > 0,
            "max_bytes_per_connection must be positive"
        );
        anyhow::ensure!(self.max_frame_bytes > 0, "max_frame_bytes must be positive");
        anyhow::ensure!(
            self.max_rate_bytes_per_second >= self.max_frame_bytes as u64,
            "max_rate_bytes_per_second must admit at least one maximum-size frame"
        );
        anyhow::ensure!(
            !self.max_connect_timeout.is_zero(),
            "max_connect_timeout must be positive"
        );
        anyhow::ensure!(
            !self.max_idle_timeout.is_zero(),
            "max_idle_timeout must be positive"
        );
        Ok(self)
    }

    fn permits_address(self, address: IpAddr) -> bool {
        address.is_loopback()
    }

    async fn resolve_connect(self, host: &str, port: u16) -> anyhow::Result<SocketAddr> {
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| anyhow::anyhow!("DNS_RESOLUTION_FAILED: {error}"))?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            anyhow::bail!("DNS_RESOLUTION_FAILED: no address for {host}:{port}");
        }
        if addresses
            .iter()
            .any(|address| !self.permits_address(address.ip()))
        {
            anyhow::bail!(
                "DESTINATION_DENIED: every resolved address must satisfy loopback policy"
            );
        }
        Ok(addresses[0])
    }

    fn validate_bind(self, host: &str, port: u16) -> anyhow::Result<SocketAddr> {
        let address = host
            .parse::<IpAddr>()
            .map_err(|_| anyhow::anyhow!("BIND_DENIED: bind_host must be a literal IP address"))?;
        if !self.permits_address(address) {
            anyhow::bail!("BIND_DENIED: baseline tunnel policy permits loopback binds only");
        }
        Ok(SocketAddr::new(address, port))
    }
}

#[derive(Debug, Clone)]
enum TunnelOpen {
    Connect {
        host: String,
        port: u16,
        connect_timeout: Duration,
        idle_timeout: Duration,
    },
    Listen {
        bind: SocketAddr,
        idle_timeout: Duration,
    },
}

impl TunnelOpen {
    fn parse(value: Value, policy: TunnelPolicy) -> anyhow::Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("net.tunnel args must be an object"))?;
        let mode = required_string(object, "mode")?;
        let idle_timeout = narrowed_timeout(object, "idle_timeout_ms", policy.max_idle_timeout)?;
        match mode {
            "connect" => {
                reject_unknown(
                    object,
                    &[
                        "mode",
                        "host",
                        "port",
                        "connect_timeout_ms",
                        "idle_timeout_ms",
                    ],
                )?;
                Ok(Self::Connect {
                    host: required_string(object, "host")?.to_string(),
                    port: required_port(object, "port")?,
                    connect_timeout: narrowed_timeout(
                        object,
                        "connect_timeout_ms",
                        policy.max_connect_timeout,
                    )?,
                    idle_timeout,
                })
            }
            "listen" => {
                reject_unknown(
                    object,
                    &["mode", "bind_host", "bind_port", "idle_timeout_ms"],
                )?;
                let host = required_string(object, "bind_host")?;
                let port = optional_port(object, "bind_port")?.unwrap_or(0);
                Ok(Self::Listen {
                    bind: policy.validate_bind(host, port)?,
                    idle_timeout,
                })
            }
            other => anyhow::bail!("net.tunnel mode must be connect or listen, got {other:?}"),
        }
    }
}

pub fn register(registry: &mut AxonAbilityCatalog) {
    let policy = TunnelPolicy::baseline();
    registry.register_bidi_with_owner(
        ABILITY_NET_TUNNEL,
        OwnerKind::locomotion_system(),
        Arc::new(move |args| open(args, policy)),
    );
}

fn open(args: Value, policy: TunnelPolicy) -> anyhow::Result<BidiSource> {
    let request = TunnelOpen::parse(args, policy)?;
    let (to_handler, from_transport) = mpsc::channel(BIDI_CHANNEL_BOUND);
    let (to_transport, from_handler) = mpsc::channel(BIDI_CHANNEL_BOUND);
    match request {
        TunnelOpen::Connect {
            host,
            port,
            connect_timeout,
            idle_timeout,
        } => spawn_connect(
            policy,
            host,
            port,
            connect_timeout,
            idle_timeout,
            from_transport,
            to_transport,
        ),
        TunnelOpen::Listen { bind, idle_timeout } => {
            spawn_listener(policy, bind, idle_timeout, from_transport, to_transport)
        }
    }
    Ok(BidiSource {
        to_client: to_handler,
        from_client: from_handler,
    })
}

fn spawn_connect(
    policy: TunnelPolicy,
    host: String,
    port: u16,
    connect_timeout: Duration,
    idle_timeout: Duration,
    mut incoming: mpsc::Receiver<Value>,
    outgoing: mpsc::Sender<BidiOutputFrame>,
) {
    tokio::spawn(async move {
        let address = match policy.resolve_connect(&host, port).await {
            Ok(address) => address,
            Err(error) => {
                emit_fatal_error(&outgoing, None, error.to_string()).await;
                return;
            }
        };
        let stream = match tokio::time::timeout(connect_timeout, TcpStream::connect(address)).await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                emit_fatal_error(&outgoing, None, format!("CONNECT_FAILED: {error}")).await;
                return;
            }
            Err(_) => {
                emit_fatal_error(&outgoing, None, "CONNECT_TIMEOUT".to_string()).await;
                return;
            }
        };
        let connection_id = uuid::Uuid::new_v4().to_string();
        if outgoing
            .send(BidiOutputFrame::json(json!({
                "type": "connected",
                "connection_id": connection_id,
                "resolved_address": address.to_string(),
            })))
            .await
            .is_err()
        {
            return;
        }
        let (mut reader, mut writer) = stream.into_split();
        let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
        let transferred = Arc::new(AtomicU64::new(0));
        let rate = Arc::new(Mutex::new(RateBudget::new()));
        let mut read_open = true;
        let mut write_open = true;
        loop {
            enum Event {
                Network(std::io::Result<usize>),
                Client(Option<Value>),
            }
            let event = tokio::time::timeout(idle_timeout, async {
                tokio::select! {
                    read = reader.read(&mut buffer), if read_open => Event::Network(read),
                    frame = incoming.recv() => Event::Client(frame),
                }
            })
            .await;
            let event = match event {
                Ok(event) => event,
                Err(_) => {
                    emit_fatal_error(&outgoing, Some(&connection_id), "IDLE_TIMEOUT".to_string())
                        .await;
                    return;
                }
            };
            match event {
                Event::Network(Ok(0)) => {
                    read_open = false;
                    let _ = outgoing
                        .send(BidiOutputFrame::json(json!({
                            "type": "half_close",
                            "connection_id": connection_id,
                            "direction": "read",
                        })))
                        .await;
                    if !write_open {
                        break;
                    }
                }
                Event::Network(Ok(read)) => {
                    if let Err(code) = charge_connection(&transferred, &rate, read, policy) {
                        emit_fatal_error(&outgoing, Some(&connection_id), code.to_string()).await;
                        return;
                    }
                    if outgoing.send(BidiOutputFrame::json(json!({
                        "type": "data",
                        "connection_id": connection_id,
                        "data": base64::engine::general_purpose::STANDARD.encode(&buffer[..read]),
                    }))).await.is_err() {
                        return;
                    }
                }
                Event::Network(Err(error)) => {
                    emit_fatal_error(
                        &outgoing,
                        Some(&connection_id),
                        format!("TUNNEL_IO_ERROR: {error}"),
                    )
                    .await;
                    return;
                }
                Event::Client(None) => break,
                Event::Client(Some(value)) => {
                    match ClientFrame::parse(value, &connection_id, policy.max_frame_bytes) {
                        Ok(ClientFrame::Data(data)) => {
                            if let Err(code) =
                                charge_connection(&transferred, &rate, data.len(), policy)
                            {
                                emit_fatal_error(&outgoing, Some(&connection_id), code.to_string())
                                    .await;
                                return;
                            }
                            if !write_open {
                                emit_fatal_error(
                                    &outgoing,
                                    Some(&connection_id),
                                    "FRAME_SEQUENCE_ERROR: data after write half-close".to_string(),
                                )
                                .await;
                                return;
                            }
                            if let Err(error) = writer.write_all(&data).await {
                                emit_fatal_error(
                                    &outgoing,
                                    Some(&connection_id),
                                    format!("TUNNEL_IO_ERROR: {error}"),
                                )
                                .await;
                                return;
                            }
                        }
                        Ok(ClientFrame::HalfClose) => {
                            if write_open {
                                write_open = false;
                                let _ = writer.shutdown().await;
                            }
                            if !read_open {
                                break;
                            }
                        }
                        Ok(ClientFrame::Close) => break,
                        Err(error) => {
                            emit_fatal_error(&outgoing, Some(&connection_id), error.to_string())
                                .await;
                            return;
                        }
                    }
                }
            }
        }
        let _ = outgoing
            .send(BidiOutputFrame::terminal_json(json!({
                "type": "complete",
                "connection_id": connection_id,
                "bytes": transferred.load(Ordering::Acquire),
            })))
            .await;
    });
}

struct ListenerConnection {
    writer: OwnedWriteHalf,
    transferred: Arc<AtomicU64>,
    rate: Arc<Mutex<RateBudget>>,
    read_open: bool,
    write_open: bool,
}

enum ReaderEvent {
    ReadClosed(String),
    Failed(String),
}

fn spawn_listener(
    policy: TunnelPolicy,
    bind: SocketAddr,
    idle_timeout: Duration,
    mut incoming: mpsc::Receiver<Value>,
    outgoing: mpsc::Sender<BidiOutputFrame>,
) {
    tokio::spawn(async move {
        let listener = match TcpListener::bind(bind).await {
            Ok(listener) => listener,
            Err(error) => {
                emit_fatal_error(&outgoing, None, format!("BIND_FAILED: {error}")).await;
                return;
            }
        };
        let local_address = match listener.local_addr() {
            Ok(address) => address,
            Err(error) => {
                emit_fatal_error(&outgoing, None, format!("BIND_FAILED: {error}")).await;
                return;
            }
        };
        let listener_id = uuid::Uuid::new_v4().to_string();
        if outgoing
            .send(BidiOutputFrame::json(json!({
                "type": "listener_ready",
                "listener_id": listener_id,
                "bind_address": local_address.to_string(),
            })))
            .await
            .is_err()
        {
            return;
        }

        let (reader_event_tx, mut reader_events) = mpsc::channel(BIDI_CHANNEL_BOUND);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let mut connections: HashMap<String, ListenerConnection> = HashMap::new();
        let mut last_activity = tokio::time::Instant::now();
        let mut idle_tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream, peer) = match accepted {
                        Ok(value) => value,
                        Err(error) => {
                            emit_fatal_error(&outgoing, None, format!("TUNNEL_IO_ERROR: {error}")).await;
                            return;
                        }
                    };
                    if connections.len() >= policy.max_connections {
                        drop(stream);
                        emit_connection_error(&outgoing, None, "CONNECTION_LIMIT".to_string()).await;
                        continue;
                    }
                    let connection_id = uuid::Uuid::new_v4().to_string();
                    let (reader, writer) = stream.into_split();
                    let transferred = Arc::new(AtomicU64::new(0));
                    let rate = Arc::new(Mutex::new(RateBudget::new()));
                    connections.insert(connection_id.clone(), ListenerConnection {
                        writer,
                        transferred: Arc::clone(&transferred),
                        rate: Arc::clone(&rate),
                        read_open: true,
                        write_open: true,
                    });
                    spawn_listener_reader(
                        connection_id.clone(),
                        reader,
                        transferred,
                        rate,
                        policy,
                        cancel_rx.clone(),
                        outgoing.clone(),
                        reader_event_tx.clone(),
                    );
                    last_activity = tokio::time::Instant::now();
                    let _ = outgoing.send(BidiOutputFrame::json(json!({
                        "type": "accepted",
                        "listener_id": listener_id,
                        "connection_id": connection_id,
                        "peer_address": peer.to_string(),
                    }))).await;
                }
                frame = incoming.recv() => {
                    let Some(frame) = frame else { break; };
                    let object = match frame.as_object() {
                        Some(object) => object,
                        None => {
                            emit_fatal_error(&outgoing, None, "FRAME_SEQUENCE_ERROR: frame must be an object".to_string()).await;
                            return;
                        }
                    };
                    let connection_id = match required_string(object, "connection_id") {
                        Ok(value) => value.to_string(),
                        Err(error) => {
                            emit_fatal_error(&outgoing, None, error.to_string()).await;
                            return;
                        }
                    };
                    let Some(connection) = connections.get_mut(&connection_id) else {
                        emit_connection_error(&outgoing, Some(&connection_id), "FRAME_SEQUENCE_ERROR: unknown connection_id".to_string()).await;
                        continue;
                    };
                    match ClientFrame::parse(frame, &connection_id, policy.max_frame_bytes) {
                        Ok(ClientFrame::Data(data)) => {
                            if !connection.write_open {
                                emit_connection_error(&outgoing, Some(&connection_id), "FRAME_SEQUENCE_ERROR: data after write half-close".to_string()).await;
                                connections.remove(&connection_id);
                                continue;
                            }
                            if let Err(code) = charge_connection(
                                &connection.transferred,
                                &connection.rate,
                                data.len(),
                                policy,
                            ) {
                                emit_connection_error(&outgoing, Some(&connection_id), code.to_string()).await;
                                connections.remove(&connection_id);
                                continue;
                            }
                            if let Err(error) = connection.writer.write_all(&data).await {
                                emit_connection_error(&outgoing, Some(&connection_id), format!("TUNNEL_IO_ERROR: {error}")).await;
                                connections.remove(&connection_id);
                            }
                        }
                        Ok(ClientFrame::HalfClose) => {
                            if connection.write_open {
                                connection.write_open = false;
                                let _ = connection.writer.shutdown().await;
                            }
                            if !connection.read_open {
                                connections.remove(&connection_id);
                            }
                        }
                        Ok(ClientFrame::Close) => { connections.remove(&connection_id); }
                        Err(error) => emit_connection_error(&outgoing, Some(&connection_id), error.to_string()).await,
                    }
                    last_activity = tokio::time::Instant::now();
                }
                event = reader_events.recv() => {
                    match event {
                        Some(ReaderEvent::ReadClosed(connection_id)) => {
                            if let Some(connection) = connections.get_mut(&connection_id) {
                                connection.read_open = false;
                                if !connection.write_open {
                                    connections.remove(&connection_id);
                                }
                            }
                            last_activity = tokio::time::Instant::now();
                        }
                        Some(ReaderEvent::Failed(connection_id)) => {
                            connections.remove(&connection_id);
                            last_activity = tokio::time::Instant::now();
                        }
                        None => {}
                    }
                }
                _ = idle_tick.tick() => {
                    if last_activity.elapsed() >= idle_timeout {
                        emit_fatal_error(&outgoing, None, "IDLE_TIMEOUT".to_string()).await;
                        return;
                    }
                }
            }
        }
        let _ = cancel_tx.send(true);
        connections.clear();
        let _ = outgoing
            .send(BidiOutputFrame::terminal_json(json!({
                "type": "complete",
                "listener_id": listener_id,
            })))
            .await;
    });
}

fn spawn_listener_reader(
    connection_id: String,
    mut reader: OwnedReadHalf,
    transferred: Arc<AtomicU64>,
    rate: Arc<Mutex<RateBudget>>,
    policy: TunnelPolicy,
    mut cancel: tokio::sync::watch::Receiver<bool>,
    outgoing: mpsc::Sender<BidiOutputFrame>,
    events: mpsc::Sender<ReaderEvent>,
) {
    tokio::spawn(async move {
        let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
        loop {
            tokio::select! {
                read = reader.read(&mut buffer) => match read {
                    Ok(0) => {
                        let _ = outgoing.send(BidiOutputFrame::json(json!({
                            "type": "half_close",
                            "connection_id": connection_id,
                            "direction": "read",
                        }))).await;
                        let _ = events.send(ReaderEvent::ReadClosed(connection_id)).await;
                        break;
                    }
                    Ok(read) => {
                        if let Err(code) = charge_connection(&transferred, &rate, read, policy) {
                            emit_connection_error(&outgoing, Some(&connection_id), code.to_string()).await;
                            let _ = events.send(ReaderEvent::Failed(connection_id)).await;
                            break;
                        }
                        if outgoing.send(BidiOutputFrame::json(json!({
                            "type": "data",
                            "connection_id": connection_id,
                            "data": base64::engine::general_purpose::STANDARD.encode(&buffer[..read]),
                        }))).await.is_err() { break; }
                    }
                    Err(error) => {
                        emit_connection_error(&outgoing, Some(&connection_id), format!("TUNNEL_IO_ERROR: {error}")).await;
                        let _ = events.send(ReaderEvent::Failed(connection_id)).await;
                        break;
                    }
                },
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() { break; }
                }
            }
        }
    });
}

struct RateBudget {
    window_started: Instant,
    used: u64,
}

impl RateBudget {
    fn new() -> Self {
        Self {
            window_started: Instant::now(),
            used: 0,
        }
    }

    fn charge(&mut self, amount: u64, maximum: u64) -> bool {
        if self.window_started.elapsed() >= Duration::from_secs(1) {
            self.window_started = Instant::now();
            self.used = 0;
        }
        let Some(next) = self.used.checked_add(amount) else {
            return false;
        };
        if next > maximum {
            return false;
        }
        self.used = next;
        true
    }
}

fn charge_connection(
    transferred: &AtomicU64,
    rate: &Mutex<RateBudget>,
    amount: usize,
    policy: TunnelPolicy,
) -> Result<(), &'static str> {
    transferred
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(amount as u64)
                .filter(|next| *next <= policy.max_bytes_per_connection)
        })
        .map_err(|_| "BYTE_BUDGET_EXCEEDED")?;
    let mut rate = rate.lock().map_err(|_| "TUNNEL_STATE_ERROR")?;
    if !rate.charge(amount as u64, policy.max_rate_bytes_per_second) {
        return Err("RATE_LIMITED");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientFrame {
    Data(Vec<u8>),
    HalfClose,
    Close,
}

impl ClientFrame {
    fn parse(
        value: Value,
        expected_connection_id: &str,
        max_frame_bytes: usize,
    ) -> anyhow::Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("FRAME_SEQUENCE_ERROR: frame must be an object"))?;
        let frame_type = required_string(object, "type")?;
        let connection_id = required_string(object, "connection_id")?;
        if connection_id != expected_connection_id {
            anyhow::bail!("FRAME_SEQUENCE_ERROR: connection_id does not match active connection");
        }
        match frame_type {
            "data" => {
                reject_unknown(object, &["type", "connection_id", "data"])?;
                let data = required_string(object, "data")?;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|error| {
                        anyhow::anyhow!("FRAME_SEQUENCE_ERROR: invalid base64: {error}")
                    })?;
                if decoded.len() > max_frame_bytes {
                    anyhow::bail!("FRAME_SIZE_EXCEEDED: decoded data exceeds policy maximum");
                }
                Ok(Self::Data(decoded))
            }
            "half_close" => {
                reject_unknown(object, &["type", "connection_id", "direction"])?;
                if required_string(object, "direction")? != "write" {
                    anyhow::bail!("FRAME_SEQUENCE_ERROR: client may half-close write only");
                }
                Ok(Self::HalfClose)
            }
            "close" => {
                reject_unknown(object, &["type", "connection_id"])?;
                Ok(Self::Close)
            }
            other => anyhow::bail!("FRAME_SEQUENCE_ERROR: unsupported frame type {other:?}"),
        }
    }
}

async fn emit_fatal_error(
    outgoing: &mpsc::Sender<BidiOutputFrame>,
    connection_id: Option<&str>,
    error: String,
) {
    emit_error(outgoing, connection_id, error, true).await;
}

async fn emit_connection_error(
    outgoing: &mpsc::Sender<BidiOutputFrame>,
    connection_id: Option<&str>,
    error: String,
) {
    emit_error(outgoing, connection_id, error, false).await;
}

async fn emit_error(
    outgoing: &mpsc::Sender<BidiOutputFrame>,
    connection_id: Option<&str>,
    error: String,
    terminal: bool,
) {
    let (code, message) = match error.split_once(':') {
        Some((code, message)) => (code, message),
        None if error
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'_') =>
        {
            (error.as_str(), error.as_str())
        }
        None => ("TUNNEL_IO_ERROR", error.as_str()),
    };
    let value = json!({
        "type": "error",
        "connection_id": connection_id,
        "code": code.trim(),
        "message": message.trim(),
    });
    let frame = if terminal {
        BidiOutputFrame::terminal_json(value)
    } else {
        BidiOutputFrame::json(value)
    };
    let _ = outgoing.send(frame).await;
}

fn required_string<'a>(object: &'a Map<String, Value>, field: &str) -> anyhow::Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("FRAME_SEQUENCE_ERROR: {field} must be a non-empty string"))
}

fn required_port(object: &Map<String, Value>, field: &str) -> anyhow::Result<u16> {
    optional_port(object, field)?
        .filter(|port| *port > 0)
        .ok_or_else(|| anyhow::anyhow!("{field} must be between 1 and 65535"))
}

fn optional_port(object: &Map<String, Value>, field: &str) -> anyhow::Result<Option<u16>> {
    object
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or_else(|| anyhow::anyhow!("{field} must fit in u16"))
        })
        .transpose()
}

fn narrowed_timeout(
    object: &Map<String, Value>,
    field: &str,
    maximum: Duration,
) -> anyhow::Result<Duration> {
    let millis = object
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("{field} must be an unsigned integer"))
        })
        .transpose()?
        .unwrap_or(maximum.as_millis() as u64);
    let timeout = Duration::from_millis(millis);
    if timeout.is_zero() || timeout > maximum {
        anyhow::bail!("{field} must be between 1 and {}", maximum.as_millis());
    }
    Ok(timeout)
}

fn reject_unknown(object: &Map<String, Value>, allowed: &[&str]) -> anyhow::Result<()> {
    let mut unknown = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .collect::<Vec<_>>();
    unknown.sort();
    if !unknown.is_empty() {
        anyhow::bail!(
            "FRAME_SEQUENCE_ERROR: unsupported field(s): {}",
            unknown
                .into_iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["mode"],
        "additionalProperties": false,
        "properties": {
            "mode": {"type": "string", "enum": ["connect", "listen"]},
            "host": {"type": "string", "minLength": 1},
            "port": {"type": "integer", "minimum": 1, "maximum": 65535},
            "bind_host": {"type": "string", "minLength": 1},
            "bind_port": {"type": "integer", "minimum": 0, "maximum": 65535},
            "connect_timeout_ms": {"type": "integer", "minimum": 1, "maximum": MAX_CONNECT_TIMEOUT.as_millis() as u64},
            "idle_timeout_ms": {"type": "integer", "minimum": 1, "maximum": MAX_IDLE_TIMEOUT.as_millis() as u64},
        },
    })
}

pub fn description() -> &'static str {
    "Governed TCP connect or loopback listener over InvokeBidi with resolved-address policy, bounded connections, byte and rate budgets, idle timeout, and half-close semantics."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_mode_round_trips_bytes_through_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let bidi = open(
            json!({
                "mode": "connect",
                "host": "127.0.0.1",
                "port": address.port(),
                "idle_timeout_ms": 5_000,
            }),
            TunnelPolicy::baseline(),
        )
        .unwrap();
        let mut output = bidi.from_client;
        let connected = tokio::time::timeout(Duration::from_secs(2), output.recv())
            .await
            .unwrap()
            .unwrap()
            .into_json_value()
            .unwrap();
        assert_eq!(connected["type"], "connected");
        let connection_id = connected["connection_id"].as_str().unwrap().to_string();

        bidi.to_client
            .send(json!({
                "type": "data",
                "connection_id": connection_id,
                "data": base64::engine::general_purpose::STANDARD.encode(b"ping"),
            }))
            .await
            .unwrap();
        bidi.to_client
            .send(json!({
                "type": "half_close",
                "connection_id": connection_id,
                "direction": "write",
            }))
            .await
            .unwrap();

        let mut response = Vec::new();
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(2), output.recv())
                .await
                .unwrap()
                .unwrap()
                .into_json_value()
                .unwrap();
            match frame["type"].as_str().unwrap() {
                "data" => response.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(frame["data"].as_str().unwrap())
                        .unwrap(),
                ),
                "half_close" => {}
                "complete" => break,
                other => panic!("unexpected tunnel frame {other}: {frame}"),
            }
        }
        assert_eq!(response, b"pong");
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn connect_mode_idle_timeout_is_terminal() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let bidi = open(
            json!({
                "mode": "connect",
                "host": "127.0.0.1",
                "port": address.port(),
                "idle_timeout_ms": 10,
            }),
            TunnelPolicy::baseline(),
        )
        .unwrap();
        let mut output = bidi.from_client;
        assert_eq!(
            output.recv().await.unwrap().into_json_value().unwrap()["type"],
            "connected"
        );
        let terminal = tokio::time::timeout(Duration::from_secs(1), output.recv())
            .await
            .unwrap()
            .unwrap();
        let terminal = terminal.into_json_value().unwrap();
        assert_eq!(terminal["type"], "error");
        assert_eq!(terminal["code"], "IDLE_TIMEOUT");
        peer.abort();
    }

    #[tokio::test]
    async fn client_disconnect_closes_with_one_terminal_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let bidi = open(
            json!({
                "mode": "connect",
                "host": "127.0.0.1",
                "port": address.port(),
            }),
            TunnelPolicy::baseline(),
        )
        .unwrap();
        let mut output = bidi.from_client;
        assert_eq!(
            output.recv().await.unwrap().into_json_value().unwrap()["type"],
            "connected"
        );
        drop(bidi.to_client);
        let terminal = tokio::time::timeout(Duration::from_secs(1), output.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.into_json_value().unwrap()["type"], "complete");
        assert!(output.recv().await.is_none());
        peer.abort();
    }

    #[tokio::test]
    async fn policy_rejects_non_loopback_after_dns_resolution() {
        let error = TunnelPolicy::baseline()
            .resolve_connect("192.0.2.1", 80)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("DESTINATION_DENIED"));
    }

    #[test]
    fn listener_policy_rejects_non_loopback_bind() {
        assert!(TunnelOpen::parse(
            json!({
                "mode": "listen",
                "bind_host": "0.0.0.0",
                "bind_port": 0,
            }),
            TunnelPolicy::baseline()
        )
        .unwrap_err()
        .to_string()
        .contains("BIND_DENIED"));
    }

    #[test]
    fn client_frame_requires_exact_connection_id() {
        let error = ClientFrame::parse(
            json!({
                "type": "close",
                "connection_id": "wrong",
            }),
            "right",
            READ_CHUNK_BYTES,
        )
        .unwrap_err();
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn policy_rejects_rate_budget_smaller_than_one_frame() {
        let mut policy = TunnelPolicy::baseline();
        policy.max_frame_bytes = 1024;
        policy.max_rate_bytes_per_second = 1023;
        assert!(policy
            .validate()
            .unwrap_err()
            .to_string()
            .contains("at least one maximum-size frame"));
    }

    #[test]
    fn client_frame_rejects_decoded_payload_above_policy_limit() {
        let error = ClientFrame::parse(
            json!({
                "type": "data",
                "connection_id": "connection",
                "data": base64::engine::general_purpose::STANDARD.encode(b"too-large"),
            }),
            "connection",
            4,
        )
        .unwrap_err();
        assert!(error.to_string().contains("FRAME_SIZE_EXCEEDED"));
    }

    #[test]
    fn connection_rate_budget_fails_closed() {
        let mut policy = TunnelPolicy::baseline();
        policy.max_frame_bytes = 4;
        policy.max_rate_bytes_per_second = 4;
        let policy = policy.validate().unwrap();
        let transferred = AtomicU64::new(0);
        let rate = Mutex::new(RateBudget::new());

        charge_connection(&transferred, &rate, 3, policy).unwrap();
        assert_eq!(
            charge_connection(&transferred, &rate, 2, policy),
            Err("RATE_LIMITED")
        );
    }
}
