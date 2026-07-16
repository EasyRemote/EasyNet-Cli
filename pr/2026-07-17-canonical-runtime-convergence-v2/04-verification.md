# Verification Log

## 2026-07-17

- `cargo fmt --check`
- `cargo check --lib --bins`
- `cargo test --lib --no-run`
- `cargo test --tests --no-run`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo test --test script_checks canonical_runtime_convergence_v2_script_contract_holds`

Result: all passed.

## 2026-07-17 RF-6 Receipt Proof Facts

- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo test --test script_checks canonical_runtime_convergence_v2_script_contract_holds`
- `python3 -m py_compile sdk/python/easynet_axon/invocation/axiom.py sdk/python/easynet_axon/invocation/audit.py sdk/python/easynet_axon/invocation/fluent.py sdk/python/easynet_axon/invocation/local_runtime.py sdk/python/tests/test_audit.py sdk/python/tests/test_authority_idiomatic.py sdk/python/tests/test_authority_tail_parity.py sdk/python/tests/test_cross_language_verify.py`
- `cd sdk/java && mvn -q -DskipTests compile`
- `npm run check` from `sdk/node`
- `go test ./easynet/invocation -run 'TestAuthorityAnchor|TestReceiptSignVerifyRoundtrip|TestReceiptVerbs|TestRuntimeReceipt'` from `sdk/go`
- `swift test --package-path sdk/swift --filter AuthorityTailParityTests`
- `swift test --package-path sdk/swift --filter BundleUsageTests`

Result: focused RF-6 checks passed.

Known unrelated suite failures observed while broad-checking EasyNet-Axon:

- `go test ./easynet/invocation` failed in `TestGoBundleAcceptedByRustVerify`
  with `invocation_inv-a_descriptor_binding_required`.
- `swift test --package-path sdk/swift` failed in
  `CrossLanguageVerifyTests.testSwiftBundleAcceptedByRustVerify` with
  `invocation_inv-a_descriptor_binding_required`, and later in
  `MessageInboxIdempotentTests.test_dup_id_delivers_once`.
- `python3 -m pytest ...` could not run because the active Python interpreter
  does not have `pytest` installed.

## 2026-07-17 RF-9 Schema Source Derivation

- `bash /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/scripts/proto/sync_axon_v1.sh --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo test --test script_checks canonical_runtime_convergence_v2_script_contract_holds`

Result: passed. The V2 convergence gate now verifies Axon's canonical
`core/proto/axon/v1` proto source against its runtime client-sdk and Rust SDK
mirrors through Axon's own syncer.

## 2026-07-17 RF-9 Active URA Transport Classification

- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo test --test script_checks canonical_runtime_convergence_v2_script_contract_holds`

Result: passed. The V2 convergence gate now rejects semantic URI naming in
active CLI source/test/include roots while allowing HTTP/gRPC transport
library `Uri` and `.uri()` APIs.

## 2026-07-17 RF-1 Product Boundary Gates

- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/scripts/checks/product_protocol_boundary.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo test --test script_checks canonical_runtime_convergence_v2_script_contract_holds`

Result: passed. The V2 convergence runner now includes EasyNet-Cli runtime SDK
product-neutrality and Axon canonical proto/Rust product-protocol boundary
checks. Non-Rust Axon product SDK extraction remains open.

## 2026-07-17 RF-7/RF-8 Daemon Tuple Route Gate

- `bash tools/scripts/check-daemon-invocation-migration.sh`
- `bash tests/scripts/test_check_daemon_invocation_migration.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo test --test script_checks canonical_runtime_convergence_v2_script_contract_holds`
- `cargo test --test script_checks daemon_invocation_migration_script_contract_holds`

Result: passed. The canonical V2 runner now includes the daemon invocation
migration guard for complete tuple builder usage, JSON control demotion, and
runtime-record adapter boundaries.
