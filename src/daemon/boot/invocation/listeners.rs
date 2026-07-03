use std::time::Duration;

use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;
use tonic::transport::{Identity, Server, ServerTlsConfig};

#[cfg(windows)]
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(any(unix, windows))]
use tokio_stream::wrappers::ReceiverStream;
#[cfg(windows)]
use tonic::transport::server::Connected;

#[cfg(unix)]
use super::local_peer::{LocalPeerGate, PeerGateError};
use super::paths::expand_home;
use super::MAX_INVOCATION_GRPC_MESSAGE_BYTES;
use crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use crate::daemon::persistence::daemon_config::DaemonConfig;
#[cfg(windows)]
use crate::support::platform::named_pipe::PipeListener;

const DEFAULT_INVOCATION_ACCEPT_QUEUE_CAPACITY: usize = 10_000;
const DEFAULT_INVOCATION_MAX_CONCURRENT_STREAMS: u32 = 10_000;
const DEFAULT_INVOCATION_CONCURRENCY_LIMIT_PER_CONNECTION: usize = 10_000;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn invocation_accept_queue_capacity() -> usize {
    env_usize(
        "EASYNET_INVOCATION_ACCEPT_QUEUE_CAPACITY",
        DEFAULT_INVOCATION_ACCEPT_QUEUE_CAPACITY,
    )
}

fn invocation_max_concurrent_streams() -> u32 {
    env_u32(
        "EASYNET_INVOCATION_MAX_CONCURRENT_STREAMS",
        DEFAULT_INVOCATION_MAX_CONCURRENT_STREAMS,
    )
}

fn invocation_concurrency_limit_per_connection() -> usize {
    env_usize(
        "EASYNET_INVOCATION_CONCURRENCY_LIMIT_PER_CONNECTION",
        DEFAULT_INVOCATION_CONCURRENCY_LIMIT_PER_CONNECTION,
    )
}

#[cfg(windows)]
#[derive(Debug)]
struct NamedPipeGrpcIo(NamedPipeServer);

#[cfg(windows)]
impl Connected for NamedPipeGrpcIo {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

#[cfg(windows)]
impl AsyncRead for NamedPipeGrpcIo {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

#[cfg(windows)]
impl AsyncWrite for NamedPipeGrpcIo {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(unix)]
pub(super) fn spawn_uds_listener(
    config: &DaemonConfig,
    service: DaemonInvocationService,
) -> anyhow::Result<()> {
    let uds_path = expand_home(
        config
            .uds_path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("daemon-config uds_path is not valid UTF-8"))?,
    );

    if uds_path.exists() {
        // The existing daemon's control.sock bind code unlinks
        // before binding; mirror that semantic so a previous
        // process's stale daemon.sock does not block us.
        if let Err(err) = std::fs::remove_file(&uds_path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                let uds_path_display = format!("{}", uds_path.display());
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = daemon_invocation,
                    kind = uds_unlink_failed,
                    uds_path = uds_path_display,
                    error = err_msg,
                    message = "bind will likely fail",
                );
            }
        }
    }

    let listener = tokio::net::UnixListener::bind(&uds_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to bind daemon Invocation UDS at {}: {err}",
            uds_path.display()
        )
    })?;

    // Mode 0600 per spec §1.2 Invariant 3. UnixListener::bind already
    // creates the file; chmod after-the-fact rather than racing the
    // bind. A failure here is a soft warning (the file is owned by
    // the same user that just bound it; mode 0600 vs 0644 is a
    // hardening detail, not a correctness one).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) =
            std::fs::set_permissions(&uds_path, std::fs::Permissions::from_mode(0o600))
        {
            let uds_path_display = format!("{}", uds_path.display());
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = uds_chmod_failed,
                uds_path = uds_path_display,
                error = err_msg,
                message = "running with default umask perms",
            );
        }
    }

    let uds_path_display = format!("{}", uds_path.display());
    crate::op_event!(
        component = daemon_invocation,
        kind = grpc_invocation_server_listening,
        transport = "uds",
        uds_path = uds_path_display,
        accept_queue_capacity = invocation_accept_queue_capacity(),
        max_concurrent_streams = invocation_max_concurrent_streams(),
        concurrency_limit_per_connection = invocation_concurrency_limit_per_connection(),
    );

    let peer_gate = LocalPeerGate::for_current_process();
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<tokio::net::UnixStream>>(
        invocation_accept_queue_capacity(),
    );
    let accept_uds_path = uds_path_display.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => match peer_gate.authorize_stream(&stream) {
                    Ok(_credential) => {
                        if tx.send(Ok(stream)).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        log_uds_peer_rejection(&accept_uds_path, &err);
                    }
                },
                Err(err) => {
                    let _ = tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });

    let incoming = ReceiverStream::new(rx);
    tokio::spawn(async move {
        let result = Server::builder()
            // UDS is loopback-only; keepalive is purely defensive
            // for symmetry with the TCP+TLS listener below. Same
            // 5s ping cadence as the TCP+TLS server so behaviour
            // is uniform across listener types.
            .http2_keepalive_interval(Some(Duration::from_secs(5)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .tcp_keepalive(Some(Duration::from_secs(15)))
            .max_concurrent_streams(Some(invocation_max_concurrent_streams()))
            .concurrency_limit_per_connection(invocation_concurrency_limit_per_connection())
            .add_service(
                InvocationServer::new(service)
                    .max_decoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES),
            )
            .serve_with_incoming(incoming)
            .await;
        if let Err(err) = result {
            let err_msg = format!("{err:#}");
            crate::op_event!(
                component = daemon_invocation,
                kind = grpc_server_exited_with_error,
                transport = "uds",
                error = err_msg,
            );
        }
    });

    Ok(())
}

#[cfg(unix)]
fn log_uds_peer_rejection(uds_path: &str, err: &PeerGateError) {
    let err_msg = format!("{err}");
    if let Some(credential) = err.credential() {
        let peer_uid = credential.uid.to_string();
        let peer_gid = credential.gid.to_string();
        let peer_pid = credential
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        crate::op_event!(
            component = daemon_invocation,
            kind = uds_peer_credential_rejected,
            transport = "uds",
            uds_path = uds_path,
            peer_uid = peer_uid,
            peer_gid = peer_gid,
            peer_pid = peer_pid,
            error = err_msg,
            message = "dropping unauthorized daemon.sock connection",
        );
    } else {
        crate::op_event!(
            component = daemon_invocation,
            kind = uds_peer_credential_unreadable,
            transport = "uds",
            uds_path = uds_path,
            error = err_msg,
            message = "dropping daemon.sock connection without OS peer credentials",
        );
    }
}

#[cfg(windows)]
pub(super) fn spawn_uds_listener(
    config: &DaemonConfig,
    service: DaemonInvocationService,
) -> anyhow::Result<()> {
    let pipe_name = config
        .uds_path()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("daemon-config named-pipe path is not valid UTF-8"))?
        .to_string();
    let mut listener = PipeListener::bind(pipe_name.clone()).map_err(|err| {
        anyhow::anyhow!(
            "failed to bind daemon Invocation named pipe {}: {err}",
            pipe_name
        )
    })?;

    let pipe_name_log = pipe_name.clone();
    crate::op_event!(
        component = daemon_invocation,
        kind = grpc_invocation_server_listening,
        transport = "named_pipe",
        pipe_name = pipe_name_log,
        accept_queue_capacity = invocation_accept_queue_capacity(),
        max_concurrent_streams = invocation_max_concurrent_streams(),
        concurrency_limit_per_connection = invocation_concurrency_limit_per_connection(),
    );

    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<NamedPipeGrpcIo>>(
        invocation_accept_queue_capacity(),
    );
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    if tx.send(Ok(NamedPipeGrpcIo(stream))).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });

    let incoming = ReceiverStream::new(rx);
    tokio::spawn(async move {
        let result = Server::builder()
            .http2_keepalive_interval(Some(Duration::from_secs(5)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .tcp_keepalive(Some(Duration::from_secs(15)))
            .max_concurrent_streams(Some(invocation_max_concurrent_streams()))
            .concurrency_limit_per_connection(invocation_concurrency_limit_per_connection())
            .add_service(
                InvocationServer::new(service)
                    .max_decoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES),
            )
            .serve_with_incoming(incoming)
            .await;
        if let Err(err) = result {
            let err_msg = format!("{err:#}");
            crate::op_event!(
                component = daemon_invocation,
                kind = grpc_server_exited_with_error,
                transport = "named_pipe",
                error = err_msg,
            );
        }
    });

    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(super) fn spawn_uds_listener(
    _config: &DaemonConfig,
    _service: DaemonInvocationService,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "daemon Invocation local listener is unavailable on this platform until the local transport \
         backend lands"
    )
}

/// Spawn the hub-mode TCP+TLS gRPC listener (PR-10 commit 1/N).
/// `DaemonConfig` already enforces invariant 2 ("TCP requires
/// TLS"), so by the time we land here `tls_cert_pem` and
/// `tls_key_pem` are both `Some`. We fail boot — not silently
/// skip — if either file fails to load: PR-10 spec INV-1
/// (fail-closed) governs.
///
/// Cert/key are loaded once at boot. Rotation today requires a
/// daemon restart; an automated rotation surface (file watcher
/// + tonic `serve_with_shutdown` swap) is a future concern that
/// PR-10's runbook §"static cert lifecycle" covers as operator-
/// owned.
pub(super) fn spawn_tcp_tls_listener(
    config: &DaemonConfig,
    listen_tcp: std::net::SocketAddr,
    service: DaemonInvocationService,
) -> anyhow::Result<()> {
    let cert_path = config
        .tls_cert_pem()
        .ok_or_else(|| anyhow::anyhow!("PR-10 invariant 1: TCP listener requires tls_cert_pem"))?;
    let key_path = config
        .tls_key_pem()
        .ok_or_else(|| anyhow::anyhow!("PR-10 invariant 1: TCP listener requires tls_key_pem"))?;

    let cert_pem = std::fs::read(cert_path).map_err(|err| {
        anyhow::anyhow!(
            "daemon-invocation: failed to read tls_cert_pem at {}: {err}",
            cert_path.display()
        )
    })?;
    let key_pem = std::fs::read(key_path).map_err(|err| {
        anyhow::anyhow!(
            "daemon-invocation: failed to read tls_key_pem at {}: {err}",
            key_path.display()
        )
    })?;

    let identity = Identity::from_pem(&cert_pem, &key_pem);
    let tls_config = ServerTlsConfig::new().identity(identity);

    let listen_tcp_display = format!("{listen_tcp}");
    let cert_path_display = format!("{}", cert_path.display());
    let key_path_display = format!("{}", key_path.display());
    crate::op_event!(
        component = daemon_invocation,
        kind = grpc_invocation_server_listening,
        transport = "tcp_tls",
        listen_tcp = listen_tcp_display,
        cert_pem = cert_path_display,
        key_pem = key_path_display,
        max_concurrent_streams = invocation_max_concurrent_streams(),
        concurrency_limit_per_connection = invocation_concurrency_limit_per_connection(),
    );

    // Production-WAN h2 hardening on the public TCP+TLS listener:
    // long-lived `session.open` bidi streams from devices behind
    // home/corporate NATs / hosting LBs need explicit keep-alive
    // PINGs or intermediaries silently drop the connection,
    // surfacing as "h2 protocol error: error reading a body" on
    // the device side and "session ended (StreamReset)" here.
    // 5s ping cadence: stays well under any NAT idle window
    // (~60s typical), surfaces dead streams in ~15s rather than
    // minutes, ~24 bytes/ping × 12/min ≈ negligible cost. Mirror
    // the device-client side at session_initiator.rs.
    let mut builder = match Server::builder().tls_config(tls_config) {
        Ok(b) => b
            .http2_keepalive_interval(Some(Duration::from_secs(5)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .tcp_keepalive(Some(Duration::from_secs(15)))
            .max_concurrent_streams(Some(invocation_max_concurrent_streams()))
            .concurrency_limit_per_connection(invocation_concurrency_limit_per_connection()),
        Err(err) => {
            return Err(anyhow::anyhow!(
                "daemon-invocation: tls_config rejected by tonic: {err}"
            ));
        }
    };

    tokio::spawn(async move {
        let result = builder
            .add_service(
                InvocationServer::new(service)
                    .max_decoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES),
            )
            .serve(listen_tcp)
            .await;
        if let Err(err) = result {
            let err_msg = format!("{err:#}");
            crate::op_event!(
                component = daemon_invocation,
                kind = grpc_server_exited_with_error,
                transport = "tcp_tls",
                error = err_msg,
            );
        }
    });

    Ok(())
}
