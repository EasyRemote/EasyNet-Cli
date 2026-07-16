# Decisions Log

- 2026-07-16: Selected the CLI hub/device release-gate slice because it closes a
  concrete runtime ownership gap: unified daemon paths are only reliable if the
  key-service binary is installed and the CLI-only proof harness is part of
  normal gate discovery.
- 2026-07-16: Kept the full E2E opt-in. The always-on gate gets the self-test,
  while release package shape is enforced by the static contract.
- 2026-07-16: Did not stage broad docs/spec churn or Rust formatting-only
  changes; they are not required for this root-fork slice.
- 2026-07-16: Read-only sidecar review recommended the purge publication FSM as
  the next most cohesive slice. Kept this iteration focused on already-selected
  release/tooling gates and left purge FSM uncommitted for a later slice.
