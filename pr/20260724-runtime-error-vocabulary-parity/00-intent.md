# Intent

## Goal

Continue removing compatibility error vocabulary from product-visible runtime
ingress and SDK adapter paths so descriptor, route, signer, and lifecycle
states remain canonical across products and language SDKs.

## Non-goals

- Do not add compatibility mappings that reinterpret daemon failures for a
  specific product.
- Do not restore retired browser placeholder abilities or descriptors.
- Do not loosen admission, signer custody, or descriptor-bound invocation.
- Do not hide missing identity provisioning behind route fallbacks.

## Acceptance criteria

- Identify one concrete legacy/compat seam by source evidence.
- Remove or structurally gate the seam at the owning layer.
- Add focused regression coverage and SPEC/static coverage if the seam is
  architectural.
- Verify the canonical convergence gate and relevant focused tests.
