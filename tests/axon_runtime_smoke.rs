//! Phase 0 smoke test for the new Axon SDK surface
//! ================================================
//!
//! Proves that CLI can link against and exercise the new Axon
//! integration points end-to-end while the CLI crate routes daemon
//! execution through Axon's `LocalRuntime`:
//!
//!   * `LocalRuntime` + `register_*_ability` (call-mode taxonomy)
//!   * `KeyResolver` trait for callee-side admission
//!   * `invoke_descriptor_bound_bidi_request_async` with an externally-signed
//!     descriptor-bound request (the entry CLI's `runtime.invoke_remote` will
//!     route through the same request-level shape)
//!   * `LedgerSink` auto-persistence (the entry that replaces CLI's
//!     in-memory `SharedReceiptStore`)
//!
//! When this test passes we know the Axon surface is fully reachable
//! from inside the CLI crate; Phase 1+ can begin the actual
//! migration.

use std::sync::Arc;
use std::time::Duration;

use easynet_axon::invocation::{
    fresh_nonce, make_ability, sign_descriptor_bound_invocation, signing_key_from_bytes,
    AbilityOptions, AgentIdentity, AxonError, BidiInputFrame, CallMode, CallerSignature,
    CausalContext, DescriptorBoundEnvelope, DescriptorBoundEnvelopeParts,
    DescriptorBoundInvocationRequest, InvocationLedger, KeyResolver, LedgerSink, LocalRuntime,
    SubjectIdentity, UraProfile,
};
use easynet_cli::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;
use ed25519_dalek::{SigningKey, VerifyingKey};

const REALM: &str = "cli-smoke";
const SMOKE_SCHEMA_HASH: [u8; 32] = [0x11; 32];
const SMOKE_IMPL_HASH: [u8; 32] = [0x22; 32];

fn agent(ura: &str) -> AgentIdentity {
    AgentIdentity::new(ura, UraProfile::EasynetStrictV2)
}

fn caller_ura() -> String {
    format!("easynet:///r/{REALM}/agent/u.alice")
}

fn callee_ura() -> String {
    format!("easynet:///r/{REALM}/device/host")
}

fn runtime_ability_ura(ability: &str) -> String {
    easynet_cli::ura::owner_ability_ura(&callee_ura(), ability).expect("callee-owned ability URA")
}

fn descriptor_proof_options(options: AbilityOptions) -> AbilityOptions {
    options.with_descriptor_proof(
        DEFAULT_ABILITY_DESCRIPTOR_VERSION,
        SMOKE_SCHEMA_HASH,
        SMOKE_IMPL_HASH,
    )
}

/// Trust anchor stand-in: returns one known key for every URA.
/// Phase 1 replaces this with a real `RealmTrustAnchorKeyResolver`.
struct FixedKey(VerifyingKey);

impl KeyResolver for FixedKey {
    fn resolve(&self, _agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        Ok(self.0)
    }
}

/// Build (runtime, ledger, tempdir-handle, caller-signing-key).
/// Tempdir is returned so the test keeps it alive for the run.
async fn build_runtime() -> (
    Arc<LocalRuntime>,
    Arc<InvocationLedger>,
    tempfile::TempDir,
    SigningKey,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("invocations.redb");
    let ledger = Arc::new(InvocationLedger::open(&path).expect("open ledger"));

    let signing_key = signing_key_from_bytes(&[0x42; 32]);
    let verifying_key = signing_key.verifying_key();

    let rt = LocalRuntime::new();
    rt.set_ledger_sink(LedgerSink::new(Arc::clone(&ledger)));
    rt.set_admission_key_resolver(Arc::new(FixedKey(verifying_key)));

    (rt, ledger, temp, signing_key)
}

/// Build a signed envelope for `ability(payload)` from caller→callee
/// in this test's realm. Mirrors how the daemon's gRPC layer will
/// reconstruct an axon envelope from the wire `InvokeBidiUp` frame 0
/// in Phase 4.
fn build_signed_envelope(
    signing_key: &SigningKey,
    ability: &str,
    payload: &[u8],
) -> (DescriptorBoundEnvelope, CallerSignature) {
    let callee = agent(&callee_ura());
    let subject = SubjectIdentity::from_callee(&callee);
    let ability_ref = format!(
        "{}@{}",
        easynet_cli::ura::owner_ability_ura(&callee.ura, ability)
            .expect("callee-owned ability URA"),
        DEFAULT_ABILITY_DESCRIPTOR_VERSION
    );
    let envelope = DescriptorBoundEnvelope::from_parts(DescriptorBoundEnvelopeParts {
        caller: agent(&caller_ura()),
        callee,
        ability: ability_ref,
        subject,
        invocation_nonce: fresh_nonce(),
        causal_context: CausalContext::None,
        args_bytes: payload,
    })
    .expect("descriptor-bound envelope");
    let signature = sign_descriptor_bound_invocation(signing_key, &envelope, "smoke-test-key");
    (envelope, signature)
}

// ── Test 1: bidi happy path + ledger persist ────────────────────────

#[tokio::test]
async fn axon_bidi_invoke_externally_signed_persists_to_ledger() {
    let (rt, ledger, _temp, signing_key) = build_runtime().await;

    // Register a bidi ability that:
    //   - reads one inbound message,
    //   - emits one progress frame,
    //   - returns terminal payload echoing what it received.
    rt.register_ability_with_options(
        runtime_ability_ura("test.echo_bidi"),
        make_ability(|ctx| async move {
            let msg = ctx
                .recv_message(Some(Duration::from_secs(5)))
                .await
                .ok_or_else(|| AxonError::internal("no inbound message received"))?;
            ctx.emit_progress(b"halfway".to_vec(), "text/plain")
                .await
                .map_err(|e| AxonError::internal(format!("emit_progress failed: {e}")))?;
            Ok(msg.payload)
        }),
        descriptor_proof_options(AbilityOptions::bidi()),
    )
    .await
    .expect("register test.echo_bidi");

    let payload = b"hello-from-cli".to_vec();
    let (envelope, signature) = build_signed_envelope(&signing_key, "test.echo_bidi", &payload);

    let request = DescriptorBoundInvocationRequest::externally_signed(
        CallMode::Bidi,
        envelope,
        signature,
        payload.clone(),
    );
    let (mut handle, _signed) = rt
        .invoke_descriptor_bound_bidi_request_async(request)
        .await
        .expect("invoke_descriptor_bound_bidi_request_async returns Ok");

    // Push one input frame in (the handler is blocked on recv_message
    // until this arrives).
    let _ack = handle
        .send_input(BidiInputFrame::new(payload.clone()))
        .await
        .expect("send_input succeeds");

    // Drain frames: expect (progress, terminal).
    let f1 = handle
        .next_frame()
        .await
        .expect("first frame yielded")
        .expect("first frame is not an error");
    assert!(
        !f1.terminal,
        "first frame must be progress (terminal=false), got terminal frame"
    );
    assert_eq!(f1.payload, b"halfway");

    let f2 = handle
        .next_frame()
        .await
        .expect("terminal frame yielded")
        .expect("terminal frame is not an error");
    assert!(f2.terminal, "second frame must be terminal");
    assert_eq!(f2.payload, payload, "terminal payload echoes input");

    // After terminal, the stream closes.
    assert!(handle.next_frame().await.is_none());

    // LedgerSink writes in the spawn task after the terminal receipt
    // emits — yield + small sleep for the flush, same as Axon's own
    // ledger_sink_and_external_signed.rs tests.
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let records = ledger.list_all().expect("list ledger");
    assert_eq!(
        records.len(),
        1,
        "exactly one terminal record persisted by LedgerSink"
    );
    let r = &records[0];
    assert_eq!(
        r.ability_name,
        format!(
            "{}@{}",
            easynet_cli::ura::owner_ability_ura(&callee_ura(), "test.echo_bidi")
                .expect("callee-owned ability URA"),
            DEFAULT_ABILITY_DESCRIPTOR_VERSION
        )
    );
    assert_eq!(r.state, "completed");
    assert!(r.result.is_some(), "completed terminal carries result");
    assert!(r.error.is_none(), "completed terminal has no error");
    assert!(
        r.receipt_chain.verified,
        "audit chain must verify; detail={}",
        r.receipt_chain.verification_detail
    );
}

// ── Test 2: args_digest mismatch is rejected without side effects ───

#[tokio::test]
async fn axon_externally_signed_rejects_args_digest_mismatch() {
    let (rt, ledger, _temp, signing_key) = build_runtime().await;
    rt.register_ability_with_options(
        runtime_ability_ura("test.echo"),
        make_ability(|ctx| async move { Ok(ctx.payload.clone()) }),
        descriptor_proof_options(AbilityOptions::default()),
    )
    .await
    .expect("register test.echo");

    let payload_original = b"original".to_vec();
    let (envelope, signature) = build_signed_envelope(&signing_key, "test.echo", &payload_original);

    // Wire-tamper: caller signed over `payload_original` but the
    // daemon receives a different body. `invoke_descriptor_bound_externally_signed_*`
    // hashes the received payload and rejects on mismatch BEFORE
    // burning the nonce or running the handler.
    //
    // Note: `Result::expect_err` needs `Debug` on the Ok arm, but
    // `InvocationHandle` doesn't implement Debug, so we match
    // by hand instead.
    let outcome = rt
        .invoke_descriptor_bound_externally_signed_async(
            envelope,
            signature,
            b"tampered".to_vec(),
            None,
            None,
        )
        .await;
    let err = match outcome {
        Err(e) => e,
        Ok(_) => panic!("args_digest mismatch must be rejected, but invoke returned Ok"),
    };
    assert!(
        err.to_string().contains("args_digest_mismatch"),
        "expected args_digest_mismatch reason, got: {err}"
    );

    // Nothing should have been persisted.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        ledger.list_all().expect("list ledger").is_empty(),
        "rejected invocation must not leak a ledger row"
    );
}

// ── Test 3: call-mode gate rejects mismatched mode ──────────────────

#[tokio::test]
async fn axon_call_mode_gate_rejects_rpc_call_to_bidi_ability() {
    let (rt, _ledger, _temp, signing_key) = build_runtime().await;
    rt.register_ability_with_options(
        runtime_ability_ura("test.bidi_only"),
        make_ability(|_| async move { Ok(Vec::new()) }),
        descriptor_proof_options(AbilityOptions::bidi()),
    )
    .await
    .expect("BIDI registration accepted");

    let payload = b"".to_vec();
    let (envelope, signature) = build_signed_envelope(&signing_key, "test.bidi_only", &payload);

    // Calling via the RPC entry should fail at the call-mode gate
    // BEFORE admission — so the nonce is never recorded and a
    // subsequent BIDI call with the same nonce would still succeed.
    let outcome = rt
        .invoke_descriptor_bound_externally_signed_async(envelope, signature, payload, None, None)
        .await;
    let err = match outcome {
        Err(e) => e,
        Ok(_) => panic!("RPC call to BIDI-only ability must be rejected"),
    };
    assert!(
        err.to_string().contains("does not support call mode"),
        "expected call-mode gate diagnostic, got: {err}"
    );
}
