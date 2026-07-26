API contract
============

Rust descriptor model
---------------------

`CallMode` has three explicit states:

- `Rpc`
- `Stream`
- `Bidi`

There is no default state.

Public behavior
---------------

No public CLI or SDK output changes in this iteration. Existing descriptor
builders still emit RPC for RPC abilities because they explicitly set that
state.

Failure semantics
-----------------

Future callers that rely on `Default::default()` for call mode should fail at
compile time and choose a mode intentionally.

Federation owner projection JSON that omits `callable_summary` is invalid.
Publishers must provide explicit callable fields and mode geometry.
