// EasyNet CLI — Device-session connection-state projection
// =========================================================
//
// The session supervisor observes transport lifecycle, but it does not own
// product-state persistence.  This port keeps that dependency explicit: the
// daemon boot assembly supplies the filesystem-backed adapter, while tests can
// supply an isolated in-memory sink without mutating process-global HOME.

use crate::daemon::boot::join_connection_state::{
    JoinConnectionSnapshot, JoinConnectionState, JoinTransition,
};

/// One state-machine transition observed by the device-session runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionConnectionStateChange {
    pub(crate) state: JoinConnectionState,
    pub(crate) transition: JoinTransition,
    pub(crate) source: String,
}

impl SessionConnectionStateChange {
    #[must_use]
    pub(crate) fn new(
        state: JoinConnectionState,
        transition: JoinTransition,
        source: impl Into<String>,
    ) -> Self {
        Self {
            state,
            transition,
            source: source.into(),
        }
    }
}

/// Output port for session-observed product connection state.
///
/// Implementations own storage and isolation. The supervisor only emits
/// semantic transitions and never reaches into global filesystem state.
pub(crate) trait SessionConnectionStateSink: Send + Sync {
    fn record(&self, change: SessionConnectionStateChange) -> anyhow::Result<()>;
}

/// Production adapter preserving the existing connection-state.json contract.
#[derive(Debug, Default)]
pub(crate) struct PersistentSessionConnectionStateSink;

impl SessionConnectionStateSink for PersistentSessionConnectionStateSink {
    fn record(&self, change: SessionConnectionStateChange) -> anyhow::Result<()> {
        let prior = crate::daemon::boot::join_connection_state::latest_snapshot();
        let snapshot = JoinConnectionSnapshot::from_parts(
            change.state,
            Some(change.transition),
            prior.realm,
            prior.node_id,
            prior.hub_endpoint,
            change.source,
        );
        crate::daemon::boot::join_connection_state::save_snapshot(&snapshot)
    }
}

/// Project one transition and keep operator events consistent across every
/// sink implementation. A storage failure is observable and fail-closed to the
/// caller as `false`; it does not terminate the long-lived session supervisor.
pub(crate) fn project_connection_state(
    sink: &dyn SessionConnectionStateSink,
    state: JoinConnectionState,
    transition: JoinTransition,
    source: &str,
) -> bool {
    let state_code = state.code();
    let transition_id = transition.id();
    let change = SessionConnectionStateChange::new(state, transition, source);
    match sink.record(change) {
        Ok(()) => {
            crate::op_event!(
                component = session,
                kind = connection_state_projected,
                state_code = state_code,
                transition_id = transition_id,
                source = source,
            );
            true
        }
        Err(err) => {
            let message = format!("{err:#}");
            crate::op_event!(
                component = session,
                kind = connection_state_projection_failed,
                state_code = state_code,
                transition_id = transition_id,
                source = source,
                error = message,
            );
            false
        }
    }
}
