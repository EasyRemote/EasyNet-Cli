# Legacy/compat retirement pass

## Intent

Continue architecture convergence by removing one root legacy/compatibility
surface instead of preserving an alternate runtime model behind passing gates.

## Boundary invariants

- URA remains the only project-owned routable identity vocabulary.
- Canonical runtime trust/admission/read-model paths must reject retired fields
  at their owning decoder or descriptor boundary.
- SDK and daemon surfaces must not preserve product-specific compatibility
  aliases just to keep old local state usable.
- Existing dirty `docs/spec/*` files are outside this iteration.

## Evidence plan

1. Use codegraph plus source search to find legacy/compat/fallback surfaces.
2. Select a root-owned surface where deletion improves the canonical model.
3. Migrate all callers and fixtures rather than adding adapters.
4. Add a regression gate or fail-closed test that blocks reintroduction.
5. Verify with format, targeted tests, and convergence gates.

## Verification log

- Codegraph refreshed with `codegraph sync .`.
- Removed daemon-local Authority owner fact synthesis from admission owner
  resolution.
- `resolve_owner` no longer accepts `daemon_ura` as an owner source.
- Authority self-read remains an explicit safe-read policy predicate rather than
  an owner-fact fallback.
- Added SPEC v2 source gate coverage to reject reintroduced local Authority
  owner synthesis.
- `cargo fmt --check` passed.
- `cargo test -q policy_gate --features axon-pb` passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh` passed.
- `tools/scripts/check-architecture-convergence.sh` passed.
