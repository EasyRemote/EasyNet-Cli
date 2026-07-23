# Decisions Log

## 2026-07-23

- Decision: delete browser mock descriptors instead of downgrading their
  `capability_state`.
- Reason: the descriptors explicitly describe V0 mock behavior and no
  executable production handler exists. Keeping them in any active state would
  preserve a product-facing compatibility surface without a runtime capability.

## 2026-07-23 Verification

- Decision: update both convergence gates.
- Reason: SPEC v2 was already guarding retired browser mock implementation
  files, but not descriptor-only publication. The active descriptor inventory is
  public runtime surface and must be covered by the same boundary.
