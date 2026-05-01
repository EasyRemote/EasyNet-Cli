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

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::{Signer as _, SigningKey};
use easynet_axon::invocation::axiom::{
    canonical_invocation_bytes, AgentIdentity as AxiomAgentIdentity, CausalContext,
    InvocationEnvelope, SubjectIdentity as AxiomSubjectIdentity, UriProfile,
};
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
    AgentIdentity, CallerSignature, Envelope, EnvelopeOpen, InvocationTarget, InvokeBidiDown,
    InvokeBidiUp, StreamDescriptor, SubjectIdentity,
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
        outbound: &mpsc::Sender<InvokeBidiUp>,
    ) -> Result<(), SessionDispatchError>;
}

/// Error from a single down-frame dispatch. Reported by the
/// dispatcher; the supervisor logs and continues.
#[derive(Debug, thiserror::Error)]
pub enum SessionDispatchError {
    #[error("session frame dispatch failed: {0}")]
    Other(String),
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
pub async fn dial_and_run_session<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_uri: String,
    signing_seed: Option<SessionSigningSeed>,
    dispatcher: Arc<D>,
) -> Result<(), SessionError> {
    let endpoint = Endpoint::from_shared(hub_endpoint.clone())
        .map_err(|err| SessionError::InvalidEndpoint {
            endpoint: hub_endpoint.clone(),
            source: err,
        })?
        // No timeout on the bidi itself — the stream is intended
        // to live forever. Connect timeout caps the dial step.
        .connect_timeout(Duration::from_secs(10));

    let channel: Channel = endpoint
        .connect()
        .await
        .map_err(|err| SessionError::ConnectFailed {
            endpoint: hub_endpoint.clone(),
            source: err,
        })?;

    let mut client = InvocationClient::new(channel);

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(SESSION_UP_CHANNEL_CAPACITY);

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
    let outbound_tx = up_tx;

    while let Some(frame_result) = down_stream.next().await {
        match frame_result {
            Ok(frame) => {
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

/// Long-lived supervisor wrapping `dial_and_run_session` with
/// exponential backoff. Returns only when `cancel` resolves; the
/// reconnect loop never exits on its own. Production daemons run
/// this on a `tokio::spawn` at boot.
pub async fn run_session_supervisor<D: SessionFrameDispatcher>(
    hub_endpoint: String,
    caller_uri: String,
    signing_seed: Option<SessionSigningSeed>,
    dispatcher: Arc<D>,
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
                Arc::clone(&dispatcher),
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
                        eprintln!(
                            "[session] bidi error ({err}); reconnecting after {:?}",
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

    #[error("internal: failed to enqueue {0} for hub send")]
    SendFailed(&'static str),
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
            _outbound: &mpsc::Sender<InvokeBidiUp>,
        ) -> Result<(), SessionDispatchError> {
            self.received.lock().await.push(frame);
            Ok(())
        }
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
        let frame = build_session_envelope_open_with_seed(
            "easynet:///r/realm/agent/n1",
            Some(seed),
        );
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
            dispatcher,
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
                dispatcher,
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
    async fn supervisor_exits_on_cancel() {
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let supervisor_handle = tokio::spawn(run_session_supervisor(
            "http://127.0.0.1:1".to_string(),
            "easynet:///r/realm/agent/n1".to_string(),
            None,
            dispatcher,
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
}
