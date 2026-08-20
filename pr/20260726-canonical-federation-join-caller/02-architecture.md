Layering
========

- `invocation_wire::ProtoEnvelope` constructs only canonical URA envelopes.
- `daemon_route_runtime::BootstrapJoinProof` is a proof over a canonical
  membership caller, not an identity model.
- `runtime_admin::BootstrapCandidateKeyProvider` remains the bounded key
  resolver seam, but its key is the canonical caller URA.
- `descriptor_bound_dispatch` continues to hand one descriptor-bound request to
  Axon LocalRuntime.

Boundary correction
===================

The old design encoded proof state into a non-URA caller identity string. That
created a second identity namespace and broke canonical Axon envelope admission.
The corrected design keeps identity as URA and puts bootstrap proof in the
admission/proof layer.
