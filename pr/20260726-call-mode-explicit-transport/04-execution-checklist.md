Execution checklist
===================

- [x] Remove `Default` derive and `#[default]` from descriptor `CallMode`.
- [x] Remove `AbilityCallableSummary` defaulting and serde fallback for missing
  callable summaries.
- [x] Add SPEC v2 guard against reintroducing implicit RPC default.
- [x] Run descriptor/control-plane tests that cover call-mode projection.
- [x] Run fmt, diff check, and convergence gates.
