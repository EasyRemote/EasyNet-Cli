# Architecture

## Boundary

`src/daemon/ability/builtins/agents/invoke.rs` owns parsing for the local agent invoke ability. Its responsibility is selecting the canonical Ability URA and forwarding typed business args to the resolved handler.

## Refactoring direction

The previous parser mixed ability business input with IPC/runtime sidecar metadata by silently accepting underscore-prefixed top-level fields. That duplicated canonical runtime envelope ownership and created a second place where caller/request facts could be interpreted.

The converged model keeps runtime metadata in the canonical invocation envelope and keeps `<agent>.invoke` args schema-strict.
