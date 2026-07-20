# Verification

Commands and outcomes will be appended after implementation.

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
