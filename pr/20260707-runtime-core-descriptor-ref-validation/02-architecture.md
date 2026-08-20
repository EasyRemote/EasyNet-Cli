# Architecture

## Root Abstraction Problem

`InvocationDraft` is the complete seven-tuple snapshot, but descriptor-ref
grammar is Axon protocol truth. Reimplementing its `ability@version` parser in
Python Runtime Core creates a second source of truth.

## Target Architecture

- Validate required Invocation tuple fields during draft inspection.
- Keep descriptor-ref projection and canonical construction in Directory + Identity helpers.
- Keep Go Runtime Core draft construction independent of descriptor-ref grammar parsing.
- Use the Go Identity/Axon-delegated projection helper when callers need descriptor facts or canonical construction.
- Use the Python Identity/Addressing projection facade for descriptor-ref decomposition.

## Module Boundaries

- `sdk/go/ability_descriptor.go`: Go descriptor-ref parser facade.
- `sdk/go/invocation.go`: Go draft construction.
- `sdk/python/easynet_sdk/ability_descriptor.py`: Python descriptor-ref projection facade.
- `sdk/python/easynet_sdk/invocation.py`: Python draft construction without descriptor-ref grammar parsing.
