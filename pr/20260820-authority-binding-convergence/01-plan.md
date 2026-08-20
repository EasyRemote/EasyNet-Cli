# Authority binding convergence closure

Date: 2026-08-20

## Goal

Close the remaining runtime authority/descriptor seams across EasyNet-Cli daemon dispatch, SDK facades, and convergence gates.

## Boundary invariants

- User and Service remain principals; neither is modeled as an Agent merely because it owns account state.
- Device remains execution substrate; callable device-native abilities are exposed through SystemAgent/CallableActor surfaces.
- Ability invocation keeps the complete Axon tuple and resolves descriptors through callable owner projections, not device/socket/plugin identities.
- Runtime receipt authority binding uses the Axon relation/evidence model:
  - `self+identity`
  - `delegated_by+delegation`
  - `session_of+session`
  - `bootstrap`
- SDK receipt facades expose generic public fields (`authority_ura`, `issuer_ura`) and reject retired generated/product fields (`backend_ura`, `user_ura`, old `subject_ura` binding semantics).

## Implementation deltas

- Added the runtime catalogue read target schema fixture and schema binding.
- Updated daemon dispatch receipt authority projection to emit generic authority/issuer fields.
- Updated Go, Python, Node, Java, and Swift runtime/direct SDK receipt projection and canonical proof hash logic for the new Axon authority relation/evidence model.
- Regenerated Axon protobuf mirrors for Go and Python.
- Rebuilt SDK conformance/public API models after authority-binding API migration.
- Tightened convergence gates for callable owner kind projection, browser CDP descriptor count, package metadata, and authority binding facade consistency.
- Fixed daemon boot/admission seams found while running gates.

## RemoteApp/parallel dirty-file review

- Current EasyNet-Cli dirty files do not include RemoteApp implementation files.
- RemoteApp appears only in script_checks as existing boundary scripts; those scripts passed without requiring RemoteApp code edits.
- The parallel EasyNet-Axon dirty files are lifecycle conformance artifact refreshes plus the Rust lifecycle runner migration from the retired `AuthorityBinding::Self_` shape to `AuthorityOrBootstrap::Binding(AuthorityBinding { relation, evidence })`.
- That Axon change is protocol evidence-model convergence, not a RemoteApp feature change.

## Verification

- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-package-metadata.sh`
- `bash tools/scripts/check-sdk-package-metadata.sh --self-test`
- `bash tests/scripts/test_check_sdk_cutover_readiness.sh`
- `bash tests/scripts/test_check_browser_cdp_axon_boundary.sh`
- `bash tests/scripts/test_check_invocation_wire_entity_ref_kind_resolution_boundary.sh`
- `bash tests/scripts/test_check_sdk_scaffold.sh`
- `bash tests/scripts/test_check_voice_call_product_contract.sh`
- `cargo fmt --check`
- `cargo test -q --features axon-pb session_authority_summary_uses_public_generic_fields`
- `cargo test -q --test script_checks`
- `git diff --check`
- `go test ./...` in `sdk/go`
- `go test -tags runtime_direct ./...` in `sdk/go`
- `python -m pytest -q sdk/python/tests/test_runtime.py sdk/python/tests/test_direct_runtime.py`
- `node --test sdk/node/test/runtime-core.test.mjs`
- `mvn test` in `sdk/java`
- `swift test` in `sdk/swift`
