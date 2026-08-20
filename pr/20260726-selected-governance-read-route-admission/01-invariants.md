Semantic invariants
===================

- Receipt-history abilities are governance read-model routes, not product
  actions.
- Runtime-catalogue abilities are governance catalogue reads, not product
  actions.
- A selected route must be classified before forwarding to presence or entering
  Axon LocalRuntime admission.

Safety invariants
=================

- A receipt-history read must use a Resource URA subject.
- A receipt-history read must not use the target device as subject.
- A catalogue read must use the canonical runtime-read subject accepted by the
  selected-route policy.
- Stream and bidi carriers must not carry governance reads; governance reads are
  unary Invoke routes.

Boundedness invariants
======================

- Rejection occurs synchronously at dispatch admission, before runtime
  invocation state, carrier queueing, or remote pending maps are allocated.
- No route may fall through to a second admission authority to discover the
  same policy failure later.
