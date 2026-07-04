# Host Context Causal Intent

## Objective

Move EasyRemote server-side `Context.call` from an always-disabled placeholder
to an SDK-mediated child dispatch path when, and only when, the daemon supplies
an opaque parent receipt anchor.

## Boundary

- EasyNet-Cli SDK Host Binding owns host-stream envelope/request DTOs.
- EasyNet-Cli SDK Receipt owns parent-receipt-to-causal-context projection.
- EasyRemote may execute Python user code and keep ergonomic `Context.call`,
  `Context.invoke`, and `Context.stream` methods.
- EasyRemote must not fabricate receipt URAs, hash anchors, or causal placement.

## Non-goals

- Do not change `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not claim Axon cryptographic receipt verification from summary data.
- Do not move Python function execution into the SDK.
