//! Product-flow end-to-end test (RFC-002 + RFC-002.2).
//!
//! User asked: 创建用户获得 key → easynet device join → 设备1 注册
//! agent1 → 设备2 注册 agent2 → agent1 调用 agent2.chat
//! 一起讨论. The previous axon-internal tests (forward_invoke_e2e,
//! multi_shard_e2e) proved the protocol layer; this one walks the
//! product-visible surface from the CLI's perspective. We do NOT
//! exercise the Hub HTTP pairing endpoint — that lives in
//! easynet.run's web tier and is out of scope for the CLI repo.
//! Instead we synthesise the post-pairing state (credentials +
//! keyring + agent registry) directly and prove that the chat
//! round-trip works through `<agent>.invoke` on each "device".
//!
//! Because the test crosses many layers it lives at the CLI's
//! integration-test surface (`tests/`), not as a unit test. It
//! takes the longest path the CLI exposes: keyring (RFC-002) →
//! agent registry → `<agent>.invoke` → forward routing
//! (RFC-002 §5.2, RFC-002.2 §2.3) → fake remote daemon handler.

use easynet_axon::invocation::LocalRuntime;
use easynet_cli::daemon::ability::builtins::agents::invoke as invoke_ability;
use easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog;
use easynet_cli::daemon::invocation::routing::target::{CallMode, InvocationTarget, TargetScope};
use easynet_cli::daemon::keyring::forward as fwd;
use easynet_cli::daemon::persistence::agent_registry::{AgentEntry, AgentRegistry, AgentType};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};

// ── Shared serial guard so two product-flow tests don't race on
//    the global forward-invoker slot.
fn product_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn local_rpc_target(ability: &str, args: Value) -> InvocationTarget {
    InvocationTarget {
        scope: TargetScope::Local,
        ability: ability.to_string(),
        normalized_args: args,
        call_mode: CallMode::Rpc,
        subject: None,
        causal_context: None,
    }
}

#[derive(Clone)]
struct ProductRuntimeHarness {
    catalog: Arc<AxonAbilityCatalog>,
}

impl ProductRuntimeHarness {
    fn new(catalog: Arc<AxonAbilityCatalog>) -> Self {
        Self { catalog }
    }

    fn invoke_rpc(&self, ability: &str, args: Value) -> anyhow::Result<Value> {
        self.catalog
            .invoke_rpc_target_json(local_rpc_target(ability, args))
    }
}

fn build_registry_with_invoke(self_agent: &str, agents: AgentRegistry) -> ProductRuntimeHarness {
    let runtime = LocalRuntime::new();
    let mut reg = AxonAbilityCatalog::new_with_runtime(Arc::clone(&runtime));
    let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
    let h_for_register = Arc::clone(&handle);
    let agents_clone = agents.clone();
    invoke_ability::register_for_agent(
        &mut reg,
        self_agent.to_string(),
        move || agents_clone.clone(),
        h_for_register,
    );
    let arc_reg = Arc::new(reg);
    if handle.set(Arc::clone(&arc_reg)).is_err() {
        panic!("handle already set");
    }
    ProductRuntimeHarness::new(arc_reg)
}

#[test]
fn user_join_two_devices_chat_round_trip() {
    let _g = product_test_lock();

    // ── Step 1: simulate "create user, get key" (Hub web does
    //    this in production; here we just generate a keypair via
    //    the CLI's keyring crypto primitives — same code path).
    use easynet_cli::daemon::keyring::crypto::fresh_ed25519_keypair;
    let (_seed, _pk) = fresh_ed25519_keypair().unwrap();
    // The seed/public key are what Hub would issue. They are
    // attached to the per-device credentials below.

    // ── Step 2: simulate `easynet device join` for device1 +
    //    device2. Both belong to the SAME user/tenant
    //    `silan.localhost`. In production this is the post-
    //    pairing state written to ~/.easynet/credentials.json.
    let tenant = "silan.localhost";
    let device1_node_id = "laptop-mac";
    let device2_node_id = "phone-pi";
    // v4.1.5: device URAs are `easynet:///r/<realm>/device/<node-id>`.
    // An earlier copy of this test used `agent/<node-id>` (v4.1.4
    // shape, single-segment tail), but the SDK ParseError::AgentBadShape
    // now requires `user.agent` two-segment tails — so the test was
    // silently routing through the unparseable-URA arm and failing
    // with `target_not_registered`. Pin the canonical device shape
    // here so federation routing sees a real URI.
    let device1_uri = format!("easynet:///r/{tenant}/device/{device1_node_id}");
    let device2_uri = format!("easynet:///r/{tenant}/device/{device2_node_id}");

    // ── Step 3: register agent1 on device1, agent2 on device2.
    //    Each device's local agent registry knows only its own
    //    locally-installed agents (matches what
    //    `easynet agent add` writes to the per-device registry).
    let mut device1_agents = AgentRegistry::default();
    device1_agents.agents.insert(
        "agent1".into(),
        AgentEntry::new(AgentType::ClaudeCode, None),
    );
    let mut device2_agents = AgentRegistry::default();
    device2_agents
        .agents
        .insert("agent2".into(), AgentEntry::new(AgentType::Codex, None));

    // ── Step 4: stand up a forward invoker that bridges the two
    //    devices' "local" registries. In a real deployment the
    //    invoker hands signed envelopes to the daemon's bridge
    //    which calls federation.forward_invoke; the hub looks up
    //    target's host node_id and routes. For this product-flow
    //    test we collapse all that into an in-process closure
    //    that calls device2's invoke handler directly when
    //    target is agent2's URA.
    //
    //    The remote ability handler reads args.message and
    //    reverses it — same shape as the axon-internal chat e2e.
    let device2_chat_handler: easynet_cli::daemon::ability::dispatch::LocalRpcHandler =
        Arc::new(|args: Value| {
            let msg = args
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("(missing)")
                .to_string();
            let reversed: String = msg.chars().rev().collect();
            Ok(json!({"echoed": reversed, "from": "agent2"}))
        });
    // device2's registry needs the chat handler installed and the
    // self-invoke wired so `agent2.chat` is dispatchable.
    let device2_dispatch_for_router = {
        let runtime = LocalRuntime::new();
        let mut reg = AxonAbilityCatalog::new_with_runtime(Arc::clone(&runtime));
        reg.register_rpc("agent2.chat", Arc::clone(&device2_chat_handler));
        let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
        let h_for_register = Arc::clone(&handle);
        let agents_clone = device2_agents.clone();
        invoke_ability::register_for_agent(
            &mut reg,
            "agent2".into(),
            move || agents_clone.clone(),
            h_for_register,
        );
        let arc_reg = Arc::new(reg);
        if handle.set(Arc::clone(&arc_reg)).is_err() {
            panic!("handle already set on device2");
        }
        ProductRuntimeHarness::new(arc_reg)
    };

    // Forward router: when device1 invokes a remote URA whose
    // realm is silan.localhost (our tenant), find the target's
    // ability handler on the matching device. v1 simplification:
    // we only know about agent2 → device2's registry. Production
    // looks the target up in the hub directory and dispatches
    // through the gRPC forward channel.
    let device2_uri_clone = device2_uri.clone();
    fwd::set_test_knower(move |target_ura: &str| target_ura == device2_uri_clone);
    let device2_for_router = device2_dispatch_for_router.clone();
    fwd::set_test_router(move |target_ura, ability, args| {
        // We get the bare ability name (e.g. "chat") from
        // invoke_ability — synthesise the qualified `agent2.chat`
        // before invoking device2's runtime, just like the real
        // forward_invoke does on the remote shard.
        let qualified = format!("agent2.{ability}");
        let _ = target_ura;
        device2_for_router.invoke_rpc(&qualified, args)
    });

    // ── Step 5: build device1's invoke pipeline. agent1 calls
    //    `<agent>.invoke(ability=<device-owned chat Ability URA>,
    //    args={message: "hello"})`. The dispatch derives the
    //    target device URA and owner-local public ability from the
    //    Ability URA, recognises the federation target, asks the
    //    test forward invoker, gets back the reversed message, and
    //    wraps it in the standard invoke envelope. End-to-end.
    let agent1_invoke = build_registry_with_invoke("agent1", device1_agents);
    let device2_chat_ability_ura =
        format!("easynet:///r/{tenant}/ability/device.{device2_node_id}.chat");
    let resp = agent1_invoke
        .invoke_rpc(
            "agent1.invoke",
            json!({
                "ability_ura": device2_chat_ability_ura,
                "args":    {"message": "hello cross-device"},
            }),
        )
        .unwrap();

    // Pin the wire shape: target URA, canonical Ability URA, and
    // the result matches what device2's handler computed.
    // fulfilled_by = federation_forward confirms the call took the
    // cross-device path, not the local-registry path.
    assert_eq!(resp["target"], device2_uri);
    assert_eq!(resp["qualified_name"], device2_chat_ability_ura);
    assert_eq!(resp["fulfilled_by"], "federation_forward");
    assert_eq!(resp["result"]["echoed"], "ecived-ssorc olleh");
    assert_eq!(resp["result"]["from"], "agent2");

    // Caller never registered agent1 in its own registry, so
    // tenant_id pinning at the device level matches what the
    // real "join → register agent" sequence produces.
    let _ = device1_uri;

    fwd::clear_test_routing();
}

#[test]
fn unknown_remote_target_falls_through_to_typed_error() {
    let _g = product_test_lock();

    // device1 is joined; agent1 is registered. The router knows
    // about agent2's URA but the caller asks for a different
    // (un-advertised) URA. Dispatch must surface
    // target_not_registered, not silently succeed.
    let mut device1_agents = AgentRegistry::default();
    device1_agents.agents.insert(
        "agent1".into(),
        AgentEntry::new(AgentType::ClaudeCode, None),
    );

    fwd::set_test_knower(|target| target == "easynet:///r/silan.localhost/device/known-only");
    fwd::set_test_router(|_t, _a, _x| {
        // Should not be called: knower rejects the target first.
        panic!("router called for unknown target");
    });

    let agent1_invoke = build_registry_with_invoke("agent1", device1_agents);
    let err = agent1_invoke
        .invoke_rpc(
            "agent1.invoke",
            json!({
                "ability_ura": "easynet:///r/silan.localhost/ability/device.never-registered.chat",
                "args":    {"message": "hi"},
            }),
        )
        .unwrap_err();
    assert!(
        format!("{err}").contains("target_not_registered"),
        "expected target_not_registered, got: {err}"
    );

    fwd::clear_test_routing();
}

#[test]
fn agent1_can_invoke_local_agent_without_federation_path() {
    let _g = product_test_lock();

    // Same device, two co-located agents. agent1 invokes
    // agent2.summarize on the same device — must NOT go through
    // the forward router. Distinguished by `fulfilled_by:
    // registry_dispatch` in the response envelope.
    let mut shared_agents = AgentRegistry::default();
    shared_agents.agents.insert(
        "agent1".into(),
        AgentEntry::new(AgentType::ClaudeCode, None),
    );
    shared_agents
        .agents
        .insert("agent2".into(), AgentEntry::new(AgentType::Codex, None));

    fwd::set_test_router(|_t, _a, _x| panic!("forward router must not be called for local target"));

    let runtime = LocalRuntime::new();
    let mut reg = AxonAbilityCatalog::new_with_runtime(Arc::clone(&runtime));
    let summarize: easynet_cli::daemon::ability::dispatch::LocalRpcHandler =
        Arc::new(|args: Value| {
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            Ok(json!({"summary": format!("{} chars", text.len())}))
        });
    reg.register_rpc("agent2.summarize", summarize);
    let handle: Arc<OnceLock<Arc<AxonAbilityCatalog>>> = Arc::new(OnceLock::new());
    let h_for_register = Arc::clone(&handle);
    let agents_clone = shared_agents.clone();
    invoke_ability::register_for_agent(
        &mut reg,
        "agent1".into(),
        move || agents_clone.clone(),
        h_for_register,
    );
    let arc_reg = Arc::new(reg);
    if handle.set(Arc::clone(&arc_reg)).is_err() {
        panic!("handle set once");
    }

    let harness = ProductRuntimeHarness::new(arc_reg);
    let resp = harness
        .invoke_rpc(
            "agent1.invoke",
            json!({
                "ability_ura": "easynet:///r/silan.localhost/ability/user-1.agent2.summarize",
                "args":    {"text": "twenty-four characters!!"},
            }),
        )
        .unwrap();
    assert_eq!(resp["target"], "agent2");
    assert_eq!(
        resp["qualified_name"],
        "easynet:///r/silan.localhost/ability/user-1.agent2.summarize"
    );
    assert_eq!(resp["fulfilled_by"], "registry_dispatch");
    assert_eq!(resp["result"]["summary"], "24 chars");

    fwd::clear_test_routing();
}
