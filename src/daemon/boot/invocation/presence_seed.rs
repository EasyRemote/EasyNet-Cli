use std::sync::Arc;

use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::persistence::daemon_config::DaemonMode;

pub(super) fn seed_boot_presence(
    mode: DaemonMode,
    daemon_ura: Option<&str>,
    presence: &Arc<PresenceRegistry>,
) {
    if matches!(mode, DaemonMode::Device) {
        seed_device_mode_self_presence(daemon_ura, presence);
    }
}

fn seed_device_mode_self_presence(daemon_ura: Option<&str>, presence: &Arc<PresenceRegistry>) {
    let Some(ura) = daemon_ura else {
        return;
    };

    // Device-mode self-presence seed.
    //
    // In device-mode the daemon's local PresenceRegistry is used by
    // backend's `federation.resolve` (over the daemon UDS) to answer
    // "which devices in this realm are online?". The hub-side
    // presence registry holds the canonical answer, but in
    // host-mode dev rigs (backend -> device daemon UDS, no separate
    // hub-mode daemon process) the backend never reaches the hub's
    // presence — it queries this daemon's local one instead.
    // Pre-this-fix: device daemon's local presence was empty because
    // session.open is an OUTBOUND dial (the daemon dials the hub),
    // not an inbound register, so nothing populated the local table.
    // backend's `federation.resolve` then returned no agents; every
    // device showed REMOVED in /api/v1/devices despite the bidi
    // being healthy.
    //
    // Seed directory-visible presence with the daemon's own URA on boot so the
    // local resolve answers "yes I'm here" when the operator's backend asks.
    // This is now an explicit resolve-only slot: it has no dispatch sender and
    // cannot be selected by `require_canonical_dispatch_session`.
    match presence.insert_resolve_only(ura.to_string()) {
        Ok(registration) if registration.displaced.is_none() => {
            crate::op_event!(
                component = daemon_invocation,
                kind = device_mode_self_presence_seeded,
                self_ura = ura,
                message =
                    "resolve-only presence; self-targeted invokes route through Axon LocalRuntime",
            );
        }
        Ok(_) => {}
        Err(error) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = device_mode_self_presence_rejected,
                self_ura = ura,
                error = error,
            );
        }
    }
}
