# Descriptor-ref resolver normalization

## Intent

Fix SDK/runtime descriptor resolution failures where the runtime has a
descriptor-bound local ability, but `ResolveDescriptorRef` reports
`descriptor_ref not found` for metadata abilities such as
`meta.list_abilities` and `meta.list_resources`.

## Invariants

1. Descriptor resolution is descriptor-bound and fail-closed.
2. The resolver accepts exactly the public selector forms the SDK exposes:
   owner-local ability name or canonical Ability URA.
3. Short names and Ability URAs must normalize to one canonical ability identity
   before catalog matching.
4. The canonical daemon catalog remains the authority; SDK/CABI layers must not
   synthesize product-specific fallback descriptor refs.
5. URA terminology is mandatory. Do not introduce URI naming.

## Boundary proof

- The bug belongs at the runtime descriptor resolver boundary because the
  failing tuple is `(callee_ura, ability, call_mode)` and the local catalog
  already contains descriptor facts after descriptor wire projection.
- The repair must not add EasyNet/EasyRemote product abstractions to SDKs.
- The repair must not weaken descriptor proof requirements; missing catalog
  facts remain errors.

## Verification

- CodeGraph exploration of descriptor resolver/canonical catalog call paths.
- Focused resolver tests for short-name and Ability-URA selectors.
- Live `meta.list_abilities` invocation against the local Device URA.
- `cargo fmt --check`.
- `cargo check --bin easynet --bin easynet-daemon`.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh`.
