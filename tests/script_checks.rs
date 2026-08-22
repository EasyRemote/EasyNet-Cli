// Integration tests that run the repo's shell-level guard scripts as
// part of `cargo test --all`. Every PR lands its CI contract here so a
// broken script or schema drift fails the normal test run without
// needing a separate CI step.
//
// The individual cases live in shell so they can be invoked directly by
// contributors: `bash tests/scripts/test_trace_parity.sh`. The shell
// bodies live under `tests/scripts/`; `tests/scripts/`
// remains the Cargo-friendly wrapper entrypoint. This Rust shim is the
// automation seam.
//
// A shell driver is selected explicitly (`bash`) instead of relying on
// the user's `$SHELL` because the scripts use `[[ ... ]]` and
// `set -euo pipefail`, which are bash-specific.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn run_bash_script(relative_path: &str) {
    let script = repo_root().join(relative_path);
    assert!(script.exists(), "script missing: {}", script.display());

    let output = Command::new("bash")
        .arg(&script)
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn bash");

    if !output.status.success() {
        panic!(
            "{} failed (exit {:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            relative_path,
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn project_structure_v1_script_contract_holds() {
    // Pins the migration-compatible root layout: production source
    // roots, retired compatibility paths, and tool-discovery wrappers.
    run_bash_script("tests/scripts/test_check_project_structure_v1.sh");
}

#[test]
fn sdk_scaffold_script_contract_holds() {
    // Pins the Daemon SDK Phase-A scaffold: public SDK docs, schema roots,
    // fixtures, conformance cases, and JSON parseability.
    run_bash_script("tests/scripts/test_check_sdk_scaffold.sh");
}

#[test]
fn sdk_parity_matrix_script_contract_holds() {
    // Pins the Go/Python canonical runtime capability ledger and the
    // validator self-test that rejects stale capability rows, invalid
    // states, product rows, false cutover readiness and missing evidence.
    run_bash_script("tests/scripts/test_check_sdk_parity_matrix.sh");
}

#[test]
fn daemon_key_service_boundary_script_contract_holds() {
    // Pins daemon key-service custody as the only private-key authority:
    // no legacy keyring inventory, seed-shaped egress, SDK private-key input,
    // caller-selected vault path, or raw signing Invocation surface may return.
    run_bash_script("tests/scripts/test_check_daemon_key_service_boundary.sh");
}

#[test]
fn sdk_cutover_readiness_script_contract_holds() {
    // Pins the aggregate cutover-readiness gate wiring. The wrapper runs the
    // source-of-truth self-test so Backend/EasyRemote boundary checks stay
    // reachable from the normal Cargo script-check path without duplicating
    // product boundary logic in Rust.
    run_bash_script("tests/scripts/test_check_sdk_cutover_readiness.sh");
}

#[test]
fn sdk_seam_test_vocabulary_script_contract_holds() {
    // Pins SDK seam tests to generic downstream workflow/profile vocabulary.
    // Test evidence must not keep concrete EasyNet/EasyRemote product client
    // names inside canonical SDK package boundaries.
    run_bash_script("tests/scripts/test_check_sdk_seam_test_vocabulary.sh");
}

#[test]
fn ability_model_convergence_script_contract_holds() {
    // Pins CallMode ownership to the daemon ability descriptor model. Plugin
    // manifest code consumes the type but must not re-export it.
    run_bash_script("tests/scripts/test_check_ability_model_convergence.sh");
}

#[test]
fn product_key_custody_boundary_script_contract_holds() {
    // Pins product processes as SDK consumers: Backend and EasyRemote must not
    // own runtime signing private material, key-service vault/passphrase
    // policy, raw daemon subprocess lifecycle, or direct C ABI loading.
    run_bash_script("tests/scripts/test_check_product_key_custody_boundary.sh");
}

#[test]
fn downstream_sdk_consumer_cutover_script_contract_holds() {
    // Pins positive downstream cutover evidence: Backend runtimeprofile source
    // stays deleted and Backend/EasyRemote consumers use canonical Go/Python
    // SDK providers for Directory, Receipt, Events, Admin, access control,
    // PrincipalLifecycle, configuration, identity, transport and receipt
    // anchors.
    run_bash_script("tests/scripts/test_check_downstream_sdk_consumer_cutover.sh");
}

#[test]
fn trace_parity_script_contract_holds() {
    // Covers happy, failure (missing fixture, extra key, removed key)
    // and edge cases (idempotence, path-only payload invariant).
    run_bash_script("tests/scripts/test_trace_parity.sh");
}

#[test]
fn check_upstream_names_script_contract_holds() {
    // Covers happy, failure (violation in src/ and Cargo.toml) and
    // edge cases (docs/ exemption, string-literal violation still
    // trips, unbanned variant passes).
    run_bash_script("tests/scripts/test_check_upstream_names.sh");
}

#[test]
fn no_raw_ura_construction_script_contract_holds() {
    // Pins the URA builder/parser boundary: only src/core/ura/mod.rs may
    // hand construct or scheme-prefix parse easynet URAs.
    run_bash_script("tests/scripts/test_no_raw_ura_construction.sh");
}

#[test]
fn daemon_invocation_migration_script_contract_holds() {
    // Pins the post-demotion boundary: JSON control stays boot/status,
    // DaemonInvocation construction stays tuple-complete, and runtime
    // invocation stays an Axon-canonical adapter.
    run_bash_script("tests/scripts/test_check_daemon_invocation_migration.sh");
}

#[test]
fn pending_dispatch_target_boundary_script_contract_holds() {
    // Pins remote dispatch terminality: pending unary/stream registrations
    // must be keyed by the selected execution-host URA so presence-loss
    // cancellation remains deterministic and no no-target compatibility
    // registration path can return.
    run_bash_script("tests/scripts/test_check_pending_dispatch_target_boundary.sh");
}

#[test]
fn canonical_runtime_convergence_v2_script_contract_holds() {
    // Pins the SPEC V2 convergence gates that are local to this repository:
    // plain admission helpers remain quarantined, lifecycle states share one
    // canonical matrix vocabulary, URA terminology is active, and Mission/EAL
    // state stays out of generic SDK/runtime surfaces.
    run_bash_script("tests/scripts/test_check_canonical_runtime_convergence_v2.sh");
}

#[test]
fn kernel_boundary_script_contract_holds() {
    // Pins the final project-structure daemon boundary: retired source
    // roots/namespaces cannot return, daemon internals do not depend on
    // CLI/FFI edges, and execution does not own federation transports.
    run_bash_script("tests/scripts/test_check_kernel_boundary.sh");
}

#[test]
fn ffi_abi_v6_exact_surface_script_contract_holds() {
    // Pins the generic binding boundary: exact header/source/dylib allowlist,
    // error code table, daemon lifecycle, and complete Invocation lifecycle.
    run_bash_script("tests/scripts/test_check_ffi_abi_v7_header.sh");
}

#[test]
fn release_package_contract_script_holds() {
    // Pins the release shape consumed by install.sh: runtime binaries,
    // dendrite bridge, and ABI v7 binding artefacts must be packaged
    // and installed together.
    run_bash_script("tests/scripts/test_check_release_package_contract.sh");
}

#[test]
fn system_ability_resource_ref_script_contract_holds() {
    // Pins host-filesystem ability manifests to ResourceRef-only input.
    // Skill package-relative paths remain outside this guard by design.
    run_bash_script("tests/scripts/test_check_system_ability_resource_refs.sh");
}

#[test]
fn system_ability_retired_alias_script_contract_holds() {
    // Pins active system manifests so retired migration aliases are
    // not re-advertised through discovery surfaces.
    run_bash_script("tests/scripts/test_check_system_ability_retired_aliases.sh");
}

#[test]
fn remote_desktop_contract_boundary_script_holds() {
    // Pins the product remote-desktop contract to canonical wire names.
    // Retired transport spelling aliases must fail during typed parse.
    run_bash_script("tests/scripts/test_check_remote_desktop_contract_boundary.sh");
}

#[test]
fn remoteapp_session_subject_boundary_script_holds() {
    // Pins remoteapp session abilities to explicit selected-resource subjects.
    // Session control must not teach callers to place subject or token fields
    // inside ability args.
    run_bash_script("tests/scripts/test_check_remoteapp_session_subject_boundary.sh");
}

#[test]
fn remoteapp_picker_subject_boundary_script_holds() {
    // Pins the remoteapp picker to live target inventory and selected
    // Resource URA subjects instead of cached meta.list_resources rows or
    // args.subject compatibility shapes.
    run_bash_script("tests/scripts/test_check_remoteapp_picker_subject_boundary.sh");
}

#[test]
fn remoteapp_frontend_invocation_boundary_script_holds() {
    // Pins the real EasyNet frontend execution surface to selected target
    // subjects. Browser calls must put the picker-selected target in the
    // Invocation envelope, never in create_session args or first-target
    // fallback paths.
    run_bash_script("tests/scripts/test_check_remoteapp_frontend_invocation_boundary.sh");
}

#[test]
fn remoteapp_target_binding_boundary_script_holds() {
    // Pins application/window sessions to binding-owned capture/input/media
    // boundaries. Production app/window media must not silently re-resolve
    // ResourceEntry rows or fall back to display capture.
    run_bash_script("tests/scripts/test_check_remoteapp_target_binding_boundary.sh");
}

#[test]
fn remoteapp_lifecycle_input_boundary_script_holds() {
    // Pins app/window lifecycle and input safety to target-owned move/resize
    // revisions, target-loss media-source degradation, weak native identity
    // ambiguity, and view-only input until focus-safe dispatch exists. The
    // exhaustive mutation self-test lives under tests/scripts; this normal
    // aggregate gate runs the production checker so `script_checks remoteapp`
    // stays bounded.
    run_bash_script("tools/scripts/check-remoteapp-lifecycle-input-boundary.sh");
}

#[test]
fn remoteapp_e2e_acceptance_boundary_script_holds() {
    // Pins host decoded-frame acceptance to live inventory, exact target
    // binding, WebRTC/H.264 evidence, and independently scanned artifact
    // pixels for window/application sessions. The exhaustive harness mutation
    // self-test lives under tests/scripts; this normal aggregate gate runs the
    // production checker so `script_checks remoteapp` stays bounded.
    run_bash_script("tools/scripts/check-remoteapp-e2e-acceptance-boundary.sh");
}

#[test]
fn remoteapp_performance_boundary_script_holds() {
    // Pins the SPEC PERF-01..PERF-07 evidence map so performance/resource
    // requirements cannot remain documentation-only claims.
    run_bash_script("tests/scripts/test_check_remoteapp_performance_boundary.sh");
}

#[test]
fn remoteapp_product_closure_audit_script_holds() {
    // Pins the product-completion audit so targeted-session boundary gates are
    // not mistaken for full interactive RemoteApp readiness.
    run_bash_script("tests/scripts/test_check_remoteapp_product_closure_audit.sh");
}

#[test]
fn remoteapp_frontend_product_flow_e2e_script_holds() {
    // Pins the runnable frontend/host product-flow harness entrypoint while
    // preserving the distinction between harness existence and product proof.
    run_bash_script("tests/scripts/test_check_remoteapp_frontend_product_flow_e2e.sh");
}

#[test]
fn frontend_ability_contract_boundary_script_holds() {
    // Pins every governed ability descriptor to one explicit execution surface
    // and subject-construction owner. Remote desktop abilities must stay on
    // the dedicated remote_desktop surface, not the generic media/catalog UI.
    run_bash_script("tests/scripts/test_check_frontend_ability_contract_boundary.sh");
}

#[test]
fn browser_cdp_axon_boundary_script_holds() {
    // Pins the browser executor to a package-owned provider, current headed
    // Chrome with isolated debugging profiles, and CDP application frames
    // carried only by the governed Axon bidi session.
    run_bash_script("tests/scripts/test_check_browser_cdp_axon_boundary.sh");
}

#[test]
fn auth_exec_canonical_tool_script_contract_holds() {
    // Pins the CLI exec path to canonical advertised tool names. The
    // retired shell.run/process.exec bare aliases must stay rejected.
    run_bash_script("tests/scripts/test_check_auth_exec_canonical_tools.sh");
}

#[test]
fn credentials_username_contract_script_holds() {
    // Pins joined device credentials to carrying the stable username
    // slug. Runtime bootstrap must not recover by reading auth.json.
    run_bash_script("tests/scripts/test_check_credentials_username_contract.sh");
}

#[test]
fn canonical_hub_ura_boundary_script_holds() {
    // Pins product Hub identity to Axon's canonical /authority builder/parser
    // and keeps the projected identity singleton-only.
    run_bash_script("tests/scripts/test_check_canonical_hub_ura_boundary.sh");
}

#[test]
fn cli_flat_command_boundary_script_holds() {
    // Pins the top-level CLI to noun-first command groups. The retired
    // join/start/stop flat shortcuts must not come back.
    run_bash_script("tests/scripts/test_check_cli_flat_command_boundary.sh");
}

#[test]
fn cli_ability_invoke_ura_script_contract_holds() {
    // Pins the human CLI ability-invoke front door to canonical
    // Ability URA selectors, not retired bare registry names.
    run_bash_script("tests/scripts/test_check_cli_ability_invoke_ura.sh");
}

#[test]
fn pages_api_ability_ura_script_contract_holds() {
    // Pins Pages API kind=ability manifests to canonical Ability URA
    // selectors, not retired local registry ability names.
    run_bash_script("tests/scripts/test_check_pages_api_ability_ura.sh");
}

#[test]
fn pages_cli_ability_boundary_script_contract_holds() {
    // Pins the Pages CLI facade to a single typed local selector
    // instead of scattering <user>.pages.* registry-key formatting.
    run_bash_script("tests/scripts/test_check_pages_cli_ability_boundary.sh");
}

#[test]
fn ability_catalogue_scope_boundary_script_holds() {
    // Pins ability-list catalogue filtering to owner_ura/ability_ura scope
    // projection. Invocation subject vocabulary must not leak past the public
    // CLI flag boundary.
    run_bash_script("tests/scripts/test_check_ability_catalogue_scope_boundary.sh");
}

#[test]
fn openai_model_ability_ura_script_contract_holds() {
    // Pins the OpenAI-compatible model field to canonical
    // agent-owned chat Ability URAs, not bare provider names.
    run_bash_script("tests/scripts/test_check_openai_model_ability_ura.sh");
}

#[test]
fn voice_call_product_contract_script_holds() {
    // Pins voice call signaling responses to its product-owned contract:
    // stable wire names plus numeric codes, not retired dual compatibility
    // fields such as state_proto/end_reason_proto.
    run_bash_script("tests/scripts/test_check_voice_call_product_contract.sh");
}

#[test]
fn media_screen_target_provider_boundary_script_holds() {
    // Pins media resource bootstrap to one authoritative screen-target
    // discovery provider per platform; macOS native discovery must not
    // silently repopulate durable resource URAs through xcap fallback.
    run_bash_script("tests/scripts/test_check_media_screen_target_provider_boundary.sh");
}

#[test]
fn orchestration_service_boundary_script_holds() {
    // Pins mission.discuss_round session continuity to a registry-
    // scoped service and keeps agent-cycle inputs grouped.
    run_bash_script("tests/scripts/test_check_orchestration_service_boundary.sh");
}

#[test]
fn mission_think_subject_boundary_script_holds() {
    // Pins mission.think CLI dispatch to a projected request plus explicit
    // local daemon subject issuer, not the generic local invoke shortcut.
    run_bash_script("tests/scripts/test_check_mission_think_subject_boundary.sh");
}

#[test]
fn mission_discuss_subject_boundary_script_holds() {
    // Pins mission discuss CLI dispatch to projected requests plus explicit
    // local daemon subject issuer, not the generic local invoke shortcut.
    run_bash_script("tests/scripts/test_check_mission_discuss_subject_boundary.sh");
}

#[test]
fn mission_runs_cli_facade_boundary_script_holds() {
    // Pins cli::mission_runs to the read/cancel facade. Daemon execution
    // service types must not leak through a wildcard re-export.
    run_bash_script("tests/scripts/test_check_mission_runs_cli_facade_boundary.sh");
}

#[test]
fn agent_registry_key_boundary_script_holds() {
    // Pins persisted Agent registry keys to canonical tenant/name form while
    // runtime catalogue and ability projection use the Agent surface name.
    // No EAL or hot-registration fallback may re-admit bare registry rows.
    run_bash_script("tests/scripts/test_check_agent_registry_key_boundary.sh");
}

#[test]
fn cli_timeout_policy_boundary_script_holds() {
    // Pins CLI timeout semantics to named policies so transport guards and
    // runtime-default request deadlines do not fork at call sites.
    run_bash_script("tests/scripts/test_check_cli_timeout_policy_boundary.sh");
}

#[test]
fn core_agent_module_boundary_script_holds() {
    // Pins core agent ontology ownership to core::agent::{id,spec}; retired
    // pre-structure core::agent_id/core::agent_spec module aliases stay deleted.
    run_bash_script("tests/scripts/test_check_core_agent_module_boundary.sh");
}

#[test]
fn local_daemon_socket_resolver_boundary_script_holds() {
    // Pins local daemon socket resolution to daemon_config. The support
    // transport layer must not reintroduce a resolver re-export shim.
    run_bash_script("tests/scripts/test_check_local_daemon_socket_resolver_boundary.sh");
}

#[test]
fn eal_interpreter_flat_call_boundary_script_holds() {
    // Pins EAL interpreter per-call execution helpers to explicit IrCall
    // inputs. Runtime block planning keeps the canonical IrStep enum owner.
    run_bash_script("tests/scripts/test_check_eal_interpreter_flat_call_boundary.sh");
}

#[test]
fn managed_signing_provider_owner_boundary_script_holds() {
    // Pins managed-signing provider trait ownership to the keyring provider
    // module. Ability handlers consume it but must not re-export it.
    run_bash_script("tests/scripts/test_check_managed_signing_provider_owner_boundary.sh");
}

#[test]
fn discover_scope_boundary_script_holds() {
    // Pins <agent>.discover to current scope literals only. The
    // retired easynet alias must stay absent from parser and schema.
    run_bash_script("tests/scripts/test_check_discover_scope_boundary.sh");
}

#[test]
fn chat_ability_input_boundary_script_holds() {
    // Pins <agent>.chat to the manifest-backed flat prompt/context shape as
    // canonical input, not a retired compatibility alias for a second model.
    run_bash_script("tests/scripts/test_check_chat_ability_input_boundary.sh");
}

#[test]
fn skill_list_managed_dir_boundary_script_holds() {
    // Pins skill.list to the current managed install directory
    // per agent type. Claude Code no longer scans root-level skills.
    run_bash_script("tests/scripts/test_check_skill_list_managed_dir_boundary.sh");
}

#[test]
fn hosted_receipt_axon_boundary_script_holds() {
    // Pins hosted receipt audit types to Axon's canonical module.
    // EasyNet-Cli must not restore the retired runtime re-export shim.
    run_bash_script("tests/scripts/test_check_hosted_receipt_axon_boundary.sh");
}

#[test]
fn cross_hub_trust_source_boundary_script_holds() {
    // Pins cross-hub federation dialing to the live SharedTrustAnchor source.
    // Boot-time RealmTrustAnchor snapshots must not return as a second trust
    // lifecycle.
    run_bash_script("tests/scripts/test_check_cross_hub_trust_source_boundary.sh");
}

#[test]
fn companion_status_package_version_boundary_script_holds() {
    // Pins Desktop companion status projection to the canonical package_version
    // field. The optional runtime version observation must not repair package
    // identity.
    run_bash_script("tests/scripts/test_check_companion_status_package_version_boundary.sh");
}

#[test]
fn plugin_independent_project_boundary_script_holds() {
    // Pins package-owned plugin projects to provider-neutral registration.
    // Desktop companion remains a manifest package kind, not a provider kind.
    run_bash_script("tests/scripts/test_check_plugin_independent_project_boundary.sh");
}

#[test]
fn namespace_resolve_qtype_boundary_script_holds() {
    // Pins local namespace.resolve ingress to canonical ResolveType enum
    // strings. It must not guess qtype from query shape or accept short/numeric
    // aliases.
    run_bash_script("tests/scripts/test_check_namespace_resolve_qtype_boundary.sh");
}

#[test]
fn mission_ability_vocabulary_boundary_script_holds() {
    // Pins mission orchestration discovery, errors, and prompts to the
    // canonical mission.* ability names after the retired easynet.* aliases
    // were removed from the registry.
    run_bash_script("tests/scripts/test_check_mission_ability_vocabulary_boundary.sh");
}

#[test]
fn workspace_agent_directory_boundary_script_holds() {
    // Pins runtime workspace projection to AgentDirectory input.
    // Dispatch must reject invalid registry roots instead of rebuilding
    // specs from AgentEntry or falling back to the caller cwd.
    run_bash_script("tests/scripts/test_check_workspace_agent_directory_boundary.sh");
}

#[test]
fn dispatch_mission_context_boundary_script_holds() {
    // Pins intra-mission agent dispatch to a typed mission context.
    // Missing or forged mission context must fail in every build mode.
    run_bash_script("tests/scripts/test_check_dispatch_mission_context_boundary.sh");
}

#[test]
fn plugin_control_subject_boundary_script_holds() {
    // Pins plugin control subject resolution to typed credential absence.
    // Existing malformed credentials must not be hidden as unpaired state.
    run_bash_script("tests/scripts/test_check_plugin_control_subject_boundary.sh");
}

#[test]
fn current_realm_hub_context_boundary_script_holds() {
    // Pins current-realm Hub dispatch to typed credential absence.
    // Existing malformed credentials must not trigger local voice fallback.
    run_bash_script("tests/scripts/test_check_current_realm_hub_context_boundary.sh");
}

#[test]
fn federation_revoke_caller_boundary_script_holds() {
    // Pins federation.revoke lifecycle callers to one explicit local daemon
    // caller fact instead of letting the helper reselect ambient identity.
    run_bash_script("tests/scripts/test_check_federation_revoke_caller_boundary.sh");
}

#[test]
fn call_create_participant_identity_boundary_script_holds() {
    // Pins call-create participant identity to typed credential absence.
    // Existing malformed credentials must not become hostname participants.
    run_bash_script("tests/scripts/test_check_call_create_participant_identity_boundary.sh");
}

#[test]
fn runtime_state_read_subject_boundary_script_holds() {
    // Pins runtime-state reads to an explicit control-discovery daemon subject.
    // History/catalog/status/watch must not re-enter implicit daemon-self reads.
    run_bash_script("tests/scripts/test_check_runtime_state_read_subject_boundary.sh");
}

#[test]
fn status_pairing_state_boundary_script_holds() {
    // Pins runtime status pairing diagnostics to explicit paired/unpaired/invalid
    // states. Invalid credentials must not render as ordinary unpaired setup.
    run_bash_script("tests/scripts/test_check_status_pairing_state_boundary.sh");
}

#[test]
fn start_credential_readiness_boundary_script_holds() {
    // Pins daemon start preflight to explicit ready/missing/invalid credential
    // states. Invalid existing credentials must not render as first-run setup.
    run_bash_script("tests/scripts/test_check_start_credential_readiness_boundary.sh");
}

#[test]
fn reset_credential_state_boundary_script_holds() {
    // Pins reset cleanup to explicit paired/missing/invalid credential states.
    // Invalid local credentials may be deleted, but must not look absent.
    run_bash_script("tests/scripts/test_check_reset_credential_state_boundary.sh");
}

#[test]
fn start_ready_signer_proof_boundary_script_holds() {
    // Pins fresh device start to the same paired-user signer proof required
    // when attaching to an existing daemon.
    run_bash_script("tests/scripts/test_check_start_ready_signer_proof_boundary.sh");
}

#[test]
fn runtime_abilities_manifest_boundary_script_holds() {
    // Pins per-agent ability discovery to authored manifests under
    // AgentDirectory. Missing roots must not synthesize chat abilities.
    run_bash_script("tests/scripts/test_check_runtime_abilities_manifest_boundary.sh");
}

#[test]
fn ability_catalog_row_boundary_script_holds() {
    // Pins CLI catalogue rendering to canonical row fields. Retired
    // ability_name/tool_name aliases must fail closed instead of being
    // silently ignored in the presentation projector.
    run_bash_script("tests/scripts/test_check_ability_catalog_row_boundary.sh");
}

#[test]
fn local_daemon_subject_owner_boundary_script_holds() {
    // Pins local daemon subject selection to LocalDaemonSystemAbilityIssuer
    // backed by daemon identity ownership. The gRPC transport must not expose
    // a second authority-subject helper.
    run_bash_script("tests/scripts/test_check_local_daemon_subject_owner_boundary.sh");
}

#[test]
fn go_sdk_lifecycle_fixture_neutrality_script_holds() {
    // Pins Go SDK runtime-host lifecycle fixtures to canonical runtime
    // lifecycle concepts. Product companion lifecycle rows must not return as
    // test-only SDK model evidence.
    run_bash_script("tests/scripts/test_check_go_sdk_lifecycle_fixture_neutrality.sh");
}

#[test]
fn sdk_doc_product_vocabulary_script_holds() {
    // Pins SDK-facing documentation to canonical runtime vocabulary. Product
    // names and workflow examples must not become the way SDK boundaries are
    // described.
    run_bash_script("tests/scripts/test_check_sdk_doc_product_vocabulary.sh");
}

#[test]
fn permission_broker_headless_policy_script_holds() {
    // Pins permission admission to explicit headless/interactive operator
    // states. Headless operation must not be modeled as a legacy allow-all
    // compatibility broker.
    run_bash_script("tests/scripts/test_check_permission_broker_headless_policy.sh");
}

#[test]
fn driver_command_state_boundary_script_holds() {
    // Pins mission driver command resolution to a typed default/explicit state.
    // Runtime drivers must not infer default binaries from empty strings.
    run_bash_script("tests/scripts/test_check_driver_command_state_boundary.sh");
}

#[test]
fn mcp_cost_metadata_projection_boundary_script_holds() {
    // Pins MCP tool cost projection to declared/undeclared metadata states.
    // The edge must not infer free/default cost through fallback helpers.
    run_bash_script("tests/scripts/test_check_mcp_cost_metadata_projection_boundary.sh");
}

#[test]
fn device_ability_call_mode_resolution_boundary_script_holds() {
    // Pins dynamic ability deployment call-mode ownership to one registrar value
    // object. Install, uninstall, replay, and runtime proof binding must not
    // reintroduce procedural inference helpers.
    run_bash_script("tests/scripts/test_check_device_ability_call_mode_resolution_boundary.sh");
}

#[test]
fn invocation_wire_entity_ref_kind_resolution_boundary_script_holds() {
    // Pins protobuf EntityRef kind projection to one explicit wire-resolution
    // object. Invocation wire construction must not reintroduce subject-kind
    // inference helpers or fallback vocabulary.
    run_bash_script(
        "tests/scripts/test_check_invocation_wire_entity_ref_kind_resolution_boundary.sh",
    );
}

#[test]
fn catalog_schema_projection_boundary_script_holds() {
    // Pins catalogue schema publication to declared/undeclared projection
    // states. Unknown ability names must not be modeled as local fallback
    // schema branches.
    run_bash_script("tests/scripts/test_check_catalog_schema_projection_boundary.sh");
}

#[test]
fn mcp_reflection_concurrency_resolution_boundary_script_holds() {
    // Pins MCP reflection fan-out configuration to configured/defaulted
    // resolution states. Malformed env values must not be hidden behind
    // procedural fallback helpers.
    run_bash_script("tests/scripts/test_check_mcp_reflection_concurrency_resolution_boundary.sh");
}

#[test]
fn runtime_stop_lifecycle_boundary_script_holds() {
    // Pins daemon process-stop probing/signaling to the lifecycle module.
    // CLI stop may render typed outcomes, but must not own pidfile/discovery
    // or pgrep process lifecycle transitions.
    run_bash_script("tests/scripts/test_check_runtime_stop_lifecycle_boundary.sh");
}

#[test]
fn transport_locator_terminology_boundary_script_holds() {
    // Pins transport-library locator names behind explicit aliases so transport
    // vocabulary cannot leak back into runtime identity/addressing code.
    run_bash_script("tests/scripts/test_check_transport_locator_terminology_boundary.sh");
}
