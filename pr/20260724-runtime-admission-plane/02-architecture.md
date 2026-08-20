# Architecture

## Boundary

`RuntimeAdmissionPlane` is a value object owned by `DaemonInvocationService`. It carries the canonical admission verifier and exposes narrowly named methods for:

- cloning the verifier into route providers/dispatchers;
- checking trusted local system envelopes;
- replacing the transport boundary during listener setup;
- reading access-control stores for test and assembly verification.

## Direction

This removes the semantic leak where the service treated admission as a legacy transport facade. The admission verifier remains reusable, but service ownership now reflects the canonical runtime plane model used by directory, federation, session, identity, and runtime planes.
