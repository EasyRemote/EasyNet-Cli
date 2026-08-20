# Invariants

1. Node Receipt facade methods must delegate fetch, projection, causal refs,
   history carriers, and chain checks to injected transports; they must not
   parse ledgers, fabricate receipt URAs, or verify Axon signatures locally.
2. `ReceiptRef` must require explicit opaque receipt URA plus receipt hash and
   reject malformed anchors.
3. Node report records must reference only shared cases that declare `node` in
   `required_for`.
4. Node evidence must be repository-local and use `node_test` evidence kind so
   the shared runner can reject cross-language proof.
5. The report must not claim Axon-backed receipt chain verification, daemon
   transport, C ABI, or product cutover cases that are not declared for Node.
6. Scaffold validation must fail if the Node report is deleted or malformed.
7. Conformance fixture validation must resolve internal schema `$ref` targets
   before applying `oneOf`, `required`, `additionalProperties`, and nested
   property checks.
8. Feature discovery fixtures and schema must agree on every shipped profile
   and symbol; the schema is a projection contract, not a looser placeholder.
9. The Node SDK remains a language facade. It may validate DTO shape and
   lifecycle locally, but it must not own Axon grammar, receipt verification,
   daemon policy, or stream/bidi protocol truth.
