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
  passed.
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
  passed.
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
