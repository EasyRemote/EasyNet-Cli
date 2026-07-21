// EasyNet CLI — daemon boot event stream
// ======================================
//
// File: src/daemon/control/boot_events.rs
// Description: Small in-process broadcast bus for daemon startup
//              progress. The control server exposes this bus through
//              `system.watch_boot` before the Axon runtime is
//              available, so `easynet start` can attach to the daemon
//              as soon as `control.sock` accepts.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Broadcast channel capacity for boot progress events.
///
/// Startup emits a small fixed number of stages today, but the UI
/// subscriber may be temporarily paused by terminal I/O. 64 keeps the
/// steady-state buffer bounded while making `RecvError::Lagged` an
/// exceptional condition rather than the common path.
const DEFAULT_BOOT_EVENT_CAPACITY: usize = 64;

/// One daemon startup progress event.
///
/// `Ready` and `Failed` are terminal events. [`BootBus`] records the
/// latest terminal event and replays it to late subscribers so `start`
/// never misses completion just because it connected after the daemon
/// finished booting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BootEvent {
    Stage {
        name: String,
        status: BootStageStatus,
    },
    PortChosen {
        service: String,
        port: u16,
        /// First port the daemon attempted in the probe range. When
        /// `start != port`, the chosen port is a fallback because the
        /// original was busy. `None` means the daemon did not declare
        /// the start candidate, so consumers must not reconstruct it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<u16>,
    },
    Ready,
    Failed {
        stage: String,
        error: String,
    },
}

/// Status payload for [`BootEvent::Stage`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BootStageStatus {
    Started,
    Ok,
    Skipped,
    Failed { reason: String },
}

/// Cloneable process-local boot event bus.
///
/// Invariants:
/// 1. Every call to `emit*` broadcasts exactly one event.
/// 2. Every emitted event is also appended to a history log so a
///    late subscriber (the CLI typically subscribes a few hundred
///    milliseconds AFTER the daemon emits its first event, because
///    `control.sock` only binds at stage `control-server`) receives
///    the full timeline from `kernel` through `Ready`, not just the
///    terminal event.
/// 3. Broadcast lag remains visible as `RecvError::Lagged`; the bus
///    does not hide slow consumers.
///
/// `history` is a `std::sync::RwLock` rather than `tokio::sync::RwLock`
/// on purpose: the only critical section is one `Vec` push or clone —
/// strictly O(stage_count), never crosses an `await`, and never
/// blocks the reactor under realistic contention. Switching to
/// `tokio::sync::RwLock` would force `emit` to become `async`, which
/// would in turn force the entire boot path through async-only call
/// sites for no measurable gain.
#[derive(Clone, Debug)]
pub struct BootBus {
    tx: broadcast::Sender<BootEvent>,
    history: Arc<RwLock<Vec<BootEvent>>>,
}

impl Default for BootBus {
    fn default() -> Self {
        Self::new()
    }
}

impl BootBus {
    /// Create a bus with the production channel capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BOOT_EVENT_CAPACITY)
    }

    /// Create a bus with an explicit broadcast capacity. Exposed for
    /// tests that need to force lag with a tiny buffer.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Subscribe to the event stream, replaying every event emitted
    /// so far before delivering new ones. The CLI subscribes only
    /// after `control.sock` accepts (i.e. after the daemon's first
    /// few stages), and the history replay is how it observes them.
    pub fn subscribe(&self) -> BootSubscription {
        let replay = self.history.read().expect("boot history lock").clone();
        BootSubscription {
            replay: replay.into_iter().collect(),
            rx: self.tx.subscribe(),
        }
    }

    /// Broadcast one event. Send failure means there are no current
    /// subscribers; this is not an error because future subscribers
    /// will pick the event up from the history replay.
    ///
    /// Every emit also produces one `op_event!` log line so the
    /// boot timeline is preserved on disk even when no CLI is
    /// subscribed (e.g. when `easynet start` was killed or never
    /// attached). The log shape mirrors the broadcast event so a
    /// grep'd boot timeline matches the CLI's UI.
    pub fn emit(&self, event: BootEvent) {
        log_boot_event(&event);
        {
            let mut history = self.history.write().expect("boot history lock");
            history.push(event.clone());
        }
        let _ = self.tx.send(event);
    }

    /// Emit a stage-start event.
    pub fn emit_started(&self, name: impl Into<String>) {
        self.emit(BootEvent::Stage {
            name: name.into(),
            status: BootStageStatus::Started,
        });
    }

    /// Emit a stage-ok event.
    pub fn emit_ok(&self, name: impl Into<String>) {
        self.emit(BootEvent::Stage {
            name: name.into(),
            status: BootStageStatus::Ok,
        });
    }

    /// Emit a stage-skipped event.
    pub fn emit_skipped(&self, name: impl Into<String>) {
        self.emit(BootEvent::Stage {
            name: name.into(),
            status: BootStageStatus::Skipped,
        });
    }

    /// Emit both the per-stage failure and terminal failure events.
    pub fn emit_failed(&self, name: impl Into<String>, reason: impl Into<String>) {
        let name = name.into();
        let reason = reason.into();
        self.emit(BootEvent::Stage {
            name: name.clone(),
            status: BootStageStatus::Failed {
                reason: reason.clone(),
            },
        });
        self.emit(BootEvent::Failed {
            stage: name,
            error: reason,
        });
    }

    /// Emit the terminal ready event.
    pub fn emit_ready(&self) {
        self.emit(BootEvent::Ready);
    }
}

/// Receiver returned by [`BootBus::subscribe`].
///
/// `replay` holds every event the bus had already emitted at
/// subscribe time; the subscriber drains that queue in order before
/// listening on the broadcast channel. This means a CLI that
/// subscribes mid-boot still sees `kernel … control-server …
/// tenant-stores …` in order.
pub struct BootSubscription {
    replay: std::collections::VecDeque<BootEvent>,
    rx: broadcast::Receiver<BootEvent>,
}

impl BootSubscription {
    /// Receive the next event. Drains the replay queue first; once
    /// drained, delegates to the underlying broadcast receiver.
    pub async fn recv(&mut self) -> Result<BootEvent, broadcast::error::RecvError> {
        if let Some(event) = self.replay.pop_front() {
            return Ok(event);
        }
        self.rx.recv().await
    }
}

/// Fan-out helper: every broadcast event is also recorded via
/// `op_event!` so daemons running without an attached CLI still have
/// a readable boot timeline in `easynet-daemon.log`.
///
/// Kinds are stable wire tokens (grep-safe): `stage_started`,
/// `stage_ok`, `stage_skipped`, `stage_failed`, `port_chosen`,
/// `ready`, `failed`.
fn log_boot_event(event: &BootEvent) {
    match event {
        BootEvent::Stage { name, status } => match status {
            BootStageStatus::Started => {
                crate::op_event!(component = boot, kind = stage_started, stage = name);
            }
            BootStageStatus::Ok => {
                crate::op_event!(component = boot, kind = stage_ok, stage = name);
            }
            BootStageStatus::Skipped => {
                crate::op_event!(component = boot, kind = stage_skipped, stage = name);
            }
            BootStageStatus::Failed { reason } => {
                crate::op_event!(
                    component = boot,
                    kind = stage_failed,
                    stage = name,
                    reason = reason,
                );
            }
        },
        BootEvent::PortChosen {
            service,
            port,
            start,
        } => {
            crate::op_event!(
                component = boot,
                kind = port_chosen,
                service = service,
                port = port,
                start = start.map(|p| p.to_string()).unwrap_or_default(),
            );
        }
        BootEvent::Ready => {
            crate::op_event!(component = boot, kind = ready);
        }
        BootEvent::Failed { stage, error } => {
            crate::op_event!(
                component = boot,
                kind = failed,
                stage = stage,
                error = error,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bus_broadcasts_to_multiple_subscribers() {
        let bus = BootBus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();

        bus.emit_started("kernel");

        assert_eq!(
            a.recv().await.unwrap(),
            BootEvent::Stage {
                name: "kernel".into(),
                status: BootStageStatus::Started,
            }
        );
        assert_eq!(
            b.recv().await.unwrap(),
            BootEvent::Stage {
                name: "kernel".into(),
                status: BootStageStatus::Started,
            }
        );
    }

    #[tokio::test]
    async fn lagged_subscriber_still_reaches_ready() {
        let bus = BootBus::with_capacity(2);
        let mut rx = bus.subscribe();

        bus.emit_started("a");
        bus.emit_ok("a");
        bus.emit_started("b");
        bus.emit_ready();

        let lag = rx.recv().await.unwrap_err();
        assert!(
            matches!(lag, broadcast::error::RecvError::Lagged(_)),
            "slow subscriber must observe broadcast lag, got {lag:?}"
        );
        assert_eq!(
            rx.recv().await.unwrap(),
            BootEvent::Stage {
                name: "b".into(),
                status: BootStageStatus::Started,
            }
        );
        assert_eq!(rx.recv().await.unwrap(), BootEvent::Ready);
    }

    #[tokio::test]
    async fn late_subscriber_replays_full_history_in_order() {
        let bus = BootBus::new();
        bus.emit_started("kernel");
        bus.emit_ok("kernel");
        bus.emit_started("control-server");
        bus.emit_ok("control-server");
        bus.emit_ready();

        let mut late = bus.subscribe();
        assert_eq!(
            late.recv().await.unwrap(),
            BootEvent::Stage {
                name: "kernel".into(),
                status: BootStageStatus::Started,
            }
        );
        assert_eq!(
            late.recv().await.unwrap(),
            BootEvent::Stage {
                name: "kernel".into(),
                status: BootStageStatus::Ok,
            }
        );
        assert_eq!(
            late.recv().await.unwrap(),
            BootEvent::Stage {
                name: "control-server".into(),
                status: BootStageStatus::Started,
            }
        );
        assert_eq!(
            late.recv().await.unwrap(),
            BootEvent::Stage {
                name: "control-server".into(),
                status: BootStageStatus::Ok,
            }
        );
        assert_eq!(late.recv().await.unwrap(), BootEvent::Ready);
    }
}
