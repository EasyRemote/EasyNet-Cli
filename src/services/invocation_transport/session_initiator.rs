// EasyNet CLI — invocation_transport — <self>.session initiator (device side)
// ====================================================================
//
// File: src/services/invocation_transport/session_initiator.rs
// Description: Device-side caller for `<self>.session`. At daemon
//              boot a device opens one long-lived `InvokeBidi`
//              stream against its configured hub, sends frame 0 =
//              `EnvelopeOpen` carrying the caller URA, then keeps
//              the stream open for the lifetime of the daemon —
//              this is the canonical reverse channel through which
//              the hub pushes `<self>.invoke_remote` and
//              `federation.forward_invoke` frames back to the
//              device.
//
// Where this fits in RFC-003
// --------------------------
// PR-1 lands the daemon-side InvocationServer.
// PR-2 (this commit) lands two halves of `<self>.session`:
//
//   commit 1/N  — hub-side acceptor: the `<self>.session` arm of
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
//   Bounded jitter, capped maximum, never gives up.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use easynet_axon::invocation::axiom::{
    canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
    InvocationEnvelope, SubjectIdentity as AxiomSubjectIdentity, UraProfile,
};
use ed25519_dalek::{Signer as _, SigningKey};
use futures::Stream;
use futures::StreamExt as _;
use rand::RngCore as _;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};
use tonic::Status;

use crate::runtime::ability_descriptor::AbilityDescriptor;

use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use easynet_axon::pb::axon::v1::{
    AgentIdentity, BidiControl, BinaryChunk, CallerSignature, Envelope, EnvelopeOpen,
    InvocationTarget, InvokeBidiDown, InvokeBidiUp, StreamDescriptor, SubjectIdentity,
};

/// Daemon-side ability name this initiator targets. The hub's
/// `InvokeBidi` dispatcher routes on
/// `EnvelopeOpen.target.ability_name`.
///
/// **Wire-pinned** — the production hub at `easynet.run` only
/// accepts the `<self>.session` literal today. The M4
/// canonical rename to `device.session` is held until EasyNet-Axon
/// (the hub-side gRPC dispatcher) ships matching dual-name
/// acceptance. EasyNet-Cli's M1 dual-aliasing answers both names
/// inbound, so the rename is safe on the device side; the
/// blocker is the hub's bidi dispatch table.
///
/// See `docs/open-questions/deprecate-self-alias-in-ability-names.md`
/// Stage 2 / Stage 4. RFC-001 v4.1.6 is the carrier window for the
/// wire-break.
// TODO(RFC-001-v4.1.6 stage-2): rename to `device.session` once the
// hub ships dual-name acceptance. Single grep anchor for all
// wire-pinned `<self>.*` constants.
pub const ABILITY_SELF_SESSION: &str = "<self>.session";

/// Stream id used by every BinaryChunk on the session bidi. PR-2
/// sub-spec §2.1 (and the wider RFC-003 transport plane) declares
/// one StreamDescriptor (id=0, content_type="application/json",
/// ordering=STRICT). Multiple streams on the same bidi are
/// reserved for future RFCs and not used by `<self>.session`.
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
#[derive(Clone)]
pub(crate) struct InitialSessionAdmissionProbe {
    tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<(), String>>>>>,
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
/// supervisor keeps retrying at this period with jitter. 30 s
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
/// `<self>.session` up-stream.
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

/// Default URA profile used when the session frame carries a signed
/// envelope. Empty profile fields canonicalise to the same value, but
/// populating the string keeps the wire explicit and easier to inspect.
const DEFAULT_URA_PROFILE: &str = "easynet-strict-v2";

/// Optional deterministic Ed25519 seed used to sign frame 0.
pub type SessionSigningSeed = [u8; 32];

/// What a device does with each `InvokeBidiDown` frame the hub
/// pushes to it: either translate the inner payload into a local
/// invocation and write the result back, or honour a control
/// frame. The trait surface intentionally looks like a single
/// `handle_down` because the bidi is duplex — the implementation
/// drives whatever response shape it needs through the supplied
/// `outbound` sender.
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
/// up-direction sequencing within one `<self>.session`.
#[derive(Clone, Debug)]
pub struct SessionUpSender {
    tx: mpsc::Sender<InvokeBidiUp>,
    next_sequence: Arc<AtomicU64>,
}

impl SessionUpSender {
    #[must_use]
    pub fn new(tx: mpsc::Sender<InvokeBidiUp>) -> Self {
        Self {
            tx,
            // Frame 0 is EnvelopeOpen. First post-frame-0 producer
            // therefore owns sequence = 1.
            next_sequence: Arc::new(AtomicU64::new(1)),
        }
    }

    fn allocate_sequence(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a BinaryChunk on the live session, stamping the next
    /// monotonic up-direction sequence number.
    pub async fn send_binary_chunk(
        &self,
        chunk: BinaryChunk,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<InvokeBidiUp>> {
        self.tx
            .send(InvokeBidiUp {
                sequence: self.allocate_sequence(),
                payload: Some(UpPayload::BinaryChunk(chunk)),
                ..InvokeBidiUp::default()
            })
            .await
    }

    /// Send a control frame on the live session, stamping the next
    /// monotonic up-direction sequence number.
    pub async fn send_control(
        &self,
        control: BidiControl,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<InvokeBidiUp>> {
        self.tx
            .send(InvokeBidiUp {
                sequence: self.allocate_sequence(),
                payload: Some(UpPayload::Control(control)),
                ..InvokeBidiUp::default()
            })
            .await
    }
}

/// Error from a single down-frame dispatch. Reported by the
/// dispatcher; the supervisor logs and continues.
#[derive(Debug, thiserror::Error)]
pub enum SessionDispatchError {
    #[error("session frame dispatch failed: {0}")]
    Other(String),
}

struct SessionUpHeartbeatTask {
    handle: tokio::task::JoinHandle<()>,
}

impl SessionUpHeartbeatTask {
    fn spawn(sender: SessionUpSender, hub_endpoint: String, caller_ura: String) -> Self {
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(SESSION_UP_HEARTBEAT_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // First tick fires immediately; consume it so the first
            // keepalive is sent after one full heartbeat window.
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = sender.send_control(BidiControl::default()).await {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = session,
                        kind = up_heartbeat_send_failed,
                        caller_ura = caller_ura,
                        hub_endpoint = hub_endpoint,
                        error = err_msg,
                        message = "stopping heartbeat task",
                    );
                    break;
                }
            }
        });
        Self { handle }
    }
}

impl Drop for SessionUpHeartbeatTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Run one `<self>.session` bidi against `hub_endpoint`. Connects,
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
    ability_descriptors: &[AbilityDescriptor],
) -> Result<(), SessionError> {
    dial_and_run_session_with_idle_timeout(
        hub_endpoint,
        caller_ura,
        signing_seed,
        hub_ca_pem_path,
        dispatcher,
        escalation_outbox,
        ability_descriptors,
        SESSION_IDLE_TIMEOUT,
        None,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn dial_and_run_session_with_idle_timeout<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_ura: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_ca_pem_path: Option<&Path>,
    dispatcher: Arc<D>,
    escalation_outbox: Option<
        &crate::services::invocation_transport::session_escalation::SharedSessionOutbox,
    >,
    ability_descriptors: &[AbilityDescriptor],
    idle_timeout: Duration,
    initial_admission: Option<InitialSessionAdmissionProbe>,
    user_trust_sync: Option<&UserTrustSync>,
) -> Result<(), SessionError> {
    warm_device_credential_for_session(&caller_ura).await;

    let mut endpoint = Endpoint::from_shared(hub_endpoint.clone())
        .map_err(|err| SessionError::InvalidEndpoint {
            endpoint: hub_endpoint.clone(),
            source: err,
        })?
        // No timeout on the bidi itself — the stream is intended
        // to live forever. Connect timeout caps the dial step.
        .connect_timeout(Duration::from_secs(10))
        // Production-WAN h2 hardening: HTTP/2 keep-alive PINGs every
        // 5s with 10s timeout. Without this, intermediate NAT /
        // hosting LB / corporate firewall can silently close idle
        // long-lived bidi streams, surfacing as "h2 protocol error:
        // error reading a body from connection" on the server side
        // and "target_offline" / dropped Dispatch frames at the
        // application layer. 5s is conservative-aggressive: stays
        // well under any NAT idle window (~60s typical) and surfaces
        // dead streams in ~15s rather than minutes. Cost is ~24
        // bytes/ping × 12/min ≈ negligible. tcp_keepalive is OS-
        // level and complements the h2 PING.
        .http2_keep_alive_interval(Duration::from_secs(5))
        .keep_alive_timeout(Duration::from_secs(10))
        .keep_alive_while_idle(true)
        .tcp_keepalive(Some(Duration::from_secs(15)));

    if let Some(ca_path) = hub_ca_pem_path {
        // Shared with `federation_client::cross_hub_dial::resolve_
        // peer_channel` via `federation_client::peer_dial::pinned_
        // tls_config` so both outbound dial sites have one audited
        // PEM-read + Certificate::from_pem + ClientTlsConfig path.
        // The pure-function helper returns a typed `PinnedTlsError`
        // that we wrap into the existing `SessionError::TlsCaRead`
        // variant — supervisor log formatting and downstream tests
        // stay byte-identical.
        let tls = crate::services::federation_client::pinned_tls_config(ca_path).map_err(
            |err| match err {
                crate::services::federation_client::PinnedTlsError::ReadFailed { path, source } => {
                    SessionError::TlsCaRead { path, source }
                }
            },
        )?;
        endpoint = endpoint
            .tls_config(tls)
            .map_err(|err| SessionError::TlsConfig {
                endpoint: hub_endpoint.clone(),
                source: err,
            })?;
    } else if hub_endpoint.starts_with("https://") {
        // No pinned CA + scheme is `https://` → caller wants TLS via
        // OS native trust roots (Let's Encrypt et al.). Tonic 0.12's
        // `Endpoint` has no default TLS config, so a `connect()` on
        // an `https://` URL without an explicit `tls_config()` call
        // fails with an opaque "transport error". `tls-roots` feature
        // on the `tonic` dep ships rustls-native-certs probing; we
        // hand it a `ClientTlsConfig::with_native_roots()` so the
        // probe runs and the connection succeeds against any
        // publicly-trusted CA. Domain validation uses the URL's
        // host automatically.
        let native_tls = tonic::transport::ClientTlsConfig::new().with_native_roots();
        endpoint = endpoint
            .tls_config(native_tls)
            .map_err(|err| SessionError::TlsConfig {
                endpoint: hub_endpoint.clone(),
                source: err,
            })?;
    }

    let channel: Channel = endpoint
        .connect()
        .await
        .map_err(|err| SessionError::ConnectFailed {
            endpoint: hub_endpoint.clone(),
            source: err,
        })?;
    // Cheap tonic Channel clone retained for the user-key re-sync
    // loop spawned after the preludes (its requests multiplex over
    // this same connection).
    let resync_channel = channel.clone();

    // Bump client-side gRPC message limits to match the server side.
    // The tonic-default 4 MiB decoder cap aborted `<self>.session`
    // mid-stream the moment a single down-frame envelope exceeded it.
    // The shared 64 MiB transport-envelope cap keeps legitimate
    // chunked traffic flowing without permitting near-unbounded
    // single-message allocations.
    let mut client = InvocationClient::new(channel)
        .max_decoding_message_size(
            crate::services::invocation_transport::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        )
        .max_encoding_message_size(
            crate::services::invocation_transport::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        );

    // Membership prelude (URA v4.1.4 dev unblock): axon-runtime hub
    // returns `AXON_MEMBERSHIP_REQUIRED` for any caller whose URA is
    // not in its membership table. The genesis exception is
    // `federation.join`, which the runtime accepts unsigned. We send
    // one before opening `<self>.session` so a fresh axon-runtime
    // (no audit-wal replay, e.g. `--reset-db` dev boot) does not
    // reject the session bidi for the lifetime of the daemon.
    //
    // The call is best-effort: we ignore both "already member" and
    // transport errors here. The session attempt below is the
    // authoritative health gate — if join didn't take, the bidi
    // still surfaces the underlying error and the supervisor's
    // reconnect loop retries everything.
    crate::op_event!(
        component = session,
        kind = federation_join_prelude_sending,
        caller_ura = caller_ura,
        hub_endpoint = hub_endpoint,
    );
    match send_federation_join_prelude(&mut client, &caller_ura).await {
        Ok(()) => {
            crate::op_event!(
                component = session,
                kind = federation_join_prelude_ok,
                message = "proceeding to <self>.session",
            );
        }
        Err(err) => {
            // `tonic::Code` has both Display and Debug; Display renders
            // the PascalCase variant name without surrounding quotes,
            // which is what SRE pipelines grep on. `err.message()` is
            // a `&str` — pass it through op_event!'s formatter so any
            // embedded whitespace gets one (and only one) layer of
            // quoting at the boundary, instead of pre-Debug-quoting it
            // here and letting the macro re-quote on top.
            let code = err.code();
            let msg = err.message();
            crate::op_event!(
                component = session,
                kind = federation_join_prelude_soft_failed,
                code = code,
                error = msg,
                message = "proceeding to <self>.session — bidi will surface the error if join was required",
            );
        }
    }

    // Owner-projection prelude (AXON-RFC-005): publish the daemon's
    // device-profile descriptors as a bounded owner projection
    // through `federation.advertise_abilities`. The hub stores the
    // projection as a read model and projects summaries back through
    // `federation.resolve(include_abilities=true)`.
    //
    // Hard gate: a device without an owner projection is not
    // namespace-visible under RFC-005. Continuing would leave the
    // owner online while every product ability resolves NODATA,
    // which is worse than a reconnectable projection failure.
    if !ability_descriptors.is_empty() {
        let ability_count = ability_descriptors.len();
        crate::op_event!(
            component = session,
            kind = advertise_abilities_prelude_sending,
            ability_count = ability_count,
        );
        if let Err(status) = send_advertise_abilities_prelude(
            &mut client,
            &caller_ura,
            &caller_ura,
            &caller_ura,
            ability_descriptors,
        )
        .await
        {
            let code = status.code();
            let msg = status.message();
            crate::op_event!(
                component = session,
                kind = advertise_abilities_prelude_failed,
                code = code,
                error = msg,
                message = "owner projection publish failed; reconnecting instead of exposing an online owner with empty abilities",
            );
            return Err(SessionError::OwnerProjectionFailed {
                endpoint: hub_endpoint,
                status,
            });
        } else {
            crate::op_event!(
                component = session,
                kind = advertise_abilities_prelude_ok,
                ability_count = ability_count,
            );
        }
    }

    // DEC-EU user-key sync prelude: import the paired user's signing
    // key from the hub registrar into the local trust anchor so the
    // admission gate can verify user-signed envelopes arriving via
    // invoke_remote user-caller pass-through. Advisory — the helper
    // logs its own failures and never blocks the session.
    //
    // Plus a session-lifetime refresh loop: keys registered at the
    // hub AFTER this dial (a new browser, a key rotation) become
    // admissible without waiting for a session re-dial. The loop's
    // requests multiplex over this session's channel; the drop-guard
    // aborts it when the session ends, and the next dial starts a
    // fresh one. Cost: one resolve_key unary per interval.
    let _user_trust_resync_guard = if let Some(sync) = user_trust_sync {
        sync_paired_user_trust_prelude(&mut client, &caller_ura, sync).await;
        let sync = sync.clone();
        let resync_caller = caller_ura.clone();
        Some(AbortOnDrop(tokio::spawn(async move {
            let mut resync_client =
                easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(
                    resync_channel,
                );
            loop {
                tokio::time::sleep(USER_TRUST_RESYNC_INTERVAL).await;
                sync_paired_user_trust_prelude(&mut resync_client, &resync_caller, &sync).await;
            }
        })))
    } else {
        None
    };

    // Hosted-agent advertise prelude (RFC-006-B v0.6 §URL +
    // RFC-006-C v0.1 §INV-2). RFC-005 namespace.resolve consumes
    // `AdvertisedAgentStore` when an owner is an agent URA
    // `agent/<u>.<a>` and selects a hosted-agent route through
    // the advertising device. Without these advertise calls the
    // resolver has owner projection but no executable placement.
    //
    // Owner segments derived from the local ability catalog:
    // every ability whose tail is `<owner>.<rest>` implies the
    // daemon hosts agent `<owner>` (skip hub-rooted `01HUB.*`
    // and the placeholder `<self>.*` shapes). Each unique
    // `<owner>` is advertised once as
    // `agent/<owner>.<owner>` HostedBy <caller_ura>; the user-
    // segment of the agent URA matches the daemon's owner
    // convention (EASYNET_PAGES_USER for pages, agent_name
    // for chat-base).
    let realm = crate::ura::parse_ura(&caller_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_default();
    // user_segment resolution order (most authoritative first):
    //   1. EASYNET_PAGES_USER env — explicit operator override,
    //      used in the docker e2e harness + multi-user dev rigs.
    //   2. credentials.json `username` — set by `easynet device join`
    //      after the backend resolves the pairing token to a user
    //      account. This is the production path on silan's Mac.
    // Empty means "no joined user identity available"; the
    // hosted-agent advertise prelude is a no-op in that case
    // (caller is in a transitional state and will republish on
    // reconnect once credentials are present).
    let user_segment = std::env::var("EASYNET_PAGES_USER")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            crate::persistence::config::load_credentials()
                .ok()
                .and_then(|c| c.username)
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_default();
    // Collect every hosted agent URA the daemon should
    // advertise. Two sources, in priority order:
    //
    //   1. `local-agents.json` (authoritative). Each row
    //      already carries the canonical agent_ura minted at
    //      `easynet agent add` time (post-RFC-001 v4.1.7 the
    //      mint is `<profile>-<name>` so the URA tail is
    //      operator-meaningful: `consent-default-0`,
    //      `llm-claude-1`, …). We advertise the URA verbatim;
    //      no string reconstruction.
    //
    //   2. Synthetic `pages` / `files` user-scoped agents
    //      (RFC-006-B + RFC-006-C). These are not stored in
    //      local-agents.json — the page/file servers register
    //      under `<user>.{pages,files}.*` ability names at
    //      runtime. We mint the URA in-line and advertise.
    #[derive(Debug, Clone)]
    struct AdvertiseEntry {
        agent_ura: String,
        /// The agent_ura's tail (`<user>.<agent_id>` after
        /// `agent/`), used purely for log lines + the
        /// pages/files user-scoped marker check.
        short_label: String,
        /// Local `agents.json` key for user-installed LLM agents.
        /// Synthetic pages/files and non-LLM profile agents do not
        /// own user-agent ability manifests, so they leave this unset.
        hosted_agent_name: Option<String>,
    }

    let mut entries: Vec<AdvertiseEntry> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Source 1: local-agents.json hosted_agents.
    let local_agents_file = crate::persistence::local_agents::load().unwrap_or_default();
    for hosted in &local_agents_file.hosted_agents {
        if hosted.agent_ura.is_empty() {
            continue;
        }
        // Reject pre-join placeholder URAs (`<unjoined>`).
        // Bootstrap repairs these on the next pass once the
        // realm + user_id land in credentials; advertising
        // them now would just push junk into the directory.
        if hosted.agent_ura.contains("<unjoined>") {
            continue;
        }
        if !seen.insert(hosted.agent_ura.clone()) {
            continue;
        }
        // Derive short_label from the URA tail for log lines.
        let short_label = crate::ura::parse_ura(&hosted.agent_ura)
            .ok()
            .filter(|p| p.kind == crate::ura::URAKind::Agent)
            .and_then(|p| {
                p.agent_ids()
                    .map(|(user_id, agent_id)| format!("{user_id}.{agent_id}"))
            })
            .unwrap_or_else(|| hosted.agent_ura.clone());
        entries.push(AdvertiseEntry {
            agent_ura: hosted.agent_ura.clone(),
            short_label,
            hosted_agent_name: (hosted.profile == "llm").then(|| hosted.name.clone()),
        });
    }

    // Source 2: synthetic pages + files. These don't appear
    // in local-agents.json (no `agent add` step mints them)
    // but the page-server / file-server handlers register
    // ability families under `<user>.{pages,files}.*` at
    // boot, so the AdvertisedAgentStore needs entries for
    // them or RFC-006-B `agent/<user>.pages` URLs route to
    // `target_offline`.
    if !realm.is_empty() && !user_segment.is_empty() && user_segment != "self" {
        for synthetic in ["pages", "files"] {
            let ura = crate::ura::agent_ura(&realm, &user_segment, synthetic);
            if seen.insert(ura.clone()) {
                entries.push(AdvertiseEntry {
                    agent_ura: ura,
                    short_label: format!("{user_segment}.{synthetic}"),
                    hosted_agent_name: None,
                });
            }
        }
    }

    if !realm.is_empty() && !entries.is_empty() {
        // Extract this device's node_id from the caller URA
        // (`easynet:///r/<realm>/device/<node_id>`) so we can
        // tell the hub which physical host serves each
        // advertised agent. Without it, `/api/v1/agents` falls
        // back to `<user>.<agent>` for `node_id`, and the
        // Frontend DeviceDetailPage's `agent.node_id ===
        // device.node_id` filter excludes the agent from the
        // device's hosted-agent list — so files / pages /
        // dynamically-added LLM agents would silently vanish
        // from the device view.
        let caller_node_id = crate::ura::parse_ura(&caller_ura)
            .ok()
            .filter(|p| p.kind == crate::ura::URAKind::Device)
            .and_then(|p| p.device_id().map(str::to_string));
        let entries_count = entries.len();
        let labels_display = format!(
            "{:?}",
            entries.iter().map(|e| &e.short_label).collect::<Vec<_>>()
        );
        crate::op_event!(
            component = session,
            kind = advertise_agent_prelude_sending,
            agent_count = entries_count,
            user = user_segment,
            labels = labels_display,
        );
        // user-scoped synthetic agents: pages + files exist
        // per-user, not per-device. Every device the user owns
        // serves them, and the user-content (published web
        // projects, uploaded blobs) is logically owned by the
        // user, not by any one host. Advertising them with a
        // concrete `host_node_id` makes `/api/v1/agents`
        // last-writer-wins — whichever device happened to
        // advertise most recently captures the directory
        // record, and the Frontend DeviceDetailPage filter
        // (`agent.node_id === device.node_id`) shows them on
        // an arbitrary device while hiding them from every
        // other one. We advertise them with no host_node_id
        // instead; backend `/api/v1/agents` falls back to the
        // `<user>.<agent>` string sentinel, the Frontend reads
        // that as "not bound to a specific device" and lists
        // them at the user level. forward_invoke against
        // `agent/<user>.{pages,files}` still resolves correctly
        // because the hub keeps the agent_ura → host_ura
        // mapping in `AdvertisedAgentStore`, independent of
        // the directory's `host_node_id`.
        //
        // Detection rule: the synthetic markers are agent_id
        // == "pages" / "files" exactly (no profile prefix).
        // Friendly-minted hosted agents have prefixed ids
        // (`consent-default-0`, `llm-pages` would never collide
        // with the synthetic `pages`).
        const USER_SCOPED_AGENT_IDS: &[&str] = &["pages", "files"];
        let agent_registry = crate::registry::agents::load_agents().unwrap_or_default();
        let live_registry = crate::runtime::agents::build_registry();
        for entry in &entries {
            // Decide whether this entry is the user-scoped
            // pages/files synthetic. Read the agent_id off
            // the URA so renames in the synthesis source
            // don't drift away from this check.
            let agent_id = crate::ura::parse_ura(&entry.agent_ura)
                .ok()
                .filter(|p| p.kind == crate::ura::URAKind::Agent)
                .and_then(|p| p.agent_ids().map(|(_, agent_id)| agent_id.to_string()))
                .unwrap_or_default();
            let host_for_advertise = if USER_SCOPED_AGENT_IDS.contains(&agent_id.as_str()) {
                None
            } else {
                caller_node_id.as_deref()
            };
            let advertise_agent_result = send_advertise_agent_prelude(
                &mut client,
                &caller_ura,
                &entry.agent_ura,
                host_for_advertise,
            )
            .await;
            match advertise_agent_result {
                Ok(()) => {
                    let descriptors = match entry.hosted_agent_name.as_deref() {
                        Some(agent_name) => {
                            let Some(agent_config) = agent_registry.agents.get(agent_name) else {
                                continue;
                            };
                            build_hosted_agent_ability_descriptors(
                                &entry.agent_ura,
                                agent_name,
                                agent_config,
                                caller_node_id.as_deref(),
                                &live_registry,
                            )
                        }
                        // Synthetic user-scoped `pages` agent carries no
                        // hosted_agent_name and is absent from the agent
                        // registry + the throwaway live registry; synthesize
                        // its fixed ability set so pages.* resolves (else the
                        // backend's namespace.resolve(pages.list) → NODATA).
                        None if agent_id == "pages" => {
                            build_synthetic_pages_ability_descriptors(&entry.agent_ura)
                        }
                        None => continue,
                    };
                    if descriptors.is_empty() {
                        continue;
                    }
                    let ability_count = descriptors.len();
                    crate::op_event!(
                        component = session,
                        kind = advertise_hosted_agent_abilities_prelude_sending,
                        agent_ura = entry.agent_ura,
                        ability_count = ability_count,
                    );
                    if let Err(err) = send_advertise_abilities_prelude(
                        &mut client,
                        &caller_ura,
                        &entry.agent_ura,
                        &caller_ura,
                        &descriptors,
                    )
                    .await
                    {
                        let code = err.code();
                        let msg = err.message();
                        crate::op_event!(
                            component = session,
                            kind = advertise_hosted_agent_abilities_prelude_soft_failed,
                            agent_ura = entry.agent_ura,
                            code = code,
                            error = msg,
                        );
                    } else {
                        crate::op_event!(
                            component = session,
                            kind = advertise_hosted_agent_abilities_prelude_ok,
                            agent_ura = entry.agent_ura,
                            ability_count = ability_count,
                        );
                    }
                }
                Err(err) => {
                    let agent_ura = entry.agent_ura.clone();
                    let code = err.code();
                    let msg = err.message();
                    crate::op_event!(
                        component = session,
                        kind = advertise_agent_prelude_soft_failed,
                        agent_ura = agent_ura,
                        code = code,
                        error = msg,
                    );
                }
            }
        }
        let entries_done_count = entries.len();
        crate::op_event!(
            component = session,
            kind = advertise_agent_prelude_done,
            agent_count = entries_done_count,
        );
    }

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(SESSION_UP_CHANNEL_CAPACITY);
    let outbound_tx = SessionUpSender::new(up_tx.clone());

    // Frame 0: EnvelopeOpen carrying caller URA + ability name
    // `<self>.session`. When boot resolved a deterministic device
    // seed from credentials we sign the canonical bytes here; older
    // sparse fixtures still degrade to the unsigned PR-2 shape.
    let frame0 = build_session_envelope_open_with_seed(&caller_ura, signing_seed);
    up_tx
        .send(frame0)
        .await
        .map_err(|_| SessionError::SendFailed("frame 0 EnvelopeOpen"))?;

    let outbound = ReceiverStream::new(up_rx);
    let response =
        client
            .invoke_bidi(outbound)
            .await
            .map_err(|status| SessionError::HubRejected {
                endpoint: hub_endpoint.clone(),
                status,
            })?;

    let mut down_stream = response.into_inner();
    let dispatcher = dispatcher;
    crate::op_event!(
        component = session,
        kind = bidi_opened,
        hub_endpoint = hub_endpoint,
        caller_ura = caller_ura,
        message = "awaiting down-stream frames",
    );
    if let Some(probe) = &initial_admission {
        probe.admitted();
    }

    // PR-N6 C4: publish the active up sender so the device-mode
    // escalation consumer task can push `SessionDispatch::Request`
    // frames onto this same bidi. Cleared on every exit path
    // below so the consumer's next snapshot reads `None` until the
    // supervisor's next successful dial.
    if let Some(outbox) = escalation_outbox {
        outbox.set(outbound_tx.clone());
    }
    let _outbox_guard = OutboxGuard::new(escalation_outbox.cloned());
    let _up_heartbeat = SessionUpHeartbeatTask::spawn(
        outbound_tx.clone(),
        hub_endpoint.clone(),
        caller_ura.clone(),
    );
    let mut expected_down_sequence = 0_u64;

    loop {
        let frame_result = match tokio::time::timeout(idle_timeout, down_stream.next()).await {
            Ok(Some(frame_result)) => frame_result,
            Ok(None) => break,
            Err(_elapsed) => {
                return Err(SessionError::IdleTimeout {
                    endpoint: hub_endpoint,
                    timeout: idle_timeout,
                });
            }
        };

        match frame_result {
            Ok(frame) => {
                if frame.sequence != expected_down_sequence {
                    return Err(SessionError::DownStreamSequence {
                        endpoint: hub_endpoint,
                        expected: expected_down_sequence,
                        actual: frame.sequence,
                        reason: REASON_BIDI_DOWN_SEQUENCE,
                    });
                }
                expected_down_sequence = expected_down_sequence.saturating_add(1);
                if let Err(err) = dispatcher.handle_down(frame, &outbound_tx).await {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = session,
                        kind = frame_dispatch_error,
                        error = err_msg,
                        message = "continuing",
                    );
                }
            }
            Err(status) => {
                return Err(SessionError::DownStreamError {
                    endpoint: hub_endpoint,
                    status,
                });
            }
        }
    }

    // Hub closed the down stream cleanly. The supervisor will
    // reconnect.
    Ok(())
}

/// RAII guard that clears the escalation outbox on Drop, ensuring
/// every `dial_and_run_session` exit path (Ok return, error
/// return, panic during the down-stream loop) leaves the outbox
/// empty. The escalation consumer's next snapshot then surfaces
/// `UpstreamFailure { reason: "no live <self>.session bidi" }`
/// to in-flight escalations until the supervisor reconnects.
struct OutboxGuard {
    outbox: Option<crate::services::invocation_transport::session_escalation::SharedSessionOutbox>,
}

impl OutboxGuard {
    fn new(
        outbox: Option<
            crate::services::invocation_transport::session_escalation::SharedSessionOutbox,
        >,
    ) -> Self {
        Self { outbox }
    }
}

impl Drop for OutboxGuard {
    fn drop(&mut self) {
        if let Some(outbox) = &self.outbox {
            outbox.clear();
        }
    }
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_session_supervisor<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_ura: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_ca_pem_path: Option<PathBuf>,
    dispatcher: Arc<D>,
    escalation_outbox: Option<
        crate::services::invocation_transport::session_escalation::SharedSessionOutbox,
    >,
    ability_descriptors: Vec<AbilityDescriptor>,
    initial_admission: Option<InitialSessionAdmissionProbe>,
    user_trust_sync: Option<UserTrustSync>,
    mut cancel: tokio::sync::oneshot::Receiver<()>,
) {
    let mut backoff = SESSION_BACKOFF_INITIAL;
    loop {
        tokio::select! {
            _ = &mut cancel => {
                crate::op_event!(
                    component = session,
                    kind = supervisor_cancelled,
                );
                return;
            }
            result = dial_and_run_session_with_idle_timeout(
                hub_endpoint.clone(),
                caller_ura.clone(),
                signing_seed,
                hub_ca_pem_path.as_deref(),
                Arc::clone(&dispatcher),
                escalation_outbox.as_ref(),
                &ability_descriptors,
                SESSION_IDLE_TIMEOUT,
                initial_admission.clone(),
                user_trust_sync.as_ref(),
            ) => {
                match result {
                    Ok(()) => {
                        // Render Duration as integer milliseconds —
                        // `Duration` has no `Display` impl, and the
                        // Debug form (`250ms` / `1.5s`) mixes unit
                        // suffixes that complicate SRE arithmetic on
                        // the field. Milliseconds is the unit operators
                        // already see in `*_ms` fields elsewhere.
                        let next_backoff_ms =
                            SESSION_BACKOFF_INITIAL.as_millis() as u64;
                        crate::op_event!(
                            component = session,
                            kind = bidi_closed_cleanly,
                            next_backoff_ms = next_backoff_ms,
                        );
                        backoff = SESSION_BACKOFF_INITIAL;
                    }
                    Err(err) => {
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

                // Sleep, also cancellable.
                tokio::select! {
                    _ = &mut cancel => return,
                    _ = tokio::time::sleep(backoff) => {}
                }

                backoff = next_backoff(backoff);
            }
        }
    }
}

fn next_backoff(current: Duration) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > SESSION_BACKOFF_MAX {
        SESSION_BACKOFF_MAX
    } else {
        doubled
    }
}

/// Best-effort REST backstop before each `<self>.session` dial.
///
/// Hub restarts can lose the in-memory trust view before the device's
/// next gRPC reconnect. The backend already exposes
/// `/api/v1/devices/verify-credential` as the idempotent path that
/// replays `<self>.register_device_pubkey` into the hub daemon. Calling
/// it here keeps the reconnect loop self-healing: if the Hub forgot this
/// device, the trust entry is restored before `federation.join` and
/// `federation.advertise_abilities` run. Failures are advisory; the
/// subsequent gRPC prelude remains the authoritative session gate.
async fn warm_device_credential_for_session(caller_ura: &str) {
    let caller_ura = caller_ura.to_string();
    let outcome = tokio::task::spawn_blocking(move || verify_device_credential_once(&caller_ura))
        .await
        .unwrap_or_else(|err| CredentialWarmupOutcome::Failed {
            api_base: String::new(),
            reason: format!("credential warmup task join failed: {err}"),
        });

    match outcome {
        CredentialWarmupOutcome::Verified { api_base } => {
            crate::op_event!(
                component = session,
                kind = credential_verify_warmup_ok,
                api_base = api_base,
                message = "device credential verified before <self>.session dial",
            );
        }
        CredentialWarmupOutcome::Skipped { reason } => {
            crate::op_event!(
                component = session,
                kind = credential_verify_warmup_skipped,
                reason = reason,
            );
        }
        CredentialWarmupOutcome::Failed { api_base, reason } => {
            crate::op_event!(
                component = session,
                kind = credential_verify_warmup_failed,
                api_base = api_base,
                reason = reason,
                message =
                    "continuing to gRPC session prelude; Hub will return the authoritative status",
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CredentialWarmupOutcome {
    Verified { api_base: String },
    Skipped { reason: String },
    Failed { api_base: String, reason: String },
}

fn verify_device_credential_once(caller_ura: &str) -> CredentialWarmupOutcome {
    // Test isolation: the warmup runs unconditionally at the top of
    // dial_and_run_session_with_idle_timeout, so every dial test would
    // otherwise read the developer's real ~/.easynet/credentials.json and
    // fire a blocking 5s ureq POST that races other tests' loopback hubs
    // (flaky Elapsed / corrupt-header failures under the parallel suite).
    // Skip the live read+POST in test builds; the HTTP wire shape itself
    // is covered by credential_warmup_posts_current_device_credential,
    // which drives verify_device_credential_for_credentials directly.
    if cfg!(test) {
        return CredentialWarmupOutcome::Skipped {
            reason: "credential warmup skipped under cargo test".to_string(),
        };
    }
    let creds = match crate::persistence::config::load_credentials() {
        Ok(creds) => creds,
        Err(err) => {
            return CredentialWarmupOutcome::Skipped {
                reason: format!("credentials unavailable: {err}"),
            };
        }
    };
    verify_device_credential_for_credentials(caller_ura, creds)
}

fn verify_device_credential_for_credentials(
    caller_ura: &str,
    creds: crate::persistence::config::Credentials,
) -> CredentialWarmupOutcome {
    let expected_caller = crate::ura::device_ura(&creds.realm, &creds.node_id);
    if expected_caller != caller_ura {
        return CredentialWarmupOutcome::Skipped {
            reason: format!(
                "credentials caller {expected_caller} does not match session caller {caller_ura}"
            ),
        };
    }

    let api_base = creds.api_base();
    let url = format!("{api_base}/api/v1/devices/verify-credential");
    let response = ureq::post(&url)
        .timeout(Duration::from_secs(5))
        .send_json(serde_json::json!({
            "node_id": creds.node_id,
            "credential_token": creds.credential_token,
        }));

    match response {
        Ok(resp) if (200..300).contains(&resp.status()) => {
            CredentialWarmupOutcome::Verified { api_base }
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            CredentialWarmupOutcome::Failed {
                api_base,
                reason: format!("HTTP {status}: {body}"),
            }
        }
        Err(ureq::Error::Status(status, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            CredentialWarmupOutcome::Failed {
                api_base,
                reason: format!("HTTP {status}: {body}"),
            }
        }
        Err(err) => CredentialWarmupOutcome::Failed {
            api_base,
            reason: err.to_string(),
        },
    }
}

/// Send a one-shot `federation.join@1` over the same gRPC channel
/// the session bidi will open on. Genesis exception in axon-runtime
/// (`signature_policy=RequireSigned` allows this ability unsigned),
/// so the call uses an envelope with caller URA only — no signing
/// material — and a minimal JoinFederationRequest payload.
///
/// We treat both success and "already member" as positive outcomes;
/// any other status is logged by the caller and we continue. The
/// session bidi's HubRejected error is the authoritative gate
/// downstream — if join was needed but failed, the bidi surfaces
/// the right status and the supervisor backs off.
async fn send_federation_join_prelude(
    client: &mut easynet_axon::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_ura: &str,
) -> Result<(), tonic::Status> {
    // Wire shape mirrors `federation_wrappers::JoinRequest`:
    // axon-runtime's deserializer is strict (Deserialize derive,
    // no #[serde(default)] on either field) and rejects payloads
    // whose top-level keys differ from `membership_ura` /
    // `realm`. Sending `agent_ura` / `tenant_id` (the field names
    // used by the local daemon's other federation.* requests)
    // earns InvalidArgument — verified in PR-1 §5 schema-compat
    // tests and observed in dev as
    //   `failed to decode JSON arguments: missing field
    //    membership_ura`
    let realm = crate::ura::parse_ura(caller_ura)
        .map(|parsed| parsed.realm)
        .unwrap_or_default();

    let body = serde_json::json!({
        "membership_ura": caller_ura,
        "realm": realm,
    });
    let arguments = serde_json::to_vec(&body)
        .map_err(|e| tonic::Status::internal(format!("federation.join prelude serialize: {e}")))?;

    let request = crate::services::invocation_transport::ProtoEnvelope::caller_only(caller_ura)
        .and_then(|env| env.invoke_request("federation.join", arguments))
        .map_err(|e| tonic::Status::invalid_argument(format!("federation.join prelude: {e}")))?;

    match client.invoke(request).await {
        Ok(reply) => {
            // AXON-RFC-001 v4.1.7 hub-broadcast contract: parse
            // the receipt body so the device seeds its
            // HubPublishedAbilityStore with whatever the hub
            // currently advertises. Failures here are
            // best-effort — a malformed body or absent fields
            // (talking to a v4.1.6 hub) leaves the store empty,
            // which is the correct transitional behavior.
            let body_bytes = reply.into_inner().result;
            if !body_bytes.is_empty() {
                if let Ok(body) = serde_json::from_slice::<
                    crate::runtime::federation_client::JoinReceipt,
                >(&body_bytes)
                {
                    let store = crate::services::hub_published_ability_store::global();
                    store.seed_from_snapshot(
                        body.hub_abilities_revision,
                        body.hub_published_abilities,
                    );
                    if !store.is_empty() {
                        let ability_count = store.len();
                        let hub_abilities_revision = body.hub_abilities_revision;
                        crate::op_event!(
                            component = session,
                            kind = hub_broadcast_abilities_seeded,
                            ability_count = ability_count,
                            hub_abilities_revision = hub_abilities_revision,
                        );
                    }
                }
            }
            Ok(())
        }
        // Already-a-member is a benign outcome; surface as success.
        Err(status)
            if status.code() == tonic::Code::AlreadyExists
                || status.message().contains("already") =>
        {
            Ok(())
        }
        Err(status) => Err(status),
    }
}

/// Send a one-shot `federation.advertise_abilities@1` over the same
/// gRPC channel the session bidi will open on.
///
/// The prelude publishes an RFC-005 owner projection. It does not
/// send raw `AbilityDescriptor` values to the hub; descriptors are
/// local input used only to derive bounded
/// `AbilityProjectionSummary` rows.
/// Session-prelude variant of `federation.advertise_agent`. The
/// device tells the hub "I host agent `<agent_ura>`"; the hub
/// upserts an `AdvertisedAgentRecord { agent_ura, host_ura }` so
/// later `namespace.resolve` calls can select a hosted-agent
/// next hop through this device.
async fn send_advertise_agent_prelude(
    client: &mut easynet_axon::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_ura: &str,
    agent_ura: &str,
    host_node_id: Option<&str>,
) -> Result<(), tonic::Status> {
    // `host_node_id` is the device's bare uuid (extracted from
    // the caller URA). The hub stores it on the directory record
    // so RFC-002 §5.2 forward_invoke knows which UDS-bound
    // local-tool registration to dispatch into, and so backend
    // `/api/v1/agents` populates `AgentInfo.node_id` correctly
    // — without which DeviceDetailPage's hosted-agent filter
    // silently drops the agent from the device view.
    let mut body = serde_json::json!({
        "agent_ura": agent_ura,
        "signing_authority": {
            "kind": "hosted_by",
            "host_ura": caller_ura,
        },
    });
    if let Some(node_id) = host_node_id {
        if let Some(map) = body.as_object_mut() {
            map.insert(
                "host_node_id".to_string(),
                serde_json::Value::String(node_id.to_string()),
            );
        }
    }
    let arguments = serde_json::to_vec(&body).map_err(|e| {
        tonic::Status::internal(format!("federation.advertise_agent prelude serialize: {e}"))
    })?;

    let request = crate::services::invocation_transport::ProtoEnvelope::caller_only(caller_ura)
        .and_then(|env| env.invoke_request("federation.advertise_agent", arguments))
        .map_err(|e| {
            tonic::Status::invalid_argument(format!("federation.advertise_agent prelude: {e}"))
        })?;

    invoke_prelude_unary(client, request, "federation.advertise_agent")
        .await
        .map(|_| ())
}

async fn send_advertise_abilities_prelude(
    client: &mut easynet_axon::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_ura: &str,
    owner_ura: &str,
    host_device_ura: &str,
    descriptors: &[AbilityDescriptor],
) -> Result<(), tonic::Status> {
    let projection = crate::runtime::owner_projection::prepare_and_persist(
        owner_ura,
        host_device_ura,
        descriptors,
    )
    .map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_abilities prelude projection: {e}"
        ))
    })?;

    let body = serde_json::json!({
        "owner_ura": projection.owner_ura,
        "host_device_ura": projection.host_device_ura,
        "projection_revision": projection.projection_revision,
        "projection_digest": projection.projection_digest,
        "lease_expires_unix_ms": projection.lease_expires_unix_ms,
        "ability_summaries": projection.ability_summaries,
    });
    let arguments = serde_json::to_vec(&body).map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_abilities prelude serialize: {e}"
        ))
    })?;

    let request = crate::services::invocation_transport::ProtoEnvelope::caller_only(caller_ura)
        .and_then(|env| env.invoke_request("federation.advertise_abilities", arguments))
        .map_err(|e| {
            tonic::Status::invalid_argument(format!("federation.advertise_abilities prelude: {e}"))
        })?;

    invoke_prelude_unary(client, request, "federation.advertise_abilities")
        .await
        .map(|_| ())
}

/// Handles the daemon needs to import its paired user's signing key
/// into the local realm trust anchor at session-establish time.
///
/// DEC-EU user-as-first-class-caller + invoke_remote user-caller
/// pass-through: the device-side admission gate verifies the user's
/// envelope signature against the LOCAL trust anchor only (INV-1:
/// same-realm local miss is final — no federation fall-through). But
/// user signing keys are registered at the realm's hub, the identity
/// registrar, by the backend. This sync closes that gap with the
/// correct authority direction: the device PULLS its own paired
/// user's key over the session channel it already authenticated
/// (pinned hub TLS CA + hub trust row), then writes the row through
/// the same `register_device_pubkey` write policy the gRPC surface
/// uses. Admission stays local-anchor-authoritative; the anchor is
/// just kept warm.
#[derive(Clone)]
pub struct UserTrustSync {
    pub daemon_realm: String,
    pub trust_anchor_path: PathBuf,
    pub cell: crate::services::trust_anchor_cell::SharedTrustAnchor,
}

/// Cadence of the session-lifetime user-key refresh loop. One
/// resolve_key unary per tick; 60s bounds the "new browser key
/// registered but not yet admissible at the device" window without
/// meaningful load.
const USER_TRUST_RESYNC_INTERVAL: Duration = Duration::from_secs(60);

/// Aborts the wrapped task when dropped — ties a background loop's
/// lifetime to the owning scope (here: one dialed session).
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Pull the paired user's pubkey from the hub and import it into the
/// local trust anchor. Advisory: every failure path logs and returns —
/// a device whose user key cannot be synced must still come online
/// (abilities that don't need user-signed admission keep working; the
/// next session dial retries the sync).
async fn sync_paired_user_trust_prelude(
    client: &mut easynet_axon::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_ura: &str,
    sync: &UserTrustSync,
) {
    let Ok(creds) = crate::persistence::config::load_credentials() else {
        return; // unpaired device — nothing to sync
    };
    let Some(username) = creds
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let realm = creds.realm.trim();
    if realm != sync.daemon_realm {
        // Roaming device anchored at a foreign realm: user keys stay
        // home-realm-authoritative (register_device_pubkey's cross-
        // realm rule); admission for such callers is a DEC-EU
        // §multi-realm follow-up, not this sync's job.
        return;
    }
    let user_ura = crate::ura::user_ura(realm, username);

    let args = match serde_json::to_vec(&serde_json::json!({ "agent_ura": user_ura })) {
        Ok(v) => v,
        Err(_) => return,
    };
    let request = match crate::services::invocation_transport::ProtoEnvelope::caller_only(
        caller_ura,
    )
    .and_then(|env| {
        env.invoke_request(
            crate::services::invocation_transport::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
            args,
        )
    }) {
        Ok(req) => req,
        Err(_) => return,
    };
    let response = match invoke_prelude_unary(client, request, "federation.resolve_key").await {
        Ok(resp) => resp,
        Err(status) => {
            let code = status.code();
            let msg = status.message();
            crate::op_event!(
                component = session,
                kind = user_trust_sync_resolve_failed,
                code = code,
                error = msg,
                user_ura = user_ura,
            );
            return;
        }
    };
    // DEC-EU multi-device: the hub returns every key registered
    // under the user URA (`public_keys_b64`); older hubs only emit
    // the single `public_key_b64`. Import ALL of them — the browser
    // signing the next invoke may hold any one of the user's keys,
    // and admission verifies against the local anchor's full set.
    let parsed = serde_json::from_slice::<serde_json::Value>(&response.result).ok();
    let mut pubkeys: Vec<String> = parsed
        .as_ref()
        .and_then(|v| v.get("public_keys_b64"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if pubkeys.is_empty() {
        if let Some(pk) = parsed
            .as_ref()
            .and_then(|v| v.get("public_key_b64"))
            .and_then(|pk| pk.as_str())
        {
            pubkeys.push(pk.to_string());
        }
    }
    if pubkeys.is_empty() {
        crate::op_event!(
            component = session,
            kind = user_trust_sync_resolve_empty,
            user_ura = user_ura,
            message = "hub returned no user keys — user key not registered at hub yet",
        );
        return;
    }

    for pubkey_b64 in pubkeys {
        let register_args = match serde_json::to_vec(&serde_json::json!({
            "agent_ura": user_ura,
            "public_key_b64": pubkey_b64,
            "role": "user",
        })) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match crate::services::invocation_transport::register_device_pubkey::handle(
            &register_args,
            &sync.daemon_realm,
            &sync.trust_anchor_path,
            &sync.cell,
        ) {
            Ok(_) => {
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_ok,
                    user_ura = user_ura,
                );
            }
            Err(status) if status.code() == tonic::Code::AlreadyExists => {
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_already_present,
                    user_ura = user_ura,
                );
            }
            Err(status) => {
                let code = status.code();
                let msg = status.message();
                crate::op_event!(
                    component = session,
                    kind = user_trust_sync_write_failed,
                    code = code,
                    error = msg,
                    user_ura = user_ura,
                );
            }
        }
    }
}

async fn invoke_prelude_unary(
    client: &mut easynet_axon::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    request: easynet_axon::pb::axon::v1::InvokeRequest,
    ability_name: &str,
) -> Result<easynet_axon::pb::axon::v1::InvokeResponse, tonic::Status> {
    let response = client.invoke(request).await?.into_inner();
    if let Some(error) = response.error.as_ref() {
        let message = if error.code.is_empty() {
            error.message.clone()
        } else if error.message.is_empty() {
            error.code.clone()
        } else {
            format!("{}: {}", error.code, error.message)
        };
        return Err(tonic::Status::failed_precondition(format!(
            "{ability_name} prelude rejected: {message}"
        )));
    }
    Ok(response)
}

fn build_hosted_agent_ability_descriptors(
    owner_ura: &str,
    agent_name: &str,
    entry: &crate::registry::agents::AgentEntry,
    host_node_id: Option<&str>,
    live_registry: &crate::runtime::ability_dispatch::AxonAbilityCatalog,
) -> Vec<AbilityDescriptor> {
    let mut descriptors = Vec::new();
    for spec in crate::runtime::abilities::abilities_for_publication(agent_name, entry) {
        let registry_name = spec.name();
        let owner_local_name = crate::runtime::abilities::public_agent_ability_name(
            owner_ura,
            agent_name,
            registry_name,
        );
        let Ok(mut descriptor) = AbilityDescriptor::new(
            owner_local_name,
            owner_ura,
            crate::runtime::ability_descriptor::Visibility::Scoped,
        ) else {
            continue;
        };
        descriptor = descriptor
            .with_description(spec.description())
            .with_input_schema(spec.parameters().clone())
            .with_hints(crate::runtime::agents::discovery_hints_for(
                live_registry,
                registry_name,
            ))
            .with_source(format!("agent:{agent_name}"))
            .with_metadata_entry("runtime", entry.agent_type.to_string())
            .with_metadata_entry("agent_type", entry.agent_type.to_string())
            .with_metadata_entry("base_runtime", entry.agent_type.to_string());
        if let Some(node_id) = host_node_id {
            descriptor = descriptor.with_metadata_entry("host_node_id", node_id.to_string());
        }
        if let Some(model) = entry.model.as_ref() {
            descriptor = descriptor
                .with_metadata_entry("model", model.clone())
                .with_metadata_entry("base_model", model.clone());
        }
        descriptors.push(descriptor);
    }
    descriptors
}

/// Build advertise descriptors for the user-scoped synthetic `pages`
/// agent. Its `pages.*` abilities are registered in the daemon's LIVE
/// catalog under `OwnerKind::User(<user>)`, but that catalog is not
/// reachable from the session prelude — and neither `build_registry()`
/// nor `build_system_registry()` carries the user id — so the fixed,
/// deterministic relative ability set is synthesized here, the same way
/// the synthetic `["pages","files"]` agent entries themselves are minted.
///
/// Name match (RFC-005): the resolver matches the relative ability name
/// (`pages.list`) and the canonical ability URA
/// (`…/ability/<user>.pages.pages.list`). `descriptor.public_name()`
/// strips the owner's agent-id prefix (`pages.`), so the descriptor name
/// must be `pages.<relative>` (`pages.pages.list`) to project back to the
/// `pages.list` relative name the backend invokes. Using `pages.list`
/// directly would project to `list` and stay NODATA.
fn build_synthetic_pages_ability_descriptors(owner_ura: &str) -> Vec<AbilityDescriptor> {
    // Single source of truth with the local registration in
    // src/runtime/agents/pages/mod.rs, so the advertised descriptor
    // carries the same input schema (project_id / folder requirements)
    // the Frontend InvokeAbilityDialog needs — otherwise it shows
    // "No input required" and empty-arg invokes 400 with missing arg.
    crate::runtime::agents::pages::management_ability_specs()
        .into_iter()
        .filter_map(|spec| {
            // Descriptor name = `pages.<verb>` so public_name() (which
            // strips the owner `pages.` agent-id prefix) projects back
            // to the `<verb>`... wait: see the name-match note. The
            // resolver matches the relative name `pages.list`, so the
            // descriptor name must be `pages.pages.list`.
            let descriptor_name = format!("pages.{}", spec.relative_name);
            AbilityDescriptor::new(
                descriptor_name,
                owner_ura,
                crate::runtime::ability_descriptor::Visibility::Scoped,
            )
            .ok()
            .map(|descriptor| {
                descriptor
                    .with_description(spec.description)
                    .with_input_schema(spec.input_schema)
                    .with_source("synthetic:pages")
            })
        })
        .collect()
}

/// Build the EnvelopeOpen frame 0 a device sends to open
/// `<self>.session`. Public so PR-2 commit 1/N's hub-side
/// acceptor tests can construct a matching expected frame, and
/// so the integration test in PR-3 commit 3/3 can drive a mock
/// device through the same shape.
#[must_use]
pub fn build_session_envelope_open(caller_ura: &str) -> InvokeBidiUp {
    build_session_envelope_open_with_seed(caller_ura, None)
}

/// Build the frame-0 `EnvelopeOpen`, optionally signing it when a
/// deterministic device seed is available.
#[must_use]
pub fn build_session_envelope_open_with_seed(
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
) -> InvokeBidiUp {
    let initial_args = Vec::new();
    let args_digest: [u8; 32] = Sha256::digest(&initial_args).into();

    let mut envelope = Envelope {
        caller: Some(AgentIdentity {
            ura: caller_ura.to_string(),
            profile: DEFAULT_URA_PROFILE.to_string(),
        }),
        // `<self>.session` is the device presenting its own long-
        // lived reverse channel; callee + subject both point at the
        // caller device so the signed tuple is stable and self-
        // describing even before a future hub-URA contract lands.
        callee: Some(AgentIdentity {
            ura: caller_ura.to_string(),
            profile: DEFAULT_URA_PROFILE.to_string(),
        }),
        subject: Some(SubjectIdentity {
            ura: caller_ura.to_string(),
            profile: DEFAULT_URA_PROFILE.to_string(),
        }),
        ..Envelope::default()
    };

    let mut mac = Vec::new();
    if let Some(seed) = signing_seed {
        let mut nonce = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        envelope.invocation_nonce = nonce.to_vec();

        let axiom_env = InvocationEnvelope {
            caller: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
            callee: AxiomAgentIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
            subject: AxiomSubjectIdentity::new(caller_ura, UraProfile::EasynetStrictV2),
            ability: ABILITY_SELF_SESSION.to_string(),
            args_digest,
            invocation_nonce: nonce,
            causal_context: CausalContext::None,
        };
        let signing_key = SigningKey::from_bytes(&seed);
        let signature = signing_key.sign(&canonical_invocation_bytes(&axiom_env));
        mac = signature.to_bytes().to_vec();
        envelope.caller_signature = Some(CallerSignature {
            algorithm: "ed25519".to_string(),
            signature: mac.clone(),
            ..CallerSignature::default()
        });
    }

    InvokeBidiUp {
        sequence: 0,
        mac,
        payload: Some(UpPayload::EnvelopeOpen(EnvelopeOpen {
            envelope: Some(envelope),
            target: Some(InvocationTarget {
                ability_name: ABILITY_SELF_SESSION.to_string(),
                ..InvocationTarget::default()
            }),
            initial_args,
            streams: vec![StreamDescriptor {
                stream_id: SESSION_STREAM_ID,
                content_type: "application/json".to_string(),
                ..StreamDescriptor::default()
            }],
            ..EnvelopeOpen::default()
        })),
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

    #[error("hub `{endpoint}` rejected `<self>.session` bidi: {status}")]
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
    use futures::stream;
    use serde_json::Value;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response};

    /// Upper bound for "did the supervisor's async machinery make
    /// progress" assertions (exit-on-cancel, initial-admission report).
    /// This is NOT a product SLA — it only guards against a genuine hang.
    /// Kept generous because the full `cargo test --lib` run (3000+ tests)
    /// saturates the scheduler and an in-process loopback bidi handshake
    /// can take well over a second under that contention; a 2 s bound
    /// flaked here as `Elapsed`. The supervisor's own connect timeout is
    /// 10 s, so 10 s still fails fast on a real stall.
    const TEST_SUPERVISOR_PROGRESS_TIMEOUT: Duration = Duration::from_secs(10);

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
            ABILITY_SELF_SESSION,
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
    fn credential_warmup_skips_when_credentials_do_not_match_session_caller() {
        let creds = crate::persistence::config::Credentials {
            node_id: "n1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "http://127.0.0.1:1".to_string(),
            realm: "realm".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some("http://127.0.0.1:1".to_string()),
            username: Some("dev".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
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

    #[test]
    fn credential_warmup_posts_current_device_credential() {
        let listener = StdTcpListener::bind("127.0.0.1:0").expect("bind verify server");
        let addr = listener.local_addr().expect("verify server addr");
        let (tx, rx) = std_mpsc::channel::<String>();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept verify request");
            let mut buf = [0_u8; 4096];
            let n = stream.read(&mut buf).expect("read verify request");
            tx.send(String::from_utf8_lossy(&buf[..n]).to_string())
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
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
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
            &[],
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
                &[],
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
            format!("http://{addr}"),
            owner.to_string(),
            None,
            None,
            dispatcher,
            None,
            &descriptors,
            Duration::from_millis(80),
            None,
            None, // user_trust_sync: not exercised here
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
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
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
            format!("http://{addr}"),
            device_ura.to_string(),
            None,
            None,
            dispatcher,
            None,
            &descriptors,
            Duration::from_millis(80),
            None,
            None, // user_trust_sync: not exercised here
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
            &[],
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

        let supervisor_handle = tokio::spawn(run_session_supervisor(
            "http://127.0.0.1:1".to_string(),
            "easynet:///r/realm/device/n1".to_string(),
            None,
            None,
            dispatcher,
            None,       // PR-N6 C4: no escalation outbox in this test
            Vec::new(), // ability_descriptors: empty in tests
            None,
            None, // user_trust_sync: not exercised here
            cancel_rx,
        ));

        // Give the supervisor a beat to start its first dial then
        // cancel. The dial will fail (connect refused on port 1)
        // and the supervisor will be sleeping in backoff when the
        // cancel arrives.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = cancel_tx.send(());

        let exit_within_bound = tokio::time::timeout(TEST_SUPERVISOR_PROGRESS_TIMEOUT, supervisor_handle)
            .await
            .expect("supervisor exits promptly after cancel");
        exit_within_bound.expect("supervisor task did not panic");
    }

    #[tokio::test]
    async fn supervisor_reports_initial_admission_failure_before_backoff() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let (probe, admission_rx) = initial_session_admission_probe();

        let supervisor_handle = tokio::spawn(run_session_supervisor(
            "http://127.0.0.1:1".to_string(),
            "easynet:///r/realm/device/n1".to_string(),
            None,
            None,
            dispatcher,
            None,
            Vec::new(),
            Some(probe),
            None, // user_trust_sync: not exercised here
            cancel_rx,
        ));

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

        let supervisor_handle = tokio::spawn(run_session_supervisor(
            format!("http://{addr}"),
            "easynet:///r/realm/device/n1".to_string(),
            None,
            None,
            dispatcher,
            None,
            Vec::new(),
            Some(probe),
            None, // user_trust_sync: not exercised here
            cancel_rx,
        ));

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
            format!("http://{addr}"),
            "easynet:///r/realm/device/n1".to_string(),
            None,
            None,
            dispatcher,
            None,
            &[],
            Duration::from_millis(80),
            None,
            None, // user_trust_sync: not exercised here
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
            format!("http://{addr}"),
            "easynet:///r/realm/device/n1".to_string(),
            None,
            None,
            dispatcher,
            None,
            &[],
            Duration::from_secs(1),
            None,
            None, // user_trust_sync: not exercised here
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
