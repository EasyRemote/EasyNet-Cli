// EasyNet CLI — invocation_transport — session.open initiator (device side)
// ====================================================================
//
// File: src/daemon/invocation/session_initiator.rs
// Description: Device-side caller for `session.open`. At daemon
//              boot a device opens one long-lived `InvokeBidi`
//              stream against its configured hub, sends frame 0 =
//              `EnvelopeOpen` carrying the caller URA, then keeps
//              the stream open for the lifetime of the daemon —
//              this is the canonical reverse channel through which the hub
//              pushes typed dispatch and control frames back to the device.
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
//   an owner-bound canonical signer, and a frame dispatcher; opens one
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
// Boot supplies an owner-bound canonical signer backed by the daemon key
// service. Frame 0 and all public preludes are signed over the same canonical
// invocation bytes the admission gate verifies; private material never enters
// the session runtime and there is no unsigned production fallback.
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
use tokio_util::sync::CancellationToken;
use tonic::Status;

use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
use crate::daemon::identity::self_identity::{CanonicalSigner, SelfIdentityError};

mod connection_state;
mod envelope;
mod frame_loop;
mod frame_scheduler;
mod heartbeat;
mod prelude;
mod supervisor;
mod tasks;
mod transport;

#[cfg(test)]
pub(crate) use connection_state::SessionConnectionStateChange;
pub(crate) use connection_state::{
    project_connection_state, PersistentSessionConnectionStateSink, SessionConnectionStateSink,
};
pub use envelope::build_session_envelope_open;
use frame_loop::{run_live_session, LiveSessionRun};
pub(crate) use prelude::owner_projection_delegation_metadata;
#[cfg(test)]
use prelude::{
    committed_device_native_owner_descriptors, committed_user_service_owner_descriptors,
};
use prelude::{run_session_preludes, SessionPreludeChannels, SessionPreludeRun};
pub use prelude::{
    PairedUserTrustSigner, SessionPreludeInputs, UserTrustBootstrapError, UserTrustSync,
};
pub use supervisor::SessionCloseStats;
use supervisor::{
    backoff_after_clean_close, full_jitter, next_backoff, DeviceSessionPhase, SessionPhaseTracker,
};
#[cfg(test)]
use supervisor::{CloseClass, PreludeStep, SESSION_HEALTHY_MIN_UPTIME};

use axon_sdk::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use axon_sdk::pb::axon::v1::{BidiControl, BinaryChunk, InvokeBidiDown, InvokeBidiUp};

/// Daemon-side ability name this initiator targets. The hub's
/// `InvokeBidi` dispatcher routes on
/// `EnvelopeOpen.target.typed_target.ability.function_name`.
///
/// `session.open` is the daemon-owned long-lived carrier for device
/// session membership. It is a direct wire break from the historical
/// caller-relative alias; no dual-name acceptance is retained.
pub const ABILITY_SESSION_OPEN: &str = crate::daemon::ability::names::device_control::SESSION_OPEN;

/// Stream id used by every BinaryChunk on the session bidi. PR-2
/// sub-spec §2.1 (and the wider RFC-003 transport plane) declares
/// one StreamDescriptor (id=0, content_type="application/json",
/// ordering=STRICT). Multiple streams on the same bidi are
/// reserved for future RFCs and not used by `session.open`.
pub const SESSION_STREAM_ID: u32 = 0;

/// Capacity of the device-side outbound mpsc that
/// `dial_and_run_session` consumes when writing `InvokeBidiUp`
/// frames into the gRPC stream. Sized matching
/// `daemon::invocation::bidi::state::presence::DISPATCH_CHANNEL_CAPACITY` so
/// the hub side and device side use symmetric backpressure
/// budgets.
const SESSION_UP_CHANNEL_CAPACITY: usize = 256;

/// Maximum time a live session writer may wait for one slot in the
/// device-to-hub request stream.
///
/// A full bounded queue is healthy backpressure only while the gRPC request
/// stream continues to drain it. Once no slot opens within this window, the
/// carrier is half-open: keeping its public session state alive would make
/// presence and directory reads lie. The sender therefore faults the entire
/// session attempt and lets the supervisor reconnect.
#[cfg(not(test))]
const SESSION_UP_SEND_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const SESSION_UP_SEND_TIMEOUT: Duration = Duration::from_millis(50);

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

/// Maximum silence window on the down-stream for one live session attempt.
///
/// Why this exists in addition to transport-level HTTP/2 PING:
/// the observed Docker failure mode was asymmetric — the hub-side
/// reader saw `h2 protocol error: error reading a body from
/// connection`, removed presence immediately, but the device-side
/// `down_stream.next()` sometimes remained parked inside tonic's
/// body machinery instead of surfacing EOF/reset promptly.
///
/// The device sends an application heartbeat every 5 seconds. The Hub emits
/// an acknowledgement only after its request-stream reader has consumed that
/// heartbeat. Requiring some Hub-originated down activity within this deadline
/// therefore proves end-to-end progress through both halves of the bidi. The
/// deadline applies before and after `SessionEstablished`; a half-open carrier
/// may never remain publicly Online indefinitely.
pub const SESSION_LIVENESS_TIMEOUT: Duration = Duration::from_secs(15);

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
/// The Hub acknowledges this frame only after draining it, and the device-side
/// `SESSION_LIVENESS_TIMEOUT` requires that acknowledgement (or other down
/// activity) to arrive. Together they make the bidi liveness proof symmetric.
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
pub const DEVICE_DISPATCH_CONTRACT_VERSION: u32 =
    crate::daemon::invocation::bidi::state::presence::CANONICAL_SESSION_CARRIER_VERSION;

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
    /// Register one live carrier attempt before any down-frame can dispatch.
    /// `scope_id` is process-local and unique for the lifetime of this daemon;
    /// wire call ids are only unique inside this scope.
    fn session_started(&self, _scope_id: u64) {}

    /// Retire all dispatch state owned by one carrier attempt. Implementations
    /// must make this idempotent because every exit path (clean close, fault,
    /// timeout, or reconnect displacement) converges here.
    fn session_ended(&self, _scope_id: u64) {}

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
    /// Process-local identity for this one `session.open` attempt. Hub call ids
    /// restart on reconnect, so dispatcher registries must key by
    /// `(scope_id, call_id)` rather than treating a wire call id as global.
    scope_id: u64,
    /// Sequence allocation and channel insertion must be one atomic
    /// step: the hub validates a strictly monotonic up-sequence and
    /// resets the whole session on violation, so two producers that
    /// allocate N and N+1 but enqueue in the opposite order kill the
    /// session. The mutex spans allocate+send — it is the session's
    /// single-writer gate, and the serialization it imposes is
    /// exactly the ordering the wire contract requires.
    sequence_gate: Arc<tokio::sync::Mutex<u64>>,
    /// Negotiated canonical dispatch contract for this session. The
    /// supervisor writes it once after session admission; dispatch is
    /// unavailable until a canonical carrier version is present.
    negotiated_contract: Arc<std::sync::atomic::AtomicU32>,
    lifecycle: Arc<SessionUpLifecycle>,
}

#[derive(Debug)]
struct SessionUpLifecycle {
    faulted: CancellationToken,
    fault: Mutex<Option<SessionUpSendError>>,
}

/// Terminal reason for one `session.open` up-channel attempt.
///
/// This is deliberately session-scoped rather than request-scoped. Once a
/// writer observes a closed or non-progressing request stream, every clone of
/// that sender is stale and the carrier must reconnect before accepting more
/// work.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SessionUpSendError {
    #[error("session up-channel is closed")]
    Closed,
    #[error("session up-channel made no progress for {timeout_ms}ms")]
    Stalled { timeout_ms: u64 },
}

impl SessionUpSender {
    #[must_use]
    pub fn new(tx: mpsc::Sender<InvokeBidiUp>) -> Self {
        static NEXT_SESSION_SCOPE_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(1);
        let scope_id = NEXT_SESSION_SCOPE_ID
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |current| current.checked_add(1),
            )
            .expect("process-local session scope id space exhausted");
        Self {
            tx,
            scope_id,
            // Frame 0 is EnvelopeOpen. First post-frame-0 producer
            // therefore owns sequence = 1.
            sequence_gate: Arc::new(tokio::sync::Mutex::new(1)),
            negotiated_contract: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            lifecycle: Arc::new(SessionUpLifecycle {
                faulted: CancellationToken::new(),
                fault: Mutex::new(None),
            }),
        }
    }

    /// Process-local identity of the carrier attempt that owns this sender.
    #[must_use]
    pub fn scope_id(&self) -> u64 {
        self.scope_id
    }

    /// Supervisor-only: record the hub's negotiated session contract.
    pub fn set_negotiated_contract(&self, version: u32) {
        self.negotiated_contract
            .store(version, std::sync::atomic::Ordering::Release);
    }

    /// True when this session speaks canonical carrier dispatch frames.
    #[must_use]
    pub fn canonical_carrier(&self) -> bool {
        self.negotiated_contract
            .load(std::sync::atomic::Ordering::Acquire)
            >= DEVICE_DISPATCH_CONTRACT_VERSION
    }

    /// Stamp the next sequence number and enqueue under the
    /// single-writer gate, so channel order always equals sequence
    /// order even with concurrent reply producers.
    async fn send_sequenced(&self, payload: UpPayload) -> Result<(), SessionUpSendError> {
        if let Some(fault) = self.fault() {
            return Err(fault);
        }
        let mut next = self.sequence_gate.lock().await;
        if let Some(fault) = self.fault() {
            return Err(fault);
        }
        let sequence = *next;
        let payload_kind = match &payload {
            UpPayload::EnvelopeOpen(_) => "EnvelopeOpen",
            UpPayload::BinaryChunk(_) => "BinaryChunk",
            UpPayload::Control(_) => "Control",
            UpPayload::DispatchResult(_) => "DispatchResult",
            UpPayload::ReverseDispatchCall(_) => "ReverseDispatchCall",
            UpPayload::ReverseBidiInput(_) => "ReverseBidiInput",
        };
        let permit = match tokio::time::timeout(SESSION_UP_SEND_TIMEOUT, self.tx.reserve()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_closed)) => return Err(self.record_fault(SessionUpSendError::Closed)),
            Err(_elapsed) => {
                return Err(self.record_fault(SessionUpSendError::Stalled {
                    timeout_ms: SESSION_UP_SEND_TIMEOUT.as_millis() as u64,
                }));
            }
        };
        permit.send(InvokeBidiUp {
            sequence,
            payload: Some(payload),
            ..InvokeBidiUp::default()
        });
        crate::op_event!(
            component = session,
            kind = session_up_payload_queued,
            sequence = sequence,
            payload = payload_kind,
        );
        *next += 1;
        Ok(())
    }

    fn record_fault(&self, candidate: SessionUpSendError) -> SessionUpSendError {
        let fault = {
            let mut guard = match self.lifecycle.fault.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.get_or_insert(candidate).clone()
        };
        self.lifecycle.faulted.cancel();
        fault
    }

    fn fault(&self) -> Option<SessionUpSendError> {
        let guard = match self.lifecycle.fault.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.clone()
    }

    /// Wait until any producer proves that this session attempt can no longer
    /// make forward progress.
    pub async fn wait_for_fault(&self) -> SessionUpSendError {
        loop {
            if let Some(fault) = self.fault() {
                return fault;
            }
            self.lifecycle.faulted.cancelled().await;
        }
    }

    /// Send a BinaryChunk on the live session, stamping the next
    /// monotonic up-direction sequence number.
    pub async fn send_binary_chunk(&self, chunk: BinaryChunk) -> Result<(), SessionUpSendError> {
        self.send_sequenced(UpPayload::BinaryChunk(chunk)).await
    }

    /// Send any up-direction payload on the live session, stamping
    /// the next monotonic sequence number. Canonical carrier reply frames
    /// (DispatchResult / ReverseDispatchCall) ride this.
    pub async fn send_payload(&self, payload: UpPayload) -> Result<(), SessionUpSendError> {
        self.send_sequenced(payload).await
    }

    /// Send a control frame on the live session, stamping the next
    /// monotonic up-direction sequence number.
    pub async fn send_control(&self, control: BidiControl) -> Result<(), SessionUpSendError> {
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
    signer: Arc<dyn CanonicalSigner>,
    hub_ca_pem_path: Option<&'a Path>,
    dispatcher: Arc<D>,
    escalation_outbox:
        Option<&'a crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox>,
    preludes: SessionPreludeInputs<'a>,
    liveness_timeout: Duration,
    initial_admission: Option<InitialSessionAdmissionProbe>,
    user_trust_sync: Option<&'a UserTrustSync>,
    connection_state_sink: Arc<dyn SessionConnectionStateSink>,
}

pub(crate) struct SessionSupervisorRunConfig<D: SessionFrameDispatcher> {
    pub(crate) hub_endpoint: String,
    pub(crate) signer: Arc<dyn CanonicalSigner>,
    pub(crate) hub_ca_pem_path: Option<PathBuf>,
    pub(crate) dispatcher: Arc<D>,
    pub(crate) escalation_outbox:
        Option<crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox>,
    pub(crate) ability_inventory: SessionAbilityDescriptorInventory,
    pub(crate) authority_published_abilities: Arc<AuthorityPublishedAbilityStore>,
    pub(crate) initial_admission: Option<InitialSessionAdmissionProbe>,
    pub(crate) user_trust_sync: Option<UserTrustSync>,
    pub(crate) connection_state_sink: Arc<dyn SessionConnectionStateSink>,
    pub(crate) cancel: tokio::sync::oneshot::Receiver<()>,
}

#[derive(Clone)]
pub(crate) enum SessionAbilityDescriptorInventory {
    #[cfg(test)]
    Fixed(Vec<AbilityDescriptor>),
    LiveCatalog(Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>),
}

impl SessionAbilityDescriptorInventory {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn fixed(descriptors: Vec<AbilityDescriptor>) -> Self {
        Self::Fixed(descriptors)
    }

    #[must_use]
    pub(crate) fn live_catalog(
        catalog: Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>,
    ) -> Self {
        Self::LiveCatalog(catalog)
    }

    #[must_use]
    fn snapshot(&self) -> Vec<AbilityDescriptor> {
        match self {
            #[cfg(test)]
            Self::Fixed(descriptors) => descriptors.clone(),
            Self::LiveCatalog(catalog) => {
                crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::capture(
                    catalog.as_ref(),
                )
                .all_descriptors()
            }
        }
    }
}

/// Run one `session.open` bidi against `hub_endpoint`. Connects,
/// sends frame 0, streams frames until either the hub closes the
/// down-stream (returns `Ok(())`) or a transport error occurs
/// (returns `Err(...)`).
///
/// The signer's owner is the device's canonical URA per spec §5.1
/// (`easynet:///r/{tenant_id}/agent/{node_id}`). The owner-bound
/// capability signs every public prelude and frame 0 without exposing
/// private key material or allowing this session to select another URA.
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
    signer: Arc<dyn CanonicalSigner>,
    hub_ca_pem_path: Option<&Path>,
    dispatcher: Arc<D>,
    escalation_outbox: Option<
        &crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox,
    >,
    preludes: SessionPreludeInputs<'_>,
) -> Result<SessionCloseStats, SessionError> {
    // One-shot dial (no supervisor): the phase stream still emits,
    // scoped to this call's private tracker.
    let mut phase = SessionPhaseTracker::new();
    phase.begin_attempt();
    dial_and_run_session_with_liveness_timeout(
        SessionDialAttempt {
            hub_endpoint,
            signer,
            hub_ca_pem_path,
            dispatcher,
            escalation_outbox,
            preludes,
            liveness_timeout: SESSION_LIVENESS_TIMEOUT,
            initial_admission: None,
            user_trust_sync: None,
            connection_state_sink: Arc::new(PersistentSessionConnectionStateSink),
        },
        &mut phase,
    )
    .await
}

async fn dial_and_run_session_with_liveness_timeout<D: SessionFrameDispatcher>(
    attempt: SessionDialAttempt<'_, D>,
    phase: &mut SessionPhaseTracker,
) -> Result<SessionCloseStats, SessionError> {
    let SessionDialAttempt {
        hub_endpoint,
        signer,
        hub_ca_pem_path,
        dispatcher,
        escalation_outbox,
        preludes,
        liveness_timeout,
        initial_admission,
        user_trust_sync,
        connection_state_sink,
    } = attempt;
    // Idempotent under a supervisor (begin_attempt already entered
    // Dialing; same-phase transitions early-return); direct callers
    // (one-shot dial, tests) enter the machine here.
    phase.transition(DeviceSessionPhase::Dialing, "dial_entered");

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
        signer: Arc::clone(&signer),
        inputs: preludes,
        user_trust_sync,
        channels: prelude_channels,
    })
    .await?;

    run_live_session(
        LiveSessionRun {
            client,
            hub_endpoint,
            signer,
            dispatcher,
            escalation_outbox,
            liveness_timeout,
            initial_admission,
            connection_state_sink,
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
        signer,
        hub_ca_pem_path,
        dispatcher,
        escalation_outbox,
        ability_inventory,
        authority_published_abilities,
        initial_admission,
        user_trust_sync,
        connection_state_sink,
        mut cancel,
    } = config;

    let mut backoff = SESSION_BACKOFF_INITIAL;
    let mut phase = SessionPhaseTracker::new();
    loop {
        phase.begin_attempt();
        let ability_descriptors = ability_inventory.snapshot();
        // Arm bodies stay trivial: the dial future holds `&mut phase`
        // for its lifetime, so phase handling (like all result
        // handling) happens after the select expression, once the
        // future is out of scope.
        let outcome = tokio::select! {
            _ = &mut cancel => None,
            result = dial_and_run_session_with_liveness_timeout(
                SessionDialAttempt {
                    hub_endpoint: hub_endpoint.clone(),
                    signer: Arc::clone(&signer),
                    hub_ca_pem_path: hub_ca_pem_path.as_deref(),
                    dispatcher: Arc::clone(&dispatcher),
                    escalation_outbox: escalation_outbox.as_ref(),
                    preludes: SessionPreludeInputs::new(
                        &ability_descriptors,
                        Arc::clone(&authority_published_abilities),
                    ),
                    liveness_timeout: SESSION_LIVENESS_TIMEOUT,
                    initial_admission: initial_admission.clone(),
                    user_trust_sync: user_trust_sync.as_ref(),
                    connection_state_sink: Arc::clone(&connection_state_sink),
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
                // A clean EOF still tears down the actual `session.open`
                // carrier and clears SharedSessionOutbox. Keep the public
                // connection projection bound to carrier liveness: a previously
                // negotiated contract is no longer sufficient evidence for
                // FRONTEND_CONNECTED once the down stream has closed.
                project_connection_state(
                    connection_state_sink.as_ref(),
                    crate::daemon::boot::join_connection_state::JoinConnectionState::ConnectedSuspect,
                    crate::daemon::boot::join_connection_state::JoinTransition::OpenSelfSession,
                    "session.clean_close_reconnecting",
                );
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
                project_connection_state(
                    connection_state_sink.as_ref(),
                    crate::daemon::boot::join_connection_state::JoinConnectionState::ConnectedSuspect,
                    crate::daemon::boot::join_connection_state::JoinTransition::OpenSelfSession,
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
    #[error("signing identity `{owner_ura}` failed while preparing {operation}: {source}")]
    SigningIdentity {
        owner_ura: String,
        operation: &'static str,
        #[source]
        source: SelfIdentityError,
    },

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

    #[error("hub `{endpoint}` session up-channel faulted: {source}")]
    UpChannelFault {
        endpoint: String,
        #[source]
        source: SessionUpSendError,
    },

    #[error("hub `{endpoint}` rejected owner projection publish: {status}")]
    OwnerProjectionFailed { endpoint: String, status: Status },

    #[error("hub `{endpoint}` cannot satisfy paired user trust bootstrap: {source}")]
    UserTrustBootstrapFailed {
        endpoint: String,
        #[source]
        source: UserTrustBootstrapError,
    },

    #[error("hub `{endpoint}` hosted-agent prelude failed: {reason}")]
    HostedAgentPreludeFailed { endpoint: String, reason: String },

    #[error("{reason}: hub `{endpoint}` sent down frame sequence {actual}, expected {expected}")]
    DownStreamSequence {
        endpoint: String,
        expected: u64,
        actual: u64,
        reason: &'static str,
    },

    #[error("hub `{endpoint}` session inbound scheduler rejected a frame: {reason}")]
    DownFrameSchedulerSaturated { endpoint: String, reason: String },

    #[error(
        "hub `{endpoint}` sent no down-stream activity for {:?}; forcing reconnect",
        timeout
    )]
    LivenessTimeout { endpoint: String, timeout: Duration },

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
    use std::net::{SocketAddr, TcpListener as StdTcpListener};

    use axon_sdk::pb::axon::v1::invocation_server::{Invocation, InvocationServer};
    use axon_sdk::pb::axon::v1::{
        InvokeRequest, InvokeResponse, InvokeServerStreamRequest, InvokeStreamChunk,
    };
    use futures::{stream, StreamExt as _};
    use rand::random;
    use rcgen::{
        BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa,
        KeyPair, KeyUsagePurpose,
    };
    use serde_json::Value;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::{Identity, ServerTlsConfig};
    use tonic::{Request, Response};

    /// Upper bound for "did the supervisor's async machinery make
    /// progress" assertions (exit-on-cancel, initial-admission report).
    /// This is NOT a product SLA — it only guards against a genuine hang.
    /// Kept generous because the full `cargo test --lib` run (3000+ tests)
    /// saturates the scheduler and an in-process loopback bidi handshake
    /// can spend most of the old 10 s budget in prelude reflection before
    /// reporting admission. This remains a hang guard, not a product SLA.
    const TEST_SUPERVISOR_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);

    fn authority_store() -> Arc<AuthorityPublishedAbilityStore> {
        AuthorityPublishedAbilityStore::new()
    }

    /// Per-test state sink. Session tests exercise lifecycle semantics without
    /// reading or writing the process-global connection-state.json path.
    #[derive(Default)]
    struct InMemorySessionConnectionStateSink {
        changes: Mutex<Vec<SessionConnectionStateChange>>,
    }

    impl InMemorySessionConnectionStateSink {
        fn changes(&self) -> Vec<SessionConnectionStateChange> {
            self.changes
                .lock()
                .expect("in-memory connection-state sink")
                .clone()
        }
    }

    impl SessionConnectionStateSink for InMemorySessionConnectionStateSink {
        fn record(&self, change: SessionConnectionStateChange) -> anyhow::Result<()> {
            self.changes
                .lock()
                .expect("in-memory connection-state sink")
                .push(change);
            Ok(())
        }
    }

    fn isolated_connection_state_sink() -> Arc<dyn SessionConnectionStateSink> {
        Arc::new(InMemorySessionConnectionStateSink::default())
    }

    struct TestSessionSigner {
        owner_ura: String,
        signing_key: ed25519_dalek::SigningKey,
    }

    impl TestSessionSigner {
        fn random(owner_ura: impl Into<String>) -> Arc<dyn CanonicalSigner> {
            Arc::new(Self {
                owner_ura: owner_ura.into(),
                signing_key: ed25519_dalek::SigningKey::from_bytes(&random()),
            })
        }
    }

    #[async_trait::async_trait]
    impl CanonicalSigner for TestSessionSigner {
        fn owner_ura(&self) -> &str {
            &self.owner_ura
        }

        async fn sign_canonical(
            &self,
            canonical_bytes: &[u8],
        ) -> Result<ed25519_dalek::Signature, SelfIdentityError> {
            use ed25519_dalek::Signer as _;

            Ok(self.signing_key.sign(canonical_bytes))
        }

        fn signing_public_key(&self) -> Result<ed25519_dalek::VerifyingKey, SelfIdentityError> {
            Ok(self.signing_key.verifying_key())
        }
    }

    #[test]
    fn owner_projection_uses_only_committed_descriptors() {
        let owner = "easynet:///r/acme/service/alice.pages";
        let other_owner = "easynet:///r/acme/agent/device.dev-1.skill-management";
        let committed = vec![
            AbilityDescriptor::new(
                "project_list",
                owner,
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            )
            .expect("pages descriptor")
            .with_input_schema(serde_json::json!({
                "type": "object",
                "required": ["project_id"],
                "properties": {"project_id": {"type": "string"}}
            })),
            AbilityDescriptor::new(
                "skill.list",
                other_owner,
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            )
            .expect("SystemAgent descriptor"),
        ];
        let mut by_owner =
            committed_user_service_owner_descriptors(&committed, "easynet:///r/acme/user/alice");
        let descriptors = by_owner
            .remove(owner)
            .expect("paired user projection contains its owned Service descriptor");
        let by_public: std::collections::BTreeMap<_, _> = descriptors
            .iter()
            .map(|d| (d.public_name(), d.canonical_ability_ura()))
            .collect();
        assert_eq!(
            by_public.get("project_list").cloned().flatten().as_deref(),
            Some("easynet:///r/acme/ability/service.alice.pages.project_list"),
        );
        assert_eq!(descriptors.len(), 1);
        let schema = &descriptors[0].schema_summary.input;
        assert_eq!(
            schema["required"][0], "project_id",
            "committed schema must survive publication, got: {schema}"
        );
        assert!(
            !descriptors[0].metadata.contains_key("host_node_id"),
            "public Service descriptor identity must not be rewritten with one execution host"
        );
    }

    #[test]
    fn device_native_projection_is_partitioned_by_system_agent_owner() {
        let device = "easynet:///r/acme/device/dev-1";
        let introspection = "easynet:///r/acme/agent/device.dev-1.runtime-introspection";
        let locomotion = "easynet:///r/acme/agent/device.dev-1.locomotion";
        let user_agent = "easynet:///r/acme/agent/alice.worker";
        let descriptors = vec![
            AbilityDescriptor::new(
                crate::daemon::ability::names::governance::META_LIST_ABILITIES,
                introspection,
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Read,
            )
            .expect("introspection descriptor"),
            AbilityDescriptor::new(
                crate::daemon::ability::names::device_control::FS_READ,
                locomotion,
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Read,
            )
            .expect("locomotion descriptor"),
            AbilityDescriptor::new(
                "agent.chat",
                user_agent,
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            )
            .expect("user Agent descriptor"),
        ];

        let by_owner = committed_device_native_owner_descriptors(&descriptors, device);

        assert_eq!(by_owner.len(), 2);
        assert_eq!(by_owner[introspection].len(), 1);
        assert_eq!(by_owner[locomotion].len(), 1);
        assert!(!by_owner.contains_key(user_agent));
        assert!(!by_owner.contains_key(device));
    }

    #[test]
    fn live_catalog_inventory_refreshes_after_dynamic_control_plane_commit() {
        let host_device_ura = "easynet:///r/acme/device/node-a";
        let owner_ura = "easynet:///r/acme/agent/device.node-a.runtime-introspection";
        let catalog = Arc::new(
            crate::daemon::ability::dispatch::AxonAbilityCatalog::new_with_runtime_and_authority_context(
                crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                    crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                    None,
                ),
                crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
                    host_device_ura,
                )
                .expect("test device authority context"),
            ),
        );
        let live_inventory = SessionAbilityDescriptorInventory::live_catalog(Arc::clone(&catalog));
        let fixed_inventory = SessionAbilityDescriptorInventory::fixed(live_inventory.snapshot());

        let before = live_inventory.snapshot();
        assert!(
            !crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::from_descriptors(
                before.clone()
            )
            .resolves(owner_ura, "session.inventory.dynamic"),
            "fresh test catalog should not publish the dynamic test descriptor"
        );

        catalog
            .hot_register_rpc_with_spec(
                "session.inventory.dynamic",
                crate::daemon::ability::dispatch::OwnerKind::runtime_introspection_system(),
                crate::daemon::ability::manifest::AbilityManifest::new(
                    "dynamic",
                    "Dynamic inventory regression descriptor.",
                    serde_json::json!({"type": "object"}),
                )
                .and_then(|manifest| manifest.with_admission_action("invoke"))
                .expect("test dynamic manifest"),
                Arc::new(|_args| Ok(serde_json::json!({"ok": true}))),
            )
            .expect("dynamic control-plane commit succeeds");

        let after = live_inventory.snapshot();
        assert!(
            crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::from_descriptors(
                after
            )
            .resolves(owner_ura, "session.inventory.dynamic"),
            "live session inventory must observe descriptors committed after supervisor construction"
        );
        assert_eq!(
            fixed_inventory.snapshot().len(),
            before.len(),
            "fixed inventories are explicit static test fixtures, not live runtime projections"
        );
    }

    /// A mock dispatcher that just records every down frame it
    /// receives. Used by tests; production wires the real
    /// AxonAbilityCatalog-backed dispatcher.
    #[derive(Default)]
    struct RecordingDispatcher {
        received: tokio::sync::Mutex<Vec<InvokeBidiDown>>,
        started_scopes: std::sync::Mutex<Vec<u64>>,
        ended_scopes: std::sync::Mutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl SessionFrameDispatcher for RecordingDispatcher {
        fn session_started(&self, scope_id: u64) {
            self.started_scopes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(scope_id);
        }

        fn session_ended(&self, scope_id: u64) {
            self.ended_scopes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(scope_id);
        }

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

    fn accepted_prelude_response() -> Response<InvokeResponse> {
        Response::new(InvokeResponse::default())
    }

    fn seed_device_only_session_credentials() {
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "n1".to_string(),
                credential_token: String::new(),
                hub_endpoint: "http://127.0.0.1:1".to_string(),
                realm: "realm".to_string(),
                join_receipt_hash: Some("a".repeat(64)),
                ..Default::default()
            },
        )
        .expect("seed session test credentials");
    }

    /// Minimal committed catalog surface for the daemon-native Agent roots
    /// projected for every paired runtime. Session tests that seed a paired
    /// user must carry the same owner invariants as production boot; an empty
    /// inventory would ask the prelude to advertise owner identities without
    /// any executable LocalRuntime descriptors.
    fn paired_session_ability_descriptors() -> Vec<AbilityDescriptor> {
        [
            (
                "health",
                "pages",
                crate::daemon::ability::descriptors::AdmissionAction::Read,
            ),
            (
                "list",
                "files",
                crate::daemon::ability::descriptors::AdmissionAction::Read,
            ),
        ]
        .into_iter()
        .map(|(ability, agent, action)| {
            AbilityDescriptor::new(
                ability,
                crate::core::ura::agent_ura("realm", "user-dev", agent),
                crate::daemon::ability::descriptors::Visibility::Scoped,
                action,
            )
            .expect("paired session descriptor")
        })
        .collect()
    }

    #[derive(Default)]
    struct SilentSessionHub {
        reject_unary_prelude: bool,
    }

    struct EstablishedThenSilentSessionHub;

    #[derive(Clone)]
    struct NotifyingSessionHub {
        opened: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    }

    #[tonic::async_trait]
    impl Invocation for SilentSessionHub {
        type InvokeStreamStream = TestInvokeStream;
        type InvokeBidiStream = TestInvokeBidiStream;

        async fn invoke(
            &self,
            _request: Request<InvokeRequest>,
        ) -> Result<Response<InvokeResponse>, Status> {
            if self.reject_unary_prelude {
                Err(Status::unimplemented("test hub rejects unary prelude"))
            } else {
                Ok(accepted_prelude_response())
            }
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

    #[tonic::async_trait]
    impl Invocation for EstablishedThenSilentSessionHub {
        type InvokeStreamStream = TestInvokeStream;
        type InvokeBidiStream = TestInvokeBidiStream;

        async fn invoke(
            &self,
            _request: Request<InvokeRequest>,
        ) -> Result<Response<InvokeResponse>, Status> {
            Ok(accepted_prelude_response())
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

            let established = InvokeBidiDown {
                sequence: 0,
                payload: Some(axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::Control(
                    BidiControl {
                        control: Some(
                            axon_sdk::pb::axon::v1::bidi_control::Control::SessionEstablished(
                                axon_sdk::pb::axon::v1::BidiSessionEstablished {
                                    contract_version: DEVICE_DISPATCH_CONTRACT_VERSION,
                                    dispatch_encoding: "proto".to_string(),
                                    session_id: 1,
                                    displaced_prior: false,
                                },
                            ),
                        ),
                    },
                )),
                ..InvokeBidiDown::default()
            };

            Ok(Response::new(Box::pin(
                stream::once(async move { Ok(established) }).chain(stream::pending()),
            ) as Self::InvokeBidiStream))
        }
    }

    #[tonic::async_trait]
    impl Invocation for NotifyingSessionHub {
        type InvokeStreamStream = TestInvokeStream;
        type InvokeBidiStream = TestInvokeBidiStream;

        async fn invoke(
            &self,
            _request: Request<InvokeRequest>,
        ) -> Result<Response<InvokeResponse>, Status> {
            Ok(accepted_prelude_response())
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

            if let Some(tx) = self
                .opened
                .lock()
                .expect("notifying hub opened mutex")
                .take()
            {
                let _ = tx.send(());
            }

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
            Ok(accepted_prelude_response())
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
            Ok(accepted_prelude_response())
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
                    payload: Some(axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::Control(
                        BidiControl {
                            control: Some(
                                axon_sdk::pb::axon::v1::bidi_control::Control::SessionEstablished(
                                    axon_sdk::pb::axon::v1::BidiSessionEstablished {
                                        contract_version: DEVICE_DISPATCH_CONTRACT_VERSION,
                                        dispatch_encoding: "proto".to_string(),
                                        session_id: 1,
                                        displaced_prior: false,
                                    },
                                ),
                            ),
                        },
                    )),
                    ..InvokeBidiDown::default()
                }),
                Ok(InvokeBidiDown {
                    sequence: 9,
                    payload: Some(axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::Control(
                        BidiControl::default(),
                    )),
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
            let mut body: Value = serde_json::from_slice(&request.arguments)
                .map_err(|e| Status::invalid_argument(format!("invalid json args: {e}")))?;
            let caller_ura = request
                .envelope
                .as_ref()
                .and_then(|envelope| envelope.caller.as_ref())
                .map(|caller| caller.ura.as_str())
                .unwrap_or("<missing>");
            if let Some(object) = body.as_object_mut() {
                object.insert(
                    "__caller_ura".to_string(),
                    Value::String(caller_ura.to_string()),
                );
            }
            let function_name =
                crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                    "recording prelude",
                    request.target.as_ref(),
                )?
                .to_string();
            let is_resolve_key = function_name == "federation.resolve_key";
            let result = match function_name.as_str() {
                "federation.join" => serde_json::to_vec(&serde_json::json!({
                    "membership_ura": body.get("membership_ura").and_then(Value::as_str)
                        .ok_or_else(|| Status::invalid_argument("join membership_ura missing"))?,
                    "realm": body.get("realm").and_then(Value::as_str)
                        .ok_or_else(|| Status::invalid_argument("join realm missing"))?,
                    "join_receipt_hash": "test-join-receipt",
                    "authority_published_abilities": [],
                    "authority_abilities_revision": 0,
                    "advertise_contract": {
                        "allowed_owner_prefixes": ["device."],
                        "allows_hosted_agents": true
                    }
                }))
                .expect("serialize recording Hub join receipt"),
                "federation.advertise_agent" => serde_json::to_vec(&serde_json::json!({
                    "ack": true,
                    "assignment": {
                        "agent_ura": body.get("agent_ura").and_then(Value::as_str)
                            .ok_or_else(|| Status::invalid_argument("advertise_agent agent_ura missing"))?,
                        "host_device_ura": caller_ura,
                        "incarnation_id": body.get("incarnation_id").and_then(Value::as_str)
                            .ok_or_else(|| Status::invalid_argument("advertise_agent incarnation_id missing"))?,
                        "generation": 1
                    }
                }))
                .expect("serialize recording Hub assignment receipt"),
                "federation.advertise_abilities" => {
                    let count = body
                        .get("ability_summaries")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            Status::invalid_argument(
                                "advertise_abilities ability_summaries missing",
                            )
                        })?
                        .len();
                    serde_json::to_vec(&serde_json::json!({"ack": true, "count": count}))
                        .expect("serialize recording Hub ability receipt")
                }
                _ if is_resolve_key => br#"{"public_keys_b64":[]}"#.to_vec(),
                _ => Vec::new(),
            };
            self.invokes.lock().await.push((function_name, body));
            if is_resolve_key {
                return Ok(Response::new(InvokeResponse {
                    result,
                    result_content_type: "application/json".to_string(),
                    ..InvokeResponse::default()
                }));
            }
            Ok(Response::new(InvokeResponse {
                result,
                result_content_type: "application/json".to_string(),
                ..InvokeResponse::default()
            }))
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

    async fn spawn_silent_session_hub() -> (SocketAddr, super::tasks::AbortOnDrop) {
        spawn_silent_session_hub_with_prelude(false).await
    }

    async fn spawn_established_then_silent_session_hub() -> (SocketAddr, super::tasks::AbortOnDrop)
    {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind established-then-silent session hub");
        let addr = listener
            .local_addr()
            .expect("established-then-silent hub local addr");
        let incoming = TcpListenerStream::new(listener);
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(InvocationServer::new(EstablishedThenSilentSessionHub))
                .serve_with_incoming(incoming)
                .await
                .expect("established-then-silent session hub server");
        });
        (addr, super::tasks::AbortOnDrop(handle))
    }

    async fn spawn_silent_session_hub_with_prelude(
        reject_unary_prelude: bool,
    ) -> (SocketAddr, super::tasks::AbortOnDrop) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind silent session hub");
        let addr = listener.local_addr().expect("silent hub local addr");
        let incoming = TcpListenerStream::new(listener);
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(InvocationServer::new(SilentSessionHub {
                    reject_unary_prelude,
                }))
                .serve_with_incoming(incoming)
                .await
                .expect("silent session hub server");
        });
        (addr, super::tasks::AbortOnDrop(handle))
    }

    fn reserve_loopback_addr() -> SocketAddr {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("reserve loopback addr");
        let addr = listener.local_addr().expect("reserved addr");
        drop(listener);
        addr
    }

    async fn spawn_notifying_session_hub_on(
        addr: SocketAddr,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        super::tasks::AbortOnDrop,
    ) {
        let listener = TcpListener::bind(addr)
            .await
            .expect("bind notifying session hub");
        let incoming = TcpListenerStream::new(listener);
        let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
        let hub = NotifyingSessionHub {
            opened: Arc::new(std::sync::Mutex::new(Some(opened_tx))),
        };
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(InvocationServer::new(hub))
                .serve_with_incoming(incoming)
                .await
                .expect("notifying session hub server");
        });
        (opened_rx, super::tasks::AbortOnDrop(handle))
    }

    async fn spawn_tls_notifying_session_hub_on(
        addr: SocketAddr,
        cert_pem: String,
        key_pem: String,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        super::tasks::AbortOnDrop,
    ) {
        let listener = TcpListener::bind(addr)
            .await
            .expect("bind tls notifying session hub");
        let incoming = TcpListenerStream::new(listener);
        let (opened_tx, opened_rx) = tokio::sync::oneshot::channel();
        let hub = NotifyingSessionHub {
            opened: Arc::new(std::sync::Mutex::new(Some(opened_tx))),
        };
        let identity = Identity::from_pem(cert_pem, key_pem);
        let tls_config = ServerTlsConfig::new().identity(identity);
        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .tls_config(tls_config)
                .expect("tls config")
                .add_service(InvocationServer::new(hub))
                .serve_with_incoming(incoming)
                .await
                .expect("tls notifying session hub server");
        });
        (opened_rx, super::tasks::AbortOnDrop(handle))
    }

    fn test_ca_and_leaf() -> (Certificate, String, String) {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "EasyNet session test CA");
        ca_params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        ca_params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        ca_params.key_usages.push(KeyUsagePurpose::CrlSign);
        let ca_key = KeyPair::generate().expect("ca key");
        let ca = ca_params.self_signed(&ca_key).expect("ca cert");

        let mut leaf_params =
            CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()])
                .expect("leaf params");
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "localhost");
        leaf_params
            .key_usages
            .push(KeyUsagePurpose::DigitalSignature);
        leaf_params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);
        let leaf_key = KeyPair::generate().expect("leaf key");
        let leaf = leaf_params
            .signed_by(&leaf_key, &ca, &ca_key)
            .expect("leaf cert");
        let ca_pem = ca.pem();
        (
            ca,
            format!("{}{}", leaf.pem(), ca_pem),
            leaf_key.serialize_pem(),
        )
    }

    async fn spawn_clean_close_session_hub() -> (SocketAddr, super::tasks::AbortOnDrop) {
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
        (addr, super::tasks::AbortOnDrop(handle))
    }

    async fn spawn_out_of_sequence_session_hub() -> (SocketAddr, super::tasks::AbortOnDrop) {
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
        (addr, super::tasks::AbortOnDrop(handle))
    }

    async fn spawn_recording_prelude_hub() -> (
        SocketAddr,
        Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
        super::tasks::AbortOnDrop,
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
        (addr, invokes, super::tasks::AbortOnDrop(handle))
    }

    #[tokio::test]
    async fn build_session_envelope_open_carries_caller_ura_and_ability_name() {
        let expected_device = crate::core::ura::device_ura("realm", "n1");
        let expected_authority = crate::core::ura::hub_ura("realm");
        let expected_ability =
            crate::core::ura::owner_ability_ura(&expected_authority, ABILITY_SESSION_OPEN)
                .expect("session.open Ability URA");
        let signer = TestSessionSigner::random(expected_device.as_str());
        let frame = build_session_envelope_open(signer.as_ref())
            .await
            .expect("signed frame");
        let UpPayload::EnvelopeOpen(eo) = frame.payload.expect("payload") else {
            panic!("frame 0 must be EnvelopeOpen");
        };
        let target = eo.target.as_ref().expect("typed session.open target");
        assert_eq!(
            eo.envelope
                .as_ref()
                .and_then(|e| e.caller.as_ref())
                .map(|a| a.ura.as_str())
                .unwrap_or(""),
            expected_device,
        );
        assert_eq!(
            eo.envelope
                .as_ref()
                .and_then(|e| e.callee.as_ref())
                .map(|a| a.ura.as_str())
                .unwrap_or(""),
            expected_authority,
        );
        let descriptor_ref =
            crate::daemon::invocation::dispatch::invocation_wire::descriptor_ref_from_invocation_target(
                "test session.open",
                &expected_authority,
                Some(target),
            )
            .expect("session.open descriptor ref");
        assert!(
            descriptor_ref.starts_with(&format!("{expected_ability}@")),
            "{descriptor_ref}"
        );
        let axon_sdk::pb::axon::v1::invocation_target::TypedTarget::Ability(ability) =
            target.typed_target.as_ref().expect("typed ability target");
        assert_eq!(ability.function_name, ABILITY_SESSION_OPEN);
        assert!(eo.metadata.is_empty());
    }

    #[tokio::test]
    async fn build_session_envelope_open_includes_one_stream_descriptor() {
        let signer = TestSessionSigner::random("easynet:///r/realm/device/n1");
        let frame = build_session_envelope_open(signer.as_ref())
            .await
            .expect("signed frame");
        let UpPayload::EnvelopeOpen(eo) = frame.payload.expect("payload") else {
            panic!("payload");
        };
        assert_eq!(eo.streams.len(), 1);
        assert_eq!(eo.streams[0].stream_id, SESSION_STREAM_ID);
        assert_eq!(eo.streams[0].content_type, "application/json");
    }

    #[tokio::test]
    async fn build_session_envelope_open_adds_signature_and_nonce() {
        let signer = TestSessionSigner::random("easynet:///r/realm/device/n1");
        let frame = build_session_envelope_open(signer.as_ref())
            .await
            .expect("signed frame");
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

    #[tokio::test]
    async fn session_up_sender_stall_is_terminal_for_every_clone() {
        let (tx, mut rx) = mpsc::channel::<InvokeBidiUp>(1);
        tx.send(InvokeBidiUp::default())
            .await
            .expect("prefill session up-channel");
        let sender = SessionUpSender::new(tx);
        let observer = sender.clone();

        let send_error = sender
            .send_control(BidiControl::default())
            .await
            .expect_err("saturated carrier must fault");
        assert_eq!(
            send_error,
            SessionUpSendError::Stalled {
                timeout_ms: SESSION_UP_SEND_TIMEOUT.as_millis() as u64,
            }
        );
        assert_eq!(observer.wait_for_fault().await, send_error);

        // Draining the queue after the deadline cannot resurrect a stale
        // session attempt. Only the supervisor may publish a fresh sender.
        let _ = rx.recv().await;
        assert_eq!(
            observer
                .send_control(BidiControl::default())
                .await
                .expect_err("faulted carrier cannot be reused"),
            send_error,
        );
        assert!(
            rx.try_recv().is_err(),
            "terminal sender must not enqueue a post-fault frame"
        );
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
        let trust_bootstrap = Preluding(PreludeStep::TrustBootstrap);
        let advertise = Preluding(PreludeStep::Advertise);
        // The forward chain.
        assert!(Idle.may_transition_to(&Dialing));
        assert!(Dialing.may_transition_to(&join));
        assert!(
            join.may_transition_to(&trust_bootstrap),
            "prelude steps chain"
        );
        assert!(
            trust_bootstrap.may_transition_to(&advertise),
            "prelude steps chain"
        );
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
            DeviceSessionPhase::Preluding(PreludeStep::TrustBootstrap),
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
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let ability_descriptors = paired_session_ability_descriptors();
        // Device-side fingerprint contract for hub-side close
        // diagnosis (incident 2026-06-11: hub logs lost, device
        // stats are the only surviving evidence). A hub that admits
        // and immediately EOFs the down stream must yield Ok with
        // zero frames and an uptime below the healthy threshold.
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_clean_close_session_hub().await;

        let stats = tokio::time::timeout(
            TEST_SUPERVISOR_PROGRESS_TIMEOUT,
            dial_and_run_session(
                format!("http://{addr}"),
                TestSessionSigner::random("easynet:///r/realm/device/n1"),
                None,
                Arc::clone(&dispatcher),
                None,
                SessionPreludeInputs::new(&ability_descriptors, authority_store()),
            ),
        )
        .await
        .expect("clean-close dial completes within the shared session progress bound")
        .expect("clean hub close must surface as Ok(stats), not an error");

        assert_eq!(stats.frames_received, 0, "empty down stream sent no frames");
        assert!(
            stats.uptime < SESSION_HEALTHY_MIN_UPTIME,
            "immediate close cannot count as healthy uptime, got {:?}",
            stats.uptime
        );
        let started = dispatcher
            .started_scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let ended = dispatcher
            .ended_scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(started.len(), 1, "one carrier scope must be registered");
        assert_eq!(
            ended, started,
            "every carrier exit must retire its exact scope"
        );
    }

    #[tokio::test]
    async fn clean_close_projects_connection_suspect_when_carrier_drops() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let (addr, _server) = spawn_clean_close_session_hub().await;
        let sink = Arc::new(InMemorySessionConnectionStateSink::default());
        let sink_port: Arc<dyn SessionConnectionStateSink> = sink.clone();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let supervisor = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: format!("http://{addr}"),
            signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
            hub_ca_pem_path: None,
            dispatcher: Arc::new(RecordingDispatcher::default()),
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(
                paired_session_ability_descriptors(),
            ),
            authority_published_abilities: authority_store(),
            initial_admission: None,
            user_trust_sync: None,
            connection_state_sink: sink_port,
            cancel: cancel_rx,
        }));

        tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, async {
            loop {
                if sink.changes().iter().any(|change| {
                    change.state
                        == crate::daemon::boot::join_connection_state::JoinConnectionState::ConnectedSuspect
                        && change.transition
                            == crate::daemon::boot::join_connection_state::JoinTransition::OpenSelfSession
                        && change.source == "session.clean_close_reconnecting"
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("clean session close must revoke connected projection");

        let _ = cancel_tx.send(());
        tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, supervisor)
            .await
            .expect("supervisor stops after cancellation")
            .expect("supervisor task did not panic");
    }

    #[tokio::test]
    async fn invalid_endpoint_returns_invalid_endpoint_error() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let result = dial_and_run_session(
            "not a valid ura".to_string(),
            TestSessionSigner::random("easynet:///r/realm/device/n1"),
            None,
            dispatcher,
            None,
            SessionPreludeInputs::new(&[], authority_store()),
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
                TestSessionSigner::random("easynet:///r/realm/device/n1"),
                None,
                dispatcher,
                None,
                SessionPreludeInputs::new(&[], authority_store()),
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
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_silent_session_hub_with_prelude(true).await;
        let host = "easynet:///r/realm/device/n1";
        let owner = "easynet:///r/realm/agent/device.n1.agent-management";
        let descriptors = vec![crate::daemon::ability::descriptors::AbilityDescriptor::new(
            "agent.start",
            owner,
            crate::daemon::ability::descriptors::Visibility::Scoped,
            crate::daemon::ability::descriptors::AdmissionAction::Invoke,
        )
        .expect("test descriptor")];

        let result = dial_and_run_session_with_liveness_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                signer: TestSessionSigner::random(host),
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&descriptors, authority_store()),
                liveness_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
                connection_state_sink: isolated_connection_state_sink(),
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
    async fn missing_paired_user_trust_blocks_session_before_bidi_open() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, invokes, _server) = spawn_recording_prelude_hub().await;
        let device_ura = "easynet:///r/realm/device/n1";
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
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
            },
        )
        .expect("save test credentials");
        let trust_dir = tempfile::tempdir().expect("trust tempdir");
        let user_trust_sync = UserTrustSync {
            daemon_realm: "realm".to_string(),
            trust_anchor_path: trust_dir.path().join("realm-trust.toml"),
            cell: crate::daemon::trust::cell::SharedTrustAnchor::default(),
            user_signer: super::prelude::PairedUserTrustSigner::fixed(TestSessionSigner::random(
                "easynet:///r/realm/user/user-dev",
            )),
        };

        let result = dial_and_run_session_with_liveness_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                signer: TestSessionSigner::random(device_ura),
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&[], authority_store()),
                liveness_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: Some(&user_trust_sync),
                connection_state_sink: isolated_connection_state_sink(),
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;

        match result {
            Err(SessionError::UserTrustBootstrapFailed { endpoint, source }) => {
                assert_eq!(endpoint, format!("http://{addr}"));
                match source {
                    UserTrustBootstrapError::MissingAtHub { user_ura } => {
                        assert_eq!(user_ura, "easynet:///r/realm/user/user-dev");
                    }
                    other => panic!("expected MissingAtHub, got {other:?}"),
                }
            }
            other => panic!("expected UserTrustBootstrapFailed, got {other:?}"),
        }
        let calls = invokes.lock().await.clone();
        assert!(
            calls
                .iter()
                .any(|(name, body)| name == "federation.resolve_key"
                    && body.get("agent_ura").and_then(Value::as_str)
                        == Some("easynet:///r/realm/user/user-dev")
                    && body.get("__caller_ura").and_then(Value::as_str)
                        == Some("easynet:///r/realm/user/user-dev")),
            "prelude must explicitly resolve paired user key before session.open: {calls:#?}"
        );
    }

    #[tokio::test]
    async fn paired_user_trust_resolve_pins_only_the_managed_signer_pubkey() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, invokes, _server) = spawn_recording_prelude_hub().await;
        let device_ura = "easynet:///r/realm/device/n1";
        let user_ura = "easynet:///r/realm/user/user-dev";
        let user_pubkey_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let user_signer = TestSessionSigner::random(user_ura);
        let signer_pubkey_b64 = {
            use base64::Engine as _;

            base64::engine::general_purpose::STANDARD.encode(
                user_signer
                    .signing_public_key()
                    .expect("paired user signer public key")
                    .to_bytes(),
            )
        };
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
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
            },
        )
        .expect("save test credentials");
        let trust_dir = tempfile::tempdir().expect("trust tempdir");
        let user_anchor = crate::daemon::trust::anchor::RealmTrustAnchor::from_entries(vec![
            crate::daemon::trust::anchor::TrustedAgent {
                agent_ura: user_ura.to_string(),
                public_key_b64: user_pubkey_b64.to_string(),
                role: crate::daemon::trust::anchor::TrustAnchorRole::User,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
        ])
        .expect("user trust anchor");
        let user_trust_sync = UserTrustSync {
            daemon_realm: "realm".to_string(),
            trust_anchor_path: trust_dir.path().join("realm-trust.toml"),
            cell: crate::daemon::trust::cell::SharedTrustAnchor::new(Arc::new(user_anchor)),
            user_signer: super::prelude::PairedUserTrustSigner::fixed(Arc::clone(&user_signer)),
        };

        let result = dial_and_run_session_with_liveness_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                signer: TestSessionSigner::random(device_ura),
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&[], authority_store()),
                liveness_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: Some(&user_trust_sync),
                connection_state_sink: isolated_connection_state_sink(),
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;

        assert!(
            matches!(
                result,
                Err(SessionError::UserTrustBootstrapFailed {
                    source: UserTrustBootstrapError::MissingAtHub { .. },
                    ..
                })
            ),
            "recording hub returns an empty resolve_key response, got {result:?}"
        );
        let calls = invokes.lock().await.clone();
        let publish_index = calls
            .iter()
            .position(|(name, body)| {
                name == "identity.register_pubkey"
                    && body.get("principal_ura").and_then(Value::as_str) == Some(user_ura)
                    && body.get("public_key_b64").and_then(Value::as_str)
                        == Some(signer_pubkey_b64.as_str())
                    && body.get("__caller_ura").and_then(Value::as_str) == Some(user_ura)
            })
            .expect("prelude must publish the paired user key as the paired User");
        let user_resolves = calls
            .iter()
            .enumerate()
            .filter(|(_, (name, body))| {
                name == "federation.resolve_key"
                    && body.get("agent_ura").and_then(Value::as_str) == Some(user_ura)
            })
            .map(|(index, (_, body))| {
                assert!(
                    index > publish_index,
                    "paired User key publication must precede resolve_key: {calls:#?}"
                );
                assert_eq!(
                    body.get("__caller_ura").and_then(Value::as_str),
                    Some(user_ura),
                    "paired user resolve_key must pin the presented public key as the paired User"
                );
                body.get("presented_pubkey_b64")
                    .and_then(Value::as_str)
                    .expect("every paired User resolve_key request must pin one local key")
                    .to_string()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            user_resolves,
            std::collections::BTreeSet::from([signer_pubkey_b64]),
            "paired User trust bootstrap must resolve only the managed signer; locally cached browser keys belong to the ephemeral Hub-attested caller projection"
        );
    }

    #[tokio::test]
    async fn session_prelude_publishes_hosted_llm_agent_ability_projection() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, invokes, _server) = spawn_recording_prelude_hub().await;
        let device_ura = "easynet:///r/realm/device/n1";
        let agent_ura = "easynet:///r/realm/agent/user-dev.anthropic";
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
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
            },
        )
        .expect("save test credentials");
        crate::daemon::persistence::local_agents::save(
            &crate::daemon::persistence::local_agents::LocalAgentsFile {
                host_device_ura: device_ura.to_string(),
                hosted_agents: vec![crate::daemon::persistence::local_agents::HostedAgentEntry {
                    profile: "llm".to_string(),
                    name: "anthropic".to_string(),
                    agent_ura: agent_ura.to_string(),
                    signing_authority: format!("hosted_by:{device_ura}"),
                    first_seen_at: "2026-06-09T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("save local agents");
        let descriptors = vec![
            AbilityDescriptor::new(
                "agent.start",
                crate::core::ura::device_agent_ura(
                    "realm",
                    "n1",
                    crate::daemon::ability::names::agents::AGENT_MANAGEMENT_SYSTEM_AGENT_ID,
                ),
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            )
            .expect("test descriptor"),
            AbilityDescriptor::new(
                "chat",
                agent_ura,
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Invoke,
            )
            .expect("committed hosted-agent descriptor"),
            AbilityDescriptor::new(
                "health",
                crate::core::ura::agent_ura("realm", "user-dev", "pages"),
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Read,
            )
            .expect("committed Pages descriptor"),
            AbilityDescriptor::new(
                "list",
                crate::core::ura::agent_ura("realm", "user-dev", "files"),
                crate::daemon::ability::descriptors::Visibility::Scoped,
                crate::daemon::ability::descriptors::AdmissionAction::Read,
            )
            .expect("committed Files descriptor"),
        ];
        let signer = TestSessionSigner::random(device_ura);
        let paired_user_signer = PairedUserTrustSigner::fixed(TestSessionSigner::random(
            "easynet:///r/realm/user/user-dev",
        ));
        let expected_public_key_hex = hex::encode(
            signer
                .signing_public_key()
                .expect("test signer public key")
                .to_bytes(),
        );
        let result = dial_and_run_session_with_liveness_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                signer,
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&descriptors, authority_store())
                    .with_paired_user_signer(paired_user_signer),
                liveness_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
                connection_state_sink: isolated_connection_state_sink(),
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;
        assert!(
            matches!(result, Err(SessionError::LivenessTimeout { .. })),
            "preludes should complete before the silent hub triggers idle timeout: {result:?}"
        );

        let calls = invokes.lock().await.clone();
        let join = calls
            .iter()
            .find(|(name, _)| name == "federation.join")
            .expect("join prelude must be sent before session open");
        assert_eq!(
            join.1.get("membership_ura").and_then(Value::as_str),
            Some(device_ura)
        );
        assert_eq!(
            join.1.get("public_key_hex").and_then(Value::as_str),
            Some(expected_public_key_hex.as_str())
        );
        let agent_advertise = calls
            .iter()
            .find(|(name, body)| {
                name == "federation.advertise_agent"
                    && body.get("agent_ura").and_then(Value::as_str) == Some(agent_ura)
            })
            .expect("hosted agent placement must be advertised before session open");
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
        assert_eq!(
            agent_advertise
                .1
                .get("incarnation_id")
                .and_then(Value::as_str)
                .map(str::len),
            Some(32),
            "Device identity advertisement must carry its durable incarnation key"
        );
        assert_eq!(
            agent_projection.1.get("generation").and_then(Value::as_u64),
            Some(1),
            "ability projection must use the Hub-assigned generation"
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
            TestSessionSigner::random("easynet:///r/realm/device/n1"),
            Some(bogus.as_path()),
            dispatcher,
            None,
            SessionPreludeInputs::new(&[], authority_store()),
        )
        .await;
        match result {
            Err(SessionError::TlsCaRead { path, .. }) => {
                assert_eq!(path, bogus);
            }
            other => panic!("expected TlsCaRead, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_supervisors_project_only_into_their_injected_sinks() {
        use crate::daemon::boot::join_connection_state::{
            load_snapshot, record_snapshot, JoinConnectionSnapshot, JoinConnectionState,
            JoinTransition,
        };

        let _home = crate::cli::commands::test_support::HomeGuard::new();
        record_snapshot(JoinConnectionSnapshot::from_parts(
            JoinConnectionState::DisconnectedRemoved,
            Some(JoinTransition::RemovePresence),
            "realm",
            "device-baseline",
            Some("http://hub.invalid".to_string()),
            "test.baseline",
        ));

        let sink_a = Arc::new(InMemorySessionConnectionStateSink::default());
        let sink_b = Arc::new(InMemorySessionConnectionStateSink::default());
        let sink_a_port: Arc<dyn SessionConnectionStateSink> = sink_a.clone();
        let sink_b_port: Arc<dyn SessionConnectionStateSink> = sink_b.clone();
        let endpoint_a = format!("http://{}", reserve_loopback_addr());
        let endpoint_b = format!("http://{}", reserve_loopback_addr());
        let (cancel_a_tx, cancel_a_rx) = tokio::sync::oneshot::channel::<()>();
        let (cancel_b_tx, cancel_b_rx) = tokio::sync::oneshot::channel::<()>();
        let (probe_a, admission_a_rx) = initial_session_admission_probe();
        let (probe_b, admission_b_rx) = initial_session_admission_probe();

        let supervisor_a = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: endpoint_a,
            signer: TestSessionSigner::random("easynet:///r/realm/device/a"),
            hub_ca_pem_path: None,
            dispatcher: Arc::new(RecordingDispatcher::default()),
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(Vec::new()),
            authority_published_abilities: authority_store(),
            initial_admission: Some(probe_a),
            user_trust_sync: None,
            connection_state_sink: sink_a_port,
            cancel: cancel_a_rx,
        }));
        let supervisor_b = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: endpoint_b,
            signer: TestSessionSigner::random("easynet:///r/realm/device/b"),
            hub_ca_pem_path: None,
            dispatcher: Arc::new(RecordingDispatcher::default()),
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(Vec::new()),
            authority_published_abilities: authority_store(),
            initial_admission: Some(probe_b),
            user_trust_sync: None,
            connection_state_sink: sink_b_port,
            cancel: cancel_b_rx,
        }));

        let (admission_a, admission_b) =
            tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, async {
                tokio::join!(admission_a_rx, admission_b_rx)
            })
            .await
            .expect("both supervisors report their first failed admission");
        assert!(admission_a
            .expect("supervisor A admission probe remains open")
            .is_err());
        assert!(admission_b
            .expect("supervisor B admission probe remains open")
            .is_err());

        let _ = cancel_a_tx.send(());
        let _ = cancel_b_tx.send(());
        tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, async {
            let (result_a, result_b) = tokio::join!(supervisor_a, supervisor_b);
            result_a.expect("supervisor A task did not panic");
            result_b.expect("supervisor B task did not panic");
        })
        .await
        .expect("both supervisors stop after cancellation");

        for changes in [sink_a.changes(), sink_b.changes()] {
            assert!(
                !changes.is_empty(),
                "each injected sink receives its owner event"
            );
            assert!(changes.iter().all(|change| {
                change.state == JoinConnectionState::ConnectedSuspect
                    && change.transition == JoinTransition::OpenSelfSession
                    && change.source == "session.error_reconnecting"
            }));
        }

        let persisted = load_snapshot().expect("baseline snapshot remains readable");
        assert_eq!(persisted.state, "OFFLINE");
        assert_eq!(persisted.state_code, "F530");
        assert_eq!(persisted.source, "test.baseline");
    }

    #[tokio::test]
    async fn supervisor_exits_on_cancel() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let supervisor_handle = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: "http://127.0.0.1:1".to_string(),
            signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
            hub_ca_pem_path: None,
            dispatcher,
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(Vec::new()),
            authority_published_abilities: authority_store(),
            initial_admission: None,
            user_trust_sync: None,
            connection_state_sink: isolated_connection_state_sink(),
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
            signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
            hub_ca_pem_path: None,
            dispatcher,
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(Vec::new()),
            authority_published_abilities: authority_store(),
            initial_admission: Some(probe),
            user_trust_sync: None,
            connection_state_sink: isolated_connection_state_sink(),
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
    async fn supervisor_reconnects_when_hub_starts_after_cli_daemon() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        // Regression guard for the product-lifecycle split: a
        // device-mode daemon may be locally running while the Hub is
        // down. The session supervisor must keep running after the
        // first connect-refused admission failure and create
        // `session.open` automatically when the Hub appears at the
        // same endpoint.
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let hub_addr = reserve_loopback_addr();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let (probe, admission_rx) = initial_session_admission_probe();

        let supervisor_handle = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: format!("http://{hub_addr}"),
            signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
            hub_ca_pem_path: None,
            dispatcher,
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(
                paired_session_ability_descriptors(),
            ),
            authority_published_abilities: authority_store(),
            initial_admission: Some(probe),
            user_trust_sync: None,
            connection_state_sink: isolated_connection_state_sink(),
            cancel: cancel_rx,
        }));

        let initial = tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, admission_rx)
            .await
            .expect("initial hub-down admission result is reported")
            .expect("initial admission channel remains open");
        let err = initial.expect_err("missing Hub must fail first admission");
        assert!(
            err.contains("failed to connect to hub"),
            "initial failure should be a connect failure, got: {err}"
        );

        let (opened_rx, hub_handle) = spawn_notifying_session_hub_on(hub_addr).await;
        tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, opened_rx)
            .await
            .expect("supervisor reconnects after Hub starts")
            .expect("notifying Hub observes session.open");

        let _ = cancel_tx.send(());
        tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, supervisor_handle)
            .await
            .expect("supervisor exits promptly after cancel")
            .expect("supervisor task did not panic");
        drop(hub_handle);
    }

    #[tokio::test]
    async fn session_channel_connects_to_tls_hub_with_pinned_ca() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let temp = tempfile::tempdir().expect("tempdir");
        let ca_path = temp.path().join("ca.pem");
        let (ca, cert_pem, key_pem) = test_ca_and_leaf();
        std::fs::write(&ca_path, ca.pem()).expect("write ca");

        let hub_addr = reserve_loopback_addr();
        let (_opened_rx, hub_handle) =
            spawn_tls_notifying_session_hub_on(hub_addr, cert_pem, key_pem).await;

        transport::connect_session_channel(&format!("https://{hub_addr}"), Some(&ca_path))
            .await
            .expect("session channel must connect to TLS hub with pinned CA");

        drop(hub_handle);
    }

    #[tokio::test]
    async fn supervisor_opens_session_to_tls_hub_with_pinned_ca() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let temp = tempfile::tempdir().expect("tempdir");
        let ca_path = temp.path().join("ca.pem");
        let (ca, cert_pem, key_pem) = test_ca_and_leaf();
        std::fs::write(&ca_path, ca.pem()).expect("write ca");

        let hub_addr = reserve_loopback_addr();
        let (opened_rx, hub_handle) =
            spawn_tls_notifying_session_hub_on(hub_addr, cert_pem, key_pem).await;
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let supervisor_handle = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: format!("https://{hub_addr}"),
            signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
            hub_ca_pem_path: Some(ca_path),
            dispatcher: Arc::new(RecordingDispatcher::default()),
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(Vec::new()),
            authority_published_abilities: authority_store(),
            initial_admission: None,
            user_trust_sync: None,
            connection_state_sink: isolated_connection_state_sink(),
            cancel: cancel_rx,
        }));

        tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, opened_rx)
            .await
            .expect("supervisor opens TLS session")
            .expect("notifying TLS Hub observes session.open");

        let _ = cancel_tx.send(());
        tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, supervisor_handle)
            .await
            .expect("supervisor exits promptly after cancel")
            .expect("supervisor task did not panic");
        drop(hub_handle);
    }

    #[tokio::test]
    async fn supervisor_reports_initial_admission_after_bidi_opens() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_silent_session_hub().await;
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let (probe, admission_rx) = initial_session_admission_probe();

        let supervisor_handle = tokio::spawn(run_session_supervisor(SessionSupervisorRunConfig {
            hub_endpoint: format!("http://{addr}"),
            signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
            hub_ca_pem_path: None,
            dispatcher,
            escalation_outbox: None,
            ability_inventory: SessionAbilityDescriptorInventory::fixed(
                paired_session_ability_descriptors(),
            ),
            authority_published_abilities: authority_store(),
            initial_admission: Some(probe),
            user_trust_sync: None,
            connection_state_sink: isolated_connection_state_sink(),
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
    async fn silent_hub_triggers_liveness_timeout_reconnect_error() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_silent_session_hub().await;
        let ability_descriptors = paired_session_ability_descriptors();

        let result = dial_and_run_session_with_liveness_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&ability_descriptors, authority_store()),
                liveness_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
                connection_state_sink: isolated_connection_state_sink(),
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;

        match result {
            Err(SessionError::LivenessTimeout { endpoint, timeout }) => {
                assert_eq!(endpoint, format!("http://{addr}"));
                assert_eq!(timeout, Duration::from_millis(80));
            }
            other => panic!("expected LivenessTimeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn established_session_without_heartbeat_ack_triggers_liveness_timeout() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_established_then_silent_session_hub().await;
        let ability_descriptors = paired_session_ability_descriptors();

        let result = dial_and_run_session_with_liveness_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&ability_descriptors, authority_store()),
                liveness_timeout: Duration::from_millis(80),
                initial_admission: None,
                user_trust_sync: None,
                connection_state_sink: isolated_connection_state_sink(),
            },
            &mut SessionPhaseTracker::new(),
        )
        .await;

        assert!(
            matches!(result, Err(SessionError::LivenessTimeout { .. })),
            "an admitted session without Hub heartbeat acknowledgements must terminate, got {result:?}"
        );
    }

    #[tokio::test]
    async fn out_of_sequence_down_frame_returns_protocol_error() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        seed_device_only_session_credentials();
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_out_of_sequence_session_hub().await;
        let ability_descriptors = paired_session_ability_descriptors();

        let result = dial_and_run_session_with_liveness_timeout(
            SessionDialAttempt {
                hub_endpoint: format!("http://{addr}"),
                signer: TestSessionSigner::random("easynet:///r/realm/device/n1"),
                hub_ca_pem_path: None,
                dispatcher,
                escalation_outbox: None,
                preludes: SessionPreludeInputs::new(&ability_descriptors, authority_store()),
                liveness_timeout: Duration::from_secs(1),
                initial_admission: None,
                user_trust_sync: None, // not exercised here
                connection_state_sink: isolated_connection_state_sink(),
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
