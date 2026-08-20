Layering:
- `src/core/identity` owns identity-shape predicates, including the retired invocation-history subject path.
- `src/daemon/invocation/admission/authority_metadata` consumes core identity predicates when classifying session authority subjects.
- FFI and SDK public ingress continue to rely on the same core predicates; no product-specific fallback is added.

Boundary decision:
- The old subject carrier is not a product lifecycle state. It is an invalid runtime identity shape.
- The daemon must fail closed rather than adapting old Hub/UI/backend subject metadata.
