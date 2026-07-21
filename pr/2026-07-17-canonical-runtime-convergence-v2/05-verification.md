# Verification

## Required Matrix

- Rust formatting and compilation.
- Targeted Rust tests for changed daemon/runtime modules.
- SDK conformance policy scripts.
- Go SDK unit tests covering invocation, receipt, lifecycle, and matrix
  evidence.
- Python SDK compile/tests covering invocation, receipt, lifecycle, and matrix
  evidence.
- Canonical runtime convergence V2 gate.
- URA terminology and schema derivation gates.

## Current Iteration Evidence

- `cargo fmt --check`
- `cargo check --lib --bins`
- `cargo test --lib --no-run`
- `cargo test --test script_checks`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `bash tools/scripts/check-skill-list-managed-dir-boundary.sh`
- `bash tests/scripts/test_check_skill_list_managed_dir_boundary.sh`
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --check`
- `SDK_CONFORMANCE_RESULT_DIR=target/sdk-conformance-live-results.current bash tools/scripts/check-sdk-conformance-reports.sh`
- `EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results.current EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
- `cd sdk/go && go test ./...`
- `cd sdk/python && python3 -m py_compile $(find easynet_sdk -name '*.py' | sort)`

Result: passed.

## Boot Lifecycle Authority Iteration Evidence

- `cargo fmt`
- `cargo fmt --check`
- `cargo test --lib start_boot_watcher --features axon-pb`
- `cargo test --lib boot_events --features axon-pb`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
- `rg -n "BootContext|pages_start_hint|fallback for daemons too old to send|pre-date this field|CLI just peeks" src/cli/commands/start_boot_watcher.rs src/cli/commands/start.rs src/daemon/control/boot_events.rs`
- `git diff --check`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-architecture-convergence.sh`

Result: passed. The `start_boot_watcher` test selector compiled the library but
matched no tests; the `boot_events` selector ran three tests and passed. The
`rg` command intentionally returned no matches after deletion.

## MCP Installer Contract Iteration Evidence

- `cargo fmt --check`
- `cargo test --lib mcp::install --features axon-pb`
- `git diff --check`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-architecture-convergence.sh`

Result: passed. The focused Rust test proves that generated MCP install args
contain only `mcp serve --tenant [--agent]` and that retired no-op flags
`--endpoint`, `--bound-node`, and `--allow-node-override` are rejected by the
parser.

## Bidi Frame0 Provider-Backed Iteration Evidence

- `cd sdk/go && go test -tags easynet_direct_runtime -run 'TestDirectRuntimeBidiRejectsMissingFrame0BeforeSessionEntry|TestDirectRuntimeTransportBidiOverUnixSocket|TestDirectRuntimeBidiCancelProjectsNonTerminalRequest' .`
- `cd sdk/python && PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:.:tests .venv/bin/python -m unittest test_direct_runtime.DirectRuntimeTests.test_direct_bidi_rejects_missing_frame0_before_session_entry test_direct_runtime.DirectRuntimeTests.test_direct_runtime_bidi_provider_json_uses_terminal_receipt test_direct_runtime.DirectRuntimeTests.test_direct_runtime_bidi_cancel_projects_non_terminal_request`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-self-test`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo fmt --all -- --check`
- `cargo check --lib --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check -- sdk/go/direct_runtime.go sdk/go/direct_runtime_test.go sdk/python/easynet_sdk/direct_runtime.py sdk/python/tests/test_direct_runtime.py sdk/conformance/cases/bidi-frame0-required.yaml sdk/conformance/runner/execution-manifest.json sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json pr/2026-07-17-canonical-runtime-convergence-v2/04-execution-checklist.md pr/2026-07-17-canonical-runtime-convergence-v2/05-verification.md pr/2026-07-17-canonical-runtime-convergence-v2/06-decisions-log.md`

Result: passed. The first Python focused command used an invalid unittest class
selector for `test_conformance_gates.py`; the pytest-style function was then
run with pytest and passed.

## Stream/Bidi Cancel Vector Iteration Evidence

- `cd sdk/go && go test -run 'TestConformanceStreamCancelRequestIsNonTerminal|TestConformanceBidiCancelRequestIsNonTerminal|TestConformanceStreamAndBidiBackpressureBounds' .`
- `cd sdk/python && PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:.:tests .venv/bin/python -m pytest -q tests/test_conformance_gates.py::test_stream_cancel_request_is_non_terminal tests/test_conformance_gates.py::test_bidi_cancel_request_is_non_terminal tests/test_conformance_gates.py::test_stream_and_bidi_backpressure_bounds`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-cancel-self-test`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-cancel-self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo fmt --all -- --check`
- `cargo check --lib --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check -- sdk/conformance/cases/bidi-cancel-request.yaml sdk/conformance/cases/stream-cancel-request.yaml sdk/go/conformance_test.go sdk/python/tests/test_conformance_gates.py sdk/conformance/runner/execution-manifest.json sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json pr/2026-07-17-canonical-runtime-convergence-v2/04-execution-checklist.md pr/2026-07-17-canonical-runtime-convergence-v2/05-verification.md pr/2026-07-17-canonical-runtime-convergence-v2/06-decisions-log.md`

Result: passed.

## Deadline Vector Iteration Evidence

- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-self-test`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --all -- --check`
- `cargo check --lib --features axon-pb`
- `cd sdk/go && go test -tags easynet_direct_runtime -run 'TestDirectRuntimeInvokeDeadlineIsTypedTimeout|TestCanonicalInvocationBytesUsesStableAxonEncoding|TestCanonicalInvocationBytesRejectsMalformedCausalHash|TestDescriptorBoundSubjectURAProjectsUserSubjectBeforeSigning|TestCanonicalInvocationBytesRejectsUnprojectedUserSubject' .`
- `cd sdk/python && PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:.:tests .venv/bin/python -m unittest test_transport.DaemonInvocationTransportTests.test_unary_pool_retires_timed_out_owned_transport`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `git diff --check -- sdk/go/direct_runtime_test.go sdk/go/invocation_canonical.go sdk/go/invocation_canonical_test.go sdk/go/runtime_subject_test.go sdk/conformance/cases/runtime-deadline-owner.yaml sdk/conformance/canonical-public-api.json sdk/conformance/runner/execution-manifest.json sdk/conformance/sdk-parity-matrix.json`
- `cd sdk/go && go test -run 'TestRuntimeSigningTransport|TestRuntimeClientPrepareDelegatesToTransport|TestRuntimeClientPrepareSigningMaterialUsesStatelessTransportContract|TestInvocationResultProjectsRuntimeTupleAndReceipts' .`

Result: passed, except `sdk_matrix.py --validate` was not run because that mode
requires a live results directory. `sdk_matrix.py --self-test` covered canonical
generation and validation logic for this iteration.

Additional result: `cd sdk/go && go test ./...` is blocked only by
`TestConformanceSevenLanguageCapabilityMatrix`, whose wrapper requires
`EASYNET_SDK_PARITY_RESULTS_DIR`. `bash tools/scripts/check-sdk-parity-matrix.sh
--self-test` also exits non-zero after printing `sdk parity matrix self-test ok`;
the underlying `sdk_matrix.py --self-test` passed and is the recorded matrix
logic proof for this iteration.

## Native Runtime Start Vector Iteration Evidence

- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-self-test`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cd sdk/go && go test -run 'TestSdkEnvironmentOwnsProcessRootAndConnectsRuntime|TestNativeRuntimeHandleClosesClientAndProvider|TestNativeRuntimeHandleAlwaysProvidesCanonicalAddressing|TestNativeRuntimeHandleRequiresHealthFacade' .`
- `cd sdk/python && PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:.:tests .venv/bin/python -m unittest test_environment.SdkEnvironmentTests.test_native_runtime_owns_runtime_and_health`
- `cargo fmt --all -- --check`
- `cargo check --lib --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check -- sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json pr/2026-07-17-canonical-runtime-convergence-v2/04-execution-checklist.md pr/2026-07-17-canonical-runtime-convergence-v2/05-verification.md pr/2026-07-17-canonical-runtime-convergence-v2/06-decisions-log.md`

Result: passed.

## Runtime Provider Mapping Iteration Evidence

- `cd sdk/go && go test -run 'TestSdkEnvironmentOwnsProcessRootAndConnectsRuntime|TestRuntimeHealthExposesControlOnlyState' .`
- `cd sdk/python && PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:.:tests .venv/bin/python -m pytest -q tests/test_environment.py::SdkEnvironmentTests::test_native_runtime_owns_runtime_and_health tests/test_control_ipc.py::ControlIpcTests::test_reads_discovery_and_negotiates_v1`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-runtime-provider-self-test`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-runtime-provider-self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check -- sdk/conformance/rebuild_public_api_model.py sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json pr/2026-07-17-canonical-runtime-convergence-v2/04-execution-checklist.md pr/2026-07-17-canonical-runtime-convergence-v2/05-verification.md pr/2026-07-17-canonical-runtime-convergence-v2/06-decisions-log.md`

Result: passed. `runtime_environment`, `runtime_connection`, and
`runtime_lifecycle` are now provider-backed for Go and Python through the same
direct runtime provider as `native_runtime`, with only the `start` lifecycle
vector closed.

Additional result: `bash tools/scripts/check-sdk-canonical-public-api.sh`
currently fails before generated-output comparison with
`inventory_source_revision_mismatch:rust:expected=def78f91805209cff0a906298c740c080b36aa58:actual=a80b21bd8f7fbbdb5bc7c864f6bf692da616189c`.
The failure reflects the current adjacent EasyNet-Axon checkout revision and is
not treated as a passed gate for this iteration.

## Stream/Bidi Deadline Vector Iteration Evidence

- `cd sdk/go && go test -tags easynet_direct_runtime -run 'TestDirectRuntimeStreamDeadlineIsTypedTimeout|TestDirectRuntimeBidiDeadlineIsTypedTimeout' -count=1 .`
- `env PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest -q -p no:cacheprovider sdk/python/tests/test_direct_runtime.py::DirectRuntimeTests::test_direct_runtime_stream_deadline_is_typed_timeout sdk/python/tests/test_direct_runtime.py::DirectRuntimeTests::test_direct_runtime_bidi_deadline_is_typed_timeout`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `python3 -m json.tool sdk/conformance/runner/execution-manifest.json >/tmp/execution-manifest.json.checked`
- `python3 -m json.tool sdk/conformance/canonical-public-api.json >/tmp/canonical-public-api.json.checked`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-stream-bidi-deadline-self-test`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-stream-bidi-deadline-self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --all -- --check`

Result: focused tests and model validation passed. Go and Python `stream` and
`bidi` now carry `deadline` lifecycle evidence through direct runtime provider
selectors. The cells remain `provider-backed`, not `cutover-ready`, because
`child_dispatch`, `restart_recover`, and `start` are still open for those
capabilities after the later dispatch-vector iteration.

## Stream/Bidi Dispatch Vector Iteration Evidence

- `cd sdk/go && go test -tags easynet_direct_runtime -run 'TestDirectRuntimeTransportStreamsOverUnixSocket|TestDirectRuntimeTransportBidiOverUnixSocket' -count=1 .`
- `env PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest -q -p no:cacheprovider sdk/python/tests/test_direct_runtime.py::DirectRuntimeTests::test_direct_runtime_stream_provider_json_uses_terminal_receipt sdk/python/tests/test_direct_runtime.py::DirectRuntimeTests::test_direct_runtime_bidi_provider_json_uses_terminal_receipt`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-stream-bidi-dispatch-self-test`

Result: focused tests and matrix validation passed. Go and Python `stream` and
`bidi` now carry `dispatch` lifecycle evidence through direct runtime provider
selectors. The cells remain `provider-backed`, not `cutover-ready`, because
`child_dispatch`, `restart_recover`, and `start` are still open for those
capabilities.

## Ability Child Dispatch Vector Iteration Evidence

- `cd sdk/go && go test -run 'TestRuntimeAbilityChildContextDispatchesWithParentReceiptCausality' -count=1 .`
- `env PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/tests PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest -q -p no:cacheprovider sdk/python/tests/test_ability_invocation.py::AbilityInvocationClientTests::test_child_context_anchors_child_invocation_to_parent_receipt`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-ability-child-dispatch-self-test`

Result: focused tests, schema validation, and matrix validation passed. Go and
Python `ability_invocation_facade` now carry `child_dispatch` lifecycle evidence
through provider-backed runtime selectors. The child vector requires a parent
terminal receipt, derives scalar Axon causal context, dispatches the child
through Runtime Core, and observes the parent receipt link in the child
terminal receipt. The cells remain `provider-backed`, not `cutover-ready`.

Additional result: the canonical public API model now records the adjacent
EasyNet-Axon revision as
`a80b21bd8f7fbbdb5bc7c864f6bf692da616189c`, aligning the model with the current
checkout used by the rebuild script instead of the prior
`def78f91805209cff0a906298c740c080b36aa58` baseline.

## Ability Provider Lifecycle Vector Iteration Evidence

- `cd sdk/go && go test -run 'TestRuntimeAbilityClientDispatchesProviderLifecycleSurfaces|TestRuntimeAbilityChildContextDispatchesWithParentReceiptCausality|TestRuntimeAbilityClientInvokesObjectResult' -count=1 .`
- `env PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/tests PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest -q -p no:cacheprovider sdk/python/tests/test_ability_invocation.py::AbilityInvocationClientTests::test_provider_lifecycle_surfaces_dispatch_stream_bidi_cancel_and_receipts sdk/python/tests/test_ability_invocation.py::AbilityInvocationClientTests::test_child_context_anchors_child_invocation_to_parent_receipt`
- `python sdk/conformance/rebuild_public_api_model.py --write`

Result: focused tests passed and the canonical public API/matrix model was
regenerated. Go and Python `ability_invocation_facade` now carry
`dispatch`, `stream_open`, `bidi_open`, `cancel`, and `terminal_receipt`
lifecycle evidence through provider-backed Runtime Core selectors. The cells
remain `provider-backed`, not `cutover-ready`, because `deadline`,
`restart_recover`, and `start` remain open.

## Ability Deadline Vector Iteration Evidence

- `cd sdk/go && go test -tags easynet_direct_runtime -run 'TestRuntimeAbilityClientDeadlineIsProviderOwned' -count=1 .`
- `env PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/tests PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest -q -p no:cacheprovider sdk/python/tests/test_direct_runtime.py::DirectRuntimeTests::test_runtime_ability_deadline_is_provider_owned`
- `python sdk/conformance/rebuild_public_api_model.py --write`

Result: focused direct-runtime tests passed and the canonical public API/matrix
model was regenerated. Go and Python `ability_invocation_facade` now carry
`deadline` lifecycle evidence through Runtime Core provider selectors. The
ability facade does not introduce a separate timeout field; it delegates to the
provider deadline owner and proves retry after timeout cleanup. The cells
remain `provider-backed`, not `cutover-ready`, because `restart_recover` and
`start` remain open.

## Ability Start Vector Iteration Evidence

- `cd sdk/go && go test -run 'TestNativeRuntimeHandleProvidesRuntimeAbilityFacade' -count=1 .`
- `env PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/tests PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest -q -p no:cacheprovider sdk/python/tests/test_environment.py::SdkEnvironmentTests::test_native_runtime_handle_provides_runtime_ability_facade`
- `env PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python/tests PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest -q -p no:cacheprovider sdk/python/tests/test_runtime_ability.py`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --validate-schema`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-ability-start-self-test`
- `PYTHONPATH=sdk/conformance python3 sdk/conformance/sdk_matrix.py --self-test --tmp /tmp/easynet-sdk-matrix-ability-start-self-test`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo fmt --all -- --check`
- `cargo check --lib --features axon-pb`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`

Result: focused tests, schema validation, matrix validation, canonical public
API validation, V2 gates, Rust formatting/checking, and architecture gates
passed. Go and Python `ability_invocation_facade` now carry `start` lifecycle
evidence by borrowing the generic ability facade from the Native Runtime
provider graph. The cells remain `provider-backed`, not `cutover-ready`,
because `restart_recover` remains open.

## SDK Directory Product DTO Removal Evidence

- `/Users/macbook.silan.tech/.local/bin/codegraph init .`
- `/Users/macbook.silan.tech/.local/bin/codegraph node session_authority_admits_subject --path .`
- `/Users/macbook.silan.tech/.local/bin/codegraph node src/daemon/federation/resolver.rs --path .`
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python PYTHONDONTWRITEBYTECODE=1 /opt/anaconda3/bin/python -m pytest sdk/python/tests/test_directory.py -q`
- `cd sdk/go && GOCACHE=/tmp/easynet-go-build-cache go test . -run 'Directory' -count=1`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-sdk-product-neutrality.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`
- `rg -n "DirectoryAgentSummary|DirectorySigningAuthority|ParseDirectoryEntry|parse_directory_entry|_directory_wire|directory_wire" sdk/go sdk/python/easynet_sdk sdk/node sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json -g '!**/__pycache__/**' -g '!**/.mypy_cache/**' -g '!**/node_modules/**' -g '!**/.venv/**'`

Result: focused Go/Python directory tests passed; SDK product-neutrality and
canonical-runtime-convergence-v2 passed. The final `rg` found no production SDK
or conformance manifest references to the removed product-owned Directory wire
DTOs. Full `cd sdk/go && go test ./...` remains blocked in this sandbox by
Unix socket bind denial in managed-signing tests; the affected Directory tests
were run directly and passed.

Generation note: `sdk/conformance/rebuild_public_api_model.py --write`
repeatedly failed because `xcrun swift-symbolgraph-extract` segfaulted while
rebuilding unchanged Swift inventory. Go/Python inventories were regenerated
directly, existing Swift inventory cache was reused, and the canonical public
API/matrix model was generated from that cache. The committed SPEC v2 gate then
validated the resulting manifests.

## Federation Advertise Abilities Catalog Authority Evidence

- `cargo fmt --check`
- `cargo test --lib daemon::invocation::dispatch::federation_wrappers --features axon-pb`
- `cargo test --lib advertise_abilities --features axon-pb`
- `git diff --check`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

Result: focused federation wrapper tests, formatting, diff hygiene, SPEC v2
gate, architecture gate, and codegraph freshness checks passed. The
`federation.advertise_abilities` handler now requires the daemon-owned
`AbilityCatalogStore` and only returns `ack=true` after the owner projection
read model accepts the publication. The missing-catalog false-success branch
was removed instead of kept as a compatibility fallback.

## FFI Remote Descriptor Static Catalog Removal Evidence

- `cargo fmt --check`
- `cargo test --lib runtime_descriptor_resolver --features axon-pb`
- `rg -n "runtime_system_descriptor_catalog_entries\\(callee_ura\\)|runtime_system_descriptor_catalog\\\"" src/ffi/invocation/mod.rs tools/scripts/check-canonical-runtime-convergence-v2.sh tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph status .`

Result: focused descriptor resolver tests passed. Remote descriptor resolution
no longer rebinding static daemon system catalog rows for arbitrary remote
owners; the only remaining `runtime_system_descriptor_catalog` source string
is in the negative resolver test that rejects the old behavior. Remote owner
descriptor resolution now fails through caller signer, route, or owner
read-model authority instead of returning a synthetic descriptor success.

## Federation Heartbeat Catalog Authority Evidence

- `cargo fmt --check`
- `cargo test --lib handle_heartbeat --features axon-pb`
- `cargo test --lib daemon::invocation::dispatch::federation_wrappers --features axon-pb`
- `rg -n "heartbeat.*no-op|legacy callers see|handle_heartbeat\\([^\\n]*None|handle_heartbeat\\([^\\n]*Some" src/daemon/invocation/dispatch/federation_wrappers.rs src/daemon/invocation/dispatch/unary_dispatcher.rs`
- `git diff --check`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-architecture-convergence.sh`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`

Result: focused heartbeat tests and the full federation wrapper test module
passed. The `federation.heartbeat` handler now requires the daemon-owned
`AbilityCatalogStore` and cannot report an active lease-refresh response when
the projection read-model sink is absent. Production dispatch already owns the
catalog, so the obsolete missing-catalog compatibility path was deleted instead
of preserved.

## Federation Resolve Catalog Authority Evidence

- `cargo check --lib --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `cargo test --lib daemon::invocation::dispatch::federation_wrappers --features axon-pb`
- `cargo test --lib daemon::invocation::routing::route_resolver --features axon-pb`
- `rg -n "catalog: Option<&crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore>|catalog: Option<&'a AbilityCatalogStore>|resolved_owner_projection_values\\(\\s*catalog: Option|handle_resolve\\([^\\n]*None|handle_resolve_at\\([^\\n]*None|handle_namespace_resolve_at\\([^\\n]*None|DaemonRouteResolver::new\\([^\\n]*None\\)" src/daemon/invocation src/daemon/ability/builtins/agents/discover.rs -g '*.rs'`

Result: Rust library check, formatting, diff hygiene, federation wrapper tests,
and route resolver tests passed. The final `rg` found no remaining
missing-catalog API surface for federation resolve, namespace resolve, owner
projection value resolution, or `DaemonRouteResolver::new`. Tests that model
"no published abilities" now pass an explicit empty `AbilityCatalogStore`,
separating an empty read model from a missing read-model authority.

## Federation Revoke Catalog Cleanup Authority Evidence

- `cargo test --lib handle_revoke --features axon-pb`
- `cargo test --lib daemon::invocation::dispatch::federation_wrappers --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `rg -n "handle_revoke\\([^\\n]*None|ability_catalog: Option|Some\\(&catalog\\)|, None,\\s*$" src/daemon/invocation/dispatch/federation_wrappers.rs src/daemon/invocation/dispatch/unary_dispatcher.rs src/daemon/federation/read_model/ability_catalog.rs`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-architecture-convergence.sh`

Result: focused revoke tests, the full federation wrapper test module,
formatting, diff hygiene, SPEC v2 gate, and architecture gate passed. The
negative `rg` check found no remaining missing-catalog `handle_revoke` API
surface. `federation.revoke` now requires the daemon-owned
`AbilityCatalogStore`: unfenced revoke removes the current owner projection,
and purge revoke removes only the matching generation so replay cannot leave a
stale route projection behind.

## Agent Registry Canonical Root Load Boundary Evidence

- `cargo test --lib daemon::persistence::agent_registry --features axon-pb`
- `cargo fmt --check`
- `git diff --check`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-architecture-convergence.sh`
- `GOCACHE=/tmp/easynet-go-build-cache bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_workspace_agent_directory_boundary.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`

Result: focused agent registry tests, formatting, diff hygiene, architecture
gate, SPEC v2 gate, workspace boundary self-test, and architecture gate
self-test passed. `load_agents()` no longer performs v1-to-v2 migration,
creates agent roots, writes `.v1.bak`, or repairs missing `root_path`. Loaded
registry rows must already carry the current schema stamp and canonical
`root_path`; retired pre-v2 rows fail closed instead of becoming implicit
runtime state mutations.
