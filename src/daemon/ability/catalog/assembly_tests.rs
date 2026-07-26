//! Assembly-level tests for the system ability registry + catalogue surface.

use super::*;
use crate::daemon::ability::catalog::{
    build_registry, build_registry_for_daemon_result, build_registry_with_services_result,
    build_system_registry, published_abilities, published_ability_names,
    published_system_abilities, recover_descriptor_import_transactions_before_daemon_registry_boot,
    RegistryBuildConfig, RegistryBuildServices, RegistryDaemonBuildConfig,
};
use crate::daemon::persistence::agent_registry::AgentRegistry;
use std::sync::Arc;

fn registry_config_for_agents(agents: &AgentRegistry) -> RegistryBuildConfig<'_> {
    let hosted_agent_roots = agents.agents.keys().map(|key| {
        let agent_id = crate::core::agent::id::AgentId::parse(key)
            .expect("assembly-test AgentRegistry keys must be canonical AgentId values");
        crate::core::ura::agent_ura("localhost", "dev", &agent_id.name)
    });
    let authority_context = crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root_with_hosted_agents(
        crate::core::ura::device_ura("localhost", "dev"),
        hosted_agent_roots,
    )
    .expect("build explicit assembly-test Device authority with hosted Agent inventory");
    registry_config_for_agents_with_authority(agents, authority_context)
}

fn registry_config_for_agents_with_authority(
    agents: &AgentRegistry,
    authority_context: crate::daemon::ability::dispatch::AbilityAuthorityContext,
) -> RegistryBuildConfig<'_> {
    let mut config = RegistryBuildConfig::new_with_authority_context(
        RegistryBuildServices::fresh(),
        agents,
        authority_context,
    );
    config.local_runtime = Some(
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        ),
    );
    config
}

fn sha256_hex_for_test(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn default_voice_capability_state_evidence(
) -> Vec<crate::daemon::ability::conformance::VoiceCapabilityStateEvidence> {
    crate::daemon::ability::conformance::voice_capability_state_evidence(
        crate::daemon::ability::conformance::VoiceAssemblyEvidence::default(),
    )
}

fn provider_backed_voice_capability_state_evidence(
) -> Vec<crate::daemon::ability::conformance::VoiceCapabilityStateEvidence> {
    crate::daemon::ability::conformance::voice_capability_state_evidence(
        crate::daemon::ability::conformance::VoiceAssemblyEvidence {
            repository_assembled: true,
            executable_delivery_evidence: false,
        },
    )
}

/// Seed `local-agents.json` with the canonical hosted-agent identities the
/// hot registrar requires before it will register `<agent>.chat` into the
/// runtime. Each tuple is `(profile, name)`; the agent URA is the canonical
/// `agent/<name>.<profile>` form under the local realm. Call inside a
/// `HomeGuard` so the file lands in the test's temp HOME.
fn seed_hosted_agents_for_chat(agent_names: &[&str]) {
    use crate::daemon::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};
    use crate::daemon::persistence::{agent_registry, config};
    config::save_credentials(&config::Credentials {
        node_id: "dev".to_string(),
        credential_token: "token".to_string(),
        hub_endpoint: "axon://hub.test:50051".to_string(),
        realm: "localhost".to_string(),
        username: Some("dev".to_string()),
        user_id: Some("user-dev".to_string()),
        ..Default::default()
    })
    .expect("seed paired Device credentials");
    let mut local = LocalAgentsFile {
        host_device_agent_ura: crate::core::ura::device_ura("localhost", "dev"),
        ..LocalAgentsFile::default()
    };
    let mut durable = AgentRegistry::default();
    for name in agent_names {
        let agent_ura = crate::core::ura::agent_ura("localhost", "dev", name);
        upsert_hosted_agent(&mut local, "llm", name, &agent_ura);
        durable.agents.insert(
            canonical_test_agent_registry_key(name),
            test_agent_entry(name),
        );
    }
    save(&local).expect("seed local-agents.json for chat handlers");
    agent_registry::save_agents(&durable).expect("seed durable Agent registry");
}

fn canonical_test_agent_registry_key(name: &str) -> String {
    crate::core::agent::id::AgentId::parse(name)
        .expect("test agent id must be canonicalizable")
        .to_string()
}

fn test_agent_entry(name: &str) -> crate::daemon::persistence::agent_registry::AgentEntry {
    let mut entry = crate::daemon::persistence::agent_registry::AgentEntry::new(
        crate::daemon::persistence::agent_registry::AgentType::ClaudeCode,
        None,
    );
    entry.root_path = Some(crate::daemon::persistence::config::agents_root().join(name));
    entry
}

#[test]
fn daemon_registry_boot_hook_recovers_acquiring_descriptor_imports() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();

    let home = std::env::var("HOME").expect("HomeGuard sets HOME");
    let manifest_path =
        std::path::Path::new(&home).join("agents/apprentice/abilities/quote.ability.toml");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
        .expect("create manifest dir");
    let manifest =
        b"name = \"quote\"\ndescription = \"imported\"\n\n[input_schema]\ntype = \"object\"\n";
    std::fs::write(&manifest_path, manifest).expect("write imported descriptor");
    let manifest_hash = sha256_hex_for_test(manifest);

    let grants_path = crate::daemon::persistence::teach_grants::path();
    std::fs::create_dir_all(grants_path.parent().expect("teach grants parent"))
        .expect("create state dir");
    std::fs::write(
        &grants_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "5",
            "grants": [],
            "imports": [{
                "ability_name": "quote",
                "learner_agent": "apprentice",
                "source_descriptor_ura": "easynet:///r/localhost/agent/dev.mentor/ability/quote",
                "manifest_hash": manifest_hash,
                "imported_at": "2026-06-23T01:02:03Z",
                "state": "acquiring",
                "acquiring_manifest_path": manifest_path.to_string_lossy(),
                "acquiring_staging_manifest_path": null,
                "acquiring_manifest_hash": manifest_hash,
                "pending_grant": null
            }]
        }))
        .expect("serialize grants"),
    )
    .expect("write acquiring teach grants fixture");

    let recovered = recover_descriptor_import_transactions_before_daemon_registry_boot()
        .expect("daemon boot recovery hook");
    assert_eq!(recovered, 1);

    let recovered: serde_json::Value =
        serde_json::from_slice(&std::fs::read(grants_path).expect("read recovered grants"))
            .expect("parse recovered grants");
    assert_eq!(recovered["imports"][0]["state"], "active");
    assert!(recovered["imports"][0]["acquiring_manifest_path"].is_null());
    assert!(recovered["imports"][0]["acquiring_manifest_hash"].is_null());
}

#[test]
fn published_ability_names_contains_agent_list_and_terminal_list() {
    // Diagnostic for the production NODATA: agent.list resolves but
    // terminal.list does not. Both are OwnerKind::Device RPC abilities
    // and both must be in the published set that (a) drives the device
    // profile and (b) is registered directly into the daemon's live,
    // embedded LocalRuntime during catalog assembly.
    let names = published_ability_names();
    assert!(
        names.iter().any(|n| n == "agent.list"),
        "agent.list missing from published names"
    );
    assert!(
        names.iter().any(|n| n == "terminal.list"),
        "terminal.list missing from published names; got {names:?}"
    );
}

#[test]
fn build_registry_publishes_canonical_descriptors_for_device_media_and_remote_desktop() {
    let reg = build_registry();
    let rows = reg.authority_ability_catalog_snapshot();

    for ability in [
        "mic.subscribe",
        "camera.snapshot",
        "camera.subscribe",
        "screen.snapshot",
        "screen.subscribe",
        "remote_desktop.create_session",
        "remote_desktop.attach",
    ] {
        let row = rows
            .iter()
            .find(|row| row.name == ability)
            .unwrap_or_else(|| panic!("{ability} must be registered at daemon boot"));
        assert_eq!(
            row.descriptor.input_schema()["type"],
            serde_json::json!("object"),
            "{ability} must expose an object input schema"
        );
    }
}

#[test]
fn terminal_list_is_owner_kind_device() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    use crate::daemon::ability::dispatch::OwnerKind;
    assert_eq!(
        system_ability_owner("terminal.list"),
        Some(OwnerKind::Device)
    );
    assert_eq!(system_ability_owner("agent.list"), Some(OwnerKind::Device));
}

#[test]
fn discovery_hints_read_only_tracks_ability_layer() {
    // The read_only discovery hint is the wire carrier of ability
    // purity: it rides meta.list_abilities to the catalog so the
    // frontend coalesces pure-read invokes without re-classifying.
    // Pin the layer→hint mapping so a future classification change
    // can't silently flip a verb's coalescability.
    let reg = build_registry();
    // Introspection read → read_only + idempotent.
    let h = discovery_hints_for(&reg, "meta.list_resources");
    assert!(h.read_only && h.idempotent, "introspection read: {h:?}");
    for name in [
        "agent.list",
        "invocation.history.list",
        "invocation.history.path",
        "meta.list_resources",
    ] {
        let h = discovery_hints_for(&reg, name);
        assert!(
            h.read_only && h.idempotent,
            "registered introspection read: {name}: {h:?}"
        );
    }
    // Observation read → read_only + idempotent.
    let h = discovery_hints_for(&reg, "observe.health");
    assert!(h.read_only && h.idempotent, "observation read: {h:?}");
    // Control decision → idempotent but NOT read_only.
    let h = discovery_hints_for(&reg, "consent.decide");
    assert!(!h.read_only && h.idempotent, "control decision: {h:?}");
    // Operational verb → neither (side effects; never coalescable).
    let h = discovery_hints_for(&reg, "screen.snapshot");
    assert!(!h.read_only && !h.idempotent, "operational verb: {h:?}");
    let h = discovery_hints_for(&reg, "remote_desktop.create_session");
    assert!(!h.read_only, "create_session must not be read_only: {h:?}");
}

#[test]
fn deterministic_registry_snapshot_does_not_replay_hosted_agent_runtime() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();

    let reg = build_registry();
    let h = discovery_hints_for(&reg, "agent.list");

    assert!(
        h.read_only && h.idempotent,
        "deterministic registry snapshots must publish static Device abilities without replaying hosted-Agent runtimes: {h:?}"
    );
}

#[test]
fn ability_layer_classification_is_complete() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    // The audit story (RFC docs/AXON-RFC-001-ability-layers.md)
    // says every published ability MUST belong to exactly one
    // semantic layer. A new ability that lands without a
    // classify_ability arm trips this test, forcing the author
    // to either pick a layer or amend the layer doc.
    let names: Vec<String> = published_system_abilities()
        .into_iter()
        .map(|meta| meta.name)
        .collect();
    let unclassified: Vec<String> = names
        .iter()
        // device.keyring.* abilities have their own ontology
        // (RFC-002 §3.3) and are not classified by the system
        // ability layer table.
        .filter(|n| !n.starts_with("device.keyring."))
        .filter(|n| classify_ability(n).is_none())
        .cloned()
        .collect();
    assert!(
        unclassified.is_empty(),
        "abilities missing a layer classification: {unclassified:?}\n\
             Add an arm to classify_ability() in src/daemon/ability/catalog/assembly_tests.rs \
             and update docs/rfc/AXON-RFC-001-ability-layers.md."
    );
}

#[test]
fn introspection_layer_includes_all_three_discovery_planes() {
    // The discovery-planes invariant from
    // docs/rfc/AXON-RFC-001-discovery-planes.md: meta.list_abilities,
    // mcp.bridge.list_tools, and a2a.bridge.list_skills MUST all
    // classify as Introspection. A regression that moved one of
    // them to a different layer would fragment the discovery story.
    for name in [
        "meta.list_abilities",
        "mcp.bridge.list_tools",
        "a2a.bridge.list_skills",
    ] {
        assert_eq!(
            classify_ability(name),
            Some(AbilityLayer::Introspection),
            "{name} must classify as Introspection (discovery plane)"
        );
    }
}

#[test]
fn build_registry_is_non_empty_and_includes_ping() {
    // Every v1 daemon publishes at least `observe.health` so a
    // peer wanting to test reachability has a known ability.
    // A regression that emptied this list would silently break
    // discovery + smoke tests.
    let reg = build_registry();
    let names = reg.list_abilities();
    assert!(
        names.iter().any(|n| n == "observe.health"),
        "observe.health must be in the v1 registry; got {names:?}"
    );
}

#[test]
fn published_ability_names_matches_live_registry() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    // The label-publishing helper and the publishable subset of the dispatch
    // registry must agree byte-for-byte. Local-only front doors may still live
    // in the registry for CLI/runtime calls, but must not be advertised.
    let live: Vec<String> = build_registry()
        .list_abilities()
        .into_iter()
        .filter(|name| is_publishable_catalog_name(name))
        .collect();
    let advertised = published_ability_names();
    assert_eq!(live, advertised);
}

#[test]
fn companion_control_abilities_are_local_only() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let registry = build_registry();
    let companion_controls = [
        crate::daemon::ability::builtins::integrations::plugins::COMPANION_STATUS_ABILITY,
        crate::daemon::ability::builtins::integrations::plugins::COMPANION_RECONCILE_ABILITY,
    ];

    for ability in companion_controls {
        assert!(
            registry.has_rpc(ability),
            "{ability} must stay callable through the local daemon registry"
        );
        assert!(
            !is_publishable_catalog_name(ability),
            "{ability} must be excluded from public catalogue publication"
        );
    }

    let published_names: std::collections::BTreeSet<String> =
        published_ability_names().into_iter().collect();
    let system_names: std::collections::BTreeSet<String> = published_system_abilities()
        .into_iter()
        .map(|metadata| metadata.name)
        .collect();

    for ability in companion_controls {
        assert!(
            !published_names.contains(ability),
            "{ability} must not be advertised through published_ability_names"
        );
        assert!(
            !system_names.contains(ability),
            "{ability} must not produce public system descriptors"
        );
    }
}

#[test]
fn every_published_ability_has_a_toml_byte_for_byte_matching_the_renderer() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    // The TOML descriptors in ability-descriptors/system/ are the
    // source of truth for external discovery tools. They are
    // GENERATED from the canonical descriptor contract inventory. This
    // test enforces that the on-disk file is byte-for-byte
    // identical to what the renderer produces; if the
    // dispatcher's metadata changed and a maintainer forgot
    // to regenerate, this test names every drifted ability
    // and tells them how to fix it.
    let mut missing: Vec<String> = Vec::new();
    let mut drift: Vec<String> = Vec::new();
    for meta in system_ability_contract_inventory() {
        let toml_path = descriptor_path_for(&meta.name);
        let on_disk = match std::fs::read_to_string(&toml_path) {
            Ok(body) => body,
            Err(_) => {
                missing.push(meta.name.clone());
                continue;
            }
        };
        let expected = ability_toml::render_ability_contract_toml(&meta);
        if on_disk != expected {
            drift.push(meta.name.clone());
        }
    }
    let mut errors: Vec<String> = Vec::new();
    if !missing.is_empty() {
        errors.push(format!(
            "no TOML on disk for: {missing:?}\n\
                 -> run `cargo run --bin gen-ability-tomls` to create them"
        ));
    }
    if !drift.is_empty() {
        errors.push(format!(
            "TOML on disk differs from renderer output for: {drift:?}\n\
                 -> run `cargo run --bin gen-ability-tomls` to regenerate"
        ));
    }
    assert!(
        errors.is_empty(),
        "ability TOML descriptor drift:\n  {}",
        errors.join("\n  ")
    );
}

#[test]
fn contract_inventory_retains_voice_without_leaking_it_into_live_inventory() {
    let contract_inventory = system_ability_contract_inventory();
    let live_names = published_system_abilities()
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect::<std::collections::BTreeSet<_>>();

    for evidence in default_voice_capability_state_evidence() {
        let contract = contract_inventory
            .iter()
            .find(|contract| contract.name == evidence.name)
            .unwrap_or_else(|| panic!("{} missing from contract inventory", evidence.name));
        assert_eq!(contract.call_mode, evidence.call_mode);
        assert_eq!(contract.capability_state, evidence.state);
        assert!(
            !live_names.contains(evidence.name),
            "{} contract must not become operational inventory",
            evidence.name
        );
        assert!(
            std::path::Path::new(&descriptor_path_for(evidence.name)).exists(),
            "{} contract descriptor must remain on disk",
            evidence.name
        );
    }

    use crate::daemon::ability::descriptors::AdmissionAction;
    let action_for = |name: &str| {
        contract_inventory
            .iter()
            .find(|contract| contract.name == name)
            .map(|contract| contract.admission_action)
            .unwrap_or_else(|| panic!("{name} missing from contract inventory"))
    };
    assert_eq!(action_for("voice.show_call"), AdmissionAction::Read);
    assert_eq!(action_for("voice.list_calls"), AdmissionAction::Read);
    assert_eq!(action_for("voice.report_metrics"), AdmissionAction::Invoke);
    assert_eq!(action_for("voice.subscribe"), AdmissionAction::Stream);
}

/// Walk the canonical published system descriptor set and confirm a handler is
/// registered under the descriptor's invocation mode. The descriptor set is
/// deliberately the baseline: deriving the baseline from `list_abilities()`
/// would hide a descriptor whose registration was omitted entirely.
///
/// What this DOES NOT verify:
///   * the handler implementation is correct (most need
///     valid args, services, real I/O — those have their
///     own per-module tests),
///   * the response shape matches the documented schema,
///   * end-to-end behavior over the wire.
/// What it DOES verify:
///   * `register(...)` was called for every published name
///     (catches the slice-16 bug class: file present, never
///     wired into build_registry_with_services),
///   * the registration mode matches the dispatcher's
///     expectation (catches "registered as Stream but
///     get_rpc() returns None" type mismatches).
#[test]
fn every_published_ability_resolves_to_a_handler() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let reg = build_registry();
    let daemon_invocation_surface: std::collections::BTreeSet<&'static str> =
        crate::daemon::ability::conformance::HubBaseline::required_abilities()
            .into_iter()
            .filter(|ability| {
                ability.surface
                    == crate::daemon::ability::conformance::BaselineSurface::DaemonInvocation
            })
            .map(|ability| ability.name)
            .collect();
    let mut unresolved: Vec<String> = Vec::new();
    let mut wrong_mode: Vec<String> = Vec::new();
    for metadata in published_system_abilities() {
        let name = metadata.name;
        if daemon_invocation_surface.contains(name.as_str()) {
            continue;
        }
        let has_rpc = reg.has_rpc(&name);
        let has_stream = reg.has_stream(&name);
        let has_bidi = reg.has_bidi(&name);
        if !(has_rpc || has_stream || has_bidi) {
            unresolved.push(name);
            continue;
        }

        let expected_mode_is_registered = if metadata.hints.streaming_only {
            has_stream
        } else if metadata.hints.bidi_only {
            has_bidi
        } else {
            has_rpc
        };
        if !expected_mode_is_registered {
            wrong_mode.push(format!(
                "{}: expected={}, registered=[rpc:{has_rpc}, stream:{has_stream}, bidi:{has_bidi}]",
                name,
                if metadata.hints.streaming_only {
                    "stream"
                } else if metadata.hints.bidi_only {
                    "bidi"
                } else {
                    "rpc"
                }
            ));
        }
    }
    assert!(
        unresolved.is_empty() && wrong_mode.is_empty(),
        "published system abilities are not executable:\n  missing handlers: {unresolved:?}\n  wrong modes: {wrong_mode:?}"
    );
}

/// For a bounded, low-cost set of read-only RPC-mode abilities,
/// actually invoke through the dispatcher with `{}` and confirm the
/// call returns SOME result (Ok(value) or a structured Err). This
/// exercises registry lookup, LocalRuntime dispatch, handler
/// invocation, and response materialisation without turning the
/// assembly suite into a daemon-runtime integration test.
///
/// Deliberately do NOT invoke every RPC ability. Operational verbs
/// such as `agent.refresh`, `voice.create_call`, `skill.install`,
/// `process.exec`, and bridge calls are real work surfaces. A broad
/// assembly test must not mutate `~/.easynet`, create daemon state, or
/// wait on device/network/process paths just to prove registration. We
/// also skip effect-free but heavyweight discovery aggregators such as
/// `meta.list_abilities`, `mcp.bridge.list_tools`, and A2A/MCP client
/// surfaces because their handler path rebuilds or reflects the
/// catalogue. Full-coverage handler behavior belongs in per-ability
/// tests.
#[test]
fn every_rpc_ability_actually_dispatches_through_to_its_handler() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    use crate::daemon::invocation::routing::target::CallMode;

    let reg = build_system_registry();
    let dispatcher = Arc::clone(&reg);
    let names = reg.list_abilities();

    let mut invoked_ok: Vec<String> = Vec::new();
    let mut invoked_err: Vec<(String, String)> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();
    let mut not_rpc: Vec<String> = Vec::new();
    let mut skipped_effectful_or_expensive: Vec<String> = Vec::new();

    for name in &names {
        // Only invoke things that ARE RPC. Stream / Bidi abilities
        // skip this stage; the previous test confirmed they have a
        // handler under their mode. Use has_rpc(), not get_rpc(), so
        // envelope-aware RPC handlers count too.
        if !reg.has_rpc(name) {
            not_rpc.push(name.clone());
            continue;
        }
        let hints = discovery_hints_for(&reg, name);
        if !hints.read_only || !is_fast_read_only_smoke_ability(name) {
            skipped_effectful_or_expensive.push(name.clone());
            continue;
        }
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                name.clone(),
                serde_json::json!({}),
                CallMode::Rpc,
            );
        match dispatcher.execute_rpc(target) {
            Ok(_) => invoked_ok.push(name.clone()),
            Err(e) => {
                let msg = format!("{e}");
                if msg.to_ascii_lowercase().contains("no rpc handler")
                    || msg.contains("ABILITY_NOT_FOUND")
                {
                    not_found.push(name.clone());
                } else {
                    invoked_err.push((name.clone(), msg));
                }
            }
        }
    }

    // Test-only diagnostic summary so a green run still shows
    // what was actually exercised (visible with
    // `cargo test ... -- --nocapture`). These are deliberately
    // raw `eprintln!` and NOT `op_event!`: they are test-binary
    // human-readable output, not daemon operator events, so
    // the `[component] kind=event` schema would only add
    // unrelated noise to a developer's terminal. The `op_event!`
    // discipline applies to daemon runtime code paths,
    // not to the test bodies that exercise them.
    eprintln!(
        "ability invoke smoke: {} OK, {} errored-but-reached-handler, {} skipped (non-RPC)",
        invoked_ok.len(),
        invoked_err.len(),
        not_rpc.len()
    );
    if !invoked_err.is_empty() {
        eprintln!("  errored (handler reached, normal):");
        for (n, m) in &invoked_err {
            let preview: String = m.chars().take(80).collect();
            eprintln!("    {n}: {preview}");
        }
    }
    if !not_rpc.is_empty() {
        eprintln!("  skipped (registered as Stream/Bidi): {not_rpc:?}");
    }
    if !skipped_effectful_or_expensive.is_empty() {
        eprintln!("  skipped (effectful or expensive RPC): {skipped_effectful_or_expensive:?}");
    }

    assert!(
        !invoked_ok.is_empty() || !invoked_err.is_empty(),
        "fast read-only RPC smoke must exercise at least one handler; \
         check discovery_hints_for/read_only classification and \
         is_fast_read_only_smoke_ability"
    );
    assert!(
        skipped_effectful_or_expensive
            .iter()
            .any(|name| name == "process.exec"),
        "effectful RPCs must stay out of the broad smoke path; \
         skipped set was {skipped_effectful_or_expensive:?}"
    );
    assert!(
        not_found.is_empty(),
        "abilities advertised but dispatcher could not find an RPC handler: {not_found:?}"
    );
}

/// Returns the read-only RPC verbs that are safe for a broad assembly
/// smoke test to call with `{}`. This is not a second source of ability
/// purity; `discovery_hints_for(...).read_only` remains the purity
/// gate above. The allow-list only answers the separate test-engineering
/// question "is this pure read cheap and environment-independent enough
/// to execute in a registry-wide smoke".
fn is_fast_read_only_smoke_ability(name: &str) -> bool {
    matches!(
        name,
        "observe.health"
            | "observe.network_health"
            | "admin.status"
            | "agent.list"
            | "terminal.list"
            | crate::daemon::ability::names::device_control::SESSION_LIST
            | "consent.list_pending"
            | "schedule.list"
            | "plugin.status"
            | "meta.list_resources"
            | "context.catalog"
            | "context.clipboard.list"
            | "context.folders.list"
            | "context.favorites.list"
            | "context.captures.list"
    )
}

#[test]
fn build_registry_actually_contains_every_baseline_locomotion_ability() {
    // Pin the AXIOM Tier 2.5 surface: every member of the
    // Baseline Locomotion Profile MUST be registered in the
    // live registry. A regression that adds a `pub mod` but
    // forgets the `register(&mut reg)` call would leave the
    // ability invisible to the dispatcher even though the
    // module compiles. This test catches that.
    let reg = build_registry();
    let names: std::collections::BTreeSet<String> = reg.list_abilities().into_iter().collect();
    let must_have = [
        // Filesystem half
        "fs.read",
        "fs.write",
        "fs.list",
        "fs.edit",
        // Execution half
        "process.exec",
        "shell.run",
        // Outbound network
        "http.request",
        // Interactive PTY trio
        "terminal.create",
        "terminal.list",
        "terminal.close",
        "terminal.attach",
        // Operator surface added in slice 16
        "admin.status",
        "agent.start",
        "agent.stop",
        "agent.purge",
        "agent.refresh",
    ];
    let missing: Vec<&str> = must_have
        .iter()
        .filter(|n| !names.contains(**n))
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "Baseline Locomotion abilities NOT registered: {missing:?}.\n\
             Live registry has {} abilities: {:?}",
        names.len(),
        names
    );
}

#[test]
fn agent_purge_descriptor_is_destructive_but_agent_stop_is_not() {
    let descriptors = published_system_abilities();
    let stop = descriptors
        .iter()
        .find(|descriptor| descriptor.name == "agent.stop")
        .expect("agent.stop descriptor");
    let purge = descriptors
        .iter()
        .find(|descriptor| descriptor.name == "agent.purge")
        .expect("agent.purge descriptor");

    assert!(!stop.hints.destructive);
    assert!(purge.hints.destructive);
    assert_ne!(stop.name, purge.name);
}

#[test]
fn build_registry_satisfies_device_baseline_contract() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let reg = build_registry();
    let device = crate::daemon::ability::conformance::DeviceBaseline::required_abilities();
    let report = crate::daemon::ability::conformance::RegistryConformance::new(&reg)
        .check("device", &device);

    assert!(
        report.is_conformant(),
        "Device baseline abilities missing or registered under the wrong call mode:\n  {}",
        report.panic_message()
    );
}

#[test]
fn default_registry_build_uses_device_authority_profile_without_realm_authority_rows() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let registry = build_registry();
    let rows = registry.authority_ability_catalog_snapshot();

    assert!(
        !rows.is_empty(),
        "default Device registry must not be empty"
    );
    assert!(
        rows.iter()
            .all(|row| row.owner != crate::daemon::ability::dispatch::OwnerKind::RealmAuthority),
        "default RegistryBuildConfig leaked RealmAuthority rows: {rows:?}"
    );
}

#[test]
fn combined_registry_binds_local_introspection_to_distinct_device_and_realm_authority_roots() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let agents = AgentRegistry::default();
    let device_ura = crate::core::ura::device_ura("realm-b", "dev-b");
    let hub_ura = crate::core::ura::hub_ura("realm-b");
    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots(
            device_ura.clone(),
        )
        .expect("combined authority context");
    let config = registry_config_for_agents_with_authority(&agents, authority_context);
    let registry = build_registry_with_services_result(config)
        .expect("assemble registry")
        .catalog;

    assert!(
        registry.static_authority_exclusion_snapshot().is_empty(),
        "combined authority set must admit every built-in owner plane"
    );

    for owner_ura in [&device_ura, &hub_ura] {
        let record = registry
            .control_plane_record_for_authority_mode(
                owner_ura,
                "meta.list_abilities",
                crate::daemon::ability::CallMode::Rpc,
            )
            .expect("authority-scoped lookup")
            .unwrap_or_else(|| panic!("meta.list_abilities missing for {owner_ura}"));
        assert_eq!(record.authority().scope().authority_root(), *owner_ura);
    }

    for evidence in default_voice_capability_state_evidence() {
        for authority in [&hub_ura, &device_ura] {
            assert!(
                registry
                    .control_plane_record_for_authority_mode(
                        authority,
                        evidence.name,
                        evidence.call_mode,
                    )
                    .expect("voice seam authority lookup")
                    .is_none(),
                "{} must not be published without its provider",
                evidence.name
            );
        }
    }
}

#[test]
fn combined_registry_exposes_invocation_history_through_one_ledger_governance_owner() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let agents = AgentRegistry::default();
    let device_ura = crate::core::ura::device_ura("history-fixture", "dev-b");
    let hub_ura = crate::core::ura::hub_ura("history-fixture");
    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots(
            device_ura.clone(),
        )
        .expect("combined authority context");
    let config = registry_config_for_agents_with_authority(&agents, authority_context);
    let registry = build_registry_with_services_result(config)
        .expect("assemble registry")
        .catalog;

    let device_record = registry
        .control_plane_record_for_authority_mode(
            &device_ura,
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("Device history authority lookup");
    let hub_record = registry
        .control_plane_record_for_authority_mode(
            &hub_ura,
            crate::daemon::ability::names::governance::INVOCATION_HISTORY_LIST,
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("Hub history authority lookup");

    let record = device_record.expect("history list must remain visible on the host device");
    assert_eq!(record.descriptor().owner_ura, device_ura);
    assert_eq!(record.authority().scope().authority_root(), device_ura);
    assert!(
        hub_record.is_none(),
        "one daemon ledger must not publish a duplicate hub-governance history route"
    );
}

#[test]
fn explicit_voice_repository_registers_only_hub_call_aggregate_routes() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let agents = AgentRegistry::default();
    let device_ura = crate::core::ura::device_ura("voice-fixture", "dev");
    let hub_ura = crate::core::ura::hub_ura("voice-fixture");
    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots(
            device_ura.clone(),
        )
        .expect("combined authority context");
    let mut config = registry_config_for_agents_with_authority(&agents, authority_context);
    let voice_shared_root = tempfile::tempdir().expect("create explicit shared Voice fixture root");
    let repository = Arc::new(
        crate::daemon::persistence::voice_calls::HubRealmVoiceCallRepository::open(
            voice_shared_root.path(),
            "voice-fixture",
        )
        .expect("open explicit shared Voice fixture"),
    );
    let provider =
        crate::daemon::ability::builtins::resources::voice_contract::VoiceCallProviderAssembly::try_new(
            repository,
        )
        .expect("qualify explicit shared Voice fixture");
    config.shared_stores =
        RegistrySharedStores::default().with_voice_call_provider_assembly(provider);
    let built = build_registry_with_services_result(config)
        .expect("assemble registry with explicit voice repository");
    assert_eq!(
        built.voice_capability_state,
        provider_backed_voice_capability_state_evidence()
    );
    let registry = built.catalog;
    let contracts = system_ability_contract_inventory_for_voice_assembly(
        crate::daemon::ability::conformance::VoiceAssemblyEvidence {
            repository_assembled: true,
            executable_delivery_evidence: false,
        },
    );

    for evidence in provider_backed_voice_capability_state_evidence() {
        let hub_record = registry
            .control_plane_record_for_authority_mode(&hub_ura, evidence.name, evidence.call_mode)
            .expect("Hub voice authority lookup");
        if evidence.state == crate::daemon::ability::conformance::CapabilityState::ProviderBacked {
            let record = hub_record.unwrap_or_else(|| panic!("{} missing", evidence.name));
            assert_eq!(record.descriptor().owner_ura, hub_ura);
            assert_eq!(record.authority().scope().authority_root(), hub_ura);
            let contract = contracts
                .iter()
                .find(|contract| contract.name == evidence.name)
                .unwrap_or_else(|| panic!("{} contract missing", evidence.name));
            assert_eq!(record.descriptor().call_mode(), contract.call_mode);
            assert_eq!(
                record.descriptor().admission_action(),
                contract.admission_action
            );
        } else {
            assert!(
                hub_record.is_none(),
                "{} has no media provider",
                evidence.name
            );
        }
        assert!(
            registry
                .control_plane_record_for_authority_mode(
                    &device_ura,
                    evidence.name,
                    evidence.call_mode,
                )
                .expect("Device voice exclusion lookup")
                .is_none(),
            "{} must never have a Device owner",
            evidence.name
        );
    }
}

#[test]
fn unqualified_voice_repository_is_rejected_before_registry_assembly() {
    let error =
        crate::daemon::ability::builtins::resources::voice_contract::VoiceCallProviderAssembly::try_new(
            Arc::new(
                crate::daemon::ability::builtins::resources::voice_contract::TestVoiceCallRepository::default(),
            ),
        )
        .expect_err("unqualified repositories must not assemble as production Voice providers");
    assert!(error
        .to_string()
        .contains("not qualified for durable realm authority"));
}

#[test]
fn pages_management_is_user_owned_and_runs_on_the_declared_pages_agent() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let agents = AgentRegistry::default();
    let device_ura = crate::core::ura::device_ura("pages-owner", "dev-1");
    let pages_agent = crate::core::ura::agent_ura("pages-owner", "alice", "pages");
    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
            device_ura,
        )
        .expect("pages test Device authority context");
    let mut config = registry_config_for_agents_with_authority(&agents, authority_context);
    config.pages_identity = crate::daemon::ability::builtins::resources::pages::PagesIdentity {
        user: Some("alice".to_string()),
        realm: Some("pages-owner".to_string()),
        listener_port: Some(8787),
    };

    let registry = build_registry_with_services_result(config)
        .expect("assemble registry")
        .catalog;
    let record = registry
        .control_plane_record_for_authority_mode(
            &pages_agent,
            "pages.publish",
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("Pages control-plane lookup")
        .expect("Pages publish must be registered under its declared execution Agent");
    assert_eq!(record.authority().scope().owner_projection(), "user:alice");
    assert_eq!(record.authority().scope().authority_root(), pages_agent);

    crate::daemon::ability::builtins::resources::pages::register_project_abilities(
        &registry,
        "pages-owner",
        "alice",
        "portfolio",
    )
    .expect("register dynamic Pages project abilities");
    let fetch_record = registry
        .control_plane_record_for_authority_mode(
            &pages_agent,
            "alice.portfolio.page.fetch",
            crate::daemon::ability::CallMode::Rpc,
        )
        .expect("dynamic Pages control-plane lookup")
        .expect("page.fetch must use the same declared Pages execution Agent");
    assert_eq!(
        fetch_record.authority().scope().owner_projection(),
        "user:alice"
    );
    assert_eq!(
        fetch_record.authority().scope().authority_root(),
        pages_agent
    );
}

#[test]
fn user_rooted_registry_rejects_paired_identity_without_realm() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let agents = AgentRegistry::default();
    let mut config = registry_config_for_agents(&agents);
    config.pages_identity = crate::daemon::ability::builtins::resources::pages::PagesIdentity {
        user: Some("alice".to_string()),
        realm: None,
        listener_port: Some(8787),
    };

    let error = match build_registry_with_services_result(config) {
        Ok(_) => panic!("registry assembly must not default a paired user into a product realm"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("explicit realm"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn hub_registry_assembly_contains_no_device_plane_control_or_runtime_rows() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let agents = AgentRegistry::default();
    let hub_ura = crate::core::ura::hub_ura("hub-only");
    let runtime = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
        crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
        None,
    );
    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_realm_authority_root(
            &hub_ura,
        )
        .expect("realm authority context");
    let mut config = registry_config_for_agents_with_authority(&agents, authority_context);
    config.local_runtime = Some(Arc::clone(&runtime));
    let registry = build_registry_with_services_result(config)
        .expect("assemble registry")
        .catalog;

    let rows = registry.authority_ability_catalog_snapshot();
    assert!(
        !rows.is_empty(),
        "RealmAuthority registry must retain its Authority-owned abilities"
    );
    let leaked_rows: Vec<_> = rows
        .iter()
        .filter(|row| {
            row.owner != crate::daemon::ability::dispatch::OwnerKind::RealmAuthority
                || row.descriptor.owner_ura != hub_ura
        })
        .collect();
    assert!(
        leaked_rows.is_empty(),
        "RealmAuthority registry leaked a non-Authority row: {leaked_rows:?}"
    );
    assert_eq!(
        rows.iter()
            .filter(|row| {
                row.name == crate::daemon::ability::names::device_control::SESSION_OPEN
                    && row.owner == crate::daemon::ability::dispatch::OwnerKind::RealmAuthority
                    && row.descriptor.owner_ura == hub_ura
                    && row.descriptor.call_mode() == crate::daemon::ability::CallMode::Bidi
            })
            .count(),
        1,
        "RealmAuthority registry must retain exactly one Authority-owned session.open descriptor"
    );
    let exclusions = registry.static_authority_exclusion_snapshot();
    assert!(
        exclusions.get("device").copied().unwrap_or_default() > 0,
        "shared assembly must report its centrally-filtered Device registrations: {exclusions:?}"
    );
    let conformance = crate::daemon::ability::conformance::RegistryConformance::new(&registry)
        .check(
            "hub",
            crate::daemon::ability::conformance::HubBaseline::required_abilities(),
        );
    assert!(
        conformance.is_conformant(),
        "RealmAuthority owner filtering broke the local authority baseline: {}",
        conformance.panic_message()
    );

    let hub_meta = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
        &hub_ura,
        "meta.list_abilities",
    )
    .expect("RealmAuthority meta runtime key");
    assert!(
        crate::support::async_bridge::run_blocking(
            runtime.ability_options(&hub_meta),
            crate::support::async_bridge::SyncBridgeRuntimePolicy::UseFuturesExecutor,
        )
        .is_some(),
        "RealmAuthority LocalRuntime must retain meta.list_abilities"
    );

    let hub_voice_list = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
        &hub_ura,
        crate::daemon::ability::names::resources::VOICE_LIST_CALLS,
    )
    .expect("RealmAuthority voice.list_calls runtime key");
    assert!(
        crate::support::async_bridge::run_blocking(
            runtime.ability_options(&hub_voice_list),
            crate::support::async_bridge::SyncBridgeRuntimePolicy::UseFuturesExecutor,
        )
        .is_none(),
        "RealmAuthority LocalRuntime must not expose voice.list_calls without a realm provider"
    );

    let former_synthetic_device = crate::core::ura::device_ura("hub-only", "local");
    let device_observe = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
        &former_synthetic_device,
        "observe.health",
    )
    .expect("hypothetical Device runtime key");
    assert!(
        crate::support::async_bridge::run_blocking(
            runtime.ability_options(&device_observe),
            crate::support::async_bridge::SyncBridgeRuntimePolicy::UseFuturesExecutor,
        )
        .is_none(),
        "RealmAuthority LocalRuntime must not contain rows under the former synthetic Device root"
    );
}

#[test]
fn realm_authority_daemon_builder_does_not_read_device_agent_transaction_state() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let state_dir = crate::daemon::persistence::config::state_dir();
    std::fs::create_dir_all(&state_dir).expect("create isolated state directory");
    std::fs::write(state_dir.join("agents.json"), b"not-json")
        .expect("write invalid Device agent registry sentinel");
    std::fs::write(
        crate::daemon::persistence::teach_grants::path(),
        b"not-json",
    )
    .expect("write invalid Device teach-transaction sentinel");

    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_realm_authority_root(
            crate::core::ura::hub_ura("hub-only"),
        )
        .expect("realm authority context");
    let mut config = RegistryDaemonBuildConfig::new_with_authority_context(
        RegistryBuildServices::fresh(),
        authority_context,
    );
    config.local_runtime = Some(
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        ),
    );
    let built = build_registry_for_daemon_result(config)
        .expect("RealmAuthority daemon builder must not parse Device agent transaction state");
    let rows = built.catalog.authority_ability_catalog_snapshot();
    assert!(
        rows.iter()
            .all(|row| row.owner == crate::daemon::ability::dispatch::OwnerKind::RealmAuthority),
        "RealmAuthority daemon builder leaked Device/Agent state: {rows:?}"
    );
    assert!(built
        .catalog
        .authority_ability_catalog_snapshot()
        .iter()
        .any(
            |row| row.name == crate::daemon::ability::names::governance::AUTHORITY_BINDING_GRANT
                && row.owner == crate::daemon::ability::dispatch::OwnerKind::RealmAuthority
        ));
}

#[test]
fn realm_authority_daemon_builder_starts_without_publishing_unprovided_voice_capabilities() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_realm_authority_root(
            crate::core::ura::hub_ura("voice-provider-required"),
        )
        .expect("realm authority context");
    let mut config = RegistryDaemonBuildConfig::new_with_authority_context(
        RegistryBuildServices::fresh(),
        authority_context,
    );
    config.local_runtime = Some(
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        ),
    );

    let built = build_registry_for_daemon_result(config)
        .expect("Hub daemon must start when optional voice providers are absent");
    assert_eq!(
        built.voice_capability_state,
        default_voice_capability_state_evidence()
    );
    let rows = built.catalog.authority_ability_catalog_snapshot();
    for evidence in default_voice_capability_state_evidence() {
        assert!(
            !rows.iter().any(|row| row.name == evidence.name),
            "{} must not appear operational without its provider",
            evidence.name
        );
    }
}

#[test]
fn device_daemon_builder_refuses_corrupt_agent_registry() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let state_dir = crate::daemon::persistence::config::state_dir();
    std::fs::create_dir_all(&state_dir).expect("create isolated state directory");
    std::fs::write(state_dir.join("agents.json"), b"not-json")
        .expect("write corrupt agent registry");

    let authority_context =
        crate::daemon::ability::dispatch::AbilityAuthorityContext::for_device_authority_root(
            crate::core::ura::device_ura("localhost", "dev"),
        )
        .expect("Device authority context");
    let mut config = RegistryDaemonBuildConfig::new_with_authority_context(
        RegistryBuildServices::fresh(),
        authority_context,
    );
    config.local_runtime = Some(
        crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        ),
    );
    let result = build_registry_for_daemon_result(config);
    assert!(
        result.is_err(),
        "device daemon boot must not hide corrupt durable agent state"
    );
    let error = result.err().expect("checked above");
    assert!(
        error.to_string().contains("load daemon agent registry"),
        "unexpected daemon registry boot error: {error:#}"
    );
}

#[test]
fn published_abilities_includes_skill_list_with_real_metadata() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    // Load-bearing for the EasyNet frontend's Skills page: the
    // backend invokes `skill.list` against the target node.
    // A regression that dropped it from `published_abilities()`
    // would silently empty the Skills page across the device set.
    let metas = published_abilities();
    let skill = metas
        .iter()
        .find(|m| m.name == "skill.list")
        .expect("skill.list must be in published_abilities");
    // Description must NOT be the unknown-name fallback.
    // `(system ability)` is what `description_for` returns when
    // an ability is added without an arm here; pin against it so
    // a future ability that lands without metadata trips the
    // test instead of shipping a generic blurb to the frontend.
    assert_ne!(
        skill.description, "(system ability)",
        "skill.list must have a real description, not the fallback"
    );
    // Input schema must be a JSON Schema object (the wire shape
    // axon-runtime stores). Empty `{}` would also pass `is_object`,
    // so additionally pin the `type` field.
    assert_eq!(
        skill.input_schema().get("type").and_then(|v| v.as_str()),
        Some("object"),
        "input schema must declare type:object; got {:?}",
        skill.input_schema()
    );
    assert!(
        !skill.hints.streaming_only && !skill.hints.bidi_only,
        "skill.list must stay unary-only; got hints {:?}",
        skill.hints
    );
}

#[test]
fn published_system_abilities_excludes_plugin_package_abilities() {
    let plugin_leaks: Vec<String> = published_system_abilities()
        .into_iter()
        .map(|meta| meta.name)
        .filter(|name| name.starts_with("remote_desktop."))
        .collect();
    assert!(
        plugin_leaks.is_empty(),
        "system descriptor generation must not include plugin abilities: {plugin_leaks:?}"
    );
}

#[test]
fn published_abilities_marks_server_stream_routes_as_streaming_only() {
    let metas = published_abilities();
    let expected = [
        "consent.subscribe",
        "discuss.subscribe",
        "loop.subscribe",
        "session.attach",
        "mic.subscribe",
        "camera.subscribe",
        "screen.subscribe",
    ];
    #[cfg(feature = "remote-desktop")]
    let expected = {
        let mut expected = expected.to_vec();
        expected.push("remote_desktop.watch_events");
        expected
    };
    #[cfg(not(feature = "remote-desktop"))]
    let expected = expected.to_vec();
    for name in expected {
        let meta = metas
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} must be published"));
        assert!(
            meta.hints.streaming_only,
            "{name} must advertise streaming_only so callers use InvokeStream"
        );
        assert!(!meta.hints.bidi_only, "{name} is server-stream, not bidi");
    }
}

#[test]
fn published_abilities_marks_bidi_routes_as_bidi_only() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let metas = published_abilities();
    let expected = ["fs.transfer", "terminal.attach", "speaker.publish"];
    #[cfg(feature = "remote-desktop")]
    let expected = {
        let mut expected = expected.to_vec();
        expected.push("remote_desktop.attach");
        expected
    };
    #[cfg(not(feature = "remote-desktop"))]
    let expected = expected.to_vec();
    for name in expected {
        let meta = metas
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} must be published"));
        assert!(meta.hints.bidi_only, "{name} must advertise bidi_only");
        assert!(
            !meta.hints.streaming_only,
            "{name} must not masquerade as server-stream"
        );
    }
}

#[test]
fn discovery_hints_leave_agent_chat_on_unary_control_plane_path() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    seed_hosted_agents_for_chat(&["alice"]);
    let mut agents = AgentRegistry::default();
    agents.agents.insert(
        canonical_test_agent_registry_key("alice"),
        test_agent_entry("alice"),
    );
    let reg = build_registry_with_services_result(registry_config_for_agents(&agents))
        .expect("assemble registry")
        .catalog;
    let hints = discovery_hints_for(&reg, "alice.chat");
    assert!(
        !hints.streaming_only && !hints.bidi_only,
        "alice.chat must stay on the unary/OpenAI path until generic InvokeStream support lands; got {:?}",
        hints
    );
}

#[test]
fn published_abilities_excludes_per_agent_chat_handlers() {
    // Deterministic system metadata excludes dynamic hosted-Agent rows. Live
    // daemon publication captures those rows from the committed control plane.
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    seed_hosted_agents_for_chat(&["alice"]);
    let mut agents = AgentRegistry::default();
    agents.agents.insert(
        canonical_test_agent_registry_key("alice"),
        test_agent_entry("alice"),
    );
    let reg = build_registry_with_services_result(registry_config_for_agents(&agents))
        .expect("assemble registry")
        .catalog;
    // Sanity: the registry itself does include alice.chat.
    assert!(reg.list_abilities().iter().any(|n| n == "alice.chat"));
    // But the system publisher's view excludes it. We can't call
    // published_abilities() with this custom registry directly
    // (it goes through build_registry()), so instead assert the
    // filter property: every entry's name does NOT end with .chat.
    for meta in published_abilities() {
        assert!(
            !meta.name.ends_with(".chat"),
            "published_abilities must filter out *.chat (came in via per-agent manifest); \
                 found {} which would double-register",
            meta.name
        );
    }
}

#[test]
fn daemon_local_discover_is_routable_but_not_publishable() {
    assert!(
        !is_publishable_catalog_name("agent.discover"),
        "agent.discover is a local aggregate-discovery front door and must not be federated"
    );
    assert!(
        is_local_runtime_routable_catalog_name("agent.discover"),
        "agent.discover must stay invokable through the local daemon Invocation surface"
    );
    for ability in ["plugin.companion_status", "plugin.companion_reconcile"] {
        assert!(
            !is_publishable_catalog_name(ability),
            "{ability} must not be published as a federated ability"
        );
        assert!(
            !is_local_runtime_routable_catalog_name(ability),
            "{ability} must remain outside public Invocation routing"
        );
    }
}

#[test]
fn description_for_and_input_schema_for_cover_every_published_name() {
    // Adding a new ability to build_registry without also adding
    // arms to `description_for`/`try_input_schema_for` would let it
    // ship with the undeclared-object schema. Pin the contract that every
    // published system name has real metadata. This deliberately uses
    // `published_system_abilities()` instead of the live daemon view so a
    // developer's `$HOME/.easynet/plugins` cannot make the unit test
    // depend on installed third-party packages.
    for name in published_system_abilities()
        .into_iter()
        .map(|meta| meta.name)
    {
        let desc = description_for(&name);
        assert_ne!(
            desc, "(system ability)",
            "{name} is missing a description_for arm — add one in daemon::ability::catalog::catalog_metadata"
        );
        let schema = try_input_schema_for(&name)
            .unwrap_or_else(|error| panic!("{name} schema lookup must be fail-closed: {error}"));
        // The undeclared object projection returns `{"type":"object"}` with
        // NO other keys. A real authored schema always pins something more —
        // `properties`, `additionalProperties`, `oneOf`, etc. —
        // even for genuinely-no-arg abilities (e.g.
        // `consent.subscribe` declares
        // `additionalProperties: false`). Distinguishing an
        // undeclared projection from an authored "no-arg" schema by structure
        // (does the object have any key besides `type`?) is
        // strictly stronger than a name allowlist.
        let obj = schema
            .as_object()
            .unwrap_or_else(|| panic!("{name} schema must be a JSON object"));
        let has_only_type = obj.len() == 1 && obj.contains_key("type");
        assert!(
            !has_only_type,
            "{name} fell through to the default `{{type: object}}` schema; \
                 add an authored schema arm (declare additionalProperties: false \
                 even if the ability is genuinely no-arg)"
        );
    }
}

#[test]
fn fallible_input_schema_projection_does_not_treat_absent_plugin_as_failure() {
    let schema = try_input_schema_for("observe.health")
        .expect("system ability schema projection must continue after absent plugin lookup");
    let description = try_description_for_owned("observe.health")
        .expect("system ability description projection must continue after absent plugin lookup");

    assert_eq!(schema["type"], "object");
    assert!(
        schema.as_object().is_some_and(|object| object.len() > 1),
        "system ability must not fall through to undeclared object schema"
    );
    assert_ne!(
        description, "(system ability)",
        "system ability must not fall through to generic description metadata"
    );
}

#[test]
fn registry_includes_chat_handler_per_registered_agent() {
    // After Phase 3 wired chat as a system ability, every agent
    // in the registry should produce a `<agent>.chat` handler in
    // the unified AxonAbilityCatalog. This is the load-bearing
    // property that lets the proxy dispatch chat through the
    // same registry as ping/session/permission.
    use crate::daemon::persistence::agent_registry::{AgentEntry, AgentType};
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    seed_hosted_agents_for_chat(&["alice", "bob"]);
    let mut agents = AgentRegistry::default();
    agents.agents.insert(
        canonical_test_agent_registry_key("alice"),
        test_agent_entry("alice"),
    );
    let mut bob = AgentEntry::new(AgentType::Codex, None);
    bob.root_path = Some(crate::daemon::persistence::config::agents_root().join("bob"));
    agents
        .agents
        .insert(canonical_test_agent_registry_key("bob"), bob);
    let reg = build_registry_with_services_result(registry_config_for_agents(&agents))
        .expect("assemble registry")
        .catalog;
    let names = reg.list_abilities();
    assert!(
        names.iter().any(|n| n == "alice.chat"),
        "alice.chat must be registered; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "bob.chat"),
        "bob.chat must be registered; got {names:?}"
    );
}

#[test]
fn build_registry_always_registers_key_service_abilities() {
    let agents = AgentRegistry::default();
    let reg = build_registry_with_services_result(registry_config_for_agents(&agents))
        .expect("assemble registry")
        .catalog;
    let names = reg.list_abilities();

    // Administrative projections are present under device.keyring.*. Raw
    // signing is SDK-only and must never be exposed as an Invocation ability.
    for verb in [
        "create",
        "list",
        "get_public",
        "rotate",
        "revoke",
        "expire_set",
        "bind_subject",
        "peer_add",
        "peer_list",
    ] {
        let want = format!("device.keyring.{verb}");
        assert!(
            names.iter().any(|n| n == &want),
            "{want} must be registered; got {names:?}"
        );
    }
    assert!(
        !names.iter().any(|name| name == "device.keyring.sign"),
        "raw signing must remain inside the local key-service capability boundary"
    );
}

/// RFC-005 lint: public catalogue names are owner-local names.
/// Device ownership is carried by `owner_ura` / `ability_ura`, so
/// catalogue rows must not expose implementation-local owner prefixes such
/// as `fs.read`.
#[test]
fn published_catalogue_does_not_duplicate_device_owner_prefix() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let names: Vec<String> = published_system_abilities()
        .into_iter()
        .map(|meta| meta.name)
        .collect();
    let violations: Vec<String> = names
        .into_iter()
        .filter(|name| name.starts_with("device."))
        .collect();
    assert!(
        violations.is_empty(),
        "RFC-005 catalogue must not duplicate device ownership in public names: {violations:?}"
    );
}

/// **M5 lint** — the legacy self alias token never appears as a first
/// segment in the published catalogue. The wire-pinned trio
/// (`session.open`,
/// `identity.register_pubkey`) goes through wire-only
/// constants; they are NOT registered into the discoverable
/// catalogue. If they ever leak, this test fails and the
/// regression is caught at CI rather than in an LLM seeing
/// a legacy self-alias entry and getting confused.
#[test]
fn published_catalogue_never_contains_self_alias() {
    let _home = crate::cli::commands::test_support::HomeGuard::new();
    let names: Vec<String> = published_system_abilities()
        .into_iter()
        .map(|meta| meta.name)
        .collect();
    let legacy_self_prefix = ["<", "self", ">"].concat();
    let leaks: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with(&legacy_self_prefix))
        .collect();
    assert!(
        leaks.is_empty(),
        "post-M5 catalogue must not expose legacy self-alias names; got {leaks:?}"
    );
}
