// EasyNet CLI — remote desktop session lifecycle
// ===============================================
//
// File: plugins/remote-desktop/src/session_lifecycle.rs
// Description: Lease, liveness, terminal cleanup, and transport teardown.
//
// Protocol Responsibility:
// - None. Axon owns Invocation/Receipt lifecycle; this module owns the
//   RemoteApp product session and transport-resource lifecycle.
//
// Implementation Approach:
// - Retire transports before terminal publication, settle them on one bounded
//   executor, and promote recovery candidates through per-session commit locks.
//
// Usage Contract:
// - Blocking transport and recovery I/O must run without the global session-map
//   mutex; terminal publication must preserve one exact staged revision.
//
// Architectural Position:
// - Remote-desktop plugin lifecycle coordinator.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::{
    REASON_SESSION_EXPIRED, REASON_TRANSPORT_SETTLEMENT_FAILED,
};
use crate::daemon::plugins::remote_desktop::errors::RemoteDesktopError;
use crate::daemon::plugins::remote_desktop::relay_lease::RemoteDesktopRelayLeaseProvider;
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session::{now_ms, RemoteDesktopSession};
use crate::daemon::plugins::remote_desktop::session_access::{
    ensure_session_control_identity, ensure_session_resource_identity,
};
use crate::daemon::plugins::remote_desktop::session_recovery::{
    RemoteDesktopRecoverySnapshot, RemoteDesktopRecoveryStagedSnapshot, RemoteDesktopRecoveryStore,
};
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::transport::{
    RetiredDiagnosticPreview, RetiredDirectWebRtcEndpoint, TransportSettlementFailureKind,
    TransportSettlementJob, TransportSettlementJobContext, TransportSettlementQueue,
    TransportSettlementStatus, TRANSPORT_SETTLEMENT_DEADLINE,
};

/// Complete process-local transport ownership retired by one terminal intent.
/// Both stop signals are published before this value leaves the session lock;
/// terminal state may be committed only after `settle` confirms both workers.
pub(in crate::daemon::plugins::remote_desktop) struct RetiredSessionTransports {
    direct_webrtc: Option<RetiredDirectWebRtcEndpoint>,
    diagnostic_preview: Option<RetiredDiagnosticPreview>,
}

impl RetiredSessionTransports {
    pub(in crate::daemon::plugins::remote_desktop) fn empty() -> Self {
        Self {
            direct_webrtc: None,
            diagnostic_preview: None,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn new(
        direct_webrtc: Option<RetiredDirectWebRtcEndpoint>,
        diagnostic_preview: Option<RetiredDiagnosticPreview>,
    ) -> Option<Self> {
        (direct_webrtc.is_some() || diagnostic_preview.is_some()).then_some(Self {
            direct_webrtc,
            diagnostic_preview,
        })
    }

    fn settlement_status_until(&mut self, deadline: Instant) -> TransportSettlementStatus {
        let preview = self
            .diagnostic_preview
            .as_mut()
            .map(|preview| preview.settlement_status_until(deadline))
            .unwrap_or(TransportSettlementStatus::Settled);
        if preview == TransportSettlementStatus::Settled {
            self.diagnostic_preview = None;
        }
        let direct = self
            .direct_webrtc
            .as_mut()
            .map(|endpoint| endpoint.settlement_status_until(deadline))
            .unwrap_or(TransportSettlementStatus::Settled);
        if direct == TransportSettlementStatus::Settled {
            self.direct_webrtc = None;
        }
        if preview == TransportSettlementStatus::Pending
            || direct == TransportSettlementStatus::Pending
        {
            TransportSettlementStatus::Pending
        } else if preview == TransportSettlementStatus::Failed
            || direct == TransportSettlementStatus::Failed
        {
            TransportSettlementStatus::Failed
        } else {
            TransportSettlementStatus::Settled
        }
    }
}

impl TransportSettlementJob for RetiredSessionTransports {
    fn settlement_status_until(&mut self, deadline: Instant) -> TransportSettlementStatus {
        RetiredSessionTransports::settlement_status_until(self, deadline)
    }
}

/// Observe one terminal transport bundle under a bounded caller deadline. If
/// workers are still exiting, ownership moves to a component-only settler that
/// commits the durable Closing intent only after real completion receipts.
///
/// This worker deliberately retains stores rather than `RemoteDesktopPlugin`;
/// target-monitor shutdown therefore stays acyclic while the terminal state
/// machine still has one authoritative finalizer.
pub(in crate::daemon::plugins::remote_desktop) fn settle_session_transports_and_finish(
    settlement_queue: TransportSettlementQueue,
    sessions: Arc<RemoteDesktopSessionStore>,
    recovery: Arc<RemoteDesktopRecoveryStore>,
    relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
    session_id: String,
    transports: RetiredSessionTransports,
) -> TransportSettlementStatus {
    settle_session_transports_and_finish_until(
        settlement_queue,
        sessions,
        recovery,
        relay_lease_provider,
        session_id,
        transports,
        Instant::now() + TRANSPORT_SETTLEMENT_DEADLINE,
    )
}

fn settle_session_transports_and_finish_until(
    settlement_queue: TransportSettlementQueue,
    sessions: Arc<RemoteDesktopSessionStore>,
    recovery: Arc<RemoteDesktopRecoveryStore>,
    relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
    session_id: String,
    mut transports: RetiredSessionTransports,
    deadline: Instant,
) -> TransportSettlementStatus {
    match transports.settlement_status_until(deadline) {
        TransportSettlementStatus::Settled => {
            match commit_settled_session_termination(&sessions, &recovery, &session_id) {
                Ok(()) => {
                    release_terminal_relay_lease(
                        &sessions,
                        relay_lease_provider.as_ref(),
                        &session_id,
                    );
                    TransportSettlementStatus::Settled
                }
                Err(error) => {
                    eprintln!(
                        "[remote-desktop] terminal persistence remains pending for {session_id}: {error}"
                    );
                    settlement_queue.enqueue(SessionTerminationSettlementJob::new(
                        transports,
                        sessions,
                        recovery,
                        relay_lease_provider,
                        session_id,
                    ));
                    TransportSettlementStatus::Pending
                }
            }
        }
        TransportSettlementStatus::Failed => {
            settlement_queue.enqueue(SessionTerminationSettlementJob::new(
                transports,
                sessions,
                recovery,
                relay_lease_provider,
                session_id,
            ));
            TransportSettlementStatus::Failed
        }
        TransportSettlementStatus::Pending => {
            settlement_queue.enqueue(SessionTerminationSettlementJob::new(
                transports,
                sessions,
                recovery,
                relay_lease_provider,
                session_id,
            ));
            TransportSettlementStatus::Pending
        }
    }
}

struct SessionTerminationSettlementJob {
    transports: RetiredSessionTransports,
    sessions: Arc<RemoteDesktopSessionStore>,
    recovery: Arc<RemoteDesktopRecoveryStore>,
    relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
    session_id: String,
    next_commit_attempt: Instant,
    commit_retry_delay: Duration,
}

impl SessionTerminationSettlementJob {
    fn new(
        transports: RetiredSessionTransports,
        sessions: Arc<RemoteDesktopSessionStore>,
        recovery: Arc<RemoteDesktopRecoveryStore>,
        relay_lease_provider: Arc<dyn RemoteDesktopRelayLeaseProvider>,
        session_id: String,
    ) -> Self {
        Self {
            transports,
            sessions,
            recovery,
            relay_lease_provider,
            session_id,
            next_commit_attempt: Instant::now(),
            commit_retry_delay: Duration::from_millis(100),
        }
    }

    fn defer_commit_retry(&mut self) {
        self.next_commit_attempt = Instant::now() + self.commit_retry_delay;
        self.commit_retry_delay = (self.commit_retry_delay * 2).min(Duration::from_secs(5));
    }
}

impl TransportSettlementJob for SessionTerminationSettlementJob {
    fn settlement_status_until(&mut self, deadline: Instant) -> TransportSettlementStatus {
        match self.transports.settlement_status_until(deadline) {
            TransportSettlementStatus::Settled => {
                if Instant::now() < self.next_commit_attempt {
                    return TransportSettlementStatus::Pending;
                }
                match commit_settled_session_termination(
                    &self.sessions,
                    &self.recovery,
                    &self.session_id,
                ) {
                    Ok(()) => {
                        release_terminal_relay_lease(
                            &self.sessions,
                            self.relay_lease_provider.as_ref(),
                            &self.session_id,
                        );
                        TransportSettlementStatus::Settled
                    }
                    Err(error) => {
                        eprintln!(
                            "[remote-desktop] deferred terminal persistence remains pending for {}: {error}",
                            self.session_id
                        );
                        self.defer_commit_retry();
                        TransportSettlementStatus::Pending
                    }
                }
            }
            TransportSettlementStatus::Pending => TransportSettlementStatus::Pending,
            TransportSettlementStatus::Failed => {
                eprintln!(
                    "[remote-desktop] deferred transport settlement failed for {}; transferring to typed quarantine",
                    self.session_id
                );
                TransportSettlementStatus::Failed
            }
        }
    }

    fn context(&self) -> TransportSettlementJobContext {
        TransportSettlementJobContext {
            job_kind: "session_termination",
            session_id: Some(self.session_id.clone()),
        }
    }

    fn next_poll_at(&self) -> Option<Instant> {
        Some(self.next_commit_attempt)
    }

    fn project_quarantine(
        &mut self,
        _failure: TransportSettlementFailureKind,
    ) -> anyhow::Result<()> {
        commit_failed_session_termination(&self.sessions, &self.recovery, &self.session_id)?;
        release_terminal_relay_lease(
            &self.sessions,
            self.relay_lease_provider.as_ref(),
            &self.session_id,
        );
        Ok(())
    }
}

fn release_terminal_relay_lease(
    sessions: &RemoteDesktopSessionStore,
    provider: &dyn RemoteDesktopRelayLeaseProvider,
    session_id: &str,
) {
    let lease = sessions.with_sessions(|rows| {
        rows.get_mut(session_id).and_then(|session| {
            session
                .is_terminal()
                .then(|| session.retire_relay_lease("hub_relay_released"))
                .flatten()
        })
    });
    if let Some(lease) = lease {
        if let Err(error) = provider.release(&lease) {
            eprintln!(
                "[remote-desktop] Hub relay lease {} release failed after terminal commit: {error}",
                lease.lease_id()
            );
        }
    }
}

fn commit_settled_session_termination(
    sessions: &RemoteDesktopSessionStore,
    recovery: &RemoteDesktopRecoveryStore,
    session_id: &str,
) -> anyhow::Result<()> {
    let commit_lock = sessions.terminal_commit_lock(session_id);
    let _commit = match commit_lock.lock() {
        Ok(commit) => commit,
        Err(poisoned) => poisoned.into_inner(),
    };
    let prepared = prepare_settled_session_termination(sessions, recovery, session_id)?;
    let Some(prepared) = prepared else {
        return Ok(());
    };
    publish_settled_session_termination(sessions, recovery, session_id, prepared)
}

fn commit_failed_session_termination(
    sessions: &RemoteDesktopSessionStore,
    recovery: &RemoteDesktopRecoveryStore,
    session_id: &str,
) -> anyhow::Result<()> {
    let commit_lock = sessions.terminal_commit_lock(session_id);
    let _commit = match commit_lock.lock() {
        Ok(commit) => commit,
        Err(poisoned) => poisoned.into_inner(),
    };
    let prepared = prepare_session_termination(
        sessions,
        recovery,
        session_id,
        SessionTerminalOutcome::Failed,
    )?;
    let Some(prepared) = prepared else {
        return Ok(());
    };
    publish_settled_session_termination(sessions, recovery, session_id, prepared)
}

#[derive(Debug, Clone, Copy)]
enum SessionTerminalOutcome {
    Closed,
    Failed,
}

type SessionTerminationRevision = (u64, u64, Option<String>);

struct PreparedSessionTermination {
    source_revision: SessionTerminationRevision,
    terminal: RemoteDesktopSession,
    staged: RemoteDesktopRecoveryStagedSnapshot,
}

fn prepare_settled_session_termination(
    sessions: &RemoteDesktopSessionStore,
    recovery: &RemoteDesktopRecoveryStore,
    session_id: &str,
) -> anyhow::Result<Option<PreparedSessionTermination>> {
    prepare_session_termination(
        sessions,
        recovery,
        session_id,
        SessionTerminalOutcome::Closed,
    )
}

fn prepare_session_termination(
    sessions: &RemoteDesktopSessionStore,
    recovery: &RemoteDesktopRecoveryStore,
    session_id: &str,
    outcome: SessionTerminalOutcome,
) -> anyhow::Result<Option<PreparedSessionTermination>> {
    let candidate = sessions.with_sessions(|rows| -> anyhow::Result<Option<_>> {
        let Some(session) = rows.get(session_id) else {
            return Ok(None);
        };
        if !session.is_terminating() {
            return Ok(None);
        }
        let source_revision = (
            session.latest_event_sequence(),
            session.updated_at_ms(),
            session.termination_reason().map(ToString::to_string),
        );
        let mut terminal = session.clone();
        match outcome {
            SessionTerminalOutcome::Closed => terminal.finish_recovered_termination(now_ms()),
            SessionTerminalOutcome::Failed => {
                terminal.finish_failed_termination(REASON_TRANSPORT_SETTLEMENT_FAILED)
            }
        }
        let snapshot = RemoteDesktopRecoverySnapshot::from_session(&terminal)?;
        Ok(Some((source_revision, terminal, snapshot)))
    })?;
    let Some((source_revision, terminal, snapshot)) = candidate else {
        return Ok(None);
    };

    // Serialization, file creation, payload write, and fsync all happen in a
    // non-authoritative staging directory without holding the session map.
    // A process crash here leaves only an ignored staging artifact; recovery
    // can never observe the speculative terminal candidate.
    let staged = recovery.stage(snapshot)?;
    Ok(Some(PreparedSessionTermination {
        source_revision,
        terminal,
        staged,
    }))
}

fn publish_settled_session_termination(
    sessions: &RemoteDesktopSessionStore,
    recovery: &RemoteDesktopRecoveryStore,
    session_id: &str,
    prepared: PreparedSessionTermination,
) -> anyhow::Result<()> {
    let PreparedSessionTermination {
        source_revision,
        terminal,
        staged,
    } = prepared;

    let claimed = sessions.with_sessions(|rows| -> anyhow::Result<bool> {
        let Some(session) = rows.get_mut(session_id) else {
            return Ok(false);
        };
        if session.is_terminal() {
            return Ok(false);
        }
        let current_revision = (
            session.latest_event_sequence(),
            session.updated_at_ms(),
            session.termination_reason().map(ToString::to_string),
        );
        if !session.is_terminating() || current_revision != source_revision {
            anyhow::bail!(
                "terminal candidate revision changed after durable staging; retry required"
            );
        }
        if !session.begin_terminal_commit() {
            anyhow::bail!("terminal candidate revision already has a commit owner");
        }
        Ok(true)
    })?;
    if !claimed {
        return Ok(());
    }

    // The operational freeze above prevents final media/event mutations for
    // this exact Closing revision. Store locking, authoritative-file reads,
    // rename and parent fsync now block only this session's commit mutex, never
    // the global session map.
    RemoteDesktopSessionStore::assert_current_thread_unlocked("terminal recovery promotion");
    if let Err(error) = recovery.promote(staged) {
        sessions.with_sessions(|rows| {
            if let Some(session) = rows.get_mut(session_id) {
                let current_revision = (
                    session.latest_event_sequence(),
                    session.updated_at_ms(),
                    session.termination_reason().map(ToString::to_string),
                );
                if current_revision == source_revision && session.terminal_commit_in_progress() {
                    session.abort_terminal_commit();
                }
            }
        });
        return Err(error);
    }

    // Host E2E can terminate the process at the exact lost-response boundary:
    // the canonical terminal snapshot is authoritative and fsynced, while the
    // in-memory row and end_session RPC response are not yet published. Product
    // builds do not compile this hook.
    #[cfg(all(feature = "remoteapp-e2e-fault-injection", unix))]
    if let Some(terminal_receipt) = terminal.terminal_receipt() {
        crate::daemon::plugins::remote_desktop::e2e_fault_injection::maybe_crash_after_terminal_promotion(
            session_id,
            terminal.end_reason().unwrap_or_default(),
            &terminal_receipt,
        );
    }

    sessions.with_sessions(move |rows| -> anyhow::Result<()> {
        let Some(session) = rows.get_mut(session_id) else {
            anyhow::bail!("terminal session row disappeared after durable promotion");
        };
        let current_revision = (
            session.latest_event_sequence(),
            session.updated_at_ms(),
            session.termination_reason().map(ToString::to_string),
        );
        if current_revision != source_revision || !session.terminal_commit_in_progress() {
            anyhow::bail!(
                "terminal session revision changed despite commit freeze after durable promotion"
            );
        }
        *session = terminal;
        Ok(())
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_resource_identity(ability, env, args, session)?;
    ensure_session_liveness(plugin, ability, session)
}

pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_control_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_control_identity(ability, env, args, session)?;
    ensure_session_liveness(plugin, ability, session)
}

pub(in crate::daemon::plugins::remote_desktop) fn ensure_session_control_audit_access(
    plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    env: &EnvelopeContext,
    args: &Value,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    ensure_session_control_identity(ability, env, args, session)?;
    let _ = plugin;
    Ok(())
}

fn ensure_session_liveness(
    _plugin: &RemoteDesktopPlugin,
    ability: &'static str,
    session: &mut RemoteDesktopSession,
) -> anyhow::Result<()> {
    if session.is_expired_at(now_ms()) {
        return Err(RemoteDesktopError::SessionExpired {
            ability,
            session_id: session.session_id().to_string(),
        }
        .into());
    }
    ensure_not_terminal(ability, session)
}

fn ensure_not_terminal(
    ability: &'static str,
    session: &RemoteDesktopSession,
) -> anyhow::Result<()> {
    if session.is_terminal() || session.is_terminating() {
        return Err(RemoteDesktopError::SessionTerminal {
            ability,
            session_id: session.session_id().to_string(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::daemon::plugins::remote_desktop) fn stop_session_transports(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    session: &mut RemoteDesktopSession,
) {
    if let Some(endpoint) = retire_session_transports_unchecked(plugin, session_id, session) {
        plugin
            .transport_manager()
            .settlement_queue()
            .enqueue(endpoint);
    }
}

/// Opaque proof that the session's Closing intent reached durable storage.
///
/// Host-visible teardown APIs require this capability so no caller can detach
/// transports, cancel ownership, or signal stop before crash recovery has a
/// monotonic Closing row to consume.
#[derive(Debug)]
pub(in crate::daemon::plugins::remote_desktop) struct DurableClosingCheckpoint {
    session_id: String,
    reason: String,
}

impl DurableClosingCheckpoint {
    pub(in crate::daemon::plugins::remote_desktop) fn assert_matches(
        &self,
        session_id: &str,
        session: &RemoteDesktopSession,
    ) {
        assert_eq!(
            self.session_id, session_id,
            "Closing checkpoint session mismatch"
        );
        assert!(
            session.is_terminating(),
            "transport teardown requires Closing"
        );
        assert_eq!(
            session.termination_reason(),
            Some(self.reason.as_str()),
            "Closing checkpoint reason mismatch"
        );
    }
}

/// Persist a previously prepared write-ahead Closing intent. A failed write
/// leaves both the host transports and the in-memory session active, allowing
/// the owning trigger to retry safely.
pub(in crate::daemon::plugins::remote_desktop) fn commit_prepared_closing_checkpoint(
    recovery: &RemoteDesktopRecoveryStore,
    snapshot: &RemoteDesktopRecoverySnapshot,
) -> anyhow::Result<DurableClosingCheckpoint> {
    if snapshot.lifecycle_state() != "closing" {
        anyhow::bail!("durable Closing checkpoint requires a Closing snapshot");
    }
    let reason = snapshot
        .termination_reason()
        .ok_or_else(|| anyhow::anyhow!("durable Closing checkpoint requires a reason"))?;
    recovery.save(snapshot)?;
    Ok(DurableClosingCheckpoint {
        session_id: snapshot.session_id().to_string(),
        reason: reason.to_string(),
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn begin_session_transport_settlement(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    session: &mut RemoteDesktopSession,
    checkpoint: DurableClosingCheckpoint,
) -> Option<RetiredSessionTransports> {
    checkpoint.assert_matches(session_id, session);
    retire_session_transports_unchecked(plugin, session_id, session)
}

fn retire_session_transports_unchecked(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    session: &mut RemoteDesktopSession,
) -> Option<RetiredSessionTransports> {
    plugin.cancel_session_lease(session_id);
    plugin.cancel_session_target_tracking(session_id);
    if let Some(stop_tx) = session.detach_preview_transport() {
        let _ = stop_tx.send(true);
    }
    let transports = plugin.transport_manager();
    let diagnostic_preview = transports.take_preview_for_settlement(session_id);
    let direct_webrtc = transports.take_endpoint_for_settlement(session_id);
    RetiredSessionTransports::new(direct_webrtc, diagnostic_preview)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum SessionExpirationOutcome {
    NotDue,
    CheckpointPending,
    SettlementStarted,
}

/// Drive lease expiry without holding the global session map across recovery
/// I/O. Expiry is a trigger, not a durable termination intent: the write-ahead
/// Closing checkpoint must commit before any transport ownership is retired.
pub(in crate::daemon::plugins::remote_desktop) fn expire_session_by_id_if_needed(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    expected_lease_expires_at_ms: Option<u64>,
) -> SessionExpirationOutcome {
    let sessions = plugin.session_store();
    let operation_lock = sessions.target_operation_lock(session_id);
    let operation = match operation_lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let closing_intent = sessions.with_sessions(|sessions| {
        let session = sessions.get(session_id)?;
        if expected_lease_expires_at_ms
            .is_some_and(|expected| session.lease_expires_at_ms() != expected)
        {
            return None;
        }
        if session.is_terminal() || session.is_terminating() || !session.is_expired_at(now_ms()) {
            return None;
        }
        Some(RemoteDesktopRecoverySnapshot::prepare_closing_intent(
            session,
            REASON_SESSION_EXPIRED,
        ))
    });
    let Some(closing_intent) = closing_intent else {
        drop(operation);
        return SessionExpirationOutcome::NotDue;
    };
    RemoteDesktopSessionStore::assert_current_thread_unlocked("lease expiry recovery persistence");
    let checkpoint = match closing_intent.and_then(|snapshot| {
        commit_prepared_closing_checkpoint(plugin.recovery_store().as_ref(), &snapshot)
    }) {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            eprintln!(
                "[remote-desktop] lease expiry Closing checkpoint remains pending for {session_id}: {error}"
            );
            drop(operation);
            return SessionExpirationOutcome::CheckpointPending;
        }
    };
    let settlement = sessions.with_sessions(|rows| -> anyhow::Result<_> {
        let Some(session) = rows.get_mut(session_id) else {
            return Ok(None);
        };
        if expected_lease_expires_at_ms
            .is_some_and(|expected| session.lease_expires_at_ms() != expected)
            || session.is_terminal()
            || session.is_terminating()
            || !session.is_expired_at(now_ms())
            || !session.begin_expiration(now_ms())
        {
            return Ok(None);
        }
        let closing_snapshot = RemoteDesktopRecoverySnapshot::from_session(session)?;
        let transports =
            begin_session_transport_settlement(plugin, session_id, session, checkpoint)
                .unwrap_or_else(RetiredSessionTransports::empty);
        Ok(Some((closing_snapshot, transports)))
    });
    let Some((closing_snapshot, transports)) = (match settlement {
        Ok(settlement) => settlement,
        Err(error) => {
            eprintln!(
                "[remote-desktop] failed to enter lease expiry Closing state for {session_id}: {error}"
            );
            drop(operation);
            return SessionExpirationOutcome::CheckpointPending;
        }
    }) else {
        drop(operation);
        return SessionExpirationOutcome::NotDue;
    };
    if let Err(error) = plugin.persist_recovery_snapshot(&closing_snapshot) {
        eprintln!(
            "[remote-desktop] durable lease expiry intent exists for {session_id}, but the richer Closing projection remains pending: {error}"
        );
    }
    drop(operation);
    let _ = settle_session_transports_and_finish(
        plugin.transport_manager().settlement_queue(),
        plugin.session_store(),
        plugin.recovery_store(),
        plugin.relay_lease_provider(),
        session_id.to_string(),
        transports,
    );
    SessionExpirationOutcome::SettlementStarted
}

pub(in crate::daemon::plugins::remote_desktop) fn expire_session_from_watchdog(
    plugin: &RemoteDesktopPlugin,
    session_id: &str,
    expected_lease_expires_at_ms: u64,
) -> SessionExpirationOutcome {
    expire_session_by_id_if_needed(plugin, session_id, Some(expected_lease_expires_at_ms))
}

pub(in crate::daemon::plugins::remote_desktop) fn expire_inactive_sessions_by_id(
    plugin: &RemoteDesktopPlugin,
    now: u64,
) {
    let expired = plugin.session_store().with_sessions(|sessions| {
        sessions
            .iter()
            .filter(|(_, session)| !session.is_terminal() && session.is_expired_at(now))
            .map(|(session_id, session)| (session_id.clone(), session.lease_expires_at_ms()))
            .collect::<Vec<_>>()
    });
    for (session_id, expected_deadline) in expired {
        let _ = expire_session_by_id_if_needed(plugin, &session_id, Some(expected_deadline));
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn prune_terminal_sessions_locked(
    sessions: &mut HashMap<String, RemoteDesktopSession>,
) -> RemoteDesktopSessionPrune {
    let removed_session_ids =
        RemoteDesktopSessionStore::prune_terminal_rows_to_active_bound_locked(sessions);
    RemoteDesktopSessionPrune {
        removed_session_ids,
    }
}

pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopSessionPrune {
    pub(in crate::daemon::plugins::remote_desktop) removed_session_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    use serde_json::json;
    use tokio::sync::watch;

    use super::*;
    use crate::daemon::ability::builtins::resources::media::screen_snapshot::SyntheticScreenBackend;
    use crate::daemon::persistence::file_lock::ExclusiveFileLock;
    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::constants::{
        REASON_SESSION_EXPIRED, TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::relay_lease::UnavailableRemoteDesktopRelayLeaseProvider;
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::session_store::MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::test_support::{
        env_for, reset_store, seed_display, test_lock, test_plugin, test_runtime_limits,
        test_session_init, TestRemoteAppTargetBindingVerifier,
    };
    use crate::daemon::plugins::remote_desktop::transport::PreviewTaskGroupCompletion;

    #[test]
    fn terminal_persistence_failure_retains_closing_until_retry_commits() {
        let sessions = RemoteDesktopSessionStore::new();
        let temp = tempfile::tempdir().expect("temporary recovery parent creates");
        let recovery_root = temp.path().join("blocked-recovery-root");
        fs::write(&recovery_root, b"not-a-directory")
            .expect("file blocks recovery directory creation");
        let recovery = RemoteDesktopRecoveryStore::new(recovery_root.clone());
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-terminal-persistence-retry",
            "easynet:///r/acme/resource/display.terminal-persistence-retry",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        assert!(session.begin_close("caller_ended"));
        sessions.with_sessions(|rows| {
            rows.insert(session.session_id().to_string(), session);
        });

        assert!(commit_settled_session_termination(
            &sessions,
            &recovery,
            "rd-terminal-persistence-retry"
        )
        .is_err());
        sessions.with_sessions(|rows| {
            let session = rows.get("rd-terminal-persistence-retry").unwrap();
            assert!(session.is_terminating());
            assert!(!session.is_terminal());
        });

        fs::remove_file(&recovery_root).expect("blocking file removes");
        fs::create_dir(&recovery_root).expect("recovery root becomes writable directory");
        commit_settled_session_termination(&sessions, &recovery, "rd-terminal-persistence-retry")
            .expect("retry durably commits terminal candidate");
        sessions.with_sessions(|rows| {
            assert!(rows
                .get("rd-terminal-persistence-retry")
                .is_some_and(RemoteDesktopSession::is_terminal));
        });
        assert_eq!(
            recovery
                .load("rd-terminal-persistence-retry")
                .expect("terminal snapshot loads")
                .expect("terminal snapshot exists")
                .lifecycle_state(),
            "closed"
        );
    }

    #[test]
    fn stale_staged_terminal_never_replaces_newer_closing_revision() {
        let sessions = RemoteDesktopSessionStore::new();
        let temp = tempfile::tempdir().expect("temporary recovery root creates");
        let recovery = RemoteDesktopRecoveryStore::new(temp.path().to_path_buf());
        let epoch = TransportEpoch::new(41);
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-terminal-stage-cas",
            "easynet:///r/acme/resource/display.terminal-stage-cas",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        assert!(session.begin_webrtc_negotiation(epoch));
        assert!(session.begin_close("caller_ended"));
        let closing_snapshot = RemoteDesktopRecoverySnapshot::from_session(&session)
            .expect("Closing snapshot derives");
        recovery
            .save(&closing_snapshot)
            .expect("Closing snapshot becomes authoritative");
        sessions.with_sessions(|rows| {
            rows.insert(session.session_id().to_string(), session);
        });

        let prepared =
            prepare_settled_session_termination(&sessions, &recovery, "rd-terminal-stage-cas")
                .expect("terminal candidate stages")
                .expect("Closing session has a terminal candidate");

        // Simulate final media statistics arriving after staging but before
        // publication. This advances the aggregate revision while the staged
        // candidate remains deliberately non-authoritative.
        sessions.record_media_pipeline_stats(
            "rd-terminal-stage-cas",
            epoch,
            json!({"terminal": true, "frames_encoded": 73}),
        );
        let error = publish_settled_session_termination(
            &sessions,
            &recovery,
            "rd-terminal-stage-cas",
            prepared,
        )
        .expect_err("stale staged terminal must fail revision publication");
        assert!(error.to_string().contains("revision changed"));
        sessions.with_sessions(|rows| {
            let session = rows.get("rd-terminal-stage-cas").unwrap();
            assert!(session.is_terminating());
            assert!(!session.is_terminal());
        });
        assert_eq!(
            recovery
                .load("rd-terminal-stage-cas")
                .expect("authoritative snapshot loads")
                .expect("authoritative Closing snapshot remains"),
            closing_snapshot,
            "a failed revision CAS must not publish the stale terminal stage"
        );

        commit_settled_session_termination(&sessions, &recovery, "rd-terminal-stage-cas")
            .expect("retry stages and publishes the new revision");
        let terminal = recovery
            .load("rd-terminal-stage-cas")
            .expect("terminal snapshot loads")
            .expect("terminal snapshot exists");
        assert_eq!(terminal.lifecycle_state(), "closed");
        assert!(terminal.events().iter().any(|event| {
            event["event_type"] == json!("MEDIA_PIPELINE_STATS")
                && event["payload"]["stats"]["frames_encoded"] == json!(73)
        }));
    }

    #[test]
    fn terminal_promotion_blocks_only_its_session_commit_boundary() {
        let sessions = Arc::new(RemoteDesktopSessionStore::new());
        let temp = tempfile::tempdir().expect("temporary recovery root creates");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let epoch = TransportEpoch::new(77);
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-terminal-lock-boundary",
            "easynet:///r/acme/resource/display.terminal-lock-boundary",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        assert!(session.begin_webrtc_negotiation(epoch));
        assert!(session.begin_close("caller_ended"));
        sessions.with_sessions(|rows| {
            rows.insert(session.session_id().to_string(), session);
        });
        let store_lock =
            ExclusiveFileLock::acquire_for_data_path(&temp.path().join(".recovery-store"))
                .expect("test owns recovery store lock");
        let sessions_for_commit = Arc::clone(&sessions);
        let recovery_for_commit = Arc::clone(&recovery);
        let commit = std::thread::spawn(move || {
            commit_settled_session_termination(
                &sessions_for_commit,
                &recovery_for_commit,
                "rd-terminal-lock-boundary",
            )
        });

        let freeze_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let frozen = sessions.with_sessions(|rows| {
                rows.get("rd-terminal-lock-boundary")
                    .is_some_and(RemoteDesktopSession::terminal_commit_in_progress)
            });
            if frozen {
                break;
            }
            assert!(
                Instant::now() < freeze_deadline,
                "terminal finalizer did not claim its aggregate revision"
            );
            std::thread::yield_now();
        }

        let sequence_before = sessions.with_sessions(|rows| {
            rows.get("rd-terminal-lock-boundary")
                .expect("session remains present")
                .latest_event_sequence()
        });
        sessions.record_media_pipeline_stats(
            "rd-terminal-lock-boundary",
            epoch,
            json!({"terminal": true, "frames_encoded": 99}),
        );
        let sequence_after = sessions.with_sessions(|rows| {
            rows.get("rd-terminal-lock-boundary")
                .expect("session remains present")
                .latest_event_sequence()
        });
        assert_eq!(
            sequence_after, sequence_before,
            "terminal commit freeze accepted a late media mutation"
        );

        let (access_tx, access_rx) = mpsc::channel();
        let sessions_for_access = Arc::clone(&sessions);
        let access = std::thread::spawn(move || {
            let row_count = sessions_for_access.with_sessions(|rows| rows.len());
            let _ = access_tx.send(row_count);
        });
        let access_result = access_rx.recv_timeout(Duration::from_millis(250));
        drop(store_lock);
        assert_eq!(
            access_result.expect("unrelated session-map access cannot wait on recovery I/O"),
            1
        );
        access.join().expect("session-map access thread joins");
        commit
            .join()
            .expect("terminal commit thread joins")
            .expect("terminal promotion completes after store lock release");
        sessions.with_sessions(|rows| {
            assert!(rows
                .get("rd-terminal-lock-boundary")
                .is_some_and(RemoteDesktopSession::is_terminal));
        });
    }

    #[test]
    fn quarantined_session_termination_publishes_durable_failed_outcome() {
        let sessions = Arc::new(RemoteDesktopSessionStore::new());
        let temp = tempfile::tempdir().expect("temporary recovery root creates");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-quarantined-terminal",
            "easynet:///r/acme/resource/display.quarantined-terminal",
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        assert!(session.begin_close("caller_ended"));
        let closing = RemoteDesktopRecoverySnapshot::from_session(&session)
            .expect("Closing snapshot derives");
        recovery.save(&closing).expect("Closing snapshot persists");
        sessions.with_sessions(|rows| {
            rows.insert(session.session_id().to_string(), session);
        });
        let mut job = SessionTerminationSettlementJob::new(
            RetiredSessionTransports::empty(),
            Arc::clone(&sessions),
            Arc::clone(&recovery),
            Arc::new(UnavailableRemoteDesktopRelayLeaseProvider),
            "rd-quarantined-terminal".to_string(),
        );

        job.project_quarantine(TransportSettlementFailureKind::ExplicitFailure)
            .expect("quarantine publishes durable Failed outcome");

        sessions.with_sessions(|rows| {
            let failed = rows.get("rd-quarantined-terminal").unwrap();
            assert!(failed.is_terminal());
            assert_eq!(failed.state().json_name(), "failed");
            assert_eq!(
                failed.termination_reason(),
                Some(REASON_TRANSPORT_SETTLEMENT_FAILED)
            );
            assert_eq!(
                failed.terminal_receipt().unwrap()["reason_code"],
                json!(REASON_TRANSPORT_SETTLEMENT_FAILED)
            );
        });
        let durable = recovery
            .load("rd-quarantined-terminal")
            .expect("Failed snapshot loads")
            .expect("Failed snapshot exists");
        assert_eq!(durable.lifecycle_state(), "failed");
        assert_eq!(
            durable.terminal_receipt().unwrap()["reason_code"],
            json!(REASON_TRANSPORT_SETTLEMENT_FAILED)
        );
    }

    #[test]
    fn expired_session_allows_audit_read_and_then_idempotent_end() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-expired-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-expired-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();
        {
            plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get_mut("rd-expired-test")
                    .unwrap()
                    .set_lease_expires_at_for_test(now_ms().saturating_sub(1));
            });
        }

        let shown = crate::daemon::plugins::remote_desktop::handlers::show_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-test", "session_token": token.clone()}),
        )
        .unwrap();
        assert_eq!(shown["state"], json!("closed"));
        assert_eq!(shown["end_reason"], json!(REASON_SESSION_EXPIRED));

        let ended = crate::daemon::plugins::remote_desktop::handlers::end_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-test", "session_token": token.clone()}),
        )
        .unwrap();
        assert_eq!(ended["already_ended"], json!(true));
        assert_eq!(ended["end_reason"], json!(REASON_SESSION_EXPIRED));
    }

    #[test]
    fn expired_session_stops_preview_worker() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-expired-preview-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-expired-preview-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let token = created["session_token"].as_str().unwrap().to_string();
        let (stop_tx, mut stop_rx) = watch::channel(false);
        {
            plugin.session_store().with_sessions(|sessions| {
                let session = sessions.get_mut("rd-expired-preview-test").unwrap();
                session.install_preview_transport_for_test(stop_tx);
                session.set_lease_expires_at_for_test(now_ms().saturating_sub(1));
            });
        }

        let shown = crate::daemon::plugins::remote_desktop::handlers::show_session::handle(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({"session_id": "rd-expired-preview-test", "session_token": token}),
        )
        .unwrap();

        assert_eq!(shown["end_reason"], json!(REASON_SESSION_EXPIRED));
        assert!(
            *stop_rx.borrow_and_update(),
            "lease expiry must signal the preview worker stop channel"
        );
    }

    #[test]
    fn lease_watchdog_terminal_path_closes_transports_without_rpc() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-watchdog-display");
        resources::save(&file).unwrap();

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-watchdog-test",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();
        let original_lease = created["lease_expires_at_ms"].as_u64().unwrap();
        assert!(original_lease > now_ms());
        let expected_lease = now_ms().saturating_sub(1);
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let preview_epoch = plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get_mut("rd-watchdog-test").unwrap();
            let preview_epoch = session.install_preview_transport_for_test(stop_tx.clone());
            session.set_lease_expires_at_for_test(expected_lease);
            preview_epoch
        });
        let (preview_done_tx, preview_done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            "rd-watchdog-test".to_string(),
            preview_epoch,
            stop_tx,
            preview_done_rx,
        );

        expire_session_from_watchdog(&plugin, "rd-watchdog-test", expected_lease);

        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get("rd-watchdog-test").unwrap();
            assert!(session.is_terminating());
            assert!(!session.is_terminal());
            assert_eq!(session.end_reason(), Some(REASON_SESSION_EXPIRED));
        });
        assert!(
            *stop_rx.borrow_and_update(),
            "watchdog expiry must signal active transports"
        );
        let closing_snapshot = plugin
            .recovery_store()
            .load("rd-watchdog-test")
            .expect("watchdog Closing snapshot loads")
            .expect("watchdog persists Closing before worker completion");
        assert_eq!(closing_snapshot.lifecycle_state(), "closing");

        preview_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("preview task group reports completion");
        let state_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let terminal = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get("rd-watchdog-test")
                    .is_some_and(|session| session.is_terminal())
            });
            if terminal {
                break;
            }
            assert!(
                Instant::now() < state_deadline,
                "watchdog expiration did not commit Closed after preview completion"
            );
            std::thread::yield_now();
        }
        let persistence_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let terminal_snapshot = plugin
                .recovery_store()
                .load("rd-watchdog-test")
                .expect("watchdog terminal snapshot loads")
                .expect("watchdog persists terminal snapshot");
            if terminal_snapshot.lifecycle_state() == "closed" {
                break;
            }
            assert!(
                Instant::now() < persistence_deadline,
                "watchdog did not durably publish Closed after in-memory settlement"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn lease_expiry_checkpoint_failure_keeps_transport_and_retry_ownership() {
        let _lock = test_lock();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("temporary recovery root");
        let recovery = Arc::new(RemoteDesktopRecoveryStore::new(temp.path().to_path_buf()));
        let plugin = RemoteDesktopPlugin::with_recovery_store_for_test(
            Arc::new(SyntheticScreenBackend),
            Arc::new(TestRemoteAppTargetBindingVerifier),
            test_runtime_limits().into(),
            Arc::clone(&recovery),
        );
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-expiry-checkpoint-display");
        resources::save(&file).expect("test resource saves");
        crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-expiry-checkpoint-failure",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .expect("test session creates");
        let expected_lease = now_ms().saturating_sub(1);
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let preview_epoch = plugin.session_store().with_sessions(|rows| {
            let session = rows
                .get_mut("rd-expiry-checkpoint-failure")
                .expect("session exists");
            let epoch = session.install_preview_transport_for_test(stop_tx.clone());
            session.set_lease_expires_at_for_test(expected_lease);
            epoch
        });
        let (done_tx, done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            "rd-expiry-checkpoint-failure".to_string(),
            preview_epoch,
            stop_tx,
            done_rx,
        );
        recovery.set_fail_saves_for_test(true);

        assert_eq!(
            expire_session_from_watchdog(&plugin, "rd-expiry-checkpoint-failure", expected_lease,),
            SessionExpirationOutcome::CheckpointPending
        );
        assert!(
            !*stop_rx.borrow_and_update(),
            "preview stop must not be sent"
        );
        plugin.session_store().with_sessions(|rows| {
            let session = rows
                .get("rd-expiry-checkpoint-failure")
                .expect("session remains");
            assert!(!session.is_terminating());
            assert!(!session.is_terminal());
            assert!(session.preview_attached());
        });

        recovery.set_fail_saves_for_test(false);
        assert_eq!(
            expire_session_from_watchdog(&plugin, "rd-expiry-checkpoint-failure", expected_lease,),
            SessionExpirationOutcome::SettlementStarted
        );
        assert!(*stop_rx.borrow_and_update(), "retry sends preview stop");
        let durable = recovery
            .load("rd-expiry-checkpoint-failure")
            .expect("Closing snapshot loads")
            .expect("Closing snapshot exists before stop completion");
        assert_eq!(durable.lifecycle_state(), "closing");
        done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("preview worker completion sends");
        reset_store(&plugin);
    }

    #[test]
    fn deferred_settler_retains_ownership_and_finishes_after_initial_timeout() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-deferred-settlement-display");
        resources::save(&file).unwrap();

        crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-deferred-settlement",
                "mode": "view_only",
            }),
        )
        .expect("test session creates");
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let preview_epoch = plugin.session_store().with_sessions(|sessions| {
            sessions
                .get_mut("rd-deferred-settlement")
                .expect("session exists")
                .install_preview_transport_for_test(stop_tx.clone())
        });
        let (preview_done_tx, preview_done_rx) = mpsc::channel();
        plugin.transport_manager().activate_preview(
            "rd-deferred-settlement".to_string(),
            preview_epoch,
            stop_tx,
            preview_done_rx,
        );
        let closing_intent = plugin.session_store().with_sessions(|sessions| {
            RemoteDesktopRecoverySnapshot::prepare_closing_intent(
                sessions
                    .get("rd-deferred-settlement")
                    .expect("session exists"),
                "caller_ended",
            )
            .expect("Closing intent derives")
        });
        let checkpoint =
            commit_prepared_closing_checkpoint(plugin.recovery_store().as_ref(), &closing_intent)
                .expect("Closing intent persists");
        let retired = plugin.session_store().with_sessions(|sessions| {
            let session = sessions
                .get_mut("rd-deferred-settlement")
                .expect("session exists");
            assert!(session.begin_close("caller_ended"));
            begin_session_transport_settlement(
                &plugin,
                "rd-deferred-settlement",
                session,
                checkpoint,
            )
            .expect("preview completion ownership is retired")
        });

        let settlement = settle_session_transports_and_finish_until(
            plugin.transport_manager().settlement_queue(),
            plugin.session_store(),
            plugin.recovery_store(),
            plugin.relay_lease_provider(),
            "rd-deferred-settlement".to_string(),
            retired,
            Instant::now() + Duration::from_millis(20),
        );
        assert_eq!(settlement, TransportSettlementStatus::Pending);
        assert!(*stop_rx.borrow_and_update());
        plugin.session_store().with_sessions(|sessions| {
            let session = sessions.get("rd-deferred-settlement").unwrap();
            assert!(session.is_terminating());
            assert!(!session.is_terminal());
        });

        preview_done_tx
            .send(PreviewTaskGroupCompletion)
            .expect("preview worker publishes its real completion receipt");
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let terminal = plugin.session_store().with_sessions(|sessions| {
                sessions
                    .get("rd-deferred-settlement")
                    .is_some_and(|session| session.is_terminal())
            });
            if terminal {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "deferred settler lost transport ownership after its initial deadline"
            );
            std::thread::yield_now();
        }
        let snapshot = plugin
            .recovery_store()
            .load("rd-deferred-settlement")
            .expect("terminal recovery snapshot loads")
            .expect("deferred settler persists terminal recovery snapshot");
        assert_eq!(snapshot.lifecycle_state(), "closed");
    }

    #[test]
    fn create_session_ignores_terminal_tombstones_for_capacity_check() {
        let _lock = test_lock();
        let plugin = test_plugin();
        reset_store(&plugin);
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "remote-desktop-capacity-display");
        resources::save(&file).unwrap();
        {
            let mut stale_sessions = Vec::new();
            for index in 0..plugin.config().max_sessions() {
                let now = now_ms();
                let session_id = format!("stale-{index}");
                let mut session = RemoteDesktopSession::new(test_session_init(
                    &session_id,
                    &ura,
                    vec![TRANSPORT_WEBRTC.to_string()],
                ));
                session.set_lease_expires_at_for_test(now.saturating_sub(1));
                assert!(session.begin_close("test_stale"));
                session.finish_close("test_stale");
                stale_sessions.push((session_id, session));
            }
            plugin.session_store().with_sessions(|sessions| {
                for (session_id, session) in stale_sessions {
                    sessions.insert(session_id, session);
                }
            });
        }

        let created = crate::daemon::plugins::remote_desktop::test_support::create_test_session(
            Arc::clone(&plugin),
            env_for(&ura),
            json!({
                "session_id": "rd-after-prune",
                "mode": "view_only",
                "lease_ttl_ms": 5000,
            }),
        )
        .unwrap();

        assert_eq!(created["session_id"], json!("rd-after-prune"));
        plugin.session_store().with_sessions(|sessions| {
            let active_sessions = sessions
                .values()
                .filter(|session| !session.is_terminal())
                .count();
            let terminal_rows = sessions
                .values()
                .filter(|session| session.is_terminal())
                .count();
            assert_eq!(active_sessions, 1);
            assert!(
                terminal_rows
                    <= active_sessions.saturating_mul(MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION)
            );
            assert!(sessions.contains_key("rd-after-prune"));
        });
    }
}
