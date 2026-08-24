// EasyNet CLI - governed device tunnel adapters
// =============================================
//
// File: src/cli/commands/device_tunnel.rs
// Description: Adapt local TCP forwarding and SOCKS5 CONNECT onto the
//              descriptor-bound net.tunnel InvokeBidi ability.
//
// Security Contract
// -----------------
// CLI listeners bind loopback only. Reverse-forward local destinations are
// resolved and revalidated as loopback. Remote destination and bind policy is
// independently enforced after DNS resolution by the target daemon.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::Context;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};

const TUNNEL_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const TCP_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForwardSpec {
    pub(crate) bind: SocketAddr,
    pub(crate) destination_host: String,
    pub(crate) destination_port: u16,
}

impl ForwardSpec {
    pub(crate) fn parse(raw: &str) -> anyhow::Result<Self> {
        let fields = split_forward_fields(raw)?;
        let (bind_host, bind_port, destination_host, destination_port) = match fields.as_slice() {
            [bind_port, destination_host, destination_port] => (
                "127.0.0.1",
                parse_port(bind_port, "listen port")?,
                destination_host.as_str(),
                parse_port(destination_port, "destination port")?,
            ),
            [bind_host, bind_port, destination_host, destination_port] => (
                bind_host.as_str(),
                parse_port(bind_port, "listen port")?,
                destination_host.as_str(),
                parse_port(destination_port, "destination port")?,
            ),
            _ => anyhow::bail!("INVALID_FORWARD_SPEC: expected [bind_address:]port:host:hostport"),
        };
        let bind_ip = bind_host.parse::<IpAddr>().map_err(|_| {
            anyhow::anyhow!("BIND_DENIED: bind address must be a literal loopback IP")
        })?;
        if !bind_ip.is_loopback() {
            anyhow::bail!("BIND_DENIED: forwarding listeners are loopback-only");
        }
        if destination_host.trim().is_empty() {
            anyhow::bail!("INVALID_FORWARD_SPEC: destination host must not be empty");
        }
        Ok(Self {
            bind: SocketAddr::new(bind_ip, bind_port),
            destination_host: destination_host.to_string(),
            destination_port,
        })
    }
}

fn split_forward_fields(raw: &str) -> anyhow::Result<Vec<String>> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut bracketed = false;
    for character in raw.trim().chars() {
        match character {
            '[' if !bracketed => bracketed = true,
            ']' if bracketed => bracketed = false,
            ':' if !bracketed => {
                fields.push(std::mem::take(&mut current));
            }
            _ => current.push(character),
        }
    }
    if bracketed {
        anyhow::bail!("INVALID_FORWARD_SPEC: unterminated IPv6 brackets");
    }
    fields.push(current);
    if fields.iter().any(|field| field.trim().is_empty()) {
        anyhow::bail!("INVALID_FORWARD_SPEC: fields must not be empty");
    }
    Ok(fields)
}

fn parse_port(raw: &str, label: &str) -> anyhow::Result<u16> {
    raw.parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| anyhow::anyhow!("INVALID_FORWARD_SPEC: {label} must be 1..65535"))
}

pub(crate) fn run_forward(
    device: &str,
    local: Option<&str>,
    remote: Option<&str>,
) -> anyhow::Result<()> {
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (device, local, remote);
        Err(
            crate::support::platform::local_invoke::federation_capability_unsupported_error(
                "opening a governed remote TCP forward",
            ),
        )
    }
    #[cfg(feature = "axon-pb")]
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("build runtime for device forward")?;
        let client = TunnelClient::load(device)?;
        match (local, remote) {
            (Some(spec), None) => {
                runtime.block_on(run_local_forward(client, ForwardSpec::parse(spec)?))
            }
            (None, Some(spec)) => {
                runtime.block_on(run_remote_forward(client, ForwardSpec::parse(spec)?))
            }
            _ => anyhow::bail!("exactly one of -L/--local or -R/--remote is required"),
        }
    }
}

pub(crate) fn run_socks(device: &str, listen: SocketAddr) -> anyhow::Result<()> {
    if !listen.ip().is_loopback() {
        anyhow::bail!("BIND_DENIED: SOCKS listener must be loopback-only");
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = device;
        Err(
            crate::support::platform::local_invoke::federation_capability_unsupported_error(
                "opening a governed remote SOCKS5 adapter",
            ),
        )
    }
    #[cfg(feature = "axon-pb")]
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("build runtime for device socks")?;
        runtime.block_on(run_socks_listener(TunnelClient::load(device)?, listen))
    }
}

pub(crate) fn run_stdio_proxy(device: &str, host: &str, port: u16) -> anyhow::Result<()> {
    if port == 0 {
        anyhow::bail!("port must be between 1 and 65535");
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (device, host, port);
        Err(
            crate::support::platform::local_invoke::federation_capability_unsupported_error(
                "opening an OpenSSH ProxyCommand tunnel",
            ),
        )
    }
    #[cfg(feature = "axon-pb")]
    {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("build runtime for device proxy")?;
        runtime.block_on(run_stdio_tunnel(TunnelClient::load(device)?, host, port))
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Clone)]
struct TunnelClient {
    target_ura: String,
    caller_ura: String,
    signer: crate::daemon::invocation::routing::remote_invoke::RemoteInvocationCallerSigner,
}

#[cfg(feature = "axon-pb")]
impl TunnelClient {
    fn load(device: &str) -> anyhow::Result<Self> {
        let identity = crate::support::platform::remote_device::PairedInvocationIdentity::load(
            "device tunnel",
        )?;
        let target_ura =
            crate::support::platform::remote_device::resolve_target_device_ura(device)?;
        if target_ura == identity.local_device_ura() {
            anyhow::bail!("INVALID_TARGET: device tunnel requires a remote Device");
        }
        let caller_ura = identity.caller_user_ura().to_string();
        let signer =
            crate::daemon::invocation::routing::remote_invoke::load_remote_invocation_caller_signer(
                &caller_ura,
            )
            .context("prepare device tunnel caller signer")?;
        Ok(Self {
            target_ura,
            caller_ura,
            signer,
        })
    }

    async fn open(&self, args: Value) -> anyhow::Result<TunnelSession> {
        let subject_ura = crate::core::ura::device_ephemeral_resource_ura(
            &self.target_ura,
            "network-tunnel",
            &uuid::Uuid::new_v4().to_string(),
        )
        .ok_or_else(|| anyhow::anyhow!("derive network tunnel Resource subject"))?;
        let session = crate::cli::daemon_client::remote_system_ability::open_remote_net_tunnel(
            &self.target_ura,
            &self.caller_ura,
            &subject_ura,
            args,
            std::sync::Arc::clone(&self.signer),
            TUNNEL_TIMEOUT,
        )
        .await?;
        Ok(TunnelSession::new(session))
    }

    async fn connect(&self, host: &str, port: u16) -> anyhow::Result<ConnectedTunnel> {
        let mut session = self
            .open(json!({"mode": "connect", "host": host, "port": port}))
            .await?;
        let first = session
            .recv()
            .await?
            .ok_or_else(|| anyhow::anyhow!("TUNNEL_INTERRUPTED: missing connected frame"))?;
        match first["type"].as_str() {
            Some("connected") => {
                let connection_id = required_frame_string(&first, "connection_id")?.to_string();
                Ok(ConnectedTunnel {
                    session,
                    connection_id,
                })
            }
            Some("error") => Err(tunnel_error(&first)),
            other => anyhow::bail!("TUNNEL_PROTOCOL_ERROR: expected connected, got {other:?}"),
        }
    }
}

#[cfg(feature = "axon-pb")]
struct TunnelSession {
    sender: crate::support::platform::bidi_session::DaemonBidiSender,
    receiver: TunnelReceiver,
}

#[cfg(feature = "axon-pb")]
struct TunnelReceiver {
    inner: crate::support::platform::bidi_session::DaemonBidiReceiver,
}

#[cfg(feature = "axon-pb")]
impl TunnelSession {
    fn new(session: crate::support::platform::bidi_session::DaemonBidiSession) -> Self {
        let (sender, receiver) = session.split();
        Self {
            sender,
            receiver: TunnelReceiver { inner: receiver },
        }
    }

    async fn send(&mut self, value: Value) -> anyhow::Result<()> {
        self.sender.send_json(&value).await
    }

    async fn recv(&mut self) -> anyhow::Result<Option<Value>> {
        self.receiver.recv().await
    }
}

#[cfg(feature = "axon-pb")]
impl TunnelReceiver {
    async fn recv(&mut self) -> anyhow::Result<Option<Value>> {
        loop {
            let Some(frame) = self.inner.recv().await? else {
                return Ok(None);
            };
            if let Some(payload) = project_tunnel_frame(frame.payload, frame.terminal)? {
                return Ok(Some(payload));
            }
        }
    }
}

fn project_tunnel_frame(payload: Value, terminal: bool) -> anyhow::Result<Option<Value>> {
    if payload.get("type").and_then(Value::as_str) != Some("receipt") {
        return Ok(Some(payload));
    }
    if !terminal {
        return Ok(None);
    }
    if let Some(failure) = payload.get("failure").filter(|failure| !failure.is_null()) {
        let code = failure["code"].as_str().unwrap_or("TUNNEL_IO_ERROR");
        let message = failure["message"].as_str().unwrap_or("tunnel failed");
        anyhow::bail!("{code}: {message}");
    }
    let business = payload.get("payload").cloned().unwrap_or(Value::Null);
    Ok((!business.is_null()).then_some(business))
}

#[cfg(feature = "axon-pb")]
struct ConnectedTunnel {
    session: TunnelSession,
    connection_id: String,
}

#[cfg(feature = "axon-pb")]
async fn run_local_forward(client: TunnelClient, spec: ForwardSpec) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(spec.bind)
        .await
        .with_context(|| format!("bind local forward listener {}", spec.bind))?;
    eprintln!(
        "forwarding {} -> {}:{} through {}",
        listener.local_addr()?,
        spec.destination_host,
        spec.destination_port,
        client.target_ura,
    );
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let client = client.clone();
                let host = spec.destination_host.clone();
                let port = spec.destination_port;
                tokio::spawn(async move {
                    if let Err(error) = bridge_local_stream(client, stream, &host, port).await {
                        eprintln!("forward connection {peer} closed: {error:#}");
                    }
                });
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("wait for Ctrl-C")?;
                return Ok(());
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
async fn bridge_local_stream(
    client: TunnelClient,
    stream: tokio::net::TcpStream,
    destination_host: &str,
    destination_port: u16,
) -> anyhow::Result<()> {
    let tunnel = client.connect(destination_host, destination_port).await?;
    bridge_connected_stream(stream, tunnel).await
}

#[cfg(feature = "axon-pb")]
async fn bridge_connected_stream(
    stream: tokio::net::TcpStream,
    tunnel: ConnectedTunnel,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let ConnectedTunnel {
        session,
        connection_id,
    } = tunnel;
    let TunnelSession {
        mut sender,
        mut receiver,
    } = session;
    let (mut local_reader, mut local_writer) = stream.into_split();
    let upstream_connection_id = connection_id.clone();
    let upstream = tokio::spawn(async move {
        let mut buffer = vec![0_u8; TCP_CHUNK_BYTES];
        loop {
            let read = local_reader.read(&mut buffer).await?;
            if read == 0 {
                sender
                    .send_json(&json!({
                        "type": "half_close",
                        "connection_id": upstream_connection_id,
                        "direction": "write",
                    }))
                    .await?;
                return Ok::<_, anyhow::Error>(());
            }
            sender
                .send_json(&json!({
                    "type": "data",
                    "connection_id": upstream_connection_id,
                    "data": BASE64_STANDARD.encode(&buffer[..read]),
                }))
                .await?;
        }
    });

    while let Some(frame) = receiver.recv().await? {
        match frame["type"].as_str() {
            Some("data") => {
                local_writer.write_all(&decode_data_frame(&frame)?).await?;
            }
            Some("half_close") => local_writer.shutdown().await?,
            Some("complete") => break,
            Some("error") => return Err(tunnel_error(&frame)),
            other => anyhow::bail!("TUNNEL_PROTOCOL_ERROR: unexpected frame {other:?}"),
        }
    }
    upstream.abort();
    match upstream.await {
        Ok(result) => result,
        Err(error) if error.is_cancelled() => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "axon-pb")]
async fn run_stdio_tunnel(client: TunnelClient, host: &str, port: u16) -> anyhow::Result<()> {
    use std::io::{Read, Write};

    let ConnectedTunnel {
        session,
        connection_id,
    } = client.connect(host, port).await?;
    let TunnelSession {
        mut sender,
        mut receiver,
    } = session;
    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    let (stdout_tx, stdout_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(32);
    let mut stdout_tx = Some(stdout_tx);
    std::thread::Builder::new()
        .name("easynet-proxy-stdin".to_string())
        .spawn(move || {
            let mut stdin = std::io::stdin().lock();
            let mut buffer = vec![0_u8; TCP_CHUNK_BYTES];
            loop {
                match stdin.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) if stdin_tx.blocking_send(buffer[..read].to_vec()).is_err() => break,
                    Ok(_) => {}
                }
            }
        })?;
    let stdout_thread = std::thread::Builder::new()
        .name("easynet-proxy-stdout".to_string())
        .spawn(move || -> std::io::Result<()> {
            let mut stdout = std::io::stdout().lock();
            while let Ok(bytes) = stdout_rx.recv() {
                stdout.write_all(&bytes)?;
                stdout.flush()?;
            }
            Ok(())
        })?;
    let mut stdin_open = true;
    loop {
        tokio::select! {
            bytes = stdin_rx.recv(), if stdin_open => {
                match bytes {
                    Some(bytes) => sender.send_json(&json!({
                        "type": "data",
                        "connection_id": connection_id,
                        "data": BASE64_STANDARD.encode(bytes),
                    })).await?,
                    None => {
                        stdin_open = false;
                        sender.send_json(&json!({
                            "type": "half_close",
                            "connection_id": connection_id,
                            "direction": "write",
                        })).await?;
                    }
                }
            }
            frame = receiver.recv() => {
                let Some(frame) = frame? else { break; };
                match frame["type"].as_str() {
                    Some("data") => stdout_tx
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("TUNNEL_PROTOCOL_ERROR: data after remote half-close"))?
                        .send(decode_data_frame(&frame)?)
                        .map_err(|_| anyhow::anyhow!("OpenSSH proxy stdout closed"))?,
                    Some("half_close") => {
                        // Propagate remote TCP FIN to OpenSSH without ending the
                        // tunnel Invocation. Its stdin half may still drain;
                        // `complete` remains the receipt-backed terminal frame.
                        stdout_tx.take();
                    }
                    Some("complete") => break,
                    Some("error") => return Err(tunnel_error(&frame)),
                    other => anyhow::bail!("TUNNEL_PROTOCOL_ERROR: unexpected frame {other:?}"),
                }
            }
        }
    }
    drop(stdout_tx.take());
    stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("OpenSSH proxy stdout thread panicked"))??;
    Ok(())
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
enum ReverseCommand {
    Data {
        connection_id: String,
        data: Vec<u8>,
    },
    HalfClose {
        connection_id: String,
    },
    Failed {
        connection_id: String,
    },
}

#[cfg(feature = "axon-pb")]
async fn run_remote_forward(client: TunnelClient, spec: ForwardSpec) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let local_destination =
        resolve_loopback_destination(&spec.destination_host, spec.destination_port).await?;
    let mut tunnel = client
        .open(json!({
            "mode": "listen",
            "bind_host": spec.bind.ip().to_string(),
            "bind_port": spec.bind.port(),
        }))
        .await?;
    let ready = tunnel
        .recv()
        .await?
        .ok_or_else(|| anyhow::anyhow!("TUNNEL_INTERRUPTED: missing listener_ready frame"))?;
    if ready["type"] == "error" {
        return Err(tunnel_error(&ready));
    }
    if ready["type"] != "listener_ready" {
        anyhow::bail!("TUNNEL_PROTOCOL_ERROR: expected listener_ready");
    }
    eprintln!(
        "forwarding {} on {} -> {}",
        required_frame_string(&ready, "bind_address")?,
        client.target_ura,
        local_destination,
    );

    let (commands_tx, mut commands_rx) = tokio::sync::mpsc::channel(64);
    let mut writers = std::collections::HashMap::new();
    loop {
        tokio::select! {
            frame = tunnel.recv() => {
                let Some(frame) = frame? else { return Ok(()); };
                match frame["type"].as_str() {
                    Some("accepted") => {
                        let connection_id = required_frame_string(&frame, "connection_id")?.to_string();
                        match tokio::net::TcpStream::connect(local_destination).await {
                            Ok(stream) => {
                                let (mut reader, writer) = stream.into_split();
                                writers.insert(connection_id.clone(), writer);
                                let commands = commands_tx.clone();
                                tokio::spawn(async move {
                                    let mut buffer = vec![0_u8; TCP_CHUNK_BYTES];
                                    loop {
                                        match reader.read(&mut buffer).await {
                                            Ok(0) => {
                                                let _ = commands.send(ReverseCommand::HalfClose { connection_id }).await;
                                                break;
                                            }
                                            Ok(read) => {
                                                if commands.send(ReverseCommand::Data {
                                                    connection_id: connection_id.clone(),
                                                    data: buffer[..read].to_vec(),
                                                }).await.is_err() { break; }
                                            }
                                            Err(_) => {
                                                let _ = commands.send(ReverseCommand::Failed { connection_id }).await;
                                                break;
                                            }
                                        }
                                    }
                                });
                            }
                            Err(error) => {
                                eprintln!("reverse destination {local_destination} rejected: {error}");
                                tunnel.send(json!({"type": "close", "connection_id": connection_id})).await?;
                            }
                        }
                    }
                    Some("data") => {
                        let connection_id = required_frame_string(&frame, "connection_id")?;
                        if let Some(writer) = writers.get_mut(connection_id) {
                            if writer.write_all(&decode_data_frame(&frame)?).await.is_err() {
                                writers.remove(connection_id);
                            }
                        }
                    }
                    Some("half_close") => {
                        let connection_id = required_frame_string(&frame, "connection_id")?;
                        if let Some(writer) = writers.get_mut(connection_id) {
                            let _ = writer.shutdown().await;
                        }
                    }
                    Some("error") => {
                        if let Some(connection_id) = frame["connection_id"].as_str() {
                            writers.remove(connection_id);
                            eprintln!("reverse connection {connection_id} closed: {}", tunnel_error(&frame));
                        } else {
                            return Err(tunnel_error(&frame));
                        }
                    }
                    Some("complete") => return Ok(()),
                    other => anyhow::bail!("TUNNEL_PROTOCOL_ERROR: unexpected frame {other:?}"),
                }
            }
            command = commands_rx.recv() => {
                let Some(command) = command else { continue; };
                match command {
                    ReverseCommand::Data { connection_id, data } => {
                        tunnel.send(json!({
                            "type": "data",
                            "connection_id": connection_id,
                            "data": BASE64_STANDARD.encode(data),
                        })).await?;
                    }
                    ReverseCommand::HalfClose { connection_id } => {
                        tunnel.send(json!({
                            "type": "half_close",
                            "connection_id": connection_id,
                            "direction": "write",
                        })).await?;
                    }
                    ReverseCommand::Failed { connection_id } => {
                        writers.remove(&connection_id);
                        tunnel.send(json!({"type": "close", "connection_id": connection_id})).await?;
                    }
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("wait for Ctrl-C")?;
                return Ok(());
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
async fn resolve_loopback_destination(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("resolve reverse-forward destination {host}:{port}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !address.ip().is_loopback()) {
        anyhow::bail!(
            "DESTINATION_DENIED: reverse-forward destination must resolve only to loopback"
        );
    }
    Ok(addresses[0])
}

#[cfg(feature = "axon-pb")]
async fn run_socks_listener(client: TunnelClient, listen: SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    eprintln!(
        "SOCKS5 listening on {} through {} (CONNECT, no-auth, target policy applies)",
        listener.local_addr()?,
        client.target_ura,
    );
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let client = client.clone();
                tokio::spawn(async move {
                    if let Err(error) = serve_socks_connection(client, stream).await {
                        eprintln!("SOCKS5 connection {peer} closed: {error:#}");
                    }
                });
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("wait for Ctrl-C")?;
                return Ok(());
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
async fn serve_socks_connection(
    client: TunnelClient,
    mut stream: tokio::net::TcpStream,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut greeting = [0_u8; 2];
    stream.read_exact(&mut greeting).await?;
    if greeting[0] != 5 || greeting[1] == 0 {
        anyhow::bail!("SOCKS_PROTOCOL_ERROR: expected SOCKS5 methods");
    }
    let mut methods = vec![0_u8; greeting[1] as usize];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        stream.write_all(&[5, 0xff]).await?;
        anyhow::bail!("SOCKS_AUTH_UNSUPPORTED: client did not offer no-auth");
    }
    stream.write_all(&[5, 0]).await?;

    let mut request = [0_u8; 4];
    stream.read_exact(&mut request).await?;
    if request[0] != 5 || request[1] != 1 || request[2] != 0 {
        write_socks_reply(&mut stream, 7).await?;
        anyhow::bail!("SOCKS_COMMAND_UNSUPPORTED: only CONNECT is supported");
    }
    let host = match request[3] {
        1 => {
            let mut bytes = [0_u8; 4];
            stream.read_exact(&mut bytes).await?;
            IpAddr::from(bytes).to_string()
        }
        3 => {
            let length = stream.read_u8().await? as usize;
            let mut bytes = vec![0_u8; length];
            stream.read_exact(&mut bytes).await?;
            String::from_utf8(bytes).context("SOCKS_PROTOCOL_ERROR: domain is not UTF-8")?
        }
        4 => {
            let mut bytes = [0_u8; 16];
            stream.read_exact(&mut bytes).await?;
            IpAddr::from(bytes).to_string()
        }
        other => {
            write_socks_reply(&mut stream, 8).await?;
            anyhow::bail!("SOCKS_ADDRESS_UNSUPPORTED: address type {other}");
        }
    };
    let port = stream.read_u16().await?;
    let tunnel = match client.connect(&host, port).await {
        Ok(tunnel) => tunnel,
        Err(error) => {
            write_socks_reply(&mut stream, 2).await?;
            return Err(error);
        }
    };
    write_socks_reply(&mut stream, 0).await?;
    bridge_connected_stream(stream, tunnel).await
}

#[cfg(feature = "axon-pb")]
async fn write_socks_reply(stream: &mut tokio::net::TcpStream, reply: u8) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    stream.write_all(&[5, reply, 0, 1, 0, 0, 0, 0, 0, 0]).await
}

fn required_frame_string<'a>(frame: &'a Value, field: &str) -> anyhow::Result<&'a str> {
    frame
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("TUNNEL_PROTOCOL_ERROR: frame omitted {field}"))
}

fn decode_data_frame(frame: &Value) -> anyhow::Result<Vec<u8>> {
    BASE64_STANDARD
        .decode(required_frame_string(frame, "data")?)
        .context("TUNNEL_PROTOCOL_ERROR: invalid base64 data")
}

fn tunnel_error(frame: &Value) -> anyhow::Error {
    let code = frame["code"].as_str().unwrap_or("TUNNEL_IO_ERROR");
    let message = frame["message"].as_str().unwrap_or("tunnel failed");
    anyhow::anyhow!("{code}: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ssh_style_forward_specs() {
        assert_eq!(
            ForwardSpec::parse("8080:localhost:80").unwrap(),
            ForwardSpec {
                bind: "127.0.0.1:8080".parse().unwrap(),
                destination_host: "localhost".to_string(),
                destination_port: 80,
            }
        );
        assert_eq!(
            ForwardSpec::parse("[::1]:8080:[::1]:80").unwrap().bind,
            "[::1]:8080".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn rejects_non_loopback_listeners() {
        let error = ForwardSpec::parse("0.0.0.0:8080:localhost:80").unwrap_err();
        assert!(error.to_string().contains("BIND_DENIED"));
    }

    #[test]
    fn stdio_proxy_rejects_zero_port_before_identity_or_network_access() {
        let error = run_stdio_proxy("easynet:///r/test/device/target", "127.0.0.1", 0)
            .expect_err("zero port must fail closed");
        assert!(error.to_string().contains("between 1 and 65535"));
    }

    #[test]
    fn tunnel_projection_skips_admission_and_unwraps_terminal_payload() {
        assert!(project_tunnel_frame(
            json!({"type": "receipt", "state": 2, "payload": null}),
            false,
        )
        .expect("admission receipt")
        .is_none());

        let terminal = project_tunnel_frame(
            json!({
                "type": "receipt",
                "state": 3,
                "failure": null,
                "payload": {"type": "complete", "bytes_in": 3, "bytes_out": 4},
            }),
            true,
        )
        .expect("terminal receipt")
        .expect("terminal business payload");
        assert_eq!(terminal["type"], "complete");
        assert_eq!(terminal["bytes_out"], 4);
    }

    #[test]
    fn tunnel_projection_surfaces_terminal_receipt_failure() {
        let error = project_tunnel_frame(
            json!({
                "type": "receipt",
                "failure": {"code": "IDLE_TIMEOUT", "message": "idle"},
                "payload": null,
            }),
            true,
        )
        .expect_err("terminal failure must fail closed");
        assert_eq!(error.to_string(), "IDLE_TIMEOUT: idle");
    }
}
