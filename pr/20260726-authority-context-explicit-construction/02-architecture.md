Architecture
============

Root abstraction
----------------

`AbilityAuthorityContext` is a registry assembly capability, not a value object
with a neutral zero state. Its construction commits the daemon to one authority
source.

Boundary decision
-----------------

Remove `Default` so callers cannot accidentally obtain host-local authority
facts through generic default construction. Also remove the ambient
metadata-only catalog `new()` constructor because it selected a local authority
source without naming it at the call site.

The explicit constructors remain:

- `from_local_environment`
- `for_device_authority_root`
- `for_realm_authority_root`
- device + realm constructors
- `new_metadata_only_with_authority_context`
- `new_with_runtime_and_authority_context`
- test fixture constructors already present in tests

Layering
--------

This keeps environment discovery in daemon boot/catalog assembly and prevents
SDK/provider/read-model code from treating local daemon state as a generic
canonical runtime default.
