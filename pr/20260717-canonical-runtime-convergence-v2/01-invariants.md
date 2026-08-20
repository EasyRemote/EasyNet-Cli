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

## Current Slice: Canonical Session Stability And Lifecycle Control

9. The session prelude is the sole publisher of baseline owner, device, and
   hosted-agent projections. A transport-ready callback may schedule dynamic
   catalogue refreshes, but it may not publish the baseline a second time.
10. A joined session becomes `ConnectedOnline / RefetchReadModel` only after
    all required preludes complete. Transport loss is the sole authority that
    demotes that session; publication callbacks do not own connection state.
11. `invocation.cancel` is generic runtime lifecycle control, not a product
    ability grant. Its descriptor-bound command must be independently signed,
    admitted only as lifecycle control, and then authorized against the exact
    registered lifecycle hash, original caller, and execution authority.
12. A BIDI close/cancel sequence yields one canonical target terminal and one
    independently observable cancellation acknowledgement. Unknown targets,
    mismatched callers, and mismatched authorities remain fail-closed.
13. Every daemon invocation surface receives the same process-owned
    `InvocationCancellationRegistry`. Session dispatchers may not construct a
    private/default registry, because registration and lifecycle control must
    consult one authority instance.
