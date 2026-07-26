API contract
============

Internal Rust contract
----------------------

Accepted owner projections:

- `device`
- `authority`
- `agent:<id>`
- `user:<id>`
- `plugin:<id>`

Rejected owner projections:

- empty marker
- whitespace-mutated marker after trimming/canonicalization
- malformed `<plane>:<id>` marker
- retired `hub` marker

Public behavior
---------------

This slice changes only the internal authority binding owner-projection marker.
It does not rename product CLI modes or Authority URAs.

Failure semantics
-----------------

Retired `hub` authority scope facts fail as
`InvalidAuthorityOwnerProjection`.
