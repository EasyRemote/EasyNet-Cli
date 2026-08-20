# Invariants

1. Local daemon helper calls still construct complete seven-tuple envelopes.
2. Helper calls still use the daemon-local system caller identity.
3. Helper calls must not pre-resolve descriptor refs.
4. The dispatcher remains the boundary that classifies trusted local-system ingress.
5. Public signed ingress remains descriptor-bound and caller-signed.
