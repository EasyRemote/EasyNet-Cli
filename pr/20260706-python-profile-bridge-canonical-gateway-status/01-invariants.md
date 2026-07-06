# Invariants

1. Gateway lifecycle/readiness state is owned by the SDK core projection, not by Python facade fallback logic.
2. `GatewayStatus` must preserve degraded states; no facade may collapse missing readiness fields into `ready`.
3. Python and Go must validate the same logical DTO fields: `profile`, `kind`, `gateway_id`, `state`, readiness booleans, listener list, identity, and metadata.
4. Product ability invocation still flows through daemon Invocation; this change is limited to Admin + Gateway status projection.
5. Public method names and request/response dataclasses remain stable.
