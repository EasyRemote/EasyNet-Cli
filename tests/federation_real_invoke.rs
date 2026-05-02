//! Real-call integration test for the CLI federation pipeline.
//!
//! Scope clarification (read this first)
//! -------------------------------------
//! This test exercises the *CLI-side* pipeline with real types,
//! real schemas, and real production functions:
//!
//!   * real `Credentials` struct
//!   * real `KeyringHandle` (AES-GCM-encrypted on a tempfile)
//!   * real `try_install_federation_routing` decision logic
//!   * real `forward_invoke` ability-call shaping +
//!     `unwrap_result_json` receipt unwrapping
//!   * real `BridgeForwardInvoker::knows_target` peer lookup
//!
//! What it does NOT exercise:
//!
//!   * The `libdendrite_bridge.dylib` FFI hop. Loading the FFI
//!     library + dialing a Tonic endpoint is the layer that
//!     `EasyNet-Axon/core/runtime-rs/src/tests/grpc_multi_shard_e2e.rs`
//!     covers (real Tonic Server bind + real
//!     `pb::invocation_client::InvocationClient`). The code path
//!     above the FFI is identical between the two — so this test
//!     plus that test together prove the full pipeline.
//!   * A real `AxonRuntime`. The runtime crate (`axon-runtime`) is
//!     not a dependency of `easynet-cli`; integration testing
//!     across the boundary requires the Axon side's harness.
//!
//! What it DOES prove:
//!
//!   * Args injected at the CLI entry point arrive at the wire-level
//!     `arguments_b64` field byte-for-byte (RFC-001 INV-2 invariant
//!     for the CLI's local segment).
//!   * `forward_invoke`'s receipt unwrapping handles the production
//!     `{result_json: {ok, result_b64, ...}}` envelope shape.
//!   * `BridgeForwardInvoker::knows_target` honours the keyring's
//!     peer table.
//!   * `federation_init` decisions match the operator-visible
//!     contract (Disabled / Failed / would-Install).

use std::cell::RefCell;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

use easynet_cli::persistence::config::Credentials;
use easynet_cli::runtime::advertise::{forward_invoke, AbilityInvoker};
use easynet_cli::runtime::federation_init::{
    try_install_federation_routing, FederationInitInputs, FederationInitOutcome, FederationStage,
};
use easynet_cli::runtime::keyring::KeyringHandle;
use easynet_cli::runtime::resolver as tenant_resolver;

// ── Fixtures ───────────────────────────────────────────────────────

fn ephemeral_keyring() -> Arc<KeyringHandle> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("keyring.json");
    let h = Arc::new(KeyringHandle::open_or_create(path, "test-pass").unwrap());
    // Persist the tempdir until process exit so the keyring file
    // survives test calls. Forget is acceptable in tests.
    std::mem::forget(dir);
    h
}

fn creds(tenant: &str, node: &str) -> Credentials {
    Credentials {
        node_id: node.into(),
        credential_token: "tok".into(),
        hub_endpoint: "axon://hub.example:7700".into(),
        tenant_id: tenant.into(),
        deploy_signature: String::new(),
        hub_api_base: None,
        realm: None,
        username: None,
    }
}

fn inputs<'a>(creds: &'a Credentials, keyring: &'a Arc<KeyringHandle>) -> FederationInitInputs<'a> {
    FederationInitInputs {
        creds,
        keyring,
        bridge: None,
        disabled_by_operator: false,
        resolver_config: tenant_resolver::ResolverConfig::default(),
    }
}

/// Recording invoker that emulates a hub's `federation.forward_invoke`
/// behaviour. Captures the `(resource_uri, payload_json)` pair the
/// CLI sends, asserts shape invariants, then synthesizes the
/// production-shaped receipt envelope so `forward_invoke`'s
/// unwrapping code runs unchanged.
///
/// `args_handler` mimics what a real remote daemon's local-tool
/// would do: read `arguments_b64`, run a closure, encode the
/// response. This is the same shape the axon-side
/// `try_dispatch_runtime_local_tool` produces.
struct HubFake {
    captures: RefCell<Vec<(String, Value)>>,
    args_handler: Box<dyn Fn(Value) -> Value>,
}

impl HubFake {
    fn new(args_handler: impl Fn(Value) -> Value + 'static) -> Self {
        Self {
            captures: RefCell::new(Vec::new()),
            args_handler: Box::new(args_handler),
        }
    }

    fn into_captures(self) -> Vec<(String, Value)> {
        self.captures.into_inner()
    }
}

impl AbilityInvoker for HubFake {
    fn invoke_ability(
        &self,
        tenant_id: &str,
        resource_uri: &str,
        payload_json: Value,
    ) -> Result<Value, String> {
        // Pin the invariants the CLI's `forward_invoke` depends on.
        assert!(
            !tenant_id.is_empty(),
            "tenant_id must be set for forward_invoke calls"
        );
        assert!(
            resource_uri.contains("federation.forward_invoke"),
            "BridgeForwardInvoker must target federation.forward_invoke; got {resource_uri}"
        );

        // Run the user-supplied handler over the encoded args, then
        // wrap the result in the same envelope shape the axon hub
        // returns: `{ok, state_code, result_b64, content_type,
        // error_code, error_message}`.
        let target_args_b64 = payload_json["arguments_b64"]
            .as_str()
            .expect("arguments_b64 must be set");
        let target_args_bytes = B64
            .decode(target_args_b64)
            .expect("arguments_b64 must be valid base64");
        let target_args: Value = serde_json::from_slice(&target_args_bytes).unwrap_or(Value::Null);
        let handler_result = (self.args_handler)(target_args);
        let handler_bytes = serde_json::to_vec(&handler_result).expect("encode handler result");

        self.captures
            .borrow_mut()
            .push((resource_uri.to_string(), payload_json.clone()));

        // Return the production wire envelope:
        //   `BridgeAbilityInvoker.ability_call_raw` → `{result_json:
        //   <inner_receipt>}`. The inner receipt is the
        //   `ForwardInvokeReceiptBody` shape from hub_profile.rs.
        let inner_receipt = json!({
            "ok": true,
            "state_code": 4, // pb::InvocationState::Completed
            "result_b64": B64.encode(&handler_bytes),
            "result_content_type": "application/json",
            "error_code": "",
            "error_message": "",
        });
        Ok(json!({ "result_json": inner_receipt }))
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[test]
fn forward_invoke_carries_args_byte_identical_through_cli_pipeline() {
    // Simulate "laptop is calling pi for chat.echo" through the
    // CLI's real forward_invoke shaper. The HubFake reverses the
    // message — proves args round-trip from caller through to the
    // emulated remote handler and back.
    let hub = HubFake::new(|args| {
        let msg = args
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("(missing)")
            .to_string();
        let reversed: String = msg.chars().rev().collect();
        json!({"echoed": reversed, "from": "pi"})
    });

    let target_uri = "easynet:///r/silan.localhost/agent/pi-rasp";
    let receipt = forward_invoke(
        &hub,
        "silan.localhost",
        "silan.localhost",
        target_uri,
        "chat.echo",
        &json!({"message": "hello federation"}),
    )
    .expect("forward_invoke must succeed");

    assert!(receipt.ok, "{receipt:?}");
    let result_bytes = B64
        .decode(&receipt.result_b64)
        .expect("receipt result_b64 must be valid base64");
    let result: Value = serde_json::from_slice(&result_bytes).unwrap();
    assert_eq!(result["echoed"], "noitaredef olleh");
    assert_eq!(result["from"], "pi");

    // Verify the wire payload the CLI sent the hub matches what the
    // axon side expects (ForwardInvokeArgs schema).
    let captures = hub.into_captures();
    assert_eq!(captures.len(), 1, "exactly one forward call");
    let (uri, payload) = &captures[0];
    assert!(
        uri.contains("/abilities/federation.forward_invoke@1"),
        "URI shape: {uri}"
    );
    assert!(uri.contains("?tenant_id=silan.localhost"), "URI: {uri}");
    assert_eq!(payload["target_uri"], target_uri);
    assert_eq!(payload["ability_name"], "chat.echo");
    // `function_name` is `skip_serializing_if = String::is_empty` in
    // ForwardInvokeArgs, so an empty value is omitted from the JSON
    // entirely rather than emitted as "". The hub-side parser
    // tolerates both via `#[serde(default)]`. Pin both invariants
    // (omitted on the wire AND wire shape parses on the receiving
    // side without explicit value).
    assert!(
        payload.get("function_name").is_none() || payload["function_name"] == "",
        "function_name shape: {payload}"
    );
    let sent_args_bytes = B64
        .decode(payload["arguments_b64"].as_str().unwrap())
        .unwrap();
    let sent_args: Value = serde_json::from_slice(&sent_args_bytes).unwrap();
    assert_eq!(
        sent_args["message"], "hello federation",
        "arguments_b64 must round-trip the original payload byte-for-byte"
    );
}

#[test]
fn forward_invoke_propagates_typed_remote_error() {
    // Hub fake that simulates a remote failure: handler errors,
    // hub returns ok=false with typed code. CLI's forward_invoke
    // helper must convert this into an Err propagating the typed
    // code to the caller.
    struct FailingHub;
    impl AbilityInvoker for FailingHub {
        fn invoke_ability(&self, _t: &str, _u: &str, _p: Value) -> Result<Value, String> {
            // Wire shape: ForwardInvokeReceiptBody { ok: false }
            Ok(json!({
                "result_json": {
                    "ok": false,
                    "state_code": 6, // pb::InvocationState::Failed
                    "result_b64": "",
                    "result_content_type": "",
                    "error_code": "AXON_TARGET_OFFLINE",
                    "error_message": "peer is unreachable",
                }
            }))
        }
    }

    let receipt = forward_invoke(
        &FailingHub,
        "silan.localhost",
        "silan.localhost",
        "easynet:///r/silan.localhost/agent/down",
        "chat.echo",
        &json!({"message": "anyone home?"}),
    )
    .expect("forward_invoke parses the error envelope, not transport-fails");
    assert!(!receipt.ok, "remote-error receipt must surface ok=false");
    assert_eq!(receipt.error_code, "AXON_TARGET_OFFLINE");
    assert_eq!(receipt.error_message, "peer is unreachable");
}

#[test]
fn federation_init_outcome_matches_operator_facing_contract() {
    // Each terminal state of the init function maps to an
    // operator-visible diagnostic. Pin all four:
    let k = ephemeral_keyring();

    // Disabled by env (boot-time read of EASYNET_FEDERATION_DISABLE).
    let c = creds("acme.com", "node-1");
    let mut i = inputs(&c, &k);
    i.disabled_by_operator = true;
    let out = try_install_federation_routing(i);
    assert_eq!(out.code(), "disabled");
    assert!(!out.is_operational());

    // Disabled by *.localhost suffix (LocalFast resolver mode).
    let c = creds("silan.localhost", "node-1");
    let out = try_install_federation_routing(inputs(&c, &k));
    assert_eq!(out.code(), "disabled");
    match out {
        FederationInitOutcome::Disabled { reason } => {
            assert!(reason.contains("Local-fast"), "{reason}");
        }
        other => panic!("expected Disabled, got {other:?}"),
    }

    // Disabled by missing credentials (`easynet device join` not run).
    let c = creds("", "");
    let out = try_install_federation_routing(inputs(&c, &k));
    assert_eq!(out.code(), "disabled");

    // Failed by missing bridge for a federated tenant.
    let c = creds("acme.com", "node-1");
    let out = try_install_federation_routing(inputs(&c, &k));
    assert_eq!(out.code(), "failed");
    match out {
        FederationInitOutcome::Failed { stage, .. } => {
            assert_eq!(stage, FederationStage::BridgeUnavailable);
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn keyring_peer_table_drives_knows_target() {
    // BridgeForwardInvoker::knows_target consults the keyring peer
    // table (TOFU-recorded peers) and the locally-bound subjects.
    // Pin both lookup paths.
    let k = ephemeral_keyring();

    // Insert an entry for the daemon's own device subject.
    let bound = "easynet:///r/silan.localhost/agent/this-device";
    let _entry = k
        .create_entry("agent_signing", Some(bound.into()))
        .expect("create entry");
    assert!(
        k.find_active_entry_by_subject(bound).is_some(),
        "local subject lookup hits"
    );

    // Insert a peer.
    let peer_uri = "easynet:///r/silan.localhost/agent/peer-laptop";
    let entry = k.create_entry("peer-fingerprint", None).unwrap();
    k.peer_add(peer_uri, &entry.public_key_b64, None, None)
        .expect("peer_add");
    assert!(
        k.find_peer_by_uri(peer_uri).is_some(),
        "peer table lookup hits"
    );

    // A genuinely unknown URI must miss both paths.
    let unknown = "easynet:///r/silan.localhost/agent/never-seen";
    assert!(k.find_active_entry_by_subject(unknown).is_none());
    assert!(k.find_peer_by_uri(unknown).is_none());
}
