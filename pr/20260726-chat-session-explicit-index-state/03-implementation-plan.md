# Implementation Plan

1. Add `SessionIndexLoadState`.
2. Add `load_index_with_state`.
3. Add explicit projection helpers for read and write call sites.
4. Migrate current call sites away from direct hidden defaulting.
5. Add regression tests for missing state and write initialization.
6. Update SPEC v2 gate and self-test fixtures.
