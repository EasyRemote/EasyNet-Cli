// EasyNet CLI — remote desktop target monitor
// ===========================================
//
// File: plugins/remote-desktop/src/target_monitor.rs
// Description: Plugin-owned target lifecycle poller for remote desktop sessions.
//
// Protocol Responsibility:
// - None. This is daemon-owned device runtime lifecycle infrastructure.
//
// Implementation Approach:
// - Maintain one worker thread per plugin instance.
// - Track session ids after TARGET_BOUND/create_session and cancel them at
//   terminal cleanup.
// - Each worker tick samples host target state once, fans the immutable sample
//   out to tracked sessions, and commits observations only through
//   RemoteDesktopSessionStore, so session aggregate state remains the single
//   mutation boundary.
//
// Architectural Position:
// - Remote-desktop plugin lifecycle infrastructure, deliberately independent of
//   WebRTC/native media loops.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::daemon::plugins::remote_desktop::lifecycle_worker::LifecycleWorker;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::TargetMediaSourceLost;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target_observer::{
    observe_bound_session_target_once, sample_platform_target_observations,
};
use crate::daemon::plugins::remote_desktop::transport::RemoteDesktopTransportManager;

const TARGET_MONITOR_INTERVAL: Duration = Duration::from_millis(250);

pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopTargetMonitor {
    worker: Mutex<LifecycleWorker<TargetMonitorCommand>>,
}

enum TargetMonitorCommand {
    Track { session_id: String },
    Cancel { session_id: String },
    Shutdown,
}

trait TargetMediaSourceStopper {
    fn stop_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool;
}

impl TargetMediaSourceStopper for RemoteDesktopTransportManager {
    fn stop_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool {
        RemoteDesktopTransportManager::stop_endpoint_if_epoch(self, session_id, epoch)
    }
}

impl RemoteDesktopTargetMonitor {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            worker: Mutex::new(LifecycleWorker::new()),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn track(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
        session_id: String,
    ) -> anyhow::Result<()> {
        let command = TargetMonitorCommand::Track { session_id };
        let tx = self.ensure_worker(plugin)?;
        let command = match tx.send(command) {
            Ok(()) => return Ok(()),
            Err(error) => error.0,
        };

        if let TargetMonitorCommand::Track { session_id } = command {
            let tx = self.restart_worker(plugin)?;
            tx.send(TargetMonitorCommand::Track { session_id })
                .map_err(|err| {
                    anyhow::anyhow!("remote desktop target monitor unavailable: {err}")
                })?;
        }
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn cancel(&self, session_id: &str) {
        let tx = self.worker().sender();
        if let Some(tx) = tx {
            let _ = tx.send(TargetMonitorCommand::Cancel {
                session_id: session_id.to_string(),
            });
        }
    }

    fn ensure_worker(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
    ) -> anyhow::Result<Sender<TargetMonitorCommand>> {
        let mut worker = self.worker();
        if let Some(tx) = worker.sender() {
            return Ok(tx);
        }
        worker
            .start(|| spawn_target_monitor_worker(Arc::downgrade(plugin)))
            .map_err(|err| anyhow::anyhow!("spawn remote desktop target monitor: {err}"))
    }

    fn restart_worker(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
    ) -> anyhow::Result<Sender<TargetMonitorCommand>> {
        let mut worker = self.worker();
        worker
            .start(|| spawn_target_monitor_worker(Arc::downgrade(plugin)))
            .map_err(|err| anyhow::anyhow!("restart remote desktop target monitor: {err}"))
    }

    fn worker(&self) -> MutexGuard<'_, LifecycleWorker<TargetMonitorCommand>> {
        match self.worker.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Drop for RemoteDesktopTargetMonitor {
    fn drop(&mut self) {
        let worker = match self.worker.get_mut() {
            Ok(worker) => worker,
            Err(poisoned) => poisoned.into_inner(),
        };
        worker.shutdown(TargetMonitorCommand::Shutdown);
    }
}

fn spawn_target_monitor_worker(
    plugin: Weak<RemoteDesktopPlugin>,
) -> std::io::Result<(Sender<TargetMonitorCommand>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let join = thread::Builder::new()
        .name("easynet-rd-target-monitor".into())
        .spawn(move || run_target_monitor(plugin, rx))?;
    Ok((tx, join))
}

fn run_target_monitor(plugin: Weak<RemoteDesktopPlugin>, rx: Receiver<TargetMonitorCommand>) {
    let mut tracked = HashSet::<String>::new();
    loop {
        if tracked.is_empty() {
            match rx.recv() {
                Ok(command) => {
                    if !apply_command(command, &mut tracked) {
                        return;
                    }
                }
                Err(_) => return,
            }
            continue;
        }

        match rx.recv_timeout(TARGET_MONITOR_INTERVAL) {
            Ok(command) => {
                if !apply_command(command, &mut tracked) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        if !poll_tracked_sessions(&plugin, &mut tracked) {
            return;
        }
    }
}

fn apply_command(command: TargetMonitorCommand, tracked: &mut HashSet<String>) -> bool {
    match command {
        TargetMonitorCommand::Track { session_id } => {
            if !session_id.is_empty() {
                tracked.insert(session_id);
            }
            true
        }
        TargetMonitorCommand::Cancel { session_id } => {
            tracked.remove(&session_id);
            true
        }
        TargetMonitorCommand::Shutdown => false,
    }
}

fn poll_tracked_sessions(
    plugin: &Weak<RemoteDesktopPlugin>,
    tracked: &mut HashSet<String>,
) -> bool {
    let Some(plugin) = plugin.upgrade() else {
        return false;
    };
    let sessions = plugin.session_store();
    let transports = plugin.transport_manager();
    let provider = sample_platform_target_observations();
    tracked.retain(|session_id| {
        let result = observe_bound_session_target_once(&sessions, session_id, &provider);
        stop_lost_media_source(transports.as_ref(), session_id, result.media_source_lost);
        result.keep_tracking
    });
    true
}

fn stop_lost_media_source<T>(
    transports: &T,
    session_id: &str,
    media_source_lost: Option<TargetMediaSourceLost>,
) -> bool
where
    T: TargetMediaSourceStopper + ?Sized,
{
    let Some(media_source_lost) = media_source_lost else {
        return false;
    };
    transports.stop_endpoint_if_epoch(session_id, media_source_lost.transport_epoch)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Mutex, MutexGuard};

    use crate::daemon::plugins::remote_desktop::session::TargetMediaSourceLost;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::TargetResolutionError;
    use crate::daemon::plugins::remote_desktop::target_monitor::{
        apply_command, stop_lost_media_source, TargetMediaSourceStopper, TargetMonitorCommand,
    };

    #[derive(Default)]
    struct RecordingStopper {
        calls: Mutex<Vec<(String, TransportEpoch)>>,
        stopped: bool,
    }

    impl RecordingStopper {
        fn calls(&self) -> MutexGuard<'_, Vec<(String, TransportEpoch)>> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }
    }

    impl TargetMediaSourceStopper for RecordingStopper {
        fn stop_endpoint_if_epoch(&self, session_id: &str, epoch: TransportEpoch) -> bool {
            self.calls().push((session_id.to_string(), epoch));
            self.stopped
        }
    }

    #[test]
    fn target_loss_poll_result_stops_endpoint_by_epoch() {
        let stopper = RecordingStopper {
            calls: Mutex::new(Vec::new()),
            stopped: true,
        };
        let epoch = TransportEpoch::new(42);
        let stopped = stop_lost_media_source(
            &stopper,
            "rd-target-loss",
            Some(TargetMediaSourceLost {
                transport_epoch: epoch,
                reason: TargetResolutionError::TargetNotFound,
            }),
        );

        assert!(stopped);
        assert_eq!(
            stopper.calls().as_slice(),
            &[("rd-target-loss".into(), epoch)]
        );
    }

    #[test]
    fn healthy_poll_result_does_not_touch_transport_manager() {
        let stopper = RecordingStopper::default();

        assert!(!stop_lost_media_source(&stopper, "rd-healthy", None));
        assert!(stopper.calls().is_empty());
    }

    #[test]
    fn target_monitor_command_state_machine_tracks_cancels_and_shuts_down() {
        let mut tracked = HashSet::<String>::new();

        assert!(apply_command(
            TargetMonitorCommand::Track {
                session_id: String::new(),
            },
            &mut tracked,
        ));
        assert!(
            tracked.is_empty(),
            "empty session ids must not enter target tracking"
        );

        assert!(apply_command(
            TargetMonitorCommand::Track {
                session_id: "rd-target-a".into(),
            },
            &mut tracked,
        ));
        assert!(apply_command(
            TargetMonitorCommand::Track {
                session_id: "rd-target-a".into(),
            },
            &mut tracked,
        ));
        assert_eq!(
            tracked.iter().cloned().collect::<Vec<_>>(),
            vec!["rd-target-a".to_string()],
            "target monitor tracking must be idempotent per session id"
        );

        assert!(apply_command(
            TargetMonitorCommand::Track {
                session_id: "rd-target-b".into(),
            },
            &mut tracked,
        ));
        assert_eq!(tracked.len(), 2);

        assert!(apply_command(
            TargetMonitorCommand::Cancel {
                session_id: "rd-target-a".into(),
            },
            &mut tracked,
        ));
        assert!(!tracked.contains("rd-target-a"));
        assert!(tracked.contains("rd-target-b"));

        assert!(!apply_command(TargetMonitorCommand::Shutdown, &mut tracked));
    }
}
