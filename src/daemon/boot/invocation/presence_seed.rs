use std::sync::Arc;

use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::persistence::daemon_config::DaemonMode;

pub(super) fn seed_boot_presence(
    mode: DaemonMode,
    daemon_ura: Option<&str>,
    presence: &Arc<PresenceRegistry>,
) {
    // Demo-only presence seed (cfg-gated). Production binaries
    // built without `--features demo-fixture` cannot honour the
    // `EASYNET_DEMO_PRESENCE_SEED` env var no matter how it gets
    // injected (container env, systemd unit override, etc.) —
    // the symbol simply isn't there. Demo / e2e scripts pass
    // `cargo build --features demo-fixture` to opt in.
    maybe_seed_demo_presence(presence);

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

/// Demo-only presence seed. Compiled into the daemon binary
/// only under `--features demo-fixture`; the production build
/// emits a no-op no matter what `EASYNET_DEMO_PRESENCE_SEED`
/// holds. The seed registers a no-op `DispatchSender` under
/// each comma-separated URA in the env var so cross-hub
/// `canonical_invoke` targeting that URA survives the presence
/// registry lookup gate without a real device pair flow.
///
/// Channel capacity 8 mirrors the `session.open` accept
/// path. A drain task discards every queued frame so the
/// channel never reports full or closed; the demo's
/// transport-plane proof terminates at "frame queued for
/// delivery". Real ability responses flow through
/// `dispatch_federation_*` handlers that do not consult the
/// presence frame queue.
#[cfg(feature = "demo-fixture")]
fn maybe_seed_demo_presence(presence: &Arc<PresenceRegistry>) {
    let Ok(seed_value) = std::env::var("EASYNET_DEMO_PRESENCE_SEED") else {
        return;
    };
    for seed_ura in seed_value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::daemon::invocation::bidi::state::presence::DispatchFrame, tonic::Status>,
        >(8);
        if let Err(error) = presence.insert_fixture_dispatch(seed_ura.to_string(), tx) {
            crate::op_event!(
                component = daemon_invocation,
                kind = demo_presence_seed_rejected,
                seed_ura = seed_ura,
                error = error,
            );
            continue;
        }
        tokio::spawn(async move {
            while rx.recv().await.is_some() {
                // discard
            }
        });
        crate::op_event!(
            component = daemon_invocation,
            kind = demo_presence_seed_registered,
            seed_ura = seed_ura,
            message = "test fixture; do not use in production",
        );
    }
}

#[cfg(not(feature = "demo-fixture"))]
fn maybe_seed_demo_presence(_presence: &Arc<PresenceRegistry>) {
    // Production build: env var is ignored. If the operator
    // set it expecting the demo behaviour, the missing log line
    // is the signal — re-build with `--features demo-fixture`.
}
