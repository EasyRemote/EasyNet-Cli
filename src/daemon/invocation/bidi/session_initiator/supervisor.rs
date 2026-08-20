use std::time::Duration;

use rand::RngCore as _;

use super::{SESSION_BACKOFF_INITIAL, SESSION_BACKOFF_MAX};

/// Minimum uptime for a cleanly-closed `session.open` to count as
/// healthy and earn a backoff reset.
///
/// A clean down-stream EOF is NOT sufficient evidence of a healthy
/// session: hub-side presence displacement (a second claimant of the
/// same caller URA), contract-skew teardown by an older hub build,
/// and rolling hub restarts all end the stream cleanly moments after
/// admission. Resetting to `SESSION_BACKOFF_INITIAL` on every clean
/// close therefore locks the supervisor into a fixed-cadence
/// reconnect hammer — incident 2026-06-11 sustained 5428
/// open → admission → clean-close cycles at the 250 ms floor because
/// the exponential curve never engaged.
///
/// 30 s spans several device-heartbeat/Hub-acknowledgement exchanges at the
/// 5 s cadence: a session that completed those round trips was genuinely live,
/// while displacement ping-pong (sub-second) and first-heartbeat teardowns
/// (~5 s) stay on the escalating schedule toward `SESSION_BACKOFF_MAX`.
pub const SESSION_HEALTHY_MIN_UPTIME: Duration = Duration::from_secs(30);

/// Device-side fingerprint of a cleanly-closed `session.open`,
/// reported by `dial_and_run_session*` on `Ok`.
///
/// This is the evidence record for diagnosing hub-side close causes
/// when hub logs are unavailable (incident 2026-06-11: the hub
/// container's logs were lost with the process; only device-side
/// correlation survived). [`SessionCloseStats::classify`] is the
/// single point that turns the fingerprint into a [`CloseClass`].
///
/// It is NOT an error type: error exits keep returning
/// `SessionError`, which carries its own diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCloseStats {
    /// Wall-clock from bidi acceptance (`invoke_bidi` returned the
    /// down stream) to down-stream EOF.
    pub uptime: Duration,
    /// Down frames received after acceptance: admission receipt,
    /// keepalives, and business dispatches all count.
    pub frames_received: u64,
}

/// First-class classification of a clean `session.open` close
/// (F-008 / T1.1: the close class drives backoff policy and ops
/// alerting; while it lived as prose + ad-hoc comparisons, the
/// 2026-06-11 displacement ping-pong stayed invisible until 5,428
/// cycles had passed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseClass {
    /// `uptime ≥ SESSION_HEALTHY_MIN_UPTIME` — ordinary close of a
    /// genuinely live session (hub shutdown, deploy). The only class
    /// that earns a backoff reset.
    Healthy,
    /// Sub-second uptime with at most the admission receipt seen —
    /// the displacement signature: a second claimant of the same
    /// caller URA replaced this session.
    DisplacedSuspect,
    /// Zero down frames: the hub accepted the RPC but never sent the
    /// RFC-003 §1.1 admission receipt (pre-2026-05-02 hub build).
    NoAdmissionReceipt,
    /// Closed after a normal admission but before healthy uptime —
    /// first-heartbeat teardown (up-frame contract skew) and
    /// rolling-restart races land here.
    ContractSkew,
}

impl CloseClass {
    /// Stable lowercase token for op_event fields and dashboards.
    pub fn as_str(&self) -> &'static str {
        match self {
            CloseClass::Healthy => "healthy",
            CloseClass::DisplacedSuspect => "displaced_suspect",
            CloseClass::NoAdmissionReceipt => "no_admission_receipt",
            CloseClass::ContractSkew => "contract_skew",
        }
    }
}

impl SessionCloseStats {
    /// The fingerprint table, in one place. Order matters: healthy
    /// uptime wins outright; a frameless session is the missing
    /// admission receipt regardless of duration; sub-second with only
    /// the receipt is displacement; everything else that died young
    /// is contract skew.
    pub fn classify(&self) -> CloseClass {
        if self.uptime >= SESSION_HEALTHY_MIN_UPTIME {
            CloseClass::Healthy
        } else if self.frames_received == 0 {
            CloseClass::NoAdmissionReceipt
        } else if self.uptime < Duration::from_secs(1) && self.frames_received <= 1 {
            CloseClass::DisplacedSuspect
        } else {
            CloseClass::ContractSkew
        }
    }
}

/// Typed phase of one supervised `session.open` (F-008 / T1.1).
///
/// The supervisor's control flow DRIVES transitions; the phase type
/// makes "what stage is this device in" a queryable, observable fact
/// instead of a position inside `dial_and_run_session*`'s control
/// flow. Macro-phase edges are strict (see `may_transition_to`);
/// ordering WITHIN the prelude is deliberately loose — prelude steps
/// are the dial function's business and may reorder as the protocol
/// evolves, while the op_event stream still records the actual
/// sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSessionPhase {
    /// Supervisor constructed, no dial attempted yet — or shut down.
    Idle,
    /// Credential warmup + endpoint connect in progress.
    Dialing,
    /// Channel up; running the session preludes.
    Preluding(PreludeStep),
    /// Bidi accepted (`bidi_opened`); frame loop running.
    Live,
    /// Between attempts, waiting out the backoff curve.
    Backoff,
}

/// The session preludes, in their current wire order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreludeStep {
    Join,
    OwnerProjection,
    TrustBootstrap,
    Advertise,
}

impl DeviceSessionPhase {
    /// Stable lowercase token for op_event fields and dashboards.
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceSessionPhase::Idle => "idle",
            DeviceSessionPhase::Dialing => "dialing",
            DeviceSessionPhase::Preluding(PreludeStep::Join) => "preluding_join",
            DeviceSessionPhase::Preluding(PreludeStep::OwnerProjection) => {
                "preluding_owner_projection"
            }
            DeviceSessionPhase::Preluding(PreludeStep::TrustBootstrap) => {
                "preluding_trust_bootstrap"
            }
            DeviceSessionPhase::Preluding(PreludeStep::Advertise) => "preluding_advertise",
            DeviceSessionPhase::Live => "live",
            DeviceSessionPhase::Backoff => "backoff",
        }
    }

    /// The legal edge relation of the session state machine.
    ///
    /// * Any phase may drop to `Backoff` (failure exits) and any
    ///   phase may return to `Idle` (supervisor shutdown).
    /// * Forward progress is strict: `Idle|Backoff → Dialing →
    ///   Preluding → Live`; no phase may skip into `Live`.
    pub fn may_transition_to(&self, to: &DeviceSessionPhase) -> bool {
        use DeviceSessionPhase::*;
        match (self, to) {
            // Shutdown and failure edges are always available.
            (_, Idle) | (_, Backoff) => true,
            (Idle, Dialing) | (Backoff, Dialing) => true,
            (Dialing, Preluding(_)) => true,
            // Prelude steps may chain in any order (loose-by-design),
            // and only a prelude may open the bidi.
            (Preluding(_), Preluding(_)) | (Preluding(_), Live) => true,
            _ => false,
        }
    }
}

/// The single transition point of the session state machine
/// (F-008 / T1.1: 转移函数集中一处). Every phase change emits one
/// `session_state_transition{from,to,attempt,reason}` op_event —
/// alerting and SLO tooling consume the transition stream instead of
/// grepping scattered log kinds. Illegal edges are a bookkeeping bug:
/// debug builds assert; release builds emit
/// `session_phase_violation` and continue — a daemon must not die
/// for an observability defect.
pub(super) struct SessionPhaseTracker {
    phase: DeviceSessionPhase,
    attempt: u64,
}

impl SessionPhaseTracker {
    pub(super) fn new() -> Self {
        Self {
            phase: DeviceSessionPhase::Idle,
            attempt: 0,
        }
    }

    /// Begin a dial attempt: bumps the attempt counter and enters
    /// `Dialing`. The counter is per-supervisor, monotonically
    /// increasing across reconnects — it correlates the transition
    /// stream with the backoff curve.
    pub(super) fn begin_attempt(&mut self) {
        self.attempt += 1;
        self.transition(DeviceSessionPhase::Dialing, "dial_attempt");
    }

    /// The current phase — for status surfaces and tests.
    #[cfg(test)]
    pub(super) fn phase(&self) -> DeviceSessionPhase {
        self.phase
    }

    pub(super) fn transition(&mut self, to: DeviceSessionPhase, reason: &str) {
        let from = self.phase;
        if from == to {
            return;
        }
        let legal = from.may_transition_to(&to);
        debug_assert!(
            legal,
            "illegal session phase transition {from:?} → {to:?} (reason: {reason})"
        );
        if !legal {
            let from_str = from.as_str();
            let to_str = to.as_str();
            crate::op_event!(
                component = session,
                kind = session_phase_violation,
                from = from_str,
                to = to_str,
                attempt = self.attempt,
                reason = reason,
            );
        }
        let from_str = from.as_str();
        let to_str = to.as_str();
        self.phase = to;
        crate::op_event!(
            component = session,
            kind = session_state_transition,
            from = from_str,
            to = to_str,
            attempt = self.attempt,
            reason = reason,
        );
    }
}

pub(super) fn next_backoff(current: Duration) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > SESSION_BACKOFF_MAX {
        SESSION_BACKOFF_MAX
    } else {
        doubled
    }
}

/// Full-jitter sample of a backoff bound: a uniform draw in
/// `[0, bound]`. AWS's "Exponential Backoff And Jitter" full-jitter
/// variant — it minimizes the collision probability of a fleet that
/// retries in lockstep, which is exactly the hub-restart thundering
/// herd. The deterministic `bound` (the doubling curve) is preserved
/// as the ceiling; only the per-attempt WAIT is randomized, so the
/// curve's escalation and reset semantics are untouched.
pub(super) fn full_jitter(bound: Duration) -> Duration {
    let ms = bound.as_millis() as u64;
    if ms == 0 {
        return Duration::ZERO;
    }
    Duration::from_millis(rand::rngs::OsRng.next_u64() % (ms + 1))
}

/// Backoff policy for a session that ended with a clean hub-side
/// close (`Ok` from `dial_and_run_session*`). Only a session that
/// stayed up at least `SESSION_HEALTHY_MIN_UPTIME` earns the reset
/// to `SESSION_BACKOFF_INITIAL`; a shorter-lived clean close keeps
/// the current backoff, which the supervisor then doubles after the
/// sleep exactly as it does for error exits. See
/// `SESSION_HEALTHY_MIN_UPTIME` for why a clean close alone must
/// not reset the curve (incident 2026-06-11, 5428-cycle loop).
pub(super) fn backoff_after_clean_close(stats: &SessionCloseStats, current: Duration) -> Duration {
    match stats.classify() {
        CloseClass::Healthy => SESSION_BACKOFF_INITIAL,
        // Every unhealthy class keeps the escalating curve — the
        // 2026-06-11 lesson: a clean EOF is not evidence of health.
        CloseClass::DisplacedSuspect
        | CloseClass::NoAdmissionReceipt
        | CloseClass::ContractSkew => current,
    }
}
