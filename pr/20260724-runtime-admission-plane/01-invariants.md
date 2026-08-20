# Invariants

1. `AdmissionFacade` remains the single verifier for transport-origin authority, trust, quota, lifecycle, and access-control policy facts.
2. `DaemonInvocationService` owns runtime planes, not loose facades.
3. Exact-route providers receive the same verifier instance as generic route dispatchers.
4. Local self-admission remains bounded by `AdmissionTransportBoundary`.
5. No compatibility layer may expose the old raw field name.
