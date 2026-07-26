API contract
============

Internal Rust contract
----------------------

`AbilityAuthorityContext` has no default state. Callers must choose an explicit
constructor that names the authority source.

`AxonAbilityCatalog` has no default metadata-only constructor. Metadata-only
catalogs must be built with an explicit `AbilityAuthorityContext`.

Public behavior
---------------

No CLI or SDK public behavior changes. Daemon boot still uses
`from_local_environment` where local authority discovery is the intended source.

Failure semantics
-----------------

If a future call site needs authority context, the compiler should fail until it
selects a constructor intentionally.
