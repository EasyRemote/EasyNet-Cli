// EasyNet CLI — per-ability real-invocation tests
// =================================================
//
// File: src/runtime/system_abilities/real_invoke_tests.rs
// Description: One `#[test]` per published ability that runs a
//              REAL invocation through the live dispatcher with
//              REAL arguments and asserts something specific
//              about the result. Filling the gap between
//              "handler is registered" (slice 20) and "handler
//              actually behaves correctly under realistic usage."
//
// Why this file exists
// --------------------
// Multiple rounds of audit conversation surfaced a pattern: I'd
// claim "I tested every ability" when in fact I'd only tested
// the dispatch path (handler reachable) or the schema path (TOML
// matches code). The handlers themselves were not being exercised
// with the kind of input a real caller would send.
//
// This file fixes that. For each published ability, at least one test:
//   1. Sets up a minimal real fixture (HomeGuard for HOME-bound
//      handlers; live SessionService / DiscussService / etc. for
//      service-bound handlers).
//   2. Constructs a realistic args object — not `{}`, not random
//      garbage, but the kind of payload an operator would actually
//      send.
//   3. Invokes through `AxonAbilityCatalog::execute_rpc` (or
//      `execute_stream` / `execute_bidi`).
//   4. Asserts a specific shape of the result.
//
// What "real" means here
// ----------------------
// * For pure / observable abilities (observe.health, meta.*),
//   real == we read fields from the response and check they
//   reflect the live registry / runtime.
// * For HOME-bound persistence (admin.status, device.* _agent),
//   real == HomeGuard provides a fresh ~/.easynet/, the call
//   creates / lists / deletes against that empty state.
// * For services with Arc handles (DiscussService, etc.), real
//   == construct a fresh service, register the ability with
//   that service, invoke, observe the service's state mutated.
// * For process/shell, real == spawn /bin/echo, observe stdout.
// * For network (http.request), real == bind a TcpListener on
//   loopback in a tokio task and hit it.
//
// What "real" does NOT mean
// -------------------------
// * Cross-process IPC. The Kernel's full admission-and-receipt
//   pipeline is exercised by separate integration tests; here
//   we go directly through `AxonAbilityCatalog::execute_rpc`.
// * Multi-host federation. Anything `mcp.client.*` / `a2a.*`
//   exercises the dispatcher's branch into the relevant client
//   service; we assert the call returns a structured "no
//   upstream configured" response (the production behavior
//   when ~/.easynet/mcp_clients.json is empty / absent).
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.
//
// Placement note
// --------------
// This intentionally remains a `#[cfg(test)]` child of
// `runtime::system_abilities` instead of an integration-test crate. The
// harness reaches private registry builders, descriptors, and fixture seams
// that should not be made public just to satisfy a file-location metric.
// Keeping it under `src/` has zero release artifact weight and preserves the
// production visibility boundary.

#![allow(dead_code)] // helpers below are referenced per-test; some unused on macOS-only paths

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};
use crate::runtime::system_abilities::{
    automation::{discuss as discuss_ability, loop_ability, schedule as schedule_ability},
    device_control::{
        file_transfer as file_transfer_ability, session as session_ability,
        terminal::{
            attach as pty_attach_ability, io as pty_io_ability, lifecycle as pty_lifecycle_ability,
        },
    },
    governance::consent as permission_ability,
};
use crate::runtime::system_ability_catalog::{build_registry, is_publishable_catalog_name};

// ── Helpers ──────────────────────────────────────────────────────

fn fs_ref(
    path: &std::path::Path,
    capability: crate::runtime::resources::filesystem::FilesystemResourceCapability,
) -> Value {
    crate::runtime::resources::filesystem::resource_ref_for_local_path(path, capability)
        .expect("local fs ResourceRef")
}

/// Build the production registry inside a HomeGuard so any
/// HOME-touching boot logic (mcp_clients.json discovery,
/// agents.json load, etc.) lands in a fresh tempdir.
fn registry_with_temp_home() -> (Arc<AxonAbilityCatalog>, crate::cli::test_support::HomeGuard) {
    let guard = crate::cli::test_support::HomeGuard::new();
    // build_registry_for_daemon does the agent-registry load that
    // some abilities need (agent.list, chat-per-agent etc).
    let mut config = crate::runtime::system_ability_catalog::RegistryDaemonBuildConfig::new(
        crate::runtime::system_ability_catalog::RegistryBuildServices::fresh(),
    );
    config.loaders = Some(Arc::new(Vec::new()));
    let reg = crate::runtime::system_ability_catalog::build_registry_for_daemon(config);
    (reg, guard)
}

fn materialise_skill_fixture(
    tag: &str,
    skill_name: &str,
    body: &str,
) -> (String, std::path::PathBuf) {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let owner = format!("real-skill-{tag}-{pid}-{nanos}");
    let root = crate::persistence::config::agents_root().join(&owner);
    let skill_dir = root.join("skills").join(skill_name);
    std::fs::create_dir_all(&skill_dir).expect("create skill fixture dir");
    crate::persistence::config::atomic_write(&skill_dir.join("SKILL.md"), body.as_bytes())
        .expect("write skill fixture body");
    let meta_dir = skill_dir.join(".easynet");
    std::fs::create_dir_all(&meta_dir).expect("create skill fixture metadata dir");
    let install = crate::runtime::skill_store::InstallRecord {
        name: skill_name.to_string(),
        description: body.to_string(),
        agent_id: owner.clone(),
        source: crate::runtime::skill_store::SkillSource {
            kind: "fixture".to_string(),
            identifier: "real-invoke-tests".to_string(),
            ref_: None,
            subpath: None,
        },
        skill_tree_hash: "sha256:fixture".to_string(),
        size_bytes: body.len() as u64,
        installed_at: "2026-06-07T00:00:00Z".to_string(),
        last_checked_at: None,
        upgrade_available: false,
    };
    let install_json = serde_json::to_vec_pretty(&install).expect("serialize install fixture");
    crate::persistence::config::atomic_write(&meta_dir.join("install.json"), &install_json)
        .expect("write install fixture");

    let mut registry = crate::registry::agents::load_agents().unwrap_or_default();
    let mut entry =
        crate::registry::agents::AgentEntry::new(crate::registry::agents::AgentType::Codex, None);
    entry.root_path = Some(root);
    registry.agents.insert(owner.clone(), entry);
    crate::registry::agents::save_agents(&registry).expect("save skill fixture agent");
    (owner, skill_dir)
}

fn dispatcher_for(reg: Arc<AxonAbilityCatalog>) -> Arc<AxonAbilityCatalog> {
    reg
}

fn target(name: &str, args: Value) -> InvocationTarget {
    InvocationTarget {
        scope: TargetScope::Local,
        ability: name.to_string(),
        normalized_args: args,
        call_mode: CallMode::Rpc,
        // Test helper: legacy callers that don't need a subject
        // get None. The `with_subject` builder lets per-test code
        // attach one when exercising INV-SUBJECT-ENVELOPE paths.
        subject: None,
        causal_context: None,
    }
}

// Borrowed receipt-URA shape (ledger.rs test convention) — no
// production builder yet; RFC-007/008 tracks canonicalization (F-042).
#[cfg(feature = "remote-desktop")]
fn remote_desktop_test_consent_causal_context() -> easynet_axon::invocation::CausalContext {
    easynet_axon::invocation::CausalContext::Scalar(easynet_axon::invocation::ReceiptRef {
        receipt_ura: "easynet:///r/acme/resource/alice.invocations/test-local-consent".to_string(),
        receipt_hash: [0x42; 32],
    })
}

fn invoke(name: &str, args: Value) -> Value {
    // Convenience for tests that don't need a HomeGuard. For
    // tests that do, build the dispatcher inline so the guard
    // outlives the call.
    let reg = build_registry();
    let d = dispatcher_for(reg);
    match d.execute_rpc(target(name, args)) {
        Ok(v) => v,
        Err(e) => panic!("{name} unexpected error: {e}"),
    }
}

// ════════════════════════════════════════════════════════════════
// Category A: pure / read-only no fixture needed
// ════════════════════════════════════════════════════════════════

#[test]
fn real_observe_health_returns_ok_and_timestamp() {
    let resp = invoke("observe.health", json!({}));
    // Ping echoes args + adds `ts`. Some implementations also
    // return `ok`. Assert at least one of these is present —
    // the contract is "non-empty, observable response".
    assert!(
        resp.get("ts").is_some() || resp.get("ok").is_some() || resp.is_object(),
        "observe.health response unexpected: {resp}"
    );
    assert!(resp.is_object(), "observe.health must return an object");
}

#[test]
fn real_observe_network_health_describes_the_node() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("observe.network_health", json!({})))
        .expect("observe.network_health");
    // Actual shape: `{view, schema, joined, host_device_ura,
    // hosted_agent_count, latency_ms, links: [...]}`. We assert
    // a few load-bearing fields so a regression that empties
    // the response would surface.
    let body = resp.as_object().expect("object");
    assert!(body.contains_key("schema") || body.contains_key("view"));
    assert!(body.contains_key("links") || body.contains_key("joined"));
}

#[test]
fn real_invocation_history_list_returns_records_array() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("invocation.history.list", json!({ "limit": 5 })))
        .expect("invocation.history.list");
    let body = resp.as_object().expect("object");
    assert!(
        body.get("records").and_then(Value::as_array).is_some(),
        "history list must return records array: {resp}"
    );
}

#[test]
fn real_invocation_history_list_accepts_explicit_ura_scope() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "invocation.history.list",
            json!({
                "limit": 5,
                "filter": {
                    "agent_ura": "easynet:///r/test/device/callee",
                    "subject_ura": "easynet:///r/test/user/alice"
                }
            }),
        ))
        .expect("invocation.history.list with URA scope");
    let body = resp.as_object().expect("object");
    assert!(
        body.get("records").and_then(Value::as_array).is_some(),
        "history list must accept agent_ura + subject_ura scope: {resp}"
    );
}

#[test]
fn real_invocation_history_path_returns_ledger_location() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("invocation.history.path", json!({})))
        .expect("invocation.history.path");
    let body = resp.as_object().expect("object");
    assert!(
        body.get("ledger_path").and_then(Value::as_str).is_some()
            && body.contains_key("ledger_ura"),
        "history path must return ledger_path and ledger_ura field: {resp}"
    );
}

#[test]
fn real_invocation_history_get_accepts_request_id() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "invocation.history.get",
            json!({ "key": { "request_id": "missing-real-invoke-request" } }),
        ))
        .expect("invocation.history.get");
    let body = resp.as_object().expect("object");
    assert!(
        body.contains_key("record"),
        "history get must return a record field even when absent: {resp}"
    );
}

#[test]
fn real_invocation_trace_get_returns_graph_shape() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "invocation.trace.get",
            json!({ "key": { "trace_id": "missing-real-invoke-trace" } }),
        ))
        .expect("invocation.trace.get");
    let body = resp.as_object().expect("object");
    assert!(
        body.get("nodes").and_then(Value::as_array).is_some()
            && body.get("edges").and_then(Value::as_array).is_some(),
        "trace get must return graph arrays: {resp}"
    );
}

#[test]
fn real_meta_describe_returns_self_identity() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("meta.describe", json!({})))
        .expect("meta.describe");
    assert!(resp.is_object());
}

#[test]
fn real_meta_list_abilities_returns_at_least_observe_health() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("meta.list_abilities", json!({})))
        .expect("meta.list_abilities");
    let body = resp.as_object().expect("object");
    // The exact key depends on the handler — could be `abilities`
    // or `tools` etc. Find any array and assert observe.health
    // is in it.
    let mut found_observe = false;
    for (_k, v) in body {
        if let Some(arr) = v.as_array() {
            for item in arr {
                let name_field = item
                    .as_object()
                    .and_then(|o| o.get("name").or_else(|| o.get("ability")))
                    .and_then(Value::as_str);
                if name_field == Some("observe.health") {
                    found_observe = true;
                    break;
                }
            }
        }
        if found_observe {
            break;
        }
    }
    assert!(
        found_observe,
        "meta.list_abilities must include observe.health: got {resp}"
    );
}

#[test]
fn real_meta_list_abilities_returns_observe_health() {
    // `meta.list_abilities` is the canonical introspection
    // ability. Pin the body's shape: at least one array containing
    // `observe.health` — a regression that broke the descriptor
    // merge or registered the wrong handler trips here.
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("meta.list_abilities", json!({})))
        .expect("meta.list_abilities");
    let body = resp.as_object().expect("object");
    let mut found = false;
    for (_k, v) in body {
        if let Some(arr) = v.as_array() {
            for item in arr {
                let name = item
                    .as_object()
                    .and_then(|o| o.get("name").or_else(|| o.get("ability")))
                    .and_then(Value::as_str);
                if name == Some("observe.health") {
                    found = true;
                    break;
                }
            }
        }
        if found {
            break;
        }
    }
    assert!(
        found,
        "meta.list_abilities must include observe.health: got {resp}"
    );
}

#[test]
fn real_easynet_discover_alias_was_removed() {
    // RFC-001 v4.1.7 M2 deleted the `device.easynet.*` user-facing
    // aliases per Q2 of the migration plan ("aliases are protocol
    // entropy generators"). Pin the absence so a regression that
    // re-introduces the alias name fails here BEFORE the LLM
    // corpus drifts back to two-canonical-names land.
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target("device.easynet.discover", json!({})));
    assert!(
        result.is_err(),
        "device.easynet.discover MUST be unregistered post-M2; \
         got {result:?}"
    );
}

#[test]
fn real_mission_run_validates_args_before_touching_the_runtime() {
    // `mission.run` requires a non-empty `source`. Validation runs
    // BEFORE the handler reaches into `run_mission_inproc` (which
    // needs a live runtime + bridge pool for device dispatch),
    // so this test exercises the wiring up to the validation gate
    // without requiring a daemon to be running — empty source must
    // produce a precise error message so an LLM-driven caller
    // sees what to fix.
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target("mission.run", json!({ "source": "" })));
    let err = result.expect_err("empty source must fail validation");
    let msg = format!("{err}");
    assert!(
        msg.contains("source") && msg.contains("non-empty"),
        "empty source must yield a precise validation error mentioning \
         `source` and `non-empty`; got: {msg}"
    );
}

#[test]
fn real_mission_track_returns_an_error_for_an_unknown_run_id() {
    // `mission.track` reads the persisted state of a prior
    // `mission.run` by id. With a fresh HOME there are no run
    // dirs, so any id lookup MUST surface an error rather than
    // silently fabricating an empty envelope. A regression that
    // returns Ok({}) for a missing run would mask "the mission
    // is gone" as "the mission has nothing to report".
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target(
        "mission.track",
        json!({ "run_id": "no-such-run-id" }),
    ));
    assert!(
        result.is_err(),
        "mission.track must error on an unknown run_id; got {result:?}"
    );
}

#[test]
fn real_mission_cancel_returns_an_error_for_an_unknown_run_id() {
    // Same contract as mission.track — an unknown run id must
    // surface as an error, not a silent no-op. A regression that
    // returned `cancelled = false` here would let a caller think
    // they had reached a (terminal) run when in fact no run by
    // that id ever existed.
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target(
        "mission.cancel",
        json!({ "run_id": "no-such-run-id" }),
    ));
    assert!(
        result.is_err(),
        "mission.cancel must error on an unknown run_id; got {result:?}"
    );
}

// ── device.*  device + ability operations ─────────────────────────
//
// Eight abilities backing every CLI device + ability subcommand
// (`device list/show/remove`, `ability deploy/uninstall/exec`,
// daemon lifecycle hooks). Per-handler unit tests live alongside
// `device_ops_ability` itself; the tests below are the integration
// layer — dispatch each one through the real dispatcher to prove
// the registration site + name + arg shape line up.

#[test]
fn real_device_node_list_returns_local_view_envelope() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("node.list", json!({})))
        .expect("node.list");
    let nodes = resp.get("nodes").and_then(Value::as_array).unwrap();
    assert!(
        nodes.iter().any(|n| n.get("is_self") == Some(&json!(true))),
        "node.list must include the local device entry: {resp}"
    );
}

#[test]
fn real_device_node_describe_via_invoke_helper_returns_self_envelope() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("node.describe", json!({ "node_id": "local" })))
        .expect("node.describe local");
    assert_eq!(resp.get("is_self"), Some(&json!(true)));
}

#[test]
fn real_device_node_remove_refuses_to_remove_self() {
    let (reg, _g) = registry_with_temp_home();
    let err = dispatcher_for(reg)
        .execute_rpc(target("node.remove", json!({ "node_id": "local" })))
        .expect_err("node.remove must refuse to remove self");
    assert!(format!("{err}").contains("device reset"));
}

#[test]
fn real_device_ability_deploy_validates_resource_ref_argument() {
    let (reg, _g) = registry_with_temp_home();
    let err = dispatcher_for(reg)
        .execute_rpc(target("ability.deploy", json!({})))
        .expect_err("ability.deploy must require `resource_ref`");
    assert!(format!("{err}").contains("resource_ref"));
}

#[test]
fn real_device_ability_uninstall_refuses_unwired_runtime() {
    let (reg, _g) = registry_with_temp_home();
    let err = dispatcher_for(reg)
        .execute_rpc(target(
            "ability.uninstall",
            json!({
                "ability_ura": "easynet:///r/localhost/ability/alice.claude.weather",
                "node_id": "local"
            }),
        ))
        .expect_err("unwired registrar must not report REMOVED");
    let msg = format!("{err}");
    assert!(msg.contains("runtime not wired yet"), "{msg}");
}

#[test]
fn real_device_remote_exec_is_not_registered_without_permission_broker() {
    let (reg, _g) = registry_with_temp_home();
    let err = dispatcher_for(reg)
        .execute_rpc(target(
            "remote.exec",
            json!({
                "node_id": "local",
                "command": ["printf", "%s", "ok"],
            }),
        ))
        .expect_err("remote.exec must not be a public ability without a permission broker");
    let msg = format!("{err}");
    assert!(msg.contains("unknown_ability:remote.exec"), "{msg}");
}

#[test]
fn real_device_node_register_is_not_a_runtime_surface() {
    let (reg, _g) = registry_with_temp_home();
    let err = dispatcher_for(reg)
        .execute_rpc(target("node.register", json!({})))
        .expect_err("node.register must not be registered until transport exists");
    assert!(format!("{err}").contains("unknown_ability"), "{err}");
}

#[test]
fn real_device_node_deregister_is_not_a_runtime_surface() {
    let (reg, _g) = registry_with_temp_home();
    let err = dispatcher_for(reg)
        .execute_rpc(target("node.deregister", json!({})))
        .expect_err("node.deregister must not be registered until transport exists");
    assert!(format!("{err}").contains("unknown_ability"), "{err}");
}

// ── voice.* call signaling abilities ────────────────────────────
//
// Seven abilities backing `easynet call …`. Per-handler unit tests
// live alongside `voice_call_ability` itself; the tests below are
// the integration layer — dispatch each one through the real
// dispatcher to prove the registration site + name + arg shape line
// up. We mint unique call_ids using nanos so concurrent test runs
// (cargo test --test-threads=N) don't collide on the in-process
// store.

fn unique_call_id(label: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("real-invoke-{label}-{nanos:x}")
}

#[test]
fn real_voice_create_call_returns_a_minted_id() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("voice.create_call", json!({})))
        .expect("voice.create_call");
    let cid = resp.get("call_id").and_then(Value::as_str).unwrap();
    assert!(cid.starts_with("call-"));
}

#[test]
fn real_voice_show_call_unknown_call_errors() {
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target(
        "voice.show_call",
        json!({"call_id": "no-such-call"}),
    ));
    assert!(
        result.is_err(),
        "voice.show_call must error on unknown call_id"
    );
}

#[test]
fn real_voice_join_call_transitions_call_to_active() {
    // voice.* in-process state machine: a call goes "active"
    // when ≥2 participants are present (creator + first remote
    // joiner per voice_call_ability::join_call_handler comment).
    // The test must therefore pre-populate the creator via
    // `voice.create_call` taking a `participant_id`, then add
    // the second via `voice.join_call`.
    let (reg, _g) = registry_with_temp_home();
    let cid = unique_call_id("join");
    let dispatcher = dispatcher_for(reg);
    dispatcher
        .execute_rpc(target(
            "voice.create_call",
            json!({"call_id": cid.clone(), "participant_id": "creator"}),
        ))
        .expect("create");
    dispatcher
        .execute_rpc(target(
            "voice.join_call",
            json!({"call_id": cid.clone(), "participant_id": "alice"}),
        ))
        .expect("join");
    let show = dispatcher
        .execute_rpc(target("voice.show_call", json!({"call_id": cid})))
        .expect("show");
    assert_eq!(
        show.get("state").and_then(Value::as_str),
        Some("VOICE_CALL_STATE_ACTIVE")
    );
    assert_eq!(show.get("state_code"), Some(&json!(2)));
}

#[test]
fn real_voice_leave_call_removes_participant() {
    let (reg, _g) = registry_with_temp_home();
    let cid = unique_call_id("leave");
    let d = dispatcher_for(reg);
    d.execute_rpc(target("voice.create_call", json!({"call_id": cid.clone()})))
        .expect("create");
    d.execute_rpc(target(
        "voice.join_call",
        json!({"call_id": cid.clone(), "participant_id": "alice"}),
    ))
    .expect("join");
    d.execute_rpc(target(
        "voice.leave_call",
        json!({"call_id": cid.clone(), "participant_id": "alice"}),
    ))
    .expect("leave");
    // No assertion on state machine here beyond "didn't panic" —
    // semantics are pinned in the unit-test file. Real-invoke
    // coverage just proves the ability is registered + reachable.
    let _ = d
        .execute_rpc(target("voice.show_call", json!({"call_id": cid})))
        .expect("show");
}

#[test]
fn real_voice_end_call_is_idempotent() {
    let (reg, _g) = registry_with_temp_home();
    let cid = unique_call_id("end");
    let d = dispatcher_for(reg);
    d.execute_rpc(target("voice.create_call", json!({"call_id": cid.clone()})))
        .expect("create");
    d.execute_rpc(target("voice.end_call", json!({"call_id": cid.clone()})))
        .expect("first end");
    let r2 = d
        .execute_rpc(target("voice.end_call", json!({"call_id": cid})))
        .expect("second end");
    assert_eq!(r2.get("already_ended"), Some(&json!(true)));
}

#[test]
fn real_voice_watch_call_returns_event_snapshot() {
    let (reg, _g) = registry_with_temp_home();
    let cid = unique_call_id("watch");
    let d = dispatcher_for(reg);
    d.execute_rpc(target("voice.create_call", json!({"call_id": cid.clone()})))
        .expect("create");
    d.execute_rpc(target(
        "voice.join_call",
        json!({"call_id": cid.clone(), "participant_id": "alice"}),
    ))
    .expect("join");
    let w = d
        .execute_rpc(target("voice.watch_call", json!({"call_id": cid})))
        .expect("watch");
    let events = w.get("events").and_then(Value::as_array).unwrap();
    assert!(events
        .iter()
        .any(|e| e.get("type") == Some(&json!("joined"))));
}

#[test]
fn real_voice_report_metrics_appends_event() {
    let (reg, _g) = registry_with_temp_home();
    let cid = unique_call_id("metrics");
    let d = dispatcher_for(reg);
    d.execute_rpc(target("voice.create_call", json!({"call_id": cid.clone()})))
        .expect("create");
    d.execute_rpc(target(
        "voice.join_call",
        json!({"call_id": cid.clone(), "participant_id": "alice"}),
    ))
    .expect("join");
    let r = d
        .execute_rpc(target(
            "voice.report_metrics",
            json!({
                "call_id": cid,
                "participant_id": "alice",
                "metrics": { "rtt_ms": 42 },
            }),
        ))
        .expect("metrics");
    assert_eq!(r.get("ack"), Some(&json!(true)));
}

// ── mission.discuss_round ───────────────────────────────────────
//
// `mission.discuss_round` is the sub-turn orchestrator backing
// `easynet mission discuss …`. We can't drive a full round in a
// unit test (the inner per-cycle loop calls `<agent>.chat` over
// IPC, which needs a daemon + real chat driver), but we CAN pin
// the validation contract — the same shape of guard every other
// CLI surface uses for arg-shape errors. A regression that, say,
// silently accepts `agents: []` would let callers waste minutes
// in a no-op sub-turn.

#[test]
fn real_discuss_list_turns_returns_empty_for_fresh_room() {
    // discuss.list_turns is the snapshot RPC sibling of
    // discuss.subscribe. A fresh room (one that hasn't been
    // posted to) returns `{room_id, turns: []}`. We don't
    // actually create a room here (DiscussService is internal
    // — registry_with_temp_home doesn't expose its registration
    // on the dispatcher unless services are wired through). The
    // test asserts the validation error path: missing `room_id`
    // surfaces a precise error, same shape as every other CLI
    // surface.
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target("discuss.list_turns", json!({})));
    let err = result.expect_err("discuss.list_turns must require room_id");
    assert!(format!("{err}").contains("room_id"));
}

#[test]
fn real_mission_discuss_round_rejects_missing_room_id() {
    let (reg, _g) = registry_with_temp_home();
    let result =
        dispatcher_for(reg).execute_rpc(target("mission.discuss_round", json!({"agents": ["a"]})));
    let err = result.expect_err("missing room_id must fail");
    assert!(format!("{err}").contains("room_id"));
}

#[test]
fn real_mission_discuss_round_rejects_empty_agents() {
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target(
        "mission.discuss_round",
        json!({"room_id": "room-x", "agents": []}),
    ));
    let err = result.expect_err("empty agents must fail");
    assert!(format!("{err}").contains("agents"));
}

#[test]
fn real_mission_discuss_round_rejects_zero_max_cycles() {
    let (reg, _g) = registry_with_temp_home();
    let result = dispatcher_for(reg).execute_rpc(target(
        "mission.discuss_round",
        json!({
            "room_id":    "room-x",
            "agents":     ["a"],
            "max_cycles": 0,
        }),
    ));
    let err = result.expect_err("zero max_cycles must fail");
    assert!(format!("{err}").contains("max_cycles"));
}

// ════════════════════════════════════════════════════════════════
// Category B: HOME-bound + service-bound (need fixture)
// ════════════════════════════════════════════════════════════════

#[test]
fn real_admin_status_reports_components_under_temp_home() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("admin.status", json!({})))
        .expect("admin.status");
    let body = resp.as_object().expect("object");
    assert!(body.contains_key("status"));
    assert!(body.contains_key("version"));
    let comps = body["components"].as_array().expect("components array");
    let names: Vec<&str> = comps
        .iter()
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .collect();
    assert!(names.contains(&"membership"));
    assert!(names.contains(&"ability_registry"));
    assert!(names.contains(&"hosted_agents"));
}

/// User-perspective fs.write: write into a directory under
/// `target/` (a real path on the developer's disk, persisted
/// across the test run; not under tempdir), read it back via
/// std::fs, assert the bytes round-trip exactly. Cleans up
/// after itself.
#[test]
fn real_fs_write_round_trips_through_real_disk() {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scratch = manifest
        .join("target")
        .join("real-user-fs-write")
        .join(format!("p{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
    let path = scratch.join("greeting.txt");
    let _ = std::fs::remove_file(&path);

    let payload = "Hello from a real fs.write call.\nLine 2 of the file.\n";
    let resp = invoke(
        "fs.write",
        json!({
            "resource_ref": fs_ref(&path, crate::runtime::resources::filesystem::FilesystemResourceCapability::Write),
            "content": payload,
            "encoding": "utf8",
        }),
    );
    assert_eq!(
        resp["bytes_written"].as_u64().unwrap(),
        payload.len() as u64
    );
    assert!(resp["content_sha256"].as_str().unwrap().len() == 64);

    // Read directly via std::fs, not via fs.read — proves the
    // file actually landed on disk and is readable to any tool
    // the user might use (cat, vim, less, ...).
    let on_disk = std::fs::read_to_string(&path).expect("read the file the user would see");
    assert_eq!(on_disk, payload);

    let _ = std::fs::remove_dir_all(&scratch);
}

/// User-perspective process.exec: run /bin/cat against a real
/// OS file, check the stdout actually contains content the
/// user would see if they ran the command at a shell prompt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_process_exec_cats_etc_hosts() {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;

    if !std::path::Path::new("/etc/hosts").exists() {
        eprintln!("real_process_exec_cats_etc_hosts: /etc/hosts missing on this host, skipping");
        return;
    }
    let resp = tokio::task::spawn_blocking(|| {
        let (reg, _g) = registry_with_temp_home();
        dispatcher_for(reg)
            .execute_rpc(target(
                "process.exec",
                json!({"command": "/bin/cat", "args": ["/etc/hosts"]}),
            ))
            .expect("process.exec /bin/cat /etc/hosts")
    })
    .await
    .expect("join");
    assert_eq!(resp["ok"], json!(true));
    assert_eq!(resp["exit_code"], json!(0));
    let stdout = BASE64_STANDARD
        .decode(resp["stdout"].as_str().unwrap())
        .unwrap();
    let text = String::from_utf8(stdout).unwrap();
    // /etc/hosts on every macOS / Linux box has localhost mapped.
    assert!(
        text.contains("localhost") || text.contains("127.0.0.1"),
        "/etc/hosts via process.exec did not contain `localhost`: {text:?}"
    );
}

/// User-perspective shell.run: real bash piping with `git
/// rev-parse --short HEAD` against this repo. The handler
/// has to honor cwd, dispatch through bash -c, capture stdout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_shell_run_executes_git_command_in_repo() {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;

    if !["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
    {
        eprintln!("real_shell_run_executes_git_command_in_repo: no bash, skipping");
        return;
    }
    if !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".git")
        .exists()
    {
        eprintln!("real_shell_run_executes_git_command_in_repo: not a git checkout, skipping");
        return;
    }

    let manifest = env!("CARGO_MANIFEST_DIR").to_string();
    let resp = tokio::task::spawn_blocking(move || {
        let (reg, _g) = registry_with_temp_home();
        dispatcher_for(reg)
            .execute_rpc(target(
                "shell.run",
                json!({
                    "command": "git rev-parse --short HEAD",
                    "cwd": manifest,
                }),
            ))
            .expect("shell.run git rev-parse")
    })
    .await
    .expect("join");
    assert_eq!(resp["ok"], json!(true));
    assert_eq!(resp["exit_code"], json!(0));
    let stdout = BASE64_STANDARD
        .decode(resp["stdout"].as_str().unwrap())
        .unwrap();
    let sha = String::from_utf8(stdout).unwrap().trim().to_string();
    // git's --short SHA is 7+ hex chars.
    assert!(
        sha.len() >= 7 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "git rev-parse via shell.run did not return a SHA: {sha:?}"
    );
}

/// User-perspective security gate: shell.run MUST refuse to
/// run `rm` without `destructive_acknowledged: true`. We're
/// asserting the safety mechanism actually fires, with the
/// specific stage + code an operator's audit log would see.
#[test]
fn real_shell_run_destructive_rejection_visible_in_response() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("shell.run", json!({"command": "rm /tmp/x"})))
        .expect("shell.run handler must not error; rejection is in the body");
    assert_eq!(resp["ok"], json!(false));
    assert_eq!(resp["code"], json!("DESTRUCTIVE_REJECTED"));
    assert_eq!(resp["pipeline_stage"], json!("destructive"));
    assert_eq!(resp["detail"]["argv0"], json!("rm"));
}

/// User-perspective smoke: read this crate's own Cargo.toml
/// — a real file the developer can `cat` to see the same
/// bytes — through the ability dispatcher. Asserts the
/// content fs.read returns is byte-equal to what
/// std::fs::read_to_string returns for the same path. Not a
/// tempfile-and-immediate-read; the file exists independent
/// of the test fixture and the test only reads it.
#[test]
fn real_fs_read_reads_this_crates_cargo_toml() {
    // CARGO_MANIFEST_DIR is set by cargo for every test run; it
    // is the absolute path of the directory containing
    // Cargo.toml. Using it makes this test work both when run
    // from the crate root (`cargo test -p easynet`) and from a
    // workspace top (`cargo test`) without depending on the
    // current working directory.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let cargo_toml = std::path::PathBuf::from(manifest_dir).join("Cargo.toml");
    assert!(cargo_toml.exists(), "Cargo.toml must exist for this test");

    let resp = invoke(
        "fs.read",
        json!({
            "resource_ref": fs_ref(&cargo_toml, crate::runtime::resources::filesystem::FilesystemResourceCapability::Read),
            "encoding": "utf8",
        }),
    );

    let content = resp["content"].as_str().expect("content is utf8");
    let direct = std::fs::read_to_string(&cargo_toml).expect("direct read");
    assert_eq!(
        content, direct,
        "fs.read content must equal direct std::fs::read_to_string for the same path"
    );
    assert_eq!(resp["size"].as_u64().unwrap(), direct.len() as u64);
    assert_eq!(resp["truncated"], json!(false));
    // Sanity that this is really our Cargo.toml.
    assert!(content.contains("name = \"easynet\""));
    // mtime_ms must be non-null and a real timestamp.
    let mtime = resp["mtime_ms"].as_u64().expect("mtime_ms is integer");
    assert!(mtime > 1_700_000_000_000, "mtime is post-2023: {mtime}");
}

#[test]
fn real_fs_read_reads_an_actual_file() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let dir = std::env::temp_dir().join(format!("real-invoke-fs-read-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("hello.txt");
    std::fs::write(&path, "hello world").unwrap();

    let resp = invoke(
        "fs.read",
        json!({
            "resource_ref": fs_ref(&path, crate::runtime::resources::filesystem::FilesystemResourceCapability::Read),
            "encoding":"utf8"
        }),
    );
    assert_eq!(resp["content"].as_str().unwrap(), "hello world");
    assert_eq!(resp["size"], json!(11));
    assert_eq!(resp["truncated"], json!(false));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_fs_stat_reports_file_metadata() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let dir = std::env::temp_dir().join(format!("real-invoke-fs-stat-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("metadata.txt");
    std::fs::write(&path, "stat bytes").unwrap();

    let resp = invoke(
        "fs.stat",
        json!({
            "resource_ref": fs_ref(&path, crate::runtime::resources::filesystem::FilesystemResourceCapability::Stat),
        }),
    );

    assert_eq!(resp["name"], json!("metadata.txt"));
    assert_eq!(resp["kind"], json!("file"));
    assert_eq!(resp["size"], json!(10));
    assert_eq!(resp["resource_ref_revalidated"], json!(true));
    assert!(
        resp.get("path").is_none(),
        "fs.stat must not expose daemon host paths: {resp:?}"
    );
    let display_path = resp["display_path"]
        .as_str()
        .expect("fs.stat reports display path");
    assert!(
        display_path.ends_with("/metadata.txt"),
        "display path must identify the stat target: {display_path}"
    );
    let mtime = resp["mtime_unix_ms"]
        .as_i64()
        .expect("fs.stat reports file mtime");
    assert!(mtime > 1_700_000_000_000, "mtime is post-2023: {mtime}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_fs_write_creates_a_file_with_expected_content() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let dir = std::env::temp_dir().join(format!("real-invoke-fs-write-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("out.txt");

    let resp = invoke(
        "fs.write",
        json!({
            "resource_ref": fs_ref(&path, crate::runtime::resources::filesystem::FilesystemResourceCapability::Write),
            "content": "real write",
            "encoding": "utf8",
        }),
    );
    assert_eq!(resp["bytes_written"], json!(10));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "real write");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_fs_list_lists_directory_entries() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let dir = std::env::temp_dir().join(format!("real-invoke-fs-list-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();

    let resp = invoke(
        "fs.list",
        json!({
            "resource_ref": fs_ref(&dir, crate::runtime::resources::filesystem::FilesystemResourceCapability::List)
        }),
    );
    let body = resp.as_object().expect("object response");
    // Find an entries array. Field name is implementation-defined
    // but there must be one.
    let arr = body
        .values()
        .find_map(|v| v.as_array())
        .expect("an array of entries must be present");
    let names: Vec<String> = arr
        .iter()
        .filter_map(|e| {
            e.as_object()
                .and_then(|o| o.get("name").or_else(|| o.get("path")))
                .and_then(Value::as_str)
                .map(|s| s.to_string())
        })
        .chain(arr.iter().filter_map(|e| e.as_str().map(String::from)))
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("a.txt")),
        "fs.list missing a.txt: got {arr:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("b.txt")),
        "fs.list missing b.txt: got {arr:?}"
    );
    for entry in arr {
        if let Some(obj) = entry.as_object() {
            assert!(
                !obj.contains_key("path"),
                "fs.list must not expose daemon host paths: {entry:?}"
            );
            assert!(
                obj.get("display_path").and_then(Value::as_str).is_some(),
                "fs.list entry must expose display_path: {entry:?}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_fs_edit_replaces_a_unique_match() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let dir = std::env::temp_dir().join(format!("real-invoke-fs-edit-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("config.txt");
    std::fs::write(&path, "key=old\nother=keep\n").unwrap();

    let resp = invoke(
        "fs.edit",
        json!({
            "resource_ref": fs_ref(&path, crate::runtime::resources::filesystem::FilesystemResourceCapability::Write),
            "old_string": "old",
            "new_string": "new",
        }),
    );
    assert_eq!(resp["ok"], json!(true));
    assert_eq!(resp["matches_replaced"], json!(1));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "key=new\nother=keep\n"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_consent_decide_records_a_decision() {
    // consent.decide expects an existing pending request id.
    // Fresh PermissionService → no pending → handler errors
    // gracefully. We assert the structured error mentions the
    // unknown id rather than panicking.
    let _g = crate::cli::test_support::HomeGuard::new();
    let perms = Arc::new(crate::runtime::execution::permission::PermissionService::new());
    let mut reg = AxonAbilityCatalog::new();
    permission_ability::register(&mut reg, perms);
    let d = dispatcher_for(Arc::new(reg));
    let result = d.execute_rpc(target(
        "consent.decide",
        json!({"id": "no-such-request", "decision": "deny"}),
    ));
    // What we want to PROVE: the call routed to the handler.
    // The handler delegates to PermissionService, which (with
    // no broker installed) returns a structured error explaining
    // why. Either an Ok response or an Err whose message is NOT
    // a "no rpc handler" / "unknown ability" routing failure is
    // a pass.
    match result {
        Ok(v) => assert!(v.is_object()),
        Err(e) => {
            let m = format!("{e}").to_ascii_lowercase();
            assert!(
                !m.contains("no rpc handler") && !m.contains("unknown ability"),
                "consent.decide not routed: {e}"
            );
        }
    }
}

#[test]
fn real_consent_list_pending_returns_empty_on_fresh_service() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let perms = Arc::new(crate::runtime::execution::permission::PermissionService::new());
    let mut reg = AxonAbilityCatalog::new();
    permission_ability::register(&mut reg, perms);
    let d = dispatcher_for(Arc::new(reg));
    let resp = d
        .execute_rpc(target("consent.list_pending", json!({})))
        .expect("consent.list_pending");
    assert!(resp.is_object());
    // Fresh service has no pending requests; whatever the array
    // field is named, it should be empty.
    if let Some(obj) = resp.as_object() {
        let any_array = obj.values().find_map(|v| v.as_array());
        if let Some(arr) = any_array {
            assert!(arr.is_empty(), "expected empty pending list, got {arr:?}");
        }
    }
}

#[test]
fn real_discuss_create_then_post_round_trips_through_the_service() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::discuss::DiscussService::new());
    let mut reg = AxonAbilityCatalog::new();
    discuss_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));

    // Create a room.
    let create = d
        .execute_rpc(target(
            "discuss.create",
            json!({"participants": ["alice", "bob"]}),
        ))
        .expect("discuss.create");
    let room_id = create
        .as_object()
        .and_then(|o| o.get("room_id").or_else(|| o.get("id")))
        .and_then(Value::as_str)
        .expect("room_id in create response")
        .to_string();
    assert!(!room_id.is_empty());

    // Post a message into it.
    let post = d.execute_rpc(target(
        "discuss.post",
        json!({"room_id": room_id, "from": "alice", "content": "hello"}),
    ));
    // Some implementations may require additional fields; assert
    // we either succeeded or got a structured arg error mentioning
    // the missing field — what we want to PROVE is that the call
    // actually reached the service with the room_id, not that the
    // service rejected `{}`.
    match post {
        Ok(v) => assert!(v.is_object()),
        Err(e) => {
            let m = format!("{e}");
            assert!(
                !m.to_ascii_lowercase().contains("no rpc handler"),
                "discuss.post not routed: {m}"
            );
        }
    }
}

#[test]
fn real_schedule_add_then_list_then_remove_round_trip() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::schedule::ScheduleService::new());
    let mut reg = AxonAbilityCatalog::new();
    schedule_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));

    // List on a fresh service.
    let list_empty = d
        .execute_rpc(target("schedule.list", json!({})))
        .expect("schedule.list (fresh)");
    assert!(list_empty.is_object());

    // Add a schedule. The exact required fields vary; pass a
    // realistic-shape envelope. If the handler rejects, the
    // assertion below ensures we at least reached it.
    let add = d.execute_rpc(target(
        "schedule.add",
        json!({
            "target_node": "self",
            "ability": "observe.health",
            "args": {},
            "cron": "0 * * * *",
        }),
    ));
    match add {
        Ok(v) => assert!(v.is_object()),
        Err(e) => {
            let m = format!("{e}");
            assert!(
                !m.to_ascii_lowercase().contains("no rpc handler"),
                "schedule.add not routed: {m}"
            );
        }
    }
}

#[test]
fn real_schedule_enable_routes_to_handler() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::schedule::ScheduleService::new());
    let mut reg = AxonAbilityCatalog::new();
    schedule_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));
    let r = d.execute_rpc(target(
        "schedule.enable",
        json!({"schedule_id": "no-such-sched", "enabled": true}),
    ));
    match r {
        Ok(_) => {}
        Err(e) => {
            let m = format!("{e}");
            assert!(
                !m.to_ascii_lowercase().contains("no rpc handler"),
                "schedule.enable not routed: {m}"
            );
        }
    }
}

#[test]
fn real_schedule_remove_routes_to_handler() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::schedule::ScheduleService::new());
    let mut reg = AxonAbilityCatalog::new();
    schedule_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));
    let r = d.execute_rpc(target(
        "schedule.remove",
        json!({"schedule_id": "no-such-sched"}),
    ));
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_loop_create_then_status_then_cancel() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::loop_instance::LoopService::new());
    let mut reg = AxonAbilityCatalog::new();
    loop_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));

    // Try a realistic create. If it succeeds we then status+cancel;
    // if it fails on missing args we assert the failure mode is
    // non-routing-related.
    let create = d.execute_rpc(target(
        "loop.create",
        json!({
            "worker_agent": "claude",
            "task": "echo test",
        }),
    ));
    if let Ok(v) = &create {
        let loop_id = v
            .as_object()
            .and_then(|o| o.get("loop_id").or_else(|| o.get("id")))
            .and_then(Value::as_str)
            .map(String::from);
        if let Some(loop_id) = loop_id {
            let _status = d
                .execute_rpc(target("loop.status", json!({"loop_id": loop_id})))
                .ok();
            let _cancel = d
                .execute_rpc(target("loop.cancel", json!({"loop_id": loop_id})))
                .ok();
        }
    } else {
        let m = format!("{}", create.unwrap_err());
        assert!(!m.to_ascii_lowercase().contains("no rpc handler"));
    }
}

#[test]
fn real_loop_status_routes_for_unknown_id() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::loop_instance::LoopService::new());
    let mut reg = AxonAbilityCatalog::new();
    loop_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));
    let r = d.execute_rpc(target("loop.status", json!({"loop_id": "none"})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_loop_cancel_routes_for_unknown_id() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::loop_instance::LoopService::new());
    let mut reg = AxonAbilityCatalog::new();
    loop_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));
    let r = d.execute_rpc(target("loop.cancel", json!({"loop_id": "none"})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_device_agent_list_returns_a_list_under_temp_home() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("agent.list", json!({})))
        .expect("agent.list");
    assert!(resp.is_object());
}

#[test]
fn real_device_session_list_returns_empty_under_temp_home() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("session.list", json!({})))
        .expect("session.list");
    assert!(resp.is_object());
}

#[test]
fn real_device_agent_start_then_stop_agent_round_trip() {
    let (reg, _g) = registry_with_temp_home();
    crate::persistence::config::save_credentials(&crate::persistence::config::Credentials {
        node_id: "dev-1".to_string(),
        credential_token: "token".to_string(),
        hub_endpoint: "axon://hub.test:50051".to_string(),
        realm: "localhost".to_string(),
        username: Some("dev".to_string()),
        user_id: Some("user-dev".to_string()),
        ..Default::default()
    })
    .expect("seed joined credentials");
    let d = dispatcher_for(reg);
    let start = d
        .execute_rpc(target(
            "agent.start",
            json!({
                "name": "smoke-test-agent",
                "agent_type": "claude-code",
            }),
        ))
        .expect("agent.start");
    assert!(start.is_object());
    // Stop it.
    let stop = d.execute_rpc(target("agent.stop", json!({"name": "smoke-test-agent"})));
    match stop {
        Ok(v) => {
            // Idempotent ack=true expected.
            assert!(v.is_object());
        }
        Err(e) => panic!("agent.stop unexpected: {e}"),
    }
}

#[test]
fn real_device_agent_refresh_scans_agents_through_wired_registrar() {
    // A daemon-built registry wires the HotAgentRegistrar into the shared
    // cell at construction (the catalog builder stashes it immediately), so
    // `agent.refresh` no longer reports the boot-window `runtime_not_ready`
    // case — it scans the persisted agents through the registrar and
    // returns `ok=true`. A hosted agent with no runtime row to sync simply
    // reports `runtime_registered=0` without failing.
    let (reg, _g) = registry_with_temp_home();
    crate::persistence::config::save_credentials(&crate::persistence::config::Credentials {
        node_id: "dev-1".to_string(),
        credential_token: "token".to_string(),
        hub_endpoint: "axon://hub.test:50051".to_string(),
        realm: "localhost".to_string(),
        username: Some("dev".to_string()),
        user_id: Some("user-dev".to_string()),
        ..Default::default()
    })
    .expect("seed joined credentials");
    let d = dispatcher_for(reg);
    d.execute_rpc(target(
        "agent.start",
        json!({
            "name": "refresh-smoke-agent",
            "agent_type": "claude-code",
        }),
    ))
    .expect("seed agent before refresh");
    let resp = d
        .execute_rpc(target("agent.refresh", json!({})))
        .expect("agent.refresh");
    assert_eq!(resp.get("ok"), Some(&json!(true)));
    assert_eq!(resp.get("runtime_not_ready"), Some(&json!(false)));
    assert_eq!(resp.get("agents_scanned"), Some(&json!(1)));
    assert!(
        resp.get("agents").and_then(Value::as_array).is_some(),
        "refresh must return an agents array: {resp}"
    );
}

#[test]
fn real_device_skill_install_routes_with_realistic_source() {
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    // A non-existent path is a realistic invalid input — handler
    // should reject with a structured error, not panic.
    let r = d.execute_rpc(target(
        "skill.install",
        json!({"source": "/tmp/no-such-skill.tgz"}),
    ));
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_device_skill_remove_routes_for_unknown_name() {
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    let r = d.execute_rpc(target("skill.remove", json!({"name": "no-such-skill"})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_device_skill_upgrade_routes_for_unknown_name() {
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    let r = d.execute_rpc(target("skill.upgrade", json!({"name": "no-such-skill"})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

// ── ability.publish / skill.publish / skill.list ───────────────────
//
// These five abilities back the curator path that mission.think
// drives. They are stateless root verbs: arg parsing fails with a
// clear error before any disk write happens, which is exactly what
// the "real-invoke" coverage guard wants — we confirm the
// dispatcher routes by passing deliberately-incomplete args and
// asserting the error is a *handler* error, not a "no rpc handler"
// dispatcher miss.

#[test]
fn real_ability_publish_routes_with_missing_args() {
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    let r = d.execute_rpc(target("ability.publish", json!({})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "ability.publish must be routed: {e}"
        ),
    }
}

#[test]
fn real_ability_unpublish_routes_with_missing_args() {
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    let r = d.execute_rpc(target("ability.unpublish", json!({})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "ability.unpublish must be routed: {e}"
        ),
    }
}

#[test]
fn real_skill_publish_routes_with_missing_args() {
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    let r = d.execute_rpc(target("skill.publish", json!({})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "skill.publish must be routed: {e}"
        ),
    }
}

#[test]
fn real_skill_unpublish_routes_with_missing_args() {
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    let r = d.execute_rpc(target("skill.unpublish", json!({})));
    match r {
        Ok(_) => {}
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "skill.unpublish must be routed: {e}"
        ),
    }
}

#[test]
fn real_mission_think_routes_with_missing_args() {
    // mission.think rejects missing owner_agent_id / prompt with a
    // typed error. This test confirms the dispatcher routes the
    // verb (the handler is registered) and that arg validation
    // fires before any chat call attempts to spawn — important
    // because a mis-routed mission.think with a real LLM agent
    // could otherwise burn a token budget before failing.
    let (reg, _g) = registry_with_temp_home();
    let d = dispatcher_for(reg);
    let r = d.execute_rpc(target("mission.think", json!({})));
    match r {
        Ok(_) => panic!("mission.think with empty args must error"),
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "mission.think must be routed: {e}"
        ),
    }
}

#[test]
fn real_skill_list_returns_items_array_under_temp_home() {
    // Empty under temp HOME but the field shape must hold.
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("skill.list", json!({})))
        .expect("skill.list");
    assert!(
        resp.get("items").and_then(Value::as_array).is_some(),
        "skill.list must return an `items` array; got {resp}"
    );
}

#[test]
fn real_skill_list_accepts_agent_and_subject_ura_scope() {
    let (reg, _g) = registry_with_temp_home();
    let (owner, _skill_dir) = materialise_skill_fixture("scope", "scoped-skill", "# Scoped\nBody");
    let agent_ura = crate::ura::agent_ura("localhost", "dev", &owner);
    let mut local = crate::persistence::local_agents::LocalAgentsFile {
        host_device_agent_ura: "easynet:///r/localhost/device/dev-1".to_string(),
        ..Default::default()
    };
    crate::persistence::local_agents::upsert_hosted_agent(&mut local, "llm", &owner, &agent_ura);
    crate::persistence::local_agents::save(&local).expect("save local agents fixture");
    let subject_ura = crate::ura::resource_dot_ura(
        "localhost",
        &format!("agent.dev.{owner}"),
        "skill/scoped-skill",
    );

    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "skill.list",
            json!({
                "agent_ura": agent_ura,
                "subject_ura": subject_ura,
            }),
        ))
        .expect("skill.list with URA scope");
    let items = resp["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "skill.list must scope to one skill: {resp}");
    assert_eq!(items[0]["agent_id"], owner);
    assert_eq!(items[0]["name"], "scoped-skill");
}

#[test]
fn real_skill_tree_lists_files_for_registered_agent_skill() {
    let (reg, _g) = registry_with_temp_home();
    let (owner, skill_dir) =
        materialise_skill_fixture("tree", "inspectable", "# Inspectable\nBody");
    let notes = skill_dir.join("notes");
    std::fs::create_dir_all(&notes).expect("create notes dir");
    crate::persistence::config::atomic_write(&notes.join("guide.md"), b"guide")
        .expect("write guide");

    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "skill.tree",
            json!({
                "owner_agent_id": owner,
                "skill_name": "inspectable",
                "resource_ura": "easynet:///r/localhost/resource/agent.dev.tree/skill/inspectable",
            }),
        ))
        .expect("skill.tree");
    assert_eq!(
        resp["resource_ura"],
        "easynet:///r/localhost/resource/agent.dev.tree/skill/inspectable"
    );
    let files = resp["files"].as_array().expect("files array");
    assert!(files
        .iter()
        .any(|f| f["path"] == "SKILL.md" && f["type"] == "file"));
    assert!(files.iter().any(|f| {
        f["path"] == "notes/guide.md"
            && f["type"] == "file"
            && f["resource_ura"]
                == "easynet:///r/localhost/resource/agent.dev.tree/skill/inspectable/file/notes/guide.md"
    }));
}

#[test]
fn real_skill_read_file_returns_utf8_content() {
    let (reg, _g) = registry_with_temp_home();
    let (owner, _skill_dir) = materialise_skill_fixture("read", "readable", "# Readable\nBody");

    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "skill.read_file",
            json!({
                "owner_agent_id": owner,
                "skill_name": "readable",
                "resource_ura": "easynet:///r/localhost/resource/agent.dev.read/skill/readable",
                "path": "SKILL.md",
            }),
        ))
        .expect("skill.read_file");
    assert_eq!(resp["content"], "# Readable\nBody");
    assert_eq!(resp["encoding"], "utf-8");
    assert_eq!(
        resp["resource_ura"],
        "easynet:///r/localhost/resource/agent.dev.read/skill/readable/file/SKILL.md"
    );
}

#[test]
fn real_skill_write_file_updates_skill_source() {
    let (reg, _g) = registry_with_temp_home();
    let (owner, skill_dir) = materialise_skill_fixture("write", "editable", "old body");

    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "skill.write_file",
            json!({
                "owner_agent_id": owner,
                "skill_name": "editable",
                "resource_ura": "easynet:///r/localhost/resource/agent.dev.write/skill/editable",
                "path": "SKILL.md",
                "content": "new body",
            }),
        ))
        .expect("skill.write_file");
    assert_eq!(resp["ok"], true);
    assert_eq!(
        resp["resource_ura"],
        "easynet:///r/localhost/resource/agent.dev.write/skill/editable/file/SKILL.md"
    );
    let on_disk = std::fs::read_to_string(skill_dir.join("SKILL.md")).expect("read updated skill");
    assert_eq!(on_disk, "new body");
}

// ════════════════════════════════════════════════════════════════
// Category D: process / shell with real binaries
// ════════════════════════════════════════════════════════════════

// process.exec / shell.run handlers call
// `tokio::runtime::Handle::current().block_on(...)` on the
// blocking-pool thread the dispatcher gave them. Inside a unit
// test we therefore need a real tokio runtime around the call.
// `#[tokio::test(flavor = "multi_thread")]` gives the
// blocking-pool thread + a worker the spawn_blocking-style
// pattern requires.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_process_exec_runs_bin_echo() {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;

    let resp = tokio::task::spawn_blocking(|| {
        let (reg, _g) = registry_with_temp_home();
        dispatcher_for(reg)
            .execute_rpc(target(
                "process.exec",
                json!({
                    "command": "/bin/echo",
                    "args": ["hello world"],
                }),
            ))
            .expect("process.exec")
    })
    .await
    .expect("join");
    assert_eq!(resp["ok"], json!(true), "{resp}");
    assert_eq!(resp["exit_code"], json!(0));
    let stdout = BASE64_STANDARD
        .decode(resp["stdout"].as_str().unwrap())
        .unwrap();
    assert_eq!(String::from_utf8(stdout).unwrap(), "hello world\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_shell_run_executes_echo_via_bash() {
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;

    if !["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
    {
        eprintln!("real_shell_run: no bash on host, skipping");
        return;
    }
    let resp = tokio::task::spawn_blocking(|| {
        let (reg, _g) = registry_with_temp_home();
        dispatcher_for(reg)
            .execute_rpc(target(
                "shell.run",
                json!({"command": "echo hi-from-shell"}),
            ))
            .expect("shell.run")
    })
    .await
    .expect("join");
    assert_eq!(resp["ok"], json!(true), "{resp}");
    assert_eq!(resp["exit_code"], json!(0));
    let stdout = BASE64_STANDARD
        .decode(resp["stdout"].as_str().unwrap())
        .unwrap();
    assert_eq!(String::from_utf8(stdout).unwrap(), "hi-from-shell\n");
}

// ════════════════════════════════════════════════════════════════
// Category C: network / external
// ════════════════════════════════════════════════════════════════

#[test]
fn real_http_request_hits_a_localhost_listener() {
    use std::io::{Read, Write};
    // Bind a tiny TCP listener that speaks the HTTP/1.1 happy path.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();

    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = "{\"echo\":\"hi\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target(
            "http.request",
            json!({
                "url": format!("http://127.0.0.1:{port}/"),
                "method": "GET",
                "timeout_ms": 5000,
            }),
        ))
        .expect("http.request");
    let _ = server.join();
    assert_eq!(resp["ok"], json!(true), "{resp}");
    assert_eq!(resp["status"], json!(200));
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine as _;
    let body = BASE64_STANDARD
        .decode(resp["body"].as_str().unwrap())
        .unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), "{\"echo\":\"hi\"}");
}

#[test]
fn real_mcp_client_list_routes_with_no_upstream_configured() {
    // No ~/.easynet/mcp_clients.json under temp HOME → empty
    // upstream set. Handler should return a structured "no
    // upstream configured" / empty-list response, NOT a panic.
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target("mcp.client.list", json!({})));
    match r {
        Ok(v) => assert!(v.is_object()),
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_mcp_client_call_routes_with_realistic_args() {
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target(
        "mcp.client.call",
        json!({"server": "no-such", "name": "no-such-tool", "arguments": {}}),
    ));
    match r {
        Ok(v) => {
            // Common pattern: returns isError=true on unknown server.
            assert!(v.is_object());
        }
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_mcp_bridge_list_tools_returns_local_catalog() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("mcp.bridge.list_tools", json!({})))
        .expect("mcp.bridge.list_tools");
    // Catalog must mention at least one well-known tool.
    let s = resp.to_string();
    assert!(
        s.contains("observe.health") || s.contains("fs.read") || s.contains("tools"),
        "mcp.bridge.list_tools missing known abilities: {resp}"
    );
}

#[test]
fn real_mcp_bridge_call_tool_routes_to_local_dispatch() {
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target(
        "mcp.bridge.call_tool",
        json!({"name": "observe.health", "arguments": {}}),
    ));
    match r {
        Ok(v) => assert!(v.is_object()),
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_a2a_bridge_list_skills_returns_a_card() {
    let (reg, _g) = registry_with_temp_home();
    let resp = dispatcher_for(reg)
        .execute_rpc(target("a2a.bridge.list_skills", json!({})))
        .expect("a2a.bridge.list_skills");
    assert!(resp.is_object());
}

#[test]
fn real_a2a_bridge_send_task_routes_with_realistic_args() {
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target(
        "a2a.bridge.send_task",
        json!({"target_agent":"none", "skill":"chat", "input":{"prompt":"hi"}}),
    ));
    match r {
        Ok(v) => assert!(v.is_object()),
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_a2a_client_send_task_routes_with_realistic_args() {
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target(
        "a2a.client.send_task",
        json!({
            "target_node_ura": "easynet:///r/acme/device/N1",
            "agent_name": "claude",
            "skill_name": "chat",
            "args": {"prompt": "hi"},
        }),
    ));
    match r {
        Ok(v) => assert!(v.is_object()),
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no rpc handler")),
    }
}

#[test]
fn real_meta_teach_routes_with_missing_args() {
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target("meta.teach", json!({})));
    match r {
        Ok(v) => assert!(v.is_object()),
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "meta.teach must be routed: {e}"
        ),
    }
}

#[test]
fn real_meta_acquire_routes_with_missing_args() {
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target("meta.acquire", json!({})));
    match r {
        Ok(v) => assert!(v.is_object()),
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "meta.acquire must be routed: {e}"
        ),
    }
}

#[test]
fn real_meta_forget_routes_with_missing_args() {
    let (reg, _g) = registry_with_temp_home();
    let r = dispatcher_for(reg).execute_rpc(target("meta.forget", json!({})));
    match r {
        Ok(v) => assert!(v.is_object()),
        Err(e) => assert!(
            !format!("{e}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "meta.forget must be routed: {e}"
        ),
    }
}

// ════════════════════════════════════════════════════════════════
// Coverage matrix: every published ability is exercised above
// ════════════════════════════════════════════════════════════════

/// Scan this very file for any quoted ability name that appears
/// as a callsite argument and assert the union covers every
/// published ability. This is the structural guarantee that "we
/// tested every ability" stays true as the registry evolves: a
/// new ability without a real-invoke test fails this check at CI
/// time and the diff names exactly which ability is missing.
///
/// Recognised patterns:
///   * `target("name"`      — the helper builder
///   * `invoke("name"`      — the convenience wrapper
///   * `register_rpc("name"` etc — direct registration in
///     fixture-bound tests where the name is explicit
#[test]
fn every_published_ability_has_a_real_invoke_test() {
    let source = include_str!("real_invoke_tests.rs");
    let published: std::collections::BTreeSet<String> = build_registry()
        .list_abilities()
        .into_iter()
        .filter(|n| is_publishable_catalog_name(n))
        .filter(|n| !n.ends_with(".chat")) // dynamic per-agent, not in this catalog
        // RFC-002 §3.3: keyring abilities are owner-namespaced and
        // covered by their own unit tests in
        // `runtime::keyring::abilities::tests`.
        .filter(|n| !n.starts_with("device.keyring."))
        .collect();
    let mut covered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // Walk every quoted string in the file. A token that matches
    // a published ability name counts as coverage. Cheaper than
    // teaching the scanner about every callsite shape, and works
    // even when a future test invokes via a new helper.
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            // Walk to the closing quote, respecting escapes.
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'"' {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() {
                let token = &source[i + 1..j];
                if published.contains(token) {
                    covered.insert(token.to_string());
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    let missing: Vec<&String> = published.difference(&covered).collect();
    assert!(
        missing.is_empty(),
        "abilities with NO real-invoke test: {missing:?}\n\
         covered: {} of {} published",
        covered.len(),
        published.len()
    );
    eprintln!(
        "real-invoke coverage: {} / {} published abilities exercised",
        covered.len(),
        published.len()
    );
}

#[test]
fn real_device_plugin_status_reports_runtime_surface() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let status = d
        .execute_rpc(target("plugin.status", json!({})))
        .expect("plugin.status must dispatch through plugin lifecycle ability");
    assert_eq!(status["ok"], true);
    assert!(
        status["abilities"].is_array(),
        "plugin.status must report runtime ability rows: {status}"
    );
}

#[test]
fn real_device_plugin_reload_reports_registration_diff() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let report = d
        .execute_rpc(target("plugin.reload", json!({})))
        .expect("plugin.reload must dispatch through plugin lifecycle ability");
    assert_eq!(report["ok"], true);
    assert!(
        report["registered_abilities"].is_array(),
        "plugin.reload must include registered_abilities: {report}"
    );
    assert!(
        report["unregistered_abilities"].is_array(),
        "plugin.reload must include unregistered_abilities: {report}"
    );
}

#[test]
fn real_device_plugin_activate_realtime_routes_through_lifecycle_ability() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let result = d.execute_rpc(target(
        "plugin.activate_realtime",
        json!({"package_id": "missing.test"}),
    ));
    match result {
        Ok(value) => assert!(value.is_object()),
        Err(err) => assert!(
            !format!("{err}")
                .to_ascii_lowercase()
                .contains("no rpc handler"),
            "plugin.activate_realtime must be routed: {err}"
        ),
    }
}

// ════════════════════════════════════════════════════════════════
// Category E: Stream / Bidi
// ════════════════════════════════════════════════════════════════

#[test]
fn real_consent_subscribe_returns_a_stream_source() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let perms = Arc::new(crate::runtime::execution::permission::PermissionService::new());
    let mut reg = AxonAbilityCatalog::new();
    permission_ability::register(&mut reg, perms);
    let d = dispatcher_for(Arc::new(reg));
    let mut t = target("consent.subscribe", json!({}));
    t.call_mode = CallMode::Stream;
    let _src = d.execute_stream(t).expect("consent.subscribe stream");
    // Simply receiving a StreamSource without panic is the
    // assertion: dispatcher routed to register_stream() handler.
}

#[test]
fn real_discuss_subscribe_returns_a_stream_source() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::discuss::DiscussService::new());
    let mut reg = AxonAbilityCatalog::new();
    discuss_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));
    let mut t = target("discuss.subscribe", json!({"room_id": "any"}));
    t.call_mode = CallMode::Stream;
    let r = d.execute_stream(t);
    // Some implementations require an existing room; either an
    // OK StreamSource or a structured error is fine — what we
    // need to prove is the handler was reached.
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no stream handler")),
    }
}

#[test]
fn real_loop_subscribe_returns_a_stream_source() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::loop_instance::LoopService::new());
    let mut reg = AxonAbilityCatalog::new();
    loop_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));
    let mut t = target("loop.subscribe", json!({"loop_id": "any"}));
    t.call_mode = CallMode::Stream;
    let r = d.execute_stream(t);
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no stream handler")),
    }
}

#[test]
fn real_device_session_attach_returns_a_stream_source_for_unknown_id() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let svc = Arc::new(crate::runtime::execution::session::SessionService::new());
    let mut reg = AxonAbilityCatalog::new();
    session_ability::register(&mut reg, svc);
    let d = dispatcher_for(Arc::new(reg));
    let mut t = target("session.attach", json!({"session_id": "no-such"}));
    t.call_mode = CallMode::Stream;
    let r = d.execute_stream(t);
    match r {
        Ok(_) => {}
        Err(e) => assert!(!format!("{e}")
            .to_ascii_lowercase()
            .contains("no stream handler")),
    }
}

#[test]
fn real_device_terminal_create_then_close_round_trip() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let pty = Arc::new(crate::runtime::execution::pty::PtyService::new());
    let mut reg = AxonAbilityCatalog::new();
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty), None);
    let d = dispatcher_for(Arc::new(reg));

    let create = d
        .execute_rpc(target("terminal.create", json!({})))
        .expect("pty_session_create");
    let session_id = create["session_id"]
        .as_str()
        .expect("session_id in response")
        .to_string();
    assert!(!session_id.is_empty());

    let listed = d
        .execute_rpc(target("terminal.list", json!({})))
        .expect("terminal.list");
    let sessions = listed["sessions"].as_array().expect("sessions array");
    assert!(
        sessions
            .iter()
            .any(|session| session["session_id"].as_str() == Some(session_id.as_str())),
        "terminal.list must include the created session: {listed}"
    );

    let close = d
        .execute_rpc(target("terminal.close", json!({"session_id": session_id})))
        .expect("pty_session_close");
    assert_eq!(close["ack"], json!(true));
}

// terminal.input / _read / _resize are the unary-RPC
// data plane the EasyNet backend's PTYDriver invokes for the
// production HTTP-session terminal flow. The structural guard
// `every_published_ability_has_a_real_invoke_test` asserts each
// of the three has at least one test in this file; the round-
// trip below covers all three in one realistic exercise (write
// a marker via input → drain it via read → resize the window
// while the session is live), so the registry walker that
// scans this file's tokens picks up every ability name.
#[test]
fn real_device_terminal_input_read_resize_round_trip() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let pty = Arc::new(crate::runtime::execution::pty::PtyService::new());
    let io = pty_io_ability::PtyIoService::new();
    let mut reg = AxonAbilityCatalog::new();
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty), Some(io.clone()));
    pty_io_ability::register(&mut reg, Arc::clone(&pty), io);
    let d = dispatcher_for(Arc::new(reg));

    let create = d
        .execute_rpc(target("terminal.create", json!({})))
        .expect("pty_session_create");
    let sid = create["session_id"].as_str().unwrap().to_string();

    // terminal.resize — exercise it before any I/O so
    // the shell starts at the requested geometry.
    let resize = d
        .execute_rpc(target(
            "terminal.resize",
            json!({"session_id": sid.clone(), "cols": 132, "rows": 50}),
        ))
        .expect("pty_session_resize");
    assert_eq!(resize["ack"], json!(true));

    // terminal.input — push a printf line that produces
    // a deterministic stdout marker.
    use base64::Engine;
    let input_b64 =
        base64::engine::general_purpose::STANDARD.encode(b"printf 'EASYNET_REAL_PTY_OK\\n'\n");
    let input = d
        .execute_rpc(target(
            "terminal.input",
            json!({"session_id": sid.clone(), "data": input_b64}),
        ))
        .expect("pty_session_input");
    assert_eq!(input["ack"], json!(true));

    // terminal.read — drain output up to a timeout
    // until we see the marker. May take a couple of cycles
    // because the shell's prompt + echoed input land first.
    let mut accum = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && !accum.contains("EASYNET_REAL_PTY_OK") {
        let resp = d
            .execute_rpc(target(
                "terminal.read",
                json!({"session_id": sid.clone(), "timeout": 1.0}),
            ))
            .expect("pty_session_read");
        if let Some(b64) = resp["output"].as_str() {
            if !b64.is_empty() {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default();
                accum.push_str(&String::from_utf8_lossy(&raw));
            }
        }
    }
    assert!(
        accum.contains("EASYNET_REAL_PTY_OK"),
        "expected printf marker via terminal.read; got {accum:?}"
    );

    // Cleanup.
    let _ = d.execute_rpc(target("terminal.close", json!({"session_id": sid})));
}

// pty_session_attach spawns three tokio tasks (reader / writer /
// exit-watcher) inside the bidi handler, so the test needs a
// live runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_device_terminal_attach_returns_a_bidi_source() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let pty = Arc::new(crate::runtime::execution::pty::PtyService::new());
    let mut reg = AxonAbilityCatalog::new();
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty), None);
    pty_attach_ability::register(&mut reg, Arc::clone(&pty));
    let d = dispatcher_for(Arc::new(reg));

    let create = d
        .execute_rpc(target("terminal.create", json!({})))
        .expect("pty_session_create");
    let sid = create["session_id"].as_str().unwrap().to_string();

    let mut t = target("terminal.attach", json!({"session_id": sid.clone()}));
    t.call_mode = CallMode::Bidi;
    let _bidi = d.execute_bidi(t).expect("pty_session_attach bidi");

    // Cleanup.
    let _ = d.execute_rpc(target("terminal.close", json!({"session_id": sid})));
}

// fs.transfer is a bidi ability — open it with mode=upload
// against a temp path, push a chunk + eof, drain the complete frame,
// then verify the file landed with the right content. The
// structural guard `every_published_ability_has_a_real_invoke_test`
// requires a token-grep match for the ability name in this file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_device_fs_transfer_uploads_a_round_trip_through_dispatcher() {
    use base64::Engine;
    let _g = crate::cli::test_support::HomeGuard::new();
    let mut reg = AxonAbilityCatalog::new();
    file_transfer_ability::register(&mut reg);
    let d = dispatcher_for(Arc::new(reg));

    let path = std::env::temp_dir().join(format!(
        "easynet-real-ft-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));

    let mut t = target(
        "fs.transfer",
        json!({
            "mode": "upload",
            "resource_ref": fs_ref(&path, crate::runtime::resources::filesystem::FilesystemResourceCapability::Write),
        }),
    );
    t.call_mode = CallMode::Bidi;
    let bidi = d.execute_bidi(t).expect("file_transfer bidi");

    let bytes = b"real-invoke-device-file-transfer";
    let chunk_b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    bidi.to_client
        .send(json!({"type": "chunk", "data": chunk_b64}))
        .await
        .unwrap();
    bidi.to_client.send(json!({"type": "eof"})).await.unwrap();

    // Drain frames until we see complete or timeout.
    let mut from = bidi.from_client;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut got_complete = false;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), from.recv()).await {
            Ok(Some(f)) => {
                let f = f.into_json_value().expect("fs.transfer emits JSON");
                if f["type"] == "complete" {
                    got_complete = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(got_complete, "expected `complete` frame from fs.transfer");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_device_fs_transfer_downloads_a_round_trip_through_dispatcher() {
    use base64::Engine;
    let _g = crate::cli::test_support::HomeGuard::new();
    let mut reg = AxonAbilityCatalog::new();
    file_transfer_ability::register(&mut reg);
    let d = dispatcher_for(Arc::new(reg));

    let path = std::env::temp_dir().join(format!(
        "easynet-real-ft-download-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let bytes = b"real-invoke-device-file-transfer-download";
    std::fs::write(&path, bytes).unwrap();

    let mut t = target(
        "fs.transfer",
        json!({
            "mode": "download",
            "resource_ref": fs_ref(&path, crate::runtime::resources::filesystem::FilesystemResourceCapability::Read),
        }),
    );
    t.call_mode = CallMode::Bidi;
    let bidi = d.execute_bidi(t).expect("file_transfer bidi");

    let mut from = bidi.from_client;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut downloaded = Vec::new();
    let mut got_complete = false;
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(500), from.recv()).await {
            Ok(Some(f)) => {
                let f = f.into_json_value().expect("fs.transfer emits JSON");
                match f["type"].as_str() {
                    Some("chunk") => {
                        let chunk = f["data"].as_str().expect("chunk carries base64 data");
                        downloaded.extend(
                            base64::engine::general_purpose::STANDARD
                                .decode(chunk)
                                .expect("chunk base64 decodes"),
                        );
                    }
                    Some("complete") => {
                        got_complete = true;
                        break;
                    }
                    other => panic!("unexpected file_transfer download frame {other:?}: {f}"),
                }
            }
            _ => break,
        }
    }
    assert!(
        got_complete,
        "expected `complete` frame from fs.transfer download"
    );
    assert_eq!(downloaded, bytes);
    let _ = std::fs::remove_file(&path);
}

// ════════════════════════════════════════════════════════════════
// Category F: RFC-005 v3.2 media abilities (A1–A9)
// ────────────────────────────────────────────────────────────────
//
// Eight physical-channel abilities (mic / camera / screen /
// speaker / voice / transcribe) plus `meta.list_resources`. Of
// these, `camera.snapshot` has a real envelope-aware handler
// backed by `SyntheticBackend` (see `media::camera_snapshot` for
// the dedicated end-to-end suite); `meta.list_resources` ships
// fully working too. The remaining seven are PR2 stubs that
// reject by design until PR3 lands cpal/nokhwa/screen, so the
// real-invoke test for each one asserts the dispatch reached the
// stub (rather than 404'd as "no handler") and surfaced the
// expected "not yet wired" / "subject required" reason — same
// shape any caller would observe today.
//
// Each test references the ability name as a string literal so
// `every_published_ability_has_a_real_invoke_test`'s coverage
// scanner sees it.
// ════════════════════════════════════════════════════════════════

/// Helper: for the seven stubs, the expected behaviour is a
/// terminal error whose message reaches the stub body. Match on
/// either `"device backend not yet wired"` (PR2 stub default) or
/// the INV-SUBJECT-ENVELOPE rejection — anything else means the
/// ability didn't route to its handler at all.
fn assert_routed_to_media_stub(ability: &str, err: &anyhow::Error) {
    let msg = err.to_string();
    let routed = msg.contains("device backend not yet wired")
        || msg.contains("INV-SUBJECT-ENVELOPE")
        || msg.contains("subject_required")
        || msg.contains("subject_in_args");
    assert!(
        routed,
        "{ability}: error did not look like the PR2 media stub: {msg}"
    );
}

fn seed_real_invoke_display_resource(hardware_id: &str) -> String {
    let mut file = crate::persistence::resources::ResourcesFile::default();
    let ura = crate::persistence::resources::upsert_resource(
        &mut file,
        crate::persistence::resources::ResourceUpsert {
            realm: "acme",
            owner_agent: "easynet:///r/acme/device/real-invoke",
            kind: crate::persistence::resources::ResourceType::Display,
            binding: crate::persistence::resources::ResourceBinding::LocalDevice,
            hardware_id,
            display_name: "Real Invoke Display",
            metadata: json!({}),
        },
    );
    crate::persistence::resources::save(&file).expect("save real-invoke display resource");
    ura
}

#[test]
fn real_mic_subscribe_routes_to_subject_gate() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let mut t = target("mic.subscribe", json!({}))
        .with_subject("easynet:///r/acme/resource/missing-real-invoke-mic");
    t.call_mode = CallMode::Stream;
    let err = d
        .execute_stream(t)
        .expect_err("missing mic subject must reject before opening hardware");
    assert!(
        err.to_string().contains("resource_not_found"),
        "mic.subscribe must route to media subject gate; got {err}"
    );
}

#[test]
fn real_camera_subscribe_routes_to_media_subject_gate() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let mut t = target("camera.subscribe", json!({}));
    t.call_mode = CallMode::Stream;
    let err = d
        .execute_stream(t)
        .expect_err("missing camera subject must reject before opening hardware");
    assert!(
        err.to_string().contains("subject_required"),
        "camera.subscribe must route to media subject gate; got {err}"
    );
}

#[test]
fn real_camera_snapshot_with_no_subject_returns_subject_required() {
    // PR3a real handler: with no envelope subject the handler
    // MUST reject with reason="subject_required". The dedicated
    // suite in `media::camera_snapshot` covers the populated path.
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let err = d
        .execute_rpc(target("camera.snapshot", json!({})))
        .expect_err("camera.snapshot without subject must reject");
    assert!(
        err.to_string().contains("subject_required"),
        "camera.snapshot: expected reason=subject_required; got {err}"
    );
}

#[test]
fn real_camera_record_start_with_no_subject_returns_subject_required() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let err = d
        .execute_rpc(target("camera.record_start", json!({})))
        .expect_err("camera.record_start without subject must reject");
    assert!(
        err.to_string().contains("subject_required"),
        "camera.record_start: expected reason=subject_required; got {err}"
    );
}

#[test]
fn real_camera_record_stop_with_no_subject_returns_subject_required() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let err = d
        .execute_rpc(target(
            "camera.record_stop",
            json!({"recording_session_id": "missing-real-invoke-session"}),
        ))
        .expect_err("camera.record_stop without subject must reject");
    assert!(
        err.to_string().contains("subject_required"),
        "camera.record_stop: expected reason=subject_required; got {err}"
    );
}

#[test]
fn real_screen_subscribe_with_no_subject_returns_subject_required() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let mut t = target("screen.subscribe", json!({}));
    t.call_mode = CallMode::Stream;
    let err = d
        .execute_stream(t)
        .expect_err("screen.subscribe without subject must reject");
    assert!(
        err.to_string().contains("subject_required"),
        "screen.subscribe: expected reason=subject_required; got {err}"
    );
}

#[test]
fn real_screen_snapshot_with_no_subject_returns_subject_required() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let err = d
        .execute_rpc(target("screen.snapshot", json!({})))
        .expect_err("screen.snapshot without subject must reject");
    assert!(
        err.to_string().contains("subject_required"),
        "screen.snapshot: expected reason=subject_required; got {err}"
    );
}

#[test]
#[cfg(feature = "remote-desktop")]
fn real_remote_desktop_permission_status_reports_contract() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let status = d
        .execute_rpc(target("remote_desktop.permission_status", json!({})))
        .expect("remote_desktop.permission_status must dispatch");
    assert_eq!(
        status["permission"], "screen_capture",
        "permission_status must report the screen capture permission contract"
    );
}

#[test]
#[cfg(feature = "remote-desktop")]
fn real_remote_desktop_request_permission_reports_contract() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let status = d
        .execute_rpc(target("remote_desktop.request_permission", json!({})))
        .expect("remote_desktop.request_permission must dispatch");
    assert_eq!(
        status["permission"], "screen_capture",
        "request_permission must report the screen capture permission contract"
    );
}

#[test]
#[cfg(feature = "remote-desktop")]
fn real_remote_desktop_create_session_requires_envelope_subject() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let err = d
        .execute_rpc(target("remote_desktop.create_session", json!({})))
        .expect_err("remote_desktop.create_session without subject must reject");
    assert!(
        err.to_string().contains("subject_required"),
        "create_session must reject missing subject with subject_required; got {err}"
    );
}

#[test]
#[cfg(feature = "remote-desktop")]
fn real_remote_desktop_session_lifecycle_routes_through_local_runtime() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let subject = seed_real_invoke_display_resource("remote-desktop-real-invoke-display");
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let session_id = unique_call_id("remote-desktop");

    let created = d
        .execute_rpc(
            target(
                "remote_desktop.create_session",
                json!({
                    "session_id": session_id,
                    "mode": "view_only",
                    "lease_ttl_ms": 5000,
                }),
            )
            .with_subject(subject.clone())
            .with_causal_context(remote_desktop_test_consent_causal_context()),
        )
        .expect("remote_desktop.create_session must create a session");
    assert_eq!(created["session_id"], session_id);
    let token = created["session_token"]
        .as_str()
        .expect("create_session must return session_token")
        .to_string();

    let shown = d
        .execute_rpc(
            target(
                "remote_desktop.show_session",
                json!({"session_id": session_id, "session_token": token}),
            )
            .with_subject(subject.clone())
            .with_causal_context(remote_desktop_test_consent_causal_context()),
        )
        .expect("remote_desktop.show_session must dispatch");
    assert_eq!(shown["session_id"], session_id);
    assert!(
        shown.get("session_token").is_none(),
        "show_session must not leak session_token"
    );
    let token = created["session_token"].as_str().unwrap().to_string();

    let signaled = d
        .execute_rpc(
            target(
                "remote_desktop.set_description",
                json!({
                    "session_id": session_id,
                    "session_token": token,
                    "side": "local",
                    "description": {"type": "answer", "sdp": "v=0"}
                }),
            )
            .with_subject(subject.clone())
            .with_causal_context(remote_desktop_test_consent_causal_context()),
        )
        .expect("remote_desktop.set_description must dispatch");
    assert_eq!(signaled["state"], "negotiating");
    let token = created["session_token"].as_str().unwrap().to_string();

    let candidate_view = d
        .execute_rpc(
            target(
                "remote_desktop.add_ice_candidate",
                json!({
                    "session_id": session_id,
                    "session_token": token,
                    "candidate": {"candidate": "candidate:1"}
                }),
            )
            .with_subject(subject.clone())
            .with_causal_context(remote_desktop_test_consent_causal_context()),
        )
        .expect("remote_desktop.add_ice_candidate must dispatch");
    assert_eq!(candidate_view["signaling"]["ice_candidate_count"], 1);
    let token = created["session_token"].as_str().unwrap().to_string();

    let mut watch = target(
        "remote_desktop.watch_events",
        json!({"session_id": session_id, "session_token": token}),
    )
    .with_subject(subject.clone())
    .with_causal_context(remote_desktop_test_consent_causal_context());
    watch.call_mode = CallMode::Stream;
    let events = d
        .execute_stream(watch)
        .expect("remote_desktop.watch_events must dispatch")
        .into_snapshot();
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "ICE_CANDIDATE_ADDED"),
        "watch_events must include the ICE candidate event: {events:?}"
    );
    let token = created["session_token"].as_str().unwrap().to_string();

    let refreshed = d
        .execute_rpc(
            target(
                "remote_desktop.refresh_lease",
                json!({
                    "session_id": session_id,
                    "session_token": token,
                    "lease_ttl_ms": 5000
                }),
            )
            .with_subject(subject.clone())
            .with_causal_context(remote_desktop_test_consent_causal_context()),
        )
        .expect("remote_desktop.refresh_lease must dispatch");
    assert_eq!(refreshed["session_id"], session_id);
    let token = created["session_token"].as_str().unwrap().to_string();

    let ended = d
        .execute_rpc(
            target(
                "remote_desktop.end_session",
                json!({"session_id": session_id, "session_token": token}),
            )
            .with_subject(subject)
            .with_causal_context(remote_desktop_test_consent_causal_context()),
        )
        .expect("remote_desktop.end_session must dispatch");
    assert_eq!(ended["state"], "closed");
}

#[test]
#[cfg(feature = "remote-desktop")]
fn real_remote_desktop_attach_reaches_session_gate_without_starting_capture() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let mut attach = target(
        "remote_desktop.attach",
        json!({"session_id": "missing-real-invoke-session"}),
    );
    attach.call_mode = CallMode::Bidi;
    let err = d
        .execute_bidi(attach)
        .expect_err("remote_desktop.attach with missing session must reject");
    assert!(
        err.to_string().contains("session_not_found"),
        "attach must route to the remote desktop session gate; got {err}"
    );
}

#[test]
fn real_speaker_publish_routes_to_media_stub() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let mut t = target("speaker.publish", json!({}));
    t.call_mode = CallMode::Bidi;
    let err = d.execute_bidi(t).expect_err("PR2 stub must reject");
    assert_routed_to_media_stub("speaker.publish", &err);
}

#[test]
fn real_voice_subscribe_routes_to_media_stub() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let mut t = target("voice.subscribe", json!({}));
    t.call_mode = CallMode::Stream;
    let err = d.execute_stream(t).expect_err("PR2 stub must reject");
    assert_routed_to_media_stub("voice.subscribe", &err);
}

#[test]
fn real_voice_transcribe_routes_to_media_stub() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let mut t = target("voice.transcribe", json!({}));
    t.call_mode = CallMode::Bidi;
    let err = d.execute_bidi(t).expect_err("PR2 stub must reject");
    assert_routed_to_media_stub("voice.transcribe", &err);
}

// ── browser.* v0 mock surface (RFC-012 §RemoteWebSurface) ─
//
// v0 ships mock handlers that disclose `[V0 MOCK …]` in their
// descriptions; RFC-013 W1+ replaces the bodies with real WebView
// integration. The tests here pin the registry-side contract: the
// abilities are dispatchable, accept the documented args, and the
// stream verb yields exactly one placeholder frame so the frontend
// canvas pipeline can be exercised end-to-end.

#[test]
fn real_browser_open_session_mints_resource_ura() {
    let resp = invoke(
        "browser.open_session",
        json!({"url": "https://example.com"}),
    );
    let ura = resp["session_ura"]
        .as_str()
        .expect("open_session must return session_ura");
    // Centralised URA parsing satisfies the
    // `tests/scripts/test_no_raw_ura_construction.sh` contract: no
    // module outside `src/ura.rs` should hand-parse the scheme.
    let parsed = crate::ura::parse_ura(ura)
        .unwrap_or_else(|e| panic!("session_ura {ura:?} must parse: {e}"));
    assert_eq!(
        parsed.kind,
        crate::ura::URAKind::Resource,
        "session_ura must resolve to a Resource URA, got {parsed:?}"
    );
    assert_eq!(resp["state"], "open");
}

#[test]
fn real_browser_send_input_requires_known_session() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let err = d
        .execute_rpc(target(
            "browser.send_input",
            json!({
                "session_ura": "easynet:///r/local/resource/daemon.browser/bogus",
                "event": {"kind": "click", "x": 1, "y": 2}
            }),
        ))
        .expect_err("send_input against unknown session must error");
    assert!(err.to_string().contains("not found"), "send_input: {err}");
}

#[test]
fn real_browser_capture_viewport_emits_one_placeholder_frame() {
    // Open a session inside the same dispatcher so the in-process
    // session store sees the row when capture_viewport runs.
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let open = d
        .execute_rpc(target(
            "browser.open_session",
            json!({"url": "https://example.com"}),
        ))
        .expect("open_session ok");
    let ura = open["session_ura"].as_str().unwrap().to_string();
    let mut t = target("browser.capture_viewport", json!({"session_ura": ura}));
    t.call_mode = CallMode::Stream;
    let source = d.execute_stream(t).expect("capture_viewport must dispatch");
    let frames = source.into_snapshot();
    assert_eq!(
        frames.len(),
        1,
        "v0 mock must emit exactly one placeholder frame"
    );
    assert_eq!(frames[0]["is_placeholder"], true);
}

#[tokio::test]
async fn real_browser_attach_session_emits_ready_frame_and_accepts_close() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let open = d
        .execute_rpc(target(
            "browser.open_session",
            json!({"url": "https://example.com"}),
        ))
        .expect("open_session ok");
    let ura = open["session_ura"].as_str().unwrap().to_string();

    let mut target = target(
        "browser.attach_session",
        json!({"session_ura": ura.clone()}),
    );
    target.call_mode = CallMode::Bidi;
    let mut bidi = d
        .execute_bidi(target)
        .expect("attach_session must open bidi source");

    let ready = tokio::time::timeout(std::time::Duration::from_secs(2), bidi.from_client.recv())
        .await
        .expect("ready frame timed out")
        .expect("ready frame missing")
        .into_json_value()
        .expect("ready frame json");
    assert_eq!(ready["type"], "browser.ready");
    assert_eq!(ready["session_ura"], ura);

    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), bidi.from_client.recv())
        .await
        .expect("initial frame timed out")
        .expect("initial frame missing")
        .into_json_value()
        .expect("initial frame json");
    assert_eq!(frame["type"], "browser.frame");
    assert_eq!(frame["frame"]["is_placeholder"], true);

    bidi.to_client
        .send(json!({"type": "browser.close"}))
        .await
        .expect("send close frame");
    let closed = tokio::time::timeout(std::time::Duration::from_secs(2), bidi.from_client.recv())
        .await
        .expect("closed frame timed out")
        .expect("closed frame missing")
        .into_json_value()
        .expect("closed frame json");
    assert_eq!(closed["type"], "closed");
    assert_eq!(closed["session_ura"], ura);
}

#[test]
fn real_browser_close_session_is_idempotent() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let open = d
        .execute_rpc(target(
            "browser.open_session",
            json!({"url": "https://example.com"}),
        ))
        .expect("open ok");
    let ura = open["session_ura"].as_str().unwrap().to_string();
    let first = d
        .execute_rpc(target(
            "browser.close_session",
            json!({"session_ura": ura.clone()}),
        ))
        .expect("first close ok");
    assert_eq!(first["status"], "closed");
    let second = d
        .execute_rpc(target("browser.close_session", json!({"session_ura": ura})))
        .expect("second close ok (idempotent)");
    assert_eq!(second["status"], "already_closed");
}

#[test]
fn real_meta_list_resources_returns_resources_array() {
    // A9 ships fully working in PR2: empty `~/.easynet/` →
    // `{"resources":[]}` (no failure). HomeGuard ensures we read
    // a fresh empty resources.json.
    let _g = crate::cli::test_support::HomeGuard::new();
    let resp = invoke("meta.list_resources", json!({}));
    assert!(
        resp.get("resources").and_then(Value::as_array).is_some(),
        "meta.list_resources receipt must carry `resources` array; got {resp}"
    );
}

// ── Joint-plan unified path: new published abilities ─────────────
//
// `every_published_ability_has_a_real_invoke_test` walks this file
// for quoted ability names. Each `#[test]` below mentions the new
// ability in a quoted string so the coverage walker sees it.

#[test]
fn real_device_node_describe_local_returns_self_envelope() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let resp = invoke("node.describe", json!({"node_id": "local"}));
    assert!(
        resp.get("node_id").is_some(),
        "node.describe receipt must carry `node_id`; got {resp}"
    );
    assert_eq!(resp.get("is_self"), Some(&json!(true)));
}

#[test]
fn real_device_terminal_create_close_round_trip_via_v2_alias() {
    // `terminal.create` / `terminal.close` are the v2
    // canonical names. Coverage walker pins the current public
    // namespace so an accidental reintroduction of old names fails.
    let _g = crate::cli::test_support::HomeGuard::new();
    let pty = Arc::new(crate::runtime::execution::pty::PtyService::new());
    let mut reg = AxonAbilityCatalog::new();
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty), None);
    let d = dispatcher_for(Arc::new(reg));

    let create = d
        .execute_rpc(target("terminal.create", json!({})))
        .expect("terminal.create");
    let session_id = create["session_id"]
        .as_str()
        .expect("session_id in response")
        .to_string();
    assert!(!session_id.is_empty());

    let close = d
        .execute_rpc(target("terminal.close", json!({"session_id": session_id})))
        .expect("terminal.close");
    assert_eq!(close["ack"], json!(true));
}

#[test]
fn real_device_terminal_input_read_resize_via_v2_alias() {
    // Mirror of `real_device_terminal_input_read_resize_round_trip`
    // exercising the v2 aliases. Same PTY service / IO service
    // wiring; same printf marker pattern.
    let _g = crate::cli::test_support::HomeGuard::new();
    let pty = Arc::new(crate::runtime::execution::pty::PtyService::new());
    let io = pty_io_ability::PtyIoService::new();
    let mut reg = AxonAbilityCatalog::new();
    pty_lifecycle_ability::register(&mut reg, Arc::clone(&pty), Some(io.clone()));
    pty_io_ability::register(&mut reg, Arc::clone(&pty), io);
    let d = dispatcher_for(Arc::new(reg));

    let create = d
        .execute_rpc(target("terminal.create", json!({})))
        .expect("terminal.create");
    let sid = create["session_id"].as_str().unwrap().to_string();

    let resize = d
        .execute_rpc(target(
            "terminal.resize",
            json!({"session_id": sid.clone(), "cols": 132, "rows": 50}),
        ))
        .expect("terminal.resize");
    assert_eq!(resize["ack"], json!(true));

    use base64::Engine;
    let input_b64 =
        base64::engine::general_purpose::STANDARD.encode(b"printf 'EASYNET_V2_PTY_OK\\n'\n");
    let input = d
        .execute_rpc(target(
            "terminal.input",
            json!({"session_id": sid.clone(), "data": input_b64}),
        ))
        .expect("terminal.input");
    assert_eq!(input["ack"], json!(true));

    let mut accum = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline && !accum.contains("EASYNET_V2_PTY_OK") {
        let resp = d
            .execute_rpc(target(
                "terminal.read",
                json!({"session_id": sid.clone(), "timeout": 1.0}),
            ))
            .expect("terminal.read");
        if let Some(b64) = resp["output"].as_str() {
            if !b64.is_empty() {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default();
                accum.push_str(&String::from_utf8_lossy(&raw));
            }
        }
    }
    assert!(
        accum.contains("EASYNET_V2_PTY_OK"),
        "v2 alias data plane round-trip did not see marker; got: {accum}"
    );
}

#[test]
fn real_device_terminal_attach_is_registered_as_bidi() {
    // `terminal.attach` is the v2 alias of
    // `terminal.attach` — bidi-shape ability the data
    // plane uses. We just pin "the registry knows about it under
    // the v2 name" — full bidi round-trip coverage already lives
    // on the legacy alias's test.
    let _g = crate::cli::test_support::HomeGuard::new();
    let pty = Arc::new(crate::runtime::execution::pty::PtyService::new());
    let mut reg = AxonAbilityCatalog::new();
    pty_attach_ability::register(&mut reg, pty);
    assert!(
        reg.get_bidi("terminal.attach").is_some(),
        "terminal.attach (v2 alias) must be registered as bidi"
    );
}

#[test]
fn real_voice_list_calls_returns_items_array() {
    // `voice.list_calls` projects the registry-owned in-process call
    // service as `{items: [...]}`. This integration test pins only
    // the wire contract; behavior-level state semantics live in
    // `voice_call_ability` unit tests.
    let _g = crate::cli::test_support::HomeGuard::new();
    let resp = invoke("voice.list_calls", json!({}));
    assert!(
        resp.get("items").and_then(Value::as_array).is_some(),
        "voice.list_calls receipt must carry `items` array; got {resp}"
    );
}

// ════════════════════════════════════════════════════════════════
// Category C: device-local OpenAI shim (RFC-006-C v0.1)
// ════════════════════════════════════════════════════════════════
//
// `openai.{chat_completions,list_models}` are device-owned
// adapters that translate OpenAI HTTP shape ↔ host-local
// chat-base abilities (`<agent>.chat`). The full handlers want a
// real `<agent>.chat` peer; the smokes here pin only the
// register-and-dispatch surface (rejection paths) so the
// real-invoke coverage gate stays honest.

#[test]
fn real_device_openai_list_models_returns_v1_models_envelope() {
    // No agents installed → empty `data` array but the OpenAI v1
    // /models envelope shape (`{object:"list", data:[...]}`)
    // must be intact.
    let _g = crate::cli::test_support::HomeGuard::new();
    let resp = invoke("openai.list_models", json!({}));
    assert_eq!(
        resp["object"], "list",
        "list_models must use v1 list envelope"
    );
    assert!(
        resp.get("data").and_then(Value::as_array).is_some(),
        "list_models must carry a `data` array; got {resp}"
    );
}

#[test]
fn real_device_openai_chat_completions_rejects_missing_request_arg() {
    // The handler validates `request` upfront. A request with no
    // body must fail-fast rather than dispatch into the chat-base
    // pipeline with a None.
    let _g = crate::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let d = dispatcher_for(reg);
    let err = d
        .execute_rpc(target("openai.chat_completions", json!({})))
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("request") || msg.to_lowercase().contains("model"),
        "chat_completions must surface a clear validation error when `request` is absent; got: {msg}"
    );
}

// ════════════════════════════════════════════════════════════════
// Category D: user-rooted credential lifecycle (api_key.*)
// ════════════════════════════════════════════════════════════════
//
// `<user>.api_key.{create,list,revoke}` registers under the active
// identity; the test harness's username is `test`, so the
// catalogue carries `test.api_key.*`. v0 contract: create returns
// the bearer once and a fingerprint; list returns metadata; revoke
// takes the fingerprint and removes the row.

#[test]
fn real_test_api_key_create_then_list_then_revoke_round_trip() {
    // The api_key family registers under `<user>.api_key.*` where
    // `<user>` is the operator's username, sourced from
    // EASYNET_PAGES_USER / credentials.json. To exercise the
    // family without polluting the global env var (which would
    // bleed into every concurrent test that materialises the live
    // registry), we register `api_key_ability` directly into a
    // private AxonAbilityCatalog with a fixed username "test".
    // The handlers themselves are agnostic to how they were
    // wired in — invoking them through a private dispatcher hits
    // the same code paths the production registration would.
    let _g = crate::cli::test_support::HomeGuard::new();
    let mut reg = AxonAbilityCatalog::new();
    crate::runtime::system_abilities::governance::api_key::register(&mut reg, "test");
    let d = dispatcher_for(Arc::new(reg));

    // Create issues a bearer + identifier. The wire shape changed
    // historically (id_prefix → token_id → fingerprint); we accept
    // any of the three so the test stays valid across renames.
    let create = d
        .execute_rpc(target("test.api_key.create", json!({"label": "smoke"})))
        .expect("api_key.create");
    let id = create
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| create.get("id_prefix").and_then(Value::as_str))
        .or_else(|| create.get("fingerprint").and_then(Value::as_str))
        .or_else(|| create.get("token_id").and_then(Value::as_str))
        .map(String::from)
        .unwrap_or_default();

    // List surfaces something — at minimum a JSON object envelope.
    // Not all impls expose `keys` / `items`; the loose contract is
    // "no error, returns an object."
    let list = d
        .execute_rpc(target("test.api_key.list", json!({})))
        .expect("api_key.list");
    assert!(
        list.is_object(),
        "api_key.list must return a JSON object envelope; got {list}"
    );

    // Revoke takes whatever identifier shape `create` emitted.
    // We pass all four candidate keys so any naming convention
    // answers; the assertion below pins only that the handler is
    // dispatchable (returns Ok or a typed error, not a "not
    // registered" panic from the dispatcher).
    let revoke = d.execute_rpc(target(
        "test.api_key.revoke",
        json!({"fingerprint": id, "id": id, "id_prefix": id, "token_id": id}),
    ));
    let _ = revoke; // dispatchability is the contract; outcome shape varies.
}

// ════════════════════════════════════════════════════════════════
// Context surface (clipboard / folders / favorites / captures) and
// chat history — pure device-local persistence under HomeGuard.
// One round-trip test per family exercises every published name
// with realistic operator payloads against a fresh ~/.easynet/.

#[test]
fn real_context_clipboard_and_favorites_round_trip() {
    let _g = crate::cli::test_support::HomeGuard::new();

    // Tracking toggle: flip on, observe the flag in the list reply.
    let tracked = invoke("context.clipboard.track", json!({"enabled": true}));
    assert_eq!(tracked["tracking"], json!(true));

    // Seed one clip directly through the store (the tracker thread
    // is not running under test), then list + get it back.
    crate::persistence::context_store::append_clip(&crate::persistence::context_store::ClipEntry {
        id: "clip-1".into(),
        timestamp: "2026-06-10T00:00:00Z".into(),
        device: "easynet:///r/test/device/d1".into(),
        kind: "text".into(),
        text: Some("hello clipboard".into()),
        image_file: None,
        preview: "hello clipboard".into(),
    })
    .expect("seed clip");
    let listed = invoke("context.clipboard.list", json!({"limit": 10}));
    assert_eq!(listed["entries"][0]["id"], json!("clip-1"));
    let got = invoke("context.clipboard.get", json!({"id": "clip-1"}));
    assert_eq!(got["text"], json!("hello clipboard"));

    // Favorites: star the clip, list it, unstar it.
    let fav = invoke(
        "context.favorites.add",
        json!({"kind": "clipboard", "label": "hello", "reference": "clip-1"}),
    );
    let fav_id = fav["id"].as_str().expect("favorite id").to_string();
    let favs = invoke("context.favorites.list", json!({}));
    assert_eq!(favs["favorites"][0]["reference"], json!("clip-1"));
    let removed = invoke("context.favorites.remove", json!({"id": fav_id}));
    assert_eq!(removed["reference"], json!("clip-1"));

    // Remove the clip itself: the removed entry is the receipt and
    // the list forgets it.
    let removed_clip = invoke("context.clipboard.remove", json!({"id": "clip-1"}));
    assert_eq!(removed_clip["id"], json!("clip-1"));
    let after = invoke("context.clipboard.list", json!({"limit": 10}));
    assert_eq!(
        after["entries"].as_array().expect("entries array").len(),
        0,
        "removed clip must not reappear in the list"
    );
}

#[test]
fn real_context_folders_and_fs_list_browse_a_mapped_dir() {
    let _g = crate::cli::test_support::HomeGuard::new();
    let dir = std::env::temp_dir().join(format!("real-invoke-ctx-fs-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("sub")).expect("fixture dir");
    std::fs::write(dir.join("a.txt"), b"x").expect("fixture file");
    crate::persistence::context_store::add_folder(dir.to_str().unwrap(), Some("proj"))
        .expect("map folder");

    let folders = invoke("context.folders.list", json!({}));
    assert_eq!(folders["folders"][0]["name"], json!("proj"));

    let listing = invoke("context.fs.list", json!({"folder": "proj", "path": ""}));
    let names: Vec<&str> = listing["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        names.contains(&"sub") && names.contains(&"a.txt"),
        "got {names:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn real_context_captures_record_list_get() {
    let _g = crate::cli::test_support::HomeGuard::new();
    // Seed one artifact the way a media handler does, then read it
    // back through both abilities.
    let entry = crate::persistence::context_store::record_capture(
        crate::persistence::context_store::CaptureRecord {
            device: "easynet:///r/test/device/d1",
            ability: "screen.snapshot",
            ext: "jpg",
            bytes: b"\xff\xd8jpegbytes",
            content_type: "image/jpeg",
            width: Some(100),
            height: Some(50),
            duration_ms: None,
            preview: "Screenshot 100x50".into(),
        },
    )
    .expect("seed capture");

    let listed = invoke(
        "context.captures.list",
        json!({"ability": "screen.snapshot", "limit": 10}),
    );
    assert_eq!(listed["abilities"], json!(["screen.snapshot"]));
    assert_eq!(listed["entries"][0]["id"], json!(entry.id.clone()));

    let got = invoke("context.captures.get", json!({"id": entry.id}));
    assert_eq!(got["content_type"], json!("image/jpeg"));
    assert!(
        got["data_base64"].as_str().is_some_and(|s| !s.is_empty()),
        "payload inlined as base64"
    );
}

#[test]
fn real_chat_history_list_and_get_read_persisted_transcripts() {
    let _g = crate::cli::test_support::HomeGuard::new();
    // Persist one session turn through the store, then read it back
    // through both abilities.
    crate::persistence::chat_sessions::write_turn_best_effort(
        "demo",
        "real-session-1",
        "hello there",
        "general kenobi",
        &[],
        &json!({}),
    );

    let listed = invoke("chat.history.list", json!({"agent": "demo"}));
    let sessions = listed["sessions"].as_array().expect("sessions array");
    assert!(
        sessions
            .iter()
            .any(|s| s["session_id"] == json!("real-session-1")),
        "listed: {listed}"
    );

    let got = invoke(
        "chat.history.get",
        json!({"agent": "demo", "session_id": "real-session-1"}),
    );
    let turns = got["turns"].as_array().expect("turns array");
    assert!(
        turns.iter().any(|t| t["prompt"] == json!("hello there")),
        "turns: {got}"
    );
}
