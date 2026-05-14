// EasyNet CLI — local daemon Invocation transport helpers
// =======================================================
//
// File: src/support/local_daemon_grpc.rs
// Description: Resolve, probe, and (when `axon-pb` is enabled)
//              connect to the local easynet-daemon Invocation
//              listener across Unix UDS and Windows named pipes.
//
// Why this exists
// ---------------
// The CLI had grown four separate copies of "resolve daemon.sock,
// build a tonic endpoint, connect over UnixStream". That made the
// Windows port brittle: every copy needed a different named-pipe
// branch, and missing one would silently strand a subcommand. This
// module is the single control point for the local daemon transport.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};
#[cfg(any(windows, feature = "axon-pb"))]
use std::time::Duration;

/// Resolve the local daemon Invocation endpoint. Historical env var
/// name `EASYNET_DAEMON_GRPC_UDS` stays in place for backwards
/// compatibility; on Windows the value is interpreted as a named-pipe
/// name instead of a filesystem socket path.
pub(crate) fn resolve_socket_path() -> PathBuf {
    let raw = std::env::var("EASYNET_DAEMON_GRPC_UDS").ok();
    match raw {
        Some(raw) if !raw.trim().is_empty() => {
            #[cfg(windows)]
            {
                return PathBuf::from(raw);
            }
            #[cfg(not(windows))]
            {
                if let Some(rest) = raw.strip_prefix("~/") {
                    if let Some(home) = std::env::var_os("HOME") {
                        return PathBuf::from(home).join(rest);
                    }
                }
                return PathBuf::from(raw);
            }
        }
        _ => crate::persistence::daemon_config::resolved_local_uds_path(),
    }
}

/// Best-effort local liveness probe used by `runtime start` to avoid
/// racing a second daemon process against an already-live listener.
pub(crate) fn probe_accepting(path: &Path) -> bool {
    #[cfg(unix)]
    {
        if !path.exists() {
            return false;
        }
        return std::os::unix::net::UnixStream::connect(path).is_ok();
    }

    #[cfg(windows)]
    {
        let Some(name) = path.to_str() else {
            return false;
        };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return false,
        };
        return runtime
            .block_on(crate::support::named_pipe::connect_with_retry(
                name,
                Duration::from_millis(250),
            ))
            .is_ok();
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        false
    }
}

#[cfg(feature = "axon-pb")]
pub(crate) async fn connect_channel(
    path: PathBuf,
    timeout: Duration,
    connect_timeout: Duration,
) -> anyhow::Result<tonic::transport::Channel> {
    let endpoint = tonic::transport::Endpoint::try_from("http://[::1]:50051")?
        .timeout(timeout)
        .connect_timeout(connect_timeout);

    #[cfg(unix)]
    {
        return endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let path = path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
            .map_err(Into::into);
    }

    #[cfg(windows)]
    {
        let pipe_name = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("named-pipe path is not valid UTF-8"))?
            .to_string();
        return endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let pipe_name = pipe_name.clone();
                async move {
                    let stream =
                        crate::support::named_pipe::connect_with_retry(&pipe_name, connect_timeout)
                            .await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
            .map_err(Into::into);
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        let _ = timeout;
        let _ = connect_timeout;
        anyhow::bail!("local daemon Invocation transport is unsupported on this platform");
    }
}
