// EasyNet CLI — axon_serve — <self>.session initiator (device side)
// ====================================================================
//
// File: src/services/axon_serve/session_initiator.rs
// Description: Device-side caller for `<self>.session`. At daemon
//              boot a device opens one long-lived `InvokeBidi`
//              stream against its configured hub, sends frame 0 =
//              `EnvelopeOpen` carrying the caller URI, then keeps
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
//   2. Send EnvelopeOpen frame 0 with caller URI from
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
//   caller URI, signing key, and a frame dispatcher; opens one
//   bidi, runs forever (until error / shutdown).
// - `SessionFrameDispatcher` trait: the local-side handler
//   implementation. Trait so PR-3 commit 3/3 (integration test)
//   can plug in a mock dispatcher that records frames received
//   without spinning up the full LocalAbilityRegistry.
// - The exponential-backoff supervisor `run_session_supervisor`:
//   the `dial_and_run_session` returns an error → wait → retry.
//   Bounded jitter, capped maximum, never gives up.
//
// Signature model
// ---------------
// When boot supplies a deterministic per-device Ed25519 seed, frame 0
// is signed over the same canonical invocation bytes the admission gate
// verifies. Sparse legacy credentials that only carry `agent_uri` still
// degrade to the unsigned PR-1/PR-2 behaviour so older tests and
// partially-migrated devices do not fail hard during boot.
//
// What this commit does NOT do
// ----------------------------
// - LocalAbilityRegistry stream/unary multiplexing beyond the
//   current RPC path. Production now wires
//   `LocalAbilityDispatcher` at boot for local RPC abilities; true
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
use std::sync::Arc;
use std::time::Duration;

use easynet_axon::invocation::axiom::{
    canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
    InvocationEnvelope, SubjectIdentity as AxiomSubjectIdentity, UriProfile,
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

use crate::pb::axon::v1::invocation_client::InvocationClient;
use crate::pb::axon::v1::invoke_bidi_up::Payload as UpPayload;
use crate::pb::axon::v1::{
    AgentIdentity, BidiControl, BinaryChunk, CallerSignature, Envelope, EnvelopeOpen,
    InvocationTarget, InvokeBidiDown, InvokeBidiUp, StreamDescriptor, SubjectIdentity,
};

/// Daemon-side ability name this initiator targets. The hub's
/// `InvokeBidi` dispatcher routes on
/// `EnvelopeOpen.target.ability_name` and the `<self>.session`
/// arm is the hub-side acceptor PR-2 commit 1/N lands.
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

/// Default URI profile used when the session frame carries a signed
/// envelope. Empty profile fields canonicalise to the same value, but
/// populating the string keeps the wire explicit and easier to inspect.
const DEFAULT_URI_PROFILE: &str = "easynet-strict-v2";

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
///   `LocalAbilityDispatcher` reply frames, device-mode
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
    fn spawn(sender: SessionUpSender, hub_endpoint: String, caller_uri: String) -> Self {
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(SESSION_UP_HEARTBEAT_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // First tick fires immediately; consume it so the first
            // keepalive is sent after one full heartbeat window.
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = sender.send_control(BidiControl::default()).await {
                    eprintln!(
                        "[session] up-heartbeat send failed for `{caller_uri}` on `{hub_endpoint}`: {err}; stopping heartbeat task"
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
/// `caller_uri` is the device's canonical URI per spec §5.1
/// (`easynet:///r/{tenant_id}/agent/{node_id}`). PR-1 staging
/// admits a missing `caller_signature` if the URI is in the
/// hub's realm trust anchor (or matches the hub's own URI for
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
/// `hub_uri` matches `hub_endpoint`.
pub async fn dial_and_run_session<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_uri: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_ca_pem_path: Option<&Path>,
    dispatcher: Arc<D>,
    escalation_outbox: Option<
        &crate::services::axon_serve::session_escalation::SharedSessionOutbox,
    >,
    ability_catalog: &[String],
) -> Result<(), SessionError> {
    dial_and_run_session_with_idle_timeout(
        hub_endpoint,
        caller_uri,
        signing_seed,
        hub_ca_pem_path,
        dispatcher,
        escalation_outbox,
        ability_catalog,
        SESSION_IDLE_TIMEOUT,
    )
    .await
}

async fn dial_and_run_session_with_idle_timeout<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_uri: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_ca_pem_path: Option<&Path>,
    dispatcher: Arc<D>,
    escalation_outbox: Option<
        &crate::services::axon_serve::session_escalation::SharedSessionOutbox,
    >,
    ability_catalog: &[String],
    idle_timeout: Duration,
) -> Result<(), SessionError> {
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

    // Bump client-side gRPC message limits to match the server side
    // (`MAX_INVOCATION_GRPC_MESSAGE_BYTES` = 1 GiB). The tonic-default
    // 4 MiB decoder cap aborted `<self>.session` mid-stream the moment
    // a single down-frame envelope exceeded ~4 MiB — the symptom was
    // `OutOfRange: decoded message length too large` on file-transfer
    // 1 MB+ uploads, where backend's 64 KiB chunks accumulate into
    // larger framed payloads on the down direction. Server side
    // already configures both directions; the client side must too.
    let mut client = InvocationClient::new(channel)
        .max_decoding_message_size(
            crate::services::axon_serve::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        )
        .max_encoding_message_size(
            crate::services::axon_serve::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
        );

    // Membership prelude (URA v4.1.4 dev unblock): axon-runtime hub
    // returns `AXON_MEMBERSHIP_REQUIRED` for any caller whose URI is
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
    eprintln!("[session] sending federation.join prelude as {caller_uri} against {hub_endpoint}");
    match send_federation_join_prelude(&mut client, &caller_uri).await {
        Ok(()) => eprintln!("[session] federation.join prelude OK; proceeding to <self>.session"),
        Err(err) => eprintln!(
            "[session] federation.join prelude soft-failed (code={:?}, msg={:?}); \
             proceeding to <self>.session — bidi will surface the error if join was required",
            err.code(),
            err.message(),
        ),
    }

    // Ability-catalog prelude (URA v4.1.4 dev unblock): publish
    // every locally-registered ability the daemon knows about to
    // the hub's `AbilityCatalogStore` via
    // `federation.advertise_abilities`. The hub projects this back
    // through `federation.resolve(include_abilities=true)` to drive
    // the backend's `/api/v1/abilities` page. Without this, the
    // catalog page renders empty even when devices have
    // observe.health / fs.read / fs.write / etc. registered.
    //
    // Best-effort: a failed advertise leaves the catalog page
    // empty for this device but does not block the bidi from
    // opening. Production daemons retry on every reconnect (the
    // supervisor calls `dial_and_run_session` per backoff), so a
    // transient hub outage self-heals on the next loop pass.
    if !ability_catalog.is_empty() {
        eprintln!(
            "[session] sending federation.advertise_abilities prelude with {} abilities",
            ability_catalog.len()
        );
        if let Err(err) =
            send_advertise_abilities_prelude(&mut client, &caller_uri, ability_catalog).await
        {
            eprintln!(
                "[session] advertise_abilities prelude soft-failed (code={:?}, msg={:?}); \
                 proceeding — Frontend `/api/v1/abilities` page will be empty for this device \
                 until the next reconnect",
                err.code(),
                err.message(),
            );
        } else {
            eprintln!(
                "[session] advertise_abilities prelude OK ({} abilities)",
                ability_catalog.len()
            );
        }

        // Hosted-agent advertise prelude (RFC-006-B v0.6 §URL +
        // RFC-006-C v0.1 §INV-2). The hub's
        // `lookup_target_with_agent_fallback` consults
        // `AdvertisedAgentStore` when the wire callee is an agent
        // URA `agent/<u>.<a>`; without these advertise calls the
        // store is empty and chat-base / page.fetch invocations
        // fall through to `target_offline`.
        //
        // Owner segments derived from the local ability catalog:
        // every ability whose tail is `<owner>.<rest>` implies the
        // daemon hosts agent `<owner>` (skip hub-rooted `01HUB.*`
        // and the placeholder `<self>.*` shapes). Each unique
        // `<owner>` is advertised once as
        // `agent/<owner>.<owner>` HostedBy <caller_uri>; the user-
        // segment of the agent URA matches the daemon's owner
        // convention (EASYNET_PAGES_USER for pages, agent_name
        // for chat-base).
        let realm = caller_uri
            .strip_prefix("easynet:///r/")
            .and_then(|s| s.split_once('/'))
            .map(|(r, _)| r.to_string())
            .unwrap_or_default();
        let user_segment = std::env::var("EASYNET_PAGES_USER").unwrap_or_default();
        let mut owners: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        for name in ability_catalog {
            if let Some((owner, _rest)) = name.split_once('.') {
                if owner == "01HUB" || owner == "<self>" || owner.is_empty() {
                    continue;
                }
                if owner == user_segment {
                    // `<user>.pages.<verb>` — owner segment IS the
                    // user-segment, agent name lives one level
                    // deeper. We pick that up below by walking the
                    // second segment.
                    continue;
                }
                owners.insert(owner.to_string());
            }
        }
        // Synthesise the `pages` agent unconditionally — pages
        // abilities (`<user>.pages.{publish,list,get,unpublish}` +
        // dynamic `<user>.<project>.page.fetch`) register via the
        // late-binding resolver fallback, so they aren't in
        // `ability_catalog` at session-prelude time. The hub's
        // pages_public handler addresses callee=agent/<user>.pages
        // for every page.fetch invocation, so always advertise it.
        if !user_segment.is_empty() && user_segment != "self" {
            owners.insert("pages".to_string());
        }
        if !realm.is_empty() && !user_segment.is_empty() && !owners.is_empty() {
            eprintln!(
                "[session] sending federation.advertise_agent prelude for {} agent(s) \
                 under user `{}`: {:?}",
                owners.len(),
                user_segment,
                owners.iter().collect::<Vec<_>>()
            );
            for owner in &owners {
                let agent_uri =
                    format!("easynet:///r/{realm}/agent/{user_segment}.{owner}");
                if let Err(err) =
                    send_advertise_agent_prelude(&mut client, &caller_uri, &agent_uri).await
                {
                    eprintln!(
                        "[session] advertise_agent {agent_uri} prelude soft-failed \
                         (code={:?}, msg={:?})",
                        err.code(),
                        err.message(),
                    );
                }
            }
            eprintln!(
                "[session] advertise_agent prelude done ({} agent(s))",
                owners.len()
            );
        }
    }

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(SESSION_UP_CHANNEL_CAPACITY);
    let outbound_tx = SessionUpSender::new(up_tx.clone());

    // Frame 0: EnvelopeOpen carrying caller URI + ability name
    // `<self>.session`. When boot resolved a deterministic device
    // seed from credentials we sign the canonical bytes here; older
    // sparse fixtures still degrade to the unsigned PR-2 shape.
    let frame0 = build_session_envelope_open_with_seed(&caller_uri, signing_seed);
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
    eprintln!(
        "[session] bidi opened against `{hub_endpoint}` as {caller_uri}; awaiting down-stream frames"
    );

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
        caller_uri.clone(),
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
                    eprintln!("[session] frame dispatch error: {err}; continuing");
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
    outbox: Option<crate::services::axon_serve::session_escalation::SharedSessionOutbox>,
}

impl OutboxGuard {
    fn new(
        outbox: Option<crate::services::axon_serve::session_escalation::SharedSessionOutbox>,
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
pub async fn run_session_supervisor<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_uri: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_ca_pem_path: Option<PathBuf>,
    dispatcher: Arc<D>,
    escalation_outbox: Option<crate::services::axon_serve::session_escalation::SharedSessionOutbox>,
    ability_catalog: Vec<String>,
    mut cancel: tokio::sync::oneshot::Receiver<()>,
) {
    let mut backoff = SESSION_BACKOFF_INITIAL;
    loop {
        tokio::select! {
            _ = &mut cancel => {
                eprintln!("[session] supervisor cancelled, exiting");
                return;
            }
            result = dial_and_run_session(
                hub_endpoint.clone(),
                caller_uri.clone(),
                signing_seed,
                hub_ca_pem_path.as_deref(),
                Arc::clone(&dispatcher),
                escalation_outbox.as_ref(),
                &ability_catalog,
            ) => {
                match result {
                    Ok(()) => {
                        eprintln!(
                            "[session] hub closed bidi cleanly; reconnecting after {:?}",
                            SESSION_BACKOFF_INITIAL,
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
                        eprintln!(
                            "[session] bidi error ({err:#}); reconnecting after {:?}",
                            backoff,
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

/// Send a one-shot `federation.join@1` over the same gRPC channel
/// the session bidi will open on. Genesis exception in axon-runtime
/// (`signature_policy=RequireSigned` allows this ability unsigned),
/// so the call uses an envelope with caller URI only — no signing
/// material — and a minimal JoinFederationRequest payload.
///
/// We treat both success and "already member" as positive outcomes;
/// any other status is logged by the caller and we continue. The
/// session bidi's HubRejected error is the authoritative gate
/// downstream — if join was needed but failed, the bidi surfaces
/// the right status and the supervisor backs off.
async fn send_federation_join_prelude(
    client: &mut crate::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_uri: &str,
) -> Result<(), tonic::Status> {
    use crate::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest};

    // Wire shape mirrors `federation_wrappers::JoinRequest`:
    // axon-runtime's deserializer is strict (Deserialize derive,
    // no #[serde(default)] on either field) and rejects payloads
    // whose top-level keys differ from `canonical_agent_uri` /
    // `realm`. Sending `agent_uri` / `tenant_id` (the field names
    // used by the local daemon's other federation.* requests)
    // earns InvalidArgument — verified in PR-1 §5 schema-compat
    // tests and observed in dev as
    //   `failed to decode JSON arguments: missing field
    //    canonical_agent_uri`
    let realm = caller_uri
        .strip_prefix("easynet:///r/")
        .and_then(|rest| rest.split_once('/'))
        .map(|(realm, _)| realm.to_string())
        .unwrap_or_default();

    let body = serde_json::json!({
        "canonical_agent_uri": caller_uri,
        "realm": realm,
    });
    let arguments = serde_json::to_vec(&body)
        .map_err(|e| tonic::Status::internal(format!("federation.join prelude serialize: {e}")))?;

    let request = InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                uri: caller_uri.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        function_name: "federation.join".to_string(),
        arguments,
        ..Default::default()
    };

    match client.invoke(request).await {
        Ok(_) => Ok(()),
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
/// gRPC channel the session bidi will open on. Publishes the device
/// daemon's locally-registered ability names to the hub's
/// `AbilityCatalogStore` so the backend's `/api/v1/abilities` page
/// can project them under the device's URI.
///
/// Each ability is represented by a JSON object with `name` and
/// `tool_name` fields — the minimum the catalog projection needs.
/// Richer descriptors (input_schema, description, hints) live in
/// the runtime's per-ability descriptor and can be added here if a
/// future projection wants them; v1 advertises just enough to
/// surface the catalog rows.
/// Session-prelude variant of `federation.advertise_agent`. The
/// device tells the hub "I host agent `<agent_uri>`"; the hub
/// upserts an `AdvertisedAgentRecord { agent_uri, host_uri }` so
/// later inbound invocations addressed to that agent URA resolve
/// to this device's bidi sender via
/// `lookup_target_with_agent_fallback`.
async fn send_advertise_agent_prelude(
    client: &mut crate::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_uri: &str,
    agent_uri: &str,
) -> Result<(), tonic::Status> {
    use crate::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest};

    let body = serde_json::json!({
        "agent_uri": agent_uri,
        "signing_authority": {
            "kind": "hosted_by",
            "host_uri": caller_uri,
        },
    });
    let arguments = serde_json::to_vec(&body).map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_agent prelude serialize: {e}"
        ))
    })?;

    let request = InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                uri: caller_uri.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        function_name: "federation.advertise_agent".to_string(),
        arguments,
        ..Default::default()
    };

    client.invoke(request).await.map(|_| ())
}

async fn send_advertise_abilities_prelude(
    client: &mut crate::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_uri: &str,
    ability_names: &[String],
) -> Result<(), tonic::Status> {
    use crate::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest};

    let abilities: Vec<serde_json::Value> = ability_names
        .iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "tool_name": name,
            })
        })
        .collect();

    let body = serde_json::json!({
        "agent_uri": caller_uri,
        "abilities": abilities,
    });
    let arguments = serde_json::to_vec(&body).map_err(|e| {
        tonic::Status::internal(format!(
            "federation.advertise_abilities prelude serialize: {e}"
        ))
    })?;

    let request = InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                uri: caller_uri.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        function_name: "federation.advertise_abilities".to_string(),
        arguments,
        ..Default::default()
    };

    client.invoke(request).await.map(|_| ())
}

/// Build the EnvelopeOpen frame 0 a device sends to open
/// `<self>.session`. Public so PR-2 commit 1/N's hub-side
/// acceptor tests can construct a matching expected frame, and
/// so the integration test in PR-3 commit 3/3 can drive a mock
/// device through the same shape.
#[must_use]
pub fn build_session_envelope_open(caller_uri: &str) -> InvokeBidiUp {
    build_session_envelope_open_with_seed(caller_uri, None)
}

/// Build the frame-0 `EnvelopeOpen`, optionally signing it when a
/// deterministic device seed is available.
#[must_use]
pub fn build_session_envelope_open_with_seed(
    caller_uri: &str,
    signing_seed: Option<SessionSigningSeed>,
) -> InvokeBidiUp {
    let initial_args = Vec::new();
    let args_digest: [u8; 32] = Sha256::digest(&initial_args).into();

    let mut envelope = Envelope {
        caller: Some(AgentIdentity {
            uri: caller_uri.to_string(),
            profile: DEFAULT_URI_PROFILE.to_string(),
        }),
        // `<self>.session` is the device presenting its own long-
        // lived reverse channel; callee + subject both point at the
        // caller device so the signed tuple is stable and self-
        // describing even before a future hub-URI contract lands.
        callee: Some(AgentIdentity {
            uri: caller_uri.to_string(),
            profile: DEFAULT_URI_PROFILE.to_string(),
        }),
        subject: Some(SubjectIdentity {
            uri: caller_uri.to_string(),
            profile: DEFAULT_URI_PROFILE.to_string(),
        }),
        ..Envelope::default()
    };

    let mut mac = Vec::new();
    if let Some(seed) = signing_seed {
        let mut nonce = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        envelope.invocation_nonce = nonce.to_vec();

        let axiom_env = InvocationEnvelope {
            caller: AxiomAgentIdentity::new(caller_uri, UriProfile::EasynetStrictV2),
            callee: AxiomAgentIdentity::new(caller_uri, UriProfile::EasynetStrictV2),
            subject: AxiomSubjectIdentity::new(caller_uri, UriProfile::EasynetStrictV2),
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
        ..InvokeBidiUp::default()
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
    use std::net::SocketAddr;

    use crate::pb::axon::v1::invocation_server::{Invocation, InvocationServer};
    use crate::pb::axon::v1::{
        InvokeRequest, InvokeResponse, InvokeServerStreamRequest, InvokeStreamChunk,
    };
    use futures::stream;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::{Request, Response};

    /// A mock dispatcher that just records every down frame it
    /// receives. Used by tests; production wires the real
    /// LocalAbilityRegistry-backed dispatcher.
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
                    payload: Some(crate::pb::axon::v1::invoke_bidi_down::Payload::Receipt(
                        crate::pb::axon::v1::InvocationReceipt {
                            state: crate::pb::axon::v1::InvocationState::Admitted as i32,
                            ..crate::pb::axon::v1::InvocationReceipt::default()
                        },
                    )),
                    ..InvokeBidiDown::default()
                }),
                Ok(InvokeBidiDown {
                    sequence: 9,
                    payload: Some(crate::pb::axon::v1::invoke_bidi_down::Payload::Control(
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

    #[test]
    fn build_session_envelope_open_carries_caller_uri_and_ability_name() {
        let frame = build_session_envelope_open("easynet:///r/realm/agent/n1");
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
                .map(|a| a.uri.as_str())
                .unwrap_or(""),
            "easynet:///r/realm/agent/n1",
        );
    }

    #[test]
    fn build_session_envelope_open_includes_one_stream_descriptor() {
        let frame = build_session_envelope_open("easynet:///r/realm/agent/n1");
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
            build_session_envelope_open_with_seed("easynet:///r/realm/agent/n1", Some(seed));
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
                .map(|s| s.uri.as_str())
                .unwrap_or(""),
            "easynet:///r/realm/agent/n1",
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

    #[tokio::test]
    async fn invalid_endpoint_returns_invalid_endpoint_error() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let result = dial_and_run_session(
            "not a valid uri".to_string(),
            "easynet:///r/realm/agent/n1".to_string(),
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
                "easynet:///r/realm/agent/n1".to_string(),
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
            "easynet:///r/realm/agent/n1".to_string(),
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
            "easynet:///r/realm/agent/n1".to_string(),
            None,
            None,
            dispatcher,
            None,       // PR-N6 C4: no escalation outbox in this test
            Vec::new(), // ability_catalog: empty in tests
            cancel_rx,
        ));

        // Give the supervisor a beat to start its first dial then
        // cancel. The dial will fail (connect refused on port 1)
        // and the supervisor will be sleeping in backoff when the
        // cancel arrives.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = cancel_tx.send(());

        let exit_within_bound = tokio::time::timeout(Duration::from_secs(2), supervisor_handle)
            .await
            .expect("supervisor exits within 2 s of cancel");
        exit_within_bound.expect("supervisor task did not panic");
    }

    #[tokio::test]
    async fn silent_hub_triggers_idle_timeout_reconnect_error() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (addr, _server) = spawn_silent_session_hub().await;

        let result = dial_and_run_session_with_idle_timeout(
            format!("http://{addr}"),
            "easynet:///r/realm/agent/n1".to_string(),
            None,
            None,
            dispatcher,
            None,
            &[],
            Duration::from_millis(80),
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
            "easynet:///r/realm/agent/n1".to_string(),
            None,
            None,
            dispatcher,
            None,
            &[],
            Duration::from_secs(1),
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
