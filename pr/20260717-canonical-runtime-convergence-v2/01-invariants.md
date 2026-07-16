# Canonical Runtime Convergence V2 - Invariants

1. Every public invocation is a complete seven-tuple and reaches exactly one
   terminal receipt or terminal error.
2. Descriptor-bound proof is the only canonical signing, verification,
   replay, admission, and receipt-binding path.
3. SDK code never creates or caches an authority key as a fallback. Signing is
   performed by an explicit caller-owned signer or a daemon-owned key service.
4. Axon owns generic invocation semantics only. EasyNet-Cli owns Mission/EAL,
   product identity, plugins, integrations, and device policy.
5. Every language facade implements the same lifecycle capability matrix and
   passes the same state-transition vectors.
6. CLI adapters submit a complete invocation to Axon constructors; they do
   not hand-assemble canonical envelope or receipt proof shape.
7. A compatibility adapter, when release policy requires one, sits outside the
   canonical domain, is one-way, and has a removal version and gate.
8. URA is the only routable identity/address term in active protocol, code,
   tests, and normative documentation.
