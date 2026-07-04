# Pipeline Live Tail Cutover

## Goal

Move EasyRemote Pipeline/Mission live event tailing semantics into the Python
SDK Mission profile while preserving EasyRemote's product-facing API shape.

## Boundary

- EasyRemote owns the Python Pipeline DSL and EAL source generation.
- The daemon owns Mission/EAL execution, status, cancellation, and child
  Invocation semantics.
- The Python SDK owns mission event DTOs, cursor advancement, drop reporting,
  terminal handling, and bounded live-tail iteration.
- EasyRemote may expose ergonomic `tail_events()` methods, but it must not own
  event cursor state or terminal/drop semantics.

## Invariants

- Tail iteration advances only through SDK `MissionEventPage` projections.
- Dropped daemon events are not silently ignored.
- Terminal mission events close the tail iterator deterministically.
- Empty non-terminal pages are bounded by an explicit caller option.
- Pipeline DSL generation remains unchanged.

## Non-Goals

- Do not change the daemon SDK requirements spec.
- Do not implement a new daemon streaming protocol in Python.
- Do not move Pipeline DSL syntax or EAL rendering into the SDK.
- Do not claim full child Invocation execution conformance in this slice.
