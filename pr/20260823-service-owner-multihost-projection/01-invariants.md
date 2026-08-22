# Invariants

1. Device remains execution substrate; Service remains the callee/owner for Service-owned abilities.
2. Agent, SystemAgent, Device, and Authority owner projections keep their single-owner revision fence.
3. Service owner projections are fenced per `(owner_ura, host_device_ura)`.
4. `federation.resolve` must not surface Service projections as generic Agent/SystemAgent rows.
5. `namespace.resolve` may route a Service-owned ability only through a live same-realm host Device projection.
6. Multiple Service host projections must not produce `accepted_count=0` solely because the owner URA is shared.
