# Explicit Invocation Target Issuer Cutover Plan

Date: 2026-07-27

## Goal

Remove the remaining production-facing direct constructor path for explicit
local `InvocationTarget` tuple bindings. Public tuple facts must enter daemon
routing through a named ingress/issuer boundary, not through ad hoc
`InvocationTarget` construction from arbitrary production modules.

## Root Abstraction Problem

`InvocationTarget` is a resolved routing value object. It carries scope,
ability, call mode, subject, causal context, and request metadata after the
caller authority has already been classified.

The existing `InvocationTarget::local_explicit_tuple` constructor is public and
lets production modules bypass a named issuer when converting explicit tuple
facts into a local target. Even when callers pass complete fields, the
constructor location makes ownership unclear:

1. `InvocationTarget` appears to own public ingress policy.
2. Production modules can assemble explicit tuple targets directly.
3. Gates cannot distinguish legitimate public ingress preservation from a new
   hidden subject/default path except by broad grep.

The clean model is:

```text
public tuple facts -> PublicInvocationTargetIssuer -> InvocationTarget
daemon-system facts -> SystemInvocationTargetIssuer -> InvocationTarget
```

## Boundary Invariants

1. `InvocationTarget` remains a resolved value object, not a public ingress
   policy factory.
2. Public ingress must preserve explicit `subject` and `causal_context`.
3. Daemon-system root subject/causal derivation remains available only through
   `SystemInvocationTargetIssuer`.
4. No production module outside `routing/target.rs` may call
   `InvocationTarget::local_explicit_tuple`.
5. Tests may inspect the value object, but production construction must use the
   named issuer boundary.

## Implementation Direction

1. Add a `PublicInvocationTargetIssuer` with a semantically named method for
   local explicit tuple construction.
2. Make the raw `InvocationTarget::local_explicit_tuple` constructor private to
   `routing/target.rs`.
3. Migrate production callers in `agents/discover.rs`,
   `governance/meta.rs`, and `local_runtime_invoker.rs`.
4. Add a negative architecture gate rejecting production calls to the retired
   direct constructor outside `routing/target.rs`.
5. Keep public behavior unchanged: same tuple fields, same LocalRuntime request
   projection, same error behavior for invalid subjects.

## Acceptance Checks

- codegraph caller search showed production direct constructor calls removed;
  remaining raw calls are inside `target.rs` and the negative gate fixture.
- `cargo test -q --features axon-pb explicit_tuple` passed.
- `cargo test -q --features axon-pb invocation_target` passed.
- `bash tools/scripts/check-daemon-invocation-migration.sh` passed.
- `bash tests/scripts/test_check_daemon_invocation_migration.sh` passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `cargo fmt --check` passed.
- `git diff --check` passed.

## Completed Refactoring

- Added `PublicInvocationTargetIssuer` as the named production boundary for
  public explicit tuple target construction.
- Made `InvocationTarget::local_explicit_tuple` private to
  `routing/target.rs`, so the value object no longer doubles as a public
  ingress factory.
- Migrated the production discover provider route and cross-module tests to
  the issuer.
- Extended `check-daemon-invocation-migration.sh` and its self-test to reject
  new production calls to the retired raw constructor.
