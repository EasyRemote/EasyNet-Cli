Semantic invariants:
- Pages HTTP serving is an adapter projection from a schema-bound `page.fetch` result into HTTP bytes.
- Direct handler consumption must not be described as canonical Invocation dispatch.
- Canonical receipt ownership belongs to daemon Invocation paths, not to the HTTP listener adapter.

Safety invariants:
- Malformed fetch output must fail closed and must not produce HTTP 200.
- Fetch error details remain daemon/operator diagnostics; HTTP responses stay coarse.

Boundary invariants:
- The listener owns HTTP framing only.
- Pages fetch owns sandboxed byte reads.
- No compatibility module name keeps the old pseudo-ability boundary alive.
