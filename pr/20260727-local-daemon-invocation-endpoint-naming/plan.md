# Local daemon Invocation endpoint naming convergence

## Goal

Remove production-path wording that describes the EasyNet product daemon Invocation endpoint as a "local Axon daemon". The local process is the EasyNet daemon; Axon owns protocol/runtime semantics embedded behind the daemon Invocation endpoint.

## Invariants

- Product daemon ownership must not be exposed as Axon process ownership.
- Public or diagnostic errors may say "local daemon Invocation endpoint" but must not imply a separate local Axon daemon product path.
- Axon remains the protocol/runtime model used behind the endpoint.
- No invocation tuple behavior, receipt behavior, or public command shape changes in this slice.

## Boundary proof

- `src/support/platform/local_daemon_grpc.rs` is a CLI/product transport helper for the local easynet-daemon Invocation listener.
- The correct abstraction name at this layer is the daemon Invocation endpoint, not an Axon daemon.
- The SPEC v2 local-daemon system contract is the gate that should reject future ownership wording regressions.

## Refactoring plan

1. Replace production error/context strings that say "local Axon daemon" with "local daemon Invocation endpoint".
2. Keep feature-gated unsupported messages explicit about the daemon Invocation transport requiring `axon-pb`.
3. Extend the canonical-runtime-convergence v2 gate to reject the retired wording in production source.

## Verification

- Focused Rust tests for the local daemon gRPC helper.
- Canonical runtime convergence v2 gate.
- Transport locator terminology gate.
- Formatting and diff checks.
- codegraph query for the retired wording.
