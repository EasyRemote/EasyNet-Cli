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
