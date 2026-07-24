# Boundary proof

## Root abstraction problem

Child invocation construction treats descriptor/callee binding as mandatory, but
does not currently require the selected route reference to be present in the
route shape invariant. The built child invocation serializes `route_ref`; if
that value can be empty, downstream audit and receipt analysis lose the single
selected route authority even though the invocation otherwise looks
descriptor-bound.

## Ownership

- Route selection owns the route reference.
- `SelectedChildRoute` is the typed handoff from route selection into child
  invocation construction.
- `ChildInvocationBuilder` owns the fail-closed boundary before target
  admission.

## Invariants

- A child invocation cannot be built without a selected route reference.
- Route-selected descriptor/callee/ability/dispatch facts remain mandatory.
- Externally signed route mutation must be asserted against the expected failure
  code, not by self-comparing `err.code`.
- No product or adapter layer may repair a missing route reference.
