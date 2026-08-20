# Invariants

1. Every remote invocation must have one canonical caller signer loaded before
   route dispatch.
2. Receipt-history requests must bind the session authority subject to the
   caller principal, not to the target device.
3. Descriptor resolution must be authority-bound and must not consult a second
   product-owned descriptor path.
4. Offline owner and descriptor-not-found outcomes must surface as typed route
   failures, not internal errors.
5. Cleaning old local state is allowed for this task and must not introduce
   compatibility paths for stale data.
