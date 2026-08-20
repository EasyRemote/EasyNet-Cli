# SDK-owned EasyRemote daemon lifecycle facade

## Objective

Move EasyRemote daemon lifecycle configuration, legacy start-wire projection,
typed status projection, and runtime-client opening behind the EasyNet-Cli
Python SDK Runtime Core profile.

## Boundary

- The SDK owns daemon lifecycle DTOs, state validation, handle status, endpoint,
  runtime opening, and stop semantics.
- EasyRemote keeps only its public `DaemonStartConfig` and `DaemonHandle` names
  as product-facing aliases.
- Axon remains below the daemon; this slice does not create or alter protocol
  semantics, Invocation encoding, receipt verification, or URA grammar.

## Invariants

- `hub` and `device` lifecycle modes are explicit and validated before start.
- `node_id` remains an EasyRemote-facing alias for SDK `device_id`.
- Existing EasyRemote public behavior for `to_wire()`, `status()`,
  `invocation_endpoint()`, `open_client()`, and `stop()` is preserved.
- EasyRemote no longer owns conversion from its start config to SDK
  `StartConfig` or daemon status dict projection.
