# Device meta.list_abilities descriptor binding

## Intent

Fix `meta.list_abilities` invocation against the local Device URA failing with
`descriptor_ref not found` even though the daemon is connected and the ability
should be a first-class device-owned metadata surface.

## Invariants

1. `meta.list_abilities` must have a descriptor for every local runtime owner
   that can receive public control-plane metadata calls.
2. Descriptor lookup must stay descriptor-bound; callers must not fall back to
   string-only invocation.
3. Device, Hub, and hosted-Agent authority roots must use the same catalog
   descriptor machinery.
4. A missing descriptor is a boot/catalog assembly bug, not a network or Docker
   fallback condition.
5. The fix must not introduce product-specific behavior into the canonical SDK.

## Boundary proof

- The repair belongs in daemon ability catalog assembly / authority-root
  projection, because the failing key is `(callee_ura, ability, call_mode)`.
- Admission and FFI should continue to reject missing descriptor_refs.
- Existing dirty worktree changes outside this descriptor path are not part of
  this task and must not be staged.

## Verification

- CodeGraph exploration of `meta.list_abilities`, descriptor catalog assembly,
  and device authority roots.
- Focused unit tests for descriptor presence on Device `meta.list_abilities`.
- `cargo check --bin easynet --bin easynet-daemon`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
