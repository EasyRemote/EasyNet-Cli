# Route-negative failure classification cutover

## Goal

Stop projecting route-negative owner/offline failures as ordinary
`ABILITY_NOT_FOUND` / `NotFound` misses. A route-negative answer means the
runtime could not reach or admit the route owner; it is not proof that the
ability descriptor does not exist.

## Invariants

1. Remote invocation failure classification must preserve route terminality:
   route-negative owner/offline detail maps to route unavailability, not
   descriptor/ability absence.
2. Admission and routing failures remain distinct from descriptor catalog
   misses.
3. No caller may use `ABILITY_NOT_FOUND` as a compatibility fallback for
   `ROUTE_NEGATIVE`, `NEGATIVE_REASON_NXDOMAIN`, or `owner is not online`.
4. The classification is shared at the daemon forwarding boundary so all
   SDK/FFI consumers receive the same semantic transport result.

## Boundary proof

- `src/daemon/invocation/dispatch/remote_failure.rs` is the daemon boundary that
  converts remote runtime failures into gRPC status.
- It must classify typed/detail route-negative evidence before generic
  not-found handling.
- The SPEC v2 gate should reject a classifier that checks `ABILITY_NOT_FOUND`
  before route-negative evidence or lacks a regression test.

## Verification plan

1. Run targeted `remote_failure` Rust tests.
2. Run SPEC v2 convergence gate.
3. Run architecture convergence gate.
4. Run `cargo fmt --check` and `git diff --check`.

## Decision log

- Treat `ROUTE_NEGATIVE`, `NEGATIVE_REASON_NXDOMAIN`, and `owner is not online`
  as route unavailability evidence even if a legacy remote carrier reports
  `ABILITY_NOT_FOUND`. This removes a product-visible compatibility ambiguity:
  route owner absence must not appear as an ability-descriptor absence.
