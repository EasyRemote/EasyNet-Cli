# Verification

Commands and outcomes will be appended after implementation.

## 2026-07-22 Runtime-owner signer User custody guard

- `cargo test -q runtime_owner_signing_identity_rejects_user_before_keyring_lookup --lib`
  — PASS; proves User URAs fail before the `SelfIdentity` provider is queried.
- `cargo test -q runtime_caller_signer_resolver_does_not_fall_back_from_user_to_owner_key --lib`
  — PASS; preserves the managed-user resolver behavior that returns
  `managed user signing key not found` rather than legacy runtime-owner keyring
  lookup failures.
- `cargo test -q daemon::identity::self_identity::tests --lib` — PASS (`10`
  tests).
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  a negative fixture where `RuntimeSigningIdentity::load` calls
  `provider.public_key(owner_ura)` before custody classification.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the same runtime-owner signer User
  custody regression.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-22 SDK history authority subject exact binding

- `go test . -run
  'TestAuthorizedRuntimeSessionHistoryRejects(AuthoritySubjectMismatch|OwnerEquivalentSubjectExpansion|FilterSubjectExpansion)'`
  from `sdk/go` — PASS; focused history tests reject device-subject mismatch,
  same-owner different-session subject expansion, and filter subject expansion
  before receipt provider dispatch.
- `go test .` from `sdk/go` — PASS.
- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python3 -m unittest
  sdk.python.tests.test_authorized_runtime_session` — PASS (`7` tests);
  includes same-owner different-session subject expansion rejection before the
  Python receipt provider is called.
- `python3 -m py_compile
  sdk/python/easynet_sdk/authorized_runtime_session.py` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative SDK history authority fixture that reuses
  owner-expanding subject admission and must fail.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; covers
  architecture gate negative fixtures after adding exact history authority
  subject checks.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-21 Ability discovery candidate projection

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  cli::commands::discover::tests --lib` — PASS (`25 passed`); includes
  malformed minted `qualified_name` failure, missing `candidates[]` failure,
  zero-score valid row ranking miss, unminted identity non-callable projection,
  and JSON contract removal of `skipped_unparseable`.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q discover --lib` —
  observed unrelated failure in
  `daemon::ability::catalog::assembly_tests::discovery_hints_read_only_tracks_ability_layer`
  (`hot hosted-Agent registrar is not ready: pending_runtime`). The focused
  CLI discover module above isolates this slice's changed projection boundary.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "discover project_rows Candidate from_ladder_row skipped_unparseable
  malformed qualified_name candidates array"` — PASS; reports the fallible
  `DiscoverExecutionState::extend_candidates -> project_rows ->
  Candidate::from_ladder_row` path and no `skipped_unparseable` field in
  `DiscoverReport`.
- `rg -n "skipped_unparseable|candidate\\(s\\) dropped|&mut skipped|let mut
  skipped|filter_map\\(\\|row\\|" src/cli/commands/discover.rs` — PASS; no
  legacy partial-success/drop counter path remains in the discover surface.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-21 Ability recording resource read-model projection

- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q ability_record --lib`
  — PASS (`11 passed`); includes valid resource row selection, matching row
  missing `resource_ura`, non-Resource URA rejection, and all-returned-row
  validation before selecting the first resource subject.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "ability_record select_default_resource_ura resource_row_ura
  meta.list_resources resource_ura filter_map selected"` — PASS; reports
  `default_resource_ura -> select_default_resource_ura -> resource_row_ura`,
  with `resource_row_ura` owning schema-bound row validation.
- `rg -n "meta\\.list_resources.*filter_map|filter_map\\(\\|entry\\|
  entry\\.get\\(\\\"resource_ura\\\"\\)|resources\\s*\\.iter\\(\\)\\s*\\.filter_map|resource_ura\\\"\\)\\.and_then\\(Value::as_str\\).*find"
  src/cli/commands/ability_record.rs` — PASS; no silent
  `meta.list_resources` resource_ura projection remains in the recording
  surface.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

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

## 2026-07-21 Federation peer trust projection

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "federation_peers realm trust trusted_agent role unwrap_or schema-B
  trusted_hubs"` — PASS; reports `RealmTrustAnchor` as the daemon-owned trust
  aggregate and identifies the federation peer CLI trust projection seam.
- `rg -n
  "role\\\"\\).*unwrap_or|trusted_agent.*unwrap_or|parse_trusted_hubs_from|trusted_hubs_from_anchor|RealmTrustAnchor::load_or_empty"
  src/cli/commands/federation_peers.rs src/cli src/daemon -S` — PASS for the
  selected seam; `federation_peers.rs` now contains
  `RealmTrustAnchor::load_or_empty` and `trusted_hubs_from_anchor`, and no
  `parse_trusted_hubs_from` or `trusted_agent` role defaulting remains. The
  remaining `openai_compat.rs` `role.unwrap_or("")` occurrence is an OpenAI
  chat-message role projection, not realm trust.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q federation_peers --lib`
  — PASS (`10 passed`); includes missing-file empty state, malformed
  daemon-config rejection, missing trust role rejection, malformed hub URA
  rejection, missing hub `agent_ura` rejection, and schema-incomplete hub-row
  rejection before `trusted_hubs` output.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; indexed the
  updated federation peer projection.

## 2026-07-21 Remote desktop consent receipt projection

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "mission session filter_map serde_json from_value ok corrupt rows skipped
  session state"` — PASS; reports `session_consent.rs` receipt extraction and
  access-check relationships.
- `rg -n
  "filter_map\\(RemoteDesktopConsentReceipt::from_value\\)|first_receipt_from_causal_context\\(|causal_context_contains_receipt\\("
  plugins/remote-desktop/src -S` — PASS; no `filter_map` receipt projection
  remains, and creation/access call sites use the fallible parser.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q
  remote_desktop::session_consent --lib --features remote-desktop` — PASS
  (`9 passed`); includes malformed causal-context list row rejection and
  rejection before owner-self-consent fallback.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q show_session --lib
  --features remote-desktop` — PASS (`3 passed`); preserves consent receipt
  binding in the product-visible session projection.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins
  --features remote-desktop` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; indexed the
  updated remote-desktop consent receipt projection.

## 2026-07-21 Invocation history filter schema

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "invocation_history filter subject_uras caller_ura callee_ura
  unwrap_or_default filter_map invalid scope history list"` — PASS; reports
  `invocation_history.rs` filter parsing, attempt matching, and ledger query
  relationships.
- `rg -n
  "filter_map\\(non_empty_str\\)|filter_string_set\\(|string_set_arg\\(|subject_filter_values\\(|attempt_matches_filter\\("
  src/daemon/ability/builtins/governance/invocation_history.rs` — PASS; no
  subject-scope `filter_map(non_empty_str)` remains, and both ledger/attempt
  filters route through fallible helpers.
- `/Users/macbook.silan.tech/.cargo/bin/cargo fmt --check` — PASS.
- `/Users/macbook.silan.tech/.cargo/bin/cargo test -q invocation_history
  --lib` — PASS (`31 passed`); includes malformed subject array rejection,
  malformed scalar filter-field rejection, malformed ability-set rejection,
  and attempt-filter malformed subject-scope rejection.
- `/Users/macbook.silan.tech/.cargo/bin/cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; index was
  already up to date after the invocation-history filter-schema refactor.

## 2026-07-21 Remote desktop create-session ingress schema

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop parse_video_constraints codec_preferences scale_mode
  hardware_acceleration_required input_policy video schema invalid default"`
  — PASS; reports `parse_video_constraints`, `parse_input_policy`, session
  profile consumers, and schema/registration relationships.
- `cargo fmt --check` — PASS after formatting the parser/schema imports.
- `cargo test -q remote_desktop::request --lib --features remote-desktop` —
  PASS (`5 passed`); includes present malformed `video`, malformed scalar
  defaults, malformed TTL/session id, and malformed/mutually-exclusive input
  policy rejection.
- `cargo test -q remote_desktop::handlers::create_session --lib --features
  remote-desktop` — PASS (`3 passed`); preserves subject/consent create-session
  lifecycle while using the fallible parser.
- `cargo test -q remote_desktop::registration --lib --features
  remote-desktop` — PASS (`3 passed`); verifies the plugin registration path
  accepts the expanded nested create-session schema.
- `cargo check --lib --bins --features remote-desktop` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; indexed
  the updated remote-desktop request parser/schema contract.

## 2026-07-21 Remote desktop interactive input frame schema

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop invoke_bidi frame type unwrap_or empty malformed frame
  ignored input media terminal session"` — PASS; identified the diagnostic
  InvokeBidi control-loop `type.unwrap_or("")` seam and the shared input frame
  parser.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop input frame serde default deny_unknown_fields
  bidi_control_frame_type invalid_frame key code clipboard file_drop"` —
  PASS; reports `RemoteDesktopInputFrame`, key/pointer frame DTOs,
  `handle_bidi_input_frame`, and the new control frame type parser.
- `rg -n
  "unwrap_or\\(\\\"\\\"\\)|serde\\(default\\)|filter_map\\(|unknown_frame|invalid_frame|bidi_control_frame_type|key input frame must include"
  plugins/remote-desktop/src/input.rs
  plugins/remote-desktop/src/invoke_bidi.rs -S` — PASS for the selected seam;
  `invoke_bidi.rs` no longer uses empty-string type fallback, and remaining
  `serde(default)` uses are limited to optional pointer coordinates and the
  key/code pair guarded by parser validation.
- `cargo test -q remote_desktop::input --lib --features remote-desktop` —
  PASS (`6 passed`); includes unknown-field, missing key identity, missing
  clipboard text, missing/empty file-drop payload, and blank file-drop path
  rejection.
- `cargo test -q remote_desktop::invoke_bidi --lib --features
  remote-desktop` — PASS (`5 passed`); includes malformed control frame type
  rejection as `invalid_frame` instead of `unknown_frame`.
- `cargo check --lib --bins --features remote-desktop` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; indexed
  the updated remote-desktop input-frame and diagnostic Bidi control parser.

## 2026-07-21 Remote desktop ICE candidate signaling schema

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop session_store unwrap_or_default malformed store json default
  empty corrupt session state"` — PASS; identified local WebRTC candidate
  projection and session store mutation helpers.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "remote_desktop ICE candidate record_local_webrtc_candidate
  remote_ice_candidate_inits malformed candidate unwrap_or_default
  add_ice_candidate"` — PASS; reports `remote_ice_candidate_inits`,
  `record_local_webrtc_candidate`, `add_ice_candidate`, and live WebRTC
  callback relationships.
- `rg -n
  "record_local_webrtc_candidate|remote_ice_candidate_inits|ice_candidate_text|unwrap_or_default\\(\\)|candidate.*and_then\\(Value::as_str\\)"
  plugins/remote-desktop/src/session_store.rs
  plugins/remote-desktop/src/sdp.rs
  plugins/remote-desktop/src/handlers/add_ice_candidate.rs
  plugins/remote-desktop/src/transport/webrtc.rs -S` — PASS for the selected
  seam; local candidate projection no longer uses `unwrap_or_default`, and
  remote/local candidate ingress share the SDP/ICE parser.
- `cargo test -q remote_desktop::sdp --lib --features remote-desktop` —
  PASS (`4 passed`); includes schema-incomplete candidate rejection and
  explicit null/empty end-marker handling.
- `cargo test -q remote_desktop::session_store --lib --features
  remote-desktop` — PASS (`2 passed`); includes malformed local candidate
  rejection before session signaling projection.
- `cargo test -q remote_desktop::handlers::add_ice_candidate --lib
  --features remote-desktop` — PASS (`1 passed`); proves malformed remote
  candidates fail before being stored on the session.
- `cargo check --lib --bins --features remote-desktop` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; indexed
  the updated ICE candidate parser and session projection boundary.

## 2026-07-21 Desktop companion desired-state store schema

- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "companion state_store unwrap_or_default malformed toml json default empty
  corrupt plugin desired state"` — PASS; identified the companion state store
  row parser and desired-state lifecycle consumers.
- `/Users/macbook.silan.tech/.local/bin/codegraph explore
  "companion state_store desired_state serde default duplicate row missing
  desired_state validate_state"` — PASS; reports `desired_state`, `record`,
  `set_desired_state`, and reconciliation/status relationships.
- `rg -n
  "CompanionStateRecord|CompanionStateToml|serde\\(default\\)|desired_state|validate_state|duplicate companion"
  src/daemon/plugins/companion/state_store.rs -S` — PASS for the selected seam;
  `desired_state` no longer has serde default while optional action/error
  telemetry remains explicitly optional.
- `cargo test -q companion::state_store --lib` — PASS (`4 passed`);
  includes missing `desired_state`, blank identity, unknown field, duplicate
  row, and fresh missing-file/absent-row disabled-state coverage.
- `cargo test -q daemon::plugins::companion --lib` — PASS (`41 passed`);
  verifies the manager/status/reconcile tests do not rely on old row repair.
- `cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .` — PASS; indexed
  the companion desired-state store parser and validation boundary.

## 2026-07-21 Installed plugin active-state schema

- `/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/bin/override/codegraph
  --version` — FAIL; override path was absent in the current environment.
- `/Users/macbook.silan.tech/.local/bin/codegraph --version` — PASS
  (`1.4.1`).
- `/Users/macbook.silan.tech/.local/bin/codegraph explore "plugin install
  state PluginStateToml serde default plugins malformed blank duplicate
  lockfile installed package state"` — UNAVAILABLE; checkout had no
  `.codegraph/` index and the tool instructed agents not to initialize one.
- `rg -n
  "PluginStateToml|InstalledPluginRecord|plugin-lock|plugins.toml|serde\\(default\\)|state_path\\(|PluginStateStore"
  src/daemon/plugins -S` — PASS for seam identification; found the state
  store parser, index lockfile parser, and old `plugins` default.
- `cargo test -q daemon::plugins::install::state --lib` — PASS (`6 passed`);
  covers missing-file fresh empty state, existing file without `plugins`,
  unknown fields, blank identity, duplicate active row, and multiple active
  versions.
- `cargo test -q daemon::plugins::install --lib` — PASS (`22 passed`);
  verifies install/update/remove and companion transaction paths still use the
  strict state parser.
- `cargo test -q daemon::plugins::index --lib` — PASS (`7 passed`);
  includes resilient lockfile reporting for malformed active state without
  projecting a successful installed package index.
- `cargo fmt --check` — PASS.
- `cargo check --lib --bins` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-21 Context store read-model schema

- `rg -n
  "read_to_string|serde_json::from_str|unwrap_or_default|filter_map"
  src/daemon/persistence/context_store.rs -S` — PASS for seam identification;
  found config/folders/favorites repair-to-default paths and capture JSONL row
  skipping.
- `cargo test -q daemon::persistence::context_store --lib` — PASS
  (`12 passed`); includes malformed config rejection, malformed folder state
  rejection, malformed favorite state rejection, malformed capture JSONL
  rejection, and existing clipboard corrupt-row fail-closed coverage.
- `cargo test -q resources::context --lib` — PASS (`28 passed`); verifies the
  context ability surface still returns the same public JSON shapes while
  propagating store errors.
- `cargo check --lib --bins` — PASS after migrating CLI, ability handlers,
  clipboard tracker, and media tests to the fallible Context store readers.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-21 Global skill pool package inventory

- `rg -n
  "scan_global_pool_into|global_skill_record_from_dir|skill_dir_in_global_pool|filter_map|flatten|\\.ok\\(\\)|unwrap_or_default"
  src/daemon/resources/skills/store.rs
  src/daemon/ability/builtins/resources/skills/list.rs
  src/daemon/ability/builtins/resources/skills/publish.rs -S` — PASS for seam
  identification; found the global pool `Option` / `flatten` paths that hid
  corrupt skill packages.
- `cargo test -q daemon::resources::skills::store --lib` — PASS (`24 passed`);
  covers declared-name lookup, corrupt package rejection, and archive top-dir
  scan behavior after removing entry `filter_map`.
- `cargo test -q resources::skills::list --lib` — PASS (`11 passed`);
  includes cache-level and handler-level rejection of global skill packages
  with missing frontmatter `name`.
- `cargo check --lib --bins` — PASS after migrating `skill.list` and
  `skill.publish` global-pool lookup to fallible APIs.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-21 Pages dynamic API ability discovery

- `rg -n
  "api_ability_names_for_project|read_dir\\(&api_dir\\)|read_dir\\.flatten|return Vec::new\\(\\)"
  src/daemon/ability/builtins/resources/pages/api.rs -S` — PASS for seam
  identification; found the dynamic API route discovery empty-list fallback.
- `cargo test -q pages::api --lib` — PASS (`4 passed`); covers missing
  `api/` as valid no-API state and non-directory `api` path as corrupt state
  that must fail discovery.
- `cargo check --lib --bins` — PASS after migrating
  `register_api_abilities_for_project` to fallible discovery.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-21 Device product local identity fallback

- `/Users/macbook.silan.tech/.local/bin/codegraph status
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — NOT
  INITIALIZED; no `.codegraph/` index was created for this checkout.
- `rg -n
  "load_credentials\\(\\)\\.ok\\(\\)|unwrap_or_default\\(\\).*local_node|local_tenant|String::new\\(\\).*local_ura|classify_device_show_target\\("
  src/cli/commands/groups/device.rs -S` — PASS; no credentials swallow,
  empty local identity synthesis, or old classifier signature remains.
- `cargo test -q cli::commands::groups::device --lib` — PASS (`8 passed`);
  includes explicit `DeviceLocalIdentity` construction, blank realm/node
  rejection, canonical self Device URA local classification, bare remote id
  rejection, and existing `node.describe` schema-bound projections.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  R94 negative fixture for `load_credentials().ok()`, `unwrap_or_default()`,
  empty `local_ura`, and old local-node-only classifier fallback.
- `cargo check --lib --bins` — PASS after migrating device show/remove to
  explicit local identity.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `PATH="/Users/macbook.silan.tech/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
  SDK_CONFORMANCE_PYTHON="$(pwd)/sdk/python/.venv/bin/python" bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-22 Pages API body ingress fallback

- `/Users/macbook.silan.tech/.local/bin/codegraph status` — NOT
  INITIALIZED; no `.codegraph/` index was created for this checkout.
- `rg -n
  "unwrap_or\\(Value::Null\\)|unwrap_or_default\\(\\)|\\.ok\\(\\)\\?|\\.ok\\(\\)"
  src/daemon/resources/pages ...` — PASS for seam identification; found the
  Pages listener `/api/<verb>` malformed-body-to-null fallback.
- `cargo test -q daemon::resources::pages::pages_listener --lib` — PASS
  (`12 passed`); includes absent body projected to null, malformed body
  rejected by the parser, and malformed `/api/<verb>` request returning HTTP
  400 before dispatch.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  R95 negative fixture for `serde_json::from_slice(&body_bytes).unwrap_or(
  serde_json::Value::Null)`.
- `cargo check --lib --bins` — PASS.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-22 Runtime lifecycle projection load fallback

- `/Users/macbook.silan.tech/.local/bin/codegraph status` — NOT
  INITIALIZED; no `.codegraph/` index was created for this checkout.
- `rg -n
  "RuntimeProjectionStore|\\.load\\(\\).*projection|projection_store|load_current\\(\\)|RuntimeStatusReport::capture|from_parts\\("
  src/daemon/boot/lifecycle src/cli src/ffi tests` — PASS for seam
  identification; found `RuntimeSessionProjection::load_current` swallowing
  `config::load` errors through `Option`.
- `cargo test -q daemon::persistence::config --lib` — PASS (`21 passed`);
  includes missing `runtime.json` returning `None` and malformed existing
  `runtime.json` returning a parse error.
- `cargo test -q daemon::boot::lifecycle --lib` — PASS (`22 passed`);
  includes malformed projection rejection at `RuntimeSessionProjection` and
  `RuntimeLifecycleService::preflight_start` without removing the corrupt
  projection as stale state.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  R96 negative fixture for `config::load().ok().map(Self::from_state)`.
- `cargo check --lib --bins` — PASS.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-22 Device reset runtime projection fallback

- `/Users/macbook.silan.tech/.local/bin/codegraph status` — NOT
  INITIALIZED; no `.codegraph/` index was created for this checkout.
- `rg -n
  "config::load\\(\\)\\.ok\\(\\)|RuntimeLifecycleService|runtime projection|runtime_state|runtime\\.json"
  src/cli/presentation src/cli/commands src/daemon/boot/lifecycle ...` — PASS
  for seam identification; found `reset` reading `runtime.json` directly.
- `env RUSTC_WRAPPER= cargo test -q cli::commands::reset --lib` — PASS
  (`2 passed`); covers malformed `runtime.json` aborting before credentials
  deletion and stale parseable projection cleanup through the lifecycle
  report.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  R97 negative fixture for `config::load().ok()` and `config::remove().ok()`
  in `reset`.
- `env RUSTC_WRAPPER= cargo check --lib --bins` — PASS.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-22 MCP status runtime projection fallback

- `/Users/macbook.silan.tech/.local/bin/codegraph status` — NOT
  INITIALIZED; no `.codegraph/` index was created for this checkout.
- `rg -n
  "mcp status|run_status\\(|config::load\\(\\)\\.ok\\(\\)|RuntimeLifecycleService"
  src/cli tests tools/scripts/check-architecture-convergence.sh ...` — PASS
  for seam identification; found MCP status reading `runtime.json` directly.
- `env RUSTC_WRAPPER= cargo test -q cli::commands::groups::mcp --lib` — PASS
  (`1 passed`); covers malformed `runtime.json` aborting MCP status instead
  of rendering "runtime not running".
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  R98 negative fixture for `config::load().ok()` in MCP status.
- `env RUSTC_WRAPPER= cargo check --lib --bins` — PASS.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-22 Banner runtime projection fallback

- `/Users/macbook.silan.tech/.local/bin/codegraph status` — NOT
  INITIALIZED; no `.codegraph/` index was created for this checkout.
- `rg -n
  "config::load\\(\\)\\.ok\\(\\)|render_top_level_banner|write_runtime_status|RuntimeLifecycleService|runtime projection"
  src/cli/presentation src/cli ...` — PASS for seam identification; found
  the top-level help banner reading `runtime.json` directly.
- `env RUSTC_WRAPPER= cargo test -q cli::presentation::banner --lib` — PASS
  (`5 passed`); covers clean banner rendering and malformed `runtime.json`
  rendering `metadata unavailable` instead of stopped.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS; includes
  R99 negative fixture for `config::load().ok()` in banner runtime status.
- `rg -n "config::load\\(\\)\\.ok\\(\\)"
  src/cli src/daemon src/support sdk tests tools/scripts/check-architecture-convergence.sh
  tests/scripts/test_check_architecture_convergence.sh` — PASS; only
  architecture gate patterns and negative fixtures remain, no production
  `src/` path remains.
- `env RUSTC_WRAPPER= cargo check --lib --bins` — PASS.
- `cargo fmt --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).

## 2026-07-22 Auth agents canonical backend row projection

- `/Users/macbook.silan.tech/.local/bin/codegraph status
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; index was
  up to date before edits.
- `rg -n
  'or_else\(\|\| a\.get\("(ura|name)"\)\)|a\.get\("(ura|name)"\)'
  src/cli/commands/auth.rs` — PASS; no production `auth agents` row alias
  fallback remains.
- `cargo test -q auth_agents --lib --features axon-pb` — PASS (`2 passed`);
  covers canonical backend fields and rejects retired `ura` / `name` row
  aliases.
- `cargo fmt --all -- --check` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes the negative fixture for retired `auth agents` row alias
  fallback.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS
  (`test_check_canonical_runtime_convergence_v2 ok`).
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; synced
  changed Rust nodes.
- `/Users/macbook.silan.tech/.local/bin/codegraph status
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; index is
  up to date after edits.

## 2026-07-22 Pages identity credential state classification

- `/Users/macbook.silan.tech/.local/bin/codegraph query -p
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli "fallback"
  --limit 40` — PASS for candidate discovery; selected Pages identity because
  `from_env()` swallowed credentials and port parse failures into default
  product state.
- `rg -n
  "PagesIdentity::from_env|pub fn from_env\\(\\) -> Self|load_credentials\\(\\)\\s*\\.ok\\(\\)|EASYNET_PAGES_PORT.*parse::<u16>\\(\\)\\.ok\\(\\)"
  src/daemon/ability/builtins/resources/pages src/bin/easynet-daemon.rs
  src/bin/real-user-smoke.rs src/daemon/persistence/config.rs` — PASS; no
  retired Pages identity credential/port fallback remains in the migrated
  path.
- `cargo test -q pages_identity --lib --features axon-pb` — PASS (`5
  passed`); covers missing credentials as unpaired, present credentials,
  malformed credentials failure, invalid port failure, and zero port failure.
- `cargo test -q load_credentials_optional --lib --features axon-pb` — PASS
  (`2 passed`); covers missing-file optional state and malformed-existing-file
  failure.
- `cargo check -q --bin easynet-daemon --bin real-user-smoke --features
  axon-pb` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes the negative fixture for the retired infallible Pages
  identity env resolver.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS
  (`test_check_canonical_runtime_convergence_v2 ok`).
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; synced
  changed Rust nodes.
- `/Users/macbook.silan.tech/.local/bin/codegraph status
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; index is
  up to date after edits.

## 2026-07-22 Mission traditional agent target conflict naming

- `/Users/macbook.silan.tech/.local/bin/codegraph query -p
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli
  "find_implicit_agent_fallback" --limit 20` — PASS for candidate
  discovery; found the Mission validator still named after the retired
  implicit fallback concept.
- `rg -n
  "find_implicit_agent_fallback|ImplicitAgentFallback|implicit agent
  fallback|implicit-agent-fallback|no_implicit_agent_fallback|implicit-fallback"
  src/daemon/execution/mission src/eal/parser src/eal/runtime
  tests/scripts/test_check_architecture_convergence.sh` — PASS; no retired
  Mission fallback concept remains in active Mission source or architecture
  fixture.
- `cargo test -q traditional_agent_target_conflict --lib --features axon-pb`
  — PASS (`3 passed`); covers registered-agent traditional target rejection,
  member-call acceptance, and device-name traditional target acceptance.
- `cargo fmt --all -- --check` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS
  (`all cases passed`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes the negative fixture for retired Mission fallback naming.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS
  (`test_check_canonical_runtime_convergence_v2 ok`).
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; synced
  changed Rust/script nodes.
- `/Users/macbook.silan.tech/.local/bin/codegraph status
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; index is
  up to date after edits.

## 2026-07-22 Device settings fail-closed loader

- `/Users/macbook.silan.tech/.local/bin/codegraph query -p
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli
  "descriptor_ref_for_call_mode" --limit 30` — PASS for candidate triage;
  the descriptor helper was test-only, so the production fallback selected
  for this iteration was the `device_settings.json` loader.
- `rg -n
  "load_device_settings\\(\\) -> DeviceSettings|fs::read_to_string\\(&path\\)\\s*\\.ok\\(\\)|serde_json::from_str\\(&data\\)\\.ok\\(\\)|unwrap_or_default\\(\\)"
  src/daemon/persistence/config.rs src/cli/commands/config_cmd.rs` — PASS;
  no retired settings default fallback remains in the migrated path.
- `cargo test -q load_device_settings --lib --features axon-pb` — PASS (`3
  passed`); covers missing-file default, malformed existing file failure, and
  unknown-field failure.
- `cargo test -q install_id_is_generated_once_and_stable_across_calls_and_reset
  --lib --features axon-pb` — PASS (`1 passed`).
- `cargo check -q --lib --features axon-pb` — PASS; covers config command
  callers after `load_device_settings()` became fallible.
- `cargo fmt --all -- --check` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes the negative fixture for the retired defaulting settings
  loader.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS
  (`canonical-runtime-convergence-v2: OK`).
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS
  (`architecture-convergence: OK`).
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS
  (`test_check_canonical_runtime_convergence_v2 ok`).
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS
  (`all cases passed`).
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; synced
  changed Rust/script nodes.
- `/Users/macbook.silan.tech/.local/bin/codegraph status
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; index is
  up to date after edits.
## 2026-07-22 Local API key default-token cache

- `cargo test -q local_default_token_cache --lib --features axon-pb` — PASS
  (`5 passed`); covers missing cache as no-default-token, written token read,
  malformed TOML rejection, unknown-field rejection, and blank token rejection.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired `Option + .ok()?`
  local-token cache reader.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.

## 2026-07-22 Runtime trust revoke credential projection

- `cargo test -q local_connection_state_projector --lib --features axon-pb`
  — PASS (`2 passed`); covers missing credentials as no-projector state and
  malformed credentials as fail-closed unavailable identity state.
- `cargo test -q removed_local_user_revoke_records_disconnected_removed_snapshot
  --lib --features axon-pb` — PASS (`1 passed`); verifies the migrated
  projector path still records the local disconnected-removed lifecycle state
  for a valid local user revoke.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired
  `load_credentials().ok()?` projector and post-mutation projector creation.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.

## 2026-07-22 Admission owner credential projection

- `cargo test -q local_device_owner_resolution_rejects_malformed_credentials
  --lib --features axon-pb` — PASS (`1 passed`); covers malformed
  credentials failing owner resolution instead of projecting unresolved owner.
- `cargo test -q paired_device_subject_projects_credentials_owner --lib
  --features axon-pb` — PASS (`1 passed`); preserves valid paired-device
  owner projection.
- `cargo test -q user_subject_projects_owner_policy_allow --lib --features
  axon-pb` — PASS (`1 passed`); verifies the fallible owner-resolution path
  still feeds policy allow for a user-owned subject.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired
  `load_credentials().ok()?` local owner fallback.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.

## 2026-07-22 Shared local device owner projection

- `cargo test -q local_device_owner_fact --lib --features axon-pb` — PASS
  (`3 passed`); covers missing credentials, valid credentials, and malformed
  credentials at the shared projector.
- `cargo test -q device_principal_projection_rejects_malformed_local_credentials
  --lib --features axon-pb` — PASS (`1 passed`).
- `cargo test -q malformed_local_credentials_make_bootstrap_owner_unavailable
  --lib --features axon-pb` — PASS (`1 passed`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired `Option` projector,
  non-fallible device principal, and bootstrap `NotApplicable` fallback.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.

## 2026-07-22 Node session authority subject binding

- `node --test sdk/node/test/runtime-core.test.mjs` — PASS (`12 passed`);
  covers valid typed authority metadata, all-zero owner rejection, mismatched
  user subject rejection, mismatched user-session resource rejection, and
  request-side rejection of device subjects.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired all-zero-only Node
  session authority validator.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; synced
  Node SDK/test changes and left the index up to date.
- `git diff --check` — PASS.

## 2026-07-22 Session prelude paired-user credentials

- `cargo test -q paired_user_trust_bootstrap --lib --features axon-pb`
  — PASS (`2 passed`); covers missing credentials as the only `NotRequired`
  local state and malformed credentials as `CredentialsUnavailable`.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired `let Ok(...) else
  NotRequired` credential fallback.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.

## 2026-07-22 Descriptor-ref route selector fail-closed cutover

- `cargo test -q descriptor_ref --lib --features axon-pb` — PASS (`35
  passed`); covers malformed descriptor refs and descriptor owner mismatch
  failing before public-name/catalog lookup.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired
  `Option`/`.ok()?` descriptor selector fallback.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` —
  PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date` with 35,276 nodes and
  135,370 edges.

## 2026-07-22 Heartbeat owner-projection cursor input

- `cargo test -q -p easynet --lib
  daemon::invocation::bidi::session_initiator::heartbeat::tests::heartbeat_refresh_owner_uras`
  — PASS (`3 passed`); covers missing cursor store as empty first-boot state,
  corrupt/schema-less cursor rejection, and caller-owner filtering.
- `cargo test -q -p easynet --lib
  daemon::invocation::bidi::session_initiator::heartbeat::tests::federation_heartbeat_receipt`
  — PASS (`3 passed`); revalidated heartbeat receipt state after moving owner
  refresh construction behind a fallible boundary.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date`.

## 2026-07-22 Python SDK session-authority subject projection

- `go test . -run 'TestAuthorizedRuntimeSession|TestRuntimeAbilityClient'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/go` —
  PASS.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python
  python -m pytest -q
  sdk/python/tests/test_authorized_runtime_session.py
  sdk/python/tests/test_runtime_ability.py` — PASS (`21 passed`).
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes the SDK history owner-expansion negative fixture and now
  requires structured Python authority subject projection.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date` with 35,291 nodes and
  135,427 edges.

## 2026-07-22 Daemon exact-route descriptor-ref projection

- `cargo test -q -p easynet --lib route_table_ --features axon-pb` — PASS
  (`3 passed`); covers descriptor-ref projection, malformed descriptor-ref
  fail-closed behavior, and descriptor owner mismatch rejection.
- `cargo test -q -p easynet --lib
  malformed_descriptor_ref_does_not_fall_through_as_public_name --features
  axon-pb` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for daemon exact-route descriptor
  projection fallback.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date` with 35,295 nodes and
  135,449 edges.

## 2026-07-22 Bidi exact-route descriptor-ref projection

- `cargo test -q -p easynet --lib route_table_ --features axon-pb` — PASS
  (`4 passed`); now includes Hub-owned `session.open` bidi descriptor-ref
  projection.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; gate now requires bidi frame-0 routing to call
  `dispatch_function_name_for_route_table`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date` with 35,296 nodes and
  135,458 edges.

## 2026-07-22 Canonical ability catalog projection

- `cargo test -q -p easynet --lib
  daemon::federation::read_model::hub_published_abilities` — PASS (`8
  passed`); covers canonical hub descriptor storage, noncanonical snapshot
  rejection, and atomic diff rejection.
- `cargo test -q -p easynet --lib
  daemon::ability::builtins::governance::meta::tests::list_abilities_realm_scope_includes_hub_published_entries`
  — PASS; verifies realm-scope hub entries are serialized as canonical
  descriptor rows with `descriptor_ref`.
- `cargo test -q -p easynet --lib
  ffi::invocation::tests::descriptor_catalog_dedupe_rejects_schema_incomplete_rows`
  — PASS; verifies FFI descriptor catalog dedupe no longer silently drops
  schema-incomplete rows.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired opaque hub catalog and
  silent descriptor-catalog dedupe fallback.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date` with 35,260 nodes and
  135,317 edges.

## 2026-07-22 Federation prelude receipt state machine

- `cargo test -q -p easynet --lib
  daemon::invocation::bidi::session_initiator::prelude::tests::federation_join_receipt_rejects_empty_or_malformed_body`
  — PASS.
- `cargo test -q -p easynet --lib
  daemon::invocation::bidi::session_initiator::prelude::tests::federation_join_receipt_seeds_canonical_hub_catalog`
  — PASS.
- `cargo test -q -p easynet --lib
  daemon::invocation::bidi::session_initiator::heartbeat::tests::federation_heartbeat_receipt`
  — PASS (`3 passed`); covers empty/malformed heartbeat receipt rejection,
  revision-only diff cursor advancement, and canonical added-row projection.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for tolerant join/heartbeat receipt
  decode and revision-only diff skipping.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.

## 2026-07-22 FFI descriptor runtime owner precondition

- `cargo test -q runtime_descriptor --lib --features axon-pb` — PASS (`7
  passed`); covers runtime descriptor local catalog resolution, missing
  `call_mode`, local catalog miss behavior, explicit `caller_ura`, runtime
  owner identity precondition, and descriptor error projection.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired
  `runtime_owner_ura_from_session(session).ok()` descriptor resolver fallback.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` —
  PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date`.

## 2026-07-22 Invocation history ledger URA projection

- `cargo test -q ledger_resource_ura --lib --features axon-pb` — PASS (`1
  passed`); covers unjoined null projection, canonical device/user/agent
  ledger resource URAs, and malformed hosted identity fail-closed behavior.
- `cargo test -q invocation_history --lib --features axon-pb` — PASS (`34
  passed`); preserves existing history list/get/trace/path behavior after
  making `ledger_ura` fallible.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes a negative fixture for the retired
  `ledger_resource_ura() -> Option<String>` and
  `load_hosted_identity_status().ok()?` fallback.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `GOCACHE=/tmp/easynet-go-build-cache bash
  tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` —
  PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date`.

## 2026-07-22 Invocation history filter scope

- `cargo test -q -p easynet --lib invocation_history --features axon-pb` —
  PASS (`36 passed`); covers canonical key URA validation, malformed
  caller/callee/agent/subject/ability/state filter rejection, canonical
  Ability URA set filters, cursor behavior, and attempt-ledger filter sharing.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS;
  includes `check_invocation_history_filter_scope_contract`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` —
  PASS; includes a negative fixture for the retired bare string filter
  projection.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date` with 35,305 nodes and
  135,492 edges.

## 2026-07-22 Namespace resolver authority projection

- `cargo test -q -p easynet --lib authority_projection_ --features axon-pb`
  — PASS (`3 passed`); covers route-ref embedded realm projection,
  descriptor-ref embedded realm projection, and invalid query unavailable
  authority instead of localhost fallback.
- `cargo test -q -p easynet --lib
  namespace_resolve_input_failure_does_not_fabricate_localhost_authority
  --features axon-pb` — PASS (`1 passed`); covers schema-failure
  `namespace.resolve` output using explicit unavailable authority state.
- `cargo test -q -p easynet --lib
  namespace_resolve_rejects_missing_qtype_without_guessing_route_shape
  --features axon-pb` — PASS (`1 passed`); preserves typed qtype ingress
  rejection after authority projection refactor.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` —
  PASS; includes a negative fixture for legacy resolver authority projection
  that defaulted malformed query identity to localhost.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS;
  codegraph `status` reports `Index is up to date` with 35,311 nodes and
  135,509 edges.

## 2026-07-22 Java SDK runtime receipt projection

- `mvn -q -f sdk/java/pom.xml test` — PASS; covers Java runtime result
  construction through canonical terminal receipts, retired `receipt` alias
  rejection, and missing `authority_proof` rejection through both
  `RuntimeReceipt` and `InvocationResult`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` —
  PASS; includes a negative fixture for the retired Java
  `terminal_receipt` map-to-empty downgrade.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS;
  includes `check_java_sdk_runtime_receipt_projection_contract`.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; codegraph
  `status` reports `Index is up to date` with 1,019 files, 35,361 nodes, and
  135,698 edges.

## 2026-07-22 Node SDK and cross-SDK RuntimeReceipt type/state binding

- `go test . -run
  'TestRuntimeReceipt|TestInvocationResultRejectsConflictingCanonicalReceiptBindings'`
  from `sdk/go` — PASS; covers direct RuntimeReceipt proof facts, fail-closed
  lifecycle parsing, and rejection of retired `terminal`/mismatched
  `receipt_type` values before result-topology parsing.
- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python:sdk/python/tests
  python3 -m unittest sdk.python.tests.test_runtime` — PASS (`36` tests);
  covers direct Python `RuntimeReceipt` proof-fact validation and
  lifecycle-derived `receipt_type` rejection.
- `mvn -q -f sdk/java/pom.xml test` — PASS; covers Java `RuntimeReceipt`
  direct rejection of `receipt_type="terminal"` and InvocationResult receipt
  projection.
- `npm test --prefix sdk/node` — PASS (`36` tests); covers the new Node
  `RuntimeReceipt`, mandatory proof facts, retired top-level `receipt` alias
  rejection, and canonical terminal receipt conformance fixtures.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS;
  includes the SPEC gate self-test harness after adding the cross-SDK
  receipt type/state binding contract.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes negative fixtures for opaque Node receipt projection and
  SDK RuntimeReceipt type/state binding regressions.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS;
  includes `check_node_sdk_runtime_receipt_projection_contract` and
  `check_sdk_runtime_receipt_type_state_binding_contract`.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS; synced the
  changed SDK/gate files.
- `/Users/macbook.silan.tech/.local/bin/codegraph status` — PASS; index is up
  to date with 1,019 files, 35,395 nodes, and 135,788 edges.

## 2026-07-22 AbilityDescriptor descriptor-ref derivation

- `cargo test -q -p easynet --lib descriptor_ref --features axon-pb` — PASS
  (`40` tests); includes direct fail-closed descriptor-ref derivation for a
  corrupt descriptor identity and existing descriptor-ref route/bridge tests.
- `cargo test -q -p easynet --lib hub_published_abilities --features axon-pb`
  — PASS (`8` tests); confirms hub-published ability rows still validate
  canonical descriptors after descriptor-ref derivation became fallible.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes the legacy optional descriptor-ref fixture under the
  canonical ability catalogue projection gate.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` — PASS.
- `cargo check -q -p easynet --lib --features axon-pb` — PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS after applying `cargo fmt --all`.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph status` — PASS; index is up
  to date with 1,019 files, 35,397 nodes, and 135,799 edges.

## 2026-07-22 FFI invocation JSON projection

- `cargo test -q -p easynet --lib
  unary_result_json_rejects_declared_json_output_that_is_not_json --features
  axon-pb` — PASS; proves declared JSON unary output no longer projects as
  `null` when parsing fails.
- `cargo test -q -p easynet --lib
  stream_chunk_json_rejects_declared_json_payload_that_is_not_json --features
  axon-pb` — PASS; proves declared JSON stream payloads fail closed before
  callback projection.
- `cargo test -q -p easynet --lib ffi::invocation::tests --features
  axon-pb` — PASS (`88` tests); covers the migrated handle snapshot/events
  projection plus existing FFI invocation construction/signing/stream paths.
- `cargo check -q -p easynet --lib --features axon-pb` — PASS.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  — PASS; includes the legacy `.ok()` JSON projection downgrade fixture.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — PASS;
  includes `check_ffi_invocation_json_projection_contract`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh` —
  PASS.
- `bash tools/scripts/check-architecture-convergence.sh` — PASS.
- `bash tests/scripts/test_check_architecture_convergence.sh` — PASS.
- `cargo fmt --all -- --check` — PASS.
- `git diff --check` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync
  /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` — PASS.
- `/Users/macbook.silan.tech/.local/bin/codegraph status` — PASS; index is up
  to date with 1,019 files, 35,400 nodes, and 135,836 edges.
