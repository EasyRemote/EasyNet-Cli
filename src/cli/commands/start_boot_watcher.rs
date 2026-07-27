// EasyNet CLI — daemon boot event watcher
// ========================================
//
// File: src/cli/start_boot_watcher.rs
// Description: `easynet runtime start`'s daemon-side bridge. Polls
//              `control.sock` until the freshly-spawned daemon
//              starts accepting, subscribes to `system.watch_boot`,
//              and translates each `BootEvent` frame into one call
//              against [`presentation::stage::StageRenderer`]. This
//              module owns no UI primitives of its own — every
//              spinner, shimmer, icon, or color decision lives in
//              the `presentation` layer.
//
// Why a separate module from `start.rs`
// -------------------------------------
// `start.rs` orchestrates: load credentials, spawn the daemon
// process, persist runtime.json, optionally enter foreground mode.
// The watcher's only job is "wait for the daemon to finish booting
// while keeping the user informed"; isolating it keeps both files
// small and unit-testable in their respective concerns.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::cli::presentation::stage::StageRenderer;
use crate::daemon::control::boot_events::{BootEvent, BootStageStatus};
use crate::daemon::control::discovery;
use crate::daemon::control::frames::{IncomingFrame, OutgoingFrame};
use crate::daemon::control::server::WATCH_BOOT_ABILITY;

const BOOT_SUBSCRIPTION_ID: &str = "cli-start-watch-boot";
const NO_EVENT_WATCHDOG: Duration = Duration::from_secs(60);
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Upper bound on how long we wait for `control.sock` to be bound
/// by a live daemon child. Once the child exits we bail
/// immediately via `try_wait`, so this only kicks in for a daemon
/// that is alive but stuck BEFORE stage 1 (filesystem permission
/// errors, kernel-level fork issues). 30 s is comfortable for a
/// cold boot on a busy disk.
const SOCKET_WAIT_BUDGET: Duration = Duration::from_secs(30);

/// Result collected from the boot event stream.
#[derive(Debug, Clone, Default)]
pub struct BootProgressOutcome {
    pub pages_port: Option<u16>,
    pub ready_capability_flags: Vec<String>,
}

impl BootProgressOutcome {
    pub fn has_ready_capability_flag(&self, flag: &str) -> bool {
        let flag = flag.trim();
        !flag.is_empty()
            && self
                .ready_capability_flags
                .iter()
                .any(|candidate| candidate == flag)
    }
}

/// Wait until the daemon's control socket accepts, then subscribe
/// to boot events until the daemon reports Ready or Failed.
///
/// `daemon` is the freshly-spawned `easynet-daemon` child, if any.
/// If the child exits while we are still waiting for `control.sock`
/// to accept, surface that as an error instead of polling forever
/// — a dead daemon will never bind the socket, and we should not
/// pretend otherwise.
pub fn wait_for_daemon_boot(
    control_socket: &Path,
    daemon: Option<&mut std::process::Child>,
) -> anyhow::Result<BootProgressOutcome> {
    let mut renderer =
        StageRenderer::with_initial_message("waiting for easynet-daemon control socket");

    let mut daemon = daemon;
    let socket_deadline = std::time::Instant::now() + SOCKET_WAIT_BUDGET;
    while !crate::support::platform::local_daemon_grpc::probe_accepting(control_socket) {
        if let Some(child) = daemon.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    renderer.finish();
                    anyhow::bail!(
                        "easynet-daemon exited before binding control.sock (status: {status}); \
                         check ~/.easynet/logs/easynet-daemon.log for the failure"
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    renderer.finish();
                    anyhow::bail!("failed to poll daemon child status: {e}");
                }
            }
        }
        if std::time::Instant::now() >= socket_deadline {
            renderer.finish();
            anyhow::bail!(
                "easynet-daemon did not bind control.sock within {}s; \
                 the daemon is alive but stuck early in boot — check the daemon log",
                SOCKET_WAIT_BUDGET.as_secs()
            );
        }
        std::thread::sleep(SOCKET_POLL_INTERVAL);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build boot progress runtime")?;

    let result = runtime.block_on(subscribe_boot_events(&renderer));
    renderer.finish();
    result
}

async fn subscribe_boot_events(renderer: &StageRenderer) -> anyhow::Result<BootProgressOutcome> {
    let control_json =
        discovery::try_default_path().context("resolve boot control discovery path")?;
    let disc = loop {
        match discovery::read(&control_json)? {
            Some(disc) => break disc,
            None => tokio::time::sleep(SOCKET_POLL_INTERVAL).await,
        }
    };

    #[cfg(unix)]
    let stream = {
        let socket_path = disc
            .socket_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("control.json has no socket_path"))?;
        tokio::net::UnixStream::connect(&socket_path)
            .await
            .with_context(|| format!("connect boot progress socket {}", socket_path.display()))?
    };
    #[cfg(windows)]
    let stream = {
        let pipe_name = disc
            .pipe_name
            .clone()
            .ok_or_else(|| anyhow::anyhow!("control.json has no pipe_name"))?;
        crate::support::platform::named_pipe::connect_with_retry(&pipe_name, Duration::from_secs(5))
            .await?
    };

    let codec = LengthDelimitedCodec::builder().little_endian().new_codec();
    let mut framed = Framed::new(stream, codec);
    let req = IncomingFrame::Subscribe {
        subscription_id: BOOT_SUBSCRIPTION_ID.into(),
        ability: WATCH_BOOT_ABILITY.into(),
        args: serde_json::json!({}),
    };
    framed
        .send(Bytes::from(serde_json::to_vec(&req)?))
        .await
        .context("send boot Subscribe frame")?;

    let mut outcome = BootProgressOutcome::default();
    loop {
        let next = tokio::time::timeout(NO_EVENT_WATCHDOG, framed.next())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "daemon boot made no visible progress for {} seconds",
                    NO_EVENT_WATCHDOG.as_secs()
                )
            })?;
        let Some(frame_res) = next else {
            anyhow::bail!("daemon closed boot progress stream before Ready");
        };
        let bytes = frame_res.context("read boot progress frame")?;
        let outgoing: OutgoingFrame =
            serde_json::from_slice(&bytes).context("decode boot progress frame")?;
        match outgoing {
            OutgoingFrame::Frame { frame, .. } => {
                if frame.get("type").and_then(serde_json::Value::as_str) == Some("lagged") {
                    let dropped = frame
                        .get("dropped")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    renderer.info(&format!(
                        "boot stream lagged; skipped {dropped} stale event(s)"
                    ));
                    continue;
                }
                let event: BootEvent =
                    serde_json::from_value(frame).context("decode BootEvent frame")?;
                if apply_event(renderer, &event, &mut outcome)? {
                    return Ok(outcome);
                }
            }
            OutgoingFrame::Terminal { reason, .. } if reason == "done" => {
                return Ok(outcome);
            }
            OutgoingFrame::Terminal { reason, .. } => {
                anyhow::bail!("boot progress stream ended before Ready: {reason}");
            }
            OutgoingFrame::Error { code, message, .. } => {
                anyhow::bail!("boot progress subscription failed (code={code}): {message}");
            }
        }
    }
}

/// Translate one `BootEvent` into one [`StageRenderer`] call.
/// Returns `Ok(true)` when the terminal `Ready` event arrives, so
/// the caller can break out of the read loop.
fn apply_event(
    renderer: &StageRenderer,
    event: &BootEvent,
    outcome: &mut BootProgressOutcome,
) -> anyhow::Result<bool> {
    match event {
        BootEvent::Stage { name, status } => match status {
            BootStageStatus::Started => {
                renderer.set_active(name.clone());
            }
            BootStageStatus::Ok => {
                renderer.stage_ok(name);
            }
            BootStageStatus::Skipped => {
                renderer.stage_skipped(name, "skipped");
            }
            BootStageStatus::Failed { reason } => {
                renderer.stage_failed(name, reason);
            }
        },
        BootEvent::PortChosen {
            service,
            port,
            start,
        } => {
            if service == "pages" {
                outcome.pages_port = Some(*port);
            }
            let line = match start {
                Some(s) if *s != *port => {
                    format!("{service} port {port} (fell back from {s})")
                }
                _ => format!("{service} port {port}"),
            };
            renderer.stage_ok(&line);
        }
        BootEvent::Ready => {
            let path = discovery::try_default_path()
                .context("resolve daemon ready discovery path after Ready")?;
            let disc = discovery::read(&path)
                .context("read daemon ready discovery after Ready")?
                .ok_or_else(|| anyhow::anyhow!("daemon Ready without control discovery"))?;
            outcome.ready_capability_flags = disc.capability_flags.clone();
            renderer.stage_ok("daemon ready");
            return Ok(true);
        }
        BootEvent::Failed { stage, error } => {
            renderer.stage_failed(stage, error);
            anyhow::bail!("daemon failed during {stage}: {error}");
        }
    }
    Ok(false)
}

/// Read the final pages port from control.json, falling back to the
/// value observed in the event stream.
pub fn final_pages_port(event_port: Option<u16>) -> Option<u16> {
    let path: PathBuf = match discovery::try_default_path() {
        Ok(path) => path,
        Err(_) => return event_port,
    };
    discovery::read(&path)
        .ok()
        .flatten()
        .and_then(|disc| disc.pages_port)
        .or(event_port)
}
