# Invariants

1. A prepared invocation must carry an explicit top-level `descriptor_ref`.
2. `signing_material.descriptor_ref` must match the invocation tuple descriptor.
3. The top-level `descriptor_ref` must match `signing_material.descriptor_ref`.
4. No SDK may backfill the prepared descriptor from signing material.
5. Go, Python, Node, and Java must reject the same malformed payload shape.
6. Valid public API shape remains compatible: callers still pass/read
   `descriptor_ref`; only malformed provider/runtime payloads fail earlier.
