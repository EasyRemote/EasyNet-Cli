// EasyNet CLI — remote desktop lease monitor
// ==========================================
//
// File: plugins/remote-desktop/src/lease_monitor.rs
// Description: Plugin-owned lease-expiry scheduler for remote desktop sessions.
//
// Protocol Responsibility:
// - None. Lease expiry is daemon-owned remote desktop session policy.
//
// Implementation Approach:
// - Maintain one scheduler thread per plugin instance. Refreshes replace the
//   session deadline in a bounded map instead of accumulating independent
//   timers. Drop sends an explicit shutdown command; external owners join the
//   worker while worker-thread destruction detaches to avoid self-join.
//
// Usage Contract:
// - Schedule after session creation and lease refresh.
// - Cancel after any explicit terminal close.
//
// Architectural Position:
// - Remote-desktop plugin lifecycle infrastructure.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::daemon::plugins::remote_desktop::lifecycle_worker::LifecycleWorker;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::session_lifecycle::expire_session_from_watchdog;

pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopLeaseMonitor {
    worker: Mutex<LifecycleWorker<LeaseMonitorCommand>>,
}

enum LeaseMonitorCommand {
    Schedule {
        session_id: String,
        lease_expires_at_ms: u64,
    },
    Cancel {
        session_id: String,
    },
    Shutdown,
}

impl RemoteDesktopLeaseMonitor {
    pub(in crate::daemon::plugins::remote_desktop) fn new() -> Self {
        Self {
            worker: Mutex::new(LifecycleWorker::new()),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn schedule(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
        session_id: String,
        lease_expires_at_ms: u64,
    ) -> anyhow::Result<()> {
        let command = LeaseMonitorCommand::Schedule {
            session_id,
            lease_expires_at_ms,
        };
        let tx = self.ensure_worker(plugin)?;
        let command = match tx.send(command) {
            Ok(()) => return Ok(()),
            Err(error) => error.0,
        };

        if let LeaseMonitorCommand::Schedule {
            session_id,
            lease_expires_at_ms,
        } = command
        {
            let tx = self.restart_worker(plugin)?;
            tx.send(LeaseMonitorCommand::Schedule {
                session_id,
                lease_expires_at_ms,
            })
            .map_err(|err| anyhow::anyhow!("remote desktop lease monitor unavailable: {err}"))?;
        }
        Ok(())
    }

    pub(in crate::daemon::plugins::remote_desktop) fn cancel(&self, session_id: &str) {
        let tx = self.worker().sender();
        if let Some(tx) = tx {
            let _ = tx.send(LeaseMonitorCommand::Cancel {
                session_id: session_id.to_string(),
            });
        }
    }

    fn ensure_worker(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
    ) -> anyhow::Result<Sender<LeaseMonitorCommand>> {
        let mut worker = self.worker();
        if let Some(tx) = worker.sender() {
            return Ok(tx);
        }
        worker
            .start(|| spawn_lease_monitor_worker(Arc::downgrade(plugin)))
            .map_err(|err| anyhow::anyhow!("spawn remote desktop lease monitor: {err}"))
    }

    fn restart_worker(
        &self,
        plugin: &Arc<RemoteDesktopPlugin>,
    ) -> anyhow::Result<Sender<LeaseMonitorCommand>> {
        let mut worker = self.worker();
        worker
            .start(|| spawn_lease_monitor_worker(Arc::downgrade(plugin)))
            .map_err(|err| anyhow::anyhow!("restart remote desktop lease monitor: {err}"))
    }

    fn worker(&self) -> MutexGuard<'_, LifecycleWorker<LeaseMonitorCommand>> {
        match self.worker.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Drop for RemoteDesktopLeaseMonitor {
    fn drop(&mut self) {
        let worker = match self.worker.get_mut() {
            Ok(worker) => worker,
            Err(poisoned) => poisoned.into_inner(),
        };
        worker.shutdown(LeaseMonitorCommand::Shutdown);
    }
}

fn spawn_lease_monitor_worker(
    plugin: Weak<RemoteDesktopPlugin>,
) -> std::io::Result<(Sender<LeaseMonitorCommand>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let join = thread::Builder::new()
        .name("easynet-rd-lease-monitor".into())
        .spawn(move || run_lease_monitor(plugin, rx))?;
    Ok((tx, join))
}

fn run_lease_monitor(plugin: Weak<RemoteDesktopPlugin>, rx: Receiver<LeaseMonitorCommand>) {
    let mut deadlines = HashMap::<String, u64>::new();
    loop {
        expire_due_sessions(&plugin, &mut deadlines);
        let Some(next_deadline) = deadlines.values().copied().min() else {
            match rx.recv() {
                Ok(command) => {
                    if !apply_command(command, &mut deadlines) {
                        return;
                    }
                }
                Err(_) => return,
            }
            continue;
        };

        let timeout = Duration::from_millis(next_deadline.saturating_sub(now_ms()));
        match rx.recv_timeout(timeout) {
            Ok(command) => {
                if !apply_command(command, &mut deadlines) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn apply_command(command: LeaseMonitorCommand, deadlines: &mut HashMap<String, u64>) -> bool {
    match command {
        LeaseMonitorCommand::Schedule {
            session_id,
            lease_expires_at_ms,
        } => {
            if !session_id.is_empty() {
                deadlines.insert(session_id, lease_expires_at_ms);
            }
            true
        }
        LeaseMonitorCommand::Cancel { session_id } => {
            deadlines.remove(&session_id);
            true
        }
        LeaseMonitorCommand::Shutdown => false,
    }
}

fn expire_due_sessions(plugin: &Weak<RemoteDesktopPlugin>, deadlines: &mut HashMap<String, u64>) {
    let now = now_ms();
    let due: Vec<(String, u64)> = deadlines
        .iter()
        .filter(|(_, deadline)| **deadline <= now)
        .map(|(session_id, deadline)| (session_id.clone(), *deadline))
        .collect();
    for (session_id, deadline) in due {
        deadlines.remove(&session_id);
        let Some(plugin) = plugin.upgrade() else {
            return;
        };
        expire_session_from_watchdog(&plugin, &session_id, deadline);
    }
}
