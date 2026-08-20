Goal
====

Retire the trait-level `Default` implementation for descriptor `CallMode`.

Non-goals
=========

- Do not change authored descriptor records that explicitly choose RPC.
- Do not change route resolution, stream, or bidi dispatch behavior.
- Do not remove explicit `CallMode::Rpc` choices from constructors in this
  iteration.

Acceptance criteria
===================

- `CallMode` no longer derives `Default` and has no `#[default]` variant.
- Owner projection `AbilityCallableSummary` no longer defaults a missing
  callable summary into an implicit RPC read-model row.
- Existing callers continue to compile because transport mode is selected
  explicitly.
- SPEC v2 rejects reintroducing an implicit RPC default on descriptor call mode.
