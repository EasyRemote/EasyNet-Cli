# Verification

Commands and outcomes will be appended after implementation.

## 2026-07-21 Pairing auto-wire credential facts

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q federation_wire --lib`
  — PASS (`20 passed`); includes regressions that federated peer auto-wire
  rejects blank pairing `realm`, realm-trust auto-wire rejects blank
  `realm`/`node_id`, and absent daemon-config remains the only successful
  no-local-hub-config no-op.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "PairingTrustFacts auto_wire_self_realm_trust_from_credentials
  auto_wire_federated_peer_from_credentials required pairing realm node_id let
  _"` — PASS; reports both public auto-wire helpers using
  `required_pairing_fact`, realm-trust projection using `PairingTrustFacts`,
  and `runtime start` as a caller of the fallible helper.
- `rg -n "empty tenant is no-op|empty node_id is a no-op|silent no-op|let _ =
  super::federation_wire::auto_wire_self_realm_trust_from_credentials|creds\.realm\.trim\(\)\.is_empty\(\).*return
  Ok|creds\.node_id\.trim\(\)\.is_empty\(\).*return
  Ok|device_ura\(creds\.realm|hub_ura\(creds\.realm|hub_tls_ca_path_for_join\(creds\.realm"
  src/cli/commands/federation_wire.rs src/cli/commands/start.rs` — PASS; no
  legacy no-op, swallowed trust auto-wire result, or inner raw credential URA
  projection remained.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-21 User-device directory projection fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q list_user_devices --lib`
  — PASS (`15 passed`; covers local malformed presence rejection, peer
  response `node_id`/Device URA binding rejection, selected peer without
  federation client failure, and malformed peer directory response failure).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q invoke_federation_proxy_list_user_devices --lib`
  — PASS (`2 passed`; selected peer scope cannot become an empty successful
  list when federation transport or peer schema is invalid).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q invoke_federation_list_user_devices_rejects_malformed_device_presence --lib`
  — PASS (`1 passed`; same-realm malformed Device presence fails in-band).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore "dispatch_federation_proxy_list_user_devices selected peer_hub_urls empty success fanout error validate_list_user_devices_response"`
  — PASS; confirms the current proxy path validates peer rows and converts
  selected-peer fanout errors into explicit failure before response merge.

## 2026-07-21 Namespace proxy resolve fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q namespace_proxy_resolve --lib`
  — PASS (`5 passed`; covers successful typed peer resolve, missing qtype
  rejection, legacy input alias rejection, selected peer without federation
  client failure, and malformed peer record schema failure).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore "dispatch_namespace_proxy_resolve selected peer_hub_urls namespace_proxy_resolve_merge_answer namespace_record_merge_key recordType unwrap_or_default fanout_errors"`
  — PASS; confirms selected peer failures flow to explicit failure and peer
  records are validated through the canonical merge-key helper before merge.

## 2026-07-20 InvokeBidi receipt payload projection fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q local_invoke --lib --features axon-pb`
  — PASS (`11 passed`; includes lossless BinaryChunk projection and receipt
  payload rejection for non-JSON content type / malformed JSON).
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --features axon-pb`
  — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R77 for single-owner InvokeBidi down-frame projection and fail-closed receipt
  payload parsing.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `DownPayload::Receipt` malformed payload fallback to
  `data_b64`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `git diff --check` — PASS.
- `rg -n "DownPayload::Receipt[\\s\\S]{0,900}data_b64|serde_json::from_slice\\(&receipt\\.payload\\)\\.unwrap_or_else" ...`
  — PASS; no production matches in local/remote bidi drain projection.

## 2026-07-20 Cross-Hub peer envelope subject fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q peer_envelope_signer --lib --features axon-pb`
  — PASS (`8 passed`; includes forwarded-caller rejection when caller URA is
  absent and explicit-subject acceptance for fresh daemon-owned peer requests).
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --features axon-pb`
  — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R76 for explicit peer envelope subject state.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for optional caller envelope defaulting and subject
  fallback to `target_ura`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Plugin wire profile core-only fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --features remote-desktop`
  — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R75, which rejects daemon plugin wire profile fallback to core-only registry,
  independent transport profile reload, and process-global helpers that swallow
  default profile load errors.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture covering `PluginRuntimeManager::new() -> Self`,
  `unwrap_or_else(|_| AbilityWireRegistry::core())`, transport
  `ability_wire_registry_load_failed` downgrade, and a free
  `bidi_wire_kind_for(...)` helper that calls `load_default_profile().ok()`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 API key credential-store parse fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q api_key --lib` — PASS
  (`5 passed`; includes missing store as fresh-install empty state and
  malformed store rejection for list/create/bearer resolution).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R68, which requires `load_store() -> anyhow::Result<ApiKeyStore>` and rejects
  parse fallback to an empty credential store.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `toml::from_str(&text).unwrap_or_default()` inside the
  API key credential-store loader.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query load_store --limit 40`
  — PASS; reports `api_key.rs::load_store() -> anyhow::Result<ApiKeyStore>`.
- `rg -n "toml::from_str\\(&text\\)\\.unwrap_or_default\\(|fn load_store\\(\\) -> ApiKeyStore|parse API key store" ...`
  — PASS; legacy parse fallback appears only in architecture-gate/self-test
  negative fixtures, while production `api_key.rs` carries the parse diagnostic.

## 2026-07-20 EAL agent registry unavailable-state fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q eal::interpreter::dispatch::tests::dispatcher_accepts_missing_registry_as_first_run_empty_state --lib`
  — PASS (`1 passed`; missing `agents.json` remains the valid first-run empty
  registry state owned by registry persistence).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q eal::interpreter::dispatch::tests::dispatcher_rejects_malformed_registry_instead_of_empty_fallback --lib`
  — PASS (`1 passed`; malformed `agents.json` prevents
  `AgentAwareDispatcher` construction instead of becoming an empty registry).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query load_registry_or_warn --limit 30`
  — PASS; no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query load_registry_projection_for_dispatch --limit 30`
  — PASS; reports the fail-closed EAL registry projection loader.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after updating
  R41 to require `AgentAggregateRepository::load_registered_agent_registry_projection()`
  and reject empty-registry fallbacks.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for repository-backed `unwrap_or_default()` fallback.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Ability catalogue descriptor-ref synthesis removal

- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query schema_bound_catalogue_entry --limit 30` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query enrich_descriptor_ref --limit 30` — PASS; no results.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q ability_catalog --lib` — PASS (`13 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q abilities --lib` — PASS (`123 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Local daemon loopback subject fallback removal

- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query LocalDaemonLoopbackSubjectPolicy --limit 30` — PASS; reports only the explicit `LocalDaemonSelf` and `Explicit` policy variants.
- `/Users/macbook.silan.tech/.local/bin/codegraph query invoke_local_ability_with_subject --limit 30` — PASS; reports helper signatures requiring `subject_ura: &str`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q local_daemon --lib` — PASS (`16 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Ability target tuple default fallback removal

- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query AbilityTargetRequest --limit 30` — PASS; confirmed the Python target facade routes through one `AbilityTargetRequest` model.
- `/Users/macbook.silan.tech/.local/bin/codegraph query buildWithCallMode --limit 30` — PASS; confirmed the Go target builder seam.
- `source sdk/conformance/python_toolchain.sh && resolve_sdk_python_toolchain "$PWD" pytest && cd sdk/python && "$SDK_CONFORMANCE_PYTHON" -m pytest -q tests/test_ability_invocation.py` — PASS (`18 passed, 4 subtests passed`).
- `source sdk/conformance/toolchain_path.sh && resolve_sdk_toolchain_path "$PWD" && gofmt -w sdk/go/runtime_ability.go sdk/go/runtime_ability_test.go && cd sdk/go && go test -run 'TestRuntimeAbilityClientBuildRequiresExplicitCallMode|TestRuntimeClientResolveDescriptorRefRequiresCallMode|TestRuntimeAbilityClientBuildsCompleteCanonicalDraft' -count=1` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — initially failed with `sdk_concepts: provider_implementation_mismatch:go:ability_invocation_facade`, proving the Go/Python SDK source attestation detected the changed provider implementation.
- `source sdk/conformance/toolchain_path.sh && resolve_sdk_toolchain_path "$PWD" && source sdk/conformance/python_toolchain.sh && resolve_sdk_python_toolchain "$PWD" pytest && "$SDK_CONFORMANCE_PYTHON" sdk/conformance/rebuild_public_api_model.py --write` — PASS; refreshed `sdk/conformance/canonical-public-api.json` and `sdk/conformance/sdk-parity-matrix.json` for the ability invocation facade hash changes.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS after attestation refresh.

## 2026-07-20 Doctor agent projection fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q agent_ --lib` — PASS
  (`305 passed`; includes the new doctor agent projection tests and the
  centralized local CLI probe mapping test).
- `rg -n "is_claude_code\\(|filter_map\\(\\|row\\||not claude|Vec::new\\(\\)" src/cli/commands/doctor.rs src/cli/commands/agent/inspect.rs src/cli/daemon_client/agent_view.rs src/cli/commands/agent_cli_probe.rs`
  — no legacy probe-selection or unavailable-registry fallback matches in the
  inspected agent diagnostic surfaces.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query LocalAgentCliProbe` —
  reports the new `src/cli/commands/agent_cli_probe.rs` enum, `for_runtime`,
  `run`, and the two command imports.
- `/Users/macbook.silan.tech/.local/bin/codegraph query is_claude_code` — no
  results.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Descriptor catalog matched-row schema fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q descriptor_catalog_resolution --lib`
  — PASS (`1 passed`; Rust FFI rejects matching catalog rows that omit
  `descriptor_ref`; descriptor resolver error projection reports schema-bad
  catalog rows as `INVALID_ARGUMENT` at `provider_payload`, not
  `DESCRIPTOR_NOT_FOUND`).
- `source sdk/conformance/toolchain_path.sh && resolve_sdk_toolchain_path "$PWD" && gofmt -w sdk/go/cabi_runtime.go sdk/go/cabi_runtime_resolver_test.go && (cd sdk/go && go test -tags easynet_cabi -run TestResolveDescriptorRefFromDiagnostics -count=1)`
  — PASS.
- `source sdk/conformance/python_toolchain.sh && resolve_sdk_python_toolchain "$PWD" pytest && (cd sdk/python && "$SDK_CONFORMANCE_PYTHON" -m pytest -q tests/test_cabi.py -k descriptor_diagnostics)`
  — PASS (`2 passed, 24 deselected`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- Codegraph queries for `descriptor_catalog_resolution_from_entries`,
  `cabiRequiredCatalogString`, and `_required_catalog_entry_string` report the
  Rust/Go/Python required-row validators.
- `rg -n 'descriptor_ref.*unwrap_or_default|entry\\.get\\(\"descriptor_ref\"\\).*or \"\"|descriptorRef := strings\\.TrimSpace\\(cabiStringOrEmpty\\(entry\\[\"descriptor_ref\"\\]\\)\\)' src/ffi/invocation/mod.rs sdk/go/cabi_runtime.go sdk/python/easynet_sdk/_cabi.py`
  — no matched-row descriptor_ref empty projection remains.
- `source sdk/conformance/toolchain_path.sh && resolve_sdk_toolchain_path "$PWD" && source sdk/conformance/python_toolchain.sh && resolve_sdk_python_toolchain "$PWD" pytest && "$SDK_CONFORMANCE_PYTHON" sdk/conformance/rebuild_public_api_model.py --write`
  — PASS; refreshed `sdk/conformance/canonical-public-api.json` for the Go
  `cabi_runtime_provider` source attestation.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Authority tuple empty-identity fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q authority_binding --lib`
  — PASS (`5 passed`; includes missing session subject and missing delegation
  callee being reported as `ENVELOPE_INCOMPLETE`, not authority
  mismatch/audience violation).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query subject_ura_required`
  — reports the canonical subject tuple helper.
- `rg -n 'verify_authority_proof_metadata|verify_session_authority_bindings|verify_delegation_bindings|caller_ura_required|callee_ura_required|subject_ura_required|unwrap_or_default\\(\\)|unwrap_or\\(\"\"\\)' src/daemon/invocation/admission/admission_facade.rs`
  — target verifier paths use required tuple helpers and no longer contain
  empty identity defaults.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 FFI caller signature key identity fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q caller_signature --lib`
  — PASS (`5 passed`; includes explicit-signature preservation plus missing
  and blank `key_id_hint` rejections for canonical invocation JSON and detached
  signature JSON).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query caller_signature_key_id_hint`
  — reports the single FFI helper now delegating to `required_string`.
- `rg -n 'optional_string\\(obj, "key_id_hint"\\)|optional_string\\(obj, "signer_public_key_base64"\\)|signer_public_key_base64.*key_hint|projects_signer_pubkey' src/ffi/invocation/mod.rs`
  — no production fallback remains.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.

## 2026-07-20 Authority metadata clock fallback and invocation signer custody removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test authority_metadata --lib`
  — PASS (`6 passed`; includes pre-epoch authority clock rejection instead of
  epoch-zero projection).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test receipt_signing --lib`
  — PASS (`7 passed`; includes raw signer capability rejection when no owned
  self-signed authority or hosted lease exists).
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R78 for authority clock fail-closed projection and R79 for invocation signer
  custody authority.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  negative fixtures for epoch-zero authority clock fallback and
  `strict_identity(caller_ura).ok()` signer authority construction.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo check --lib --bins`
  — PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  REASON_AUTHORITY_CLOCK_UNAVAILABLE --limit 20` — PASS; reports the explicit
  authority clock state.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  invocation_signing_requires_owned_authority_not_just_a_signer_key --limit 20`
  — PASS; reports the regression test proving raw signer capabilities no
  longer imply invocation authority.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Runtime caller signer custody fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q runtime_caller --lib`
  — PASS (`2 passed`; covers explicit caller custody classification and the
  User-caller missing-managed-key path using the real test key-service
  transport).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query RuntimeCallerSignerResolver`
  — reports the single resolver object in
  `src/daemon/identity/self_identity.rs`.
- `/Users/macbook.silan.tech/.local/bin/codegraph query register_paired_user_runtime_signer`
  — reports the boot-time paired User signer registration owner in
  `src/daemon/boot/invocation/mod.rs`.
- `rg -n 'Err\(_) => return Ok\(\(\)\)|user_ura\(\).*return Ok\(\(\)\)|is_user_owner_ura\(|load_runtime_caller_signer\(' src/daemon/identity/self_identity.rs src/daemon/boot/invocation/mod.rs src/daemon/invocation src/ffi -S`
  — no silent paired-User boot skip remains; production remote invocation
  callers still enter the single signer resolver.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Device show legacy state projection removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q device_show --lib` —
  PASS (`5 passed`).
- `rg -n "integer-indexed|Axon SDK enum|1 => \"JOINING\"|unwrap_or_else\\(\\|\\| \"UNKNOWN\"\\.to_string\\(\\)\\)" src/cli/commands/groups/device.rs`
  — no matches after removing the legacy numeric-state mapping.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query device_show_state` —
  reports the new schema-bound extractor in
  `src/cli/commands/groups/device.rs`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 LocalDevice resource subject migration removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q resources --lib` — PASS
  (`215 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q remote_desktop --lib` —
  PASS (`94 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q media --lib` — PASS
  (`79 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q real_invoke --lib` —
  PASS (`139 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Remote-device local-realm target fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q remote_device --lib` —
  PASS (`5 passed`).
- `rg 'local_realm_fallback|local realm fallback|directory_hit_beats|wrap.*local realm|wrapping.*local realm|legacy local-realm' src/support/platform/remote_device.rs`
  — no matches after renaming the negative tests.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query local_realm_fallback`
  — no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query directory_hit_beats_local_realm_fallback`
  — no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query local_realm_fallback_is_used_when_directory_misses`
  — no results.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS after
  applying rustfmt to the edited file.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; index was
  already up to date.
- `/Users/macbook.silan.tech/.local/bin/codegraph query canonical_resource_ura_for_existing`
  — no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query legacy_bootstrap_screen_target`
  — no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query upsert_pre_join_records_empty_owner_agent`
  — no results.
- `rg 'canonical_resource_ura_for_existing|upsert_migrates_legacy_device_resource_ura_to_stream_subject_shape|upsert_pre_join_records_empty_owner_agent|legacy_bootstrap_screen_target|old single-segment|migrated on the next upsert|patched on next save' ...`
  — no matches in the resource store/bootstrap files.

## 2026-07-20 Recording artifact content-type fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q ability_record --lib` —
  PASS (`7 passed`).
- `rg 'fallback_content_type|audio/L16"|image/jpeg"|unwrap_or_else\\(\\|\\| kind' src/cli/commands/ability_record.rs`
  — no matches.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query fallback_content_type`
  — no results.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS after
  applying rustfmt to the edited file.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20

- `go test ./...` in `sdk/go` — PASS.
- `cargo fmt --check` — PASS.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — PASS.
- `python sdk/conformance/edge_adapter_policy.py` through the SDK Python
  toolchain — PASS.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-sdk-cutover-readiness.sh` — current run FAILS only
  on SDK evidence completeness gates:
  - `SDK conformance reports` initially failed on stale adapter evidence; after
    `python3 sdk/conformance/refresh_adapter_report_evidence.py --write`,
    `--check` passes. A full rerun of
    `bash tools/scripts/check-sdk-conformance-reports.sh` with the bundled Node
    toolchain on `PATH` also passes.
  - `SDK live parity matrix` still fails with
    `missing_live_results:rust,c_abi,go,python,node,java,swift`.
  - Later runtime/product gates passed in the same run: canonical runtime
    convergence V2, SDK receipt URA boundary, Python SDK static contract,
    daemon latest input boundary, daemon Invocation migration, release package
    contract, EasyRemote SDK boundary, backend route-family coverage, backend
    SDK-only boundary, downstream SDK consumer cutover, product key-custody
    boundary, EasyRemote/backend product smokes, runtime events cross-repo,
    runtime events live daemon E2E, standalone Hub PrincipalLifecycle E2E,
    Python SDK live smoke, and Go SDK live smoke.
# Verification

## 2026-07-20 LedgerSink fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test ledger_ --lib -- --nocapture` — PASS
  (`12 passed`), including:
  - `ledger_route_resolver_rejects_authority_bare_ability_instead_of_system_fallback`
  - `ledger_route_resolver_rejects_unowned_route_instead_of_system_fallback`
  - `ledger_invocation_resolver_rejects_unowned_record_instead_of_system_fallback`
- `/Users/macbook.silan.tech/.cargo/bin/cargo test call_tool_rejects_retired_canonical_dotted_alias --lib -- --nocapture` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test canonical_dotted_tool_call_is_rejected --lib -- --nocapture` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Mission model fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test resolve_model --lib -- --nocapture` — PASS
  (`14 passed`; includes the mission model resolver tests and unrelated
  OpenAI model resolver tests matched by the filter).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test entry_model_is_ignored_when_spec_model_is_none --lib -- --nocapture` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test spec_model_is_written_to_meta_json_when_set --lib -- --nocapture` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test resolve_model_with_overrides_per_call_wins_over_spec --lib -- --nocapture` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Mission timeout fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test resolve_timeout --lib -- --nocapture` — PASS (`3 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test resolve_model --lib -- --nocapture` — PASS (`14 passed`; re-ran model resolver coverage while expanding R66).
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Trust-anchor user-key fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test user_multi_pubkey_lookup_requires_presented_pubkey --lib -- --nocapture` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test resolve_rejects_bare_user_ura --lib -- --nocapture` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test same_realm_user_existing_different_key_still_syncs_presented_key --lib -- --nocapture` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test daemon::trust:: --lib -- --nocapture` — PASS (`50 passed`).
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Descriptor call-mode ingress fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test runtime_descriptor_resolver_requires_explicit_call_mode --lib -- --nocapture` — PASS.
- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_runtime.py::RuntimeTests::test_descriptor_resolution_rejects_blank_call_mode sdk/python/tests/test_cabi.py::CABIDescriptorDiagnosticsTests::test_descriptor_diagnostics_requires_call_mode -q` — PASS (`2 passed`).
- `(cd sdk/go && /opt/homebrew/bin/go test . -run 'TestRuntimeClientResolveDescriptorRefRequiresCallMode|TestResolveDescriptorRefFromDiagnosticsRequiresCallMode')` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Python key-service compatibility facade removal

- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_managed_signing.py`
  — PASS (`18 passed`).
- `bash tests/scripts/test_check_sdk_product_neutrality.sh` — PASS.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `git diff --check` — PASS.

## 2026-07-20 Go daemon lifecycle compatibility facade removal

- `(cd sdk/go && /opt/homebrew/bin/go test ./...)` — PASS.
- `bash tests/scripts/test_check_sdk_product_neutrality.sh` — PASS.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cargo/bin:/opt/homebrew/bin:/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH" sdk/python/.venv/bin/python sdk/conformance/edge_adapter_policy.py --self-test`
  — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cargo/bin:/opt/homebrew/bin:/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH" bash tools/scripts/check-sdk-canonical-public-api.sh`
  — PASS.
- `git diff --check` — PASS.

## 2026-07-20 Local daemon-system subject fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q loopback_tuple_plan_requires_explicit_targeted_subject` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q local_system_context_requires_complete_explicit_facts` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `bash tools/scripts/check-daemon-invocation-migration.sh` — PASS.
- `bash tests/scripts/test_check_daemon_invocation_migration.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `git diff --check` — PASS.
- `codegraph query CallerDeclaredSubject` — no results.
- `codegraph query targeted_root_with_declared_subject` — no results.
- `codegraph query explicit_or_caller_declared` — no results.

## 2026-07-20 Agent workspaces layout migration removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q agents_root_is_canonical_even_before_the_directory_exists` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q external_agent_type_round_trips_and_has_no_default_command` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q registry` — PASS (`100 passed`).
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `git diff --check` — PASS.
- `codegraph query migrate_legacy_agents_directory` — no results.
- `codegraph query legacy_agents_root` — no results.
- `codegraph query migrate_registry_root_paths` — no results.

## 2026-07-20 Authority all-zero principal rejection

- `(cd sdk/go && /opt/homebrew/bin/go test ./...)` — PASS.
- `(cd sdk/node && /Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node --test test/runtime-core.test.mjs)` — PASS (`11 passed`).
- `(cd sdk/java && mvn test -Dtest=RuntimeCoreSeamTest)` — PASS.
- `(cd sdk/swift && swift test --filter RuntimeCoreSeamTests)` — PASS (`18 passed`).
- `/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3 -m py_compile ...` for changed Python SDK files/tests — PASS.
- `bash tools/scripts/check-python-sdk-static-contract.sh` — PASS.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — PASS.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `check-sdk-cutover-readiness.sh` was run until broader downstream failures:
  stale adapter evidence / missing live parity results and existing backend
  build errors around removed Go daemon compatibility names
  (`ModeHub`, `ModeBoth`, `DaemonMode`, `StartConfig`). The authority-specific
  Python static failure found during that run was fixed and re-verified.

## 2026-07-20 Paired User signer lifecycle convergence

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q user_signing_identity --lib`
  — PASS (`4 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q load_credentials_rejects_all_zero_user_id --lib`
  — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q managed_user_runtime_signer_signs_with_subject_bound_inventory_key --lib`
  — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-sdk-product-neutrality.sh` — PASS.
- `git diff --check` — PASS.
- `codegraph impact ensure_user_runtime_signing_identity` — current production
  impact is daemon Invocation boot plus the explicit CLI
  `auth signing-key register` facade; `src/cli/commands/start.rs` is no longer
  a signer lifecycle owner.
- A pre-existing `check-sdk-cutover-readiness.sh` run from an earlier source
  snapshot failed later in Go SDK live smoke because that snapshot did not yet
  contain `register_paired_user_runtime_signer(...)`. Current worktree
  `cargo check --lib` and SPEC v2 gate compile the fixed symbol successfully.

## 2026-07-20 Plugin realtime transport fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q plugins::manifest --lib`
  — PASS (`7 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q plugins::realtime --lib`
  — PASS (`5 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q plugins::broker --lib`
  — PASS (`3 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q remote_desktop --lib`
  — PASS (`94 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q plugin --lib`
  — PASS (`226 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.

## 2026-07-20 Global skill directory-name identity fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q global_skill --lib` —
  PASS (`6 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q global_skill_pool_cache --lib`
  — PASS (`2 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q tree_and_read_file_resolve_global_pool_skill_returned_by_list --lib`
  — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q tree_and_read_file_accept_unscoped_global_pool_owner --lib`
  — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `rg` for directory-name identity fallback patterns under
  `src/daemon/resources/skills` and
  `src/daemon/ability/builtins/resources/skills` — no production/test matches;
  only this plan pack's audit text describes the removed fallback.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query global_skill_pool_ref_resolves_directory_name_without_alias_scan`
  — no results.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Runtime status empty-fleet fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q fleet_directory --lib`
  — PASS (`2 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query require_fleet_directory_entries`
  — PASS; indexed the new fail-closed helper in `status.rs`.
- `rg 'fn fetch_directory_entries\(\) -> Vec<Value>|Fleet: cannot query.*Vec::new' src/cli/commands/status.rs`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Federation peer inspection fail-closed config parsing

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q federation_peers --lib`
  — PASS (`8 passed`).
- `rg 'read_federated_peers\\(\\)\\.unwrap_or_default|read_trusted_hubs\\(\\)\\.unwrap_or_default|unwrap_or\\(\"\"\\).*agent_ura' src/cli/commands/federation_peers.rs`
  — PASS; no matches.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "read_federated_peers().unwrap_or_default"`
  — PASS; no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query read_federated_peers_from_path`
  — PASS; path-level reader is indexed.
- `/Users/macbook.silan.tech/.local/bin/codegraph query malformed_daemon_config_fails_closed`
  — PASS; failure-path test is indexed.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Device show target and ability fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q device_show --lib`
  — PASS (`4 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query classify_device_show_target`
  — PASS; indexed the explicit target classifier.
- `/Users/macbook.silan.tech/.local/bin/codegraph query device_show_abilities`
  — PASS; indexed the fail-closed ability extractor.
- `rg 'same-realm fallback|node\.describe.*fallback|Err\(_\) => Vec::new\(\)|meta\.list_abilities.*fallback|fall back to.*local realm' src/cli/commands/groups/device.rs`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Device remove same-realm target fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q device_remove --lib`
  — PASS (`2 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q device_show --lib`
  — PASS (`4 passed`), proving the adjacent canonical target classifier still
  holds.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query canonicalize_remove_target_ura`
  — PASS; indexed the single remove-target canonicalization helper.
- `rg 'device_ura\(&local_tenant, trimmed\)|pair this device first|same-realm.*remove|remove.*same-realm' src/cli/commands/groups/device.rs`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Chat session inventory fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q chat_sessions --lib`
  — PASS (`16 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q chat_history --lib`
  — PASS (`6 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q stream_handler_resolves_lifelong_sentinel_against_bound_pointer --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query rescan_dir` — no
  results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query list_sessions_falls_back_to_dir_scan_when_index_missing`
  — no results.
- `rg 'fallback when the index|directory scan|rescan_dir|falls_back_to_dir_scan|corrupt cache'
  src/daemon/persistence/chat_sessions.rs` — no results.

## 2026-07-20 MCP reflection mode fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q reflection_mode --lib`
  — PASS (`3 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q reflects_two_tools_with_clean_descriptors --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q plan_eager_mode_reflects_synchronously_and_returns_per_server_index --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q lazy_supervisor_registers_reflected_tools_in_dynamic_overlay --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q refresh_server_diffs_added_and_removed_tools --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query reflection_mode_unknown`
  — no results.
- `rg 'reflection_mode_unknown|fallback = Self::Lazy|warn-and-fallback|legacy blocking|Legacy / benchmark|silently running the wrong mode'
  src/daemon/ability/builtins/integrations/mcp/reflective_registry.rs
  src/daemon/ability/catalog/build.rs` — no results.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS after
  applying rustfmt to the edited test assertion.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/docker-media-bidi-e2e.sh --self-test` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `git diff --check` — PASS.
- `rg 'fallback_transport|FallbackReady|fallback_ready|fallback_transports|fallback_transport_ready|with_invoke_bidi_fallback|transport_adapter\.fallback|\.fallback_transport\(' src plugins tools/scripts sdk ...`
  — no matches.
- `codegraph query fallback_transport` — no results after `codegraph sync`.

## 2026-07-20 PrincipalLifecycle admission convergence

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test same_realm_principal_lifecycle_key_resolves_local_miss_without_dial --lib` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test federated_key_resolver --lib` — PASS (`16 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test lifecycle_reader_resolves_only_active_unexpired_public_keys --lib` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test principal_lifecycle --lib` — PASS (`13 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test caller_role_resolves_active_same_realm_user_from_principal_lifecycle --lib` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test admission_facade --lib` — PASS (`8 passed`).
- `bash tools/scripts/backend-live-http-daemon-e2e.sh` — PASS. This reproduces
  the original product failure and proves browser HTTP → backend →
  PrincipalLifecycle registration → live daemon signed `meta.list_abilities`
  no longer fails with `CALLER_KEY_NOT_FOUND` or product-policy
  `CALLER_UNKNOWN`.
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --write` followed
  by `--check` — PASS.
- `PATH=.../dependencies/node/bin:$PATH bash tools/scripts/check-sdk-conformance-reports.sh`
  — PASS.
- `bash tools/scripts/check-sdk-cutover-readiness.sh` — FAIL only on SDK live
  parity result files missing for all seven languages; runtime/product gates
  listed above passed, including the browser HTTP live daemon E2E inside the
  standalone Hub PrincipalLifecycle gate.

## 2026-07-20 SDK conformance snapshot toolchain convergence

- Manual source-snapshot reproduction of
  `SDK_CONFORMANCE_LANGUAGES=go bash tools/scripts/check-sdk-conformance-reports.sh`
  before the fix — FAIL:
  - `source-attestation.json` was written.
  - `go.json` was missing.
  - Preserved runner output showed the only failed case was
    `sdk/seven_language_capability_matrix`.
  - Direct selector replay in the snapshot failed with
    `sdk-conformance: no Python interpreter satisfies required modules: pytest`.
- `PATH="/Users/macbook.silan.tech/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$PATH" bash tools/scripts/check-sdk-conformance-reports.sh --self-test`
  — PASS.
- `SDK_CONFORMANCE_RESULT_DIR="$(mktemp -d target/sdk-live-results-go-fixed.XXXXXX)" SDK_CONFORMANCE_LANGUAGES=go SDK_CONFORMANCE_REPORT_TIMEOUT_SECONDS=300 bash tools/scripts/check-sdk-conformance-reports.sh`
  with the same toolchain `PATH` — PASS:
  - emitted `go.json`
  - emitted `source-attestation.json`
- `EASYNET_SDK_PARITY_RESULTS_DIR=<go-result-dir> EASYNET_SDK_PARITY_LANGUAGES=go EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
  — PASS.
- `codegraph callers runRepositoryGate` — PASS evidence:
  `TestConformanceSevenLanguageCapabilityMatrix` is one of four Go
  repository-gate wrapper callers.
- `codegraph node TestConformanceSevenLanguageCapabilityMatrix` — PASS
  evidence: the selector delegates to
  `check-sdk-parity-matrix.sh --self-test`.

## 2026-07-20 SDK conformance toolchain attestation convergence

- Full-language `SDK_CONFORMANCE_RESULT_DIR=<tmp> bash tools/scripts/check-sdk-conformance-reports.sh`
  before the toolchain resolver fix — PASS for report generation, emitted all
  seven language result files.
- `EASYNET_SDK_PARITY_RESULTS_DIR=<tmp> EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
  before the toolchain resolver fix — FAIL:
  `toolchain_attestation_mismatch:node:ability/descriptor_projection`.
- `SDK_CONFORMANCE_LANGUAGES=node SDK_CONFORMANCE_RESULT_DIR=<tmp> bash tools/scripts/check-sdk-conformance-reports.sh`
  after the fix — PASS. The emitted `node.json` recorded:
  - `toolchain_version = v22.16.0`
  - `toolchain_sha256 = 9b39296a4f4b1abd947c3a77638efe639ad046d76e9e055e29e6fef4e788bcf5`
- `EASYNET_SDK_PARITY_RESULTS_DIR=<node-result-dir> EASYNET_SDK_PARITY_LANGUAGES=node EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
  — PASS.
- Full-language `SDK_CONFORMANCE_RESULT_DIR=<tmp> bash tools/scripts/check-sdk-conformance-reports.sh`
  after the fix — PASS. Emitted:
  `rust.json`, `c_abi.json`, `go.json`, `python.json`, `node.json`,
  `java.json`, `swift.json`, and `source-attestation.json`.
- Full-language `EASYNET_SDK_PARITY_RESULTS_DIR=<tmp> EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
  after the fix — PASS.

## 2026-07-20 Remote desktop diagnostic transport projection convergence

- `codegraph query legacy_label --limit 40` — no results after
  `codegraph sync`.
- `codegraph query diagnostic_fallback --limit 40` — no results after
  `codegraph sync`.
- `codegraph query xcap_compatible_screen_entry --limit 20` — no results after
  `codegraph sync`.
- `codegraph query native_compatible_screen_entry --limit 20` — no results
  after `codegraph sync`.
- `rg 'legacy_label|diagnostic_fallback|Diagnostic fallback|diagnostic fallback|diagnostic-only fallback|diagnostic fallbacks|marked fallback|xcap_snapshot_fallback|legacy registry compatibility|xcap_compatible_screen_entry|native_compatible_screen_entry' plugins/remote-desktop`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q plugins::remote_desktop --lib`
  — PASS (`82 passed`).

## 2026-07-20 Owner projection cursor pre-v2 migration removal

- `codegraph query migrate_legacy_schema_unlocked --limit 20` — no results
  after `codegraph sync`.
- `codegraph query LegacyOwnerProjectionCursor --limit 20` — no results after
  `codegraph sync`.
- `codegraph query load_and_migrate_unlocked --limit 20` — no results after
  `codegraph sync`.
- `rg 'load_and_migrate_unlocked|migrate_legacy_schema_unlocked|LegacyOwnerProjection|legacy_store_is_migrated' src/daemon/persistence/owner_projections.rs src/daemon/federation/read_model/owner_projection.rs`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q owner_projections --lib`
  — PASS (`5 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q owner_projection --lib`
  — PASS (`39 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `git diff --check` — PASS.

## 2026-07-20 EAL agent registry bare-key fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q validate_agent_target --lib`
  — PASS (`2 passed`).
- The focused tests prove both sides of the cutover:
  `validate_agent_target_rejects_bare_default_registry_key` rejects a retired
  `claude` registry row for parsed target `default/claude`, and
  `validate_agent_target_uses_canonical_registry_key_only` proves
  `default/claude` is the row selector that reaches ability-manifest lookup.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "registry.agents.get(&agent_id.name)" --limit 20`
  — no results.
- `rg 'Backwards-compat: registry files written|registry\.agents\.get\(&agent_id\.name\)|bare name|bare-name|default/claude' src/eal/interpreter/dispatch.rs`
  — no production fallback remains; only the focused `default/claude` test
  assertions match.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Agent run-store workspace-dir helper removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q run_dir_create_uses_supplied_agent_root --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q run_store --lib` —
  PASS (`3 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q dispatch_emits_admitted_and_failed_events_when_adapter_fails --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query workspace_dir --limit 30`
  — no results.
- `rg 'workspace_dir\(|RunDir::create\(agent_name|Legacy path helper retained|agents_root\(\)\.join\(agent_name\)' src/daemon/execution/mission`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Local daemon identity fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q local_daemon_ura --lib`
  — PASS (`2 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q local_daemon_default_callee_ura --lib`
  — PASS (`0 matched`, compile passed).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q invoke_federation_discover --lib`
  — PASS (`0 matched`, compile passed).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "local_daemon_ura_uses_control_discovery_identity_before_device_fallback" --limit 20`
  — no results.
- `rg 'local_daemon_ura_uses_control_discovery_identity_before_device_fallback|device fallback|local_daemon_ura\(\)' ...`
  — only the `Result` API, propagated `?` call sites, and focused tests remain.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt` — PASS after rustfmt
  compacted the updated teach call site.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Files resource producer-fact and selector fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q files_store::handlers --lib`
  — PASS (`11 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q real_device_openai_files_upload_retrieve_delete_round_trip --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.local/bin/codegraph init .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph impact handle_put` —
  impact remains limited to `files_store::register`, local handler tests, and
  the OpenAI files real-invoke round-trip.
- `/Users/macbook.silan.tech/.local/bin/codegraph impact read_metadata` —
  impact remains inside files_store handler/register flows and focused tests;
  same-name Pages `handle_get` appears as a codegraph symbol-name collision,
  while `rg` confirms Pages still owns a separate `page.get/fetch` path API.
- `rg 'mime_from_filename|sniff_content_type|content_type\?|page.fetch-shape compat|provide one of \{sha256, ura, path\}|\{ path: "<sha256>" \}' src/daemon/ability/builtins/resources/files_store tools/scripts tests/scripts`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Federation discover user-scope cutover

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q invoke_discover_with_user_id --lib`
  — PASS (`3 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q invoke_discover_without_user_id_does_not_filter --lib`
  — PASS (`1 passed`). This preserves the explicit operator/audit path while
  keeping it named and isolated.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph callers invoke_federation_discover_for_operator_audit`
  — PASS; only `read_federated_directory_for_operator_audit` calls it.
- `/Users/macbook.silan.tech/.local/bin/codegraph callers read_federated_directory_for_operator_audit`
  — PASS; only `easynet federation discover` calls it.
- `/Users/macbook.silan.tech/.local/bin/codegraph callers read_federated_directory_for_current_user`
  — PASS; product/status/doctor/remote-device reads use current-user scoped
  discovery.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 AgentSpec implicit schema removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q core::agent::spec --lib`
  — PASS (`22 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q agent_registry --lib`
  — PASS (`18 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q mission::directory --lib`
  — PASS (`22 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q cli::commands::agent --lib`
  — PASS (`70 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q ability_management::publish --lib`
  — PASS (`9 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q resources::skills::publish --lib`
  — PASS (`12 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q agents::discover --lib`
  — PASS (`31 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q agents::invoke --lib`
  — PASS (`30 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q build_agent_ability_handler_routes_shell_exec_manifest_through_shell_executor --lib`
  — PASS (`1 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q --lib` — PASS
  (`4151 passed`, `3 ignored`).

## 2026-07-20 Plugin status daemon-authority cutover

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q plugin_control --lib`
  — PASS (`4 passed`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query require_plugin_control_value`
  — PASS; the plugin command module now has one daemon-required status helper.
- `/Users/macbook.silan.tech/.local/bin/codegraph query offline_plugin_surface_report`
  — PASS; no results after helper deletion.
- `rg 'offline_plugin_surface_report|select_companion_status|offline planned plugin status|showing local manager observation|PluginLoadPlanner|PluginSurfaceProjector' src/cli/commands/groups/plugin.rs`
  — PASS; no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Ability catalogue fulfilled_by classifier removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q abilities --lib` —
  PASS (`121 passed`; includes
  `extract_columns_ignores_fulfilled_by_as_kind_classifier`).
- `rg -n 'fulfilled_by.*wins|fulfilled_by.*override|entry\s*\.get\("fulfilled_by"\)|legacy `fulfilled_by`|handler tag.*override|handler implementation hints.*cannot override' src/cli/commands/abilities.rs`
  — no production classifier override matches.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query extract_columns` —
  reports the production projection function and the focused regression test.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Device list directory projection fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q devices --lib` — PASS
  (`23 passed`; includes missing `node_id`, missing `status`, unknown status,
  and Device URA/node-id mismatch failures).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query project_directory_entry`
  — reports the fallible `anyhow::Result<Value>` projection.
- `/Users/macbook.silan.tech/.local/bin/codegraph query directory_device_state`
  — reports the explicit status transition helper.
- `rg -n 'unwrap_or\(""\)|unwrap_or\("active"\)|state: "UNKNOWN"|unsupported status|omitted string|project_directory_entry' src/cli/commands/devices.rs`
  — no critical directory projection default remains; remaining empty-string
  display defaults are optional platform fields, and the `unsupported
  status`/`omitted string` matches are fail-closed errors/tests.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 PrincipalLifecycle anonymous external key rejection

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q principal_signing_key --lib`
  — PASS (`5 passed`; includes anonymous and blank external key-id
  rejections).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query resolve_principal_signing_key`
  — reports the production resolver and focused tests.
- `rg -n 'source\.key_id\.unwrap_or_default|key_id: source\.key_id' src/cli/commands/groups/principal.rs`
  — no anonymous key-id projection remains.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 PrincipalLifecycle default proof reference removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q proof_ref --lib` —
  PASS (`3 passed`; includes explicit trim and blank proof-ref rejection).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query default_proof_ref` —
  no results.
- `/Users/macbook.silan.tech/.local/bin/codegraph query required_proof_ref` —
  reports the production validator and focused tests.
- `rg -n "default_proof_ref|bootstrap_proof_ref|proof:<|proof_ref.*unwrap_or_else|pub proof_ref: Option" src/cli/commands/groups/principal.rs`
  — no matches.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Context clipboard history fallback removal

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q context_store --lib` —
  PASS (`8 passed`; includes missing clipboard log as the first-run empty
  history state and malformed JSONL rejection for list/summaries/remove).
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R69, which requires fallible clipboard history loading/projection and rejects
  empty/skip fallbacks in the read model.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for malformed clipboard JSONL being skipped with
  `filter_map(... .ok())`/empty fallback.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query read_clipboard_log --limit 30`
  — PASS; reports `read_clipboard_log() ->
  anyhow::Result<Option<String>>`.
- `/Users/macbook.silan.tech/.local/bin/codegraph query parse_clipboard_log --limit 30`
  — PASS; reports `parse_clipboard_log(content: &str) ->
  anyhow::Result<Vec<ClipEntry>>`.

## 2026-07-20 Node authority binding preflight removal

- `/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node --test --test-reporter=spec test/runtime-core.test.mjs test/conformance-cases.test.mjs`
  from `sdk/node` — PASS (`30 passed`; includes session authority subject
  mismatch rejected at SDK build time and user-owned resource subject admitted).
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R70, which requires Node `InvocationDraft` authority-binding preflight and
  the canonical `AUTHORITY_SUBJECT_MISMATCH` error code.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for Node shape-only authority metadata validation.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query InvocationAuthorityBindingValidator --limit 30`
  — PASS; reports the cohesive Node Runtime Core authority-binding validator
  object and its delegation/session validation methods.
- `/Users/macbook.silan.tech/.local/bin/codegraph query validateInvocationAuthorityBinding --limit 30`
  — PASS; reports the thin `InvocationDraft` entrypoint into the validator.
- `/Users/macbook.silan.tech/.local/bin/codegraph query sessionAuthorityAdmitsSubject --limit 30`
  — PASS; reports the Node session subject-admission helper.
- `/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node --test --test-reporter=spec`
  from `sdk/node` — known pre-existing non-slice failure in
  `test/types.test.ts` importing removed product-neutrality symbol
  `AdminClient`; runtime/conformance `.mjs` tests pass.

## 2026-07-20 Node product-neutral type test repair

- `/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node --test --test-reporter=spec`
  from `sdk/node` — PASS (`35 passed`; includes `test/types.test.ts`).
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R71, which rejects product-symbol runtime imports and opaque authority
  placeholders in the Node type test.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `import { AdminClient }` and `opaque-authority`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query productSymbols --limit 30`
  — PASS; reports product-neutrality assertion lists in runtime and type tests.
- `/Users/macbook.silan.tech/.local/bin/codegraph query RuntimeTransport --limit 20`
  — PASS; reports the generic Node `RuntimeTransport` seam.
- `rg -n "import \\{[^\\n]*(AdminClient|CompanionClient|CompatibilityClient|DirectoryClient|MissionClient|ReceiptClient|SurfaceClient)|void (AdminClient|CompanionClient|CompatibilityClient|DirectoryClient|MissionClient|ReceiptClient|SurfaceClient)|opaque-authority" sdk/node/test/types.test.ts sdk/node/index.js sdk/node/index.d.ts`
  — PASS; no product runtime import or opaque authority placeholder remains.

## 2026-07-20 Authorized history subject-binding gate

- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo fmt --check` —
  PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  descriptor_resolution_errors_project_canonical_runtime_codes --features
  axon-pb` — PASS; includes signer-missing descriptor resolution now returning
  `ERR_PERMISSION_DENIED` with canonical `CALLER_SIGNER_UNAVAILABLE`.
- `PATH="/opt/homebrew/bin:$PATH" go test . -run
  'TestAuthorizedRuntimeSessionHistoryRejectsAuthoritySubjectMismatchBeforeReceiptProvider|TestAuthorizedRuntimeSessionRejectsAuthoritySubjectMismatchBeforeDispatch'`
  from `sdk/go` — PASS; proves Go history subject mismatch is rejected before
  the receipt provider is called.
- `./.venv/bin/python -m unittest tests/test_authorized_runtime_session.py`
  from `sdk/python` — PASS (`5 tests`); proves Python history subject
  mismatch is rejected before the receipt provider is called.
- `./sdk/python/.venv/bin/python -m compileall -q
  sdk/python/easynet_sdk/authorized_runtime_session.py
  sdk/python/tests/test_authorized_runtime_session.py` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; `codegraph
  status .` reports the index is up to date.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  validateSessionHistoryRuntimeCall --limit 20` — PASS; reports the Go SDK
  gate.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  _validate_session_history_call --limit 20` — PASS; reports the Python SDK
  gate.
- `tools/scripts/check-architecture-convergence.sh` — PASS.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.

## 2026-07-20 Federation resolve-key trust material gate

- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  resolve_key_response --lib` — PASS (`2 passed`); covers invalid base64 and
  non-32-byte public key rejection.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  handle_resolve_key --lib` — PASS (`4 passed`); preserves hit/miss and
  presented-key pinning semantics under the new `Result<Option<_>>` state.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  invoke_dispatches_federation_resolve_key --lib` — PASS (`3 passed`);
  confirms dispatcher behavior after converting corrupt key material into
  `FailedPrecondition`.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo fmt --check` —
  PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph callers
  resolve_key_response --limit 20` — PASS; reports the resolver handler and
  failure-path tests, while `rg` confirms the previous decode
  `unwrap_or_default()` path is gone from this resolver response builder.
- `tools/scripts/check-architecture-convergence.sh` — PASS.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `git diff --check` — PASS.

## 2026-07-20 Plugin sidecar stderr diagnostic gate

- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  sidecar_stderr --lib` — PASS (`3 passed`); covers binary stderr
  preservation, read failure diagnostics, and reader panic diagnostics.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q sidecar --lib`
  — PASS (`17 passed`); confirms unary, stream, and bidi sidecar contracts
  still hold after the shared diagnostic capture refactor.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo fmt --check` —
  PASS.
- `tools/scripts/check-architecture-convergence.sh` — PASS.
- `tests/scripts/test_check_architecture_convergence.sh` — PASS; includes the
  `R72_PLUGIN_SIDECAR_STDERR_DIAGNOSTICS` negative fixture for the previous
  `read_to_string` / `unwrap_or_default()` fallback.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo check --lib --bins`
  — PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  capture_stderr_diagnostics --limit 30` — PASS; reports the single sidecar
  stderr diagnostic helper.

## 2026-07-20 Trust key resolver corrupt user-key gate

- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  key_resolver --lib` — PASS (`23 passed`); includes
  `resolve_all_rejects_corrupt_user_key_instead_of_skipping_it`.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo fmt --check` —
  PASS.
- `tools/scripts/check-architecture-convergence.sh` — PASS.
- `tests/scripts/test_check_architecture_convergence.sh` — PASS; includes a
  negative fixture for the previous
  `filter_map(|row| decode_pubkey(...).ok())` user-key skip fallback.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo check --lib --bins`
  — PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query resolve_all --limit
  30` — PASS; reports `RealmTrustAnchorKeyResolver::resolve_all`.
- `rg -n "filter_map\\([^\\n]*decode_pubkey|decode_pubkey\\(&row\\.public_key_b64, agent_ura\\)\\.ok\\(|decode_pubkey\\(&row\\.public_key_b64, agent_ura\\)\\?" ...`
  — PASS; production resolver uses `decode_pubkey(...)?`, and the old skip
  pattern remains only in the architecture-gate negative fixture.

## 2026-07-20 Device trust sync resolve-key schema gate

- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  device_trust_sync --lib` — PASS (`11 passed`); includes missing
  `public_keys_b64` rejection, malformed row rejection, and explicit empty
  array hub-miss handling.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo fmt --check` —
  PASS.
- `tools/scripts/check-architecture-convergence.sh` — PASS.
- `tests/scripts/test_check_architecture_convergence.sh` — PASS; includes a
  negative fixture for the previous legacy `public_key_b64` repair and
  malformed-row `filter_map` skip.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo check --lib --bins`
  — PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  parse_public_keys_b64_field --limit 30` — PASS; reports the schema-bound
  parser helper.
- `rg -n "parse_resolved_caller_trust|public_key_b64|filter_map|unwrap_or_default\\(\\)|parse_public_keys_b64_field" ...`
  — PASS; production parser delegates to `parse_public_keys_b64_field`, and
  the legacy fallback pattern remains only in tests/gate negative fixtures.

## 2026-07-20 Pages serve fetch projection schema gate

- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test -q
  pages_serve_ability --lib` — PASS (`5 passed`); covers valid projection,
  missing `bytes_b64`, invalid base64, sha mismatch, and non-boolean
  `force_attachment`.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo fmt --check` —
  PASS.
- `tools/scripts/check-architecture-convergence.sh` — PASS.
- `tests/scripts/test_check_architecture_convergence.sh` — PASS; includes a
  negative fixture for the previous Pages fetch projection defaults.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo check --lib --bins`
  — PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query bytes_from_value
  --limit 30` — PASS; reports the fallible Pages fetch projection parser.
- `rg -n "bytes_from_value|bytes_b64|unwrap_or_default\\(\\)|unwrap_or\\(\\\"application/octet-stream\\\"\\)|sha256 != actual_sha256|required_non_empty_string" ...`
  — PASS; production parser requires fields and verifies sha256, while the old
  defaulting pattern remains only in the architecture-gate negative fixture.

## 2026-07-20 Ability catalogue authority-context fallback removal

- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo fmt --check` —
  PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo check --locked --lib
  --bins` — PASS; using `--locked` proved the change does not depend on
  dependency/version drift.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" cargo test
  catalog::assembly_tests --lib` — PASS (`38 passed`).
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R80 for concrete catalogue authority context.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `Option<AbilityAuthorityContext>`,
  `authority_context.unwrap_or_default()`, and `authority_context: None`.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query RegistryBuildConfig
  --limit 30` and `codegraph query RegistryDaemonBuildConfig --limit 20` —
  PASS; reports concrete build config structs and constructors.

## 2026-07-20 Ability publication projection fail-closed gate

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q owner_projection_values
  --lib` — PASS (`1 passed`); includes corrupt committed descriptor rejection.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q handle_resolve --lib` —
  PASS (`10 passed`); covers live local publication, hosted-agent projection,
  no-fabrication, prefix filtering, and expired projection behavior under the
  new `Result` resolver surface.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q route_resolver --lib` —
  PASS (`40 passed`); proves namespace/route resolver callers continue to
  resolve valid projections while retaining explicit negative states.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R81 for ability publication projection fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for the previous `filter_map(... .ok())` local
  publication path and silent `summary_from_value(...).and_then(...)` merge
  path.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  "owner_projection_values resolved_owner_projection_values fail closed ability
  publication" --limit 80` — PASS; reports both production functions with
  `Result` return types.
- `git diff --check` — PASS.

## 2026-07-20 Desktop companion status projection errors

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  json_payload_exposes_companion --lib` — PASS (`2 passed`); covers successful
  companion DTO projection and explicit `desktop_companion_errors` JSON output.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  plugin_host_surface_projects_desktop_companion_as_package_only --lib` —
  PASS (`1 passed`); proves normal desktop companion package rows still expose
  companion DTOs without `companion_error`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  activate_realtime_projects_ready_resource_and_permission_actions --lib` —
  PASS (`1 passed`); covers plugin integration fixture migration for the
  expanded package surface record.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R82 for desktop companion projection-error preservation.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `desktop_companion_statuses() -> Vec<Value>`,
  `return Vec::new()`, `status_json(package).ok()`, and `.ok()` projection
  cause erasure inside `DesktopCompanionManager::status_json`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  "DesktopCompanionStatusObservation desktop_companion_errors companion_error
  status_json" --limit 80` — PASS; reports the companion manager projector and
  runtime JSON surface.
- `git diff --check` — PASS.

## 2026-07-20 Curator owner catalog fail-closed acquisition

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  collect_owner_catalog --lib` — PASS (`2 passed`); covers first-run missing
  registry as empty catalog and corrupt `agents.json` as unavailable owner
  catalog state.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  curator_attempts_publish_when_verdict_qualifies --lib` — PASS (`1 passed`);
  proves the curator/publish loop still runs with a valid isolated registry.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R83 for curator catalog fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for the previous best-effort `Err(_) => return
  Vec::new()` collector and missing `stage = "catalog"` curator outcome.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  "collect_owner_catalog owner ability catalog unavailable curator catalog"
  --limit 80` — PASS; reports `collect_owner_catalog(owner: &str) ->
  Result<Vec<CatalogEntry>, String>` and the new regression tests.
- `git diff --check` — PASS.

## 2026-07-20 Schedule due selection fail-closed runtime state

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q due_ --lib` — PASS
  (`12 passed`); covers existing misfire semantics plus corrupt cached cron
  rejection and poisoned cache rejection.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R84 for schedule due-selection fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for the previous poisoned-cache empty list and invalid
  cron row skip.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "ScheduleService due
  schedule tick due selection failed invalid cron" --limit 80` — PASS; reports
  `ScheduleService::due(...) -> anyhow::Result<Vec<DueFire>>`,
  `spawn_schedule_tick`, and the new corrupt-cron/poisoned-cache regression
  tests.
- `git diff --check` — PASS.

## 2026-07-20 Schedule snapshot fail-closed read model

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q schedule --lib` — PASS
  (`32 passed`); includes the new poisoned-cache schedule list regression plus
  schedule ability/context loader coverage.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after extending
  R84 to schedule snapshot/list semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for the previous `ScheduleService::list() -> Vec<_>`,
  poisoned-cache empty list, `schedule.list` error erasure, and `null` row
  serialization fallback.
- `CARGO_TARGET_DIR=/tmp/easynet-codex-check-target
  /Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS. The separate target dir avoided unrelated concurrent cargo build locks.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "ScheduleService list
  schedule snapshot failed schedule list cache lock poisoned" --limit 80` —
  PASS; reports `ScheduleService::list(...) ->
  anyhow::Result<Vec<ScheduleEntry>>`, `list_handler`, and
  `list_rejects_poisoned_cache_instead_of_empty_schedule_read_model`.

## 2026-07-20 Schedule context next-fire fail-closed projection

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q schedule --lib` — PASS
  (`33 passed`); includes
  `loader_rejects_corrupt_schedule_instead_of_empty_context`.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after
  extending R84 to `ScheduleLoader::load` and shared schedule-entry cron
  validation.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `next_fire_after(&entry.id, now)` plus
  `Ok(None) | Err(_) => continue` in the context loader.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "ScheduleLoader
  next_fire_for_entry corrupt schedule invalid cron empty context" --limit 80`
  — PASS; reports `ScheduleLoader`,
  `ScheduleService::next_fire_for_entry`, `parse_entry_cron`, and the new
  loader regression.

## 2026-07-20 Session index fail-closed read model

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q session --lib` — PASS
  (`254 passed`); includes `SessionService` poison regressions and
  device.session list/attach poison propagation tests.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R85 for session-index fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `list_active() -> Vec<_>`, `Err(_) => Vec::new()`,
  `.read().ok()` lookup erasure, device.session `null` row fallback, and
  attach empty-snapshot fallback on index failure.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "SessionService
  list_active get session index lock poisoned device session list attach empty
  snapshot" --limit 100` — PASS; reports
  `SessionService::list_active(...) -> anyhow::Result<Vec<Session>>`.

## 2026-07-20 Discuss room registry fail-closed read model

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q discuss --lib` — PASS
  (`21 passed`); includes
  `list_rejects_poisoned_room_registry_instead_of_empty_rooms`.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R86 for discuss room-registry fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `DiscussService::list() -> Vec<_>`,
  `Err(_) => Vec::new()`, and Kernel `Ok((*self.discuss).list())` projection.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "DiscussService list
  room registry lock poisoned list_discuss_rooms" --limit 80` — PASS; reports
  `DiscussService::list(...) -> anyhow::Result<Vec<DiscussRoom>>`,
  `Kernel::list_discuss_rooms`, and the new poison regression.
- `git diff --check` — PASS.

## 2026-07-20 Loop cache fail-closed read model

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q loop --lib` — PASS
  (`50 passed`); includes poisoned-cache regressions for `status`, `list`,
  `subscribe`, `loop.status`, and `loop.subscribe`.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R87 for loop-cache fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `LoopService::status() -> Option<_>`,
  `.read().ok()` lookup erasure, `LoopService::list() -> Vec<_>`,
  `Err(_) => Vec::new()`, Kernel `Ok(self.loop_svc.status(id))`, and
  `loop.status` failure erasure.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "LoopService status
  list cache lock poisoned loop_status status_handler subscribe" --limit 100`
  — PASS; reports `LoopService::status(...) ->
  anyhow::Result<Option<LoopInstance>>`, `LoopService::list(...) ->
  anyhow::Result<Vec<LoopInstance>>`, `Kernel::loop_status`, and the new poison
  regressions.
- `git diff --check` — PASS.

## 2026-07-20 Chat cross-agent registry fail-closed discovery

- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q agents::chat --lib` —
  PASS (`61 passed`); includes corrupt-registry regressions for direct
  cross-agent enumeration, RPC chat, and stream chat.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R88 for chat cross-agent registry fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `Err(_) => return Vec::new()` and non-propagating RPC
  / stream chat call sites.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --locked --lib --bins` —
  PASS.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  "enumerate_other_agent_specs cross-agent ability registry projection
  invoke_direct_with_progress stream_handler" --limit 100` — PASS; reports
  `enumerate_other_agent_specs(...) -> anyhow::Result<Vec<...>>`,
  `invoke_direct_with_progress`, `stream_handler`, and the new corrupt-registry
  regressions.
- `git diff --check` — PASS.

## 2026-07-21 Permission pending queue fail-closed read model

- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R89 for permission pending queue fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture for `SubscriberBroker::pending_snapshot() -> Vec<_>`,
  `.read().ok()`, `unwrap_or_default()`, non-result
  `PermissionService::pending`, consent `unwrap_or(Value::Null)`, and Kernel
  `Ok(self.permission.pending())` projection.
- `/Users/macbook.silan.tech/.cargo/bin/rustfmt --edition 2021 --check
  src/daemon/execution/permission/mod.rs
  src/daemon/ability/builtins/governance/consent.rs
  src/daemon/boot/kernel/mod.rs` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q governance::consent
  --lib` — PASS (`7 passed`); includes poisoned pending queue regressions for
  `consent.list_pending` and `consent.subscribe`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q permission --lib` —
  PASS (`45 passed`); includes poisoned pending queue regressions for
  `SubscriberBroker` and `PermissionService`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "SubscriberBroker
  pending_snapshot pending queue lock poisoned consent list_pending subscribe
  PermissionService pending" --limit 80` — PASS; reports
  `SubscriberBroker::pending_snapshot(...) ->
  anyhow::Result<Vec<PermissionRequest>>`,
  `PermissionService::pending(...) -> anyhow::Result<Vec<PermissionRequest>>`,
  and the poisoned queue regressions.
- `git diff --check` — PASS.

## 2026-07-21 Authority metadata all-zero principal fail-closed validation

- `/Users/macbook.silan.tech/.cargo/bin/rustfmt --edition 2021 --check
  src/daemon/invocation/admission/authority_metadata.rs` — PASS after
  formatting.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  invocation::admission::authority_metadata --lib` — PASS (`8 passed`);
  includes all-zero delegation and session-authority regressions.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R90 for daemon-side all-zero principal rejection.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture where authority validators only require non-empty fields
  and never call the all-zero rejection helper.
- `PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query "authority metadata
  all-zero principal session_owner_user_id reject_all_zero_authority_fields
  validate_session_authority_payload_shape" --limit 80` — PASS; reports
  `reject_all_zero_authority_fields`,
  `validate_session_authority_payload_shape`,
  `ALL_ZERO_PRINCIPAL_ID`, and the new all-zero authority regressions.
- `git diff --check` — PASS.

## 2026-07-21 C ABI descriptor diagnostics fallback typing

- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  resolveDescriptorRefFromDiagnostics _resolve_descriptor_ref_from_diagnostics
  descriptor_resolution_error_projection` — PASS; reports Go/Python fallback
  functions, Rust FFI typed projection, and the Go resolver test dependency.
- `(cd sdk/go && PATH="/opt/homebrew/bin:$PATH" go test -tags
  'easynet_cabi' . -run 'TestResolveDescriptorRefFromDiagnostics')` — PASS.
- `PYTHONPATH=sdk/python:sdk/python/tests sdk/python/.venv/bin/python -m
  pytest -q sdk/python/tests/test_cabi.py -k 'descriptor_diagnostics'` —
  PASS (`3 passed, 24 deselected`).
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R91 for descriptor fallback typed not-found semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture where Go/Python diagnostics fallback returns generic
  `NOT_FOUND`.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python"
  sdk/python/.venv/bin/python sdk/conformance/rebuild_public_api_model.py
  --write` — PASS; refreshed Go/Python C ABI provider hashes only.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-21 Invocation attempt audit fail-closed history diagnostics

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  InvocationAttemptLedger InvocationAttemptHandle invocation.history.list
  missing_invocation_attempt_ledger invocation_attempt_audit_status` — PASS;
  reports the attempt ledger, service wiring, and history consumer boundary.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q
  invocation::dispatch::attempt_audit --lib` — PASS (`2 passed`); includes
  append/read ordering and corrupt-row fail-closed regression.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q invocation_history --lib` —
  PASS (`27 passed`); includes merged invocation/attempt diagnostic rows.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q daemon_invocation_service
  --lib` — PASS (`120 passed`, `3 ignored`); verifies direct service
  construction and all three Invocation RPC shells remain wired with the
  canonical attempt ledger.
- `PATH="$HOME/.cargo/bin:$PATH" cargo check --lib` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R92 for invocation attempt audit fail-closed semantics.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture with disabled handle, boot-disabled continuation, silent
  append failure, and corrupt-row skipping.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-21 Session prelude resolve_key schema and paired user key pinning

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  sync_paired_user_trust_prelude paired_user_resolve_key_args
  publish_paired_user_keys_prelude resolved_public_keys federation.resolve_key
  presented_pubkey_b64` — PASS; reports the paired-user trust prelude,
  resolve-key request builder, key publication prelude, and the two
  `resolved_public_keys` consumers.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q
  session_initiator::prelude --lib` — PASS (`6 passed`); covers canonical
  `public_keys_b64[]`, legacy single-key rejection, malformed-row rejection,
  and paired-user resolve arguments carrying `presented_pubkey_b64`.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q
  paired_user_trust_resolve_pins_presented_pubkey --lib` — PASS (`1 passed`);
  verifies session open invokes `federation.resolve_key` with the locally
  published public key after paired-user key publication.
- `PATH="$HOME/.cargo/bin:$PATH" cargo check --lib` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R93 for session prelude resolve-key schema binding and paired-user proof
  pinning.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture where `resolved_public_keys` repairs legacy
  `public_key_b64`, skips malformed rows, defaults invalid JSON to an empty
  key set, and paired-user resolve omits `presented_pubkey_b64`.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-21 FFI descriptor catalog provider payload fail-closed ingestion

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "descriptor_ref not found owner is not online browser.open_session
  invocation.history.list AUTHORITY_SUBJECT_MISMATCH
  target_owned_descriptor_catalog_subject_ura"` — PASS; reports the FFI
  descriptor resolver, remote `meta.list_abilities` probe, system descriptor
  catalog, and descriptor binding helpers.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q
  descriptor_catalog_ingestion_rejects_malformed_provider_rows --lib` — PASS
  (`1 passed`); covers malformed provider descriptor hashes and missing
  required provider fields.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q runtime_descriptor_resolver
  --lib` — PASS (`4 passed`); verifies local descriptor resolution,
  explicit call mode, no remote probe for local catalog miss, and remote
  system ability descriptor synthesis.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R94 for fail-closed FFI descriptor catalog ingestion.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture where descriptor catalog rows are skipped with
  `filter_map`, system row rebind/projection failures are dropped, and parser
  functions return `Option`.
- `PATH="$HOME/.cargo/bin:$PATH" cargo check --lib` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-21 FFI descriptor remote probe explicit caller tuple

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "runtime_resolve_descriptor_ref_json caller_ura subject_ura unwrap_or_else
  runtime_owner_ura invocation.history.list authorized runtime session
  descriptor resolve tuple default"` — PASS; reports the FFI descriptor
  resolver, SDK runtime tuple fields, and the prior runtime-owner fallback
  site.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q
  runtime_descriptor_remote_probe_requires_explicit_caller_ura --lib` — PASS
  (`1 passed`); verifies remote descriptor probes fail before daemon IO when
  `caller_ura` is missing.
- `PATH="$HOME/.cargo/bin:$PATH" cargo test -q runtime_descriptor_resolver
  --lib` — PASS (`4 passed`); confirms local and system descriptor resolution
  behavior is preserved.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R95 for explicit remote descriptor probe caller binding.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture where `caller_ura` is synthesized from
  `runtime_owner_ura` through `unwrap_or_else`.
- `PATH="$HOME/.cargo/bin:$PATH" cargo check --lib` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-21 SDK authorized history filter tuple binding

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "invocation.history.list subject_ura authority subject
  session_authority_admits_subject AUTHORITY_SUBJECT_MISMATCH
  RuntimeReceiptQuery authorized_runtime_session history caller_ura
  callee_ura"` — PASS; reports the authorized session history operations,
  receipt providers, and authority subject binding functions.
- `(cd sdk/go && PATH="/opt/homebrew/bin:$PATH" go test . -run
  'TestAuthorizedRuntimeSessionHistory')` — PASS; includes authority subject
  mismatch and filter subject expansion rejection before the receipt provider.
- `PYTHONPATH=sdk/python:sdk/python/tests
  sdk/python/.venv/bin/python -m pytest -q
  sdk/python/tests/test_authorized_runtime_session.py -k 'history'` — PASS
  (`2 passed, 4 deselected`); mirrors the Go history regressions.
- `(cd sdk/go && PATH="/opt/homebrew/bin:$PATH" go fmt ./...)` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS after adding
  R96 for cross-SDK authorized history filter tuple binding.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture where Go/Python history list only validates
  `request.Call` and never checks filter tuple expansion.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python"
  sdk/python/.venv/bin/python sdk/conformance/rebuild_public_api_model.py
  --write` — PASS; no public API inventory drift was produced.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `git diff --check` — PASS.

## 2026-07-21 Device visibility route-gate

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "meta.list_abilities browser.open_session route visible owner is not online
  descriptor_ref stale device"` — PASS; reports the product-facing
  `federation_probe` device visibility helpers and `node.describe` handlers.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS before
  focused tests.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q federation_probe --lib`
  — PASS (`10 passed`); includes regression coverage that probe-failed
  directory devices move to `unavailable_nodes` and that `node.describe` does
  not return stale ability summaries for unrouteable devices.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q describe_node --lib`
  — PASS (`2 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q network_health --lib`
  — PASS (`4 passed`).
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-21 Namespace resolver qtype ingress

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "namespace.resolve qtype query_name ability_name default infer empty string
  route directory fallback"` — PASS; reports public resolver ingress,
  resolver state selection, and proxy forwarding.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q namespace_resolve --lib`
  — PASS (`7 passed`); includes missing-qtype and shorthand-qtype rejection at
  local public ingress.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  invoke_rejects_namespace_proxy_resolve_missing_qtype --lib` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  invoke_dispatches_namespace_proxy_resolve_to_typed_peer_surface --lib` —
  PASS; proves canonical proxy qtype still forwards to peer
  `namespace.resolve`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  "validate_namespace_resolve_query"` — PASS; indexed local ingress validator.
- `/Users/macbook.silan.tech/.local/bin/codegraph query
  "validate_namespace_proxy_resolve_request"` — PASS; indexed proxy ingress
  validator.
