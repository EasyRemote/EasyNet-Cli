# Boundary Proof

## Ownership

SPEC section 27 defines SDK conformance obligations. The gate belongs in
EasyNet-Cli SDK infrastructure because it validates the canonical runtime model
across language facades and product cutover gates.

## Runtime Model

The manifest maps SPEC requirements to shared daemon SDK conformance cases. It
does not introduce product-specific lifecycle, product-specific receipt shapes,
or facade-only behavior.

## No Compatibility Alias

Coverage mappings preserve the SPEC case ids as normative inputs. A broader
shared case may cover a SPEC case only when the manifest names the relationship
explicitly and points at existing shared case files.

## Failure Mode

If a section 27 case is removed, renamed, or mapped to a non-existent shared
case, the cutover readiness gate fails before any backend or EasyRemote cutover
claim can be made.
