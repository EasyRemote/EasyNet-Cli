# RF-8 Public Remote Ingress Cutover

## Intent

Remove the public remote invocation root-state defaults from
`easynet ability invoke --node`. Public remote ingress now supplies the
caller-visible AXIOM tuple facts before lowering into the daemon invocation
runtime path.

## Architecture Decision

- Public remote invocation uses `RemoteInvocationTuplePlan::public_explicit`.
- Public callers must provide `subject`, `invocation_nonce`, and
  `causal_context` before dispatch.
- Root causal placement is represented by an explicit CLI declaration
  (`--causal-root`) and materialized through `declared_root_causal_context`.
- Fresh nonce generation remains only in named runtime-admission and
  daemon-system paths, not the public remote ingress constructor.

## Refactoring

- Deleted the public `public_root` tuple constructor.
- Deleted `PairedOwnerDerived` remote subject policy.
- Deleted `PublicRootDerived` local causal policy.
- Replaced public remote nonce derivation with `RemoteInvocationNonce::Explicit`.
- Flipped the daemon invocation migration gate to reject reintroduction of the
  removed public default paths.

## Remaining RF-8 Work

- Local loopback support helpers still need the same explicit-public vs
  named-system issuer split.
- Runtime bridges that mint fresh runtime-admission nonces still need migration
  to child invocation or an explicit `ProductInvocationPolicy` owner.
- Full RF-7/RF-8 closure still requires live daemon inventory and the
  two-node EasyRemote CLI E2E.

## Verification

- PASS: `cargo fmt --all -- --check`
- PASS: `bash tools/scripts/check-daemon-invocation-migration.sh`
- PASS: `bash tests/scripts/test_check_daemon_invocation_migration.sh`
- PASS: `bash tools/scripts/check-architecture-convergence.sh`
- PASS: `bash tests/scripts/test_check_architecture_convergence.sh`
- PASS: `git diff --check`
- PASS: `cargo test --features axon-pb --lib tuple_plan -- --nocapture`
  - Result: 6 passed.
- PASS: `cargo test --features axon-pb --lib cli::commands::invoke::tests -- --nocapture`
  - Result: 9 passed.
