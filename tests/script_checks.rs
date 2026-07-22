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
fn ffi_abi_v5_exact_surface_script_contract_holds() {
    // Pins the generic binding boundary: exact header/source/dylib allowlist,
    // error code table, daemon lifecycle, and complete Invocation lifecycle.
    run_bash_script("tests/scripts/test_check_ffi_abi_v5_header.sh");
}

#[test]
fn release_package_contract_script_holds() {
    // Pins the release shape consumed by install.sh: runtime binaries,
    // dendrite bridge, and ABI v5 binding artefacts must be packaged
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
fn invoke_ability_ura_input_script_contract_holds() {
    // Pins <agent>.invoke to canonical ability_ura input only. The
    // retired target/ability request fields must stay rejected.
    run_bash_script("tests/scripts/test_check_invoke_ability_ura_input.sh");
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
