Boundary proof
==============

Before
------

`NewRuntimeAdminClient` accepted `*RuntimeHost` and `RuntimeAdminClient` stored
that concrete host. This made the neutral admin facade depend on the SDK host
implementation even though the operations it needs are exactly the lifecycle
contract.

After
-----

`NewRuntimeAdminClient` accepts `RuntimeLifecycle`, and `RuntimeAdminClient`
stores `RuntimeLifecycle`. `*RuntimeHost` remains a valid argument because it
implements the interface. Tests prove an independent lifecycle implementation
can back admin start/discover/attach without being a `RuntimeHost`.

Ownership
---------

- `RuntimeAdminClient`: command orchestration, health aggregation and readiness
  projection.
- `RuntimeLifecycle`: start, attach, discover and connect-local contract.
- `RuntimeHost`: default lifecycle implementation over
  `RuntimeLifecycleTransport`.
- `RuntimeHandle`: handle state and lifecycle transition validation.
