Runtime admin lifecycle interface boundary
==========================================

Root fork
---------

Runtime administration is product-neutral, but the Go admin facade still stores
and accepts the concrete `RuntimeHost` implementation. That keeps admin
ownership coupled to the host provider root instead of the canonical lifecycle
contract. Python already accepts `RuntimeLifecycle`, so the two SDKs should
converge on an interface/contract dependency.

Expected effect
---------------

This slice is architecture convergence:

- runtime admin depends on the neutral lifecycle interface, not a concrete host
  struct;
- existing callers that pass `*RuntimeHost` or the `DaemonControl` alias keep
  compiling because `RuntimeHost` implements the interface;
- provider, transport, health and runtime-handle behavior remain unchanged.
