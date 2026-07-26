Goal
====

Remove the ShellGuard path-normalization fallback that converts an invalid
empty normalized path into `/`.

Non-goals
=========

- Do not change shell command parsing.
- Do not touch filesystem state or introduce canonicalize-based symlink
  resolution.
- Do not change read redirect policy.

Acceptance criteria
===================

- Path normalization returns an explicit error for empty cwd/target states.
- `pathconstraints::evaluate` projects normalization failures as a distinct
  verdict rather than treating `/` as a safe replacement.
- Existing allowed-root and outside-root behavior remains unchanged.
