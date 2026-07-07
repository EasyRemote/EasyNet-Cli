# Boundary Proof

## Ownership

The C ABI is the binding-facing Runtime Core projection for `libeasynet_cli`.
Header/export/spec alignment is therefore a SDK cutover precondition, not a
release-packaging-only check.

## ABI Direction

ABI v4 is the current daemon SDK projection. ABI v3 remains historical
Invocation-only context and must not be used as the active cutover gate.

## Product Boundary

The gate validates generic SDK ABI symbols and typed runtime contracts. It does
not introduce product-specific daemon behavior or profile policy.

## No Compatibility Alias

The existing ABI v4 checker already rejects retired auto-spawn and old ability
module exposure. Wiring it into cutover readiness makes those failures part of
the main SDK completion audit.
