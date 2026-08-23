# Execution Checklist

- [x] Add explicit CLI scope selection.
- [x] Make the current paired User the default.
- [x] Reject non-Authority operator scope before I/O.
- [x] Add focused unit and convergence coverage.
- [x] Run formatting, focused tests, architecture gate, and live audit.

The architecture gate reaches an unrelated pre-existing violation in
`admission_facade.rs`; the new R153 rules themselves pass.
