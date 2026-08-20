# Swift Async Sequence Seam Plan

## Goal

Add Swift-native `AsyncSequence` support to the Runtime Core stream and bidi seam while preserving the existing injected transport model and bounded lifecycle state.

## Scope

- Make `StreamHandle` an `AsyncSequence` over `StreamEvent`.
- Make `BidiSession` an `AsyncSequence` over `BidiFrame`.
- Yield terminal stream/bidi items once, then finish the async sequence.
- Extend Swift seam tests and docs to cover the idiomatic async sequence surface.

## Non-Scope

- No daemon or C ABI provider.
- No profile clients beyond Runtime Core.
- No product stream or session lifecycle.
- No second buffering or transport model.
