// EasyNet CLI — invocation_transport — session.open initiator (device side)
// ====================================================================
//
// File: src/services/invocation_transport/session_initiator.rs
// Description: Device-side caller for `session.open`. At daemon
//              boot a device opens one long-lived `InvokeBidi`
//              stream against its configured hub, sends frame 0 =
//              `EnvelopeOpen` carrying the caller URA, then keeps
//              the stream open for the lifetime of the daemon —
//              this is the canonical reverse channel through which
//              the hub pushes `runtime.invoke_remote` and
//              `federation.forward_invoke` frames back to the
//              device.
//
// Where this fits in RFC-003
// --------------------------
// PR-1 lands the daemon-side InvocationServer.
// PR-2 (this commit) lands two halves of `session.open`:
//
//   commit 1/N  — hub-side acceptor: the `session.open` arm of
//                 the daemon's `invoke_bidi` dispatcher. (Adjacent
//                 file `daemon_invocation_service.rs`; coordinated
//                 with PR-3 commit 1/3 currently being written,
//                 lands together.)
//
//   commit 2/N  — device-side initiator (this file): the boot-time
//                 task that dials the hub and holds the bidi
//                 stream open for the daemon's lifetime.
//
// Liveness model (spec §3)
// ------------------------
// Liveness is **stream membership**, not periodic heartbeat. The
// hub registers this device's `DispatchSender` in the
// `PresenceRegistry` exactly when the bidi is established and
// removes it exactly when the bidi closes (graceful or otherwise).
// The device side's job is therefore:
//
//   1. Dial hub. Once.
//   2. Send EnvelopeOpen frame 0 with caller URA from
//      `credentials.json`.
//   3. Loop: read `InvokeBidiDown` frames, dispatch each into the
//      local in-process invocation pipeline, write the reply (or
//      stream of replies) back as `InvokeBidiUp` frames.
//   4. On disconnect: log + reconnect with exponential backoff;
//      DO NOT dial twice in parallel (the hub would reject
//      displacement).
//
// What this commit lands
// ----------------------
// - The `dial_and_run_session(...)` function: takes hub endpoint,
//   caller URA, signing key, and a frame dispatcher; opens one
//   bidi, runs forever (until error / shutdown).
// - `SessionFrameDispatcher` trait: the local-side handler
//   implementation. Trait so PR-3 commit 3/3 (integration test)
//   can plug in a mock dispatcher that records frames received
//   without spinning up the full AxonAbilityCatalog.
// - The exponential-backoff supervisor `run_session_supervisor`:
//   the `dial_and_run_session` returns an error → wait → retry.
//   Full-jitter delay (uniform in [0, curve]), capped maximum,
//   never gives up.
//
// Signature model
// ---------------
// When boot supplies a deterministic per-device Ed25519 seed, frame 0
// is signed over the same canonical invocation bytes the admission gate
// verifies. Sparse credentials that only carry `agent_ura` still
// degrade to the unsigned PR-1/PR-2 behaviour so transition tests and
// partially-migrated devices do not fail hard during boot.
//
// What this commit does NOT do
// ----------------------------
// - AxonAbilityCatalog stream/unary multiplexing beyond the
//   current RPC path. Production now wires
//   `LocalAxonSessionDispatcher` at boot for local RPC abilities; true
//   multi-frame stream forwarding stays out of PR-2 and belongs to
//   the future streaming-ability surface.
// - daemon-config integration. The supervisor takes a hub
//   endpoint string directly today; the binary boot path will
//   wire it up to `DaemonConfig::hub_endpoint()` in the
//   integration step.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::Stream;
use tokio::sync::mpsc;
use tonic::Status;

use crate::runtime::ability_descriptor::AbilityDescriptor;
use crate::services::hub_published_ability_store::HubPublishedAbilityStore;

mod envelope;
mod frame_loop;
mod heartbeat;
mod prelude;
mod supervisor;
mod tasks;
mod transport;
mod warmup;

pub use envelope::{
    build_session_envelope_open, build_session_envelope_open_with_seed, SessionSigningSeed,
};
use frame_loop::{run_live_session, LiveSessionRun};
#[cfg(test)]
use prelude::build_synthetic_pages_ability_descriptors;
use prelude::{run_session_preludes, SessionPreludeChannels, SessionPreludeRun};
pub use prelude::{SessionPreludeInputs, UserTrustSync};
pub use supervisor::SessionCloseStats;
use supervisor::{
    backoff_after_clean_close, full_jitter, next_backoff, DeviceSessionPhase, SessionPhaseTracker,
};
#[cfg(test)]
use supervisor::{CloseClass, PreludeStep, SESSION_HEALTHY_MIN_UPTIME};
use warmup::warm_device_credential_for_session;
#[cfg(test)]
use warmup::{verify_device_credential_for_credentials, CredentialWarmupOutcome};

use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{BidiControl, BinaryChunk, InvokeBidiDown, InvokeBidiUp};

/// Daemon-side ability name this initiator targets. The hub's
/// `InvokeBidi` dispatcher routes on
/// `EnvelopeOpen.target.ability_name`.
///
/// `session.open` is the daemon-owned long-lived carrier for device
/// session membership. It is a direct wire break from the historical
/// caller-relative alias; no dual-name acceptance is retained.
pub const ABILITY_SESSION_OPEN: &str = "session.open";

/// Stream id used by every BinaryChunk on the session bidi. PR-2
/// sub-spec §2.1 (and the wider RFC-003 transport plane) declares
/// one StreamDescriptor (id=0, content_type="application/json",
/// ordering=STRICT). Multiple streams on the same bidi are
/// reserved for future RFCs and not used by `session.open`.
pub const SESSION_STREAM_ID: u32 = 0;

/// Capacity of the device-side outbound mpsc that
/// `dial_and_run_session` consumes when writing `InvokeBidiUp`
/// frames into the gRPC stream. Sized matching
/// `services::presence_registry::DISPATCH_CHANNEL_CAPACITY` so
/// the hub side and device side use symmetric backpressure
/// budgets.
const SESSION_UP_CHANNEL_CAPACITY: usize = 256;

/// One-shot notification used by daemon boot to distinguish "the
/// supervisor task was spawned" from "Hub admitted this device into
/// PresenceRegistry". Only the first admission attempt matters for
/// boot; after the daemon is admitted once, the long-lived supervisor
/// owns reconnects.
/// One-shot sender of the initial-admission verdict (Ok, or an error
/// string). Aliased to keep `InitialSessionAdmissionProbe`'s field
/// type legible.
type AdmissionVerdictSender = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<(), String>>>>>;

#[derive(Clone)]
pub(crate) struct InitialSessionAdmissionProbe {
    tx: AdmissionVerdictSender,
}

impl InitialSessionAdmissionProbe {
    fn admitted(&self) {
        self.complete(Ok(()));
    }

    fn failed(&self, reason: String) {
        self.complete(Err(reason));
    }

    fn complete(&self, outcome: Result<(), String>) {
        let Some(tx) = self
            .tx
            .lock()
            .expect("initial admission probe mutex")
            .take()
        else {
            return;
        };
        let _ = tx.send(outcome);
    }
}

pub(crate) fn initial_session_admission_probe() -> (
    InitialSessionAdmissionProbe,
    tokio::sync::oneshot::Receiver<Result<(), String>>,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    (
        InitialSessionAdmissionProbe {
            tx: Arc::new(Mutex::new(Some(tx))),
        },
        rx,
    )
}

/// Initial backoff interval between reconnect attempts. The
/// supervisor uses exponential backoff capped at
/// `SESSION_BACKOFF_MAX`; 250 ms aligns with the MVP supervisor
/// the production daemon previously used for federation_client.
pub const SESSION_BACKOFF_INITIAL: Duration = Duration::from_millis(250);

/// Cap for the backoff curve. After the curve hits this value the
/// supervisor keeps retrying, each wait drawn uniformly in
/// [0, this] (full jitter). 30 s
/// matches the reconnect SLO the production deploy script
/// configures for federation_client.
pub const SESSION_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Maximum silence window on the down-stream before the device
/// declares the session dead and forces a reconnect.
///
/// Why this exists in addition to transport-level HTTP/2 PING:
/// the observed Docker failure mode was asymmetric — the hub-side
/// reader saw `h2 protocol error: error reading a body from
/// connection`, removed presence immediately, but the device-side
/// `down_stream.next()` sometimes remained parked inside tonic's
/// body machinery instead of surfacing EOF/reset promptly. A
/// bounded inactivity watchdog closes that gap: if the hub stops
/// sending *anything* (real dispatches, receipts, or no-op
/// keepalives) for 15 s, the device tears the bidi down and the
/// supervisor redials.
///
/// Paired with the hub-side no-op control keepalive every 5 s
/// (`daemon_invocation_service::SESSION_DOWN_HEARTBEAT_INTERVAL`);
/// 15 s = three missed keepalive windows, which is conservative
/// enough to avoid false positives on a briefly busy runtime but
/// fast enough to self-heal the session well before the old
/// "hang forever in client.invoke_bidi" failure mode becomes
/// user-visible.
pub const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

/// Device → hub application-level heartbeat cadence for the
/// `session.open` up-stream.
///
/// The hub-side failure signature the user observed is
/// specifically "error reading a body from connection" — i.e. the
/// server's request-body reader saw the stream die while the
/// client-side task was otherwise idle. Transport-level HTTP/2
/// PING keeps the connection warm, but some proxies / LB stacks
/// make stream-idle decisions on DATA/HEADERS activity, not on
/// connection-level PING alone. Emitting a no-op control frame
/// every 5 s keeps the request body observably alive.
///
/// Paired with the hub-side `SESSION_DOWN_HEARTBEAT_INTERVAL` and
/// the device-side `SESSION_IDLE_TIMEOUT`. Together they make the
/// bidi liveness story symmetric in both directions.
pub const SESSION_UP_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

const REASON_BIDI_DOWN_SEQUENCE: &str = "AXON_BIDI_DOWN_SEQUENCE";

/// What a device does with each `InvokeBidiDown` frame the hub
/// pushes to it: either translate the inner payload into a local
/// invocation and write the result back, or honour a control
/// frame. The trait surface intentionally looks like a single
/// `handle_down` because the bidi is duplex — the implementation
/// drives whatever response shape it needs through the supplied
/// `outbound` sender.
/// Highest dispatch-frame contract this device speaks (DEC-F004).
pub const DEVICE_DISPATCH_CONTRACT_VERSION: u32 = 1;

/// T1.2 claimant fingerprint: one random 16-byte nonce per process
/// boot. Lets the hub distinguish a same-device restart (same URA,
/// new nonce each boot — sequential) from two live processes fighting
/// over one URA (alternating distinct nonces).
pub fn claimant_boot_nonce() -> &'static [u8; 16] {
    static NONCE: std::sync::OnceLock<[u8; 16]> = std::sync::OnceLock::new();
    NONCE.get_or_init(|| {
        let mut nonce = [0_u8; 16];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        nonce
    })
}

#[async_trait::async_trait]
pub trait SessionFrameDispatcher: Send + Sync + 'static {
    /// Handle one inbound frame. The dispatcher writes any reply
    /// frames into the supplied `outbound` channel (which is the
    /// device's bidi up sender). Returning an error from this
    /// method will not tear down the session — the supervisor
    /// logs and continues, treating malformed frames as the
    /// individual frames they are.
    async fn handle_down(
        &self,
        frame: InvokeBidiDown,
        outbound: &SessionUpSender,
    ) -> Result<(), SessionDispatchError>;
}

/// Session-scoped sender for post-frame-0 `InvokeBidiUp` frames.
///
/// Why this exists:
/// - Up-direction sequence numbers are independent from down-
///   direction numbers per RFC 001 §A16.
/// - Multiple producers share one live bidi after frame 0:
///   `LocalAxonSessionDispatcher` reply frames, device-mode
///   `SessionEscalationHandle` Request frames, and the no-op
///   up-heartbeat task.
/// - Using raw `mpsc::Sender<InvokeBidiUp>` let each producer
///   invent sequences independently; one path even hard-coded 0 on
///   every post-frame-0 Request. That is a wire bug once strict
///   sequence validation is enabled and is already the wrong
///   mental model today.
///
/// This wrapper is the single source of truth for post-frame-0
/// up-direction sequencing within one `session.open`.
#[derive(Clone, Debug)]
pub struct SessionUpSender {
    tx: mpsc::Sender<InvokeBidiUp>,
    /// Sequence allocation and channel insertion must be one atomic
    /// step: the hub validates a strictly monotonic up-sequence and
    /// resets the whole session on violation, so two producers that
    /// allocate N and N+1 but enqueue in the opposite order kill the
    /// session. The mutex spans allocate+send — it is the session's
    /// single-writer gate, and the serialization it imposes is
    /// exactly the ordering the wire contract requires.
    sequence_gate: Arc<tokio::sync::Mutex<u64>>,
    /// Negotiated dispatch contract for THIS session (DEC-F004):
    /// written once by the supervisor when the admission receipt's
    /// session_contract arrives; read by reply producers to pick the
    /// frame encoding. 0 until negotiation lands = JSON era.
    negotiated_contract: Arc<std::sync::atomic::AtomicU32>,
}

impl SessionUpSender {
    #[must_use]
    pub fn new(tx: mpsc::Sender<InvokeBidiUp>) -> Self {
        Self {
            tx,
            // Frame 0 is EnvelopeOpen. First post-frame-0 producer
            // therefore owns sequence = 1.
            sequence_gate: Arc::new(tokio::sync::Mutex::new(1)),
            negotiated_contract: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }

    /// Supervisor-only: record the hub's negotiated session contract.
    pub fn set_negotiated_contract(&self, version: u32) {
        self.negotiated_contract
            .store(version, std::sync::atomic::Ordering::Release);
    }

    /// True when this session speaks carrier-v1 dispatch frames.
    #[must_use]
    pub fn carrier_v1(&self) -> bool {
        self.negotiated_contract
            .load(std::sync::atomic::Ordering::Acquire)
            >= 1
    }

    /// Stamp the next sequence number and enqueue under the
    /// single-writer gate, so channel order always equals sequence
    /// order even with concurrent reply producers.
    async fn send_sequenced(
        &self,
        payload: UpPayload,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<InvokeBidiUp>> {
        let mut next = self.sequence_gate.lock().await;
        let sequence = *next;
        let sent = self
            .tx
            .send(InvokeBidiUp {
                sequence,
                payload: Some(payload),
                ..InvokeBidiUp::default()
            })
            .await;
        if sent.is_ok() {
            *next += 1;
        }
        sent
    }

    /// Send a BinaryChunk on the live session, stamping the next
    /// monotonic up-direction sequence number.
    pub async fn send_binary_chunk(
        &self,
        chunk: BinaryChunk,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<InvokeBidiUp>> {
        self.send_sequenced(UpPayload::BinaryChunk(chunk)).await
    }

    /// Send any up-direction payload on the live session, stamping
    /// the next monotonic sequence number. Carrier-v1 reply frames
    /// (DispatchResult / ReverseDispatchCall) ride this.
    pub async fn send_payload(
        &self,
        payload: UpPayload,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<InvokeBidiUp>> {
        self.send_sequenced(payload).await
    }

    /// Send a control frame on the live session, stamping the next
    /// monotonic up-direction sequence number.
    pub async fn send_control(
        &self,
        control: BidiControl,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<InvokeBidiUp>> {
        self.send_sequenced(UpPayload::Control(control)).await
    }
}

/// Error from a single down-frame dispatch. Reported by the
/// dispatcher; the supervisor logs and continues.
#[derive(Debug, thiserror::Error)]
pub enum SessionDispatchError {
    #[error("session frame dispatch failed: {0}")]
    Other(String),
}

struct SessionDialAttempt<'a, D: SessionFrameDispatcher> {
    hub_endpoint: String,
    caller_ura: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_ca_pem_path: Option<&'a Path>,
    dispatcher: Arc<D>,
    escalation_outbox:
        Option<&'a crate::services::invocation_transport::session_escalation::SharedSessionOutbox>,
    preludes: SessionPreludeInputs<'a>,
    idle_timeout: Duration,
    initial_admission: Option<InitialSessionAdmissionProbe>,
    user_trust_sync: Option<&'a UserTrustSync>,
}

pub(crate) struct SessionSupervisorRunConfig<D: SessionFrameDispatcher> {
    pub(crate) hub_endpoint: String,
    pub(crate) caller_ura: String,
    pub(crate) signing_seed: Option<SessionSigningSeed>,
    pub(crate) hub_ca_pem_path: Option<PathBuf>,
    pub(crate) dispatcher: Arc<D>,
    pub(crate) escalation_outbox:
        Option<crate::services::invocation_transport::session_escalation::SharedSessionOutbox>,
    pub(crate) ability_descriptors: Vec<AbilityDescriptor>,
    pub(crate) hub_published_abilities: Arc<HubPublishedAbilityStore>,
    pub(crate) initial_admission: Option<InitialSessionAdmissionProbe>,
    pub(crate) user_trust_sync: Option<UserTrustSync>,
    pub(crate) cancel: tokio::sync::oneshot::Receiver<()>,
}

/// Run one `session.open` bidi against `hub_endpoint`. Connects,
/// sends frame 0, streams frames until either the hub closes the
/// down-stream (returns `Ok(())`) or a transport error occurs
/// (returns `Err(...)`).
///
/// `caller_ura` is the device's canonical URA per spec §5.1
/// (`easynet:///r/{tenant_id}/agent/{node_id}`). PR-1 staging
/// admits a missing `caller_signature` if the URA is in the
/// hub's realm trust anchor (or matches the hub's own URA for
/// loopback); PR-7 closes the loop with real ed25519 signing.
///
/// `hub_ca_pem_path` pins the hub's TLS CA when set, mirroring the
/// pattern `cross_hub_dial::resolve_peer_channel` already uses for
/// hub-to-hub dials. With `None`, tonic falls back to the system
/// trust store (production deployments using publicly-trusted
/// certs); with `Some(path)`, the PEM is loaded and supplied via
/// `ClientTlsConfig::ca_certificate` so a self-signed hub cert
/// rooted at an operator-pinned CA validates. Production daemons
/// resolve the path from `realm-trust.toml`'s Hub-role entry whose
/// `hub_endpoint` matches `hub_endpoint`.
pub async fn dial_and_run_session<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_ura: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_ca_pem_path: Option<&Path>,
    dispatcher: Arc<D>,
    escalation_outbox: Option<
        &crate::services::invocation_transport::session_escalation::SharedSessionOutbox,
    >,
    preludes: SessionPreludeInputs<'_>,
) -> Result<SessionCloseStats, SessionError> {
    // One-shot dial (no supervisor): the phase stream still emits,
    // scoped to this call's private tracker.
    let mut phase = SessionPhaseTracker::new();
    phase.begin_attempt();
    dial_and_run_session_with_idle_timeout(
        SessionDialAttempt {
            hub_endpoint,
            caller_ura,
            signing_seed,
            hub_ca_pem_path,
            dispatcher,
            escalation_outbox,
            preludes,
            idle_timeout: SESSION_IDLE_TIMEOUT,
            initial_admission: None,
            user_trust_sync: None,
        },
        &mut phase,
    )
    .await
}

async fn dial_and_run_session_with_idle_timeout<D: SessionFrameDispatcher>(
    attempt: SessionDialAttempt<'_, D>,
    phase: &mut SessionPhaseTracker,
) -> Result<SessionCloseStats, SessionError> {
    let SessionDialAttempt {
        hub_endpoint,
        caller_ura,
        signing_seed,
        hub_ca_pem_path,
        dispatcher,
        escalation_outbox,
        preludes,
        idle_timeout,
        initial_admission,
        user_trust_sync,
    } = attempt;
    // Idempotent under a supervisor (begin_attempt already entered
    // Dialing; same-phase transitions early-return); direct callers
    // (one-shot dial, tests) enter the machine here.
    phase.transition(DeviceSessionPhase::Dialing, "dial_entered");
    warm_device_credential_for_session(&caller_ura).await;

    let channel = transport::connect_session_channel(&hub_endpoint, hub_ca_pem_path).await?;
    // Cheap tonic Channel clone retained for the user-key re-sync
    // loop spawned after the preludes (its requests multiplex over
    // this same connection).
    let resync_channel = channel.clone();
    // Second clone for the federation.heartbeat liveness loop — same
    // multiplexing over this connection, independent client.
    let heartbeat_channel = channel.clone();

    // Bump client-side gRPC message limits to match the server side.
    // The tonic-default 4 MiB decoder cap aborted `session.open`
    // mid-stream the moment a single down-frame envelope exceeded it.
    // The shared 64 MiB transport-envelope cap keeps legitimate
    // chunked traffic flowing without permitting near-unbounded
    // single-message allocations.
    let mut client = transport::session_invocation_client(channel);

    let prelude_channels = SessionPreludeChannels::new(resync_channel, heartbeat_channel);
    let _prelude_guards = run_session_preludes(SessionPreludeRun {
        client: &mut client,
        phase,
        hub_endpoint: &hub_endpoint,
        caller_ura: &caller_ura,
        signing_seed,
        inputs: preludes,
        user_trust_sync,
        channels: prelude_channels,
    })
    .await?;

    run_live_session(
        LiveSessionRun {
            client,
            hub_endpoint,
            caller_ura,
            signing_seed,
            dispatcher,
            escalation_outbox,
            idle_timeout,
            initial_admission,
        },
        phase,
    )
    .await
}

/// Long-lived supervisor wrapping `dial_and_run_session` with
/// exponential backoff. Returns only when `cancel` resolves; the
/// reconnect loop never exits on its own. Production daemons run
/// this on a `tokio::spawn` at boot.
///
/// `hub_ca_pem_path` is forwarded to every dial attempt so a
/// SIGHUP that swaps the trust anchor takes effect on the next
/// reconnect. Today the supervisor is constructed once at boot;
/// if a future change wires SIGHUP-driven trust reloads through
/// the supervisor, this value should become a cell snapshot
/// rather than an owned `PathBuf`.
pub(crate) async fn run_session_supervisor<D: SessionFrameDispatcher>(
    config: SessionSupervisorRunConfig<D>,
) {
    let SessionSupervisorRunConfig {
        hub_endpoint,
        caller_ura,
        signing_seed,
        hub_ca_pem_path,
        dispatcher,
        escalation_outbox,
        ability_descriptors,
        hub_published_abilities,
        initial_admission,
        user_trust_sync,
        mut cancel,
    } = config;

    let mut backoff = SESSION_BACKOFF_INITIAL;
    let mut phase = SessionPhaseTracker::new();
    loop {
        phase.begin_attempt();
        // Arm bodies stay trivial: the dial future holds `&mut phase`
        // for its lifetime, so phase handling (like all result
        // handling) happens after the select expression, once the
        // future is out of scope.
        let outcome = tokio::select! {
            _ = &mut cancel => None,
            result = dial_and_run_session_with_idle_timeout(
                SessionDialAttempt {
                    hub_endpoint: hub_endpoint.clone(),
                    caller_ura: caller_ura.clone(),
                    signing_seed,
                    hub_ca_pem_path: hub_ca_pem_path.as_deref(),
                    dispatcher: Arc::clone(&dispatcher),
                    escalation_outbox: escalation_outbox.as_ref(),
                    preludes: SessionPreludeInputs::new(
                        &ability_descriptors,
                        Arc::clone(&hub_published_abilities),
                    ),
                    idle_timeout: SESSION_IDLE_TIMEOUT,
                    initial_admission: initial_admission.clone(),
                    user_trust_sync: user_trust_sync.as_ref(),
                },
                &mut phase,
            ) => Some(result),
        };
        let Some(result) = outcome else {
            phase.transition(DeviceSessionPhase::Idle, "supervisor_cancelled");
            crate::op_event!(component = session, kind = supervisor_cancelled,);
            return;
        };
        match result {
            Ok(stats) => {
                backoff = backoff_after_clean_close(&stats, backoff);
                // Render Durations as integer milliseconds —
                // `Duration` has no `Display` impl, and the
                // Debug form (`250ms` / `1.5s`) mixes unit
                // suffixes that complicate SRE arithmetic on
                // the field. Milliseconds is the unit operators
                // already see in `*_ms` fields elsewhere.
                let uptime_ms = stats.uptime.as_millis() as u64;
                let frames_received = stats.frames_received;
                let close_class = stats.classify().as_str();
                phase.transition(DeviceSessionPhase::Backoff, close_class);
                let next_backoff_ms = backoff.as_millis() as u64;
                crate::op_event!(
                    component = session,
                    kind = bidi_closed_cleanly,
                    uptime_ms = uptime_ms,
                    frames_received = frames_received,
                    close_class = close_class,
                    next_backoff_ms = next_backoff_ms,
                );
            }
            Err(err) => {
                phase.transition(DeviceSessionPhase::Backoff, "session_error");
                // The hub session errored and we are about to back off and
                // reconnect — presence is NOT admitted. Downgrade the snapshot to
                // ConnectedSuspect so `doctor` stops reporting FRONTEND_CONNECTED
                // while session.open is wedged in a reconnect loop (the exact lie
                // that hid the descriptor-ref rejection). frame_loop promotes back
                // to ConnectedOnline once a reconnect re-negotiates the contract.
                frame_loop::record_connection_state(
                    crate::runtime::join_connection_state::JoinConnectionState::ConnectedSuspect,
                    crate::runtime::join_connection_state::JoinTransition::OpenSelfSession,
                    "session.error_reconnecting",
                );
                // `{err:#}` walks the std::error::Error source
                // chain so opaque `tonic::transport::Error`
                // ("transport error") surfaces the underlying
                // rustls / hyper / io cause. Without this, a
                // CA-trust failure or DNS error is
                // indistinguishable from a NAT idle drop.
                let err_msg = format!("{err:#}");
                if let Some(probe) = &initial_admission {
                    probe.failed(err_msg.clone());
                }
                let next_backoff_ms = backoff.as_millis() as u64;
                crate::op_event!(
                    component = session,
                    kind = bidi_error_reconnecting,
                    error = err_msg,
                    next_backoff_ms = next_backoff_ms,
                );
            }
        }

        // Sleep the FULL-JITTER delay, also cancellable. The
        // deterministic curve (`backoff`) is the upper bound;
        // the actual wait is uniform in [0, backoff], which
        // de-synchronizes a fleet that all lost a hub at the
        // same instant (the 250 ms in-phase thundering herd
        // b2ba441 only damped per-device). The curve state is
        // unchanged — doubling and the healthy-uptime reset
        // operate on `backoff`, not the jittered sample.
        let jittered = full_jitter(backoff);
        let cancelled = tokio::select! {
            _ = &mut cancel => true,
            _ = tokio::time::sleep(jittered) => false,
        };
        if cancelled {
            phase.transition(DeviceSessionPhase::Idle, "supervisor_cancelled");
            return;
        }

        backoff = next_backoff(backoff);
    }
}

/// Reasons `dial_and_run_session` can fail. The supervisor maps
/// each into a backoff + reconnect; production logs include the
/// variant name + endpoint so operators can distinguish between
/// "hub is down" and "this device is not in the trust anchor".
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("invalid hub endpoint `{endpoint}`: {source}")]
    InvalidEndpoint {
        endpoint: String,
        #[source]
        source: tonic::transport::Error,
    },

    #[error("failed to connect to hub `{endpoint}`: {source}")]
    ConnectFailed {
        endpoint: String,
        #[source]
        source: tonic::transport::Error,
    },

    #[error("hub `{endpoint}` rejected `session.open` bidi: {status}")]
    HubRejected { endpoint: String, status: Status },

    #[error("hub `{endpoint}` sent error frame on down stream: {status}")]
    DownStreamError { endpoint: String, status: Status },

    #[error("hub `{endpoint}` rejected owner projection publish: {status}")]
    OwnerProjectionFailed { endpoint: String, status: Status },

    #[error("{reason}: hub `{endpoint}` sent down frame sequence {actual}, expected {expected}")]
    DownStreamSequence {
        endpoint: String,
        expected: u64,
        actual: u64,
        reason: &'static str,
    },

    #[error(
        "hub `{endpoint}` sent no down-stream activity for {:?}; forcing reconnect",
        timeout
    )]
    IdleTimeout { endpoint: String, timeout: Duration },

    #[error("internal: failed to enqueue {0} for hub send")]
    SendFailed(&'static str),

    #[error("read tls_ca_pem_path `{path}`: {source}")]
    TlsCaRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("apply ClientTlsConfig for hub `{endpoint}`: {source}")]
    TlsConfig {
        endpoint: String,
        #[source]
        source: tonic::transport::Error,
    },
}

/// Boxed pinned stream alias used by the frame-handler trait
/// implementations that want to return reply streams from
/// `handle_down`. Public so PR-3 / PR-7 implementors can use the
/// same alias.
pub type SessionReplyStream =
    Pin<Box<dyn Stream<Item = Result<InvokeBidiUp, Status>> + Send + 'static>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener as StdTcpListener};
    use std::sync::mpsc as std_mpsc;
    use std::thread;

    use easynet_axon::pb::axon::v1::invocation_server::{Invocation, InvocationServer};
    use easynet_axon::pb::axon::v1::{
        InvokeRequest, InvokeResponse, InvokeServerStreamRequest, InvokeStreamChunk,
    };
    use futures::{stream, StreamExt as _};
    use serde_json::Value;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response};

    /// Upper bound for "did the supervisor's async machinery make
    /// progress" assertions (exit-on-cancel, initial-admission report).
    /// This is NOT a product SLA — it only guards against a genuine hang.
    /// Kept generous because the full `cargo test --lib` run (3000+ tests)
    /// saturates the scheduler and an in-process loopback bidi handshake
    /// can spend most of the old 10 s budget in prelude reflection before
    /// reporting admission. This remains a hang guard, not a product SLA.
    const TEST_SUPERVISOR_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);

    fn hub_store() -> Arc<HubPublishedAbilityStore> {
        HubPublishedAbilityStore::new()
    }

    #[test]
    fn synthetic_pages_descriptors_match_resolver_lookup_keys() {
        // RFC-005 name match: the backend invokes `pages.list` against
        // `agent/<user>.pages`; the resolver looks up the relative name
        // `pages.list` AND the canonical ability URA
        // `…/ability/<user>.pages.pages.list`. The advertised descriptor
        // must project to both, or pages.* stays NODATA. This pins the
        // `pages.pages.list` descriptor-name trick (public_name() strips
        // the owner's `pages.` agent-id prefix).
        let owner = "easynet:///r/acme/agent/alice.pages";
        let descriptors = build_synthetic_pages_ability_descriptors(owner);
        let by_public: std::collections::BTreeMap<_, _> = descriptors
            .iter()
            .map(|d| (d.public_name(), d.canonical_ability_ura()))
            .collect();
        assert_eq!(
            by_public.get("pages.list").cloned().flatten().as_deref(),
            Some("easynet:///r/acme/ability/alice.pages.pages.list"),
        );
        assert_eq!(
            by_public.get("pages.publish").cloned().flatten().as_deref(),
            Some("easynet:///r/acme/ability/alice.pages.pages.publish"),
        );
        // All four management abilities are present.
        assert_eq!(descriptors.len(), 4);

        // The advertised descriptor must carry the input schema so the
        // Frontend InvokeAbilityDialog renders a form (not "No input
        // required" → empty-arg invoke → missing project_id 400).
        let get = descriptors
            .iter()
            .find(|d| d.public_name() == "pages.get")
            .expect("pages.get descriptor present");
        let schema = &get.schema_summary.input;
        assert_eq!(
            schema["required"][0], "project_id",
            "pages.get must advertise project_id as required, got: {schema}"
        );
    }

    /// A mock dispatcher that just records every down frame it
    /// receives. Used by tests; production wires the real
    /// AxonAbilityCatalog-backed dispatcher.
    #[derive(Default)]
    struct RecordingDispatcher {
        received: tokio::sync::Mutex<Vec<InvokeBidiDown>>,
    }

    #[async_trait::async_trait]
    impl SessionFrameDispatcher for RecordingDispatcher {
        async fn handle_down(
            &self,
            frame: InvokeBidiDown,
            _outbound: &SessionUpSender,
        ) -> Result<(), SessionDispatchError> {
            self.received.lock().await.push(frame);
            Ok(())
        }
    }

    type TestInvokeStream =
        Pin<Box<dyn Stream<Item = Result<InvokeStreamChunk, Status>> + Send + 'static>>;
    type TestInvokeBidiStream =
        Pin<Box<dyn Stream<Item = Result<InvokeBidiDown, Status>> + Send + 'static>>;

    #[derive(Default)]
    struct SilentSessionHub;

    #[tonic::async_trait]
    impl Invocation for SilentSessionHub {
        type InvokeStreamStream = TestInvokeStream;
        type InvokeBidiStream = TestInvokeBidiStream;

        async fn invoke(
            &self,
            _request: Request<InvokeRequest>,
        ) -> Result<Response<InvokeResponse>, Status> {
            Err(Status::unimplemented("test hub only wires InvokeBidi"))
        }

        async fn invoke_stream(
            &self,
            _request: Request<InvokeServerStreamRequest>,
        ) -> Result<Response<Self::InvokeStreamStream>, Status> {
            Err(Status::unimplemented("test hub only wires InvokeBidi"))
        }

        async fn invoke_bidi(
            &self,
            request: Request<tonic::Streaming<InvokeBidiUp>>,
        ) -> Result<Response<Self::InvokeBidiStream>, Status> {
            let mut up = request.into_inner();
            let frame0 = up
                .next()
                .await
                .ok_or_else(|| Status::invalid_argument("expected frame 0"))?
                .map_err(|status| Status::internal(format!("frame 0 recv: {status}")))?;
            let UpPayload::EnvelopeOpen(_) = frame0.payload.ok_or_else(|| {
                Status::invalid_argument("frame 0 must carry EnvelopeOpen payload")
            })?
            else {
                return Err(Status::invalid_argument("frame 0 must be EnvelopeOpen"));
            };

            // Hold the bidi open forever without producing any
            // down-stream frames. The session-side idle watchdog
            // must turn this silent hang into a bounded error.
            Ok(Response::new(
                Box::pin(stream::pending()) as Self::InvokeBidiStream
            ))
        }
    }

    /// Hub that admits the bidi and immediately ends the down stream
    /// with a clean EOF — the device-observable shape of hub-side
    /// presence displacement (registered `DispatchSender` dropped
    /// right after accept) and of incident 2026-06-11's repeated
    /// closes.
    struct CleanCloseSessionHub;

    #[tonic::async_trait]
    impl Invocation for CleanCloseSessionHub {
        type InvokeStreamStream = TestInvokeStream;
        type InvokeBidiStream = TestInvokeBidiStream;

        async fn invoke(
            &self,
            _request: Request<InvokeRequest>,
        ) -> Result<Response<InvokeResponse>, Status> {
            Err(Status::unimplemented("test hub only wires InvokeBidi"))
        }

        async fn invoke_stream(
            &self,
            _request: Request<InvokeServerStreamRequest>,
        ) -> Result<Response<Self::InvokeStreamStream>, Status> {
            Err(Status::unimplemented("test hub only wires InvokeBidi"))
        }

        async fn invoke_bidi(
            &self,
            request: Request<tonic::Streaming<InvokeBidiUp>>,
        ) -> Result<Response<Self::InvokeBidiStream>, Status> {
            let mut up = request.into_inner();
            let frame0 = up
                .next()
                .await
                .ok_or_else(|| Status::invalid_argument("expected frame 0"))?
                .map_err(|status| Status::internal(format!("frame 0 recv: {status}")))?;
            let UpPayload::EnvelopeOpen(_) = frame0.payload.ok_or_else(|| {
                Status::invalid_argument("frame 0 must carry EnvelopeOpen payload")
            })?
            else {
                return Err(Status::invalid_argument("frame 0 must be EnvelopeOpen"));
            };

            Ok(Response::new(
                Box::pin(stream::empty()) as Self::InvokeBidiStream
            ))
        }
    }

    #[derive(Default)]
    struct OutOfSequenceSessionHub;

    #[derive(Clone, Default)]
    struct RecordingPreludeHub {
        invokes: Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
    }

    #[tonic::async_trait]
    impl Invocation for OutOfSequenceSessionHub {
        type InvokeStreamStream = TestInvokeStream;
        type InvokeBidiStream = TestInvokeBidiStream;

        async fn invoke(
            &self,
            _request: Request<InvokeRequest>,
        ) -> Result<Response<InvokeResponse>, Status> {
            Err(Status::unimplemented("test hub only wires InvokeBidi"))
        }

        async fn invoke_stream(
            &self,
            _request: Request<InvokeServerStreamRequest>,
        ) -> Result<Response<Self::InvokeStreamStream>, Status> {
            Err(Status::unimplemented("test hub only wires InvokeBidi"))
        }

        async fn invoke_bidi(
            &self,
            request: Request<tonic::Streaming<InvokeBidiUp>>,
        ) -> Result<Response<Self::InvokeBidiStream>, Status> {
            let mut up = request.into_inner();
            let frame0 = up
                .next()
                .await
                .ok_or_else(|| Status::invalid_argument("expected frame 0"))?
                .map_err(|status| Status::internal(format!("frame 0 recv: {status}")))?;
            let UpPayload::EnvelopeOpen(_) = frame0.payload.ok_or_else(|| {
                Status::invalid_argument("frame 0 must carry EnvelopeOpen payload")
            })?
            else {
                return Err(Status::invalid_argument("frame 0 must be EnvelopeOpen"));
            };

            let frames = vec![
                Ok(InvokeBidiDown {
                    sequence: 0,
                    payload: Some(
                        easynet_axon::pb::axon::v1::invoke_bidi_down::Payload::Receipt(
                            easynet_axon::pb::axon::v1::InvocationReceipt {
                                state: easynet_axon::invocation::InvocationState::Admitted
                                    .to_wire_i32(),
                                ..easynet_axon::pb::axon::v1::InvocationReceipt::default()
                            },
                        ),
                    ),
                    ..InvokeBidiDown::default()
                }),
                Ok(InvokeBidiDown {
                    sequence: 9,
                    payload: Some(
                        easynet_axon::pb::axon::v1::invoke_bidi_down::Payload::Control(
                            BidiControl::default(),
                        ),
                    ),
                    ..InvokeBidiDown::default()
                }),
            ];
            Ok(Response::new(
                Box::pin(stream::iter(frames)) as Self::InvokeBidiStream
            ))
        }
    }

    #[tonic::async_trait]
    impl Invocation for RecordingPreludeHub {
        type InvokeStreamStream = TestInvokeStream;
        type InvokeBidiStream = TestInvokeBidiStream;

        async fn invoke(
            &self,
            request: Request<InvokeRequest>,
        ) -> Result<Response<InvokeResponse>, Status> {
            let request = request.into_inner();
            let body: Value = serde_json::from_slice(&request.arguments)
                .map_err(|e| Status::invalid_argument(format!("invalid json args: {e}")))?;
            self.invokes
                .lock()
                .await
                .push((request.function_name, body));
            Ok(Response::new(InvokeResponse::default()))
        }

        async fn invoke_stream(
            &self,
            _request: Request<InvokeServerStreamRequest>,
        ) -> Result<Response<Self::InvokeStreamStream>, Status> {
            Err(Status::unimplemented(
                "test hub only wires Invoke/InvokeBidi",
            ))
        }

        async fn invoke_bidi(
            &self,
            request: Request<tonic::Streaming<InvokeBidiUp>>,
        ) -> Result<Response<Self::InvokeBidiStream>, Status> {
            let mut up = request.into_inner();
            let frame0 = up
                .next()
                .await
                .ok_or_else(|| Status::invalid_argument("expected frame 0"))?
                .map_err(|status| Status::internal(format!("frame 0 recv: {status}")))?;
            let UpPayload::EnvelopeOpen(_) = frame0.payload.ok_or_else(|| {
                Status::invalid_argument("frame 0 must carry EnvelopeOpen payload")
            })?
            else {
                return Err(Status::invalid_argument("frame 0 must be EnvelopeOpen"));
            };
            Ok(Response::new(
                Box::pin(stream::pending()) as Self::InvokeBidiStream
            ))
        }
    }

    async fn spawn_silent_session_hub() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind silent session hub");
        let addr = listener.local_addr().expect("silent hub local addr");
        let incoming = TcpListenerStream::new(listener);
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(InvocationServer::new(SilentSessionHub))
                .serve_with_incoming(incoming)
                .await
                .expect("silent session hub server");
        });
        (addr, handle)
    }

    async fn spawn_clean_close_session_hub() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind clean-close session hub");
        let addr = listener.local_addr().expect("clean-close hub local addr");
        let incoming = TcpListenerStream::new(listener);
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(InvocationServer::new(CleanCloseSessionHub))
                .serve_with_incoming(incoming)
                .await
                .expect("clean-close session hub server");
        });
        (addr, handle)
    }

    async fn spawn_out_of_sequence_session_hub() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind out-of-sequence session hub");
        let addr = listener
            .local_addr()
            .expect("out-of-sequence hub local addr");
        let incoming = TcpListenerStream::new(listener);
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(InvocationServer::new(OutOfSequenceSessionHub))
                .serve_with_incoming(incoming)
                .await
                .expect("out-of-sequence session hub server");
        });
        (addr, handle)
    }

    async fn spawn_recording_prelude_hub() -> (
        SocketAddr,
        Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind recording prelude hub");
        let addr = listener.local_addr().expect("recording hub local addr");
        let incoming = TcpListenerStream::new(listener);
        let hub = RecordingPreludeHub::default();
        let invokes = hub.invokes.clone();
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(InvocationServer::new(hub))
                .serve_with_incoming(incoming)
                .await
                .expect("recording prelude hub server");
        });
        (addr, invokes, handle)
    }

    #[test]
    fn build_session_envelope_open_carries_caller_ura_and_ability_name() {
        let frame = build_session_envelope_open("easynet:///r/realm/device/n1");
        let UpPayload::EnvelopeOpen(eo) = frame.payload.expect("payload") else {
            panic!("frame 0 must be EnvelopeOpen");
        };
        assert_eq!(
            eo.target
                .as_ref()
                .map(|t| t.ability_name.as_str())
                .unwrap_or(""),
            ABILITY_SESSION_OPEN,
        );
        assert_eq!(
            eo.envelope
                .as_ref()
                .and_then(|e| e.caller.as_ref())
                .map(|a| a.ura.as_str())
                .unwrap_or(""),
            "easynet:///r/realm/device/n1",
        );
    }

    #[test]
    fn build_session_envelope_open_includes_one_stream_descriptor() {
        let frame = build_session_envelope_open("easynet:///r/realm/device/n1");
        let UpPayload::EnvelopeOpen(eo) = frame.payload.expect("payload") else {
            panic!("payload");
        };
        assert_eq!(eo.streams.len(), 1);
        assert_eq!(eo.streams[0].stream_id, SESSION_STREAM_ID);
        assert_eq!(eo.streams[0].content_type, "application/json");
    }

    #[test]
    fn build_session_envelope_open_with_seed_adds_signature_and_nonce() {
        let seed = [0x42_u8; 32];
        let frame =
            build_session_envelope_open_with_seed("easynet:///r/realm/device/n1", Some(seed));
        assert_eq!(frame.sequence, 0);
        assert_eq!(frame.mac.len(), 64);

        let UpPayload::EnvelopeOpen(eo) = frame.payload.expect("payload") else {
            panic!("payload");
        };
        let envelope = eo.envelope.expect("envelope");
        assert_eq!(envelope.invocation_nonce.len(), 16);
        let sig = envelope
            .caller_signature
            .as_ref()
            .expect("signed caller signature");
        assert_eq!(sig.algorithm, "ed25519");
        assert_eq!(sig.signature, frame.mac);
        assert_eq!(
            envelope
                .subject
                .as_ref()
                .map(|s| s.ura.as_str())
                .unwrap_or(""),
            "easynet:///r/realm/device/n1",
        );
    }

    #[tokio::test]
    async fn session_up_sender_assigns_monotonic_sequences() {
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(4);
        let sender = SessionUpSender::new(tx);

        sender
            .send_control(BidiControl::default())
            .await
            .expect("control send");
        sender
            .send_binary_chunk(BinaryChunk {
                stream_id: SESSION_STREAM_ID,
                data: b"payload".to_vec(),
                ..BinaryChunk::default()
            })
            .await
            .expect("chunk send");

        let first = rx.recv().await.expect("first frame");
        assert_eq!(first.sequence, 1);
        let second = rx.recv().await.expect("second frame");
        assert_eq!(second.sequence, 2);
    }

    #[test]
    fn full_jitter_stays_within_bound_and_curve_is_unchanged() {
        // The deterministic curve must be untouched by jitter: the
        // doubling test below still holds, and the jitter sample is
        // always within [0, bound].
        for bound_ms in [0u64, 1, 250, 30_000] {
            let bound = Duration::from_millis(bound_ms);
            for _ in 0..1000 {
                let j = full_jitter(bound);
                assert!(j <= bound, "jitter {j:?} exceeded bound {bound:?}");
            }
        }
        // Zero bound → zero wait (no panic on modulo).
        assert_eq!(full_jitter(Duration::ZERO), Duration::ZERO);
        // Non-degenerate spread: 1000 draws from a 30s bound must not
        // all collapse to one value (the bug being fixed).
        let bound = Duration::from_secs(30);
        let samples: std::collections::HashSet<u128> =
            (0..1000).map(|_| full_jitter(bound).as_millis()).collect();
        assert!(
            samples.len() > 100,
            "jitter not spread: {} distinct",
            samples.len()
        );
    }

    #[test]
    fn next_backoff_doubles_until_cap() {
        assert_eq!(
            next_backoff(SESSION_BACKOFF_INITIAL),
            Duration::from_millis(500)
        );
        assert_eq!(next_backoff(Duration::from_secs(1)), Duration::from_secs(2));
        assert_eq!(next_backoff(Duration::from_secs(20)), SESSION_BACKOFF_MAX);
        // Past the cap stays at the cap.
        assert_eq!(next_backoff(SESSION_BACKOFF_MAX), SESSION_BACKOFF_MAX);
    }

    #[test]
    fn session_phase_edge_relation() {
        use DeviceSessionPhase::*;
        let join = Preluding(PreludeStep::Join);
        let advertise = Preluding(PreludeStep::Advertise);
        // The forward chain.
        assert!(Idle.may_transition_to(&Dialing));
        assert!(Dialing.may_transition_to(&join));
        assert!(join.may_transition_to(&advertise), "prelude steps chain");
        assert!(advertise.may_transition_to(&Live));
        assert!(Live.may_transition_to(&Backoff));
        assert!(Backoff.may_transition_to(&Dialing));
        // Failure and shutdown edges are available from anywhere.
        assert!(Dialing.may_transition_to(&Backoff));
        assert!(join.may_transition_to(&Backoff));
        assert!(Live.may_transition_to(&Idle));
        // Illegal: nothing skips into Live, nothing resurrects from
        // Backoff or Idle without dialing.
        assert!(!Idle.may_transition_to(&Live));
        assert!(!Dialing.may_transition_to(&Live));
        assert!(!Backoff.may_transition_to(&Live));
        assert!(!Idle.may_transition_to(&join));
    }

    #[test]
    fn phase_tracker_walks_the_supervised_lifecycle() {
        let mut t = SessionPhaseTracker::new();
        assert_eq!(t.phase(), DeviceSessionPhase::Idle);
        t.begin_attempt();
        assert_eq!(t.phase(), DeviceSessionPhase::Dialing);
        t.transition(DeviceSessionPhase::Preluding(PreludeStep::Join), "test");
        t.transition(
            DeviceSessionPhase::Preluding(PreludeStep::OwnerProjection),
            "test",
        );
        t.transition(
            DeviceSessionPhase::Preluding(PreludeStep::Advertise),
            "test",
        );
        t.transition(DeviceSessionPhase::Live, "test");
        t.transition(DeviceSessionPhase::Backoff, "healthy");
        // The next attempt re-enters Dialing from Backoff.
        t.begin_attempt();
        assert_eq!(t.phase(), DeviceSessionPhase::Dialing);
        // Shutdown is reachable from any phase.
        t.transition(DeviceSessionPhase::Idle, "supervisor_cancelled");
        assert_eq!(t.phase(), DeviceSessionPhase::Idle);
    }

    #[test]
    fn close_class_fingerprint_table() {
        // The four-class table (F-008/T1.1), pinned: each fingerprint
        // from the 2026-06-11 incident analysis maps to its class.
        let class = |uptime_ms: u64, frames: u64| {
            SessionCloseStats {
                uptime: Duration::from_millis(uptime_ms),
                frames_received: frames,
            }
            .classify()
        };
        // Healthy uptime wins regardless of frame count.
        assert_eq!(class(30_000, 7), CloseClass::Healthy);
        assert_eq!(class(45_000, 0), CloseClass::Healthy);
        // Frameless young session: hub never sent the admission receipt.
        assert_eq!(class(120, 0), CloseClass::NoAdmissionReceipt);
        assert_eq!(class(5_000, 0), CloseClass::NoAdmissionReceipt);
        // Sub-second with only the receipt: displacement signature.
        assert_eq!(class(120, 1), CloseClass::DisplacedSuspect);
        // Survived admission but died young: contract skew family.
        assert_eq!(class(5_000, 2), CloseClass::ContractSkew);
        assert_eq!(class(1_200, 1), CloseClass::ContractSkew);
    }

    #[test]
    fn clean_close_backoff_resets_only_after_healthy_uptime() {
        // A session that outlived SESSION_HEALTHY_MIN_UPTIME earns
        // the reset back to the 250 ms floor.
        let healthy = SessionCloseStats {
            uptime: SESSION_HEALTHY_MIN_UPTIME,
            frames_received: 7,
        };
        assert_eq!(
            backoff_after_clean_close(&healthy, Duration::from_secs(8)),
            SESSION_BACKOFF_INITIAL
        );

        // A sub-second clean close (hub-side displacement / contract
        // skew) must NOT reset the curve — incident 2026-06-11 held
        // 5428 cycles at the 250 ms floor because it did.
        let displaced = SessionCloseStats {
            uptime: Duration::from_millis(120),
            frames_received: 1,
        };
        assert_eq!(
            backoff_after_clean_close(&displaced, Duration::from_secs(8)),
            Duration::from_secs(8),
            "short-lived clean close must keep the escalating backoff"
        );
        // At the floor the policy is a no-op; the supervisor's
        // post-sleep next_backoff() doubling does the escalation.
        assert_eq!(
            backoff_after_clean_close(&displaced, SESSION_BACKOFF_INITIAL),
            SESSION_BACKOFF_INITIAL
        );
    }

    #[tokio::test]
    async fn clean_close_reports_uptime_and_frame_count_stats() {
        let _home = crate::facade::cli::test_support::HomeGuard::new();
        // Device-side fingerprint contract for hub-side close
        // diagnosis (incident 2026-06-11: hub logs lost, device
        // stats are the only surviving evidence). A hub that admits
        // and immediately EOFs the down stream must yield Ok with
        // zero frames and an uptime below the healthy threshold.
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_clean_close_session_hub().await;

        let stats = tokio::time::timeout(
            Duration::from_secs(5),
            dial_and_run_session(
                format!("http://{addr}"),
                "easynet:///r/realm/device/n1".to_string(),
                None,
                None,
                dispatcher,
                None,
                SessionPreludeInputs::new(&[], hub_store()),
            ),
        )
        .await
        .expect("clean-close dial bounded to 5 s")
        .expect("clean hub close must surface as Ok(stats), not an error");

        assert_eq!(stats.frames_received, 0, "empty down stream sent no frames");
        assert!(
            stats.uptime < SESSION_HEALTHY_MIN_UPTIME,
            "immediate close cannot count as healthy uptime, got {:?}",
            stats.uptime
        );
    }

    #[test]
    fn credential_warmup_skips_when_credentials_do_not_match_session_caller() {
        let creds = crate::persistence::config::Credentials {
            node_id: "n1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "http://127.0.0.1:1".to_string(),
            realm: "realm".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some("http://127.0.0.1:1".to_string()),
            username: Some("dev".to_string()),
            user_id: Some("user-dev".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };

        let outcome =
            verify_device_credential_for_credentials("easynet:///r/realm/device/other", creds);

        match outcome {
            CredentialWarmupOutcome::Skipped { reason } => {
                assert!(
                    reason.contains("does not match session caller"),
                    "skip reason should explain caller mismatch, got: {reason}"
                );
            }
            other => panic!("expected caller-mismatch skip, got {other:?}"),
        }
    }

    /// Read one full HTTP/1.1 request — header block plus exactly
    /// `Content-Length` body bytes — off `stream`. A single `read()`
    /// is NOT enough here: ureq writes the header block and the JSON
    /// body in separate syscalls, so they can arrive as separate TCP
    /// segments. A server that answers after the first segment and
    /// drops the socket leaves the in-flight body unread in the
    /// receive buffer; the kernel then resets the connection and the
    /// client surfaces a transport error instead of the 200 (the
    /// 1-in-3 flake of `credential_warmup_posts_current_device_credential`
    /// recorded on 2026-06-11).
    fn read_full_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let mut chunk = [0_u8; 4096];
        loop {
            if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.trim()
                            .eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if buf.len() >= header_end + 4 + content_length {
                    return String::from_utf8_lossy(&buf).to_string();
                }
            }
            let n = stream.read(&mut chunk).expect("read verify request");
            if n == 0 {
                // Peer closed mid-request; return what arrived so the
                // assertion failure shows the truncated request.
                return String::from_utf8_lossy(&buf).to_string();
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    }

    #[test]
    fn credential_warmup_posts_current_device_credential() {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind verify server");
        let addr = listener.local_addr().expect("verify server addr");
        let (tx, rx) = std_mpsc::channel::<String>();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept verify request");
            tx.send(read_full_http_request(&mut stream))
                .expect("send captured request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 14\r\n\r\n{\"valid\":true}",
                )
                .expect("write verify response");
        });

        let creds = crate::persistence::config::Credentials {
            node_id: "n1".to_string(),
            credential_token: "token-secret".to_string(),
            hub_endpoint: format!("http://{addr}"),
            realm: "realm".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some(format!("http://{addr}")),
            username: Some("dev".to_string()),
            user_id: Some("user-dev".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };

        let outcome =
            verify_device_credential_for_credentials("easynet:///r/realm/device/n1", creds);

        assert_eq!(
            outcome,
            CredentialWarmupOutcome::Verified {
                api_base: format!("http://{addr}")
            }
        );
        let request = rx.recv().expect("captured verify request");
        server.join().expect("verify server exits");
        assert!(
            request.starts_with("POST /api/v1/devices/verify-credential "),
            "unexpected request line: {request}"
        );
        assert!(request.contains("\"node_id\":\"n1\""), "{request}");
        assert!(
            request.contains("\"credential_token\":\"token-secret\""),
            "{request}"
        );
    }

    #[tokio::test]
    async fn invalid_endpoint_returns_invalid_endpoint_error() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let result = dial_and_run_session(
            "not a valid ura".to_string(),
            "easynet:///r/realm/device/n1".to_string(),
            None,
            None,
            dispatcher,
            None,
            SessionPreludeInputs::new(&[], hub_store()),
        )
        .await;
        match result {
            Err(SessionError::InvalidEndpoint { .. }) => {}
            other => panic!("expected InvalidEndpoint, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unreachable_endpoint_returns_connect_failed() {
        // 127.0.0.1:1 is an unreserved low port that the OS will
        // refuse on connect (no listener). Bounded by a small
        // tokio::time::timeout so a future bug that hangs the
        // dial step surfaces as a test failure within 5 s, not
        // a CI hang.
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            dial_and_run_session(
                "http://127.0.0.1:1".to_string(),
                "easynet:///r/realm/device/n1".to_string(),
                None,
                None,
                dispatcher,
                None,
                SessionPreludeInputs::new(&[], hub_store()),
            ),
        )
        .await
        .expect("dial step bounded to 5 s");

        match result {
            Err(SessionError::ConnectFailed { .. }) => {}
            other => panic!("expected ConnectFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn owner_projection_publish_failure_blocks_namespace_visible_session() {
        let _home = crate::facade::cli::test_support::HomeGuard::new();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_silent_session_hub().await;
        let owner = "easynet:///r/realm/device/n1";
        let descriptors = vec![crate::runtime::ability_descriptor::AbilityDescriptor::new(
            "agent.start",
            owner,
            crate::runtime::ability_descriptor::Visibility::Scoped,
        )
        .expect("test descriptor")];

        let result = dial_and_run_session_with_idle_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                caller_ura: owner.to_string(),
                signing_seed: None,
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&descriptors, hub_store()),
                idle_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;

        match result {
            Err(SessionError::OwnerProjectionFailed { endpoint, status }) => {
                assert_eq!(endpoint, format!("http://{addr}"));
                assert_eq!(status.code(), tonic::Code::Unimplemented);
            }
            other => panic!("expected OwnerProjectionFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_prelude_publishes_hosted_llm_agent_ability_projection() {
        let _home = crate::facade::cli::test_support::HomeGuard::new();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, invokes, _server) = spawn_recording_prelude_hub().await;
        let device_ura = "easynet:///r/realm/device/n1";
        let agent_ura = "easynet:///r/realm/agent/dev.anthropic";
        crate::persistence::config::save_credentials(&crate::persistence::config::Credentials {
            node_id: "n1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: format!("http://{addr}"),
            realm: "realm".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("dev".to_string()),
            user_id: Some("user-dev".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        })
        .expect("save test credentials");
        crate::persistence::local_agents::save(
            &crate::persistence::local_agents::LocalAgentsFile {
                host_device_agent_ura: device_ura.to_string(),
                hosted_agents: vec![crate::persistence::local_agents::HostedAgentEntry {
                    profile: "llm".to_string(),
                    name: "anthropic".to_string(),
                    agent_ura: agent_ura.to_string(),
                    signing_authority: format!("hosted_by:{device_ura}"),
                    first_seen_at: "2026-06-09T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("save local agents");
        let mut registry = crate::registry::agents::AgentRegistry::default();
        registry.agents.insert(
            "anthropic".to_string(),
            crate::registry::agents::AgentEntry::new(
                crate::registry::agents::AgentType::ClaudeCode,
                Some("sonnet".to_string()),
            ),
        );
        crate::registry::agents::save_agents(&registry).expect("save agents registry");

        let descriptors = vec![AbilityDescriptor::new(
            "agent.start",
            device_ura,
            crate::runtime::ability_descriptor::Visibility::Scoped,
        )
        .expect("test descriptor")];
        let result = dial_and_run_session_with_idle_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                caller_ura: device_ura.to_string(),
                signing_seed: None,
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&descriptors, hub_store()),
                idle_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;
        assert!(
            matches!(result, Err(SessionError::IdleTimeout { .. })),
            "preludes should complete before the silent hub triggers idle timeout: {result:?}"
        );

        let calls = invokes.lock().await.clone();
        assert!(
            calls
                .iter()
                .any(|(name, body)| name == "federation.advertise_agent"
                    && body.get("agent_ura").and_then(Value::as_str) == Some(agent_ura)),
            "hosted agent placement must be advertised before session open: {calls:#?}"
        );
        let agent_projection = calls
            .iter()
            .find(|(name, body)| {
                name == "federation.advertise_abilities"
                    && body.get("owner_ura").and_then(Value::as_str) == Some(agent_ura)
            })
            .expect("hosted agent ability owner projection must be advertised");
        assert_eq!(
            agent_projection
                .1
                .get("host_device_ura")
                .and_then(Value::as_str),
            Some(device_ura)
        );
        let summaries = agent_projection
            .1
            .get("ability_summaries")
            .and_then(Value::as_array)
            .expect("ability summaries");
        assert!(
            summaries.iter().any(|summary| {
                summary
                    .get("callable_summary")
                    .and_then(|value| value.get("public_name"))
                    .and_then(Value::as_str)
                    == Some("chat")
            }),
            "agent projection must expose owner-local chat ability: {summaries:#?}"
        );
    }

    #[tokio::test]
    async fn missing_ca_path_returns_tls_ca_read() {
        // Operator wires `tls_ca_pem_path` in realm-trust.toml to a
        // file that doesn't exist (typo, broken symlink, deleted on
        // disk). The dial must surface a structured `TlsCaRead` error
        // naming the path so the supervisor's reconnect log makes
        // the misconfig actionable rather than presenting it as a
        // generic transport failure.
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let bogus = std::path::PathBuf::from("/tmp/easynet-test-no-such-ca-file-xyz.pem");
        let result = dial_and_run_session(
            "https://127.0.0.1:1".to_string(),
            "easynet:///r/realm/device/n1".to_string(),
            None,
            Some(bogus.as_path()),
            dispatcher,
            None,
            SessionPreludeInputs::new(&[], hub_store()),
        )
        .await;
        match result {
            Err(SessionError::TlsCaRead { path, .. }) => {
                assert_eq!(path, bogus);
            }
            other => panic!("expected TlsCaRead, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn supervisor_exits_on_cancel() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let supervisor_handle = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: "http://127.0.0.1:1".to_string(),
            caller_ura: "easynet:///r/realm/device/n1".to_string(),
            signing_seed: None,
            hub_ca_pem_path: None,
            dispatcher,
            escalation_outbox: None,
            ability_descriptors: Vec::new(),
            hub_published_abilities: hub_store(),
            initial_admission: None,
            user_trust_sync: None,
            cancel: cancel_rx,
        }));

        // Give the supervisor a beat to start its first dial then
        // cancel. The dial will fail (connect refused on port 1)
        // and the supervisor will be sleeping in backoff when the
        // cancel arrives.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = cancel_tx.send(());

        let exit_within_bound =
            tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, supervisor_handle)
                .await
                .expect("supervisor exits promptly after cancel");
        exit_within_bound.expect("supervisor task did not panic");
    }

    #[tokio::test]
    async fn supervisor_reports_initial_admission_failure_before_backoff() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let (probe, admission_rx) = initial_session_admission_probe();

        let supervisor_handle = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: "http://127.0.0.1:1".to_string(),
            caller_ura: "easynet:///r/realm/device/n1".to_string(),
            signing_seed: None,
            hub_ca_pem_path: None,
            dispatcher,
            escalation_outbox: None,
            ability_descriptors: Vec::new(),
            hub_published_abilities: hub_store(),
            initial_admission: Some(probe),
            user_trust_sync: None,
            cancel: cancel_rx,
        }));

        let outcome = tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, admission_rx)
            .await
            .expect("initial admission result is reported before first backoff")
            .expect("initial admission channel remains open");
        let err = outcome.expect_err("unreachable hub must fail initial admission");
        assert!(
            err.contains("failed to connect to hub `http://127.0.0.1:1`"),
            "error should preserve the structured SessionError, got: {err}"
        );

        let _ = cancel_tx.send(());
        supervisor_handle
            .await
            .expect("supervisor task did not panic");
    }

    #[tokio::test]
    async fn supervisor_reports_initial_admission_after_bidi_opens() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_silent_session_hub().await;
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let (probe, admission_rx) = initial_session_admission_probe();

        let supervisor_handle = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: format!("http://{addr}"),
            caller_ura: "easynet:///r/realm/device/n1".to_string(),
            signing_seed: None,
            hub_ca_pem_path: None,
            dispatcher,
            escalation_outbox: None,
            ability_descriptors: Vec::new(),
            hub_published_abilities: hub_store(),
            initial_admission: Some(probe),
            user_trust_sync: None,
            cancel: cancel_rx,
        }));

        let outcome = tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, admission_rx)
            .await
            .expect("initial admission result is reported when bidi opens")
            .expect("initial admission channel remains open");
        outcome.expect("accepted bidi must satisfy initial admission");

        let _ = cancel_tx.send(());
        supervisor_handle
            .await
            .expect("supervisor task did not panic");
    }

    #[tokio::test]
    async fn silent_hub_triggers_idle_timeout_reconnect_error() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_silent_session_hub().await;

        let result = dial_and_run_session_with_idle_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                caller_ura: "easynet:///r/realm/device/n1".to_string(),
                signing_seed: None,
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&[], hub_store()),
                idle_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;

        match result {
            Err(SessionError::IdleTimeout { endpoint, timeout }) => {
                assert_eq!(endpoint, format!("http://{addr}"));
                assert_eq!(timeout, Duration::from_millis(80));
            }
            other => panic!("expected IdleTimeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn out_of_sequence_down_frame_returns_protocol_error() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_out_of_sequence_session_hub().await;

        let result = dial_and_run_session_with_idle_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                caller_ura: "easynet:///r/realm/device/n1".to_string(),
                signing_seed: None,
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&[], hub_store()),
                idle_timeout: Duration::from_secs(1),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;

        match result {
            Err(SessionError::DownStreamSequence {
                endpoint,
                expected,
                actual,
                reason,
            }) => {
                assert_eq!(endpoint, format!("http://{addr}"));
                assert_eq!(expected, 1);
                assert_eq!(actual, 9);
                assert_eq!(reason, REASON_BIDI_DOWN_SEQUENCE);
            }
            other => panic!("expected DownStreamSequence, got {other:?}"),
        }
    }
}
