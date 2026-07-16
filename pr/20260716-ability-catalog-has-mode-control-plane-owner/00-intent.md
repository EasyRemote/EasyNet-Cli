# Intent

Converge `AxonAbilityCatalog::has_rpc`, `has_stream`, `has_bidi`, and
`list_rpc_names` onto the catalog transaction boundary.

Concrete use case: if a control-plane descriptor row is removed or a
registration transaction rolls back, catalog presence checks must not report
success merely because a handler entry or Axon `LocalRuntime` ability option
still exists. Routeability should require both the committed control-plane
mode record and the committed execution-index handler; `LocalRuntime` remains
the invocation engine, not a second catalog source of truth.

Expected effect: architecture convergence and cleaner failure semantics.
