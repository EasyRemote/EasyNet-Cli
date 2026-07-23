# Verification

Planned checks:

- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo test -q subscribe_directory_v2 --features axon-pb`
- `cargo test -q cross_realm_directory_streaming --features axon-pb`
- `cargo fmt --check`
- `git diff --check`

Results:

- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` —
  passed. The self-test now includes a descriptor-only v1 fixture and confirms
  the gate fails if `federation.subscribe_directory.ability.toml` reappears.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo test -q subscribe_directory_v2 --features axon-pb` — passed
  (`6 passed` for the matching unit filter).
- `cargo test -q --test cross_realm_directory_streaming_e2e --features axon-pb`
  — passed (`2 passed`).
- `cargo fmt --check` — passed.
- `git diff --check` — passed.

Implementation note: the first attempt to run
`cargo test -q --test cross_realm_directory_streaming_e2e --features axon-pb`
failed because the in-process e2e fixture constructed
`DaemonInvocationService` without the required invocation attempt audit ledger.
The fixture now wires the same production observability dependency used by
daemon boot.
