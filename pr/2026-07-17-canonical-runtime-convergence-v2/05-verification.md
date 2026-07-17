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
`child_dispatch`, `dispatch`, `restart_recover`, and `start` are still open for
those capabilities.

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
