Layering
========

- `daemon::ability::names::governance` owns the canonical ability predicates.
- `daemon::invocation::dispatch::governance_read_route` owns selected-route
  governance read admission.
- Unary, stream, and bidi dispatchers call the shared gate before LocalRuntime
  or presence dispatch.

Boundary change
===============

The previous module name encoded the implementation detail "remote". The actual
boundary is selected-route governance-read admission. LocalRuntime selected
routes and remote presence routes are both downstream dispatch targets of the
same resolver decision, so the policy must live above both.

Removed architecture fork
=========================

Remote selected routes no longer have a dedicated governance read authority
while LocalRuntime selected routes defer to Axon admission. Both dispatch paths
now share one policy object.
