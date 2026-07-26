# Implementation Plan

1. Add an explicit `RealmTrustAnchorLoadState` enum in the trust-anchor module.
2. Replace `load_or_empty` with `load_with_state`.
3. Migrate daemon boot to handle `Missing` as first-run empty state.
4. Migrate daemon reload to reject `Missing` without mutating the trust cell.
5. Migrate CLI read projections to explicitly render `Missing` as empty output.
6. Migrate receipt resolver source loading to preserve a distinct missing state.
7. Update tests and SPEC gate coverage so the obsolete compatibility helper
   cannot return.
