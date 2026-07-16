# Verification

Planned checks:

- Focused exact stream close tests.
- Focused exact stream event, resume, and heartbeat tests.
- `cargo check --features axon-pb`
- `tools/scripts/check-architecture-convergence.sh`
- `tests/scripts/test_check_architecture_convergence.sh`
- Targeted `rustfmt --check`
- `git diff --check`

Lifecycle proof:

- `DaemonInvocationService` owns the only strong stream-route lifecycle token.
- `DaemonStreamRouteProvider` stores only weak lifecycle/presence references.
- Bridge tasks exit when the lifecycle token can no longer be upgraded.
