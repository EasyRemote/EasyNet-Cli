# Intent

## Goal

Refresh the architecture-convergence shell self-test fixture so its canonical
fixture passes the current production gate set again.

## Problem

`tools/scripts/check-architecture-convergence.sh` passed against the real
checkout, but `tests/scripts/test_check_architecture_convergence.sh` failed in
its own "canonical fixture should pass" setup. The fixture no longer expressed
newer canonical architecture requirements for routeability ownership, callee-only
target extraction, local loopback subject policy, local target subject policy,
identity projection, and bidi receipt payload projection.

## Non-goals

- Do not relax production architecture gates.
- Do not remove negative fixtures.
- Do not make the fixture compile as a Rust crate; this shell test validates
  text-level gate coverage.
- Do not introduce product compatibility aliases.

## Acceptance Criteria

- The canonical fixture passes the current architecture convergence gate.
- Existing negative fixtures still fail on their expected rule markers.
- The rule marker for retired FFI descriptor remote-probe caller defaults is
  updated to the current bounded-catalog rule.
