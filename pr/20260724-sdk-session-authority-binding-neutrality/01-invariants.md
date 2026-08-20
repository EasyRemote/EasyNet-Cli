# Invariants

1. Canonical authority bytes for session authority must not change.
2. SDK facade JSON must not expose product/topology field names for session
   authority binding.
3. DirectRuntime providers may translate from generated Axon field names into
   the SDK facade model at the provider boundary.
4. Go, Python, Node, Java and Swift validators must converge on the same field
   names and rejection behavior.
5. No compatibility fallback: retired facade fields fail closed.
