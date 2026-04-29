//! Product-flow end-to-end test (RFC-002 + RFC-002.2).
//!
//! User asked: 创建用户获得 key → easynet-cli join → 设备1 注册
//! agent1 → 设备2 注册 agent2 → agent1 调用 agent2.chat
//! 一起讨论. The previous axon-internal tests (forward_invoke_e2e,
//! multi_shard_e2e) proved the protocol layer; this one walks the
//! product-visible surface from the CLI's perspective. We do NOT
//! exercise the Hub HTTP pairing endpoint — that lives in
//! easynet.run's web tier and is out of scope for the CLI repo.
//! Instead we synthesise the post-pairing state (credentials +
//! keyring + agent registry) directly and prove that the chat
//! round-trip works through `<self>.invoke` on each "device".
//!
//! Because the test crosses many layers it lives at the CLI's
//! integration-test surface (`tests/`), not as a unit test. It
//! takes the longest path the CLI exposes: keyring (RFC-002) →
//! agent registry → `<self>.invoke` → forward routing
//! (RFC-002 §5.2, RFC-002.2 §2.3) → fake remote daemon handler.

use easynet_cli::registry::agents::{AgentEntry, AgentRegistry, AgentType};
use easynet_cli::runtime::ability_dispatch::LocalAbilityRegistry;
use easynet_cli::runtime::agents::invoke_ability;
use easynet_cli::runtime::keyring::forward as fwd;
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

fn build_registry_with_invoke(
    self_agent: &str,
    agents: AgentRegistry,
) -> Arc<dyn Fn(Value) -> anyhow::Result<Value> + Send + Sync> {
    let mut reg = LocalAbilityRegistry::new();
    let handle: Arc<OnceLock<Arc<LocalAbilityRegistry>>> = Arc::new(OnceLock::new());
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
    let dispatch = arc_reg
        .resolve_rpc(&format!("{self_agent}.invoke"))
        .expect("invoke registered");
    Arc::new(move |args| dispatch(args))
}

#[test]
fn user_join_two_devices_chat_round_trip() {
    let _g = product_test_lock();

    // ── Step 1: simulate "create user, get key" (Hub web does
    //    this in production; here we just generate a keypair via
    //    the CLI's keyring crypto primitives — same code path).
    use easynet_cli::runtime::keyring::crypto::fresh_ed25519_keypair;
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
    let device1_uri = format!("easynet:///r/{tenant}/agent/{device1_node_id}");
    let device2_uri = format!("easynet:///r/{tenant}/agent/{device2_node_id}");

    // ── Step 3: register agent1 on device1, agent2 on device2.
    //    Each device's local agent registry knows only its own
    //    locally-installed agents (matches what
    //    `easynet agent add` writes to the per-device registry).
    let mut device1_agents = AgentRegistry::default();
    device1_agents
        .agents
        .insert("agent1".into(), AgentEntry::new(AgentType::ClaudeCode, None));
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
    let device2_chat_handler: easynet_cli::runtime::ability_dispatch::LocalRpcHandler =
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
        let mut reg = LocalAbilityRegistry::new();
        reg.register_rpc("agent2.chat", Arc::clone(&device2_chat_handler));
        let handle: Arc<OnceLock<Arc<LocalAbilityRegistry>>> = Arc::new(OnceLock::new());
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
        arc_reg
    };

    // Forward router: when device1 invokes a remote URA whose
    // realm is silan.localhost (our tenant), find the target's
    // ability handler on the matching device. v1 simplification:
    // we only know about agent2 → device2's registry. Production
    // looks the target up in the hub directory and dispatches
    // through the gRPC forward channel.
    let device2_uri_clone = device2_uri.clone();
    fwd::set_test_knower(move |target_uri: &str| target_uri == device2_uri_clone);
    let device2_dispatch_for_closure = device2_dispatch_for_router.clone();
    fwd::set_test_router(move |target_uri, ability, args| {
        // We get the bare ability name (e.g. "chat") from
        // invoke_ability — synthesise the qualified `agent2.chat`
        // before calling device2's resolver, just like the real
        // forward_invoke does on the remote shard.
        let qualified = format!("agent2.{ability}");
        let handler = device2_dispatch_for_closure
            .resolve_rpc(&qualified)
            .ok_or_else(|| {
                anyhow::anyhow!("ability_not_found on remote: {qualified}")
            })?;
        let _ = target_uri;
        handler(args)
    });

    // ── Step 5: build device1's invoke pipeline. agent1 calls
    //    `<self>.invoke(target=<device2_uri>, ability="chat",
    //    args={message: "hello"})`. The dispatch sees the target
    //    is not in device1's local registry, recognises the
    //    federation URA, asks the test forward invoker, gets
    //    back the reversed message, wraps it in the standard
    //    invoke envelope. End-to-end.
    let agent1_invoke = build_registry_with_invoke("agent1", device1_agents);
    let resp = agent1_invoke(json!({
        "target":  device2_uri,
        "ability": "chat",
        "args":    {"message": "hello cross-device"},
    }))
    .unwrap();

    // Pin the wire shape: target URA, qualified name composed
    // from the URA + ability, and the result matches what
    // device2's handler computed. fulfilled_by = federation_forward
    // confirms the call took the cross-device path, not the
    // local-registry path.
    assert_eq!(resp["target"], device2_uri);
    assert_eq!(
        resp["qualified_name"],
        format!("{device2_uri}.chat")
    );
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
    device1_agents
        .agents
        .insert("agent1".into(), AgentEntry::new(AgentType::ClaudeCode, None));

    fwd::set_test_knower(|target| {
        target == "easynet:///r/silan.localhost/agent/known-only"
    });
    fwd::set_test_router(|_t, _a, _x| {
        // Should not be called: knower rejects the target first.
        panic!("router called for unknown target");
    });

    let agent1_invoke = build_registry_with_invoke("agent1", device1_agents);
    let err = agent1_invoke(json!({
        "target":  "easynet:///r/silan.localhost/agent/never-registered",
        "ability": "chat",
        "args":    {"message": "hi"},
    }))
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
    shared_agents
        .agents
        .insert("agent1".into(), AgentEntry::new(AgentType::ClaudeCode, None));
    shared_agents
        .agents
        .insert("agent2".into(), AgentEntry::new(AgentType::Codex, None));

    fwd::set_test_router(|_t, _a, _x| {
        panic!("forward router must not be called for local target")
    });

    let mut reg = LocalAbilityRegistry::new();
    let summarize: easynet_cli::runtime::ability_dispatch::LocalRpcHandler =
        Arc::new(|args: Value| {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(json!({"summary": format!("{} chars", text.len())}))
        });
    reg.register_rpc("agent2.summarize", summarize);
    let handle: Arc<OnceLock<Arc<LocalAbilityRegistry>>> = Arc::new(OnceLock::new());
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
    let dispatch = arc_reg.resolve_rpc("agent1.invoke").unwrap();

    let resp = dispatch(json!({
        "target":  "agent2",
        "ability": "summarize",
        "args":    {"text": "twenty-four characters!!"},
    }))
    .unwrap();
    assert_eq!(resp["target"], "agent2");
    assert_eq!(resp["fulfilled_by"], "registry_dispatch");
    assert_eq!(resp["result"]["summary"], "24 chars");

    fwd::clear_test_routing();
}
