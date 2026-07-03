use std::path::Path;
use std::time::Duration;

use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use tonic::transport::{Channel, Endpoint};

use super::SessionError;

pub(super) async fn connect_session_channel(
    hub_endpoint: &str,
    hub_ca_pem_path: Option<&Path>,
) -> Result<Channel, SessionError> {
    let mut endpoint = Endpoint::from_shared(hub_endpoint.to_string())
        .map_err(|err| SessionError::InvalidEndpoint {
            endpoint: hub_endpoint.to_string(),
            source: err,
        })?
        // No timeout on the bidi itself; the stream is intended to
        // live forever. Connect timeout caps only the dial step.
        .connect_timeout(Duration::from_secs(10))
        // Production-WAN h2 hardening: HTTP/2 keep-alive PINGs every
        // 5s with 10s timeout. This protects the long-lived reverse
        // channel against NAT/LB idle reaping without changing the
        // application-level heartbeat and idle-timeout semantics.
        .http2_keep_alive_interval(Duration::from_secs(5))
        .keep_alive_timeout(Duration::from_secs(10))
        .keep_alive_while_idle(true)
        .tcp_keepalive(Some(Duration::from_secs(15)));

    if let Some(ca_path) = hub_ca_pem_path {
        let tls =
            crate::daemon::federation::client::pinned_tls_config(ca_path).map_err(
                |err| match err {
                    crate::daemon::federation::client::PinnedTlsError::ReadFailed {
                        path,
                        source,
                    } => SessionError::TlsCaRead { path, source },
                },
            )?;
        endpoint = endpoint
            .tls_config(tls)
            .map_err(|err| SessionError::TlsConfig {
                endpoint: hub_endpoint.to_string(),
                source: err,
            })?;
    } else if hub_endpoint.starts_with("https://") {
        let native_tls = tonic::transport::ClientTlsConfig::new().with_native_roots();
        endpoint = endpoint
            .tls_config(native_tls)
            .map_err(|err| SessionError::TlsConfig {
                endpoint: hub_endpoint.to_string(),
                source: err,
            })?;
    }

    endpoint
        .connect()
        .await
        .map_err(|err| SessionError::ConnectFailed {
            endpoint: hub_endpoint.to_string(),
            source: err,
        })
}

pub(super) fn session_invocation_client(channel: Channel) -> InvocationClient<Channel> {
    InvocationClient::new(channel)
        .max_decoding_message_size(
            crate::daemon::boot::invocation::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        )
        .max_encoding_message_size(
            crate::daemon::boot::invocation::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        )
}
