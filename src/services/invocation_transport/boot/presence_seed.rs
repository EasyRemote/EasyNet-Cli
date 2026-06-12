use std::sync::Arc;

use crate::persistence::daemon_config::DaemonMode;
use crate::services::presence_registry::PresenceRegistry;

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
    // <self>.session is an OUTBOUND dial (the daemon dials the hub),
    // not an inbound register, so nothing populated the local table.
    // backend's `federation.resolve` then returned no agents; every
    // device showed REMOVED in /api/v1/devices despite the bidi
    // being healthy.
    //
    // Seed the local presence with the daemon's own URA on boot so
    // the local resolve answers "yes I'm here" when the operator's
    // backend asks. The dispatch sender pushes into a drain task
    // (kept alive as long as the daemon process), so try_send never
    // observes Closed/Full and the entry stays in the registry.
    //
    // This entry is RESOLVE-ONLY: it must never receive a dispatch
    // frame, because the drain task drops frames without completing
    // the pending entry. Two layers keep invokes off it: the
    // dispatch surfaces short-circuit self-targeted invocations to
    // the local Axon runtime (matches_self_target_ura fork), and
    // `dispatch_frame_to_presence` refuses any selected execution
    // host equal to the daemon's own URA before try_send fires.
    let (noop_tx, mut noop_rx) =
        tokio::sync::mpsc::channel(crate::services::presence_registry::DISPATCH_CHANNEL_CAPACITY);
    // Drain task: holds the receiver alive for the lifetime
    // of the daemon process. Without this, the receiver
    // gets dropped when the seeding scope ends and the
    // sender's first try_send observes Closed -> presence
    // entry deleted -> the very state we're trying to fix.
    tokio::spawn(async move {
        while let Some(_frame) = noop_rx.recv().await {
            // Drop on the floor. The self-targeted dispatch path
            // runs inline through Axon LocalRuntime; only defensive
            // out-of-path frames land here.
        }
    });
    let prior = presence.insert(ura.to_string(), noop_tx);
    if prior.is_none() {
        crate::op_event!(
            component = daemon_invocation,
            kind = device_mode_self_presence_seeded,
            self_ura = ura,
            message =
                "drain task holds receiver; self-targeted invokes route through Axon LocalRuntime",
        );
    }
}

/// Demo-only presence seed. Compiled into the daemon binary
/// only under `--features demo-fixture`; the production build
/// emits a no-op no matter what `EASYNET_DEMO_PRESENCE_SEED`
/// holds. The seed registers a no-op `DispatchSender` under
/// each comma-separated URA in the env var so cross-hub
/// `forward_invoke` targeting that URA survives the presence
/// registry lookup gate without a real device pair flow.
///
/// Channel capacity 8 mirrors the `<self>.session` accept
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
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(8);
        presence.insert(seed_ura.to_string(), tx);
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
