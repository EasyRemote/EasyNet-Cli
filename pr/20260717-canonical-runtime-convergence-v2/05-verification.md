# Canonical Runtime Convergence V2 - Verification Matrix

## Descriptor Projection Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo test -q daemon::ability::descriptors --lib --features axon-pb`:
  passed, 42 tests.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 26
  remaining pre-existing errors. The previous
  `src/daemon/ability/descriptors/mod.rs::governed_schema_summary`
  `too_many_arguments` finding is removed.

This evidence verifies only the descriptor projection slice. It does not prove
SPEC completion.

## Mission Terminal Transition Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo test -q daemon::execution::mission::orchestration --lib --features axon-pb`:
  passed, 23 tests.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 24
  remaining pre-existing errors. The two previous
  `src/daemon/execution/mission/orchestration.rs::MissionRunTerminalTransition::{completed,failed}`
  `too_many_arguments` findings are removed.

This evidence verifies only the Mission terminal transition slice. It does not
prove SPEC completion.

## Kernel Default Lifecycle Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo test -q daemon::boot::kernel --lib --features axon-pb`: passed,
  9 tests.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 23
  remaining pre-existing errors. The previous
  `src/daemon/boot/kernel/mod.rs::Kernel::new` `new_without_default` finding
  is removed.

This evidence verifies only the Kernel default lifecycle slice. It does not
prove SPEC completion.

## Bidi Event Payload Ownership Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo test -q daemon::invocation::bidi --lib --features axon-pb`: passed,
  84 tests.
- `cargo test -q daemon::invocation::dispatch::daemon_invocation_service::tests::bidi --lib --features axon-pb`:
  passed, 34 tests.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 20
  remaining pre-existing errors. The previous large-enum findings for
  `LocalBidiHandlerFrame`, `CarrierDispatchEvent`, and `DispatchStreamEvent`
  are removed.

This evidence verifies only the bidi event payload ownership slice. It does
not prove SPEC completion.

## Session Escalation Reply Ownership Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo test -q daemon::invocation::bidi::session_escalation --lib --features axon-pb`:
  passed, 9 tests.
- `cargo test -q daemon::invocation::dispatch::local_session_dispatcher::tests --lib --features axon-pb`:
  passed, 16 tests.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 18
  remaining pre-existing errors. The previous large-enum finding for
  `EscalationReply` and type-complexity finding for `SharedSessionOutbox`
  ready hooks are removed.

This evidence verifies only the session escalation reply ownership slice. It
does not prove SPEC completion.

## Dispatch Result Projection Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo test -q daemon::invocation::dispatch::local_session_dispatcher::tests --lib --features axon-pb`:
  passed, 16 tests.
- `cargo test -q daemon::axon_bridge::dispatch_shim --lib --features axon-pb`:
  passed, 10 tests.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. The previous
  `src/daemon/axon_bridge/dispatch_shim.rs::drain_to_outcome`
  `obfuscated_if_else` finding and two previous carrier result
  `needless_update` findings in
  `src/daemon/invocation/dispatch/local_session_dispatcher.rs` are removed.

This evidence verifies only the dispatch result projection slice. It does not
prove SPEC completion.

## Resolver Ingress Tuple Source Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: initially failed on formatting after the new
  negative test; `cargo fmt --all` was applied.
- `cargo test -q daemon::invocation::routing::target --lib --features axon-pb`:
  passed, 8 tests.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `target.rs` finding was reported.

This evidence verifies only the resolver ingress tuple-source slice. It does
not prove RF-8 or SPEC completion.

## Invocation Target Construction Boundary Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed after formatting the new constructor
  tests.
- `cargo test -q daemon::invocation::routing::target --lib --features axon-pb`:
  passed, 11 tests.
- `cargo test -q daemon::ability::builtins::agents::discover --lib --features axon-pb`:
  passed, 31 tests.
- `cargo test -q daemon::ability::builtins::agents::invoke --lib --features axon-pb`:
  passed, 30 tests.
- `cargo test -q daemon::ability::builtins::integrations --lib --features axon-pb`:
  passed, 101 tests.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No migrated file or `target.rs` constructor
  finding was reported.

This evidence verifies only the invocation target construction-boundary slice.
It does not prove RF-8/RF-7 or SPEC completion.

## Plugin Host Target Test Boundary Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo test -q daemon::plugins::host_api --lib --features axon-pb`: passed,
  7 tests.
- `cargo check --lib --features axon-pb`: passed.
- `rg -n "InvocationTarget \\{" src/daemon/plugins/host_api.rs`: no matches.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `host_api.rs` finding was reported.

This evidence verifies only the plugin host target test-boundary slice. It
does not prove RF-8/RF-7 or SPEC completion.

## Resource and Governance Target Boundary Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: initially required formatting after the
  governance health constructor migration; `cargo fmt --all` was applied.
- `cargo test -q daemon::ability::builtins::resources::pages::api --lib --features axon-pb`:
  passed, 2 tests.
- `cargo test -q daemon::ability::builtins::governance::health --lib --features axon-pb`:
  passed, 3 tests.
- `cargo check --lib --features axon-pb`: passed.
- `rg -n "InvocationTarget \\{|TargetScope|InvocationSubject|InvocationCausalContext" src/daemon/ability/builtins/resources/pages/api.rs src/daemon/ability/builtins/governance/health.rs`:
  no matches.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No migrated file finding was reported.

This evidence verifies only the resource/governance target-boundary slice. It
does not prove RF-8/RF-7 or SPEC completion.

## Media Subject Target Fixture Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: initially required formatting after the
  screen missing-subject constructor migration; `cargo fmt --all` was applied.
- `cargo test -q daemon::ability::builtins::resources::media::mic_subscribe --lib --features axon-pb`:
  passed, 9 tests.
- `cargo test -q daemon::ability::builtins::resources::media::screen_snapshot --lib --features axon-pb`:
  passed, 8 tests.
- `cargo check --lib --features axon-pb`: passed.
- `rg -n "InvocationTarget \\{|TargetScope|InvocationSubject|InvocationCausalContext" src/daemon/ability/builtins/resources/media/mic_subscribe.rs src/daemon/ability/builtins/resources/media/screen_snapshot.rs`:
  no matches.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No migrated media file finding was reported.

This evidence verifies only the media subject target-fixture slice. It does
not prove RF-8/RF-7 or SPEC completion.

## Camera Subject Target Fixture Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo test -q daemon::ability::builtins::resources::media::camera_snapshot --lib --features axon-pb`:
  passed, 13 tests.
- `cargo check --lib --features axon-pb`: passed.
- `rg -n "InvocationTarget \\{|TargetScope|InvocationSubject|InvocationCausalContext" src/daemon/ability/builtins/resources/media/camera_snapshot.rs`:
  no matches.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No migrated camera media finding was
  reported.

This evidence verifies only the camera subject target-fixture slice. It does
not prove RF-8/RF-7 or SPEC completion.

## LocalRuntime Subject Derivation Ownership Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: initially reported one formatting diff after
  adding `InvocationTarget::resolved_subject_ura`; `cargo fmt --all` was
  applied and the check then passed.
- `cargo test -q daemon::invocation::routing::target --lib --features axon-pb`:
  initially exposed an invalid hand-written hub URA in the new test, then
  passed, 14 tests, after switching the fixture to the canonical hub Ability
  URA builder.
- `cargo test -q daemon::invocation::dispatch::local_runtime_invoker --lib --features axon-pb`:
  passed, 4 tests.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `rg -n "LocalRuntimeSubjectPolicy|DescriptorDefault|descriptor default subject|AbilitySelector::parse\\(&target\\.ability\\)|target\\.causal_context\\.as_axon\\(\\)" src/daemon/invocation/dispatch/local_runtime_invoker.rs src/daemon/invocation/routing/target.rs`:
  only reported `AbilitySelector::parse(&target.ability)` in
  `local_runtime_invoker.rs`, which remains the callee-owner projection, not
  subject fallback ownership.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `target.rs` or `local_runtime_invoker.rs`
  finding was reported.

This evidence verifies only the LocalRuntime subject derivation ownership
slice. It does not prove RF-8/RF-7 or SPEC completion.

## Mission Catalog Gateway Target Boundary Slice

Commands run on 2026-07-17:

- `rg -n "InvocationTarget \\{|TargetScope|InvocationSubject|InvocationCausalContext" src/daemon/execution/mission/invocation_gateway.rs`:
  no matches.
- `cargo fmt --all -- --check`: passed.
- `cargo test -q daemon::execution::mission::invocation_gateway --lib --features axon-pb`:
  passed, 4 tests.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `invocation_gateway.rs` finding was
  reported.

This evidence verifies only the Mission catalog gateway target-boundary slice.
It does not prove RF-8/RF-7, RF-2, or SPEC completion.

## Ability Dispatch Target Fixture Boundary Slice

Commands run on 2026-07-17:

- `cargo fmt --all -- --check`: passed.
- `cargo test -q daemon::invocation::routing::target --lib --features axon-pb`:
  passed, 15 tests.
- `cargo test -q daemon::ability::dispatch::tests --lib --features axon-pb`:
  passed, 84 tests.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `rg -n "InvocationTarget \\{|TargetScope::Local|InvocationSubject|InvocationCausalContext" src/daemon/ability/dispatch.rs`:
  only reported the `ping_target_local` return type; no target literal or
  target enum construction remains in `dispatch.rs`.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `dispatch.rs` or `target.rs` finding was
  reported.

This evidence verifies only the ability dispatch target-fixture slice. It does
not prove RF-8/RF-7 or SPEC completion.

## LocalRuntime Invoker Target Fixture Boundary Slice

Commands run on 2026-07-17:

- `rg -n "InvocationTarget \\{|InvocationSubject|InvocationCausalContext" src/daemon/invocation/dispatch/local_runtime_invoker.rs`:
  only reported helper function signatures; no target literal or target enum
  construction remains in the file.
- `cargo fmt --all -- --check`: passed.
- `cargo test -q daemon::invocation::dispatch::local_runtime_invoker --lib --features axon-pb`:
  passed, 4 tests.
- `cargo test -q daemon::invocation::routing::target --lib --features axon-pb`:
  passed, 15 tests.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `local_runtime_invoker.rs` finding was
  reported.

This evidence verifies only the LocalRuntime invoker target-fixture slice. It
does not prove RF-8/RF-7 or SPEC completion.

## Builtins Smoke Target Fixture Boundary Slice

Commands run on 2026-07-17:

- `rg -n "InvocationTarget \\{|TargetScope|InvocationSubject|InvocationCausalContext" src/daemon/ability/builtins/real_invoke_tests.rs src/daemon/ability/catalog/assembly_tests.rs`:
  only reported helper function signatures in `real_invoke_tests.rs`; no
  target literal or target enum construction remains in the migrated files.
- `cargo fmt --all -- --check`: passed.
- `cargo test -q daemon::ability::catalog::assembly_tests --lib --features axon-pb`:
  passed, 38 tests.
- `cargo test -q daemon::ability::builtins::real_invoke_tests --lib --features axon-pb`:
  passed, 138 tests.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `real_invoke_tests.rs` or
  `assembly_tests.rs` finding was reported.

This evidence verifies only the builtins smoke target-fixture slice. It does
not prove RF-8/RF-7 or SPEC completion.

## CLI Agent Command Target Fixture Boundary Slice

Commands run on 2026-07-17:

- `rg -n "InvocationTarget \\{|TargetScope|InvocationSubject|InvocationCausalContext" src/cli/commands/agent/tests.rs`:
  no matches.
- `cargo fmt --all -- --check`: passed.
- `cargo test -q cli::commands::agent::tests --lib --features axon-pb`:
  passed, 51 tests.
- `cargo test -q cli::commands::agent --lib --features axon-pb`: passed,
  68 tests.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No `agent/tests.rs` finding was reported.

This evidence verifies only the CLI agent command target-fixture slice. It
does not prove RF-8/RF-7 or SPEC completion.

## Protobuf Transport Target Projection Boundary Slice

Commands run on 2026-07-17:

- `rg -n "target: Some\\(InvocationTarget \\{" src/daemon/invocation src/support/platform`:
  no matches.
- `rg -n "InvocationTarget" src/daemon/invocation/dispatch/request.rs src/daemon/invocation/bidi/session_initiator/envelope.rs src/support/platform/local_daemon_grpc.rs src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs src/daemon/invocation/dispatch/invocation_wire.rs`:
  only reported the new `wire_invocation_target` helper and the unrelated
  `LocalDaemonCanonicalInvocationTarget` domain struct.
- `cargo fmt --all -- --check`: initially reported one rustfmt line-wrap diff
  in `request.rs`; the line wrap was manually updated and the check then
  passed.
- `cargo test -q daemon::invocation::dispatch::invocation_wire::tests::wire_invocation_target_trims_and_rejects_empty_selector --lib --features axon-pb`:
  passed, 1 test.
- `cargo test -q extract_envelope_open_returns_inner_for_envelope_open_frame --lib --features axon-pb`:
  passed, 1 test.
- `cargo test -q remote_bidi_open_frame_is_canonical_and_fail_closed --lib --features axon-pb`:
  passed, 1 test.
- `cargo test -q daemon::invocation::dispatch::daemon_invocation_service_tests::bidi_open_forwards_to_local_dispatcher --lib --features axon-pb`:
  completed with 0 matched tests; this command is not counted as coverage.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  remaining pre-existing errors. No migrated transport target file finding was
  reported.

This evidence verifies only the protobuf transport target projection slice. It
does not prove RF-8/RF-7, receipt cutover, SDK convergence, or SPEC completion.

## RF-5 Rust Public Surface Signer Fallback Removal Slice

Commands run on 2026-07-17:

- `rg -n 'GeneratedSubjectAuth|generate_subject_auth|generate_private_agent_auth|generate_private_hub_auth' /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/src/invocation/runtime_admin.rs /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/src/invocation/mod.rs`:
  passed with no matches after removing the Rust public fallback root.
- `cargo fmt --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml -- --check`:
  passed.
- `cargo test -q --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml runtime_admin`:
  passed, 7 tests.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  regenerated public API artifacts using the baseline Python AST parser.
- Manifest inspection after regeneration:
  `GeneratedSubjectAuth`, `generate_subject_auth`,
  `generate_private_agent_auth`, `generate_private_hub_auth`, and matching
  `runtime_admin.*` exports are absent from
  `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json`.
- `/Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-self-test`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 pre-existing lint errors; no generated-auth or conformance/policy finding
  was reported.

This evidence verifies the RF-5 Rust public fallback root removal and
conformance enforcement. It does not prove cross-language signer-handle parity,
daemon KeyService cutover, or full RF-5 completion.

## RF-3 Public Plain Proof Helper Removal Slice

Commands run on 2026-07-17:

- `rg -n "pub use (admission|axiom)::\\{.*(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|run_admission|verify_signature)|\\\"(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|run_admission|verify_signature)\\\"" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/src/invocation/mod.rs /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python/easynet_axon/invocation/__init__.py`:
  no plain helper exports were reported.
- `rg -n "pub(\\([^)]*\\))? fn (canonical_invocation_bytes|sign_invocation|verify_invocation_signature|run_admission|verify_signature)" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/src/invocation/axiom.rs /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/src/invocation/admission.rs`:
  reported only `pub(crate)` test/internal functions, not public Rust API.
- `cargo fmt --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml -- --check`:
  passed.
- `cargo check --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml`:
  passed.
- `cargo test -q --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml runtime_admin`:
  passed, 7 tests.
- `cargo test -q --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml admission`:
  passed, 24 tests.
- `cargo test -q --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml axiom`:
  passed, 51 tests.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python /Users/macbook.silan.tech/.local/bin/python3.12 - <<'PY' ...`:
  passed; `canonical_invocation_bytes`, `sign_invocation`,
  `verify_invocation_signature`, `run_admission`, and `verify_signature` are
  absent from `easynet_axon.invocation`, while descriptor-bound replacements
  are present.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  regenerated public API artifacts from the current Axon checkout.
- Exact recursive JSON scan over
  `sdk/conformance/canonical-public-api.json` and
  `sdk/conformance/sdk-parity-matrix.json`: passed; the plain helper set is
  absent from canonical and non-canonical public graphs.
- `/Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/sdk_concepts.py --self-test --tmp /tmp/easynet-sdk-concepts-rf3-self-test`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 pre-existing lint errors; no RF-3 conformance or Axon public-surface
  finding was reported.

This evidence verifies the RF-3 public plain proof helper removal for the
Rust/Python Axon package roots and EasyNet-Cli public-surface gate. It does not
prove the remaining cross-language descriptor-bound vector/example audit or
full RF-3 completion.

## RF-3 Python Submodule Plain Proof Hardening Slice

Commands run on 2026-07-17:

- `rg --pcre2 -n "(?<!_)\\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|run_admission)\\b" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python/easynet_axon /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python/tests --glob '!**/__pycache__/**'`:
  no matches after renaming the Python plain helper group to private fixture
  names.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python /Users/macbook.silan.tech/.local/bin/python3.12 - <<'PY' ...`:
  passed; `easynet_axon.invocation` does not expose plain helper names and
  does expose descriptor-bound replacements.
- `uv run --project /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python pytest -q tests/test_admission.py tests/test_axiom_vectors.py`:
  passed, 24 tests and 3 skips.
- `cargo fmt --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml -- --check`:
  passed.
- `cargo check --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo fmt --all -- --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 pre-existing lint errors; no RF-3 Python hardening or V2 gate finding was
  reported.

This evidence verifies that Axon Python no longer exposes plain proof helpers
as normal submodule APIs and that the V2 gate now checks the source-level
boundary directly. It does not complete the remaining RF-3 vector/example audit
for all languages.

## RF-6 Java LocalRuntime Receipt Proof Facts Slice

Commands run on 2026-07-17:

- `rg -n "ReceiptProofFacts\\.empty\\(\\)|descriptorBoundBinding|LocalReceiptProofFacts|withOutputHash" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java/src/main/java/run/easynet/axon/invocation/LocalRuntime.java /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java/src/main/java/run/easynet/axon/invocation/InvocationReceipt.java /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java/src/main/java/run/easynet/axon/invocation/Axiom.java`:
  reported `descriptorBoundBinding`, `LocalReceiptProofFacts`, and
  `withOutputHash`; no `ReceiptProofFacts.empty()` match was reported in
  Java `LocalRuntime`.
- `mvn -q -Dtest=run.easynet.axon.invocation.InvokeSignedTest,run.easynet.axon.invocation.AdmissionTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed. The new assertions verify non-empty signed and system-local receipt
  proof facts and terminal output-hash projection.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- A broad manual `javac` compile attempt without Maven classpath failed because
  it pulled `PersistentLog.java`, which imports Gson. Maven is the correct Java
  SDK dependency context and passed the targeted tests above.
- After the Axon RF-6 Java commit, `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and updated `sdk/conformance/canonical-public-api.json` to Axon
  revision `057966488f62bae0b81b6f67cce4ada70a40cf1e`.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on 15
  existing lint errors in files outside this RF-6 Java/gate slice:
  `publish.rs`, `runtime.rs`, `lifecycle.rs`, `access_control.rs`,
  `catalog_metadata.rs`, `health.rs`, `route_resolver.rs`,
  `agent_lifecycle.rs`, and `config.rs`.

This evidence verifies the Java LocalRuntime empty proof-fact production-path
removal and the EasyNet-Cli V2 regression gate for that path. It does not
complete RF-6 for all SDK languages or prove descriptor proof-binding parity
with Rust.

## RF-6 Python LocalRuntime Receipt Proof Facts Slice

Commands run on 2026-07-17:

- `rg -n "proof_facts\\s*=\\s*ReceiptProofFacts\\(\\)|ReceiptProofFacts\\(\\)" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python/easynet_axon/invocation/local_runtime.py /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python/easynet_axon/invocation/handle.py /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python/easynet_axon/invocation/axiom.py`:
  no matches after moving Python LocalRuntime binding construction to
  `_LocalReceiptProofFacts`.
- `uv run --project /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python pytest -q tests/test_admission.py`:
  passed, 13 tests. The new assertions verify non-empty signed and
  system-local receipt proof facts and terminal output-hash projection.
- `PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python /Users/macbook.silan.tech/.local/bin/python3.12 - <<'PY' ...`:
  passed; a LocalRuntime invocation emitted 7 receipts, `verify_receipt_chain`
  accepted the chain, and the terminal receipt proof-fact output hash was
  non-zero.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- After the Axon RF-6 Python commit, `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and updated `sdk/conformance/canonical-public-api.json` to Axon
  revision `ba08ffcdbaa5ef56ed58003a4d8542163ac4464f`.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 existing lint errors in files outside this RF-6 Python/gate slice:
  `publish.rs`, `runtime.rs`, `lifecycle.rs`, `access_control.rs`,
  `catalog_metadata.rs`, `health.rs`, `route_resolver.rs`,
  `agent_lifecycle.rs`, and `config.rs`.
- `uv run --project /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python pytest -q tests/test_admission.py tests/industrial/test_audit_receipt_chain_verify.py tests/industrial/test_audit_tamper_detection.py tests/industrial/test_invocation_terminal_monotonicity.py tests/industrial/test_supervisor_cleanup_before_receipt.py`:
  failed 3 of 17 tests because those industrial tests call current-missing
  `LocalRuntime.core()` / `LocalRuntime.cancel()` APIs. This is not used as
  RF-6 evidence; the failure remains separate lifecycle/test API debt.

This evidence verifies the Python LocalRuntime empty proof-fact
production-path removal and the EasyNet-Cli V2 regression gate for that path.
It does not complete RF-6 for Go, Node, remaining examples/tests, or
descriptor proof-binding parity with Rust.

## RF-6 Go LocalRuntime Receipt Proof Facts Slice

Commands run on 2026-07-17:

- `rg -n "EmptyReceiptProofFacts\\(\\)|descriptorBoundBinding|localReceiptProofFacts|WithOutputHash" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go/easynet/invocation/local_runtime.go /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go/easynet/invocation/handle.go /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go/easynet/invocation/axiom.go`:
  reported `descriptorBoundBinding`, `localReceiptProofFacts*`, and
  `WithOutputHash`; no `EmptyReceiptProofFacts()` match was reported in Go
  `local_runtime.go`.
- `go test ./easynet/invocation -run 'TestInvokeSigned|TestAdmission' -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed. The new assertions verify non-empty signed and system-local receipt
  proof facts and terminal output-hash projection.
- `go test ./easynet/invocation -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed. This includes the Go-produced bundle accepted by the Rust verifier
  after migrating the fixture to descriptor-bound signing and
  digest/action-bound AbilityDescriptorRef values.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Go `LocalRuntime` fixture that calls
  `EmptyReceiptProofFacts()` and proves the gate rejects it.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `go test ./tests/industrial -run 'Test_audit_receipt_chain_verify|Test_audit_tamper_detection|Test_invocation_terminal_monotonicity|Test_supervisor_cleanup_before_receipt' -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  failed to compile at the RF-6 checkpoint because the industrial package
  referenced missing `LocalRuntime.CoreOf`, `LocalRuntime.Cancel`, and
  `LocalRuntime.SendMessage` public APIs. That lifecycle facade gap is handled
  by the RF-4 Go Runtime Lifecycle Facade slice below.
- After the Axon RF-6 Go commit, `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and updated `sdk/conformance/canonical-public-api.json` to Axon
  revision `f167a15444d8f5f744f9d87c25208e5ff4209c1d`.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 existing lint errors in files outside this RF-6 Go/gate slice:
  `publish.rs`, `runtime.rs`, `lifecycle.rs`, `access_control.rs`,
  `catalog_metadata.rs`, `health.rs`, `route_resolver.rs`,
  `agent_lifecycle.rs`, and `config.rs`.

This evidence verifies the Go LocalRuntime empty proof-fact production-path
removal, the Go descriptor-ref parser convergence required by Rust verifier
interoperability, and the EasyNet-Cli V2 regression gate for that path. It
does not complete RF-6 for Node, remaining examples/tests, or full descriptor
proof-binding parity with Rust.

## RF-4 Go Runtime Lifecycle Facade Slice

Commands run on 2026-07-17:

- `go test ./easynet/invocation -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./tests/industrial -run 'Test_audit_receipt_chain_verify|Test_audit_tamper_detection|Test_event_stream_sequence_monotonic|Test_invocation_parent_child_cancel_propagation|Test_message_inbox_fifo|Test_message_inbox_idempotent|Test_supervisor_cleanup_before_receipt|Test_invocation_terminal_monotonicity' -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./tests/industrial -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- After the Axon RF-4 Go lifecycle facade commit, `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and updated `sdk/conformance/canonical-public-api.json` to Axon
  revision `a1ff7f72a42b4738072233bf9b448adf8413bad0`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 existing lint errors in files outside this RF-4 Go/gate slice:
  `publish.rs`, `runtime.rs`, `lifecycle.rs`, `access_control.rs`,
  `catalog_metadata.rs`, `health.rs`, `route_resolver.rs`,
  `agent_lifecycle.rs`, and `config.rs`.

This evidence verifies the Go runtime-level lifecycle facade for audit
inspection, idempotent cancellation, parent-child cancellation propagation,
bounded FIFO/idempotent messaging, terminal monotonicity, event sequencing,
and cleanup-before-terminal-receipt behavior. It does not complete RF-4
globally because the shared transition-vector suite and cross-language
provider-backed/cutover-ready statuses remain open.

## RF-6 Node LocalRuntime Receipt Proof Facts Slice

Commands run on 2026-07-17:

- `npm run check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed.
- `npm run build`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed and produced no tracked generated-file drift.
- `node --test tests/invoke-signed.test.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 11 tests. The new assertions verify non-empty signed and
  system-local receipt proof facts and terminal output-hash projection.
- `npm run generated:check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed.
- `npm run verify`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, including TypeScript checks, generated verification, 14/14 axiom
  vectors, and protocol-pack vectors.
- `rg -n "EMPTY_RECEIPT_PROOF_FACTS" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node/src/invocation/local-runtime.ts /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node/src/invocation/handle.ts /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node/src/invocation/axiom.ts`:
  reported only the sentinel definition in `axiom.ts`, with no production
  runtime emission path using it.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Node `LocalRuntime` fixture that uses
  `EMPTY_RECEIPT_PROOF_FACTS` and proves the gate rejects it.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- After the Axon RF-6 Node commit, `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and updated `sdk/conformance/canonical-public-api.json` to Axon
  revision `1dad6b5994c4f17473d860f7cf1fb0d493c831b9`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `cargo fmt --all -- --check`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 existing lint errors in files outside this RF-6 Node/gate slice:
  `publish.rs`, `runtime.rs`, `lifecycle.rs`, `access_control.rs`,
  `catalog_metadata.rs`, `health.rs`, `route_resolver.rs`,
  `agent_lifecycle.rs`, and `config.rs`.

This evidence verifies the Node LocalRuntime empty proof-fact production-path
removal and the EasyNet-Cli V2 regression gate for that path. It does not
complete RF-6 for remaining examples/tests or full descriptor proof-binding
parity.

## RF-5 Rust Local-Fast Signer Feature Removal Slice

Commands run on 2026-07-17:

- `cargo fmt --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml -- --check`:
  passed.
- `cargo check --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml`:
  passed.
- `cargo check --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml --features proto`:
  passed.
- `cargo check --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml --examples`:
  passed.
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml --features proto --no-run`:
  passed and compiled all Rust SDK test targets.
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml --features proto --test signed_receipt_api_gate`:
  passed, 2 tests. The gate asserts `local-fast-probes` is absent from
  `Cargo.toml`, public feature cfg is absent from invocation sources, and
  local-fast helper declarations are `cfg(test)` only.
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml --features proto --test ability_context_child_rpc`:
  passed, 13 tests. These fixtures now use explicit test providers instead of
  SDK public fallback helpers.
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml --features proto --test ledger_sink_and_external_signed public_invocation_core_cannot_emit_terminal_receipt`:
  passed, 1 test.
- `cargo run --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/Cargo.toml --example receipt_closure`:
  passed. The example now constructs an explicit receipt signing authority
  provider.
- `rg -n 'local-fast-probes|feature = "local-fast-probes"' /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust -S`:
  reported only negative assertions in `signed_receipt_api_gate.rs`.
- `rg -n 'LocalReceiptSigningAuthorityProvider|new_local_fast|Ed25519InvocationSigningAuthority|StaticInvocationSigningAuthorityProvider|Ed25519ReceiptSigningAuthority|StaticReceiptSigningAuthorityProvider' /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/tests /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust/examples -S --glob '!signed_receipt_api_gate.rs'`:
  passed with no matches.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a failing Rust `local-fast-probes`
  fixture.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed against Axon revision
  `7f1113eada00a01a62c4b2d02892880dc1f37b31`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `7f1113eada00a01a62c4b2d02892880dc1f37b31`.
- `cargo check --bin real-user-smoke` from EasyNet-Cli:
  passed after removing the downstream feature requirement and migrating the
  binary to an explicit local smoke receipt provider.
- `cargo test --test pages_unit u14_pages_management_abilities_are_in_local_runtime --features axon-pb`:
  passed after migrating the Pages integration runtime fixture to an explicit
  Pages-owned test receipt provider.
- `cargo fmt --all -- --check` from EasyNet-Cli: passed.
- `cargo check --lib --features axon-pb` from EasyNet-Cli: passed.
- `cargo check --all-targets --features axon-pb` from EasyNet-Cli:
  passed after replacing CLI lib-test usage of Axon `cfg(test)` signer helper
  re-exports with explicit CLI test providers.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo test -q daemon::execution::mission::invocation_gateway --lib --features axon-pb`:
  passed, 4 tests.
- `cargo test -q support::platform::local_daemon_grpc --lib --features axon-pb`:
  passed, 10 tests.
- `rg -n 'local-fast-probes|new_local_fast|LocalReceiptSigningAuthorityProvider|Ed25519InvocationSigningAuthority|StaticInvocationSigningAuthorityProvider|Ed25519ReceiptSigningAuthority|StaticReceiptSigningAuthorityProvider' Cargo.toml src tests tools plugins -S --glob '!tools/scripts/check-canonical-runtime-convergence-v2.sh'`:
  passed with no matches after removing downstream feature/helper consumption.
- `cargo clippy --lib --features axon-pb -- -D warnings`: failed on the same
  15 existing lint errors outside this RF-5 local-fast slice. `cargo clippy
  --bin real-user-smoke -- -D warnings` also failed before bin-specific
  analysis because Cargo lints the same library first.

This evidence verifies removal of the Rust SDK public local-fast signer
fallback feature, migration of external Rust SDK tests/examples to explicit
providers, removal of EasyNet-Cli's downstream local-fast feature request,
and EasyNet-Cli V2 regression coverage for that boundary. It does not complete
RF-5 because signer-handle parity and daemon KeyService authority cutover
remain open.

## RF-5 Runtime Client Subject Auth Generator Removal Slice

Commands run on 2026-07-17:

- `cargo fmt --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/client-sdk/Cargo.toml -- --check`:
  passed.
- `cargo check --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/client-sdk/Cargo.toml`:
  passed.
- `cargo test --manifest-path /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/client-sdk/Cargo.toml`:
  passed, 58 unit tests plus 5 `invocation_envelope_interop` tests.
- `rg -n 'default_auth_for_subject|generate_subject_auth|generate_private_agent_auth|generate_private_hub_auth|GeneratedSubjectAuth|ProcessLocalSigner|PrivateKeyAuthenticator|DefaultAuthForSubject|GenerateSubjectAuth|defaultAuthForSubject' /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/client-sdk/src /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs/src -S --glob '!**/tests/**' --glob '!**/*_test.go' --glob '!**/*.test.*'`:
  passed with no matches.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes an Axon runtime client SDK fixture that
  reintroduces `generate_subject_auth`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed against Axon revision
  `29d6ec1b1adba9d1db0b2c2b9ff66626cb980b5d`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `29d6ec1b1adba9d1db0b2c2b9ff66626cb980b5d`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and updated `sdk/conformance/canonical-public-api.json` to the
  current Axon revision.

This evidence verifies removal of the runtime client SDK public subject auth
generator and source-level V2 regression coverage for process-local signer
fallback helpers. It does not complete RF-5 because host auth DTOs still need
cross-language signer-handle or daemon KeyService convergence.

## RF-3 Go Public Plain Proof Helper Removal Slice

Commands run on 2026-07-17:

- `go test ./easynet/invocation -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `rg -n '^func (CanonicalInvocationBytes|SignInvocation|VerifyInvocationSignature|VerifySignature|RunAdmission)\b|\b(CanonicalInvocationBytes|SignInvocation|VerifyInvocationSignature|VerifySignature|RunAdmission)\b' /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go/easynet/invocation /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/API_MAPPING.md -S`:
  passed with no matches.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Go invocation package fixture that
  reintroduces `CanonicalInvocationBytes`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and updated `sdk/conformance/canonical-public-api.json` to the
  current Axon revision.

This evidence verifies the Go public plain proof helper removal and V2
regression coverage for the Go surface. It does not complete RF-3 because
remaining language surfaces, examples, and vector documentation still require
audit.

## RF-3 Node Public Plain Proof Helper Removal Slice

Commands run on 2026-07-17:

- `npm run build`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed and regenerated Node JS/declaration outputs.
- `node ./scripts/run-axiom-vectors.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 14/14 vectors.
- `node --test tests/axiom-vectors.test.mjs tests/admission.test.mjs tests/cross-language-verify.test.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 38 passed, 3 skipped, 0 failed. The cross-language bundle test now
  signs descriptor-bound invocation bytes and is accepted by Rust
  `easynet-verify`.
- `rg -n '\b(canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' sdk/node/src sdk/node/tests sdk/node/scripts -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no old public helper names after the Node build.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Node invocation package fixture that
  reintroduces `canonicalInvocationBytes`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed; no additional public API manifest delta was required after Node type
  declarations removed the plain helper names.

This evidence verifies the Node public plain proof helper removal, descriptor
bound cross-language bundle production, and V2 regression coverage for the
Node surface. It does not complete RF-3 because remaining language surfaces,
package exports, examples, and vector documentation still require audit.

## RF-3 Java Public Plain Proof Helper Removal Slice

Commands run on 2026-07-17:

- `mvn -q -Dtest=run.easynet.axon.invocation.AxiomVectorsTest,run.easynet.axon.invocation.AdmissionTest,run.easynet.axon.invocation.CrossLanguageVerifyTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed. The Java cross-language bundle test now signs descriptor-bound
  invocation bytes and is accepted by Rust `easynet-verify` when the verifier
  binary is available.
- `mvn -q test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  failed in existing receipt anchor tests:
  `ReceiptAuthorityAnchorTest.receiptAnchorFormNone`,
  `receiptAnchorFormScalar`, `receiptAnchorFormList`,
  `receiptAnchorFormMerkle`, and `receiptAnchorHostedNone`.
  This RF-3 Java slice does not modify receipt canonical bytes; the failure is
  recorded as broader Java receipt parity debt, not proof of RF-3 regression.
- `rg -n 'public static [^{;=]+ (canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b|\b(canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' sdk/java/src/main/java/run/easynet/axon/invocation -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after Java production helper migration.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Java invocation package fixture that
  reintroduces public static `canonicalInvocationBytes`.

This evidence verifies the Java public plain proof helper removal, descriptor
bound cross-language bundle production, and V2 regression coverage for the
Java surface. It does not complete RF-3 because Swift and remaining
package/export/vector cleanup still require audit.

## RF-3 Swift Public Plain Proof Helper Removal Slice

Commands run on 2026-07-17:

- `swift test --filter EasyNetAxonTests.AxiomVectorsTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/swift`:
  passed, 2 tests.
- `swift test --filter EasyNetAxonTests.AdmissionTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/swift`:
  passed, 24 tests.
- `swift test --filter EasyNetAxonTests.CrossLanguageVerifyTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/swift`:
  passed, 3 tests. The Swift cross-language bundle now signs
  descriptor-bound invocation bytes and is accepted by Rust `easynet-verify`.
- `swift test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/swift`:
  passed, 145 tests.
- `rg -n 'public func (canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b|\b(canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' sdk/swift/Sources/EasyNetAxon/Invocation sdk/swift/README.md sdk/swift/Examples -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after Swift production helper and public example
  migration.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Swift invocation source fixture that
  reintroduces public `canonicalInvocationBytes`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `0d461370e7038575b99ac0327d798e8bfc165c04`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies the Swift public plain proof helper removal,
descriptor-bound cross-language bundle production, public example migration,
and V2 regression coverage for the Swift surface. It does not complete the
whole SPEC; RF-1 through RF-9 acceptance gates still require full closure.

## RF-3 Go Legacy Plain Fixture Naming Hardening Slice

Commands run on 2026-07-17:

- `rg -n '\b(canonicalInvocationBytes|signInvocation|verifyInvocationSignature|verifySignature|runAdmission)\b' sdk/go/easynet/invocation -S --glob '!**/*_test.go'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after the Go production source rename.
- `go test ./easynet/invocation`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./...`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Go production invocation fixture that
  reintroduces both exported and package-private retired plain helper names.
- `git diff --check`: passed in EasyNet-Axon before commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `a6a034c5db8b978c9a70eb61aa4305907a7a42ed`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `git diff --check`: passed in EasyNet-Cli.

This evidence verifies that Go production invocation source no longer
normalizes retired plain proof/admission helper names. It does not complete
RF-3 globally because final package/export/vector/example audit and legacy
implementation deletion remain separate closure work.

## RF-3 Rust Legacy Plain Fixture Naming Hardening Slice

Commands run on 2026-07-17:

- `rg -n '\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature|verify_phase|run_admission)\b' sdk/rust/src/invocation -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after the Rust invocation source rename.
- `cargo test -q invocation::axiom --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust`:
  passed, 50 tests.
- `cargo test -q invocation::admission --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust`:
  passed, 22 tests.
- `cargo test -q invocation::bundle --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust`:
  passed, 6 tests.
- `cargo test -q --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust`:
  passed, 222 tests.
- `cargo check --all-targets`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust`:
  passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/rust`:
  passed.
- `git diff --check` from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed before commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `b95ae265fa30ec3b04c98b6b23d05bc2c8043dd4`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Rust invocation source fixture that
  reintroduces a retired plain helper name.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `git diff --check`: passed in EasyNet-Cli.

This evidence verifies that Rust invocation source no longer normalizes
retired plain proof/admission helper names, while descriptor-bound proof and
admission helpers remain the canonical runtime boundary. It does not complete
RF-3 globally because final package/export/vector/example audit and legacy
implementation deletion remain separate closure work.

## RF-3 Python Legacy Plain Fixture Naming and Producer Hardening Slice

Commands run on 2026-07-17:

- `rg -n '\b(_canonical_invocation_bytes|_sign_invocation|_verify_invocation_signature|_verify_signature|_run_admission|canonical_invocation_bytes_empty)\b' sdk/python -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after the Python source and test rename.
- `python3 -m compileall -q sdk/python/easynet_axon/invocation sdk/python/tests/test_axiom_vectors.py sdk/python/tests/test_cross_language_verify.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed.
- `/Users/macbook.silan.tech/.local/bin/uv run --extra dev pytest tests/test_axiom_vectors.py -q`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python`:
  passed, 13 tests passed and 3 skipped.
- `/Users/macbook.silan.tech/.local/bin/uv run --extra dev pytest tests/test_cross_language_verify.py -q`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python`:
  passed, 3 tests. The Python producer is accepted by the Rust verifier with
  descriptor-bound invocation signatures.
- `/Users/macbook.silan.tech/.local/bin/uv run --extra dev pytest tests -q`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python`:
  failed with 17 failed, 221 passed, and 3 skipped. Failures are existing
  broader Python SDK parity debt in industrial LocalRuntime helper tests and
  authority/proof-fact parity tests, not this RF-3 Python proof-boundary slice.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Python private plain proof fixture that
  reintroduces `_canonical_invocation_bytes`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `468a230090f2921059dd89c3b1678000d2b4bc32`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies that Python SDK source no longer normalizes retired
plain proof/admission helper names and that the Python cross-language producer
uses descriptor-bound invocation signatures. It does not complete RF-3
globally because final package/export/vector/example audit and legacy
implementation deletion remain separate closure work.

## RF-3 Node Production Legacy Plain Export Removal Slice

Commands run on 2026-07-17:

- `rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission)\b|canonical_invocation_bytes_empty' sdk/node/src -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after deleting Node production legacy plain helpers.
- `npm run build`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed.
- `node --test tests/admission.test.mjs tests/axiom-vectors.test.mjs tests/cross-language-verify.test.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 38 tests passed and 3 skipped.
- `npm run axiom:vectors`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 14/14 vectors.
- `npm run verify`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Node production source fixture that
  reintroduces `legacyPlainInvocationBytes`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `dee7bea46fc262db30eb8639bb4b055f38f50473`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies that Node production invocation source no longer hosts
the legacy plain proof/admission implementation and that retained historical
plain vector coverage is isolated to an explicit fixture. It does not complete
RF-3 globally because final package/export/vector/example audit and legacy
implementation deletion remain separate closure work.

## RF-3 Go Production Legacy Plain Implementation Removal Slice

Commands run on 2026-07-17:

- `go test ./easynet/invocation`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./...`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `rg -n '\b(legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission)\b|legacy_plain_invocation_bytes_empty' sdk/go/easynet/invocation --glob '!**/*_test.go'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after deleting Go production legacy plain helpers.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed before commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `b56fea0140e1e4bd3fe0f4cb8f444625252b70b8`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a Go production invocation fixture that
  reintroduces `legacyPlainInvocationBytes`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `73c509a8562fef88c406b0a0470bf5168db1edc9`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies that Go production invocation source no longer hosts
the legacy plain proof/admission implementation and that retained historical
plain vector coverage is isolated to a test-only fixture. It does not complete
RF-3 globally because final package/export/vector/example audit and legacy
fixture closure remain separate work.

## RF-9 Protocol-Pack URA Vector Naming Slice

Commands run on 2026-07-17:

- `rg -n "easynet-uri-v1|input_uri|canonical_uri|URI canonicalization" packaging/protocol-pack document/plans/ecosystem/08-packaging-and-release-plan.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after the vector rename.
- `bash packaging/protocol-pack/build_protocol_pack.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed and created `dist/protocol-pack/easynet-protocol-pack-1.0.0.tar.gz`.
- `bash scripts/checks/protocol_pack_conformance_consumers.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes an Axon protocol-pack fixture that
  reintroduces `easynet-uri-v1.json`, `input_uri`, and `canonical_uri`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies the RF-9 protocol-pack conformance artifact migration
from URI naming to URA naming and adds regression coverage in the V2 gate. It
does not complete RF-9 globally because Axon normative documents, compatibility
wire naming, and generated-schema ownership remain separate closure work.

## RF-9 Active Invocation Normative Document URA Naming Slice

Commands run on 2026-07-17:

- `rg -n '\bURI \+ profile\b|\bURIs\b|\bURI\b|<uri>|\b(subject|caller|callee) URI\b|\b(caller|callee|subject|caller_binding|callee_binding|subject_binding)\.uri\b|\bstring uri\b|uri_profile|resolver\.resolve\(uri\)|canonical URI format' document/concepts/AXIOM.tex document/rfcs/001-envelope-axiom-alignment.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after the normative document migration.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes an Axon normative document fixture that
  reintroduces `string uri`, `URI + profile`, and `caller.uri`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies that the active invocation axiom documents now match
the URA-only schema and SDK vocabulary for identity composites, signing bytes,
admission replay keys, and resolver interfaces. It does not complete RF-9
globally because RFC-002/keyring terminology, historical documents, and
generated-schema ownership remain separate closure work.

## RF-9 Keyring Resolver URA Naming Slice

Commands run on 2026-07-17:

- `rg -n '\bURI \+ profile\b|\bURIs\b|\bURI\b|<uri>|\b(subject|caller|callee) URI\b|\b(caller|callee|subject|caller_binding|callee_binding|subject_binding)\.uri\b|\bstring uri\b|\bpeer_uri\b|find_peer_by_uri|uri_profile|resolver\.resolve\(uri\)|canonical URI format' document/concepts/AXIOM.tex document/rfcs/001-envelope-axiom-alignment.md document/rfcs/002-keyring-and-keyresolver.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after the RFC-002 keyring resolver migration.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes an RFC-002 fixture that reintroduces
  `string uri`, `peer_uri`, and `find_peer_by_uri`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies that RFC-002 keyring/keyresolver examples now use URA
vocabulary for identity fields, peer-table projections, and peer key lookup.
It does not complete RF-9 globally because historical document classification
and generated-schema ownership remain separate closure work.

## RF-1 React Tool Adapter Product Surface Removal Slice

Commands run on 2026-07-17:

- `rg -n '\b(tool_adapter|useAbilityTools|AbilityTool(Renderer|Invocation|Result|Options)?|AbilityTools)\b' sdk/react/src sdk/react/README.md sdk/react/SKILL.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after removing the React tool-adapter surface.
- `rg -n '\buri\b|\bURI\b|uris|URIs|easynet:///' sdk/react/README.md sdk/react/SKILL.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after replacing React public examples with
  `resourceUra` wording.
- `npm test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/react`:
  passed, 5 tests.
- `npm run types`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/react`:
  passed. `types:verify` now confirms `dist/types/tool_adapter.d.ts` is absent
  and `dist/types/index.d.ts` does not export `useAbilityTools`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a React SDK fixture that reintroduces
  `tool_adapter.ts` and `useAbilityTools`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed against Axon revision
  `745a9f442a94a4b832908df6ef0efce4a9ff6b37`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `745a9f442a94a4b832908df6ef0efce4a9ff6b37`.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies that the React SDK no longer publishes the
tool-adapter product surface and that the V2 RF-1 gate covers React
reintroduction. It does not complete RF-1 globally because package naming,
remaining product-flavoured docs, audio/MCP/preset inventory, and downstream
provider extraction remain separate work.

## RF-9 Active Proto URA Vocabulary Slice

Commands run on 2026-07-17:

- `rg -n '\bURI\b|\bURIs\b|<uri>|\b(canonical|device|agent|resource|subject|caller|callee|payload|receipt)[^[:cntrl:]]*\bURI\b|\bURI[^[:cntrl:]]*\b(canonical|device|agent|resource|subject|caller|callee|payload|receipt)\b|_[Uu][Rr][Ii]\b|\b[A-Za-z0-9]+URI\b' core/proto/axon/v1 core/runtime-rs/client-sdk/proto/axon/v1 sdk/rust/proto/axon/v1 --glob '*.proto'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after replacing the active proto comment with URA
  vocabulary.
- `bash scripts/proto/sync_axon_v1.sh --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, proving the runtime client-sdk and Rust SDK proto mirrors match the
  canonical source.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes a proto fixture that reintroduces
  `canonical device URIs`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `git diff --check`: passed in EasyNet-Cli and EasyNet-Axon.

This evidence verifies the RF-9 active proto terminology correction and the
new regression gate for Axon proto URI vocabulary. It does not complete RF-9
globally because historical document classification, broader active source
vocabulary, and final generated-schema ownership closure remain separate
work.

## Product-Neutral SDK URA Error Contract Slice

Commands run on 2026-07-17:

- `rg -n 'EasyNet URA|EasyNet URAs|EasyNet URA syntax|must be an EasyNet|must use EasyNet|SYSTEM_URI' sdk/go/easynet sdk/java/src/main/java sdk/node/src sdk/python/easynet_axon sdk/rust/src sdk/swift/Sources sdk/react/src --glob '!**/node_modules/**' --glob '!**/__pycache__/**'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after the SDK validation error contract update.
- `npm run verify`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed.
- `go test ./...`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `uv run --project sdk/python pytest -q sdk/python/tests/test_client.py sdk/python/tests/test_federation_conformance.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, 26 tests.
- `mvn -Dtest=SubjectUraTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed, 4 tests.
- `swift test --filter EasyNetAxonTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/swift`:
  passed, 126 tests.
- `cargo check --manifest-path sdk/rust/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed.
- `mvn test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  failed on five existing `ReceiptAuthorityAnchorTest` receipt-anchor drift
  assertions. `SubjectUraTest` passed; this failure remains RF-6/RF-3
  residual verification debt, not evidence of full Java SDK health.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now reintroduces Node `EasyNet URA` error text and
  Swift `SYSTEM_URI`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.

This evidence verifies the product-neutral SDK URA validation contract and the
new V2 regression gate. It does not complete RF-1 or RF-9 because package
renaming, broader docs/examples, historical classification, and remaining
product-owned SDK capabilities remain separate work.

## RF-6 Cross-Language Receipt Anchor Fixture Convergence Slice

Commands run on 2026-07-17:

- `cargo test --manifest-path sdk/rust/Cargo.toml strict_receipt_anchor_vectors_match_cross_language_pins`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, 1 test. This pins the Rust SDK strict proof-facts receipt anchors
  for none, scalar, list, merkle, and hosted receipt forms.
- `mvn -Dtest=ReceiptAuthorityAnchorTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed, 9 tests, after Java adopted the shared strict proof-facts fixture.
- `uv run --project sdk/python pytest -q sdk/python/tests/test_authority_tail_parity.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, 11 tests. Python now rejects missing receipt authority instead of
  silently defaulting it.
- `node --test tests/receipt-authority-anchor.test.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 9 tests.
- `swift test --filter AuthorityTailParityTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/swift`:
  passed, 7 tests.
- `cargo fmt --manifest-path sdk/rust/Cargo.toml -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed.
- `mvn test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed, 164 tests. This resolves the previously recorded Java full-suite
  failure in `ReceiptAuthorityAnchorTest`.
- `uv run --project sdk/python pytest -q sdk/python/tests/test_authority_tail_parity.py sdk/python/tests/test_cross_language_verify.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, 14 tests.
- `swift test --filter EasyNetAxonTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/swift`:
  passed, 126 tests.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed before this slice documentation update.

This evidence verifies only the cross-language receipt anchor fixture
convergence slice. It does not complete RF-6 globally because examples,
remaining empty-proof test fixtures, constructor hardening, and descriptor
proof-binding parity still require separate deletion/migration work.

## RF-6 Python Fluent Receipt Proof-Facts Boundary Slice

Commands run on 2026-07-17:

- `uv run --project sdk/python pytest -q sdk/python/tests/test_authority_idiomatic.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, 14 tests. This includes a negative test that
  `ReceiptSession(...).call(...)` rejects omitted proof facts.
- `uv run --project sdk/python pytest -q sdk/python/tests/test_authority_tail_parity.py sdk/python/tests/test_cross_language_verify.py sdk/python/tests/test_admission.py sdk/python/tests/test_audit.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, 35 tests.
- `uv run --project sdk/python python sdk/python/examples/authority_receipt.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed and completed the call -> receipt -> verify -> trace ->
  prove-authority flow with explicit descriptor/runtime proof facts.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` plus
  `sdk/conformance/sdk-parity-matrix.json` to Axon revision
  `2ff8120c76abe20ec0626bcd749b34b52d88b173`.
- `uv run --project sdk/python pytest -q sdk/python/tests --ignore=sdk/python/tests/industrial`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed, 219 tests and 3 skipped.
- `uv run --project sdk/python pytest -q sdk/python/tests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  failed, 231 passed, 3 skipped, and 8 failed. The failures are the existing
  industrial LocalRuntime helper/lifecycle gap: tests still call absent public
  APIs `LocalRuntime.core`, `children_of`, `send_message`, and `cancel`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `cargo fmt --all -- --check && cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.

This evidence verifies only the Python fluent receipt proof-facts boundary
slice. It does not complete RF-6 globally because other language examples,
remaining empty-proof fixtures, and public empty-proof helper exports still
need separate migration or removal.

## RF-6 Java Empty Receipt Proof Helper Removal Slice

Commands run on 2026-07-17:

- `rg -n "ReceiptProofFacts\\.empty\\(|static ReceiptProofFacts empty|new byte\\[32\\], new byte\\[32\\], \"\"" sdk/java/src/main/java sdk/java/src/test/java -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after removing the Java empty proof-facts helper and
  migrating remaining example/test call sites.
- `mvn -Dtest=ReceiptVerbsTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed, 11 tests.
- `mvn -q exec:java -Dexec.mainClass=run.easynet.axon.examples.ReceiptClosureExample`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed and completed the receipt -> verify -> trace -> prove-authority flow
  with explicit descriptor/runtime proof facts.
- `mvn test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed, 164 tests.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed before committing the Java slice.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `a4efe9cc29f60da3daa640dc05a6551b92a3942d`. The SDK parity matrix
  had no content change for this Java helper removal.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` and
  `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed in both repositories.
- `rg -n "ReceiptProofFacts\\.empty|static ReceiptProofFacts empty|ReceiptProofFacts empty" /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java/src /Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/conformance -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed with no matches.

This evidence verifies only the Java empty receipt proof helper removal and
the migration of Java receipt example/test call sites to explicit proof facts.
It does not complete RF-6 globally because constructor hardening, remaining
fixtures, and descriptor proof-binding parity still require final
cross-language closure.

## RF-6 Go Empty Receipt Proof Helper Removal Slice

Commands run on 2026-07-17:

- `rg -n "EmptyReceiptProofFacts\\(|ReceiptProofFacts\\(\\)|EMPTY_RECEIPT_PROOF_FACTS|ReceiptProofFacts\\.empty" sdk/go/easynet/invocation -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after removing the Go empty proof-facts helper and
  migrating Go receipt tests.
- `go test ./easynet/invocation -run 'TestVerb|TestAuthorityAnchor|TestReceiptSignVerifyRoundtrip' -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./easynet/invocation -count=1`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./...`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed, including `easynet`, `easynet/invocation`, and
  `tests/industrial`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed. The self-test now includes a Go production helper fixture that
  reintroduces `EmptyReceiptProofFacts()`.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed before committing the Go slice.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed and refreshed `sdk/conformance/canonical-public-api.json` to Axon
  revision `e2f41f12f1f325215364966c102cae24f87bb2a6`. The SDK parity matrix
  had no content change for this Go helper removal.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli` and
  `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed in both repositories.

This evidence verifies only the Go empty receipt proof helper removal and the
new regression gate for that helper. It does not complete RF-6 globally
because Swift empty proof helpers and final constructor/example parity remained
open before the next slice.

## RF-6 Swift Empty Receipt Proof Helper Removal Slice

Commands run on 2026-07-17:

- `rg -n 'public static let empty\s*=\s*ReceiptProofFacts|ReceiptProofFacts\.empty|proofFacts:\s*\.empty|proofFacts:\s*ReceiptProofFacts\s*=\s*\.empty|try\s+ReceiptProofFacts\(\s*\)|\?\? \.selfAuthority|authorityBinding: AuthorityBinding\? = nil' sdk/swift -S --glob '!**/.build/**'`:
  passed with no matches after removing Swift empty receipt proof helpers,
  empty constructor defaults, and fixture-level authority fallback.
- `swift test --filter InvokeSignedTests`: passed, 13 tests. The signed
  invocation LocalRuntime path now carries descriptor-bound proof facts and
  plain async LocalRuntime receipts carry system-local proof facts.
- `swift test --filter AuthorityMethodsTests`: passed, 8 tests. Receipt verb
  fixtures now pass explicit authority bindings and explicit proof facts.
- `swift test --filter BundleUsageTests`: passed, 4 tests. Bundle JSON
  fixtures now construct complete proof facts, and non-32-byte hash validation
  still rejects invalid facts.
- `swift test --filter CrossLanguageVerifyTests`: passed, 3 tests. Swift
  bundle receipts with explicit proof facts are accepted by the Rust verifier
  and rejected after receipt/usage tampering.
- `swift test`: failed after executing 147 tests with 2 unrelated industrial
  message-inbox failures:
  `MessageInboxFifoTests.test_fifo_order_preserved` observed `[1, 0, 2, ...]`
  instead of FIFO order, and
  `MessageInboxIdempotentTests.test_dup_id_delivers_once` observed `0`
  instead of `1`. Receipt, authority, bundle, cross-language, and signed
  invocation suites passed before these failures.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`:
  passed after the Swift public `ReceiptProofFacts` constructor surface
  changed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`:
  passed. The self-test now includes Swift fixtures for
  `ReceiptProofFacts.empty`, `proofFacts: .empty`, and empty
  `try ReceiptProofFacts()` construction.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`:
  passed against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo check --lib --features axon-pb`: passed.
- `git diff --check` in EasyNet-Axon: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`:
  passed.

This evidence verifies only the Swift empty receipt proof helper removal,
Swift LocalRuntime proof-fact ownership, and V2 regression coverage for this
slice. It does not complete RF-6 globally because final cross-language
constructor hardening and descriptor proof-binding parity still need a separate
closure pass.

## RF-6 Node Empty Receipt Proof Helper Removal Slice

Commands run on 2026-07-17:

- `rg -n "EMPTY_RECEIPT_PROOF_FACTS|proofFacts:\s*EMPTY|ReceiptProofFacts\.empty" sdk/node -S --glob '!**/node_modules/**'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed with no matches after deleting the Node empty proof-facts helper,
  exports, and obsolete excluded authority-anchor test.
- `npm run build`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed.
- `node --test tests/receipt-authority-anchor.test.mjs tests/cross-language-verify.test.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 11 tests. The active Node receipt-anchor suite still matches the
  shared `axon-receipt-anchor-v2` pins, and the Node bundle remains accepted
  by Rust `easynet-verify`.
- `npm run verify`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed. This includes TypeScript checking, generated type verification,
  generated artifact verification, axiom vectors, and protocol-pack vectors.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed after the Node public empty proof-facts export was removed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed. The self-test now reintroduces a Node
  `EMPTY_RECEIPT_PROOF_FACTS` helper and proves the RF-6 gate rejects it.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`:
  passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`:
  passed.

This evidence verifies only the Node empty receipt proof helper removal and
the new V2 regression coverage for that helper. It does not complete RF-6
globally because final constructor hardening, package/example audit, and
descriptor proof-binding parity still require a separate closure pass.

## RF-6 Python Receipt Proof Constructor Hardening Slice

Commands run on 2026-07-17:

- `uv run pytest tests/test_audit.py tests/test_authority_idiomatic.py tests/test_cross_language_verify.py tests/test_invocation_receipt_projection.py tests/test_authority_tail_parity.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python`:
  passed, 39 tests. Python receipt audit, fluent authority, cross-language
  verifier, projection, and shared authority-anchor fixtures all construct
  receipt proof facts explicitly.
- `rg -n "ReceiptProofFacts\\(\\)" sdk/python -S --glob '!**/__pycache__/**'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no
  matches.
- `/Users/macbook.silan.tech/.local/bin/python3.12 -m compileall -q sdk/python/easynet_axon sdk/python/tests/test_audit.py sdk/python/tests/test_authority_idiomatic.py sdk/python/tests/test_cross_language_verify.py sdk/python/tests/test_invocation_receipt_projection.py sdk/python/tests/test_authority_tail_parity.py sdk/python/examples/authority_receipt.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `PYTHONPATH=sdk/python /Users/macbook.silan.tech/.local/bin/python3.12 -`
  with a direct `ReceiptProofFacts()` construction probe from
  `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed by
  raising `TypeError` and printing `empty-rejected`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the Python constructor signature changed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects empty `ReceiptProofFacts()` into a Python test helper
  and proves the AST gate rejects it.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Python receipt proof constructor hardening and the
new V2 regression coverage for Python empty `ReceiptProofFacts()` calls. It
does not complete RF-6 globally because remaining language constructor
hardening, package/example audit, and descriptor proof-binding parity still
require separate closure.

## RF-6 Rust Receipt Proof Default Constructor Removal Slice

Commands run on 2026-07-17:

- `cargo fmt --manifest-path sdk/rust/Cargo.toml -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed
  after `cargo fmt --manifest-path sdk/rust/Cargo.toml` was applied to the
  Rust SDK edits.
- `cargo check --manifest-path sdk/rust/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo test --manifest-path sdk/rust/Cargo.toml receipt_proof_normalization`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  6 tests. This includes rejection of caller-supplied proof facts missing
  descriptor version or subject ref.
- `cargo test --manifest-path sdk/rust/Cargo.toml invocation::axiom::tests::receipt_proof_facts`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  2 tests.
- `cargo test --manifest-path sdk/rust/Cargo.toml invocation::audit::tests::proof_fact_bound_predicates_treat_zero_hash_as_unbound`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  1 test.
- `cargo test --manifest-path sdk/rust/Cargo.toml --features proto --test ledger_sink_and_external_signed public_invocation_core_cannot_emit_terminal_receipt`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  1 test. This preserves the public `InvocationCore::new()` fail-closed
  receipt-authority behavior after it stopped using default proof facts.
- `rg -n "ReceiptProofFacts::default\\(|proof_facts:\\s*Default::default\\(|#\\[derive\\([^\\]]*Default[^\\]]*\\)\\]\\s*pub struct ReceiptProofFacts" sdk/rust/src/invocation sdk/rust/tests -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no
  receipt-proof default matches. The broader command output only contained
  unrelated `unwrap_or_default()` uses in other domains.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the Rust `ReceiptProofFacts` public constructor surface changed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Rust `ReceiptProofFacts::default()`,
  `proof_facts: Default::default()`, and `Default` derive fixtures and proves
  the RF-6 gate rejects them.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Rust receipt proof default constructor removal,
stricter descriptor-bound proof-fact normalization, and the new V2 regression
coverage for Rust default proof-facts construction. It does not complete RF-6
globally because authority-proof default semantics, package/example audit, and
descriptor proof-binding parity still require separate closure.

## RF-6 Python Authority Proof Constructor Hardening Slice

Commands run on 2026-07-17:

- `uv run pytest tests/test_audit.py tests/test_authority_idiomatic.py tests/test_cross_language_verify.py tests/test_invocation_receipt_projection.py tests/test_authority_tail_parity.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python`:
  passed, 39 tests. Python receipt audit, fluent authority, cross-language
  verifier, projection, and shared authority-anchor fixtures all construct
  authority proof facts explicitly.
- `/Users/macbook.silan.tech/.local/bin/python3.12 -m compileall -q sdk/python/easynet_axon sdk/python/tests/test_audit.py sdk/python/tests/test_authority_idiomatic.py sdk/python/tests/test_cross_language_verify.py sdk/python/tests/test_invocation_receipt_projection.py sdk/python/tests/test_authority_tail_parity.py sdk/python/examples/authority_receipt.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `PYTHONPATH=sdk/python /Users/macbook.silan.tech/.local/bin/python3.12 -`
  with a direct `InvocationAuthorityProof()` construction probe from
  `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed by
  raising `TypeError` and printing `authority-empty-rejected`.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the manifest source revision was refreshed to the Axon authority-proof
  hardening commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Python authority-proof dataclass defaults and a
  partial `InvocationAuthorityProof(...)` call and proves the RF-6 gate
  rejects both.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Python authority-proof constructor hardening and
the new V2 regression coverage for Python omitted authority proof fields. It
does not complete RF-6 globally because Node/Java/Swift empty authority
helpers, Go/Rust authority-proof zero-struct audits, and descriptor
proof-binding parity still require separate closure.

## RF-6 Node Empty Authority Proof Helper Removal Slice

Commands run on 2026-07-17:

- `npm run build`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed after deleting the Node empty authority-proof export.
- `rg -n "EMPTY_AUTHORITY_PROOF" sdk/node -S --glob '!**/node_modules/**'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no
  matches after renaming the local receipt-authority anchor fixture.
- `node --test tests/receipt-authority-anchor.test.mjs tests/cross-language-verify.test.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 11 tests. The active Node receipt-anchor suite still matches the
  shared pins, and the Node bundle remains accepted by Rust `easynet-verify`.
- `npm run verify`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed. This includes TypeScript checking, generated type verification,
  generated artifact verification, axiom vectors, and protocol-pack vectors.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the manifest source revision was refreshed to the Axon Node empty authority
  proof removal commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now reintroduces a Node `EMPTY_AUTHORITY_PROOF` helper and proves
  the RF-6 gate rejects it.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Node empty authority-proof helper removal and the
new V2 regression coverage for that helper. It does not complete RF-6
globally because Java/Swift empty authority helpers, Go/Rust authority-proof
zero-struct audits, and descriptor proof-binding parity still require
separate closure.

## RF-6 Java Empty Authority Proof Helper Removal Slice

Commands run on 2026-07-17:

- `mvn -q -Dtest=run.easynet.axon.invocation.ReceiptAuthorityAnchorTest,run.easynet.axon.invocation.ReceiptVerbsTest,run.easynet.axon.invocation.CrossLanguageVerifyTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed. Java receipt authority anchors, receipt verbs, and cross-language
  verifier fixtures now use explicit authority-proof fixtures instead of the
  removed public empty helper.
- `rg -n "InvocationAuthorityProof\\.empty\\(" sdk/java -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after deleting the Java helper and migrating examples/tests.
- `mvn -q test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the manifest source revision was refreshed to the Axon Java empty authority
  proof removal commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: initially
  exposed that the Java helper gate caught call sites but not the nested
  factory declaration; after widening the pattern to reject
  `static InvocationAuthorityProof empty(...)`, passed.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Java empty authority-proof helper removal and the
new V2 regression coverage for that helper. It does not complete RF-6
globally because Swift empty authority helper removal, Go/Rust
authority-proof zero-struct audits, final cross-language constructor
hardening, and descriptor proof-binding parity still require separate
closure.

## RF-6 Swift Authority Proof Constructor Hardening Slice

Commands run on 2026-07-17:

- `swift test --package-path sdk/swift --filter AuthorityTailParityTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 7
  tests. The shared authority-anchor suite now uses a test-local explicit
  authority-proof fixture instead of `.empty`.
- `rg -n "InvocationAuthorityProof\\(|InvocationAuthorityProof\\.empty|authorityProof: \\.empty|public static let empty\\s*=\\s*InvocationAuthorityProof|proofHashUnchecked|proofPayload: Data =|signature: CalleeSignature\\? =|proofType: String =|binding: AuthorityBinding\\? = nil|admissionHook: String =" sdk/swift/Sources sdk/swift/Tests sdk/swift/Examples -S --glob '!**/.build/**'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: only
  explicit `InvocationAuthorityProof(...)` construction sites remain; no
  empty helper, `.empty` usage, unchecked initializer, or defaulted authority
  proof parameter matched.
- `swift test --package-path sdk/swift --filter MessageInboxIdempotentTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 1
  test, after the full-suite run exposed the residual inbox ordering failure.
- `swift test --package-path sdk/swift --filter 'AuthorityMethodsTests|AuthorityTailParityTests|BundleUsageTests|CrossLanguageVerifyTests|InvokeSignedTests'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 35
  tests. This covers the Swift authority-proof construction sites touched by
  this slice.
- `swift test --package-path sdk/swift`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: failed on
  two attempts with the same residual failure:
  `IndustrialTests.MessageInboxIdempotentTests.test_dup_id_delivers_once`
  expected counter `1` but observed `0`. The same test passed in isolation;
  this is not counted as full Swift suite success.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the manifest source revision was refreshed to the Axon Swift authority-proof
  hardening commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Swift empty authority-proof and defaulted initializer
  fixtures and proves the RF-6 gate rejects both.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Swift authority-proof constructor hardening and
the new V2 regression coverage for Swift omitted authority proof fields. It
does not complete RF-6 globally because Go/Rust authority-proof zero-struct
audits, final cross-language constructor hardening, and descriptor
proof-binding parity still require separate closure. The full Swift suite
also retains the `MessageInboxIdempotentTests` order-sensitive failure noted
above.

## RF-6 Go Zero Authority Proof Fixture Removal Slice

Commands run on 2026-07-17:

- `go test ./easynet/invocation -run 'TestAuthorityAnchor|TestCrossLanguage|TestReceiptProofFacts|TestReceiptVerbs'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed. Go receipt authority anchors and cross-language verifier fixtures
  now use explicit `anchorAuthorityProof()` instead of bare
  `InvocationAuthorityProof{}`.
- `rg -n "InvocationAuthorityProof\\{\\}" sdk/go/easynet/invocation -S --glob '!bundle.go'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no
  matches after migrating the Go anchor fixtures.
- `gofmt -w sdk/go/easynet/invocation/authority_anchor_test.go sdk/go/easynet/invocation/cross_language_verify_test.go`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: applied
  with no semantic changes beyond formatting the edited Go files.
- `go test ./...`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the manifest source revision was refreshed to the Axon Go explicit
  authority-anchor commit.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects a Go `InvocationAuthorityProof{}` fixture and proves
  the RF-6 gate rejects it.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check -- sdk/go/easynet/invocation/authority_anchor_test.go sdk/go/easynet/invocation/cross_language_verify_test.go`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Go zero authority-proof fixture removal and the
new V2 regression coverage for Go bare authority-proof structs. It does not
complete RF-6 globally because Rust authority-proof zero-struct audit, final
cross-language constructor hardening, and descriptor proof-binding parity
still require separate closure.

## RF-6 Rust Authority Proof Default Removal Slice

Commands run on 2026-07-17:

- `cargo test --manifest-path sdk/rust/Cargo.toml --features proto --test verify_binary --test verify_end_to_end --no-run`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: initially
  failed because `verify_binary.rs` and `verify_end_to_end.rs` still used
  `ReceiptProofFacts { ..Default::default() }`; after migrating those tests
  to `ReceiptProofFacts::new(...)`, passed.
- `cargo fmt --manifest-path sdk/rust/Cargo.toml -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: initially
  required formatting in `local_runtime/binding.rs`; after
  `cargo fmt --manifest-path sdk/rust/Cargo.toml`, passed.
- `rg -n "#\\[derive\\([^\\]]*Default[^\\]]*\\)\\]\\s*pub struct InvocationAuthorityProof|(^|[^:])InvocationAuthorityProof::default\\(|\\.\\.InvocationAuthorityProof::default\\(|InvocationAuthorityProof\\s*\\{[^}]*\\.\\.Default::default\\(|ReceiptProofFacts\\s*\\{[^}]*\\.\\.Default::default\\(" sdk/rust/src/invocation sdk/rust/tests -U`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after removing the Rust authority/proof default construction paths.
- `cargo test --manifest-path sdk/rust/Cargo.toml invocation::axiom::tests::authority_proof --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 2
  tests.
- `cargo test --manifest-path sdk/rust/Cargo.toml receipt_proof_normalization --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 6
  tests.
- `cargo test --manifest-path sdk/rust/Cargo.toml --features proto --test verify_binary --test verify_end_to_end`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 34
  tests.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Rust authority-proof default call and Default derive
  fixtures and proves the RF-6 gate rejects them.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `cargo check --manifest-path sdk/rust/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 /Users/macbook.silan.tech/.local/bin/python3.12 sdk/conformance/rebuild_public_api_model.py --write`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed after
  the manifest source revision was refreshed to the Axon Rust authority-proof
  default removal commit.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Rust authority/proof default removal and the new
V2 regression coverage for Rust omitted authority proof defaults. It does not
complete RF-6 globally because final cross-language constructor hardening,
remaining package/example audit, and descriptor proof-binding parity still
require separate closure.

## RF-6/RF-3 Runtime Client Receipt Proof Adapter Hardening Slice

Commands run on 2026-07-17:

- `cargo check --manifest-path core/runtime-rs/client-sdk/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: initially
  failed because the runtime client adapter still called
  `InvocationAuthorityProof::default()` after the canonical Rust SDK removed
  that constructor; passed after making authority proof required transport
  data.
- `cargo fmt --manifest-path core/runtime-rs/client-sdk/Cargo.toml -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo fmt --manifest-path core/runtime-rs/Cargo.toml -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo fmt --manifest-path core/runtime-rs/easynet-verify/Cargo.toml -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo check --manifest-path core/runtime-rs/client-sdk/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo check --manifest-path core/runtime-rs/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo check --manifest-path core/runtime-rs/easynet-verify/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo test --manifest-path core/runtime-rs/client-sdk/Cargo.toml domain::admission::tests --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 4
  tests.
- `cargo test --manifest-path core/runtime-rs/easynet-verify/Cargo.toml --test e2e_receipt_verify --no-run`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo test --manifest-path core/runtime-rs/easynet-verify/Cargo.toml --test e2e_receipt_verify`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: initially
  exposed stale verifier error projection after replacing the removed
  `AxonError.detail` field; after projecting canonical signature failures back
  to stable `AXON_CALLER_SIGNATURE_INVALID`, passed, 23 tests.
- `rg -n "#\\[derive\\([^\\]]*Default[^\\]]*\\)\\]\\s*pub struct ReceiptProofFacts|authority_proof:\\s*Option<|InvocationAuthorityProof::default\\(" core/runtime-rs/client-sdk/src/domain/admission.rs -U`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects a runtime client receipt proof adapter with
  `ReceiptProofFacts: Default`, optional authority proof, and
  `InvocationAuthorityProof::default()` and proves the RF-6 gate rejects it.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.

This evidence verifies only the runtime client receipt proof adapter
hardening and the new V2 regression coverage for that duplicate Rust proof
DTO. It does not complete RF-3 globally because descriptor-bound-only public
proof still needs remaining package/vector/example audit, and it does not
complete RF-6 globally because final cross-language constructor/package/vector
closure remains open.

## RF-3 Rust Legacy Plain Proof Implementation Removal Slice

Commands run on 2026-07-17:

- `rg -n "legacy_plain_invocation_bytes|sign_legacy_plain_invocation|verify_legacy_plain_invocation_signature|verify_phase_legacy_plain|run_legacy_plain_admission|verify_legacy_plain_signature|legacyPlainInvocation" sdk/rust/src/invocation sdk/rust/tests sdk/rust/examples -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after deleting the Rust private legacy plain proof/admission path.
- `cargo fmt --manifest-path sdk/rust/Cargo.toml -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo test --manifest-path sdk/rust/Cargo.toml invocation::axiom::tests --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: initially
  failed because legacy arbitrary subject/callee fixture URAs were invalid
  under descriptor-bound validation; after replacing them with valid runtime
  URAs, passed, 50 tests.
- `cargo check --manifest-path sdk/rust/Cargo.toml`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `cargo test --manifest-path sdk/rust/Cargo.toml invocation::admission::tests --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 22
  tests.
- `cargo test --manifest-path sdk/rust/Cargo.toml invocation::bundle::tests --lib`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 6
  tests.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Rust `legacy_plain_invocation_bytes` and
  `run_legacy_plain_admission` fixtures and proves the RF-3 gate rejects
  them.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Rust private legacy plain proof/admission
implementation removal and the new V2 regression coverage for those helper
names. It does not complete RF-3 globally because the remaining SDK
package/vector/example audit and descriptor-bound-only proof parity still
require separate closure.

## RF-3 Java Legacy Plain Proof Implementation Removal Slice

Commands run on 2026-07-17:

- `rg -n "legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission|verifyPhaseLegacyPlain|canonical_invocation_bytes_empty" sdk/java/src/main/java/run/easynet/axon/invocation sdk/java/src/test/java/run/easynet/axon/invocation -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after deleting the Java production legacy plain proof/admission path and
  migrating Java invocation tests.
- `mvn -q -Dtest=run.easynet.axon.invocation.AdmissionTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  initially failed because one negative admission fixture still used the old
  plain ability string `echo`; after replacing it with a valid descriptor ref
  so the test remained focused on caller-envelope validation, passed.
- `mvn -q -Dtest=run.easynet.axon.invocation.AxiomVectorsTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed after migrating vector assertions to descriptor-bound canonical bytes,
  signing, and verification.
- `mvn -q -Dtest=run.easynet.axon.invocation.AxiomWorkedExampleTest,run.easynet.axon.invocation.InvokeSignedTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed.
- `mvn -q test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Java `legacyPlainInvocationBytes` and proves the RF-3
  gate rejects package-private production legacy plain helpers.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Java production legacy plain proof/admission
implementation removal and the new V2 regression coverage for those helper
names. It does not complete RF-3 globally because remaining package,
historical-vector, script, and example audits still require separate closure.

## RF-3 Python Legacy Plain Proof Implementation Removal Slice

Commands run on 2026-07-17:

- `rg -n "legacy_plain|canonical_invocation_bytes_empty|_run_legacy_plain_admission|_verify_legacy_plain_signature|_sign_legacy_plain_invocation|_verify_legacy_plain_invocation_signature|_legacy_plain_invocation_bytes" sdk/python/easynet_axon/invocation sdk/python/tests sdk/python/examples -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after deleting the Python production legacy plain proof/admission path and
  migrating Python invocation tests.
- `PYTHONPATH=sdk/python pytest -q sdk/python/tests/test_axiom_vectors.py sdk/python/tests/test_admission.py sdk/python/tests/test_axiom_worked_example.py sdk/python/tests/test_invoke_signed.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  41 passed and 3 skipped.
- `PYTHONPATH=sdk/python pytest -q sdk/python/tests/test_cross_language_verify.py sdk/python/tests/test_audit.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  11 tests.
- `python3 -m compileall -q sdk/python/easynet_axon/invocation`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `PYTHONPATH=sdk/python pytest -q sdk/python/tests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: failed with
  231 passed, 3 skipped, and 8 failed. The failures are existing industrial
  lifecycle facade tests requiring public `LocalRuntime.core`,
  `children_of`, `send_message`, and `cancel` methods; they are RF-4 lifecycle
  facade debt and not part of this RF-3 proof-boundary migration.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Python `_legacy_plain_invocation_bytes` and
  `_run_legacy_plain_admission` fixtures and proves the RF-3 gate rejects
  them.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check && cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.

This evidence verifies only Python production legacy plain proof/admission
implementation removal and the new V2 regression coverage for those helper
names. It does not complete RF-3 globally because remaining package, script,
historical-vector, and example audits still require separate closure.

## RF-3 Swift Legacy Plain Proof Implementation Removal Slice

Commands run on 2026-07-17:

- `rg -n "legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission|verifyPhaseLegacyPlain|legacy_plain_invocation_bytes_empty|canonical_invocation_bytes_empty" sdk/swift/Sources sdk/swift/Tests -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after deleting the Swift production legacy plain proof/admission path and
  migrating Swift invocation tests.
- `swift test --package-path sdk/swift --filter AdmissionTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  24 tests.
- `swift test --package-path sdk/swift --filter AxiomVectorsTests`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed,
  2 tests.
- `swift test --package-path sdk/swift`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: failed with
  146 passed and 1 failed. The remaining failure is
  `IndustrialTests.MessageInboxIdempotentTests.test_dup_id_delivers_once`,
  which observed `0` deliveries instead of `1`; this is existing RF-4
  lifecycle/message-inbox debt, not an RF-3 proof-boundary failure.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Swift `legacyPlainInvocationBytes` and proves the
  RF-3 gate rejects production legacy plain helpers.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Swift production legacy plain proof/admission
implementation removal and the new V2 regression coverage for those helper
names. It does not complete RF-3 globally because remaining package, script,
historical-vector, and example audits still require separate closure.

## RF-3 Go Legacy Plain Proof Test Fixture Removal Slice

Commands run on 2026-07-17:

- `rg -n "legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|verifyLegacyPlainSignature|runLegacyPlainAdmission|legacy_plain_invocation_bytes_empty" sdk/go/easynet/invocation -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after deleting `legacy_plain_fixtures_test.go` and migrating Go invocation
  tests to descriptor-bound proof helpers.
- `go test ./easynet/invocation -run 'TestCanonicalInvocationIsDeterministic|TestNonceIsPartOfSignedBytes|TestSubjectIsPartOfSignedBytes|TestProfileIsPartOfSignedBytes|TestSignVerifyRoundtrip|TestTamperedNonceFailsVerification|TestUnsupportedAlgorithmRejected|TestAxiomVectorsAllPass'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./easynet/invocation`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `go test ./...`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects Go `legacyPlainInvocationBytes` in a `_test.go`
  fixture and proves the RF-3 gate rejects test-scoped legacy plain helpers.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Go legacy plain proof test-fixture removal and
the new V2 regression coverage for test-scoped Go helper names. It does not
complete RF-3 globally because remaining Node scripts/tests and broader
package, historical-vector, and example audits still require separate closure.

## RF-3 Node Legacy Plain Proof Script Removal Slice

Commands run on 2026-07-17:

- `rg -n "legacyPlainInvocationBytes|signLegacyPlainInvocation|verifyLegacyPlainInvocationSignature|legacy_plain_invocation|canonical_invocation_bytes unexpectedly empty|legacy plain" sdk/node -S`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after deleting `scripts/legacy-plain-fixtures.mjs` and migrating Node tests
  and vector runner to descriptor-bound proof helpers.
- `node --test tests/axiom-vectors.test.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, 12 passed and 3 skipped.
- `node ./scripts/run-axiom-vectors.mjs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, `axiom vectors: 14/14 passed (Node)`.
- `npm run axiom:vectors`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, including `npm run build` and `axiom vectors: 14/14 passed (Node)`.
- `npm run verify`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/node`:
  passed, including type checks, generated checks, axiom vectors, and
  protocol-pack vectors.
- `PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects a Node `scripts/legacy-plain-fixtures.mjs` helper and
  proves the RF-3 gate rejects script-scoped legacy plain helpers.
- `EASYNET_AXON_ROOT=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon PYTHON=/Users/macbook.silan.tech/.local/bin/python3.12 bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only Node legacy plain proof script/test removal and
the new V2 regression coverage for Node helper names. It does not complete
RF-3 globally because broader package, historical-vector, and example audits
still require separate closure.

## RF-3/RF-9 Active Proof Vocabulary Gate Slice

Commands run on 2026-07-17:

- `rg -n '\b(canonical_invocation_bytes|sign_invocation|verify_invocation_signature|verify_signature)\b|\bcanonicalInvocationBytes\b|plain canonical invocation|client-sdk::admission::canonical_invocation_bytes|\bURI \+ profile\b|\bURIs\b|\bURI\b|<uri>|\b(subject|caller|callee) URI\b|\b(caller|callee|subject|caller_binding|callee_binding|subject_binding|axiom_binding\.caller|envelope\.caller)\.uri\b|\bstring uri\b|\bpeer_uri\b|find_peer_by_uri|uri_profile|resolver\.resolve\(uri\)|canonical URI format|"(uri)"|"\buri\b"\s*:' document/rfcs/001-envelope-axiom-alignment.md document/rfcs/001-pr2-acceptance-checklist.md document/rfcs/002-keyring-and-keyresolver.md sdk/SDK_INTERFACE_SPEC.md sdk/FEDERATION_INVOKE_SCHEMAS.md sdk/conformance/cases/axiom/axiom-admission-pipeline.json sdk/conformance/cases/axiom/axiom-worked-example-authenticated.json sdk/go/easynet/dendrite_bridge_signed_invoke_cgo.go sdk/go/easynet/invocation/axiom.go sdk/java/src/test/java/run/easynet/axon/invocation/AxiomWorkedExampleTest.java sdk/python/easynet_axon/invocation/axiom.py`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after rewriting active proof and URA terminology.
- `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects active proof-document regressions and active
  URI-document regressions and proves both gates reject them.
- `python3 -m json.tool sdk/conformance/cases/axiom/axiom-admission-pipeline.json >/dev/null && python3 -m json.tool sdk/conformance/cases/axiom/axiom-worked-example-authenticated.json >/dev/null`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `bash core/runtime-rs/scripts/check-canonical-invocation-boundary.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh && bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only active proof/URA vocabulary convergence and the
new V2 regression coverage for active specification/comment drift. It does
not complete RF-3, RF-9, or the full canonical runtime convergence SPEC.

## RF-9 Active Ontology and Axiom Vector URA Naming Slice

Commands run on 2026-07-17:

- `rg -n "URI|URIs|Uri|uri|AgentUri" document/concepts/ONTOLOGY_AGENT_ABILITY.md sdk/conformance/cases/axiom/axiom-identity-composite-required.json sdk/conformance/cases/axiom/README.md sdk/java/src/test/java/run/easynet/axon/AbilityLifecycleStartServerTest.java`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after rewriting active ontology/conformance terminology and the Java test
  method name.
- `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh && bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects `AgentUri`, URI addressability, and axiom-vector URI
  wording to prove the active-document gate rejects this RF-9 regression.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `python3 -m json.tool sdk/conformance/cases/axiom/axiom-identity-composite-required.json >/dev/null`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `mvn -q -Dtest=AbilityLifecycleStartServerTest test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/java`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh && bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies only active ontology/conformance URA terminology
convergence and the new V2 regression coverage for that active-document set.
It does not complete RF-9 or the full canonical runtime convergence SPEC.

## RF-9 Axon Document-Wide URA Vocabulary Gate Slice

Commands run on 2026-07-17:

- `rg -n "\bURI\b|\bURIs\b|\bUri\b|\buri\b|_uri\b|AgentUri" document sdk/SDK_INTERFACE_SPEC.md sdk/FEDERATION_INVOKE_SCHEMAS.md sdk/conformance/cases/axiom/README.md sdk/conformance/cases/axiom/axiom-identity-composite-required.json -g '*.md' -g '*.tex' -g '*.json' -g '!target' -g '!node_modules'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after replacing the remaining brand and ecosystem-plan retired terminology.
- `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh && bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh && bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies Axon document-wide URA terminology convergence and the
new V2 regression coverage for the `document/` tree. It does not complete
RF-9 because generated-schema ownership and any non-document source/test
terminology gaps still require separate closure.

## RF-9 Dendrite Active Source/Test URA Vocabulary Slice

Commands run on 2026-07-17:

- `rg -n "\bURI\b|\bURIs\b|\bUri\b|\buri\b|_uri\b|[A-Za-z0-9]+Uri\b|[A-Za-z0-9]+URI\b" core/runtime-rs/dendrite-bridge/docs/AUTHENTICATED_INVOCATION.md sdk/go/easynet/signed_invoke_request_test.go sdk/go/easynet/ability_lifecycle_server_test.go -g '!target'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches
  after the Dendrite contract and Go test naming updates.
- `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh && bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed. The
  self-test now injects `CallerURI`, `resourceURI`, and URI-suffixed Go test
  names to prove the active source/test gate rejects this RF-9 regression.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `go test ./easynet -run 'TestSignedInvokeRequest_RejectsEmptyCalleeURA|TestNormalizeHubEndpointConvertsAxonEndpoint'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/go`:
  passed.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh && bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies Dendrite active contract and Go SDK test URA
terminology convergence plus V2 regression coverage for those paths. It does
not complete RF-9 because broader non-document source/test audits and
generated-schema ownership remain separate closure work.

## RF-9 Axon Active Source-Wide URA Vocabulary Gate Slice

Commands run on 2026-07-17:

- `rg -n "\bURI\b|\bURIs\b|\bUri\b|\buri\b|_uri\b|[A-Za-z0-9]+Uri\b|[A-Za-z0-9]+URI\b" core sdk scripts packaging -g '!target' -g '!node_modules' -g '!*.pb.go' -g '!*.lock' -g '!*.png' -g '!*.jpg' -g '!dist/**' -g '!build/**' -g '!*.egg-info/**' -g '!.venv/**' -g '!.venv-test/**' -g '!.pytest_cache/**'`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: only
  transport-library uses remain:
  `core/runtime-rs/dendrite-bridge/src/common.rs:40` and
  `core/runtime-rs/dendrite-bridge/src/raw_transport.rs:32`, both importing
  `http::uri::PathAndQuery`.
- `bash -n tools/scripts/check-canonical-runtime-convergence-v2.sh && bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed
  against the live EasyNet-Axon checkout.
- `cargo test -q -p axon-runtime join_accepts`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/core/runtime-rs`:
  passed, 2 passed.
- `cargo test -q --manifest-path sdk/rust/Cargo.toml bootstrap_appends_distinct_keys_for_same_node_ura_within_bound`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed, 1
  passed.
- `rg -n "membership_uri|canonical_device_uri|same_node_uri|URI|Uri" core/runtime-rs/src/services/invocation/hub_profile/tests/join.rs sdk/rust/src/invocation/runtime_admin.rs`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: no matches.
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `bash tools/scripts/check-architecture-convergence.sh && bash tests/scripts/test_check_architecture_convergence.sh`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo fmt --all -- --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `cargo check --lib --features axon-pb`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.
- `git diff --check`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon`: passed.
- `git diff --check -- tools/scripts/check-canonical-runtime-convergence-v2.sh pr/20260717-canonical-runtime-convergence-v2/02-architecture.md pr/20260717-canonical-runtime-convergence-v2/04-execution-checklist.md pr/20260717-canonical-runtime-convergence-v2/05-verification.md pr/20260717-canonical-runtime-convergence-v2/06-decisions-log.md`
  from `/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli`: passed.

This evidence verifies active Axon source-wide URA terminology convergence and
V2 regression coverage across active source roots. It does not complete RF-9
because generated-schema ownership remains a separate closure requirement.
