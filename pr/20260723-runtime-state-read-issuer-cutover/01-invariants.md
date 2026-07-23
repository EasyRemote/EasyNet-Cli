# Invariants

1. Runtime-state read projections derive their subject from paired
   credentials, not daemon/device identity.
2. Missing user identity fails before any daemon/device fallback is attempted.
3. `meta.list_abilities`, `meta.list_resources`, `observe.health`, and
   `invocation.history.*` local CLI reads enter through
   `LocalRuntimeStateReadIssuer`.
4. Product action invokes are not migrated blindly; they must choose a subject
   according to their own lifecycle semantics.
5. Public CLI behavior remains compatible: command arguments and output shapes
   do not change.
