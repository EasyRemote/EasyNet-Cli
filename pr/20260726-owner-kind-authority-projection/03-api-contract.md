API contract
============

Internal Rust contract
----------------------

`OwnerKind::authority_projection()` emits:

- `device`
- `authority`
- `agent:<id>`
- `user:<id>`

`owner_kind_from_projection()` accepts those same values and returns `None` for
retired `hub`.

Public behavior
---------------

This slice changes internal control-plane owner projection facts. It does not
rename product CLI modes or user-facing Hub URA helper functions.

Failure semantics
-----------------

A control-plane row with owner projection `hub` is treated as an unknown owner
projection and fails before registry owner reconstruction.
