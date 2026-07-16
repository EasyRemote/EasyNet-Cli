# Invariants

- `SelectedInvokeRoute` is the dispatch contract for resolver-selected calls.
- Resolver-selected descriptor refs come from owner projection/control-plane
  mode geometry, not from `LocalRuntime::ability_options`.
- `LocalRuntime` remains the execution installation check for selected local
  dispatch.
- Exact daemon routes and carrier wire-target ingress keep their current
  `from_wire_target` binding path in this slice.
- A selected route without descriptor proof must fail closed before Axon
  invocation starts.
