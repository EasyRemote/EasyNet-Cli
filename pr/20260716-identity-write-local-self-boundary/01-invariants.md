# Invariants

1. `AdmissionTransportBoundary` owns the local-self caller predicate.
2. `IdentityWriteGate` receives the current transport boundary as a read-only
   projection from `AdmissionFacade`.
3. Identity writes use `local_self` wording/state, not a separate loopback flag.
4. `OffBoxStrict` must not authorize daemon-URA spoofing as local self.
5. The old `loopback` caller field and `is_loopback` gate must not reappear in
   identity-write authorization.
