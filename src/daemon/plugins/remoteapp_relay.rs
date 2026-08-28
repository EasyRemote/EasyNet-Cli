//! Daemon adapter for Hub-owned RemoteApp relay leases.
//!
//! The daemon is the only component allowed to read the durable device
//! credential. The Remote Desktop plugin receives this adapter as a port and
//! never opens `credentials.json` or invents Hub authentication itself.

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

use crate::daemon::persistence::config;
use crate::daemon::plugins::remote_desktop::relay_lease::{
    RemoteDesktopRelayLease, RemoteDesktopRelayLeaseAvailability, RemoteDesktopRelayLeaseInit,
    RemoteDesktopRelayLeaseProvider,
};

const HUB_RELAY_REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const HUB_RELAY_RESPONSE_LIMIT: u64 = 64 * 1024;

#[derive(Debug, Default)]
pub(super) struct HubRemoteDesktopRelayLeaseProvider;

#[derive(Deserialize)]
struct RelayLeaseResponse {
    provider: String,
    lease_id: String,
    session_id: String,
    device_ura: String,
    resource_ura: String,
    urls: Vec<String>,
    username: String,
    credential: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
    refresh_after_ms: u64,
}

impl RemoteDesktopRelayLeaseProvider for HubRemoteDesktopRelayLeaseProvider {
    fn acquire(
        &self,
        session_id: &str,
        resource_ura: &str,
    ) -> anyhow::Result<RemoteDesktopRelayLeaseAvailability> {
        let Some(credentials) = config::load_credentials_optional()? else {
            return Ok(RemoteDesktopRelayLeaseAvailability::unavailable(
                "device_not_paired",
            ));
        };
        let url = format!(
            "{}/api/v1/devices/relay-leases/acquire",
            credentials.api_base()
        );
        let response = ureq::post(&url)
            .timeout(HUB_RELAY_REQUEST_DEADLINE)
            .send_json(serde_json::json!({
                "node_id": credentials.node_id,
                "credential_token": credentials.credential_token,
                "session_id": session_id,
                "resource_ura": resource_ura,
            }));
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(503, _)) => {
                return Ok(RemoteDesktopRelayLeaseAvailability::unavailable(
                    "hub_relay_unavailable",
                ));
            }
            Err(ureq::Error::Status(status, _)) => {
                anyhow::bail!("Hub relay lease acquire rejected with HTTP {status}");
            }
            Err(ureq::Error::Transport(_)) => {
                return Ok(RemoteDesktopRelayLeaseAvailability::unavailable(
                    "hub_relay_transport_unavailable",
                ));
            }
        };
        let body: RelayLeaseResponse = read_bounded_json(response)?;
        let lease = RemoteDesktopRelayLease::from_init(
            session_id,
            resource_ura,
            RemoteDesktopRelayLeaseInit {
                provider: body.provider,
                lease_id: body.lease_id,
                session_id: body.session_id,
                device_ura: body.device_ura,
                resource_ura: body.resource_ura,
                urls: body.urls,
                username: body.username,
                credential: body.credential,
                issued_at_ms: body.issued_at_ms,
                expires_at_ms: body.expires_at_ms,
                refresh_after_ms: body.refresh_after_ms,
            },
        )?;
        Ok(RemoteDesktopRelayLeaseAvailability::Active(lease))
    }

    fn release(&self, lease: &RemoteDesktopRelayLease) -> anyhow::Result<()> {
        let Some(credentials) = config::load_credentials_optional()? else {
            return Ok(());
        };
        let url = format!(
            "{}/api/v1/devices/relay-leases/release",
            credentials.api_base()
        );
        let response = ureq::post(&url)
            .timeout(HUB_RELAY_REQUEST_DEADLINE)
            .send_json(serde_json::json!({
                "node_id": credentials.node_id,
                "credential_token": credentials.credential_token,
                "session_id": lease.session_id(),
                "resource_ura": lease.resource_ura(),
                "lease_id": lease.lease_id(),
            }));
        match response {
            Ok(response) => {
                let _: serde_json::Value = read_bounded_json(response)?;
                Ok(())
            }
            // Release is an expiry-bounded cleanup. A temporarily unavailable
            // Hub must not reopen or roll back a terminal product session.
            Err(ureq::Error::Status(503, _)) | Err(ureq::Error::Transport(_)) => Ok(()),
            Err(ureq::Error::Status(status, _)) => {
                anyhow::bail!("Hub relay lease release rejected with HTTP {status}")
            }
        }
    }
}

fn read_bounded_json<T: serde::de::DeserializeOwned>(
    response: ureq::Response,
) -> anyhow::Result<T> {
    let mut body = Vec::new();
    response
        .into_reader()
        .take(HUB_RELAY_RESPONSE_LIMIT + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > HUB_RELAY_RESPONSE_LIMIT {
        anyhow::bail!("Hub relay lease response exceeds size limit");
    }
    serde_json::from_slice(&body).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;

    use crate::daemon::plugins::remote_desktop::relay_lease::EASYNET_RELAY_PROVIDER;

    #[test]
    fn daemon_adapter_uses_device_credential_and_bounded_hub_contract() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener");
        let address = listener.local_addr().unwrap();
        config::save_credentials(&config::Credentials {
            node_id: "device-1".to_string(),
            credential_token: "durable-device-secret".to_string(),
            hub_endpoint: "https://127.0.0.1:50443".to_string(),
            realm: "acme".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some(format!("http://{address}")),
            username: Some("owner".to_string()),
            user_id: Some("2ce7a746-fb6c-45dc-9aff-d494296acf48".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        })
        .expect("test credentials save");

        let (request_tx, request_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            for operation in ["acquire", "release"] {
                let (stream, _) = listener.accept().expect("Hub request accepts");
                let (path, request) = read_http_json(stream.try_clone().unwrap());
                request_tx.send((path.clone(), request.clone())).unwrap();
                let response = if operation == "acquire" {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;
                    serde_json::json!({
                        "provider": EASYNET_RELAY_PROVIDER,
                        "lease_id": "lease-http-adapter",
                        "session_id": request["session_id"],
                        "device_ura": "easynet:///r/acme/device/device-1",
                        "resource_ura": request["resource_ura"],
                        "urls": ["turn:relay.example.test:3478?transport=udp"],
                        "username": "ephemeral-user",
                        "credential": "ephemeral-credential",
                        "issued_at_ms": now,
                        "refresh_after_ms": now + 60_000,
                        "expires_at_ms": now + 120_000,
                    })
                } else {
                    serde_json::json!({
                        "provider": EASYNET_RELAY_PROVIDER,
                        "lease_id": request["lease_id"],
                        "session_id": request["session_id"],
                        "device_ura": "easynet:///r/acme/device/device-1",
                        "resource_ura": request["resource_ura"],
                        "released_at_ms": 1,
                        "expires_at_ms": 2,
                    })
                };
                write_http_json(stream, &response);
            }
        });

        let provider = HubRemoteDesktopRelayLeaseProvider;
        let resource_ura = "easynet:///r/acme/resource/device.device-1/streams/window.42";
        let lease = match provider
            .acquire("rd-http-adapter", resource_ura)
            .expect("Hub acquire succeeds")
        {
            RemoteDesktopRelayLeaseAvailability::Active(lease) => lease,
            unavailable => panic!("unexpected relay outcome: {unavailable:?}"),
        };
        provider.release(&lease).expect("Hub release succeeds");
        server.join().expect("Hub test server joins");

        let (acquire_path, acquire) = request_rx.recv().unwrap();
        assert_eq!(acquire_path, "/api/v1/devices/relay-leases/acquire");
        assert_eq!(acquire["node_id"], "device-1");
        assert_eq!(acquire["credential_token"], "durable-device-secret");
        assert_eq!(acquire["session_id"], "rd-http-adapter");
        let (release_path, release) = request_rx.recv().unwrap();
        assert_eq!(release_path, "/api/v1/devices/relay-leases/release");
        assert_eq!(release["lease_id"], "lease-http-adapter");
        assert_eq!(release["credential_token"], "durable-device-secret");
    }

    fn read_http_json(stream: TcpStream) -> (String, serde_json::Value) {
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let path = request_line.split_whitespace().nth(1).unwrap().to_string();
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();
        (path, serde_json::from_slice(&body).unwrap())
    }

    fn write_http_json(mut stream: TcpStream, value: &serde_json::Value) {
        let body = serde_json::to_vec(value).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
        stream.flush().unwrap();
    }
}
