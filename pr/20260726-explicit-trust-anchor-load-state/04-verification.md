# Verification Plan

- Targeted trust-anchor unit tests for missing, strict, empty, loaded, and
  malformed files.
- Targeted daemon reload test proving missing storage preserves the existing
  trust cell.
- Targeted local daemon receipt resolver test proving missing and load-failed
  trust sources remain distinct.
- SPEC v2 self-test and full gate.
- Architecture convergence gate.
- `cargo fmt --check`.
- `git diff --check`.
