# Architecture

## Boundary

- `src/cli/daemon_client/remote_system_ability.rs` owns CLI sugar for target-owned remote daemon-system abilities such as catalog/device introspection.
- `src/cli/commands/groups/invocation.rs` owns CLI projection of invocation history filters into `invocation.history.list`.
- SDK `AuthorizedRuntimeSession` owns canonical receipt history preflight for language consumers.
- Daemon admission owns final authority verification and must reject malformed tuples.

## Refactoring Direction

The target-owned remote system facade must fail closed for receipt history abilities. This prevents a device-owned subject policy from being applied to a session-scoped query and removes the need for downstream admission to diagnose a predictable construction error.
