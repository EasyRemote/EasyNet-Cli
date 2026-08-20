# Swift Async Sequence Seam Boundary Proof

Swift `AsyncSequence` support is an idiomatic facade over the existing Runtime Core stream and bidi handles. It does not add a new provider path, daemon transport, generated wire type, or product lifecycle.

`StreamHandle` and `BidiSession` remain the lifecycle owners. Their async iterators call the existing `next()` methods and therefore reuse the same close/cancel checks, terminal-state handling, and bounded retained-history policy. The iterator only adds Swift-native consumption semantics by yielding the terminal item once and then ending.

This keeps the Swift seam aligned with the shared runtime model while satisfying the P1 Swift idiom requirement.
