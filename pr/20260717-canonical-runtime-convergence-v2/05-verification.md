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
