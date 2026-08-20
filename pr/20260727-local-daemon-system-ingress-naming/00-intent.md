# Local Daemon System Ingress Naming

## Goal

Retire `LocalDaemonLoopback*` authority terminology from the local daemon gRPC helper path and replace it with `LocalDaemonSystem*` names that describe the actual trust model.

## Non-goals

- Do not change public CLI, SDK, FFI, or wire behavior.
- Do not rename genuine network loopback tests or socket helpers.
- Do not add compatibility aliases for old internal names.

## Acceptance criteria

- The local daemon helper tuple plan and invocation projection use `LocalDaemonSystem*` names.
- Tests that assert local daemon system tuple behavior no longer call it loopback.
- Descriptor ref resolution still occurs inside daemon dispatch, not pre-resolved in the local helper.
- Existing convergence gates continue to pass.
