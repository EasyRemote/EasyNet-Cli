//! Assembly-level tests for the agents registry + catalogue
//! surface (moved verbatim from agents/mod.rs, F-027 / T4.5).

use super::registry_builder::build_system_registry;
use super::*;
use crate::registry::agents::AgentRegistry;
use std::sync::Arc;

fn registry_config_for_agents(agents: &AgentRegistry) -> RegistryBuildConfig<'_> {
    RegistryBuildConfig::new(RegistryBuildServices::fresh(), agents)
}

fn sha256_hex_for_test(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// Seed `local-agents.json` with the canonical hosted-agent identities the
/// hot registrar requires before it will register `<agent>.chat` into the
/// runtime. Each tuple is `(profile, name)`; the agent URA is the canonical
/// `agent/<name>.<profile>` form under the local realm. Call inside a
/// `HomeGuard` so the file lands in the test's temp HOME.
fn seed_hosted_agents_for_chat(agents: &[(&str, &str)]) {
    use crate::persistence::local_agents::{save, upsert_hosted_agent, LocalAgentsFile};
    let mut local = LocalAgentsFile::default();
    for (profile, name) in agents {
        // Agent URA is `agent/<user>.<name>`; the registrar verifies its
        // agent-id (the after-dot segment) equals the registry name, so
        // `name` must be the third arg.
        let agent_ura = crate::ura::agent_ura("localhost", profile, name);
        upsert_hosted_agent(&mut local, profile, name, &agent_ura);
    }
    save(&local).expect("seed local-agents.json for chat handlers");
}

#[test]
fn daemon_registry_boot_hook_recovers_acquiring_descriptor_imports() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();

    let home = std::env::var("HOME").expect("HomeGuard sets HOME");
    let manifest_path =
        std::path::Path::new(&home).join("agents/apprentice/abilities/quote.ability.toml");
    std::fs::create_dir_all(manifest_path.parent().expect("manifest parent"))
        .expect("create manifest dir");
    let manifest =
        b"name = \"quote\"\ndescription = \"imported\"\n\n[input_schema]\ntype = \"object\"\n";
    std::fs::write(&manifest_path, manifest).expect("write imported descriptor");
    let manifest_hash = sha256_hex_for_test(manifest);

    let grants_path = crate::persistence::teach_grants::path();
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

    let recovered =
        super::registry_builder::recover_descriptor_import_transactions_before_daemon_registry_boot()
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
    // profile and (b) is registered into the live LocalRuntime via
    // runtime.register_local_tool.
    // Hold the env lock: published_ability_names() reads HOME-rooted
    // registry state, so a concurrent HOME-mutating test must not race it.
    let _home = crate::facade::cli::test_support::HomeGuard::new();
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
fn terminal_list_is_owner_kind_device() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    use crate::runtime::ability_dispatch::OwnerKind;
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
fn ability_layer_classification_is_complete() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
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
             Add an arm to classify_ability() in src/runtime/agents/mod.rs \
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
    let _home = crate::facade::cli::test_support::HomeGuard::new();
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
fn every_published_ability_has_a_toml_byte_for_byte_matching_the_renderer() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    // The TOML descriptors in abilities/system/ are the
    // source of truth for external discovery tools. They are
    // GENERATED from `render_ability_toml(name,
    // description_for(name), input_schema_for(name))`. This
    // test enforces that the on-disk file is byte-for-byte
    // identical to what the renderer produces; if the
    // dispatcher's metadata changed and a maintainer forgot
    // to regenerate, this test names every drifted ability
    // and tells them how to fix it.
    let mut missing: Vec<String> = Vec::new();
    let mut drift: Vec<String> = Vec::new();
    for meta in published_system_abilities() {
        let toml_path = descriptor_path_for(&meta.name);
        let on_disk = match std::fs::read_to_string(&toml_path) {
            Ok(body) => body,
            Err(_) => {
                missing.push(meta.name.clone());
                continue;
            }
        };
        let _ = rfc006_for(&meta.name);
        let expected =
            ability_toml::render_ability_toml(&meta.name, &meta.description, &meta.input_schema);
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

/// Walk every published ability and confirm a handler is
/// registered under SOME invocation mode (RPC, Stream, or
/// Bidi). Distinguishes "ability advertised in
/// list_abilities() but dispatcher returns ABILITY_NOT_FOUND"
/// from "ability is callable". This is the bare minimum for
/// the question "is this ability really wired".
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
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let names: Vec<String> = reg.list_abilities();
    let mut unresolved: Vec<String> = Vec::new();
    for name in &names {
        // <agent>.chat handlers register as Stream. Most
        // system abilities register as RPC. Bidi is rare
        // (PTY attach). We accept any of the three.
        let has_rpc = reg.has_rpc(name);
        let has_stream = reg.has_stream(name);
        let has_bidi = reg.has_bidi(name);
        if !(has_rpc || has_stream || has_bidi) {
            unresolved.push(name.clone());
        }
    }
    assert!(
        unresolved.is_empty(),
        "abilities listed by list_abilities() but with NO handler registered: {unresolved:?}"
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
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

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
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: name.clone(),
            normalized_args: serde_json::json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        };
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
    // discipline applies to `src/runtime/**` daemon code paths,
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
            | "session.list"
            | "consent.list_pending"
            | "schedule.list"
            | "plugin.status"
            | "meta.list_resources"
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
fn build_registry_satisfies_device_baseline_contract() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    let reg = build_registry();
    let device = crate::runtime::ability::conformance::DeviceBaseline::required_abilities();
    let report = crate::runtime::ability::conformance::RegistryConformance::new(&reg)
        .check("device", &device);

    assert!(
        report.is_conformant(),
        "Device baseline abilities missing or registered under the wrong call mode:\n  {}",
        report.panic_message()
    );
}

#[test]
fn published_abilities_includes_skill_list_with_real_metadata() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
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
        skill.input_schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "input schema must declare type:object; got {:?}",
        skill.input_schema
    );
    assert!(
        !skill.hints.streaming_only && !skill.hints.bidi_only,
        "skill.list must stay unary-only; got hints {:?}",
        skill.hints
    );
}

#[test]
fn published_system_abilities_excludes_plugin_package_abilities() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
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
    // Hold the env lock: published_abilities() reads HOME-rooted registry
    // state, so a concurrent HOME-mutating test must not race it.
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    let metas = published_abilities();
    let expected = [
        "consent.subscribe",
        "discuss.subscribe",
        "loop.subscribe",
        "session.attach",
        "mic.subscribe",
        "camera.subscribe",
        "screen.subscribe",
        "voice.subscribe",
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
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    let metas = published_abilities();
    let expected = [
        "fs.transfer",
        "terminal.attach",
        "speaker.publish",
        "voice.transcribe",
    ];
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
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    use crate::registry::agents::{AgentEntry, AgentType};
    let mut agents = AgentRegistry::default();
    agents
        .agents
        .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
    let reg = build_registry_with_services(registry_config_for_agents(&agents));
    let hints = discovery_hints_for(&reg, "alice.chat");
    assert!(
        !hints.streaming_only && !hints.bidi_only,
        "alice.chat must stay on the unary/OpenAI path until generic InvokeStream support lands; got {:?}",
        hints
    );
}

#[test]
fn published_abilities_excludes_per_agent_chat_handlers() {
    // `<agent>.chat` is published via the per-agent manifest path
    // (`runtime::publish::republish_abilities_via_advertise`) off the
    // on-disk `chat.ability.toml`. Re-publishing it through the
    // system path would double-register with a synthesised schema
    // that shadows the manifest's real one. The filter in
    // `published_abilities()` enforces this; pin it.
    use crate::registry::agents::{AgentEntry, AgentType};
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    seed_hosted_agents_for_chat(&[("claude", "alice")]);
    let mut agents = AgentRegistry::default();
    agents
        .agents
        .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
    let reg = build_registry_with_services(registry_config_for_agents(&agents));
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
fn description_for_and_input_schema_for_cover_every_published_name() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    // Adding a new ability to build_registry without also adding
    // arms to `description_for`/`input_schema_for` would let it
    // ship with the unknown-name fallback ("(system ability)" and
    // empty `{type: object}` schema). Pin the contract that every
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
            "{name} is missing a description_for arm — add one in runtime::system::mod"
        );
        let schema = input_schema_for(&name);
        // The default fallback returns `{"type":"object"}` with
        // NO other keys. A real arm always pins something more —
        // `properties`, `additionalProperties`, `oneOf`, etc. —
        // even for genuinely-no-arg abilities (e.g.
        // `consent.subscribe` declares
        // `additionalProperties: false`). Distinguishing the
        // fallback from an authored "no-arg" schema by structure
        // (does the object have any key besides `type`?) is
        // strictly stronger than a name allowlist.
        let obj = schema
            .as_object()
            .unwrap_or_else(|| panic!("{name} schema must be a JSON object"));
        let has_only_type = obj.len() == 1 && obj.contains_key("type");
        assert!(
            !has_only_type,
            "{name} fell through to the default `{{type: object}}` schema; \
                 add an input_schema_for arm (declare additionalProperties: false \
                 even if the ability is genuinely no-arg)"
        );
    }
}

#[test]
fn registry_includes_chat_handler_per_registered_agent() {
    // After Phase 3 wired chat as a system ability, every agent
    // in the registry should produce a `<agent>.chat` handler in
    // the unified AxonAbilityCatalog. This is the load-bearing
    // property that lets the proxy dispatch chat through the
    // same registry as ping/session/permission.
    use crate::registry::agents::{AgentEntry, AgentType};
    let _home = crate::facade::cli::test_support::HomeGuard::new();
    seed_hosted_agents_for_chat(&[("claude", "alice"), ("codex", "bob")]);
    let mut agents = AgentRegistry::default();
    agents
        .agents
        .insert("alice".into(), AgentEntry::new(AgentType::ClaudeCode, None));
    agents
        .agents
        .insert("bob".into(), AgentEntry::new(AgentType::Codex, None));
    let reg = build_registry_with_services(registry_config_for_agents(&agents));
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
fn build_registry_registers_keyring_abilities_when_not_disabled() {
    // Run in a child-process-style isolation: redirect the
    // keyring file path to a tempdir + clear DISABLE so the
    // auto-init path runs. The default tests set DISABLE.
    // NOTE: this test already serialises via env_lock() directly — do
    // NOT also take a HomeGuard (it acquires the same non-reentrant
    // env_lock and would deadlock).
    let _env_lock = crate::facade::cli::test_support::env_lock();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("keyring.json");
    let prev_disable = std::env::var_os("EASYNET_KEYRING_DISABLE");
    let prev_path = std::env::var_os("EASYNET_KEYRING_PATH");
    let prev_pass = std::env::var_os("EASYNET_KEYRING_PASS");
    std::env::remove_var("EASYNET_KEYRING_DISABLE");
    std::env::set_var("EASYNET_KEYRING_PATH", &path);
    std::env::set_var("EASYNET_KEYRING_PASS", "test-pass-keyring-init");

    let agents = AgentRegistry::default();
    let reg = build_registry_with_services(registry_config_for_agents(&agents));
    let names = reg.list_abilities();

    // Restore env before assertions so a panic doesn't leak
    // environment changes into other tests in the same binary.
    match prev_disable {
        Some(v) => std::env::set_var("EASYNET_KEYRING_DISABLE", v),
        None => std::env::remove_var("EASYNET_KEYRING_DISABLE"),
    }
    match prev_path {
        Some(v) => std::env::set_var("EASYNET_KEYRING_PATH", v),
        None => std::env::remove_var("EASYNET_KEYRING_PATH"),
    }
    match prev_pass {
        Some(v) => std::env::set_var("EASYNET_KEYRING_PASS", v),
        None => std::env::remove_var("EASYNET_KEYRING_PASS"),
    }

    // All 10 abilities must be present under device.keyring.*.
    for verb in [
        "create",
        "list",
        "get_public",
        "sign",
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
    assert!(path.exists(), "keyring file must have been auto-created");
}

/// RFC-005 lint: public catalogue names are owner-local names.
/// Device ownership is carried by `owner_ura` / `ability_ura`, so
/// catalogue rows must not expose implementation-local owner prefixes such
/// as `fs.read`.
#[test]
fn published_catalogue_does_not_duplicate_device_owner_prefix() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
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
/// (`session.open`, `runtime.invoke_remote`,
/// `identity.register_pubkey`) goes through wire-only
/// constants; they are NOT registered into the discoverable
/// catalogue. If they ever leak, this test fails and the
/// regression is caught at CI rather than in an LLM seeing
/// a legacy self-alias entry and getting confused.
#[test]
fn published_catalogue_never_contains_self_alias() {
    let _home = crate::facade::cli::test_support::HomeGuard::new();
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
